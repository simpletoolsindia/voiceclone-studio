use std::path::PathBuf;

/// Per-user data folder shared with the Python engine (voices, outputs, projects).
pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("VCSTUDIO_DATA_DIR") {
        return PathBuf::from(p);
    }
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let p = base.join("VoiceCloneStudio");
    let _ = std::fs::create_dir_all(&p);
    p
}

pub fn projects_dir() -> PathBuf {
    let p = data_dir().join("projects");
    let _ = std::fs::create_dir_all(&p);
    p
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}
