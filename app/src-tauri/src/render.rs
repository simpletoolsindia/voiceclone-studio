//! Story -> video orchestration with progress, ETA and cancellation.
//!
//! Runs as a background job owned by Rust: it drives the GPU worker for the
//! AI stages and ffmpeg for composition, saving every asset into the project
//! folder so re-rendering after an edit only redoes what changed.

use crate::engine::Engine;
use crate::story::{self, Project};
use crate::video::{self, Layout};
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Serialize, Clone)]
pub struct RenderJob {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub progress: f64,
    pub percent: f64,
    pub stage: String,
    pub started: f64,
    pub eta_sec: Option<f64>,
    pub elapsed_sec: f64,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Renders {
    jobs: Mutex<HashMap<String, (RenderJob, Arc<AtomicBool>)>>,
}

fn now_f() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

impl Renders {
    pub fn list(&self) -> Vec<RenderJob> {
        let mut v: Vec<RenderJob> = self.jobs.lock().unwrap().values().map(|(j, _)| {
            let mut j = j.clone();
            if j.status == "running" {
                j.elapsed_sec = now_f() - j.started;
                if j.progress > 0.03 {
                    j.eta_sec = Some(j.elapsed_sec / j.progress - j.elapsed_sec);
                }
            }
            j
        }).collect();
        v.sort_by(|a, b| b.started.partial_cmp(&a.started).unwrap());
        v
    }

    pub fn cancel(&self, id: &str) -> bool {
        self.jobs.lock().unwrap().get(id).map(|(_, c)| c.store(true, Ordering::Relaxed)).is_some()
    }

    pub fn clear_finished(&self) {
        self.jobs.lock().unwrap().retain(|_, (j, _)| j.status == "running");
    }

    fn create(&self, title: &str) -> (String, Arc<AtomicBool>) {
        let id = format!("r{}", story::new_id());
        let flag = Arc::new(AtomicBool::new(false));
        let job = RenderJob {
            id: id.clone(), kind: "story".into(), title: title.into(), status: "running".into(), progress: 0.0,
            percent: 0.0, stage: "Starting".into(), started: now_f(), eta_sec: None, elapsed_sec: 0.0,
            result: None, error: None,
        };
        self.jobs.lock().unwrap().insert(id.clone(), (job, flag.clone()));
        (id, flag)
    }

    fn update(&self, id: &str, progress: f64, stage: &str) {
        if let Some((j, _)) = self.jobs.lock().unwrap().get_mut(id) {
            j.progress = j.progress.max(progress.clamp(0.0, 1.0));
            j.percent = (j.progress * 1000.0).round() / 10.0;
            if !stage.is_empty() {
                j.stage = stage.into();
            }
        }
    }

    fn finish(&self, id: &str, r: Result<serde_json::Value>) {
        if let Some((j, _)) = self.jobs.lock().unwrap().get_mut(id) {
            j.elapsed_sec = now_f() - j.started;
            j.eta_sec = None;
            match r {
                Ok(v) => {
                    j.status = "done".into();
                    j.progress = 1.0;
                    j.percent = 100.0;
                    j.stage = "Finished".into();
                    j.result = Some(v);
                }
                Err(e) if e.to_string() == "cancelled" => {
                    j.status = "cancelled".into();
                    j.stage = "Cancelled".into();
                }
                Err(e) => {
                    j.status = "error".into();
                    j.stage = "Failed".into();
                    j.error = Some(e.to_string());
                }
            }
        }
    }
}

/// Map a stage's own 0..1 progress into a slice of the overall bar.
struct Stage<'a> {
    renders: &'a Renders,
    id: &'a str,
    a: f64,
    b: f64,
}
impl Stage<'_> {
    fn set(&self, f: f64, msg: &str) {
        self.renders.update(self.id, self.a + (self.b - self.a) * f.clamp(0.0, 1.0), msg);
    }
}

pub fn start(engine: Arc<Engine>, renders: Arc<Renders>, project_id: String, settings: crate::settings::Settings) -> Result<String> {
    let p = story::load(&project_id)?;
    let (id, flag) = renders.create(&format!("Story video: {}", p.title));
    let rid = id.clone();
    tokio::spawn(async move {
        let r = run(&engine, &renders, &rid, &flag, p, &settings).await;
        renders.finish(&rid, r);
    });
    Ok(id)
}

fn stable_seed(s: &str) -> u64 {
    s.bytes().fold(1469598103934665603u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211)) % 2_000_000_000
}

async fn run(engine: &Engine, renders: &Renders, id: &str, flag: &AtomicBool, mut p: Project,
             settings: &crate::settings::Settings) -> Result<serde_json::Value> {
    let cancelled = || flag.load(Ordering::Relaxed);
    let check = || if flag.load(Ordering::Relaxed) { Err(anyhow!("cancelled")) } else { Ok(()) };
    let dir = p.dir();
    for sub in ["audio", "faces", "scenes", "talk", "segments"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    let vertical = p.format == "shorts";
    let total_lines: usize = p.scenes.iter().map(|s| s.lines.len()).sum();
    if total_lines == 0 {
        return Err(anyhow!("The story has no lines yet. Write or paste a script first."));
    }

    // 1. voices ------------------------------------------------------------
    let st = Stage { renders, id, a: 0.0, b: 0.04 };
    st.set(0.0, "Checking character voices");
    let voices: Vec<serde_json::Value> = serde_json::from_value(engine.request("GET", "/voices", None).await?)?;
    let voice_exists = |vid: &str| voices.iter().any(|v| v["id"] == vid);
    let preset_voice = |preset: &str| voices.iter()
        .find(|v| v["preset"] == preset && v["kind"] == "designed").and_then(|v| v["id"].as_str()).map(String::from);
    let missing: Vec<String> = {
        let mut m: Vec<String> = p.characters.iter()
            .filter(|c| c.voice_id.as_deref().map(|v| !voice_exists(v)).unwrap_or(true))
            .filter(|c| preset_voice(&c.preset).is_none())
            .map(|c| c.preset.clone()).collect();
        m.sort();
        m.dedup();
        m
    };
    if !missing.is_empty() {
        engine.run_job("/voices/presets", json!({"keys": missing}), |f, m| st.set(f, m), &cancelled).await?;
    }
    let voices: Vec<serde_json::Value> = serde_json::from_value(engine.request("GET", "/voices", None).await?)?;
    for c in p.characters.iter_mut() {
        let ok = c.voice_id.as_deref().map(|v| voices.iter().any(|x| x["id"] == v)).unwrap_or(false);
        if !ok {
            c.voice_id = voices.iter().find(|v| v["preset"] == c.preset.as_str() && v["kind"] == "designed")
                .and_then(|v| v["id"].as_str()).map(String::from);
        }
    }
    story::save(&mut p)?;

    // 2. speech ------------------------------------------------------------
    let st = Stage { renders, id, a: 0.04, b: 0.26 };
    let mut done = 0usize;
    for si in 0..p.scenes.len() {
        for li in 0..p.scenes[si].lines.len() {
            check()?;
            let line = p.scenes[si].lines[li].clone();
            let out = dir.join("audio").join(format!("{}.wav", line.id));
            if line.audio.is_none() || !out.exists() {
                let voice = p.character(&line.character_id).and_then(|c| c.voice_id.clone());
                let base = done as f64 / total_lines as f64;
                let r = engine.run_job("/tts", json!({"text": line.text, "voice_id": voice, "language": p.language,
                    "mode": "auto", "out": out.to_string_lossy()}),
                    |f, _| st.set(base + f / total_lines as f64, &format!("Speaking line {} of {}", done + 1, total_lines)),
                    &cancelled).await?;
                let l = &mut p.scenes[si].lines[li];
                l.audio = Some(out.to_string_lossy().into());
                l.duration = r["duration"].as_f64().unwrap_or(0.0);
                l.talking_video = None; // new audio -> face must be re-animated
            } else if line.duration <= 0.0 {
                p.scenes[si].lines[li].duration = video::audio_duration(&out);
            }
            done += 1;
            st.set(done as f64 / total_lines as f64, &format!("Speaking line {done} of {total_lines}"));
        }
        story::save(&mut p)?;
    }

    // 3. faces --------------------------------------------------------------
    let st = Stage { renders, id, a: 0.26, b: 0.33 };
    let need: Vec<usize> = p.characters.iter().enumerate()
        .filter(|(_, c)| !c.is_narrator && c.face_image.as_ref().map(|f| !PathBuf::from(f).exists()).unwrap_or(true))
        .map(|(i, _)| i).collect();
    for (k, &ci) in need.iter().enumerate() {
        check()?;
        let c = p.characters[ci].clone();
        let presets = engine.request("GET", "/presets", None).await.unwrap_or_default();
        let look = if c.look.is_empty() { presets[&c.preset]["face"].as_str().unwrap_or("a Tamil person").to_string() } else { c.look.clone() };
        let out = dir.join("faces").join(format!("{}.png", c.id));
        engine.run_job("/images", json!({"prompt": look, "kind": "portrait", "style": p.style, "width": 1024,
            "height": 1024, "seed": stable_seed(&c.id), "out": out.to_string_lossy()}),
            |f, _| st.set((k as f64 + f) / need.len() as f64, &format!("Painting {}'s face", c.name)), &cancelled).await?;
        p.characters[ci].face_image = Some(out.to_string_lossy().into());
        story::save(&mut p)?;
    }

    // 4. scenes -------------------------------------------------------------
    let st = Stage { renders, id, a: 0.33, b: 0.43 };
    let (sw, sh) = if vertical { (768, 1344) } else { (1344, 768) };
    let n_sc = p.scenes.len();
    for si in 0..n_sc {
        check()?;
        let sc = p.scenes[si].clone();
        if sc.image.as_ref().map(|f| PathBuf::from(f).exists()).unwrap_or(false) {
            continue;
        }
        let prompt = if sc.visual.is_empty() { format!("{}, Tamil Nadu", sc.title) } else { sc.visual.clone() };
        let out = dir.join("scenes").join(format!("{}.png", sc.id));
        engine.run_job("/images", json!({"prompt": prompt, "kind": "scene", "style": p.style, "width": sw,
            "height": sh, "seed": stable_seed(&sc.id), "out": out.to_string_lossy()}),
            |f, _| st.set((si as f64 + f) / n_sc as f64, &format!("Painting scene {} of {}", si + 1, n_sc)),
            &cancelled).await?;
        p.scenes[si].image = Some(out.to_string_lossy().into());
        story::save(&mut p)?;
    }
    // optional cinematic clips (MiniMax H3 API)
    if p.use_minimax && !settings.minimax_api_key.is_empty() {
        let st = Stage { renders, id, a: 0.43, b: 0.47 };
        for si in 0..n_sc {
            check()?;
            if p.scenes[si].clip.as_ref().map(|c| PathBuf::from(c).exists()).unwrap_or(false) {
                continue;
            }
            st.set(si as f64 / n_sc as f64, &format!("MiniMax H3: animating scene {} of {}", si + 1, n_sc));
            let sc = p.scenes[si].clone();
            let out = dir.join("scenes").join(format!("{}.mp4", sc.id));
            match crate::minimax::scene_clip(&settings.minimax_api_key, &settings.minimax_model,
                sc.image.as_deref().unwrap_or_default(), &sc.visual, vertical, &out, &cancelled).await {
                Ok(()) => p.scenes[si].clip = Some(out.to_string_lossy().into()),
                Err(e) if e.to_string() == "cancelled" => return Err(e),
                Err(e) => renders.update(id, 0.0, &format!("MiniMax clip skipped: {e}")),
            }
            story::save(&mut p)?;
        }
    }

    // 5. thumbnail ------------------------------------------------------------
    let st = Stage { renders, id, a: 0.47, b: 0.49 };
    let thumb_bg = dir.join("thumbnail_bg.png");
    if !thumb_bg.exists() {
        check()?;
        let prompt = if p.thumbnail_prompt.is_empty() { p.scenes[0].visual.clone() } else { p.thumbnail_prompt.clone() };
        let _ = engine.run_job("/images", json!({"prompt": prompt, "kind": "thumbnail", "style": p.style,
            "width": if vertical { 768 } else { 1344 }, "height": if vertical { 1344 } else { 768 },
            "seed": stable_seed(&p.id), "out": thumb_bg.to_string_lossy()}), |f, _| st.set(f, "Painting the thumbnail"),
            &cancelled).await;
    }

    // 6. talking faces --------------------------------------------------------
    let st = Stage { renders, id, a: 0.49, b: 0.86 };
    let caps = engine.request("GET", "/capabilities", None).await.unwrap_or_default();
    if p.talking_faces && caps["talking_faces"].as_bool().unwrap_or(false) {
        let mut jobs = vec![];
        let mut idx = vec![];
        for (si, sc) in p.scenes.iter().enumerate() {
            for (li, l) in sc.lines.iter().enumerate() {
                let Some(c) = p.character(&l.character_id) else { continue };
                if c.is_narrator || l.audio.is_none() {
                    continue;
                }
                if l.talking_video.as_ref().map(|v| PathBuf::from(v).exists()).unwrap_or(false) {
                    continue;
                }
                let Some(face) = &c.face_image else { continue };
                let out = dir.join("talk").join(format!("{}.mp4", l.id));
                let expr = match l.emotion.to_lowercase().as_str() {
                    e if e.contains("excit") || e.contains("happy") || e.contains("angry") || e.contains("shout") => 1.3,
                    e if e.contains("sad") || e.contains("soft") || e.contains("whisper") || e.contains("calm") => 0.85,
                    _ => 1.05,
                };
                jobs.push(json!({"image": face, "audio": l.audio, "out": out.to_string_lossy(), "expression": expr}));
                idx.push((si, li));
            }
        }
        if !jobs.is_empty() {
            let r = engine.run_job("/talking/batch", json!({"jobs": jobs}), |f, m| st.set(f, m), &cancelled).await?;
            if let Some(paths) = r["paths"].as_array() {
                for (k, (si, li)) in idx.iter().enumerate() {
                    if let Some(pth) = paths.get(k).and_then(|v| v.as_str()) {
                        p.scenes[*si].lines[*li].talking_video = Some(pth.into());
                    }
                }
            }
            story::save(&mut p)?;
        }
    } else {
        st.set(1.0, "Talking faces off - using animated portraits");
    }

    // 7. compose ----------------------------------------------------------------
    let st = Stage { renders, id, a: 0.86, b: 0.97 };
    st.set(0.0, "Composing video");
    let layout = Layout::for_format(&p.format);
    let seg_dir = dir.join("segments");
    let assets = video::make_card_assets(&seg_dir, &layout)?;
    let nvenc = video::has_nvenc();
    let segments = video::plan(&p, 0.30);
    let n = segments.len();
    let outs: Vec<PathBuf> = (0..n).map(|i| seg_dir.join(format!("seg{i:04}.mp4"))).collect();
    let progress = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sem = Arc::new(tokio::sync::Semaphore::new(if nvenc { 3 } else { 2 }));
    let mut handles = vec![];
    let segments = Arc::new(segments);
    for i in 0..n {
        check()?;
        let permit = sem.clone().acquire_owned().await?;
        let (segs, out, sd, assets, prog) = (segments.clone(), outs[i].clone(), seg_dir.clone(), assets.clone(), progress.clone());
        let lay = Layout::for_format(&p.format);
        let subs = p.subtitles;
        handles.push(tokio::task::spawn_blocking(move || {
            let r = video::render_segment(&segs[i], &lay, &out, &sd, &assets, subs, nvenc);
            prog.fetch_add(1, Ordering::Relaxed);
            drop(permit);
            r
        }));
        let done = progress.load(Ordering::Relaxed);
        st.set(done as f64 / n as f64, &format!("Composing shot {} of {}", done.min(n - 1) + 1, n));
    }
    for h in handles {
        h.await??;
        let done = progress.load(Ordering::Relaxed);
        st.set(done as f64 / n as f64, &format!("Composed {} of {} shots", done, n));
    }
    check()?;
    let st = Stage { renders, id, a: 0.97, b: 1.0 };
    st.set(0.0, "Mixing music and final audio");
    let safe_title: String = p.title.chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect::<String>().trim().to_string();
    let out = dir.join(format!("{}.mp4", if safe_title.is_empty() { "story" } else { &safe_title }));
    video::finalize(&outs, p.music.as_deref().map(std::path::Path::new), p.music_volume, &out, &seg_dir)?;
    for o in &outs {
        let _ = std::fs::remove_file(o);
        let _ = std::fs::remove_file(o.with_extension("ass"));
    }
    // thumbnail with title text
    let thumb = dir.join("thumbnail.jpg");
    if thumb_bg.exists() {
        let _ = make_thumbnail(&thumb_bg, &p.title, &thumb, vertical);
    }
    p.output = Some(out.to_string_lossy().into());
    story::save(&mut p)?;
    Ok(json!({"path": out, "thumbnail": if thumb.exists() { Some(thumb) } else { None },
        "duration": video::audio_duration(&out)}))
}

fn make_thumbnail(bg: &std::path::Path, title: &str, out: &std::path::Path, vertical: bool) -> Result<()> {
    let (w, h) = if vertical { (1080, 1920) } else { (1280, 720) };
    let ass = out.with_extension("ass");
    let fs = if vertical { 120 } else { 96 };
    std::fs::write(&ass, format!(
        "[Script Info]\nPlayResX: {w}\nPlayResY: {h}\nScaledBorderAndShadow: yes\n\n[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: T,Nirmala UI,{fs},&H0000E6FF,&H000000FF,&H00000000,&HA0000000,-1,0,0,0,100,100,0,0,1,8,6,2,50,50,60,1\n\n[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:05.00,T,,0,0,0,,{}\n",
        title.replace('{', "(").replace('}', ")"))).ok();
    let mut cmd = std::process::Command::new(video::ffmpeg());
    cmd.current_dir(out.parent().unwrap()).args(["-y", "-v", "error", "-i"]).arg(bg)
        .args(["-vf", &format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},eq=contrast=1.08:saturation=1.2,ass='{}':shaping=complex",
            ass.file_name().unwrap().to_string_lossy()), "-frames:v", "1", "-q:v", "2"]).arg(out);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let _ = cmd.output();
    let _ = std::fs::remove_file(ass);
    Ok(())
}
