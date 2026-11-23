use serde::{Deserialize, Serialize};

/// App settings, stored as JSON in the user data folder.
/// API keys are optional and only used for the opt-in cloud features.
#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    #[serde(default)]
    pub anthropic_api_key: String,
    #[serde(default)]
    pub minimax_api_key: String,
    #[serde(default = "d_mm")]
    pub minimax_model: String,
    #[serde(default = "d_writer")]
    pub writer: String,          // "ollama" | "claude"
    #[serde(default)]
    pub ollama_model: String,
    #[serde(default = "d_true")]
    pub close_to_tray: bool,     // keep running (and rendering) in the background
    #[serde(default = "d_true")]
    pub autostart_engine: bool,
    /// Use an existing AI engine folder (e.g. a source checkout) instead of the bundled one
    #[serde(default)]
    pub engine_dir: String,
    /// Channel intro / outro clips added to every story video
    #[serde(default)]
    pub intro_video: String,
    #[serde(default)]
    pub outro_video: String,
}
fn d_mm() -> String { "MiniMax-H3-Lite".into() }
fn d_writer() -> String { "ollama".into() }
fn d_true() -> bool { true }

impl Default for Settings {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap()
    }
}

pub fn load() -> Settings {
    std::fs::read(crate::paths::settings_path()).ok()
        .and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(s: &Settings) -> anyhow::Result<()> {
    std::fs::write(crate::paths::settings_path(), serde_json::to_vec_pretty(s)?)?;
    Ok(())
}

/// Settings as shown to the UI: keys are masked, never sent back in full.
pub fn public(s: &Settings) -> serde_json::Value {
    let mask = |k: &str| if k.len() > 8 { format!("{}…{}", &k[..4], &k[k.len() - 4..]) } else if k.is_empty() { String::new() } else { "••••".into() };
    serde_json::json!({
        "anthropic_api_key": mask(&s.anthropic_api_key), "has_anthropic_key": !s.anthropic_api_key.is_empty(),
        "minimax_api_key": mask(&s.minimax_api_key), "has_minimax_key": !s.minimax_api_key.is_empty(),
        "minimax_model": s.minimax_model, "writer": s.writer, "ollama_model": s.ollama_model,
        "close_to_tray": s.close_to_tray, "autostart_engine": s.autostart_engine, "engine_dir": s.engine_dir,
        "intro_video": s.intro_video, "outro_video": s.outro_video,
    })
}
