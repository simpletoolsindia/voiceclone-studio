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
pub struct StepInfo {
    pub key: String,
    pub label: String,
    /// pending | running | done | skipped
    pub status: String,
    pub progress: f64,
    /// estimated seconds (used to weight the overall bar and for the ETA)
    pub estimate: f64,
    pub started: Option<f64>,
    pub finished: Option<f64>,
    pub detail: String,
}

#[derive(Serialize, Clone)]
pub struct RenderJob {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub project_id: String,
    pub status: String,
    pub progress: f64,
    pub percent: f64,
    pub stage: String,
    pub started: f64,
    pub eta_sec: Option<f64>,
    pub elapsed_sec: f64,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub steps: Vec<StepInfo>,
    /// latest generated asset (image) to show as a live preview
    pub preview: Option<String>,
    /// recent milestones, newest last
    pub events: Vec<String>,
    /// what the job contains, e.g. [("Lines", "22"), ("Shots", "28")]
    pub facts: Vec<(String, String)>,
}

#[derive(Default)]
pub struct Renders {
    jobs: Mutex<HashMap<String, (RenderJob, Arc<AtomicBool>)>>,
}

fn now_f() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn fmt_secs(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s < 60 { format!("{s}s") } else { format!("{}m {:02}s", s / 60, s % 60) }
}

impl RenderJob {
    /// Overall progress weighted by each step's estimated duration.
    fn recompute(&mut self) {
        let total: f64 = self.steps.iter().filter(|s| s.status != "skipped").map(|s| s.estimate).sum();
        if total <= 0.0 {
            return;
        }
        let done: f64 = self.steps.iter().filter(|s| s.status != "skipped")
            .map(|s| s.estimate * if s.status == "done" { 1.0 } else { s.progress }).sum();
        self.progress = self.progress.max((done / total).clamp(0.0, 1.0));
        self.percent = (self.progress * 1000.0).round() / 10.0;
    }

    /// Remaining time: the running step extrapolates from its own speed; pending
    /// steps use their estimate scaled by how fast finished steps really ran.
    fn eta(&self, now: f64) -> Option<f64> {
        let (mut est_done, mut act_done) = (0.0, 0.0);
        for s in self.steps.iter().filter(|s| s.status == "done") {
            if let (Some(a), Some(b)) = (s.started, s.finished) {
                est_done += s.estimate;
                act_done += b - a;
            }
        }
        let k = if est_done > 5.0 { (act_done / est_done).clamp(0.3, 4.0) } else { 1.0 };
        let mut remaining = 0.0;
        let mut any = false;
        for s in &self.steps {
            match s.status.as_str() {
                "running" => {
                    any = true;
                    let el = s.started.map(|t| now - t).unwrap_or(0.0);
                    remaining += if s.progress > 0.08 && el > 3.0 {
                        el * (1.0 - s.progress) / s.progress
                    } else {
                        (s.estimate * k - el).max(s.estimate * k * (1.0 - s.progress) * 0.5)
                    };
                }
                "pending" => {
                    any = true;
                    remaining += s.estimate * k;
                }
                _ => {}
            }
        }
        if any { Some(remaining.max(1.0)) } else { None }
    }
}

impl Renders {
    pub fn list(&self) -> Vec<RenderJob> {
        let now = now_f();
        let mut v: Vec<RenderJob> = self.jobs.lock().unwrap().values().map(|(j, _)| {
            let mut j = j.clone();
            if j.status == "running" {
                j.elapsed_sec = now - j.started;
                j.eta_sec = j.eta(now);
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

    fn create(&self, title: &str, project_id: &str) -> (String, Arc<AtomicBool>) {
        let id = format!("r{}", story::new_id());
        let flag = Arc::new(AtomicBool::new(false));
        let job = RenderJob {
            id: id.clone(), kind: "story".into(), title: title.into(), project_id: project_id.into(),
            status: "running".into(), progress: 0.0, percent: 0.0, stage: "Preparing".into(), started: now_f(),
            eta_sec: None, elapsed_sec: 0.0, result: None, error: None, steps: vec![], preview: None,
            events: vec![], facts: vec![],
        };
        self.jobs.lock().unwrap().insert(id.clone(), (job, flag.clone()));
        (id, flag)
    }

    fn with<F: FnOnce(&mut RenderJob)>(&self, id: &str, f: F) {
        if let Some((j, _)) = self.jobs.lock().unwrap().get_mut(id) {
            f(j);
        }
    }

    fn set_plan(&self, id: &str, steps: Vec<(&str, &str, f64)>, facts: Vec<(String, String)>) {
        self.with(id, |j| {
            j.steps = steps.into_iter().map(|(k, l, e)| StepInfo {
                key: k.into(), label: l.into(), status: "pending".into(), progress: 0.0, estimate: e.max(0.5),
                started: None, finished: None, detail: String::new(),
            }).collect();
            j.facts = facts;
            let total: f64 = j.steps.iter().map(|s| s.estimate).sum();
            j.events.push(format!("Plan ready: {} steps, about {}", j.steps.len(), fmt_secs(total)));
        });
    }

    fn event(&self, id: &str, msg: String) {
        self.with(id, |j| {
            j.events.push(msg);
            if j.events.len() > 40 {
                j.events.remove(0);
            }
        });
    }

    pub fn set_preview(&self, id: &str, path: &str) {
        if std::path::Path::new(path).exists() {
            self.with(id, |j| j.preview = Some(path.to_string()));
        }
    }

    fn step_start(&self, id: &str, key: &str) {
        let now = now_f();
        self.with(id, |j| {
            let mut msgs = vec![];
            for s in j.steps.iter_mut() {
                if s.status == "running" && s.key != key {
                    s.status = "done".into();
                    s.progress = 1.0;
                    s.finished = Some(now);
                    msgs.push(format!("✓ {} ({})", s.label, fmt_secs(now - s.started.unwrap_or(now))));
                }
            }
            if let Some(s) = j.steps.iter_mut().find(|s| s.key == key) {
                if s.status != "running" {
                    s.status = "running".into();
                    s.started = Some(now);
                    msgs.push(format!("▶ {}", s.label));
                }
            }
            j.events.extend(msgs);
            j.recompute();
        });
    }

    fn step_progress(&self, id: &str, key: &str, f: f64, detail: &str) {
        self.with(id, |j| {
            if let Some(s) = j.steps.iter_mut().find(|s| s.key == key) {
                s.progress = s.progress.max(f.clamp(0.0, 1.0));
                if !detail.is_empty() {
                    s.detail = detail.into();
                }
            }
            if !detail.is_empty() {
                j.stage = detail.into();
            }
            j.recompute();
        });
    }

    /// Kept for warnings that are not tied to a step.
    fn update(&self, id: &str, _progress: f64, stage: &str) {
        self.event(id, format!("⚠ {stage}"));
    }

    fn finish(&self, id: &str, r: Result<serde_json::Value>) {
        let now = now_f();
        self.with(id, |j| {
            j.elapsed_sec = now - j.started;
            j.eta_sec = None;
            match r {
                Ok(v) => {
                    for s in j.steps.iter_mut() {
                        if s.status == "running" || s.status == "pending" {
                            if s.status == "running" {
                                s.finished = Some(now);
                            }
                            s.status = if s.status == "running" { "done".into() } else { "skipped".into() };
                            s.progress = 1.0;
                        }
                    }
                    j.status = "done".into();
                    j.progress = 1.0;
                    j.percent = 100.0;
                    j.stage = "Finished".into();
                    j.events.push(format!("🎉 Video ready in {}", fmt_secs(j.elapsed_sec)));
                    j.result = Some(v);
                }
                Err(e) if e.to_string() == "cancelled" => {
                    j.status = "cancelled".into();
                    j.stage = "Cancelled".into();
                    j.events.push("■ Cancelled".into());
                }
                Err(e) => {
                    j.status = "error".into();
                    j.stage = "Failed".into();
                    j.events.push(format!("✕ {e}"));
                    j.error = Some(e.to_string());
                }
            }
        });
    }
}

/// One named step of the render; its 0..1 progress feeds the overall bar.
struct Stage<'a> {
    renders: &'a Renders,
    id: &'a str,
    key: &'static str,
}
impl<'a> Stage<'a> {
    fn new(renders: &'a Renders, id: &'a str, key: &'static str) -> Self {
        renders.step_start(id, key);
        Stage { renders, id, key }
    }
    fn set(&self, f: f64, msg: &str) {
        self.renders.step_progress(self.id, self.key, f, msg);
    }
}

/// Estimate each step's duration from what actually needs doing (cached assets are free).
fn plan_steps(p: &Project, voices: &[serde_json::Value], caps: &serde_json::Value, settings: &crate::settings::Settings)
              -> (Vec<(&'static str, &'static str, f64)>, Vec<(String, String)>) {
    let exists = |o: &Option<String>| o.as_ref().map(|f| PathBuf::from(f).exists()).unwrap_or(false);
    let dialogue: Vec<&story::Line> = p.scenes.iter().flat_map(|s| s.lines.iter()).filter(|l| l.is_dialogue()).collect();
    let need_audio = dialogue.iter().filter(|l| !exists(&l.audio)).count() as f64;
    let have_preset = |k: &str| voices.iter().any(|v| v["preset"] == k && v["kind"] == "designed");
    let mut missing: Vec<&str> = p.characters.iter().filter(|c| !have_preset(&c.preset)).map(|c| c.preset.as_str()).collect();
    missing.sort();
    missing.dedup();
    let on_screen: Vec<&story::Character> = p.characters.iter().filter(|c| !c.is_narrator).collect();
    let need_faces = on_screen.iter().filter(|c| !exists(&c.face_image)).count() as f64;
    let need_scenes = p.scenes.iter().filter(|s| !exists(&s.image)).count() as f64;
    let need_plates = p.scenes.iter().filter(|s| !exists(&s.plate)
        && s.lines.iter().any(|l| on_screen.iter().any(|c| c.id == l.character_id))).count() as f64;
    let images = need_faces + need_scenes + need_plates;
    let cinematic = p.cinematic && caps["cinematic"].as_bool().unwrap_or(false);
    let inserts: Vec<&story::Line> = p.scenes.iter().flat_map(|s| s.lines.iter()).filter(|l| l.kind == "insert").collect();
    let spoken_on_screen = dialogue.iter().filter(|l| on_screen.iter().any(|c| c.id == l.character_id)).count() as f64;
    let need_shots = dialogue.iter().filter(|l| on_screen.iter().any(|c| c.id == l.character_id) && !exists(&l.shot_video)).count() as f64;
    let need_raw = dialogue.iter().filter(|l| on_screen.iter().any(|c| c.id == l.character_id) && !exists(&l.shot_video) && !exists(&l.clip)).count() as f64
        + inserts.iter().filter(|l| !exists(&l.clip)).count() as f64
        + p.scenes.iter().filter(|s| !exists(&s.clip)).count() as f64;
    let masks = on_screen.iter().filter(|c| c.face_image.as_ref().map(|f| !PathBuf::from(f).with_file_name(format!("{}_mask.png", c.id)).exists()).unwrap_or(true)).count() as f64;
    let segments = (p.scenes.len() + p.scenes.iter().map(|s| s.lines.len()).sum::<usize>() + 3) as f64;

    let mut steps: Vec<(&'static str, &'static str, f64)> = vec![
        ("voices", "Character voices", 1.0 + missing.len() as f64 * 40.0),
        ("speech", "Tamil speech", need_audio * 3.5 + if need_audio > 0.0 { 8.0 } else { 0.0 }),
        ("faces", "Character faces", need_faces * 18.0 + if need_faces > 0.0 { 20.0 } else { 0.0 }),
        ("scenes", "Scenes and empty sets", (need_scenes + need_plates) * 17.0 + if images > 0.0 && need_faces == 0.0 { 20.0 } else { 0.0 }),
    ];
    if p.use_minimax && !settings.minimax_api_key.is_empty() {
        steps.push(("minimax", "MiniMax H3 scene clips", p.scenes.len() as f64 * 150.0));
    }
    steps.push(("thumbnail", "YouTube thumbnail", if p.dir().join("thumbnail_bg.png").exists() { 0.5 } else { 17.0 }));
    if cinematic {
        steps.push(("cutouts", "Character cutouts", masks * 12.0 + if masks > 0.0 { 25.0 } else { 0.0 }));
        steps.push(("setup", "Shot keyframes and close-ups", need_shots * 0.6 + inserts.iter().filter(|l| !exists(&l.image)).count() as f64 * 17.0));
        steps.push(("animate", "Animating shots", need_raw * 20.0 + if need_raw > 0.0 { 45.0 } else { 0.0 }));
        steps.push(("lipsync", "Lip-sync", need_shots * 22.0 + if need_shots > 0.0 { 60.0 } else { 0.0 }));
    } else if p.talking_faces && caps["talking_faces"].as_bool().unwrap_or(false) {
        let need_talk = dialogue.iter().filter(|l| on_screen.iter().any(|c| c.id == l.character_id) && !exists(&l.talking_video)).count() as f64;
        steps.push(("talking", "Talking faces", need_talk * 60.0 + if need_talk > 0.0 { 30.0 } else { 0.0 }));
    }
    steps.push(("fx", "Cutouts, stickers and sound effects", 3.0 + if cinematic { 0.0 } else { masks * 12.0 }));
    steps.push(("compose", "Composing shots", segments * 1.2 + 5.0));
    steps.push(("finalize", "Music, loudness and final mix", 12.0));
    let facts = vec![
        ("Scenes".to_string(), p.scenes.len().to_string()),
        ("Spoken lines".to_string(), dialogue.len().to_string()),
        ("Characters".to_string(), on_screen.len().to_string()),
        (if cinematic { "Animated shots" } else { "On-screen lines" }.to_string(),
         if cinematic { (spoken_on_screen as usize + inserts.len() + p.scenes.len()).to_string() } else { (spoken_on_screen as usize).to_string() }),
        ("Mode".to_string(), if cinematic { "Cinematic".into() } else { "Fast".into() }),
        ("Format".to_string(), if p.format == "shorts" { "Shorts 9:16".into() } else { "YouTube 16:9".into() }),
    ];
    (steps, facts)
}

pub fn start(engine: Arc<Engine>, renders: Arc<Renders>, project_id: String, settings: crate::settings::Settings) -> Result<String> {
    let p = story::load(&project_id)?;
    let (id, flag) = renders.create(&format!("Story video: {}", p.title), &p.id);
    let rid = id.clone();
    tauri::async_runtime::spawn(async move {
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
    let pv = |v: &str| renders.set_preview(id, v);
    let dir = p.dir();
    for sub in ["audio", "faces", "scenes", "talk", "segments"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    {
        let voices: Vec<serde_json::Value> = serde_json::from_value(engine.request("GET", "/voices", None).await?).unwrap_or_default();
        let caps = engine.request("GET", "/capabilities", None).await.unwrap_or_default();
        let (steps, facts) = plan_steps(&p, &voices, &caps, settings);
        renders.set_plan(id, steps, facts);
    }
    let vertical = p.format == "shorts";
    let total_lines: usize = p.scenes.iter().map(|s| s.lines.iter().filter(|l| l.is_dialogue()).count()).sum();
    if total_lines == 0 {
        return Err(anyhow!("The story has no lines yet. Write or paste a script first."));
    }

    // 1. voices ------------------------------------------------------------
    let st = Stage::new(renders, id, "voices");
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
    let st = Stage::new(renders, id, "speech");
    let mut done = 0usize;
    for si in 0..p.scenes.len() {
        for li in 0..p.scenes[si].lines.len() {
            check()?;
            let line = p.scenes[si].lines[li].clone();
            if !line.is_dialogue() {
                continue;
            }
            let out = dir.join("audio").join(format!("{}.wav", line.id));
            let voice_of = p.character(&line.character_id).and_then(|c| c.voice_id.clone());
            let tts_key = fnv_hex(&format!("tts-v1|{:?}|{}|{}|{:?}|{}", voice_of, line.text, line.take, line.speed, p.language));
            if (line.audio.is_none() || !out.exists()) && cache_get("tts", &tts_key, "wav", &out) {
                let l = &mut p.scenes[si].lines[li];
                l.audio = Some(out.to_string_lossy().into());
                l.duration = video::audio_duration(&out);
                renders.event(id, format!("♻ Reused cached voice for line {}", done + 1));
            } else if line.audio.is_none() || !out.exists() {
                let voice = voice_of.clone();
                let base = done as f64 / total_lines as f64;
                let r = engine.run_job("/tts", json!({"text": line.text, "voice_id": voice, "language": p.language,
                    "mode": "auto", "out": out.to_string_lossy(), "take": line.take, "speed": line.speed}),
                    |f, _| st.set(base + f / total_lines as f64, &format!("Speaking line {} of {}", done + 1, total_lines)),
                    &cancelled).await?;
                cache_put("tts", &tts_key, "wav", &out);
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
    let st = Stage::new(renders, id, "faces");
    let need: Vec<usize> = p.characters.iter().enumerate()
        .filter(|(_, c)| !c.is_narrator && c.face_image.as_ref().map(|f| !PathBuf::from(f).exists()).unwrap_or(true))
        .map(|(i, _)| i).collect();
    for (k, &ci) in need.iter().enumerate() {
        check()?;
        let c = p.characters[ci].clone();
        let presets = engine.request("GET", "/presets", None).await.unwrap_or_default();
        let look = if c.look.is_empty() { presets[&c.preset]["face"].as_str().unwrap_or("a Tamil person").to_string() } else { c.look.clone() };
        let out = dir.join("faces").join(format!("{}.png", c.id));
        image_job(engine, json!({"prompt": look, "kind": "portrait", "style": p.style, "width": 1024,
            "height": 1024, "seed": stable_seed(&c.id), "out": out.to_string_lossy()}),
            |f, _| st.set((k as f64 + f) / need.len() as f64, &format!("Painting {}'s face", c.name)), &cancelled, &pv).await?;
        p.characters[ci].face_image = Some(out.to_string_lossy().into());
        story::save(&mut p)?;
    }

    // 4. scenes -------------------------------------------------------------
    let st = Stage::new(renders, id, "scenes");
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
        image_job(engine, json!({"prompt": prompt, "kind": "scene", "style": p.style, "width": sw,
            "height": sh, "seed": stable_seed(&sc.id), "out": out.to_string_lossy()}),
            |f, _| st.set((si as f64 + f) / n_sc as f64, &format!("Painting scene {} of {}", si + 1, n_sc)), &cancelled, &pv).await?;
        p.scenes[si].image = Some(out.to_string_lossy().into());
        story::save(&mut p)?;
    }
    // empty background plates for scenes where characters speak on screen
    let on_screen: Vec<String> = p.characters.iter().filter(|c| !c.is_narrator).map(|c| c.id.clone()).collect();
    for si in 0..n_sc {
        check()?;
        let sc = p.scenes[si].clone();
        let has_dialogue = sc.lines.iter().any(|l| on_screen.contains(&l.character_id));
        if !has_dialogue || sc.plate.as_ref().map(|f| PathBuf::from(f).exists()).unwrap_or(false) {
            continue;
        }
        let out = dir.join("scenes").join(format!("{}_plate.png", sc.id));
        image_job(engine, json!({"prompt": story::plate_prompt(&sc.visual, &sc.title), "kind": "plate",
            "style": p.style, "width": sw, "height": sh, "seed": stable_seed(&sc.id), "out": out.to_string_lossy()}),
            |f, _| st.set((si as f64 + f) / n_sc as f64, &format!("Painting empty set for scene {} of {}", si + 1, n_sc)), &cancelled, &pv).await?;
        p.scenes[si].plate = Some(out.to_string_lossy().into());
        story::save(&mut p)?;
    }
    // optional cinematic clips (MiniMax H3 API)
    if p.use_minimax && !settings.minimax_api_key.is_empty() {
        let st = Stage::new(renders, id, "minimax");
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
    let st = Stage::new(renders, id, "thumbnail");
    let thumb_bg = dir.join("thumbnail_bg.png");
    if !thumb_bg.exists() {
        check()?;
        let prompt = if p.thumbnail_prompt.is_empty() { p.scenes[0].visual.clone() } else { p.thumbnail_prompt.clone() };
        let _ = image_job(engine, json!({"prompt": prompt, "kind": "thumbnail", "style": p.style,
            "width": if vertical { 768 } else { 1344 }, "height": if vertical { 1344 } else { 768 },
            "seed": stable_seed(&p.id), "out": thumb_bg.to_string_lossy()}), |f, _| st.set(f, "Painting the thumbnail"), &cancelled, &pv).await;
    }

    // 6. animation: cinematic shots (LTX-Video + LatentSync) or talking faces ----
    let caps = engine.request("GET", "/capabilities", None).await.unwrap_or_default();
    if p.cinematic && caps["cinematic"].as_bool().unwrap_or(false) {
        cinematic(engine, renders, id, &mut p, &cancelled, &check).await?;
    } else {
        let st = Stage::new(renders, id, "talking");
        if p.talking_faces && caps["talking_faces"].as_bool().unwrap_or(false) {
            let mut jobs = vec![];
            let mut idx = vec![];
            for (si, sc) in p.scenes.iter().enumerate() {
                for (li, l) in sc.lines.iter().enumerate() {
                    let Some(c) = p.character(&l.character_id) else { continue };
                    if c.is_narrator || l.audio.is_none() || !l.is_dialogue() {
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
                let r = engine.run_job_pv("/talking/batch", json!({"jobs": jobs}), |f, m| st.set(f, m), &cancelled, &pv).await?;
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
    }

    // 7. character cutouts, emoji stickers, sound effects -------------------------
    let st = Stage::new(renders, id, "fx");
    let need_mask: Vec<(String, String, PathBuf)> = p.characters.iter()
        .filter(|c| !c.is_narrator)
        .filter_map(|c| c.face_image.as_ref().map(|f| (c.id.clone(), c.name.clone(), PathBuf::from(f))))
        .filter(|(cid, _, f)| f.exists() && !f.with_file_name(format!("{cid}_mask.png")).exists())
        .collect();
    for (k, (cid, name, face)) in need_mask.iter().enumerate() {
        check()?;
        let out = face.with_file_name(format!("{cid}_mask.png"));
        let r = engine.run_job("/matte", json!({"image": face, "out": out}),
            |f, _| st.set((k as f64 + f) / need_mask.len().max(1) as f64, &format!("Cutting out {name}")), &cancelled).await;
        if let Err(e) = r {
            if e.to_string() == "cancelled" { return Err(e); }
            renders.update(id, 0.0, &format!("Cutout for {name} failed - using a face card"));
        }
    }
    let assets_dir = crate::paths::data_dir().join("assets");
    let mut emoji_map: HashMap<String, PathBuf> = HashMap::new();
    for sc in &p.scenes {
        for l in &sc.lines {
            if let Some(e) = video::emoji_for(&l.emotion, &l.text) {
                if emoji_map.contains_key(e) { continue; }
                let hex: String = e.chars().map(|c| format!("{:x}", c as u32)).collect::<Vec<_>>().join("-");
                let out = assets_dir.join("emoji").join(format!("{hex}.png"));
                if out.exists() || engine.request("POST", "/emoji", Some(json!({"char": e, "out": out}))).await.is_ok() {
                    emoji_map.insert(e.to_string(), out);
                }
            }
        }
    }
    let sfx = video::make_sfx(&assets_dir.join("sfx"))?;

    // 8. compose ----------------------------------------------------------------
    let st = Stage::new(renders, id, "compose");
    st.set(0.0, "Composing video");
    let layout = Layout::for_format(&p.format);
    let seg_dir = dir.join("segments");
    let assets = video::make_card_assets(&seg_dir, &layout)?;
    let nvenc = video::has_nvenc();
    let segments = video::plan(&p, 0.30, &sfx, &|e: &str| emoji_map.get(e).cloned());
    let description = video::youtube_description(&p, &segments);
    let timeline = video::timeline(&segments);
    let subtitles = video::srt(&segments);
    let n = segments.len();
    let seg_cache = cache_root().join("segments");
    std::fs::create_dir_all(&seg_cache)?;
    let outs: Vec<PathBuf> = segments.iter()
        .map(|sg| seg_cache.join(format!("{}.mp4", fnv_hex(&video::segment_key(sg, &layout, p.subtitles, nvenc)))))
        .collect();
    let cached_segs = outs.iter().filter(|o| o.exists()).count();
    if cached_segs > 0 {
        renders.event(id, format!("♻ {cached_segs} of {n} shots unchanged - reusing them"));
    }
    let progress = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sem = Arc::new(tokio::sync::Semaphore::new(if nvenc { 3 } else { 2 }));
    let mut handles = vec![];
    let segments = Arc::new(segments);
    for i in 0..n {
        check()?;
        let permit = sem.clone().acquire_owned().await?;
        if outs[i].exists() {
            progress.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let (segs, out, sd, assets, prog) = (segments.clone(), outs[i].clone(), seg_cache.clone(), assets.clone(), progress.clone());
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
    let st = Stage::new(renders, id, "finalize");
    st.set(0.0, "Mixing music and final audio");
    let safe_title: String = p.title.chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect::<String>().trim().to_string();
    let version = p.versions.len() + 1;
    let base = if safe_title.is_empty() { "story".to_string() } else { safe_title.clone() };
    let out = dir.join(format!("{base} v{version}.mp4"));
    // channel intro / outro (prepared once, reused by every video)
    let mut all = vec![];
    let mut timeline = timeline;
    if let Some(b) = prepared_bumper(&settings.intro_video, &layout, nvenc) {
        // shift the editable timeline by the intro length so "fix at 1:22" matches the video
        let off = video::audio_duration(&b);
        for it in timeline.iter_mut() {
            for k in ["start", "end"] {
                let v = it[k].as_f64().unwrap_or(0.0) + off;
                it[k] = json!((v * 100.0).round() / 100.0);
            }
        }
        all.push(b);
    }
    all.extend(outs.iter().cloned());
    if let Some(b) = prepared_bumper(&settings.outro_video, &layout, nvenc) {
        all.push(b);
    }
    video::finalize(&all, p.music.as_deref().map(std::path::Path::new), p.music_volume, &out, &seg_dir)?;
    // thumbnail with title text
    let thumb = dir.join("thumbnail.jpg");
    if thumb_bg.exists() {
        let _ = make_thumbnail(&thumb_bg, &p.title, &thumb, vertical);
    }
    let desc_path = dir.join("description.txt");
    std::fs::write(&desc_path, &description)?;
    std::fs::write(dir.join("subtitles.srt"), &subtitles)?;
    p.output = Some(out.to_string_lossy().into());
    p.description = description.clone();
    p.timeline = timeline;
    let note = if p.pending_note.is_empty() { if version == 1 { "First render".to_string() } else { "Re-render".to_string() } } else { std::mem::take(&mut p.pending_note) };
    p.versions.push(json!({"version": version, "path": out, "time": story::now(), "note": note}));
    for sc in p.scenes.iter_mut() {
        for l in sc.lines.iter_mut() {
            l.sync_boost = false;
        }
    }
    story::save(&mut p)?;
    Ok(json!({"path": out, "thumbnail": if thumb.exists() { Some(thumb) } else { None },
        "description": description, "duration": video::audio_duration(&out)}))
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


/// Cinematic mode: compose a keyframe per shot, animate all shots with LTX-Video,
/// then lip-sync every dialogue shot with LatentSync. Assets are cached per line.
async fn cinematic(engine: &Engine, renders: &Renders, id: &str, p: &mut Project,
                   cancelled: &(dyn Fn() -> bool + Sync), check: &(dyn Fn() -> Result<()> + Sync)) -> Result<()> {
    let pv = |v: &str| renders.set_preview(id, v);
    let dir = p.dir();
    for sub in ["shots", "keyframes"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    let vertical = p.format == "shorts";
    let (vw, vh) = if vertical { (576i64, 1024i64) } else { (1024i64, 576i64) };
    let layout = Layout::for_format(&p.format);
    let k = vw as f64 / layout.w as f64;
    let presets = engine.request("GET", "/presets", None).await.unwrap_or_default();
    let look = |c: &story::Character| -> String {
        if c.look.is_empty() { presets[&c.preset]["face"].as_str().unwrap_or("a Tamil person").to_string() } else { c.look.clone() }
    };

    // 6a. cutouts for every on-screen character
    let st = Stage::new(renders, id, "cutouts");
    let chars: Vec<story::Character> = p.characters.iter().filter(|c| !c.is_narrator && c.face_image.is_some()).cloned().collect();
    for (n, c) in chars.iter().enumerate() {
        check()?;
        let face = PathBuf::from(c.face_image.as_ref().unwrap());
        let mask = face.with_file_name(format!("{}_mask.png", c.id));
        if !mask.exists() {
            engine.run_job("/matte", json!({"image": face, "out": mask}),
                |f, _| st.set((n as f64 + f) / chars.len().max(1) as f64, &format!("Cutting out {}", c.name)), cancelled).await?;
        }
    }

    // 6b. keyframes (dialogue), establishing shots and insert images
    let st = Stage::new(renders, id, "setup");
    struct ShotJob { si: usize, li: Option<usize>, image: PathBuf, prompt: String, seconds: f64, out: PathBuf, seed: u64 }
    let mut shots: Vec<ShotJob> = vec![];
    let n_sc = p.scenes.len();
    for si in 0..n_sc {
        check()?;
        let sc = p.scenes[si].clone();
        st.set(si as f64 / n_sc as f64, &format!("Setting up the shots of scene {} of {}", si + 1, n_sc));
        let setting = story::plate_prompt(&sc.visual, &sc.title).split(". The place").next().unwrap_or("").to_string();
        if sc.clip.as_ref().map(|c| !PathBuf::from(c).exists()).unwrap_or(true) {
            if let Some(img) = sc.image.clone() {
                shots.push(ShotJob { si, li: None, image: PathBuf::from(img), seconds: 5.0, seed: stable_seed(&sc.id),
                    prompt: format!("{}. Slow cinematic camera push-in, gentle living motion in the scene (rain, flickering light, swaying leaves, breathing people), 3D animated film, smooth natural motion.", sc.visual),
                    out: dir.join("shots").join(format!("{}_scene.mp4", sc.id)) });
            }
        }
        let mut dialog_count = 0usize;
        let mut last_speaker: Option<String> = None;
        for li in 0..sc.lines.len() {
            let l = sc.lines[li].clone();
            if l.kind == "insert" {
                if l.clip.as_ref().map(|c| PathBuf::from(c).exists()).unwrap_or(false) { continue; }
                let img = dir.join("keyframes").join(format!("{}.png", l.id));
                if !img.exists() {
                    let (w, h) = if vertical { (768, 1344) } else { (1344, 768) };
                    image_job(engine, json!({"prompt": l.text, "kind": "scene", "style": p.style, "width": w, "height": h,
                        "seed": stable_seed(&l.id), "out": img}), |_, _| {}, cancelled, &pv).await?;
                }
                p.scenes[si].lines[li].image = Some(img.to_string_lossy().into());
                shots.push(ShotJob { si, li: Some(li), image: img, seconds: 3.0, seed: stable_seed(&l.id) + l.take as u64 * 7919,
                    prompt: format!("{}. Subtle cinematic motion, slow push-in, shallow depth of field, 3D animated film, smooth natural motion.", l.text),
                    out: dir.join("shots").join(format!("{}_insert.mp4", l.id)) });
                continue;
            }
            if !l.is_dialogue() { continue; }
            let Some(c) = p.character(&l.character_id).cloned() else { continue };
            if c.is_narrator || c.face_image.is_none() || l.audio.is_none() { dialog_count = 0; continue; }
            if l.shot_video.as_ref().map(|v| PathBuf::from(v).exists()).unwrap_or(false) {
                dialog_count += 1;
                last_speaker = Some(c.id.clone());
                continue;
            }
            let other = |j: usize| sc.lines.get(j).and_then(|x| p.character(&x.character_id))
                .filter(|x| !x.is_narrator && x.face_image.is_some() && x.id != c.id).cloned();
            let listener = last_speaker.as_ref().filter(|s| **s != c.id).and_then(|s| p.character(s).cloned())
                .or_else(|| other(li + 1));
            let shot = if listener.is_some() && dialog_count % 4 == 0 { video::Shot::TwoShot } else { video::Shot::CloseUp };
            let idx = chars.iter().position(|x| x.id == c.id).unwrap_or(0);
            let right = idx % 2 == 1;
            let mut actors = vec![];
            if let (video::Shot::TwoShot, Some(lis)) = (shot, &listener) {
                let (x, y, sz) = video::actor_box(&layout, shot, !right, true);
                let face = PathBuf::from(lis.face_image.as_ref().unwrap());
                actors.push(json!({"image": face, "mask": face.with_file_name(format!("{}_mask.png", lis.id)),
                    "x": (x as f64 * k) as i64, "y": (y as f64 * k) as i64, "size": (sz as f64 * k) as i64, "dim": true}));
            }
            let (x, y, sz) = video::actor_box(&layout, shot, right, false);
            let face = PathBuf::from(c.face_image.as_ref().unwrap());
            actors.push(json!({"image": face, "mask": face.with_file_name(format!("{}_mask.png", c.id)),
                "x": (x as f64 * k) as i64, "y": (y as f64 * k) as i64, "size": (sz as f64 * k) as i64}));
            let plate = sc.plate.clone().or(sc.image.clone()).unwrap_or_default();
            let kf = dir.join("keyframes").join(format!("{}.png", l.id));
            engine.request("POST", "/animate/keyframe", Some(json!({"plate": plate, "actors": actors,
                "width": vw, "height": vh, "out": kf}))).await?;
            let action = if l.action.is_empty() { "talks with natural, expressive hand gestures".to_string() } else { l.action.clone() };
            let emotion = if l.emotion.is_empty() { "warm".to_string() } else { l.emotion.to_lowercase() };
            let listen = match (&listener, shot) {
                (Some(lis), video::Shot::TwoShot) => format!(" Beside them, {} listens attentively, nodding and reacting.", look(lis)),
                _ => String::new(),
            };
            shots.push(ShotJob { si, li: Some(li), image: kf, seconds: (l.duration + 0.3).min(4.0), seed: stable_seed(&l.id) + l.take as u64 * 7919,
                prompt: format!("{}, {}, speaking in a {} mood while mostly facing the camera: the mouth opens and closes as they talk, natural jaw and lip movement, blinking, gentle head movements, subtle body motion.{} Setting: {}. Only the people already in the picture appear; nobody new enters the frame. Steady camera with a very slow push-in, smooth natural motion, consistent faces, 3D animated film.",
                    look(&c), action, emotion, listen, setting),
                out: dir.join("shots").join(format!("{}_raw.mp4", l.id)) });
            dialog_count += 1;
            last_speaker = Some(c.id.clone());
        }
    }
    story::save(p)?;

    // 6c. animate every shot in one batch (model loads once)
    let st = Stage::new(renders, id, "animate");
    // reuse identical shots (same keyframe pixels, prompt, seed and length) from the cache
    let mut misses = vec![];
    for s in shots.iter() {
        let key = fnv_hex(&format!("ltx2b-0.9.8|{}|{}|{}|{}|{vw}x{vh}", file_hash(&s.image), s.prompt, s.seed, s.seconds));
        if cache_get("clips", &key, "mp4", &s.out) {
            match s.li {
                None => p.scenes[s.si].clip = Some(s.out.to_string_lossy().into()),
                Some(li) => p.scenes[s.si].lines[li].clip = Some(s.out.to_string_lossy().into()),
            }
        } else {
            misses.push((key, s));
        }
    }
    if shots.len() > misses.len() {
        renders.event(id, format!("♻ Reused {} cached animated shot(s)", shots.len() - misses.len()));
    }
    if !misses.is_empty() {
        check()?;
        let body: Vec<serde_json::Value> = misses.iter().map(|(_, s)| json!({"image": s.image, "prompt": s.prompt,
            "seconds": s.seconds, "width": vw, "height": vh, "out": s.out, "seed": s.seed})).collect();
        let r = engine.run_job_pv("/animate/shots", json!({"shots": body}), |f, m| st.set(f, m), cancelled, &pv).await?;
        let paths = r["paths"].as_array().cloned().unwrap_or_default();
        for (j, (key, s)) in misses.iter().enumerate() {
            let Some(path) = paths.get(j).and_then(|v| v.as_str()) else { continue };
            cache_put("clips", key, "mp4", std::path::Path::new(path));
            match s.li {
                None => p.scenes[s.si].clip = Some(path.into()),
                Some(li) => p.scenes[s.si].lines[li].clip = Some(path.into()),
            }
        }
        story::save(p)?;
    }

    // 6d. lip-sync every dialogue shot to its Tamil line
    let st = Stage::new(renders, id, "lipsync");
    let mut jobs = vec![];
    let mut hits: Vec<(usize, usize, String)> = vec![];
    let mut keys: Vec<String> = vec![];
    let mut idx = vec![];
    for (si, sc) in p.scenes.iter().enumerate() {
        for (li, l) in sc.lines.iter().enumerate() {
            if !l.is_dialogue() || l.shot_video.as_ref().map(|v| PathBuf::from(v).exists()).unwrap_or(false) { continue; }
            let (Some(clip), Some(audio)) = (&l.clip, &l.audio) else { continue };
            if !PathBuf::from(clip).exists() { continue; }
            let out = dir.join("shots").join(format!("{}_sync.mp4", l.id));
            let (steps, guidance) = if l.sync_boost { (32, 2.2) } else { (20, 1.5) };
            let key = fnv_hex(&format!("ls1.5|{}|{}|{steps}|{guidance}", file_hash(std::path::Path::new(clip)), file_hash(std::path::Path::new(audio))));
            if cache_get("sync", &key, "mp4", &out) {
                hits.push((si, li, out.to_string_lossy().to_string()));
                continue;
            }
            jobs.push(json!({"video": clip, "audio": audio, "out": out, "steps": steps, "guidance": guidance,
                "image": dir.join("keyframes").join(format!("{}.png", l.id))}));
            idx.push((si, li));
            keys.push(key);
        }
    }
    for (si, li, path) in &hits {
        p.scenes[*si].lines[*li].shot_video = Some(path.clone());
    }
    if !hits.is_empty() {
        renders.event(id, format!("♻ Reused {} cached lip-sync(s)", hits.len()));
        story::save(p)?;
    }
    if !jobs.is_empty() {
        check()?;
        let r = engine.run_job_pv("/lipsync", json!({"jobs": jobs}), |f, m| st.set(f, m), cancelled, &pv).await?;
        let paths = r["paths"].as_array().cloned().unwrap_or_default();
        for (j, (si, li)) in idx.iter().enumerate() {
            if let Some(pth) = paths.get(j).and_then(|v| v.as_str()) {
                cache_put("sync", &keys[j], "mp4", std::path::Path::new(pth));
                p.scenes[*si].lines[*li].shot_video = Some(pth.into());
            }
        }
        story::save(p)?;
    }
    Ok(())
}


// ---------------------------------------------------------------- content-addressed cache
//
// Everything expensive is cached by the content of its inputs in
// %APPDATA%\VoiceCloneStudio\cache\<kind>\<key>.<ext>, shared by all stories:
// identical voice lines, faces, scenes, animated shots, lip-syncs and composed
// shots are never generated twice (recurring characters stay identical too).

pub const CACHE_KINDS: [&str; 7] = ["tts", "images", "clips", "sync", "segments", "intro", "t5"];

fn fnv_hex(s: &str) -> String {
    let h = s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("{h:016x}")
}

/// Hash of a file's bytes (stable across re-generation with identical content).
fn file_hash(p: &std::path::Path) -> String {
    match std::fs::read(p) {
        Ok(b) => {
            let h = b.iter().fold(0xcbf29ce484222325u64, |h, x| (h ^ *x as u64).wrapping_mul(0x100000001b3));
            format!("{h:016x}-{}", b.len())
        }
        Err(_) => "missing".into(),
    }
}

pub fn cache_root() -> PathBuf {
    crate::paths::data_dir().join("cache")
}

fn cache_path(kind: &str, key: &str, ext: &str) -> PathBuf {
    let d = cache_root().join(kind);
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("{key}.{ext}"))
}

/// Copy a cached result to `dest`; true on a hit.
fn cache_get(kind: &str, key: &str, ext: &str, dest: &std::path::Path) -> bool {
    let c = cache_path(kind, key, ext);
    if !c.exists() || std::fs::metadata(&c).map(|m| m.len() == 0).unwrap_or(true) {
        return false;
    }
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    c == dest || std::fs::copy(&c, dest).is_ok()
}

fn cache_put(kind: &str, key: &str, ext: &str, src: &std::path::Path) {
    if src.exists() {
        let _ = std::fs::copy(src, cache_path(kind, key, ext));
    }
}

/// Size of the cache in bytes, per kind.
pub fn cache_stats() -> serde_json::Value {
    let mut total = 0u64;
    let mut kinds = serde_json::Map::new();
    for k in CACHE_KINDS {
        let mut n = 0u64;
        let mut files = 0u64;
        for e in std::fs::read_dir(cache_root().join(k)).into_iter().flatten().flatten() {
            if let Ok(m) = e.metadata() {
                n += m.len();
                files += 1;
            }
        }
        total += n;
        kinds.insert(k.into(), json!({"bytes": n, "files": files}));
    }
    json!({"total_bytes": total, "kinds": kinds, "path": cache_root()})
}

pub fn cache_clear() -> Result<()> {
    let r = cache_root();
    if r.exists() {
        std::fs::remove_dir_all(&r)?;
    }
    Ok(())
}

/// /images job with the content cache in front of it.
async fn image_job<F: FnMut(f64, &str)>(engine: &Engine, body: serde_json::Value, on_progress: F,
                                         cancelled: &(dyn Fn() -> bool + Sync), preview: &(dyn Fn(&str) + Sync)) -> Result<serde_json::Value> {
    let key = fnv_hex(&format!("img-v1|{}|{}|{}|{}|{}|{}", body["prompt"], body["kind"], body["style"], body["width"], body["height"], body["seed"]));
    let out = PathBuf::from(body["out"].as_str().unwrap_or_default());
    if cache_get("images", &key, "png", &out) {
        preview(&out.to_string_lossy());
        return Ok(json!({"path": out, "cached": true}));
    }
    let r = engine.run_job_pv("/images", body, on_progress, cancelled, preview).await?;
    cache_put("images", &key, "png", &out);
    Ok(r)
}

/// Normalise a channel intro/outro once per (file, format) to the segment format.
fn prepared_bumper(src: &str, layout: &Layout, nvenc: bool) -> Option<PathBuf> {
    let srcp = PathBuf::from(src);
    if src.is_empty() || !srcp.exists() {
        return None;
    }
    let sig = std::fs::metadata(&srcp).ok().map(|m| format!("{}-{:?}", m.len(), m.modified().ok()))?;
    let key = fnv_hex(&format!("bumper-v1|{src}|{sig}|{}x{}", layout.w, layout.h));
    let out = cache_path("intro", &key, "mp4");
    if !out.exists() && video::normalize_clip(&srcp, &out, layout, nvenc).is_err() {
        return None;
    }
    Some(out)
}
