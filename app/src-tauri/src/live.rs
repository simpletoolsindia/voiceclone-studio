//! Live microphone -> cloned voice.
//!
//! Real-time audio I/O lives here in Rust (cpal): the input callback only
//! copies samples into a bounded queue and the output callback only reads
//! from one, so no Python, GPU work or allocation-heavy code ever runs on the
//! audio threads. A tokio task streams fixed-size blocks to the GPU worker
//! over a local WebSocket and queues the converted audio for playback.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use tokio_tungstenite::tungstenite::Message;

#[derive(Serialize)]
pub struct Devices {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub default_input: Option<String>,
    pub default_output: Option<String>,
}

pub fn list_devices() -> Devices {
    let host = cpal::default_host();
    let names = |it: Option<Vec<cpal::Device>>| -> Vec<String> {
        it.unwrap_or_default().iter().filter_map(|d| d.name().ok()).collect()
    };
    Devices {
        inputs: names(host.input_devices().ok().map(|i| i.collect())),
        outputs: names(host.output_devices().ok().map(|i| i.collect())),
        default_input: host.default_input_device().and_then(|d| d.name().ok()),
        default_output: host.default_output_device().and_then(|d| d.name().ok()),
    }
}

#[derive(Deserialize, Clone)]
pub struct LiveParams {
    pub voice_id: String,
    pub input: Option<String>,
    pub output: Option<String>,
    #[serde(default = "d_block")]
    pub block_time: f64,
    #[serde(default = "d_steps")]
    pub diffusion_steps: u32,
    #[serde(default = "d_gate")]
    pub gate_db: f64,
    #[serde(default = "d_gain")]
    pub output_gain: f32,
}
fn d_block() -> f64 { 0.25 }
fn d_steps() -> u32 { 8 }
fn d_gate() -> f64 { -50.0 }
fn d_gain() -> f32 { 1.0 }

#[derive(Default)]
struct Shared {
    input: Mutex<VecDeque<f32>>,
    input_cv: Condvar,
    output: Mutex<VecDeque<f32>>,
    stop: AtomicBool,
    infer_ms: AtomicU64,   // f64 bits
    in_level: AtomicU64,   // f64 bits (dBFS)
    dropped: AtomicU64,
    blocks: AtomicU64,
    status: Mutex<String>,
    error: Mutex<Option<String>>,
}

#[derive(Serialize)]
pub struct LiveState {
    pub running: bool,
    pub status: String,
    pub error: Option<String>,
    pub infer_ms: f64,
    pub input_db: f64,
    pub latency_ms: f64,
    pub blocks: u64,
    pub dropped: u64,
}

pub struct Live {
    shared: Mutex<Option<Arc<Shared>>>,
    latency_ms: Mutex<f64>,
}

impl Default for Live {
    fn default() -> Self {
        Self { shared: Mutex::new(None), latency_ms: Mutex::new(0.0) }
    }
}

fn find_device(host: &cpal::Host, name: &Option<String>, input: bool) -> Result<cpal::Device> {
    if let Some(n) = name.as_ref().filter(|n| !n.is_empty()) {
        let mut it: Box<dyn Iterator<Item = cpal::Device>> = if input {
            Box::new(host.input_devices()?)
        } else {
            Box::new(host.output_devices()?)
        };
        if let Some(d) = it.find(|d| d.name().map(|x| &x == n).unwrap_or(false)) {
            return Ok(d);
        }
    }
    if input { host.default_input_device() } else { host.default_output_device() }
        .ok_or_else(|| anyhow!(if input { "No microphone found." } else { "No speaker/output device found." }))
}

impl Live {
    pub fn state(&self) -> LiveState {
        let g = self.shared.lock().unwrap();
        match g.as_ref() {
            None => LiveState { running: false, status: "idle".into(), error: None, infer_ms: 0.0,
                input_db: -120.0, latency_ms: 0.0, blocks: 0, dropped: 0 },
            Some(s) => LiveState {
                running: !s.stop.load(Ordering::Relaxed),
                status: s.status.lock().unwrap().clone(),
                error: s.error.lock().unwrap().clone(),
                infer_ms: f64::from_bits(s.infer_ms.load(Ordering::Relaxed)),
                input_db: f64::from_bits(s.in_level.load(Ordering::Relaxed)),
                latency_ms: *self.latency_ms.lock().unwrap(),
                blocks: s.blocks.load(Ordering::Relaxed),
                dropped: s.dropped.load(Ordering::Relaxed),
            },
        }
    }

    pub fn stop(&self) {
        if let Some(s) = self.shared.lock().unwrap().take() {
            s.stop.store(true, Ordering::Relaxed);
            s.input_cv.notify_all();
        }
    }

    pub fn start(&self, ws_url: String, p: LiveParams) -> Result<()> {
        self.stop();
        let shared = Arc::new(Shared::default());
        *shared.status.lock().unwrap() = "opening audio devices".into();
        *self.shared.lock().unwrap() = Some(shared.clone());

        // Probe the input sample rate up front (the worker must know it).
        let host = cpal::default_host();
        let in_dev = find_device(&host, &p.input, true)?;
        let in_sr = in_dev.default_input_config()?.sample_rate().0;
        *self.latency_ms.lock().unwrap() = (p.block_time * 2.0 + 0.07) * 1000.0;

        // Audio thread: owns the (non-Send) cpal streams for their lifetime.
        let sh = shared.clone();
        let params = p.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32, String>>();
        std::thread::Builder::new().name("live-audio".into()).spawn(move || {
            match open_streams(&sh, &params) {
                Ok((ins, outs, out_sr)) => {
                    let _ = ready_tx.send(Ok(out_sr));
                    while !sh.stop.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    drop(ins);
                    drop(outs);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                }
            }
        })?;
        let out_sr = ready_rx.recv().map_err(|_| anyhow!("audio thread failed"))?.map_err(|e| anyhow!(e))?;

        // Network/inference task.
        let sh = shared.clone();
        tokio::spawn(async move {
            if let Err(e) = stream_loop(&sh, ws_url, p, in_sr, out_sr).await {
                *sh.error.lock().unwrap() = Some(e.to_string());
                *sh.status.lock().unwrap() = "error".into();
            }
            sh.stop.store(true, Ordering::Relaxed);
        });
        Ok(())
    }
}

fn open_streams(sh: &Arc<Shared>, p: &LiveParams) -> Result<(cpal::Stream, cpal::Stream, u32)> {
    let host = cpal::default_host();
    let in_dev = find_device(&host, &p.input, true)?;
    let out_dev = find_device(&host, &p.output, false)?;
    let in_cfg = in_dev.default_input_config()?;
    let out_cfg = out_dev.default_output_config()?;
    let in_ch = in_cfg.channels() as usize;
    let out_ch = out_cfg.channels() as usize;
    let out_sr = out_cfg.sample_rate().0;
    let max_in = (in_cfg.sample_rate().0 as f64 * 3.0) as usize; // never buffer more than 3 s
    let err = |e| eprintln!("audio stream error: {e}");

    let shi = sh.clone();
    let on_input = move |mono: &mut dyn Iterator<Item = f32>| {
        let mut q = shi.input.lock().unwrap();
        let mut sum = 0.0f64;
        let mut n = 0usize;
        for s in mono {
            q.push_back(s);
            sum += (s as f64) * (s as f64);
            n += 1;
        }
        while q.len() > max_in {
            q.pop_front();
        }
        drop(q);
        if n > 0 {
            let db = 10.0 * (sum / n as f64 + 1e-12).log10();
            shi.in_level.store(db.to_bits(), Ordering::Relaxed);
        }
        shi.input_cv.notify_one();
    };

    let in_stream = match in_cfg.sample_format() {
        cpal::SampleFormat::F32 => {
            let f = on_input;
            in_dev.build_input_stream(&in_cfg.config(), move |d: &[f32], _| {
                f(&mut d.chunks(in_ch).map(|c| c.iter().sum::<f32>() / in_ch as f32))
            }, err, None)?
        }
        cpal::SampleFormat::I16 => {
            let f = on_input;
            in_dev.build_input_stream(&in_cfg.config(), move |d: &[i16], _| {
                f(&mut d.chunks(in_ch).map(|c| c.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / in_ch as f32))
            }, err, None)?
        }
        fmt => return Err(anyhow!("unsupported microphone sample format {fmt:?}")),
    };

    let sho = sh.clone();
    let gain = p.output_gain;
    let out_stream = match out_cfg.sample_format() {
        cpal::SampleFormat::F32 => out_dev.build_output_stream(&out_cfg.config(), move |d: &mut [f32], _| {
            let mut q = sho.output.lock().unwrap();
            for frame in d.chunks_mut(out_ch) {
                let s = q.pop_front().unwrap_or(0.0) * gain;
                frame.iter_mut().for_each(|x| *x = s);
            }
        }, err, None)?,
        cpal::SampleFormat::I16 => out_dev.build_output_stream(&out_cfg.config(), move |d: &mut [i16], _| {
            let mut q = sho.output.lock().unwrap();
            for frame in d.chunks_mut(out_ch) {
                let s = ((q.pop_front().unwrap_or(0.0) * gain).clamp(-1.0, 1.0) * 32767.0) as i16;
                frame.iter_mut().for_each(|x| *x = s);
            }
        }, err, None)?,
        fmt => return Err(anyhow!("unsupported speaker sample format {fmt:?}")),
    };
    in_stream.play()?;
    out_stream.play()?;
    Ok((in_stream, out_stream, out_sr))
}

/// Linear resampler (output device rate may differ from the microphone's).
fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let n = ((x.len() as u64 * to as u64) / from as u64) as usize;
    let ratio = from as f64 / to as f64;
    (0..n)
        .map(|i| {
            let pos = i as f64 * ratio;
            let j = pos.floor() as usize;
            let f = (pos - j as f64) as f32;
            let a = x[j.min(x.len() - 1)];
            let b = x[(j + 1).min(x.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

async fn stream_loop(sh: &Arc<Shared>, url: String, p: LiveParams, in_sr: u32, out_sr: u32) -> Result<()> {
    *sh.status.lock().unwrap() = "loading real-time model".into();
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await?;
    let cfg = serde_json::json!({
        "voice_id": p.voice_id, "sample_rate": in_sr, "block_time": p.block_time,
        "diffusion_steps": p.diffusion_steps, "gate_db": p.gate_db,
    });
    ws.send(Message::Text(cfg.to_string())).await?;
    let block_size = loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => {
                let v: serde_json::Value = serde_json::from_str(&t)?;
                match v["status"].as_str() {
                    Some("ready") => break v["block_size"].as_u64().unwrap_or(0) as usize,
                    Some("error") => return Err(anyhow!(v["error"].as_str().unwrap_or("engine error").to_string())),
                    _ => {}
                }
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(e.into()),
            None => return Err(anyhow!("engine closed the live connection")),
        }
    };
    if block_size == 0 {
        return Err(anyhow!("invalid block size"));
    }
    sh.input.lock().unwrap().clear();
    *sh.status.lock().unwrap() = "running".into();
    let max_out = (out_sr as f64 * (p.block_time * 3.0)) as usize;

    while !sh.stop.load(Ordering::Relaxed) {
        // Wait (off the async runtime) for one full block of microphone audio.
        let shc = sh.clone();
        let block: Option<Vec<f32>> = tokio::task::spawn_blocking(move || {
            let mut q = shc.input.lock().unwrap();
            while q.len() < block_size && !shc.stop.load(Ordering::Relaxed) {
                q = shc.input_cv.wait_timeout(q, std::time::Duration::from_millis(100)).unwrap().0;
            }
            if shc.stop.load(Ordering::Relaxed) {
                return None;
            }
            // If inference fell behind, skip stale audio to keep latency bounded.
            while q.len() >= block_size * 3 {
                q.drain(..block_size);
                shc.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Some(q.drain(..block_size).collect())
        })
        .await?;
        let Some(block) = block else { break };
        let bytes: Vec<u8> = block.iter().flat_map(|s| s.to_le_bytes()).collect();
        ws.send(Message::Binary(bytes)).await?;
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => {
                    let samples: Vec<f32> =
                        b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                    let samples = resample(&samples, in_sr, out_sr);
                    let mut q = sh.output.lock().unwrap();
                    q.extend(samples);
                    while q.len() > max_out {
                        q.pop_front();
                    }
                    sh.blocks.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                Some(Ok(Message::Text(t))) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                        if let Some(ms) = v["infer_ms"].as_f64() {
                            sh.infer_ms.store(ms.to_bits(), Ordering::Relaxed);
                        }
                        if v["status"] == "error" {
                            return Err(anyhow!(v["error"].as_str().unwrap_or("engine error").to_string()));
                        }
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
                None => return Err(anyhow!("engine closed the live connection")),
            }
        }
    }
    let _ = ws.close(None).await;
    *sh.status.lock().unwrap() = "stopped".into();
    Ok(())
}
