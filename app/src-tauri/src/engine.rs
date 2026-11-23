//! Supervises the local Python GPU worker (`engine/vcstudio`).
//!
//! The worker listens on 127.0.0.1 with a random per-launch token. Only this
//! process knows the token; the webview reaches the worker exclusively through
//! the `engine_request` command, so no web page can drive it.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
pub struct EngineInfo {
    pub port: u16,
    pub token: String,
}

pub struct Engine {
    pub dir: PathBuf,
    child: Mutex<Option<Child>>,
    info: Mutex<Option<EngineInfo>>,
    pub log: Arc<Mutex<VecDeque<String>>>,
    pub status: Mutex<String>,
    http: reqwest::Client,
}

impl Engine {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            child: Mutex::new(None),
            info: Mutex::new(None),
            log: Arc::new(Mutex::new(VecDeque::with_capacity(500))),
            status: Mutex::new("stopped".into()),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("http client"),
        }
    }

    pub fn python(&self) -> PathBuf {
        if cfg!(windows) {
            self.dir.join(".venv").join("Scripts").join("python.exe")
        } else {
            self.dir.join(".venv").join("bin").join("python")
        }
    }

    pub fn is_installed(&self) -> bool {
        self.python().exists() && self.dir.join("vcstudio").join("server.py").exists()
    }

    pub fn info(&self) -> Option<EngineInfo> {
        self.info.lock().unwrap().clone()
    }

    fn push_log(log: &Arc<Mutex<VecDeque<String>>>, line: String) {
        let mut l = log.lock().unwrap();
        if l.len() >= 500 {
            l.pop_front();
        }
        l.push_back(line);
    }

    /// Start the worker (blocking until it prints its ready line).
    pub fn start(&self) -> Result<EngineInfo> {
        if let Some(i) = self.info() {
            return Ok(i);
        }
        if !self.is_installed() {
            *self.status.lock().unwrap() = "not installed".into();
            return Err(anyhow!("The AI engine is not installed yet (looked in {}). Open Settings and click 'Install engine'.", self.dir.display()));
        }
        *self.status.lock().unwrap() = "starting".into();
        let token: String = {
            use rand::{distributions::Alphanumeric, Rng};
            rand::thread_rng().sample_iter(&Alphanumeric).take(40).map(char::from).collect()
        };
        let mut cmd = Command::new(self.python());
        cmd.args(["-m", "vcstudio.server", "--port", "0"])
            .current_dir(&self.dir)
            .env("VCSTUDIO_TOKEN", &token)
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUNBUFFERED", "1")
            .env("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")
            .env("TOKENIZERS_PARALLELISM", "false")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().context("could not start the Python engine")?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let log = self.log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                Self::push_log(&log, line);
            }
        });
        let (tx, rx) = std::sync::mpsc::channel::<u16>();
        let log2 = self.log.clone();
        std::thread::spawn(move || {
            let mut sent = false;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if !sent {
                    if let Some(p) = line.strip_prefix("VCSTUDIO_READY port=") {
                        if let Ok(port) = p.trim().parse::<u16>() {
                            let _ = tx.send(port);
                            sent = true;
                            continue;
                        }
                    }
                }
                Self::push_log(&log2, line);
            }
        });
        *self.child.lock().unwrap() = Some(child);
        let port = rx
            .recv_timeout(Duration::from_secs(180))
            .map_err(|_| anyhow!("The engine did not start. See Settings > Engine log."))?;
        let info = EngineInfo { port, token };
        *self.info.lock().unwrap() = Some(info.clone());
        *self.status.lock().unwrap() = "running".into();
        Ok(info)
    }

    pub fn stop(&self) {
        *self.info.lock().unwrap() = None;
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        *self.status.lock().unwrap() = "stopped".into();
    }

    pub fn ws_url(&self, path: &str) -> Result<String> {
        let i = self.info().ok_or_else(|| anyhow!("engine not running"))?;
        Ok(format!("ws://127.0.0.1:{}{}?token={}", i.port, path, i.token))
    }

    pub async fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let i = self.info().ok_or_else(|| anyhow!("The engine is not running yet."))?;
        let url = format!("http://127.0.0.1:{}{}", i.port, path);
        let m = reqwest::Method::from_bytes(method.as_bytes())?;
        let mut rb = self.http.request(m, url).header("x-token", &i.token);
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        let resp = rb.send().await.context("engine request failed")?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let detail = v.get("detail").map(|d| d.to_string()).unwrap_or_else(|| status.to_string());
            return Err(anyhow!(detail.trim_matches('"').to_string()));
        }
        Ok(v)
    }

    /// Submit a job and wait for it, forwarding progress to `on_progress`.
    pub async fn run_job<F: FnMut(f64, &str)>(
        &self,
        path: &str,
        body: Value,
        on_progress: F,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Value> {
        self.run_job_pv(path, body, on_progress, cancelled, &|_: &str| {}).await
    }

    /// Like `run_job`, also reporting the job's live preview image path.
    pub async fn run_job_pv<F: FnMut(f64, &str)>(
        &self,
        path: &str,
        body: Value,
        mut on_progress: F,
        cancelled: &(dyn Fn() -> bool + Sync),
        preview: &(dyn Fn(&str) + Sync),
    ) -> Result<Value> {
        let job = self.request("POST", path, Some(body)).await?;
        let id = job["id"].as_str().ok_or_else(|| anyhow!("no job id"))?.to_string();
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            if cancelled() {
                let _ = self.request("POST", &format!("/jobs/{id}/cancel"), None).await;
                return Err(anyhow!("cancelled"));
            }
            let j = self.request("GET", &format!("/jobs/{id}"), None).await?;
            on_progress(j["progress"].as_f64().unwrap_or(0.0), j["stage"].as_str().unwrap_or(""));
            if let Some(pv) = j["preview"].as_str() {
                preview(pv);
            }
            match j["status"].as_str().unwrap_or("") {
                "done" => return Ok(j["result"].clone()),
                "error" => return Err(anyhow!(j["error"].as_str().unwrap_or("job failed").to_string())),
                "cancelled" => return Err(anyhow!("cancelled")),
                _ => {}
            }
        }
    }
}

/// Locate the engine folder: env override, dev checkout, or bundled resources.
pub fn find_engine_dir(resource_dir: Option<&Path>) -> PathBuf {
    if let Ok(p) = std::env::var("VCSTUDIO_ENGINE_DIR") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        // Dev checkout: prefer a folder whose Python environment is installed
        // (Tauri also copies bare resources next to the debug binary).
        for anc in exe.ancestors().skip(1).take(6) {
            let cand = anc.join("engine");
            if cand.join("vcstudio").join("server.py").exists() && cand.join(".venv").exists() {
                return cand;
            }
        }
    }
    // Installed build: code ships as resources, the venv lives in the user data dir.
    let data = crate::paths::data_dir().join("engine");
    if let Some(res) = resource_dir {
        let bundled = res.join("engine");
        if bundled.join("vcstudio").exists() {
            // refresh the engine code on every start so app updates also update the engine
            let _ = copy_dir(&bundled, &data);
        }
    }
    data
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}
