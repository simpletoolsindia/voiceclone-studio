//! Optional MiniMax H3 (Hailuo 3.0) scene clips via the paid MiniMax API.
//!
//! H3 is far too large to run locally (33B transformer + 32B text encoder), so
//! this is an opt-in cloud step: each scene's painted image becomes the first
//! frame of a short cinematic clip. Clips are silent (`generate_audio: false`);
//! all Tamil voices still come from the local cloned voices.
//! API: POST /v2/video_generation, GET /v2/query/video_generation/{task_id}.

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

const BASE: &str = "https://api.minimax.io";

pub async fn scene_clip(api_key: &str, model: &str, image: &str, visual: &str, _vertical: bool, out: &Path,
                        cancelled: &(dyn Fn() -> bool + Sync)) -> Result<()> {
    let bytes = std::fs::read(image).context("scene image missing")?;
    let data_uri = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
    let model = if model.is_empty() { "MiniMax-H3-Lite" } else { model };
    let prompt = format!("{visual}. Slow cinematic camera movement, gentle natural motion, people idle and breathing, \
consistent lighting, no text, no subtitles, no talking.");
    let client = reqwest::Client::builder().timeout(Duration::from_secs(120)).build()?;
    let body = json!({
        "model": model,
        "content": [
            {"type": "text", "text": prompt},
            {"type": "image_url", "role": "first_frame", "image_url": {"url": data_uri}}
        ],
        "resolution": "768P",
        "duration": 6,
        "generate_audio": false,
        "response_format": "flat"
    });
    let resp = client.post(format!("{BASE}/v2/video_generation")).bearer_auth(api_key.trim())
        .json(&body).send().await.context("could not reach MiniMax")?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    let task_id = v["task_id"].as_str().map(String::from).ok_or_else(|| {
        let msg = v["base_resp"]["status_msg"].as_str().or(v["error"]["message"].as_str()).unwrap_or("request rejected");
        anyhow!("MiniMax ({status}): {msg}")
    })?;
    // Poll (clips usually take 1-5 minutes).
    for _ in 0..240 {
        for _ in 0..10 {
            if cancelled() {
                return Err(anyhow!("cancelled"));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let q: Value = client.get(format!("{BASE}/v2/query/video_generation/{task_id}"))
            .bearer_auth(api_key.trim()).send().await?.json().await?;
        let t = if q.get("task").is_some() { &q["task"] } else { &q };
        match t["status"].as_str().unwrap_or("") {
            "succeeded" => {
                let url = t["content"]["url"].as_str().ok_or_else(|| anyhow!("MiniMax returned no video URL"))?;
                let bytes = client.get(url).timeout(Duration::from_secs(300)).send().await?.bytes().await?;
                std::fs::write(out, &bytes)?;
                return Ok(());
            }
            "failed" | "cancelled" => {
                return Err(anyhow!("MiniMax: {}", t["error"]["message"].as_str().unwrap_or("generation failed")));
            }
            _ => {}
        }
    }
    Err(anyhow!("MiniMax clip timed out"))
}
