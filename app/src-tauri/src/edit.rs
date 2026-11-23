//! Post-render fixes ("at 1:22 change Appa's hair to grey", "fix lip-sync",
//! "change the words to ...") and exports (subtitles, MP3, Shorts teaser).
//!
//! A request is interpreted into a FixPlan the user can review and edit; applying
//! it clears only the affected assets, so the next render regenerates just that
//! part and reuses everything else from the cache.

use crate::story::{self, Project};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct FixPlan {
    /// lipsync | text | voice | animate | look | scene | emotion | delete
    pub action: String,
    pub line_id: Option<String>,
    pub scene_id: Option<String>,
    pub character_id: Option<String>,
    /// human-readable description of what will happen
    pub summary: String,
    /// what the shot currently is (for the review card)
    pub target: String,
    pub at: f64,
    // values the user can edit before applying
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub look: String,
    #[serde(default)]
    pub visual: String,
    #[serde(default)]
    pub motion: String,
    #[serde(default)]
    pub emotion: String,
    #[serde(default)]
    pub speed: Option<f64>,
    /// field the user still has to fill in (e.g. "look" when no colour was given)
    #[serde(default)]
    pub needs: Option<String>,
    pub shots: usize,
    pub estimate_sec: f64,
}

fn fmt_t(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn has(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

/// Which timeline shot is at time `t` (prefers spoken / insert shots near the cards).
fn shot_at(p: &Project, t: f64) -> Option<serde_json::Value> {
    let tl = &p.timeline;
    let hit = tl.iter().find(|s| s["start"].as_f64().unwrap_or(0.0) <= t && t < s["end"].as_f64().unwrap_or(0.0)).cloned();
    match &hit {
        Some(s) if s["line_id"].is_string() => hit,
        _ => tl.iter().filter(|s| s["line_id"].is_string() && s["kind"] != "question")
            .min_by(|a, b| {
                let da = (a["start"].as_f64().unwrap_or(0.0) - t).abs();
                let db = (b["start"].as_f64().unwrap_or(0.0) - t).abs();
                da.partial_cmp(&db).unwrap()
            }).cloned(),
    }
}

/// Character named (or described by role) in the request.
fn character_in(p: &Project, req: &str) -> Option<String> {
    let lower = req.to_lowercase();
    if let Some(c) = p.characters.iter().filter(|c| !c.name.is_empty()).find(|c| lower.contains(&c.name.to_lowercase())) {
        return Some(c.id.clone());
    }
    let roles: [(&[&str], &[&str]); 8] = [
        (&["father", "dad", "appa", "அப்பா", "uncle", "மாமா"], &["uncle", "man"]),
        (&["mother", "mom", "amma", "அம்மா", "aunty", "அத்தை"], &["aunty", "girl"]),
        (&["grandma", "granny", "paati", "patti", "பாட்டி"], &["grandma"]),
        (&["grandpa", "thatha", "தாத்தா"], &["grandpa"]),
        (&["little girl", "sister", "தங்கை", "பாப்பா"], &["child_girl", "girl"]),
        (&["little boy", "brother", "தம்பி"], &["child_boy", "boy"]),
        (&["girl", "பெண்"], &["girl", "child_girl"]),
        (&["boy", "பையன்"], &["boy", "child_boy"]),
    ];
    for (words, presets) in roles {
        if words.iter().any(|w| lower.contains(w)) {
            for pr in presets {
                if let Some(c) = p.characters.iter().find(|c| !c.is_narrator && c.preset == *pr) {
                    return Some(c.id.clone());
                }
            }
        }
    }
    if has(&lower, &["narrator", "கதைசொல்லி"]) {
        return p.characters.iter().find(|c| c.is_narrator).map(|c| c.id.clone());
    }
    None
}

/// Text in quotes, or any Tamil-script run, as the new words.
fn quoted_or_tamil(req: &str) -> Option<String> {
    for (a, b) in [('"', '"'), ('“', '”'), ('\'', '\''), ('‘', '’')] {
        if let (Some(i), Some(j)) = (req.find(a), req.rfind(b)) {
            if j > i + a.len_utf8() {
                let t = req[i + a.len_utf8()..j].trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    let tamil: String = req.chars().filter(|c| ('\u{0B80}'..='\u{0BFF}').contains(c) || matches!(c, ' ' | ',' | '.' | '?' | '!')).collect();
    let tamil = tamil.trim().to_string();
    let words = tamil.split_whitespace().count();
    if words >= 3 { Some(tamil) } else { None }
}

/// Strip time stamps, filler verbs and the character name, keeping the description.
fn description(req: &str, name: &str) -> String {
    let re = regex::Regex::new(r"(?i)\b(at\s*)?\d{1,2}:\d{2}\b|\b(please|pls|update|change|make|fix|set|the|to|his|her|their|in|on|scene|shot)\b").unwrap();
    let mut d = re.replace_all(req, " ").to_string();
    if !name.is_empty() {
        d = d.replace(name, " ");
    }
    for w in ["father's", "father", "mother's", "mother", "grandma's", "grandma", "grandpa's", "grandpa", "boy's", "girl's",
              "appa's", "appa", "amma's", "amma", "paati's", "paati", "patti's", "patti", "thatha's", "thatha", "dad's", "dad", "mom's", "mom",
              "give", "with", "a", "an", "his", "her"] {
        d = regex::Regex::new(&format!(r"(?i)\b{}\b", regex::escape(w))).unwrap().replace_all(&d, " ").to_string();
    }
    let d = d.split_whitespace().collect::<Vec<_>>().join(" ");
    d.trim_matches(|c: char| c == ',' || c == '.' || c == '\'' || c.is_whitespace()).to_string()
}

/// "shirt to blue" -> "blue shirt"
fn x_to_y(req: &str) -> String {
    regex::Regex::new(r"(?i)\b(hair|shirt|saree|sari|dress|kurta|veshti|glasses|beard|moustache|eyes|skin|clothes|outfit|top|blouse)\s+(?:colou?r\s+)?to\s+([a-z]+(?:\s+[a-z]+)?)")
        .unwrap().replace_all(req, "$2 $1").to_string()
}

/// Merge a look change into the old look, dropping clauses about the same thing.
fn merge_look(base: &str, change: &str) -> String {
    const NOUNS: [&str; 16] = ["hair", "shirt", "saree", "sari", "dress", "kurta", "veshti", "glasses", "beard", "moustache",
        "mustache", "eyes", "skin", "clothes", "outfit", "blouse"];
    let ch = change.to_lowercase();
    let kept: Vec<&str> = base.split(',').map(str::trim)
        .filter(|c| !c.is_empty() && !NOUNS.iter().any(|n| ch.contains(n) && c.to_lowercase().contains(n)))
        .collect();
    let mut out = kept.join(", ");
    if !change.is_empty() {
        if !out.is_empty() { out.push_str(", "); }
        out.push_str(change);
    }
    out
}

fn has_detail(desc: &str) -> bool {
    let colors = ["red", "blue", "green", "yellow", "black", "white", "grey", "gray", "silver", "brown", "pink", "purple",
        "orange", "golden", "gold", "dark", "light", "curly", "short", "long", "bald", "beard", "moustache", "glasses",
        "shirt", "saree", "dress", "kurta", "veshti", "older", "younger", "fair", "tan", "smiling"];
    let d = desc.to_lowercase();
    colors.iter().any(|c| d.contains(c)) || d.split_whitespace().count() >= 3
}

pub fn interpret(p: &Project, at: f64, request: &str) -> Result<FixPlan> {
    let req = request.trim();
    let lower = req.to_lowercase();
    // time can also be written inside the request ("at 1:22 ...")
    let at = regex::Regex::new(r"(\d{1,2}):(\d{2})").unwrap().captures(req)
        .map(|c| c[1].parse::<f64>().unwrap_or(0.0) * 60.0 + c[2].parse::<f64>().unwrap_or(0.0)).unwrap_or(at);
    let shot = shot_at(p, at).ok_or_else(|| anyhow!("Render the video first - there is no timeline yet."))?;
    let line_id = shot["line_id"].as_str().map(String::from);
    let scene_id = shot["scene_id"].as_str().map(String::from);
    let line = line_id.as_ref().and_then(|id| p.scenes.iter().flat_map(|s| s.lines.iter()).find(|l| &l.id == id)).cloned();
    let speaker = line.as_ref().and_then(|l| p.character(&l.character_id)).cloned();
    let named = character_in(p, req).and_then(|id| p.character(&id).cloned());
    let who = named.clone().or(speaker.clone());
    let who_name = who.as_ref().map(|c| c.name.clone()).unwrap_or_default();
    let target = format!("{} · {} · \"{}\"", fmt_t(shot["start"].as_f64().unwrap_or(0.0)), shot["label"].as_str().unwrap_or(""),
        line.as_ref().map(|l| l.text.chars().take(60).collect::<String>()).unwrap_or_default());
    let mut plan = FixPlan { line_id: line_id.clone(), scene_id: scene_id.clone(), character_id: who.as_ref().map(|c| c.id.clone()),
        target, at, ..Default::default() };

    let look_words = ["face", "hair", "colour", "color", "shirt", "dress", "saree", "sari", "glasses", "beard", "moustache",
        "mustache", "look", "skin", "older", "younger", "clothes", "outfit", "முகம்", "முடி", "கலர்", "சட்டை", "புடவை"];
    if has(&lower, &["lip", "lipsync", "lip-sync", "lip sync", "mouth", "sync", "உதடு", "வாய்"]) {
        plan.action = "lipsync".into();
        plan.summary = "Redo the lip-sync for this line with stronger, more accurate mouth movement.".into();
    } else if has(&lower, &["delete", "remove this", "cut this", "நீக்கு"]) {
        plan.action = "delete".into();
        plan.summary = "Remove this line from the story.".into();
    } else if has(&lower, &["background", "scene", "room", "place", "sky", "lighting", "daytime", "night", "பின்னணி"])
        && !has(&lower, &look_words) {
        plan.action = "scene".into();
        let sc = scene_id.as_ref().and_then(|id| p.scenes.iter().find(|s| &s.id == id));
        let desc = description(req, &who_name);
        plan.visual = format!("{}{}{}", sc.map(|s| s.visual.as_str()).unwrap_or(""), if desc.is_empty() { "" } else { ", " }, desc);
        plan.summary = "Repaint this scene's background and re-animate the shots in it.".into();
    } else if has(&lower, &look_words) {
        plan.action = "look".into();
        let c = who.clone().ok_or_else(|| anyhow!("Which character? Mention their name or role (e.g. Appa)."))?;
        if c.is_narrator {
            return Err(anyhow!("The narrator has no face on screen. Name the character whose look should change."));
        }
        let desc = description(&x_to_y(req), &c.name);
        let base = if c.look.is_empty() { String::new() } else { c.look.clone() };
        plan.look = if has_detail(&desc) { merge_look(&base, &desc) } else { base };
        if !has_detail(&desc) {
            plan.needs = Some("look".into());
        }
        plan.summary = format!("Repaint {}'s face with the new look and regenerate every shot they appear in.", c.name);
    } else if quoted_or_tamil(req).is_none()
        && has(&lower, &["voice", "slow", "fast", "louder", "softer", "pronounc", "tone", "record", "குரல்", "உச்சரிப்பு"]) {
        plan.action = "voice".into();
        if has(&lower, &["slow"]) { plan.speed = Some(0.88); }
        if has(&lower, &["fast", "quick"]) { plan.speed = Some(1.12); }
        plan.summary = format!("Record a new take of this line{}, then re-sync the lips.",
            match plan.speed { Some(s) if s < 1.0 => " a little slower", Some(_) => " a little faster", None => "" });
    } else if has(&lower, &["word", "text", "say", "said", "line", "sentence", "spelling", "dialogue", "வார்த்தை", "வசனம்"])
        || quoted_or_tamil(req).is_some() {
        plan.action = "text".into();
        match quoted_or_tamil(req) {
            Some(t) => plan.text = t,
            None => {
                plan.text = line.as_ref().map(|l| l.text.clone()).unwrap_or_default();
                plan.needs = Some("text".into());
            }
        }
        plan.summary = "Change the words of this line, record it again and re-sync the lips.".into();
    } else if has(&lower, &["voice", "sound", "audio", "pronounc", "tone", "slow", "fast", "louder", "clear", "குரல்", "உச்சரிப்பு"]) {
        plan.action = "voice".into();
        if has(&lower, &["slow"]) { plan.speed = Some(0.88); }
        if has(&lower, &["fast", "quick"]) { plan.speed = Some(1.12); }
        plan.summary = format!("Record a new take of this line{}, then re-sync the lips.",
            match plan.speed { Some(s) if s < 1.0 => " a little slower", Some(_) => " a little faster", None => "" });
    } else if has(&lower, &["happy", "sad", "angry", "excited", "surprised", "scared", "emotion", "mood", "smile", "cry"]) {
        plan.action = "emotion".into();
        plan.emotion = ["happy", "sad", "angry", "excited", "surprised", "scared"].iter().find(|e| lower.contains(*e)).map(|e| e.to_string())
            .unwrap_or_else(|| line.as_ref().map(|l| l.emotion.clone()).unwrap_or_default());
        plan.motion = description(req, &who_name);
        plan.summary = "Change the emotion, then record, animate and lip-sync this line again.".into();
    } else {
        plan.action = "animate".into();
        let desc = description(req, &who_name);
        plan.motion = if desc.split_whitespace().count() >= 2 { desc } else { line.as_ref().map(|l| l.action.clone()).unwrap_or_default() };
        plan.summary = "Re-animate this shot with new motion, then re-sync the lips.".into();
    }
    plan.shots = affected_lines(p, &plan).len().max(1);
    plan.estimate_sec = estimate(&plan);
    Ok(plan)
}

/// Lines whose shots must be regenerated.
fn affected_lines(p: &Project, plan: &FixPlan) -> Vec<String> {
    match plan.action.as_str() {
        "look" => {
            let cid = plan.character_id.clone().unwrap_or_default();
            p.scenes.iter().filter(|s| s.lines.iter().any(|l| l.character_id == cid))
                .flat_map(|s| s.lines.iter().filter(|l| l.is_dialogue()).map(|l| l.id.clone())).collect()
        }
        "scene" => p.scenes.iter().filter(|s| Some(&s.id) == plan.scene_id.as_ref())
            .flat_map(|s| s.lines.iter().map(|l| l.id.clone())).collect(),
        "delete" => vec![],
        _ => plan.line_id.iter().cloned().collect(),
    }
}

fn estimate(plan: &FixPlan) -> f64 {
    let n = plan.shots as f64;
    let compose = 25.0;
    compose + match plan.action.as_str() {
        "lipsync" => 70.0,
        "voice" | "text" | "emotion" => 10.0 + 70.0 + 30.0,
        "animate" => 40.0 + 70.0,
        "look" => 30.0 + 15.0 + n * 25.0 + 45.0 + n * 25.0 + 60.0,
        "scene" => 40.0 + n * 25.0 + 45.0 + n * 25.0 + 60.0,
        _ => 0.0,
    }
}

/// Apply a reviewed plan: change the story and clear exactly the affected assets.
pub fn apply(p: &mut Project, plan: &FixPlan) -> Result<String> {
    let dir = p.dir();
    let affected = affected_lines(p, plan);
    let clear_shot = |l: &mut story::Line| {
        l.shot_video = None;
        l.talking_video = None;
    };
    let line_id = plan.line_id.clone().unwrap_or_default();
    let find = |p: &mut Project| -> Option<(usize, usize)> {
        for (si, s) in p.scenes.iter().enumerate() {
            if let Some(li) = s.lines.iter().position(|l| l.id == line_id) {
                return Some((si, li));
            }
        }
        None
    };
    let note = match plan.action.as_str() {
        "lipsync" => {
            let (si, li) = find(p).ok_or_else(|| anyhow!("line not found"))?;
            let l = &mut p.scenes[si].lines[li];
            l.sync_boost = true;
            clear_shot(l);
            format!("Fixed lip-sync at {}", fmt_t(plan.at))
        }
        "text" => {
            let (si, li) = find(p).ok_or_else(|| anyhow!("line not found"))?;
            let t = story::clean_line(&plan.text);
            if t.is_empty() {
                return Err(anyhow!("Type the new words for this line."));
            }
            let l = &mut p.scenes[si].lines[li];
            l.text = t;
            l.audio = None;
            l.duration = 0.0;
            l.clip = None;
            clear_shot(l);
            format!("New words at {}", fmt_t(plan.at))
        }
        "voice" | "emotion" => {
            let (si, li) = find(p).ok_or_else(|| anyhow!("line not found"))?;
            let l = &mut p.scenes[si].lines[li];
            l.take += 1;
            if plan.speed.is_some() {
                l.speed = plan.speed;
            }
            if plan.action == "emotion" {
                if !plan.emotion.is_empty() { l.emotion = plan.emotion.clone(); }
                if !plan.motion.is_empty() { l.action = plan.motion.clone(); }
                l.clip = None;
            }
            l.audio = None;
            l.duration = 0.0;
            clear_shot(l);
            format!("New {} at {}", if plan.action == "voice" { "voice take" } else { "emotion" }, fmt_t(plan.at))
        }
        "animate" => {
            let (si, li) = find(p).ok_or_else(|| anyhow!("line not found"))?;
            let l = &mut p.scenes[si].lines[li];
            l.take += 1;
            if !plan.motion.is_empty() && l.is_dialogue() {
                l.action = plan.motion.clone();
            }
            l.clip = None;
            clear_shot(l);
            format!("New animation at {}", fmt_t(plan.at))
        }
        "look" => {
            let cid = plan.character_id.clone().ok_or_else(|| anyhow!("no character"))?;
            if plan.look.trim().is_empty() {
                return Err(anyhow!("Describe the new look (e.g. 'grey hair, blue shirt')."));
            }
            let name;
            {
                let c = p.characters.iter_mut().find(|c| c.id == cid).ok_or_else(|| anyhow!("character not found"))?;
                name = c.name.clone();
                c.look = plan.look.trim().to_string();
                if let Some(f) = c.face_image.take() {
                    let fp = PathBuf::from(&f);
                    let _ = std::fs::remove_file(fp.with_file_name(format!("{}_mask.png", c.id)));
                    if !f.contains("uploads") {
                        let _ = std::fs::remove_file(&fp);
                    }
                }
            }
            for s in p.scenes.iter_mut() {
                for l in s.lines.iter_mut() {
                    if affected.contains(&l.id) {
                        l.clip = None;
                        clear_shot(l);
                    }
                }
            }
            format!("New look for {name}")
        }
        "scene" => {
            let sid = plan.scene_id.clone().ok_or_else(|| anyhow!("no scene"))?;
            let s = p.scenes.iter_mut().find(|s| s.id == sid).ok_or_else(|| anyhow!("scene not found"))?;
            if !plan.visual.trim().is_empty() {
                s.visual = plan.visual.trim().to_string();
            }
            s.image = None;
            s.plate = None;
            s.clip = None;
            for l in s.lines.iter_mut() {
                if l.is_dialogue() {
                    l.clip = None;
                    clear_shot(l);
                }
            }
            let _ = std::fs::remove_file(dir.join("thumbnail_bg.png"));
            format!("Repainted scene \"{}\"", s.title)
        }
        "delete" => {
            let (si, li) = find(p).ok_or_else(|| anyhow!("line not found"))?;
            p.scenes[si].lines.remove(li);
            format!("Removed the line at {}", fmt_t(plan.at))
        }
        other => return Err(anyhow!("unknown action {other}")),
    };
    p.pending_note = if p.pending_note.is_empty() { note.clone() } else { format!("{}; {}", p.pending_note, note) };
    Ok(note)
}

// ---------------------------------------------------------------- exports

fn ff(args: &[&str]) -> Result<()> {
    let mut c = std::process::Command::new(crate::video::ffmpeg());
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let o = c.output()?;
    if !o.status.success() {
        return Err(anyhow!("ffmpeg: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("")));
    }
    Ok(())
}

pub fn export(p: &Project, kind: &str) -> Result<PathBuf> {
    let video = PathBuf::from(p.output.clone().ok_or_else(|| anyhow!("Render the video first."))?);
    let dir = p.dir();
    let stem = video.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "story".into());
    match kind {
        "srt" => {
            let f = dir.join("subtitles.srt");
            if !f.exists() {
                return Err(anyhow!("Subtitles are created with the next render."));
            }
            Ok(f)
        }
        "mp3" => {
            let out = dir.join(format!("{stem}.mp3"));
            ff(&["-y", "-v", "error", "-i", &video.to_string_lossy(), "-vn", "-c:a", "libmp3lame", "-b:a", "192k", &out.to_string_lossy()])?;
            Ok(out)
        }
        "shorts" => shorts_teaser(p, &video, &dir.join(format!("{stem} - Shorts teaser.mp4"))),
        other => Err(anyhow!("unknown export {other}")),
    }
}

/// A ≤ 58 s vertical teaser: the hook, the most dramatic lines and the viewer question,
/// letterboxed on a blurred copy of the frame, with the title on top.
fn shorts_teaser(p: &Project, video: &PathBuf, out: &PathBuf) -> Result<PathBuf> {
    let mut picks: Vec<(f64, f64)> = vec![];
    let mut total = 0.0;
    let score = |s: &serde_json::Value| -> f64 {
        let t = s["text"].as_str().unwrap_or("");
        let mut sc = t.matches(['!', '?']).count() as f64;
        if s["hook"].as_bool().unwrap_or(false) { sc += 10.0; }
        if s["kind"] == "question" { sc += 6.0; }
        if s["kind"] == "insert" { sc += 1.5; }
        sc
    };
    let mut cands: Vec<&serde_json::Value> = p.timeline.iter()
        .filter(|s| matches!(s["kind"].as_str(), Some("dialogue" | "question" | "insert"))).collect();
    cands.sort_by(|a, b| score(b).partial_cmp(&score(a)).unwrap());
    for s in cands {
        let (a, b) = (s["start"].as_f64().unwrap_or(0.0), s["end"].as_f64().unwrap_or(0.0));
        if b - a < 0.8 || total + (b - a) > 55.0 {
            continue;
        }
        picks.push((a, b));
        total += b - a;
    }
    if picks.is_empty() {
        return Err(anyhow!("No timeline yet - render the video first."));
    }
    picks.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
    let mut f = String::new();
    for (i, (a, b)) in picks.iter().enumerate() {
        f += &format!("[0:v]trim={a:.2}:{b:.2},setpts=PTS-STARTPTS[v{i}];[0:a]atrim={a:.2}:{b:.2},asetpts=PTS-STARTPTS[a{i}];");
    }
    for i in 0..picks.len() {
        f += &format!("[v{i}][a{i}]");
    }
    let ass = out.with_extension("ass");
    std::fs::write(&ass, format!("[Script Info]\nPlayResX: 1080\nPlayResY: 1920\nScaledBorderAndShadow: yes\n\n[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: T,Nirmala UI,78,&H0000E6FF,&H000000FF,&H00000000,&HA0000000,-1,0,0,0,100,100,0,0,1,6,4,8,60,60,180,1\n\
Style: B,Nirmala UI,52,&H00FFFFFF,&H000000FF,&H00000000,&HA0000000,-1,0,0,0,100,100,0,0,1,4,3,2,60,60,200,1\n\n[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:00.00,0:10:00.00,T,,0,0,0,,{}\nDialogue: 0,0:00:00.00,0:10:00.00,B,,0,0,0,,முழு கதை எங்க சேனல்ல! ▶\n",
        p.title.replace('{', "(").replace('}', ")")))?;
    f += &format!("concat=n={}:v=1:a=1[cv][ca];[cv]split[bgs][fgs];\
[bgs]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,boxblur=24:2,eq=brightness=-0.15[bg];\
[fgs]scale=1080:-2[fg];[bg][fg]overlay=(W-w)/2:(H-h)/2,ass='{}':shaping=complex,format=yuv420p[v]",
        picks.len(), ass.file_name().unwrap().to_string_lossy());
    let mut c = std::process::Command::new(crate::video::ffmpeg());
    c.current_dir(out.parent().unwrap()).args(["-y", "-v", "error", "-i"]).arg(video)
        .args(["-filter_complex", &f, "-map", "[v]", "-map", "[ca]", "-c:v", "libx264", "-preset", "medium", "-crf", "20",
            "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"]).arg(out);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let o = c.output()?;
    let _ = std::fs::remove_file(&ass);
    if !o.status.success() {
        return Err(anyhow!("ffmpeg: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("")));
    }
    Ok(out.clone())
}
