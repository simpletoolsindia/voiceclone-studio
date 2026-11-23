//! Story video compositor (ffmpeg, NVENC when available).
//!
//! Every dialogue line becomes one segment: a Ken-Burns scene background,
//! softly blurred, with the speaking character's animated face in a large
//! rounded card (sides alternate per character so conversations read
//! naturally), a name tag, and Tamil subtitles rendered by libass. Narration
//! lines show the full scene. Scene cards and a title card add structure;
//! segments are concatenated without re-encoding, then background music is
//! mixed under the voices with ducking and the result is loudness-normalised
//! to YouTube's -14 LUFS.

use crate::story::{Character, Project};
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const FPS: u32 = 30;

pub struct Layout {
    pub w: u32,
    pub h: u32,
    pub card: u32,
}

impl Layout {
    pub fn for_format(format: &str) -> Self {
        if format == "shorts" {
            Layout { w: 1080, h: 1920, card: 900 }
        } else {
            Layout { w: 1920, h: 1080, card: 820 }
        }
    }
    fn vertical(&self) -> bool {
        self.h > self.w
    }
}

pub struct Segment {
    pub kind: SegKind,
    pub background: Option<PathBuf>,     // scene image
    pub clip: Option<PathBuf>,           // optional scene video clip (MiniMax H3)
    pub face_video: Option<PathBuf>,     // talking head
    pub face_image: Option<PathBuf>,     // still fallback
    pub audio: Option<PathBuf>,
    pub duration: f64,
    pub speaker: String,
    pub text: String,
    pub side_right: bool,
    pub accent: String,                  // ASS colour &HBBGGRR
    pub fade_in: bool,
    pub fade_out: bool,
    pub motion: u32,
}

#[derive(PartialEq)]
pub enum SegKind {
    Title { title: String, subtitle: String },
    SceneCard { title: String },
    Dialogue,
    Narration,
}

pub fn ffmpeg() -> String {
    std::env::var("FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

fn run(cmd: &mut Command) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().context("ffmpeg not found - install it (winget install Gyan.FFmpeg)")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(6).collect();
        return Err(anyhow!("ffmpeg failed: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" | ")));
    }
    Ok(())
}

pub fn has_nvenc() -> bool {
    let mut c = Command::new(ffmpeg());
    c.args(["-hide_banner", "-v", "error", "-f", "lavfi", "-i", "color=black:s=256x256:d=0.1",
        "-c:v", "h264_nvenc", "-f", "null", "-"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.output().map(|o| o.status.success()).unwrap_or(false)
}

fn venc(nvenc: bool) -> Vec<&'static str> {
    if nvenc {
        vec!["-c:v", "h264_nvenc", "-preset", "p5", "-rc", "vbr", "-cq", "20", "-b:v", "0", "-profile:v", "high"]
    } else {
        vec!["-c:v", "libx264", "-preset", "medium", "-crf", "19"]
    }
}

pub fn audio_duration(path: &Path) -> f64 {
    let mut c = Command::new(ffmpeg().replace("ffmpeg", "ffprobe"));
    c.args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.output().ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok()).unwrap_or(0.0)
}

/// Accent colours per character (ASS uses &HBBGGRR).
pub fn accent_for(i: usize) -> String {
    const C: [&str; 8] = ["&H3CC8FF", "&HFF9E4A", "&H7BE07B", "&HB06CFF", "&H5A5AFF", "&HE0D040", "&H40A0FF", "&HFF70C0"];
    C[i % C.len()].into()
}

fn ass_time(t: f64) -> String {
    let t = t.max(0.0);
    let h = (t / 3600.0) as u32;
    let m = ((t % 3600.0) / 60.0) as u32;
    let s = t % 60.0;
    format!("{h}:{m:02}:{s:05.2}")
}

fn ass_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('{', "(").replace('}', ")").replace('\n', "\\N")
}

/// Split a line into caption chunks (~42 chars) shown in sequence, timed by length.
fn caption_chunks(text: &str, max: usize) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    for w in text.split_whitespace() {
        if cur.chars().count() + w.chars().count() + 1 > max && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(w);
        if w.ends_with(['.', '?', '!']) && cur.chars().count() > max / 2 {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn write_ass(path: &Path, l: &Layout, seg: &Segment, subtitles: bool) -> Result<()> {
    let vertical = l.vertical();
    let font = "Nirmala UI";
    let sub_size = if vertical { 64 } else { 58 };
    let mut s = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {w}\nPlayResY: {h}\nWrapStyle: 0\nScaledBorderAndShadow: yes\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Sub,{font},{sub_size},&H00FFFFFF,&H000000FF,&H00141414,&H96000000,-1,0,0,0,100,100,0,0,1,4,2,2,90,90,{mv},1\n\
Style: Name,{font},44,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,3,14,0,5,0,0,0,1\n\
Style: Title,{font},{ts},&H00FFFFFF,&H000000FF,&H00101010,&H96000000,-1,0,0,0,100,100,0,0,1,6,4,5,80,80,0,1\n\
Style: Small,{font},{ss},&H00E8E8E8,&H000000FF,&H00101010,&H96000000,0,0,0,0,100,100,0,0,1,3,2,5,80,80,0,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        w = l.w, h = l.h, mv = if vertical { 260 } else { 70 },
        ts = if vertical { 96 } else { 110 }, ss = if vertical { 48 } else { 52 }
    );
    let end = ass_time(seg.duration);
    match &seg.kind {
        SegKind::Title { title, subtitle } => {
            s += &format!("Dialogue: 0,0:00:00.30,{end},Title,,0,0,0,,{{\\pos({},{})\\fad(600,500)\\blur1}}{}\n",
                l.w / 2, l.h / 2 - 40, ass_escape(title));
            if !subtitle.is_empty() {
                s += &format!("Dialogue: 0,0:00:01.00,{end},Small,,0,0,0,,{{\\pos({},{})\\fad(600,500)}}{}\n",
                    l.w / 2, l.h / 2 + 80, ass_escape(subtitle));
            }
        }
        SegKind::SceneCard { title } => {
            s += &format!("Dialogue: 0,0:00:00.20,{end},Title,,0,0,0,,{{\\pos({},{})\\fad(400,400)\\fs{}}}{}\n",
                l.w / 2, l.h / 2, if vertical { 80 } else { 88 }, ass_escape(title));
        }
        SegKind::Dialogue | SegKind::Narration => {
            if seg.kind == SegKind::Dialogue && !seg.speaker.is_empty() {
                let (nx, ny) = name_pos(l, seg);
                s += &format!("Dialogue: 1,0:00:00.15,{end},Name,,0,0,0,,{{\\pos({nx},{ny})\\3c{}\\fad(200,150)}}{}\n",
                    seg.accent, ass_escape(&seg.speaker));
            }
            if subtitles && !seg.text.is_empty() {
                let chunks = caption_chunks(&seg.text, if vertical { 30 } else { 44 });
                let total: usize = chunks.iter().map(|c| c.chars().count()).sum::<usize>().max(1);
                let speak = (seg.duration - 0.35).max(0.5);
                let mut t = 0.05;
                for c in chunks {
                    let d = speak * c.chars().count() as f64 / total as f64;
                    s += &format!("Dialogue: 0,{},{},Sub,,0,0,0,,{{\\fad(120,80)}}{}\n",
                        ass_time(t), ass_time(t + d), ass_escape(&c));
                    t += d;
                }
            }
        }
    }
    std::fs::write(path, s)?;
    Ok(())
}

fn card_pos(l: &Layout, seg: &Segment) -> (i64, i64) {
    let c = l.card as i64;
    if l.vertical() {
        ((l.w as i64 - c) / 2, 300)
    } else {
        let margin = 150i64;
        let x = if seg.side_right { l.w as i64 - c - margin } else { margin };
        (x, (l.h as i64 - c) / 2 - 70)
    }
}

fn name_pos(l: &Layout, seg: &Segment) -> (i64, i64) {
    let (x, y) = card_pos(l, seg);
    (x + l.card as i64 / 2, y + l.card as i64 + 40)
}

/// Pre-render the rounded-corner mask and glow frame used by face cards.
pub fn make_card_assets(dir: &Path, l: &Layout) -> Result<(PathBuf, PathBuf)> {
    let c = l.card;
    let mask = dir.join("card_mask.png");
    let glow = dir.join("card_glow.png");
    let r = 64;
    let expr = |w: u32, r: u32| format!(
        "if(gt(abs(X-{h})\\,{h}-{r})*gt(abs(Y-{h})\\,{h}-{r})\\,if(lte(hypot(abs(X-{h})-({h}-{r})\\,abs(Y-{h})-({h}-{r}))\\,{r})\\,255\\,0)\\,255)",
        h = w / 2, r = r);
    if !mask.exists() {
        run(Command::new(ffmpeg()).args(["-y", "-v", "error", "-f", "lavfi", "-i", &format!("color=black:s={c}x{c}"),
            "-vf", &format!("format=gray,geq=lum='{}'", expr(c, r)), "-frames:v", "1"]).arg(&mask))?;
    }
    if !glow.exists() {
        let g = c + 40;
        run(Command::new(ffmpeg()).args(["-y", "-v", "error", "-f", "lavfi", "-i", &format!("color=white:s={g}x{g}"),
            "-f", "lavfi", "-i", &format!("color=black:s={g}x{g}"),
            "-filter_complex", &format!("[1]format=gray,geq=lum='{}',gblur=sigma=9[a];[0]format=rgba[c];[c][a]alphamerge,colorchannelmixer=aa=0.55",
                expr(g, r + 20)),
            "-frames:v", "1"]).arg(&glow))?;
    }
    Ok((mask, glow))
}

fn ken_burns(l: &Layout, frames: u32, motion: u32) -> String {
    let n = frames.max(1);
    let (z, x, y): (String, String, String) = match motion % 4 {
        0 => (format!("1+0.12*on/{n}"), "iw/2-(iw/zoom/2)".into(), "ih/2-(ih/zoom/2)".into()),
        1 => (format!("1.12-0.12*on/{n}"), "iw/2-(iw/zoom/2)".into(), "ih/2-(ih/zoom/2)".into()),
        2 => ("1.10".into(), format!("(iw-iw/zoom)*on/{n}"), "ih/2-(ih/zoom/2)".into()),
        _ => ("1.10".into(), format!("(iw-iw/zoom)*(1-on/{n})"), "ih/2-(ih/zoom/2)".into()),
    };
    format!("scale={sw}:{sh}:force_original_aspect_ratio=increase,crop={sw}:{sh},zoompan=z='{z}':x='{x}':y='{y}':d={n}:s={w}x{h}:fps={FPS}",
        sw = l.w * 3 / 2, sh = l.h * 3 / 2, w = l.w, h = l.h)
}

/// Render one segment to `out` (H.264 + AAC 48 kHz stereo).
pub fn render_segment(seg: &Segment, l: &Layout, out: &Path, work: &Path, assets: &(PathBuf, PathBuf),
                      subtitles: bool, nvenc: bool) -> Result<()> {
    let d = seg.duration.max(0.5);
    let frames = (d * FPS as f64).ceil() as u32;
    let ass = out.with_extension("ass");
    write_ass(&ass, l, seg, subtitles)?;
    let ass_name = ass.file_name().unwrap().to_string_lossy().to_string();
    let mut cmd = Command::new(ffmpeg());
    cmd.current_dir(work).args(["-y", "-v", "error"]);
    let mut inputs = 0usize;

    // background (input 0)
    let bg_chain = if let Some(clip) = &seg.clip {
        cmd.args(["-stream_loop", "-1", "-t", &format!("{d:.3}"), "-i"]).arg(clip);
        format!("[0:v]fps={FPS},scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1",
            w = l.w, h = l.h)
    } else if let Some(bg) = &seg.background {
        cmd.arg("-i").arg(bg);
        ken_burns(l, frames, seg.motion).replacen("scale=", "[0:v]scale=", 1)
    } else {
        cmd.args(["-f", "lavfi", "-i", &format!("color=c=0x15121c:s={}x{}:d={d:.3}:r={FPS}", l.w, l.h)]);
        "[0:v]null".to_string()
    };
    inputs += 1;

    let mut graph = String::new();
    let dialogue = seg.kind == SegKind::Dialogue && (seg.face_video.is_some() || seg.face_image.is_some());
    match seg.kind {
        SegKind::Title { .. } | SegKind::SceneCard { .. } => {
            graph += &format!("{bg_chain},eq=brightness=-0.12:saturation=0.9,gblur=sigma=3[bg];");
        }
        _ if dialogue => {
            graph += &format!("{bg_chain},gblur=sigma=14,eq=brightness=-0.10:saturation=1.05[bg];");
        }
        _ => graph += &format!("{bg_chain},eq=saturation=1.05[bg];"),
    }
    let mut last = "bg".to_string();
    if dialogue {
        let fi = inputs;
        if let Some(v) = &seg.face_video {
            cmd.arg("-i").arg(v);
        } else if let Some(img) = &seg.face_image {
            cmd.args(["-loop", "1", "-t", &format!("{d:.3}"), "-i"]).arg(img);
        }
        inputs += 1;
        let mi = inputs;
        cmd.arg("-i").arg(&assets.0);
        inputs += 1;
        let gi = inputs;
        cmd.arg("-i").arg(&assets.1);
        inputs += 1;
        let c = l.card;
        let (x, y) = card_pos(l, seg);
        // gentle "breathing" zoom keeps still faces alive
        let still_motion = if seg.face_video.is_none() {
            format!(",scale=w='{c}*(1.02+0.02*sin(t*1.7))':h=-2:eval=frame,crop={c}:{c}")
        } else {
            String::new()
        };
        graph += &format!(
            "[{fi}:v]fps={FPS},scale={c}:{c}:force_original_aspect_ratio=increase,crop={c}:{c}{still_motion},format=rgba[f];\
[{mi}:v]format=gray,scale={c}:{c}[m];[f][m]alphamerge[face];\
[{last}][{gi}:v]overlay={gx}:{gy}[g];[g][face]overlay={x}:{y}:eof_action=repeat[fc];",
            gx = x - 20, gy = y - 20);
        last = "fc".into();
    }
    let fade_in = if seg.fade_in { ",fade=t=in:st=0:d=0.45".to_string() } else { String::new() };
    let fade_out = if seg.fade_out { format!(",fade=t=out:st={:.3}:d=0.45", (d - 0.45).max(0.0)) } else { String::new() };
    graph += &format!("[{last}]ass='{ass_name}':shaping=complex,vignette=PI/5,noise=alls=4:allf=t{fade_in}{fade_out},format=yuv420p[v]");

    // audio
    let ai = inputs;
    if let Some(a) = &seg.audio {
        cmd.arg("-i").arg(a);
    } else {
        cmd.args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo"]);
    }
    let afade = if seg.fade_out { format!(",afade=t=out:st={:.3}:d=0.4", (d - 0.4).max(0.0)) } else { String::new() };
    graph += &format!(";[{ai}:a]aresample=48000,aformat=channel_layouts=stereo,apad{afade}[a]");

    cmd.args(["-filter_complex", &graph, "-map", "[v]", "-map", "[a]", "-t", &format!("{d:.3}"),
        "-r", &FPS.to_string()]);
    cmd.args(venc(nvenc));
    cmd.args(["-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-movflags", "+faststart"]);
    cmd.arg(out);
    run(&mut cmd)
}

/// Concatenate segments, mix background music with ducking, normalise loudness.
pub fn finalize(segments: &[PathBuf], music: Option<&Path>, music_volume: f64, out: &Path, work: &Path) -> Result<()> {
    let list = work.join("concat.txt");
    let body: String = segments.iter()
        .map(|p| format!("file '{}'\n", p.to_string_lossy().replace('\\', "/").replace('\'', "'\\''")))
        .collect();
    std::fs::write(&list, body)?;
    let joined = work.join("joined.mp4");
    run(Command::new(ffmpeg()).args(["-y", "-v", "error", "-f", "concat", "-safe", "0", "-i"]).arg(&list)
        .args(["-c", "copy"]).arg(&joined))?;

    let mut cmd = Command::new(ffmpeg());
    cmd.args(["-y", "-v", "error", "-i"]).arg(&joined);
    let filter = if let Some(m) = music.filter(|m| m.exists()) {
        cmd.args(["-stream_loop", "-1", "-i"]).arg(m);
        format!("[1:a]aresample=48000,volume={:.3}[mus];[0:a]asplit=2[voice][key];\
[mus][key]sidechaincompress=threshold=0.03:ratio=8:attack=20:release=400[duck];\
[voice][duck]amix=inputs=2:duration=first:normalize=0,loudnorm=I=-14:TP=-1.5:LRA=11[a]",
            music_volume.clamp(0.0, 1.0) * 2.0)
    } else {
        "[0:a]loudnorm=I=-14:TP=-1.5:LRA=11[a]".to_string()
    };
    cmd.args(["-filter_complex", &filter, "-map", "0:v", "-map", "[a]", "-c:v", "copy",
        "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-movflags", "+faststart", "-shortest"]).arg(out);
    run(&mut cmd)?;
    let _ = std::fs::remove_file(joined);
    Ok(())
}

/// Build the segment plan for a project whose assets are ready.
pub fn plan(p: &Project, pause: f64) -> Vec<Segment> {
    let side_of = |c: &Character| -> bool {
        let idx = p.characters.iter().filter(|x| !x.is_narrator).position(|x| x.id == c.id).unwrap_or(0);
        idx % 2 == 1
    };
    let mut segs = vec![];
    let first_bg = p.scenes.iter().find_map(|s| s.image.clone()).map(PathBuf::from)
        .or_else(|| p.background_image.clone().map(PathBuf::from));
    segs.push(Segment {
        kind: SegKind::Title { title: p.title.clone(), subtitle: p.logline.clone() },
        background: first_bg, clip: None, face_video: None, face_image: None, audio: None,
        duration: 4.0, speaker: String::new(), text: String::new(), side_right: false,
        accent: accent_for(0), fade_in: true, fade_out: true, motion: 0,
    });
    let mut motion = 1;
    for sc in &p.scenes {
        let bg = sc.image.clone().map(PathBuf::from).or_else(|| p.background_image.clone().map(PathBuf::from));
        let clip = sc.clip.clone().map(PathBuf::from);
        if !sc.title.is_empty() {
            segs.push(Segment {
                kind: SegKind::SceneCard { title: sc.title.clone() }, background: bg.clone(), clip: None,
                face_video: None, face_image: None, audio: None, duration: 2.4, speaker: String::new(),
                text: String::new(), side_right: false, accent: accent_for(0), fade_in: true, fade_out: false,
                motion,
            });
            motion += 1;
        }
        let n = sc.lines.len();
        for (i, line) in sc.lines.iter().enumerate() {
            let Some(c) = p.character(&line.character_id) else { continue };
            let Some(audio) = &line.audio else { continue };
            let ci = p.characters.iter().position(|x| x.id == c.id).unwrap_or(0);
            let narr = c.is_narrator || c.face_image.is_none();
            segs.push(Segment {
                kind: if narr { SegKind::Narration } else { SegKind::Dialogue },
                background: bg.clone(), clip: clip.clone(),
                face_video: if p.talking_faces { line.talking_video.clone().map(PathBuf::from) } else { None },
                face_image: c.face_image.clone().map(PathBuf::from),
                audio: Some(PathBuf::from(audio)),
                duration: line.duration.max(0.6) + pause,
                speaker: if narr { String::new() } else { c.name.clone() },
                text: line.text.clone(), side_right: side_of(c), accent: accent_for(ci),
                fade_in: false, fade_out: i + 1 == n, motion,
            });
            motion += 1;
        }
    }
    segs
}
