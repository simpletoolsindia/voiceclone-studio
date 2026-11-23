//! Story video compositor (ffmpeg; NVENC when available).
//!
//! Shot grammar (designed for retention, see README "Video style"):
//! * cold open  - the most dramatic line first, as a hook (first 3 s)
//! * title card - story title over the first scene, with a soft ding
//! * scene card - scene title, whoosh transition, ambience starts
//! * two-shot   - speaker (lip-synced) and listener cut out *inside* the scene,
//!                facing each other; the listener breathes, slightly dimmed
//! * close-up   - shot / reverse-shot on the speaker, background in soft focus
//! * narration  - the full painted scene with slow camera motion
//! * end card   - comment question + animated subscribe button + AI disclosure
//! Every shot moves (Ken Burns, punch-in on lines > 4.5 s), captions highlight
//! word by word, and strong emotions pop an emoji sticker next to the speaker.

use crate::story::Project;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const FPS: u32 = 30;

pub struct Layout {
    pub w: u32,
    pub h: u32,
}

impl Layout {
    pub fn for_format(format: &str) -> Self {
        if format == "shorts" { Layout { w: 1080, h: 1920 } } else { Layout { w: 1920, h: 1080 } }
    }
    fn vertical(&self) -> bool {
        self.h > self.w
    }
    fn card(&self) -> u32 {
        if self.vertical() { 900 } else { 820 }
    }
}

#[derive(Clone)]
pub struct Actor {
    pub video: Option<PathBuf>,
    pub image: PathBuf,
    pub mask: Option<PathBuf>,
    pub right: bool,
    pub name: String,
    pub accent: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Shot {
    Full,
    /// a finished full-frame animated shot (cinematic mode) - only captions / emoji on top
    Clip,
    TwoShot,
    CloseUp,
    Card,
}

#[derive(Clone, PartialEq)]
pub enum SegKind {
    Title { title: String, subtitle: String },
    SceneCard { title: String },
    Line,
    EndCard { question: String },
    /// Pause-and-guess card for viewers: question + 3-2-1 countdown
    Question { text: String },
}

#[derive(Clone)]
pub struct Segment {
    pub kind: SegKind,
    pub shot: Shot,
    pub background: Option<PathBuf>,
    pub clip: Option<PathBuf>,
    pub speaker: Option<Actor>,
    pub listener: Option<Actor>,
    pub audio: Option<PathBuf>,
    pub duration: f64,
    pub text: String,
    pub emoji: Option<PathBuf>,
    pub ambience: Option<PathBuf>,
    pub sfx: Vec<(PathBuf, f64, f64)>, // (file, start seconds, volume)
    pub fade_in: bool,
    pub fade_out: bool,
    pub motion: u32,
    pub chapter: Option<String>,
    /// provenance, used for the editable timeline ("at 1:22 fix ...")
    pub scene_id: Option<String>,
    pub line_id: Option<String>,
    pub label: String,
}

pub struct Sfx {
    pub whoosh: PathBuf,
    pub pop: PathBuf,
    pub ding: PathBuf,
    pub rain: PathBuf,
    pub night: PathBuf,
    pub tick: PathBuf,
}

pub fn ffmpeg() -> String {
    std::env::var("FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

fn hide(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

fn run(cmd: &mut Command) -> Result<()> {
    hide(cmd);
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
    hide(&mut c);
    c.output().map(|o| o.status.success()).unwrap_or(false)
}

fn venc(nvenc: bool) -> Vec<&'static str> {
    if nvenc {
        vec!["-c:v", "h264_nvenc", "-preset", "p5", "-rc", "vbr", "-cq", "21", "-b:v", "0", "-maxrate", "16M",
             "-bufsize", "32M", "-profile:v", "high"]
    } else {
        vec!["-c:v", "libx264", "-preset", "medium", "-crf", "20"]
    }
}

pub fn audio_duration(path: &Path) -> f64 {
    let mut c = Command::new(ffmpeg().replace("ffmpeg", "ffprobe"));
    c.args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(path);
    hide(&mut c);
    c.output().ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok()).unwrap_or(0.0)
}

/// Accent colours per character (ASS uses &HBBGGRR).
pub fn accent_for(i: usize) -> String {
    const C: [&str; 8] = ["&H3CC8FF", "&HFF9E4A", "&H7BE07B", "&HB06CFF", "&H5A5AFF", "&HE0D040", "&H40A0FF", "&HFF70C0"];
    C[i % C.len()].into()
}

/// Emoji sticker for a line's emotion, if it is strong enough to deserve one.
pub fn emoji_for(emotion: &str, text: &str) -> Option<&'static str> {
    let e = emotion.to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| e.contains(w));
    if has(&["laugh", "funny", "joke", "giggl"]) { Some("😂") }
    else if has(&["surpris", "shock", "amaz", "wow", "astonish"]) || text.starts_with("வாவ்") { Some("😮") }
    else if has(&["excit", "thrill"]) { Some("🤩") }
    else if has(&["sad", "cry", "tear", "upset"]) { Some("😢") }
    else if has(&["angry", "anger", "furious", "mad"]) { Some("😠") }
    else if has(&["love", "nostalg", "tender", "affection"]) { Some("❤️") }
    else if has(&["curious", "think", "wonder", "doubt"]) { Some("🤔") }
    else if has(&["scared", "fear", "afraid"]) { Some("😨") }
    else if has(&["bored", "sleepy", "tired"]) { Some("😴") }
    else if has(&["happy", "joy", "cheer", "proud"]) { Some("😄") }
    else { None }
}

/// Scene ambience chosen from the scene description.
pub fn ambience_for(visual: &str, title: &str, sfx: &Sfx) -> Option<PathBuf> {
    let t = format!("{} {}", visual, title).to_lowercase();
    if ["rain", "storm", "மழை"].iter().any(|k| t.contains(k)) {
        Some(sfx.rain.clone())
    } else if ["night", "candle", "moon", "இரவு", "ராத்திரி"].iter().any(|k| t.contains(k)) {
        Some(sfx.night.clone())
    } else {
        None
    }
}

/// Synthesize the small sound-effect library once (procedural, royalty-free).
pub fn make_sfx(dir: &Path) -> Result<Sfx> {
    std::fs::create_dir_all(dir)?;
    let s = Sfx {
        whoosh: dir.join("whoosh.wav"), pop: dir.join("pop.wav"), ding: dir.join("ding.wav"),
        rain: dir.join("rain.wav"), night: dir.join("night.wav"), tick: dir.join("tick.wav"),
    };
    let make = |out: &Path, graph: &str| -> Result<()> {
        if out.exists() {
            return Ok(());
        }
        run(Command::new(ffmpeg()).args(["-y", "-v", "error", "-filter_complex", graph, "-ar", "48000", "-ac", "2"]).arg(out))
    };
    make(&s.whoosh, "anoisesrc=d=0.8:c=pink:a=0.7,bandpass=f=900:width_type=o:w=2.5,highpass=f=200,afade=t=in:d=0.4:curve=exp,afade=t=out:st=0.4:d=0.4:curve=exp,volume=1.4")?;
    make(&s.pop, "sine=f=740:d=0.14[a];sine=f=1110:d=0.14[b];[a][b]amix=inputs=2,afade=t=out:st=0:d=0.14:curve=exp,volume=1.2")?;
    make(&s.ding, "sine=f=1046.5:d=1.2[a];sine=f=1568:d=1.2,volume=0.35[b];[a][b]amix=inputs=2,afade=t=out:st=0.02:d=1.18:curve=exp,volume=0.9")?;
    make(&s.rain, "anoisesrc=d=12:c=brown:a=0.6,lowpass=f=1100[a];anoisesrc=d=12:c=white:a=0.05,highpass=f=3500,lowpass=f=9000[b];[a][b]amix=inputs=2,volume=1.2")?;
    make(&s.night, "sine=f=4300:d=12,volume=0.12,tremolo=f=32:d=1,tremolo=f=0.6:d=0.95[a];anoisesrc=d=12:c=brown:a=0.15,lowpass=f=500[b];[a][b]amix=inputs=2")?;
    make(&s.tick, "sine=f=1400:d=0.05[a];sine=f=2100:d=0.05,volume=0.4[b];[a][b]amix=inputs=2,afade=t=out:st=0:d=0.05,volume=1.3")?;
    Ok(s)
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

/// Split a line into caption chunks shown in sequence.
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

pub fn actor_box(l: &Layout, shot: Shot, right: bool, listener: bool) -> (i64, i64, i64) {
    let (w, h) = (l.w as i64, l.h as i64);
    if l.vertical() {
        match (shot, listener) {
            (Shot::CloseUp, _) => (0, h - 1080 - 360, 1080),
            (_, false) => (if right { w - 860 } else { 0 }, h - 860 - 400, 860),
            (_, true) => (if right { w - 600 + 40 } else { -40 }, h - 600 - 520, 600),
        }
    } else {
        match (shot, listener) {
            (Shot::CloseUp, _) => (if right { w - 1040 - 160 } else { 160 }, h - 1040 + 150, 1040),
            (_, false) => (if right { w - 880 - 110 } else { 110 }, h - 880 + 70, 880),
            (_, true) => (if right { w - 760 - 130 } else { 130 }, h - 760 + 70, 760),
        }
    }
}

fn card_pos(l: &Layout, right: bool) -> (i64, i64) {
    let c = l.card() as i64;
    if l.vertical() {
        ((l.w as i64 - c) / 2, 300)
    } else {
        let x = if right { l.w as i64 - c - 150 } else { 150 };
        (x, (l.h as i64 - c) / 2 - 70)
    }
}

fn write_ass(path: &Path, l: &Layout, seg: &Segment, subtitles: bool) -> Result<()> {
    let vertical = l.vertical();
    let font = "Nirmala UI";
    let sub_size = if vertical { 70 } else { 62 };
    // Karaoke: SecondaryColour (white) = not yet spoken, PrimaryColour (gold) = spoken.
    let mut s = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {w}\nPlayResY: {h}\nWrapStyle: 0\nScaledBorderAndShadow: yes\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Sub,{font},{sub_size},&H0030D8FF,&H00FFFFFF,&H00101010,&H90000000,-1,0,0,0,100,100,0,0,1,5,3,2,90,90,{mv},1\n\
Style: Name,{font},42,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,3,12,0,5,0,0,0,1\n\
Style: Title,{font},{ts},&H00FFFFFF,&H000000FF,&H00101010,&H96000000,-1,0,0,0,100,100,0,0,1,6,4,5,80,80,0,1\n\
Style: Small,{font},{ss},&H00E8E8E8,&H000000FF,&H00101010,&H96000000,0,0,0,0,100,100,0,0,1,3,2,5,80,80,0,1\n\
Style: Btn,Segoe UI,{bs},&H00FFFFFF,&H000000FF,&H001A1AE0,&H00000000,-1,0,0,0,100,100,0,0,3,26,0,5,0,0,0,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        w = l.w, h = l.h, mv = if vertical { 300 } else { 60 },
        ts = if vertical { 96 } else { 110 }, ss = if vertical { 48 } else { 50 }, bs = if vertical { 64 } else { 56 }
    );
    let end = ass_time(seg.duration);
    let (cx, cy) = (l.w / 2, l.h / 2);
    match &seg.kind {
        SegKind::Title { title, subtitle } => {
            s += &format!("Dialogue: 0,0:00:00.30,{end},Title,,0,0,0,,{{\\pos({cx},{})\\fad(500,400)\\fscx85\\fscy85\\t(0,500,\\fscx100\\fscy100)}}{}\n",
                cy - 40, ass_escape(title));
            if !subtitle.is_empty() {
                s += &format!("Dialogue: 0,0:00:01.00,{end},Small,,0,0,0,,{{\\pos({cx},{})\\fad(600,400)}}{}\n", cy + 80, ass_escape(subtitle));
            }
        }
        SegKind::SceneCard { title } => {
            s += &format!("Dialogue: 0,0:00:00.15,{end},Title,,0,0,0,,{{\\pos({cx},{cy})\\fad(300,350)\\fs{}\\fscx70\\fscy70\\t(0,350,\\fscx100\\fscy100)}}{}\n",
                if vertical { 80 } else { 88 }, ass_escape(title));
        }
        SegKind::Question { text } => {
            let qy = if vertical { cy - 220 } else { cy - 150 };
            s += &format!("Dialogue: 0,0:00:00.15,{end},Title,,0,0,0,,{{\\pos({cx},{qy})\\fad(250,250)\\fs{}\\fscx80\\fscy80\\t(0,250,\\fscx100\\fscy100)}}{}\n",
                if vertical { 70 } else { 76 }, ass_escape(text));
            // big 3-2-1 countdown, one number per second, each popping in
            for (k, n) in ["3", "2", "1"].iter().enumerate() {
                let t0 = 0.8 + k as f64;
                s += &format!("Dialogue: 1,{},{},Title,,0,0,0,,{{\\pos({cx},{})\\fs{}\\c&H0030D8FF&\\fscx50\\fscy50\\t(0,180,\\fscx110\\fscy110)\\t(180,320,\\fscx100\\fscy100)\\fad(60,150)}}{n}\n",
                    ass_time(t0), ass_time(t0 + 0.95), cy + 110, if vertical { 220 } else { 200 });
            }
        }
        SegKind::EndCard { question } => {
            let qy = if vertical { cy - 260 } else { cy - 190 };
            s += &format!("Dialogue: 0,0:00:00.20,{end},Title,,0,0,0,,{{\\pos({cx},{qy})\\fad(400,300)\\fs{}}}{}\n",
                if vertical { 66 } else { 70 }, ass_escape(question));
            s += &format!("Dialogue: 0,0:00:00.60,{end},Small,,0,0,0,,{{\\pos({cx},{})\\fad(400,300)}}அடுத்த கதையில சந்திப்போம்!\n", qy + 90);
            // pulsing red SUBSCRIBE button
            let by = if vertical { cy + 120 } else { cy + 110 };
            let mut pulse = String::new();
            let mut t = 1200;
            while (t as f64) < seg.duration * 1000.0 - 400.0 {
                pulse += &format!("\\t({},{},\\fscx112\\fscy112)\\t({},{},\\fscx100\\fscy100)", t, t + 300, t + 300, t + 600);
                t += 1200;
            }
            s += &format!("Dialogue: 1,0:00:01.00,{end},Btn,,0,0,0,,{{\\pos({cx},{by})\\fad(200,300)\\fscx40\\fscy40\\t(0,250,\\fscx115\\fscy115)\\t(250,400,\\fscx100\\fscy100){pulse}}}SUBSCRIBE\n");
            s += &format!("Dialogue: 0,0:00:01.40,{end},Small,,0,0,0,,{{\\pos({cx},{})\\fad(300,300)}}லைக் • ஷேர் • சப்ஸ்கிரைப் பண்ணுங்க\n", by + 110);
            s += &format!("Dialogue: 0,0:00:00.50,{end},Small,,0,0,0,,{{\\pos({cx},{})\\fs{}\\alpha&H60&}}இந்த வீடியோ AI உதவியுடன் உருவாக்கப்பட்டது\n",
                l.h - 60, if vertical { 34 } else { 30 });
        }
        SegKind::Line => {
            // No name tags: viewers recognise characters from their faces, voices and the story.
            // a Close-up's text is an English picture description for the image model - never shown
            if subtitles && !seg.text.is_empty() && seg.label != "Close-up" {
                let chunks = caption_chunks(&seg.text, if vertical { 26 } else { 40 });
                let total: usize = chunks.iter().map(|c| c.chars().count()).sum::<usize>().max(1);
                let speak = (seg.duration - 0.40).max(0.5);
                let mut t = 0.05;
                for c in chunks {
                    let d = speak * c.chars().count() as f64 / total as f64;
                    let words: Vec<&str> = c.split(' ').collect();
                    let wc: usize = words.iter().map(|w| w.chars().count() + 1).sum::<usize>().max(1);
                    let mut kara = String::new();
                    for (i, w) in words.iter().enumerate() {
                        let cs = ((d * 100.0) * (w.chars().count() + 1) as f64 / wc as f64).round() as i64;
                        kara += &format!("{{\\k{cs}}}{}{}", ass_escape(w), if i + 1 < words.len() { " " } else { "" });
                    }
                    s += &format!("Dialogue: 1,{},{},Sub,,0,0,0,,{{\\fad(90,60)\\fscx92\\fscy92\\t(0,120,\\fscx100\\fscy100)}}{kara}\n",
                        ass_time(t), ass_time(t + d));
                    t += d;
                }
            }
        }
    }
    std::fs::write(path, s)?;
    Ok(())
}

/// Rounded-corner mask and glow frame used by fallback face cards.
pub fn make_card_assets(dir: &Path, l: &Layout) -> Result<(PathBuf, PathBuf)> {
    let c = l.card();
    let mask = dir.join(format!("card_mask_{c}.png"));
    let glow = dir.join(format!("card_glow_{c}.png"));
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

struct Inputs<'a> {
    cmd: &'a mut Command,
    n: usize,
}
impl Inputs<'_> {
    fn file(&mut self, pre: &[&str], p: &Path) -> usize {
        self.cmd.args(pre).arg("-i").arg(p);
        self.n += 1;
        self.n - 1
    }
    fn lavfi(&mut self, src: &str) -> usize {
        self.cmd.args(["-f", "lavfi", "-i", src]);
        self.n += 1;
        self.n - 1
    }
}

/// Filter chain that turns an actor (talking video or still portrait + mask) into
/// an RGBA cutout of `size`; returns the labels of its shadow and of the actor.
fn actor_chain(graph: &mut String, inp: &mut Inputs, a: &Actor, size: i64, d: f64, listener: bool, tag: &str) -> (String, String) {
    let dur = format!("{d:.3}");
    let vi = match (&a.video, listener) {
        (Some(v), false) => inp.file(&[], v),
        _ => inp.file(&["-loop", "1", "-t", &dur], &a.image),
    };
    let mi = inp.file(&["-loop", "1", "-t", &dur], a.mask.as_ref().unwrap());
    let dim = if listener { ",colorchannelmixer=rr=0.82:gg=0.82:bb=0.86" } else { "" };
    let breathe = if listener {
        format!(",scale=w='{size}*(1+0.012*sin(2*PI*t/3.4))':h=-1:eval=frame")
    } else if a.video.is_none() {
        // speaking without a lip-synced clip (cartoon animals, objects): cartoon "talk" motion -
        // squash-and-stretch on the syllable rhythm plus a gentle head tilt, so the speaker reads as talking
        format!(",scale=w='{size}*(1+0.022*sin(2*PI*t*2.3))':h='{size}*(1+0.050*abs(sin(2*PI*t*2.3+0.6)))':eval=frame,rotate='0.035*sin(2*PI*t*0.9)':c=none")
    } else {
        String::new()
    };
    *graph += &format!(
        "[{vi}:v]fps={FPS},scale={size}:{size},format=rgba{dim}[{tag}v];[{mi}:v]scale={size}:{size},format=gray[{tag}m];\
[{tag}v][{tag}m]alphamerge{breathe},split[{tag}a][{tag}s0];\
[{tag}s0]colorchannelmixer=rr=0:rg=0:rb=0:gr=0:gg=0:gb=0:br=0:bg=0:bb=0:aa=0.5,gblur=sigma=18[{tag}s];");
    (format!("{tag}s"), format!("{tag}a"))
}

/// Render one segment to `out` (H.264 + AAC 48 kHz stereo).
pub fn render_segment(seg: &Segment, l: &Layout, out: &Path, work: &Path, card: &(PathBuf, PathBuf),
                      subtitles: bool, nvenc: bool) -> Result<()> {
    let d = seg.duration.max(0.5);
    let frames = (d * FPS as f64).ceil() as u32;
    let ass = out.with_extension("ass");
    write_ass(&ass, l, seg, subtitles)?;
    let ass_name = ass.file_name().unwrap().to_string_lossy().to_string();
    let mut cmd = Command::new(ffmpeg());
    cmd.current_dir(work).args(["-y", "-v", "error"]);
    let mut inp = Inputs { cmd: &mut cmd, n: 0 };
    let mut g = String::new();

    // ---- background
    let bg_chain = if let Some(clip) = &seg.clip {
        let i = inp.file(&["-stream_loop", "-1", "-t", &format!("{d:.3}")], clip);
        format!("[{i}:v]fps={FPS},scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1", w = l.w, h = l.h)
    } else if let Some(bg) = &seg.background {
        let i = inp.file(&[], bg);
        format!("[{i}:v]{}", ken_burns(l, frames, seg.motion))
    } else {
        let i = inp.lavfi(&format!("color=c=0x15121c:s={}x{}:d={d:.3}:r={FPS}", l.w, l.h));
        format!("[{i}:v]null")
    };
    let soft = match (&seg.kind, seg.shot) {
        (SegKind::Title { .. } | SegKind::SceneCard { .. }, _) => ",eq=brightness=-0.12:saturation=0.9,gblur=sigma=3",
        (SegKind::EndCard { .. } | SegKind::Question { .. }, _) => ",gblur=sigma=16,eq=brightness=-0.22:saturation=0.9",
        (_, Shot::Clip) => "",
        (_, Shot::CloseUp) => ",gblur=sigma=9,eq=brightness=-0.05",
        (_, Shot::TwoShot) => ",gblur=sigma=2.2",
        (_, Shot::Card) => ",gblur=sigma=14,eq=brightness=-0.10:saturation=1.05",
        _ => ",eq=saturation=1.05",
    };
    g += &format!("{bg_chain}{soft}[bg];");
    let mut last = "bg".to_string();

    // ---- characters
    let with_mask = |a: &Option<Actor>| a.clone().filter(|a| a.mask.as_ref().map(|m| m.exists()).unwrap_or(false));
    let speaker_cut = with_mask(&seg.speaker);
    match seg.shot {
        Shot::TwoShot | Shot::CloseUp if speaker_cut.is_some() => {
            if seg.shot == Shot::TwoShot {
                if let Some(li) = with_mask(&seg.listener) {
                    let (x, y, sz) = actor_box(l, Shot::TwoShot, li.right, true);
                    let (sh, act) = actor_chain(&mut g, &mut inp, &li, sz, d, true, "L");
                    g += &format!("[{last}][{sh}]overlay={}:{}[l1];[l1][{act}]overlay={x}:{y}:eof_action=repeat[l2];", x + 22, y + 16);
                    last = "l2".into();
                }
            }
            let sp = speaker_cut.unwrap();
            let (x, y, sz) = actor_box(l, seg.shot, sp.right, false);
            let (sh, act) = actor_chain(&mut g, &mut inp, &sp, sz, d, false, "S");
            g += &format!("[{last}][{sh}]overlay={}:{}[s1];[s1][{act}]overlay=x='{x}+({sz}-overlay_w)/2':y='{y}+({sz}-overlay_h){hop}':eval=frame:eof_action=repeat[s2];", x + 22, y + 16,
                hop = if sp.video.is_none() { "-10*abs(sin(2*PI*t*2.3))" } else { "" });
            last = "s2".into();
            if let Some(e) = &seg.emoji {
                let ei = inp.file(&["-loop", "1", "-t", &format!("{d:.3}")], e);
                let es = if l.vertical() { 190 } else { 170 };
                let ex = if sp.right { x + (sz as f64 * 0.06) as i64 } else { x + (sz as f64 * 0.74) as i64 };
                let ey = y.max(20) + (sz as f64 * 0.04) as i64;
                g += &format!("[{ei}:v]format=rgba,scale=w='{es}*min(1\\,max(0.01\\,(t-0.25)/0.18))*(1+0.12*sin(t*9)*exp(-2*t))':h=-1:eval=frame,\
fade=t=out:st=2.2:d=0.4:alpha=1[emo];[{last}][emo]overlay=x='{ex}+({es}-overlay_w)/2':y='{ey}+({es}-overlay_h)/2':enable='between(t,0.25,2.6)':eof_action=pass[e1];");
                last = "e1".into();
            }
        }
        Shot::Clip => {
            if let (Some(e), Some(sp)) = (&seg.emoji, &seg.speaker) {
                let ei = inp.file(&["-loop", "1", "-t", &format!("{d:.3}")], e);
                let es = if l.vertical() { 190 } else { 170 };
                let ex = if sp.right { (l.w as f64 * 0.58) as i64 } else { (l.w as f64 * 0.30) as i64 };
                let ey = (l.h as f64 * 0.10) as i64;
                g += &format!(r"[{ei}:v]format=rgba,scale=w='{es}*min(1\,max(0.01\,(t-0.25)/0.18))*(1+0.12*sin(t*9)*exp(-2*t))':h=-1:eval=frame,fade=t=out:st=2.2:d=0.4:alpha=1[emo];[{last}][emo]overlay=x='{ex}+({es}-overlay_w)/2':y='{ey}+({es}-overlay_h)/2':enable='between(t,0.25,2.6)':eof_action=pass[e1];");
                last = "e1".into();
            }
        }
        Shot::TwoShot | Shot::CloseUp | Shot::Card if seg.speaker.is_some() => {
            // fallback: rounded face card
            let sp = seg.speaker.as_ref().unwrap();
            let fi = match &sp.video {
                Some(v) => inp.file(&[], v),
                None => inp.file(&["-loop", "1", "-t", &format!("{d:.3}")], &sp.image),
            };
            let mi = inp.file(&[], &card.0);
            let gi = inp.file(&[], &card.1);
            let c = l.card();
            let (x, y) = card_pos(l, sp.right);
            g += &format!("[{fi}:v]fps={FPS},scale={c}:{c}:force_original_aspect_ratio=increase,crop={c}:{c},format=rgba[f];\
[{mi}:v]format=gray,scale={c}:{c}[m];[f][m]alphamerge[face];[{last}][{gi}:v]overlay={}:{}[g0];[g0][face]overlay={x}:{y}:eof_action=repeat[fc];",
                x - 20, y - 20);
            last = "fc".into();
        }
        _ => {}
    }

    // ---- camera punch-in on long dialogue lines (visual change every ~4 s)
    if seg.kind == SegKind::Line && d > 4.5 && seg.shot != Shot::Full && seg.shot != Shot::Clip {
        let m = d * 0.5;
        g += &format!("[{last}]scale=w='{w}*(1+0.06*min(1\\,max(0\\,(t-{m:.2})/0.30)))':h=-2:eval=frame,crop={w}:{h}[pi];", w = l.w, h = l.h);
        last = "pi".into();
    }
    let fade_in = if seg.fade_in { ",fade=t=in:st=0:d=0.40".to_string() } else { String::new() };
    let fade_out = if seg.fade_out { format!(",fade=t=out:st={:.3}:d=0.40", (d - 0.40).max(0.0)) } else { String::new() };
    g += &format!("[{last}]ass='{ass_name}':shaping=complex,vignette=PI/5,noise=alls=3:allf=t{fade_in}{fade_out},format=yuv420p[v];");

    // ---- audio: voice + ambience bed + sound effects
    let vi = match &seg.audio {
        Some(a) => inp.file(&[], a),
        None => inp.lavfi("anullsrc=r=48000:cl=stereo"),
    };
    // voice clarity: the TTS output is a little dull above 4 kHz - cut rumble, lift presence and air
    let eq = if seg.audio.is_some() { ",highpass=f=75,equalizer=f=3400:t=q:w=1.1:g=3,highshelf=f=7000:g=3.5,alimiter=limit=0.95" } else { "" };
    let mut mix = vec![format!("[{vi}:a]aresample=48000:resampler=soxr{eq},aformat=channel_layouts=stereo,apad[a0]")];
    let mut labels = vec!["[a0]".to_string()];
    if let Some(amb) = &seg.ambience {
        let ai = inp.file(&["-stream_loop", "-1"], amb);
        mix.push(format!("[{ai}:a]aresample=48000,aformat=channel_layouts=stereo,volume=0.10,atrim=0:{d:.3}[amb]"));
        labels.push("[amb]".into());
    }
    for (k, (f, at, vol)) in seg.sfx.iter().enumerate() {
        let si = inp.file(&[], f);
        let ms = (at * 1000.0) as i64;
        mix.push(format!("[{si}:a]aresample=48000,aformat=channel_layouts=stereo,adelay={ms}|{ms},volume={vol:.2}[fx{k}]"));
        labels.push(format!("[fx{k}]"));
    }
    let afade = if seg.fade_out { format!(",afade=t=out:st={:.3}:d=0.4", (d - 0.4).max(0.0)) } else { String::new() };
    g += &mix.join(";");
    g += &format!(";{}amix=inputs={}:duration=first:normalize=0{afade}[a]", labels.join(""), labels.len());

    cmd.args(["-filter_complex", &g, "-map", "[v]", "-map", "[a]", "-t", &format!("{d:.3}"), "-r", &FPS.to_string()]);
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

/// Most dramatic dialogue line, used as the cold-open hook.
fn hook_line(p: &Project) -> Option<(usize, usize)> {
    let mut best: Option<((usize, usize), f64)> = None;
    for (si, sc) in p.scenes.iter().enumerate() {
        for (li, l) in sc.lines.iter().enumerate() {
            let Some(c) = p.character(&l.character_id) else { continue };
            if c.is_narrator || l.audio.is_none() || l.duration < 1.5 || l.duration > 6.5 {
                continue;
            }
            let mut score = l.text.matches(['!', '?']).count() as f64;
            if emoji_for(&l.emotion, &l.text).is_some() {
                score += 2.0;
            }
            if ["surpris", "shock", "excit", "angry", "scared"].iter().any(|k| l.emotion.to_lowercase().contains(k)) {
                score += 2.0;
            }
            score -= si as f64 * 0.1; // prefer earlier lines: fewer spoilers
            if best.as_ref().map(|b| score > b.1).unwrap_or(true) {
                best = Some(((si, li), score));
            }
        }
    }
    best.filter(|b| b.1 >= 2.0).map(|b| b.0)
}

/// Build the shot plan for a project whose assets are ready.
pub fn plan(p: &Project, pause: f64, sfx: &Sfx, _emoji_png: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<Segment> {
    let on_screen: Vec<&crate::story::Character> =
        p.characters.iter().filter(|c| !c.is_narrator && c.face_image.is_some()).collect();
    let actor = |cid: &str, video: Option<String>| -> Option<Actor> {
        let c = on_screen.iter().find(|c| c.id == cid)?;
        let idx = on_screen.iter().position(|x| x.id == cid).unwrap_or(0);
        let face = PathBuf::from(c.face_image.as_ref()?);
        let mask = face.with_file_name(format!("{}_mask.png", c.id));
        Some(Actor {
            video: if p.talking_faces { video.map(PathBuf::from).filter(|v| v.exists()) } else { None },
            mask: if mask.exists() { Some(mask) } else { None },
            image: face, right: idx % 2 == 1, name: c.name.clone(),
            accent: accent_for(p.characters.iter().position(|x| x.id == cid).unwrap_or(0)),
        })
    };
    let blank = Segment {
        kind: SegKind::Line, shot: Shot::Full, background: None, clip: None, speaker: None, listener: None,
        audio: None, duration: 1.0, text: String::new(), emoji: None, ambience: None, sfx: vec![],
        fade_in: false, fade_out: false, motion: 0, chapter: None, scene_id: None, line_id: None, label: String::new(),
    };
    let line_seg = |si: usize, li: usize, motion: u32, shot_hint: Shot, prev: Option<&str>, next: Option<&str>| -> Option<Segment> {
        let sc = &p.scenes[si];
        let l = &sc.lines[li];
        if !l.is_dialogue() {
            return None;
        }
        let c = p.character(&l.character_id)?;
        let audio = l.audio.clone()?;
        let bg = sc.image.clone().map(PathBuf::from).or_else(|| p.background_image.clone().map(PathBuf::from));
        let speaker = if c.is_narrator { None } else { actor(&c.id, l.talking_video.clone()) };
        // cutout characters stand in front of the empty set, not the painted scene
        let bg = if speaker.as_ref().map(|a| a.mask.is_some()).unwrap_or(false) {
            sc.plate.clone().map(PathBuf::from).filter(|p| p.exists()).or(bg)
        } else {
            bg
        };
        let listener_id = prev.filter(|s| *s != c.id).or(next.filter(|s| *s != c.id));
        let mut listener = listener_id.and_then(|id| actor(id, None));
        if let (Some(sp), Some(lis)) = (&speaker, listener.as_mut()) {
            lis.right = !sp.right; // characters face each other
        }
        let cine = l.shot_video.clone().map(PathBuf::from).filter(|v| v.exists());
        let shot = match &speaker {
            _ if cine.is_some() => Shot::Clip,
            None => Shot::Full,
            Some(a) if a.mask.is_none() => Shot::Card,
            Some(_) => if listener.is_some() { shot_hint } else { Shot::CloseUp },
        };
        let clip = cine.clone().or_else(|| sc.clip.clone().map(PathBuf::from).filter(|c| c.exists()));
        Some(Segment {
            shot, background: bg, clip,
            emoji: None, // no emoji stickers: they distract from the story
            speaker, listener, audio: Some(PathBuf::from(audio)), duration: l.duration.max(0.6) + pause,
            text: l.text.clone(), ambience: ambience_for(&sc.visual, &sc.title, sfx), motion,
            scene_id: Some(sc.id.clone()), line_id: Some(l.id.clone()),
            label: if c.is_narrator { "Narrator".into() } else { c.name.clone() }, ..blank.clone()
        })
    };

    let mut segs = vec![];
    // cold open: the most dramatic line as a hook
    if let Some((si, li)) = hook_line(p) {
        if let Some(mut s) = line_seg(si, li, 0, Shot::CloseUp, None, None) {
            if s.shot == Shot::TwoShot {
                s.shot = Shot::CloseUp;
            }
            if s.shot == Shot::Clip {
                s.background = None;
            }
            s.listener = None;
            s.fade_out = true;
            s.ambience = None;
            s.chapter = Some("தொடக்கம்".into());
            s.label = format!("Hook: {}", s.label);
            segs.push(s);
        }
    }
    let first_bg = p.scenes.iter().find_map(|s| s.image.clone()).map(PathBuf::from)
        .or_else(|| p.background_image.clone().map(PathBuf::from));
    let title_chapter = if segs.is_empty() { Some("தொடக்கம்".to_string()) } else { None };
    segs.push(Segment {
        kind: SegKind::Title { title: p.title.clone(), subtitle: p.logline.clone() },
        background: first_bg, duration: 2.6, fade_in: true, fade_out: true,
        sfx: vec![(sfx.ding.clone(), 0.25, 0.45)], chapter: title_chapter, label: "Title card".into(), ..blank.clone()
    });
    let mut motion = 1;
    for (si, sc) in p.scenes.iter().enumerate() {
        let bg = sc.image.clone().map(PathBuf::from).or_else(|| p.background_image.clone().map(PathBuf::from));
        let amb = ambience_for(&sc.visual, &sc.title, sfx);
        let mut chapter = Some(if sc.title.is_empty() { format!("காட்சி {}", si + 1) } else { sc.title.clone() });
        // No scene title cards: they stall the story (viewers drop off) - the scene title stays a
        // YouTube chapter name on the scene's first shot.
        if false && !sc.title.is_empty() {
            segs.push(Segment {
                kind: SegKind::SceneCard { title: sc.title.clone() }, background: bg.clone(), duration: 2.2,
                fade_in: true, ambience: amb.clone(), sfx: vec![(sfx.whoosh.clone(), 0.0, 0.55)],
                motion, chapter: chapter.take(), scene_id: Some(sc.id.clone()), label: "Scene card".into(), ..blank.clone()
            });
            motion += 1;
        }
        let n = sc.lines.len();
        let mut last_speaker: Option<String> = None;
        let mut dialog_count = 0usize;
        for li in 0..n {
            let line = &sc.lines[li];
            if line.kind == "question" {
                // viewer questions interrupt the story - not shown any more
                continue;
                #[allow(unreachable_code)]
                segs.push(Segment {
                    kind: SegKind::Question { text: line.text.clone() }, background: bg.clone(), duration: 4.2,
                    fade_in: true, ambience: amb.clone(),
                    sfx: vec![(sfx.pop.clone(), 0.1, 0.4), (sfx.tick.clone(), 0.8, 0.6), (sfx.tick.clone(), 1.8, 0.6), (sfx.tick.clone(), 2.8, 0.6)],
                    chapter: chapter.take(), scene_id: Some(sc.id.clone()), line_id: Some(line.id.clone()),
                    label: "Viewer question".into(), ..blank.clone()
                });
                continue;
            }
            if line.kind == "insert" {
                let clip = line.clip.clone().map(PathBuf::from).filter(|c| c.exists());
                let img = line.image.clone().map(PathBuf::from).filter(|c| c.exists());
                if clip.is_some() || img.is_some() {
                    segs.push(Segment {
                        kind: SegKind::Line, shot: Shot::Full, clip, background: img.or(bg.clone()), duration: 3.0,
                        ambience: amb.clone(), sfx: vec![(sfx.whoosh.clone(), 0.0, 0.25)], motion,
                        chapter: chapter.take(), scene_id: Some(sc.id.clone()), line_id: Some(line.id.clone()),
                        label: "Close-up".into(), text: line.text.clone(), ..blank.clone()
                    });
                    motion += 1;
                }
                continue;
            }
            let speaker_of = |k: usize| sc.lines.get(k).and_then(|l| p.character(&l.character_id))
                .filter(|c| !c.is_narrator).map(|c| c.id.clone());
            let prev = if li > 0 { speaker_of(li - 1) } else { None }.or(last_speaker.clone());
            let next = speaker_of(li + 1);
            // a conversation opens on a two-shot, continues with shot / reverse-shot
            // close-ups, and returns to the two-shot every 4th line
            let hint = if dialog_count % 4 == 0 { Shot::TwoShot } else { Shot::CloseUp };
            if let Some(mut s) = line_seg(si, li, motion, hint, prev.as_deref(), next.as_deref()) {
                if s.speaker.is_some() {
                    dialog_count += 1;
                    last_speaker = Some(sc.lines[li].character_id.clone());
                } else {
                    dialog_count = 0;
                }
                if s.emoji.is_some() {
                    s.sfx.push((sfx.pop.clone(), 0.25, 0.35));
                }
                s.fade_out = li + 1 == n;
                s.chapter = chapter.take();
                segs.push(s);
                motion += 1;
            }
        }
    }
    let last_bg = p.scenes.iter().rev().find_map(|s| s.image.clone()).map(PathBuf::from);
    segs.push(Segment {
        kind: SegKind::EndCard { question: "கதை பிடிச்சிருந்தா லைக் பண்ணுங்க!".into() },
        background: last_bg, duration: 4.0, fade_in: true, fade_out: true,
        sfx: vec![(sfx.whoosh.clone(), 0.0, 0.5), (sfx.pop.clone(), 1.0, 0.45)],
        chapter: Some("முடிவு".into()), label: "End card".into(), ..blank.clone()
    });
    segs
}

/// YouTube description with chapters (from the shot plan) and hashtags.
pub fn youtube_description(p: &Project, segs: &[Segment]) -> String {
    let mut t = 0.0;
    let mut chapters: Vec<(f64, String)> = vec![];
    for s in segs {
        if let Some(c) = &s.chapter {
            if chapters.last().map(|(_, n)| n != c).unwrap_or(true) {
                chapters.push((t, c.clone()));
            }
        }
        t += s.duration.max(0.5);
    }
    let fmt = |x: f64| {
        let s = x.round() as u64;
        format!("{}:{:02}", s / 60, s % 60)
    };
    let mut out = format!("{}\n\n{}\n\n", p.title, p.logline);
    if chapters.len() >= 3 {
        out += "அத்தியாயங்கள் / Chapters\n";
        for (i, (at, name)) in chapters.iter().enumerate() {
            out += &format!("{} {}\n", if i == 0 { "0:00".to_string() } else { fmt(*at) }, name);
        }
        out += "\n";
    }
    let names: Vec<String> = p.characters.iter().filter(|c| !c.is_narrator).map(|c| c.name.clone()).collect();
    if !names.is_empty() {
        out += &format!("கதாபாத்திரங்கள்: {}\n\n", names.join(", "));
    }
    out += "இந்தக் கதை உங்களுக்கு பிடிச்சிருந்தா லைக் பண்ணுங்க, ஷேர் பண்ணுங்க, சப்ஸ்கிரைப் பண்ணுங்க!\n\n";
    out += "This video was made with AI tools (voices, images and animation).\n\n";
    out += "#TamilStories #தமிழ்கதைகள் #TamilKathaigal #MoralStories #TamilCartoon #StoryTime";
    out
}


/// Editable timeline: one entry per shot with its time range and provenance.
pub fn timeline(segs: &[Segment]) -> Vec<serde_json::Value> {
    let mut t = 0.0;
    segs.iter().map(|s| {
        let d = s.duration.max(0.5);
        let kind = match &s.kind {
            SegKind::Title { .. } => "title", SegKind::SceneCard { .. } => "scene", SegKind::EndCard { .. } => "end",
            SegKind::Question { .. } => "question",
            SegKind::Line => if s.label == "Close-up" { "insert" } else if s.speaker.is_some() || s.shot == Shot::Clip { "dialogue" } else { "narration" },
        };
        let v = serde_json::json!({"start": (t * 100.0_f64).round() / 100.0, "end": ((t + d) * 100.0_f64).round() / 100.0,
            "kind": kind, "label": s.label, "text": s.text, "scene_id": s.scene_id, "line_id": s.line_id,
            "hook": s.label.starts_with("Hook:")});
        t += d;
        v
    }).collect()
}

/// Tamil subtitles (.srt) for YouTube closed captions.
pub fn srt(segs: &[Segment]) -> String {
    let ts = |x: f64| {
        let ms = (x * 1000.0).round() as u64;
        format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000)
    };
    let mut out = String::new();
    let (mut t, mut n) = (0.0, 1);
    for s in segs {
        let d = s.duration.max(0.5);
        if s.kind == SegKind::Line && !s.text.is_empty() && s.label != "Close-up" {
            out += &format!("{n}\n{} --> {}\n{}\n\n", ts(t + 0.05), ts(t + d - 0.3), s.text);
            n += 1;
        }
        t += d;
    }
    out
}


/// Everything that affects a composed shot, so identical shots are reused from the cache.
pub fn segment_key(s: &Segment, l: &Layout, subtitles: bool, nvenc: bool) -> String {
    let sig = |p: &Option<PathBuf>| p.as_ref().and_then(|p| std::fs::metadata(p).ok()
        .map(|m| format!("{}:{}:{:?}", p.display(), m.len(), m.modified().ok()))).unwrap_or_default();
    let actor = |a: &Option<Actor>| a.as_ref().map(|a| format!("{}|{}|{}|{}", sig(&a.video), sig(&Some(a.image.clone())), sig(&a.mask), a.right)).unwrap_or_default();
    let kind = match &s.kind {
        SegKind::Title { title, subtitle } => format!("T|{title}|{subtitle}"),
        SegKind::SceneCard { title } => format!("S|{title}"),
        SegKind::Line => "L".into(),
        SegKind::EndCard { question } => format!("E|{question}"),
        SegKind::Question { text } => format!("Q|{text}"),
    };
    let sfx: Vec<String> = s.sfx.iter().map(|(f, a, v)| format!("{}@{a}x{v}", f.display())).collect();
    format!("seg-v3|{}x{}|{subtitles}|{nvenc}|{kind}|{:?}|{}|{}|{}|{}|{}|{:.3}|{}|{}|{}|{}|{}|{}|{}",
        l.w, l.h, s.shot as u8, sig(&s.background), sig(&s.clip), actor(&s.speaker), actor(&s.listener), sig(&s.audio),
        s.duration, s.text, sig(&s.emoji), sig(&s.ambience), sfx.join(","), s.fade_in, s.fade_out, s.motion)
}

/// Re-encode an external clip (channel intro / outro) to the segment format.
pub fn normalize_clip(src: &Path, out: &Path, l: &Layout, nvenc: bool) -> Result<()> {
    let mut cmd = Command::new(ffmpeg());
    cmd.args(["-y", "-v", "error", "-i"]).arg(src).args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo",
        "-filter_complex", &format!("[0:v]fps={FPS},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,format=yuv420p[v];[0:a][1:a]amix=inputs=2:duration=first:normalize=0,aresample=48000,aformat=channel_layouts=stereo[a]", w = l.w, h = l.h),
        "-map", "[v]", "-map", "[a]", "-r", &FPS.to_string()]);
    cmd.args(venc(nvenc));
    cmd.args(["-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-movflags", "+faststart"]).arg(out);
    if run(&mut cmd).is_ok() {
        return Ok(());
    }
    // clips without an audio track
    let mut cmd = Command::new(ffmpeg());
    cmd.args(["-y", "-v", "error", "-i"]).arg(src).args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo",
        "-filter_complex", &format!("[0:v]fps={FPS},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,format=yuv420p[v]", w = l.w, h = l.h),
        "-map", "[v]", "-map", "1:a", "-shortest", "-r", &FPS.to_string()]);
    cmd.args(venc(nvenc));
    cmd.args(["-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-movflags", "+faststart"]).arg(out);
    run(&mut cmd)
}
