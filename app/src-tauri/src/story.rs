//! Story projects: characters, scenes and dialogue lines, the plain-text
//! script format users can paste or share, and AI story writing (local
//! Ollama model, or Claude with an API key).

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

pub const PRESET_KEYS: [&str; 9] =
    ["man", "girl", "boy", "aunty", "uncle", "grandpa", "grandma", "child_boy", "child_girl"];

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Character {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub preset: String,
    #[serde(default)]
    pub voice_id: Option<String>,
    #[serde(default)]
    pub look: String,
    #[serde(default)]
    pub face_image: Option<String>,
    #[serde(default)]
    pub is_narrator: bool,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Line {
    pub id: String,
    pub character_id: String,
    pub text: String,
    #[serde(default)]
    pub emotion: String,
    #[serde(default)]
    pub audio: Option<String>,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub talking_video: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Scene {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub visual: String,
    #[serde(default)]
    pub image: Option<String>,
    /// Empty version of the location (no people), used behind character cutouts
    #[serde(default)]
    pub plate: Option<String>,
    #[serde(default)]
    pub clip: Option<String>,
    pub lines: Vec<Line>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub logline: String,
    #[serde(default = "d_lang")]
    pub language: String,
    /// "landscape" (1920x1080, YouTube) or "shorts" (1080x1920)
    #[serde(default = "d_format")]
    pub format: String,
    /// image look: "animated" (3D animated film) | "cinematic" (photoreal) | "storybook" (painted)
    #[serde(default = "d_style")]
    pub style: String,
    pub characters: Vec<Character>,
    pub scenes: Vec<Scene>,
    #[serde(default)]
    pub music: Option<String>,
    #[serde(default = "d_music_vol")]
    pub music_volume: f64,
    #[serde(default)]
    pub background_image: Option<String>,
    #[serde(default = "d_true")]
    pub subtitles: bool,
    #[serde(default = "d_true")]
    pub talking_faces: bool,
    #[serde(default)]
    pub use_minimax: bool,
    #[serde(default)]
    pub thumbnail_prompt: String,
    #[serde(default)]
    pub output: Option<String>,
    /// YouTube description (chapters, hashtags) written by the last render
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub updated: u64,
}
fn d_lang() -> String { "ta".into() }
fn d_format() -> String { "landscape".into() }
fn d_style() -> String { "animated".into() }
fn d_music_vol() -> f64 { 0.12 }
fn d_true() -> bool { true }

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..10].to_string()
}

impl Project {
    pub fn new(title: &str) -> Self {
        Project {
            id: new_id(), title: title.into(), logline: String::new(), language: d_lang(),
            format: d_format(), style: d_style(), characters: vec![], scenes: vec![], music: None,
            music_volume: d_music_vol(), background_image: None, subtitles: true, talking_faces: true,
            use_minimax: false, thumbnail_prompt: String::new(), output: None, description: String::new(), updated: now(),
        }
    }

    pub fn dir(&self) -> PathBuf {
        crate::paths::projects_dir().join(&self.id)
    }

    pub fn character(&self, id: &str) -> Option<&Character> {
        self.characters.iter().find(|c| c.id == id)
    }

    /// Render the project as the shareable plain-text script.
    pub fn to_script(&self) -> String {
        let mut s = format!("# {}\n", self.title);
        if !self.logline.is_empty() {
            s += &format!("> {}\n", self.logline);
        }
        s += "\n";
        for c in &self.characters {
            s += &format!("@{} = {}{}\n", c.name, c.preset,
                if c.look.is_empty() { String::new() } else { format!(" | {}", c.look) });
        }
        for sc in &self.scenes {
            s += &format!("\n[Scene: {}{}]\n", sc.title,
                if sc.visual.is_empty() { String::new() } else { format!(" | {}", sc.visual) });
            for l in &sc.lines {
                let name = self.character(&l.character_id).map(|c| c.name.as_str()).unwrap_or("?");
                let emo = if l.emotion.is_empty() { String::new() } else { format!(" ({})", l.emotion) };
                s += &format!("{}{}: {}\n", name, emo, l.text);
            }
        }
        s
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Guess a voice preset from a character name (Tamil or English kinship words).
pub fn guess_preset(name: &str) -> &'static str {
    let n = name.to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| n.contains(w));
    if has(&["பாட்டி", "paati", "patti", "grandma", "granny", "ammachi", "ஆச்சி"]) { "grandma" }
    else if has(&["தாத்தா", "thatha", "grandpa", "grandfather"]) { "grandpa" }
    else if has(&["மாமா", "uncle", "அங்கிள்", "chithappa", "சித்தப்பா", "periyappa", "அப்பா", "appa"]) { "uncle" }
    else if has(&["அத்தை", "aunty", "auntie", "ஆன்ட்டி", "chithi", "சித்தி", "அம்மா", "amma", "periyamma"]) { "aunty" }
    else if has(&["பாப்பா", "papa", "little girl", "சிறுமி", "kutty girl"]) { "child_girl" }
    else if has(&["தம்பி", "thambi", "little boy", "சிறுவன்", "kutty", "குட்டி"]) { "child_boy" }
    else if has(&["அக்கா", "akka", "girl", "பெண்", "தங்கை", "thangachi", "ponnu"]) { "girl" }
    else if has(&["பையன்", "boy", "teen", "student", "payyan"]) { "boy" }
    else if has(&["narrator", "கதைசொல்லி", "storyteller"]) { "grandma" }
    else { "man" }
}

/// Parse the plain-text script format:
///
/// ```text
/// # Title
/// > one-line summary
/// @Paati = grandma | kind elderly woman, silver hair
/// [Scene: Village morning | misty paddy fields at sunrise]
/// Paati (warm): வாடா கண்ணு...
/// Narrator: ஒரு ஊர்ல...
/// ```
/// Unknown speakers become new characters with a guessed voice preset.
pub fn parse_script(text: &str, base: Option<&Project>) -> Project {
    let mut p = base.cloned().unwrap_or_else(|| Project::new("Untitled story"));
    p.scenes.clear();
    let mut chars: Vec<Character> = base.map(|b| b.characters.clone()).unwrap_or_default();
    let line_re = regex::Regex::new(r"^\s*([^:\[\]@#>]{1,40}?)\s*(?:\(([^)]{1,30})\))?\s*[:：]\s*(.+)$").unwrap();
    let find_or_add = |chars: &mut Vec<Character>, name: &str| -> String {
        let key = name.trim();
        if let Some(c) = chars.iter().find(|c| c.name.eq_ignore_ascii_case(key)) {
            return c.id.clone();
        }
        let preset = guess_preset(key).to_string();
        let c = Character {
            id: new_id(), name: key.into(), preset,
            is_narrator: key.eq_ignore_ascii_case("narrator") || key.contains("கதைசொல்லி"),
            ..Default::default()
        };
        let id = c.id.clone();
        chars.push(c);
        id
    };
    for raw in text.lines() {
        let l = raw.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(t) = l.strip_prefix("# ") {
            p.title = t.trim().into();
        } else if let Some(t) = l.strip_prefix('>') {
            p.logline = t.trim().into();
        } else if let Some(def) = l.strip_prefix('@') {
            let (name, rest) = def.split_once('=').unwrap_or((def, ""));
            let (preset, look) = rest.split_once('|').unwrap_or((rest, ""));
            let id = find_or_add(&mut chars, name);
            if let Some(c) = chars.iter_mut().find(|c| c.id == id) {
                let pr = preset.trim();
                if PRESET_KEYS.contains(&pr) {
                    c.preset = pr.into();
                }
                if !look.trim().is_empty() {
                    c.look = look.trim().into();
                }
            }
        } else if l.starts_with("[Scene") || l.starts_with("[காட்சி") {
            let inner = l.trim_start_matches('[').trim_end_matches(']');
            let inner = inner.split_once(':').map(|x| x.1).unwrap_or("");
            let (title, visual) = inner.split_once('|').unwrap_or((inner, ""));
            p.scenes.push(Scene { id: new_id(), title: title.trim().into(), visual: visual.trim().into(),
                ..Default::default() });
        } else if let Some(cap) = line_re.captures(l) {
            let name = cap.get(1).unwrap().as_str();
            let cid = find_or_add(&mut chars, name);
            if p.scenes.is_empty() {
                p.scenes.push(Scene { id: new_id(), title: "Scene 1".into(), ..Default::default() });
            }
            p.scenes.last_mut().unwrap().lines.push(Line {
                id: new_id(), character_id: cid,
                emotion: cap.get(2).map(|m| m.as_str().trim().to_string()).unwrap_or_default(),
                text: clean_line(cap.get(3).unwrap().as_str()), ..Default::default()
            });
        } else if let Some(sc) = p.scenes.last_mut() {
            // A bare paragraph is narration.
            let cid = find_or_add(&mut chars, "Narrator");
            sc.lines.push(Line { id: new_id(), character_id: cid, text: l.into(), ..Default::default() });
        }
    }
    // keep only characters that are used or explicitly declared
    p.characters = chars;
    p.updated = now();
    p
}

/// Clean a spoken line: drop stage directions in brackets and stray
/// characters from other scripts (LLMs sometimes leak Thai/CJK glyphs).
pub fn clean_line(text: &str) -> String {
    let re = regex::Regex::new(r"\([^)]*\)|\[[^\]]*\]|\*[^*]*\*").unwrap();
    let t = re.replace_all(text, " ");
    let kept: String = t.chars().filter(|c| {
        let u = *c as u32;
        (0x0B80..=0x0BFF).contains(&u)            // Tamil
            || c.is_ascii()                        // Latin letters, digits, punctuation
            || matches!(u, 0x200C | 0x200D | 0x2018 | 0x2019 | 0x201C | 0x201D | 0x2026 | 0x2014 | 0x2013)
    }).collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn mostly_ascii(s: &str) -> bool {
    let n = s.chars().count().max(1);
    s.chars().filter(|c| c.is_ascii()).count() * 10 >= n * 8
}

/// Turn a scene description into an empty-location prompt by dropping the
/// clauses that mention people (cutout characters are composited on top).
pub fn plate_prompt(visual: &str, title: &str) -> String {
    const PEOPLE: [&str; 30] = ["child", "kid", "boy", "girl", "man", "men", "woman", "women", "people", "person",
        "family", "father", "mother", "grand", "uncle", "aunt", "friend", "farmer", "villager", "crowd", "vendor",
        "shopkeeper", "student", "teacher", "baby", "couple", "sitting", "standing", "laughing", "talking"];
    let kept: Vec<&str> = visual.split([',', ';']).map(str::trim)
        .filter(|c| !c.is_empty() && !PEOPLE.iter().any(|w| c.to_lowercase().contains(w)))
        .collect();
    let base = if kept.is_empty() { title.to_string() } else { kept.join(", ") };
    format!("{base}. The place is completely empty: nobody there, no people, no children, no characters, only the environment")
}

// ---------------------------------------------------------------- AI writer

#[derive(Deserialize, Clone)]
pub struct WriteRequest {
    pub idea: String,
    #[serde(default)]
    pub characters: Vec<String>,     // preset keys to include
    #[serde(default = "d_minutes")]
    pub minutes: f64,
    #[serde(default)]
    pub genre: String,
    #[serde(default)]
    pub audience: String,
    /// "ollama" | "claude"
    #[serde(default = "d_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
}
fn d_minutes() -> f64 { 3.0 }
fn d_provider() -> String { "ollama".into() }

fn schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["title", "title_en", "logline", "thumbnail_prompt", "characters", "scenes"],
        "properties": {
            "title": {"type": "string"},
            "title_en": {"type": "string"},
            "logline": {"type": "string"},
            "thumbnail_prompt": {"type": "string"},
            "characters": {"type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["name", "preset", "look"],
                "properties": {
                    "name": {"type": "string"},
                    "preset": {"type": "string", "enum": PRESET_KEYS},
                    "look": {"type": "string"}
                }}},
            "scenes": {"type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["title", "visual", "lines"],
                "properties": {
                    "title": {"type": "string"},
                    "visual": {"type": "string"},
                    "lines": {"type": "array", "items": {
                        "type": "object", "additionalProperties": false,
                        "required": ["speaker", "emotion", "text"],
                        "properties": {
                            "speaker": {"type": "string"},
                            "emotion": {"type": "string"},
                            "text": {"type": "string"}
                        }}}
                }}}
        }
    })
}

const SYSTEM_PROMPT: &str = r#"You are an award-winning Tamil screenwriter who writes viral story videos for a Tamil YouTube channel.

Write in the natural spoken Tamil people use today (Chennai / Tamil Nadu everyday speech), not formal literary Tamil:
- Use spoken forms: "என்ன பண்ற?", "வா போலாம்", "சூப்பரா இருக்கு", "தெரியல", "இருக்கான்", "சொல்லு டா" — not "செய்கிறாய்", "இருக்கிறான்".
- Young characters may mix in common English words the way people really talk ("ஓகே", "சூப்பர்", "ப்ளீஸ்", "ஃப்ரெண்ட்ஸ்"), written in Tamil script. Elders speak warmer, traditional colloquial Tamil with proverbs.
- Short sentences, one or two per line, with natural punctuation (commas, ?, !) so they are easy to speak aloud. Spell numbers out in Tamil words.
- Every line must be in Tamil script. Never use Latin letters inside "text".

Story craft: a hook in the first 10 seconds, a clear problem, rising emotion, a twist or heart-touching moment, and a satisfying ending with a gentle message. Give each character a distinct way of speaking. Keep it family-friendly.

Use a narrator (speaker "Narrator") to set scenes and bridge time; most lines should be dialogue.

For each character choose the voice preset that matches their age and gender: man, girl, boy, aunty, uncle, grandpa, grandma, child_boy, child_girl. The Narrator must also appear in "characters" (preset grandma for a cosy storyteller, man for a dramatic one).

"look" (English): a concise visual description of the character for an image generator — age, face, hair, clothing, distinctive features. Keep it identical in spirit across the story.
"visual" (English): a vivid one-sentence image-generator prompt for the scene background — location, time of day, lighting, mood, Tamil Nadu details. No text or captions in the image.
"thumbnail_prompt" (English): a dramatic YouTube thumbnail scene with the main characters' expressions.
"title": a catchy Tamil title. "title_en": its English translation. "logline": one Tamil sentence."#;

fn user_prompt(r: &WriteRequest) -> String {
    let lines = (r.minutes * 14.0).round().max(8.0) as i64;
    let mut s = format!("Story idea: {}\n\nTarget length: about {:.0} minutes of video (roughly {} spoken lines across 4-8 scenes).\n",
        r.idea.trim(), r.minutes, lines);
    if !r.characters.is_empty() {
        s += &format!("Use these character voice presets (you may name them): {}.\n", r.characters.join(", "));
    }
    if !r.genre.is_empty() {
        s += &format!("Genre / tone: {}.\n", r.genre);
    }
    if !r.audience.is_empty() {
        s += &format!("Audience: {}.\n", r.audience);
    }
    s + "Return only the JSON object."
}

pub async fn ollama_models() -> Result<Vec<String>> {
    let v: Value = reqwest::get("http://127.0.0.1:11434/api/tags").await?.json().await?;
    Ok(v["models"].as_array().cloned().unwrap_or_default().iter()
        .filter_map(|m| m["name"].as_str().map(String::from))
        .filter(|n| !n.ends_with(":cloud"))
        .collect())
}

async fn write_ollama(r: &WriteRequest) -> Result<Value> {
    let model = match &r.model {
        Some(m) if !m.is_empty() => m.clone(),
        _ => {
            let have = ollama_models().await.context("Ollama is not running. Install it from ollama.com or pick Claude.")?;
            ["gemma4:latest", "gemma4:e4b", "qwen3.5:9b"].iter().find(|m| have.iter().any(|h| h == *m))
                .map(|s| s.to_string()).or_else(|| have.first().cloned())
                .ok_or_else(|| anyhow!("No local Ollama model installed. Run: ollama pull gemma4"))?
        }
    };
    let body = json!({
        "model": model, "stream": false, "think": false, "format": schema(),
        "options": {"temperature": 0.8, "num_ctx": 16384},
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": user_prompt(r)}
        ]
    });
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(900)).build()?;
    let v: Value = client.post("http://127.0.0.1:11434/api/chat").json(&body).send().await?.json().await?;
    if let Some(e) = v["error"].as_str() {
        return Err(anyhow!("Ollama: {e}"));
    }
    let text = v["message"]["content"].as_str().ok_or_else(|| anyhow!("empty reply from Ollama"))?;
    serde_json::from_str(text).context("the local model returned invalid JSON - try again or use Claude")
}

async fn write_claude(r: &WriteRequest, api_key: &str) -> Result<Value> {
    if api_key.trim().is_empty() {
        return Err(anyhow!("Add your Anthropic API key in Settings to write stories with Claude."));
    }
    let model = r.model.clone().filter(|m| m.starts_with("claude-")).unwrap_or_else(|| "claude-opus-5-5".into());
    let body = json!({
        "model": model,
        "max_tokens": 16000,
        "fallbacks": "default",
        "system": SYSTEM_PROMPT,
        "output_config": {"effort": "high", "format": {"type": "json_schema", "schema": schema()}},
        "messages": [{"role": "user", "content": user_prompt(r)}]
    });
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(600)).build()?;
    let resp = client.post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key.trim())
        .header("anthropic-version", "2026-10-21")
        .header("anthropic-beta", "server-side-fallback-2026-07-01")
        .header("content-type", "application/json")
        .json(&body).send().await.context("could not reach the Claude API")?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("request failed");
        return Err(anyhow!("Claude API error ({status}): {msg}"));
    }
    match v["stop_reason"].as_str() {
        Some("refusal") => return Err(anyhow!("Claude declined this story idea. Try rephrasing it.")),
        Some("max_tokens") => return Err(anyhow!("The story was too long for one reply - ask for fewer minutes.")),
        _ => {}
    }
    let text: String = v["content"].as_array().cloned().unwrap_or_default().iter()
        .filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
    serde_json::from_str(&text).context("Claude returned an unexpected format")
}

pub async fn write_story(r: &WriteRequest, api_key: &str) -> Result<Project> {
    let v = if r.provider == "claude" { write_claude(r, api_key).await? } else { write_ollama(r).await? };
    Ok(from_ai_json(&v))
}

fn from_ai_json(v: &Value) -> Project {
    let mut p = Project::new(v["title"].as_str().unwrap_or("புதிய கதை"));
    p.logline = v["logline"].as_str().unwrap_or("").into();
    p.thumbnail_prompt = v["thumbnail_prompt"].as_str().unwrap_or("").into();
    let empty = vec![];
    for c in v["characters"].as_array().unwrap_or(&empty) {
        let name = c["name"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() || p.characters.iter().any(|x| x.name == name) {
            continue;
        }
        let preset = c["preset"].as_str().filter(|s| PRESET_KEYS.contains(s)).map(String::from)
            .unwrap_or_else(|| guess_preset(&name).into());
        p.characters.push(Character {
            id: new_id(), is_narrator: name.eq_ignore_ascii_case("narrator"), name, preset,
            // image prompts must be English; otherwise fall back to the preset's portrait description
            look: c["look"].as_str().filter(|l| mostly_ascii(l)).unwrap_or("").into(), ..Default::default()
        });
    }
    for s in v["scenes"].as_array().unwrap_or(&empty) {
        let mut title = s["title"].as_str().unwrap_or("").trim().to_string();
        if title == p.title || p.scenes.iter().any(|x: &Scene| x.title == title) {
            title.clear(); // repeated titles would show the same scene card again
        }
        let mut sc = Scene { id: new_id(), title,
            visual: s["visual"].as_str().unwrap_or("").into(), ..Default::default() };
        for l in s["lines"].as_array().unwrap_or(&empty) {
            let speaker = l["speaker"].as_str().unwrap_or("Narrator").trim().to_string();
            let cid = match p.characters.iter().find(|c| c.name.eq_ignore_ascii_case(&speaker)) {
                Some(c) => c.id.clone(),
                None => {
                    let c = Character { id: new_id(), preset: guess_preset(&speaker).into(),
                        is_narrator: speaker.eq_ignore_ascii_case("narrator"), name: speaker.clone(),
                        ..Default::default() };
                    let id = c.id.clone();
                    p.characters.push(c);
                    id
                }
            };
            let text = clean_line(l["text"].as_str().unwrap_or(""));
            if !text.is_empty() {
                sc.lines.push(Line { id: new_id(), character_id: cid, text,
                    emotion: l["emotion"].as_str().unwrap_or("").into(), ..Default::default() });
            }
        }
        if !sc.lines.is_empty() {
            p.scenes.push(sc);
        }
    }
    // The narrator should not share a voice with an on-screen character.
    let used: Vec<String> = p.characters.iter().filter(|c| !c.is_narrator).map(|c| c.preset.clone()).collect();
    if let Some(n) = p.characters.iter_mut().find(|c| c.is_narrator) {
        if used.contains(&n.preset) {
            if let Some(free) = ["man", "grandma", "uncle", "aunty", "girl", "grandpa"].iter().find(|k| !used.iter().any(|u| u == *k)) {
                n.preset = free.to_string();
            }
        }
    }
    p
}

// ---------------------------------------------------------------- storage

pub fn save(p: &mut Project) -> Result<()> {
    p.updated = now();
    let dir = p.dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("project.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(p)?)?;
    std::fs::rename(tmp, dir.join("project.json"))?;
    Ok(())
}

pub fn load(id: &str) -> Result<Project> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(anyhow!("invalid project id"));
    }
    let path = crate::paths::projects_dir().join(id).join("project.json");
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

pub fn list() -> Vec<Value> {
    let mut out: Vec<Value> = std::fs::read_dir(crate::paths::projects_dir()).into_iter().flatten().flatten()
        .filter_map(|e| std::fs::read(e.path().join("project.json")).ok())
        .filter_map(|b| serde_json::from_slice::<Project>(&b).ok())
        .map(|p| json!({"id": p.id, "title": p.title, "logline": p.logline, "updated": p.updated,
            "scenes": p.scenes.len(), "lines": p.scenes.iter().map(|s| s.lines.len()).sum::<usize>(),
            "output": p.output}))
        .collect();
    out.sort_by(|a, b| b["updated"].as_u64().cmp(&a["updated"].as_u64()));
    out
}
