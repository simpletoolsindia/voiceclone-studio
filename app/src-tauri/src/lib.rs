mod engine;
mod live;
mod minimax;
mod paths;
mod render;
mod settings;
mod story;
mod video;

use engine::Engine;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State, WindowEvent};

struct AppState {
    engine: Arc<Engine>,
    renders: Arc<render::Renders>,
    live: live::Live,
    settings: Mutex<settings::Settings>,
}

type Res<T> = Result<T, String>;
fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

// ------------------------------------------------------------------ engine
#[tauri::command]
fn engine_status(s: State<AppState>) -> Value {
    json!({
        "installed": s.engine.is_installed(),
        "status": *s.engine.status.lock().unwrap(),
        "dir": s.engine.dir,
        "data_dir": paths::data_dir(),
    })
}

#[tauri::command]
async fn engine_start(s: State<'_, AppState>) -> Res<Value> {
    let eng = s.engine.clone();
    tokio::task::spawn_blocking(move || eng.start()).await.map_err(e)?.map_err(e)?;
    Ok(json!({"ok": true}))
}

#[tauri::command]
fn engine_restart(s: State<AppState>) -> Res<()> {
    s.engine.stop();
    let eng = s.engine.clone();
    std::thread::spawn(move || {
        let _ = eng.start();
    });
    Ok(())
}

#[tauri::command]
fn engine_log(s: State<AppState>) -> Vec<String> {
    s.engine.log.lock().unwrap().iter().cloned().collect()
}

/// Proxy for the GPU worker's HTTP API (keeps the auth token out of the webview).
#[tauri::command]
async fn engine_request(s: State<'_, AppState>, method: String, path: String, body: Option<Value>) -> Res<Value> {
    if !path.starts_with('/') || path.contains("..") {
        return Err("bad path".into());
    }
    s.engine.request(&method, &path, body).await.map_err(e)
}

/// Install / repair the AI engine by running the setup script; output goes to the engine log.
#[tauri::command]
fn engine_setup(s: State<AppState>, talking_faces: bool) -> Res<()> {
    let dir = s.engine.dir.clone();
    let script = dir.join("scripts").join("setup_engine.ps1");
    if !script.exists() {
        return Err(format!("setup script not found at {}", script.display()));
    }
    let log = s.engine.log.clone();
    *s.engine.status.lock().unwrap() = "installing".into();
    let eng = s.engine.clone();
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script);
        if talking_faces {
            cmd.arg("-TalkingFaces");
        }
        cmd.current_dir(&dir).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let push = |l: String| {
            let mut g = log.lock().unwrap();
            if g.len() >= 500 {
                g.pop_front();
            }
            g.push_back(l);
        };
        match cmd.spawn() {
            Ok(mut child) => {
                let err = child.stderr.take().unwrap();
                let log2 = log.clone();
                std::thread::spawn(move || {
                    for l in BufReader::new(err).lines().map_while(Result::ok) {
                        log2.lock().unwrap().push_back(l);
                    }
                });
                for l in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
                    push(l);
                }
                let ok = child.wait().map(|s| s.success()).unwrap_or(false);
                push(if ok { "SETUP COMPLETE".into() } else { "SETUP FAILED - see messages above".into() });
                *eng.status.lock().unwrap() = if ok { "stopped".into() } else { "setup failed".into() };
                if ok {
                    let _ = eng.start();
                }
            }
            Err(err) => push(format!("could not run PowerShell: {err}")),
        }
    });
    Ok(())
}

// ------------------------------------------------------------------ jobs
#[tauri::command]
async fn jobs_all(s: State<'_, AppState>) -> Res<Value> {
    let mut all: Vec<Value> = s.renders.list().into_iter().map(|j| serde_json::to_value(j).unwrap()).collect();
    if s.engine.info().is_some() {
        if let Ok(Value::Array(py)) = s.engine.request("GET", "/jobs", None).await {
            all.extend(py);
        }
    }
    Ok(Value::Array(all))
}

#[tauri::command]
async fn job_cancel(s: State<'_, AppState>, id: String) -> Res<bool> {
    if id.starts_with('r') && s.renders.cancel(&id) {
        return Ok(true);
    }
    let v = s.engine.request("POST", &format!("/jobs/{id}/cancel"), None).await.map_err(e)?;
    Ok(v["ok"].as_bool().unwrap_or(false))
}

#[tauri::command]
async fn jobs_clear(s: State<'_, AppState>) -> Res<()> {
    s.renders.clear_finished();
    let _ = s.engine.request("POST", "/jobs/clear", None).await;
    Ok(())
}

// ------------------------------------------------------------------ live
#[tauri::command]
fn live_devices() -> live::Devices {
    live::list_devices()
}

#[tauri::command]
fn live_start(s: State<AppState>, params: live::LiveParams) -> Res<()> {
    let url = s.engine.ws_url("/live/ws").map_err(e)?;
    s.live.start(url, params).map_err(e)
}

#[tauri::command]
fn live_stop(s: State<AppState>) {
    s.live.stop();
}

#[tauri::command]
fn live_state(s: State<AppState>) -> live::LiveState {
    s.live.state()
}

// ------------------------------------------------------------------ stories
#[tauri::command]
fn story_list() -> Vec<Value> {
    story::list()
}

#[tauri::command]
fn story_get(id: String) -> Res<story::Project> {
    story::load(&id).map_err(e)
}

#[tauri::command]
fn story_new(title: String) -> Res<story::Project> {
    let mut p = story::Project::new(if title.trim().is_empty() { "புதிய கதை" } else { title.trim() });
    story::save(&mut p).map_err(e)?;
    Ok(p)
}

#[tauri::command]
fn story_save(mut project: story::Project) -> Res<story::Project> {
    story::save(&mut project).map_err(e)?;
    Ok(project)
}

#[tauri::command]
fn story_delete(id: String) -> Res<()> {
    let p = story::load(&id).map_err(e)?;
    std::fs::remove_dir_all(p.dir()).map_err(e)
}

#[tauri::command]
fn story_script(id: String) -> Res<String> {
    Ok(story::load(&id).map_err(e)?.to_script())
}

/// Replace a project's characters/scenes from pasted script text (keeps voices/faces of known characters).
#[tauri::command]
fn story_from_script(text: String, id: Option<String>) -> Res<story::Project> {
    let base = id.as_deref().and_then(|i| story::load(i).ok());
    let mut p = story::parse_script(&text, base.as_ref());
    story::save(&mut p).map_err(e)?;
    Ok(p)
}

#[tauri::command]
async fn story_write(s: State<'_, AppState>, req: story::WriteRequest) -> Res<story::Project> {
    let key = s.settings.lock().unwrap().anthropic_api_key.clone();
    let mut p = story::write_story(&req, &key).await.map_err(e)?;
    story::save(&mut p).map_err(e)?;
    Ok(p)
}

#[tauri::command]
fn story_render(s: State<AppState>, id: String) -> Res<String> {
    let set = s.settings.lock().unwrap().clone();
    render::start(s.engine.clone(), s.renders.clone(), id, set).map_err(e)
}

#[tauri::command]
async fn ollama_models() -> Res<Vec<String>> {
    story::ollama_models().await.map_err(|_| "Ollama is not running".to_string())
}

// ------------------------------------------------------------------ settings / files
#[tauri::command]
fn settings_get(s: State<AppState>) -> Value {
    settings::public(&s.settings.lock().unwrap())
}

#[tauri::command]
fn settings_set(s: State<AppState>, patch: Value) -> Res<Value> {
    let mut cur = s.settings.lock().unwrap();
    let mut v = serde_json::to_value(&*cur).map_err(e)?;
    if let (Some(obj), Some(p)) = (v.as_object_mut(), patch.as_object()) {
        for (k, val) in p {
            // masked keys come back unchanged from the UI - ignore those
            if k.ends_with("_api_key") && val.as_str().map(|s| s.contains('…') || s == "••••").unwrap_or(false) {
                continue;
            }
            obj.insert(k.clone(), val.clone());
        }
    }
    *cur = serde_json::from_value(v).map_err(e)?;
    settings::save(&cur).map_err(e)?;
    Ok(settings::public(&cur))
}

#[tauri::command]
fn copy_file(src: String, dest: String) -> Res<String> {
    std::fs::copy(&src, &dest).map_err(e)?;
    Ok(dest)
}

#[tauri::command]
fn import_asset(project_id: String, src: String, kind: String) -> Res<String> {
    let p = story::load(&project_id).map_err(e)?;
    let ext = std::path::Path::new(&src).extension().and_then(|x| x.to_str()).unwrap_or("bin").to_lowercase();
    let dir = p.dir().join("uploads");
    std::fs::create_dir_all(&dir).map_err(e)?;
    let dest = dir.join(format!("{}-{}.{}", kind.replace(['/', '\\', '.'], ""), story::new_id(), ext));
    std::fs::copy(&src, &dest).map_err(e)?;
    Ok(dest.to_string_lossy().into())
}

// ------------------------------------------------------------------ app
fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Headless mode: `voiceclone-studio --render <project-id>` renders a story video
/// without opening a window (useful for batch jobs); also `--list` lists stories.
pub fn cli() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--import") => {
            let text = std::fs::read_to_string(args.get(2)?).ok()?;
            let mut p = story::parse_script(&text, None);
            story::save(&mut p).ok()?;
            println!("{}", p.id);
            Some(0)
        }
        Some("--write") => {
            let idea = args.get(2)?.clone();
            let provider = args.get(3).cloned().unwrap_or_else(|| "ollama".into());
            let rt = tokio::runtime::Runtime::new().ok()?;
            let req = story::WriteRequest { idea, characters: vec![], minutes: 2.0, genre: String::new(),
                audience: String::new(), provider, model: args.get(4).cloned() };
            match rt.block_on(story::write_story(&req, &settings::load().anthropic_api_key)) {
                Ok(mut p) => {
                    story::save(&mut p).ok()?;
                    println!("{}\n{}", p.id, p.to_script());
                    Some(0)
                }
                Err(err) => { eprintln!("{err}"); Some(1) }
            }
        }
        Some("--list") => {
            for p in story::list() {
                println!("{}  {}  ({} lines)", p["id"].as_str().unwrap_or(""), p["title"].as_str().unwrap_or(""), p["lines"]);
            }
            Some(0)
        }
        Some("--render") => {
            let id = args.get(2)?.clone();
            let rt = tokio::runtime::Runtime::new().ok()?;
            Some(rt.block_on(async move {
                let eng = Arc::new(Engine::new(engine::find_engine_dir(None)));
                let e2 = eng.clone();
                if let Err(err) = tokio::task::spawn_blocking(move || e2.start()).await.unwrap() {
                    eprintln!("engine: {err}");
                    return 2;
                }
                let renders = Arc::new(render::Renders::default());
                let jid = match render::start(eng.clone(), renders.clone(), id, settings::load()) {
                    Ok(j) => j,
                    Err(err) => { eprintln!("{err}"); eng.stop(); return 2; }
                };
                let mut last = String::new();
                let code = loop {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let Some(j) = renders.list().into_iter().find(|j| j.id == jid) else { break 2 };
                    let line = format!("[{:5.1}%] {}", j.percent, j.stage);
                    if line != last {
                        println!("{line}{}", j.eta_sec.map(|e| format!("  (~{e:.0}s left)")).unwrap_or_default());
                        last = line;
                    }
                    match j.status.as_str() {
                        "done" => { println!("DONE {}", j.result.unwrap_or_default()); break 0 }
                        "error" => { eprintln!("ERROR {}", j.error.unwrap_or_default()); break 1 }
                        "cancelled" => break 1,
                        _ => {}
                    }
                };
                eng.stop();
                code
            }))
        }
        _ => None,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let res = app.path().resource_dir().ok();
            let dir = engine::find_engine_dir(res.as_deref());
            let st = AppState {
                engine: Arc::new(Engine::new(dir)),
                renders: Arc::new(render::Renders::default()),
                live: live::Live::default(),
                settings: Mutex::new(settings::load()),
            };
            let autostart = st.settings.lock().unwrap().autostart_engine;
            let eng = st.engine.clone();
            app.manage(st);
            if autostart {
                std::thread::spawn(move || {
                    let _ = eng.start();
                });
            }

            let show = MenuItem::with_id(app, "show", "Open VoiceClone Studio", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit (stops background jobs)", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("VoiceClone Studio")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id.as_ref() {
                    "show" => show_main(app),
                    "quit" => {
                        if let Some(s) = app.try_state::<AppState>() {
                            s.live.stop();
                            s.engine.stop();
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, .. } = ev {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let to_tray = app.try_state::<AppState>().map(|s| s.settings.lock().unwrap().close_to_tray).unwrap_or(true);
                if to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                } else if let Some(s) = app.try_state::<AppState>() {
                    s.live.stop();
                    s.engine.stop();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            engine_status, engine_start, engine_restart, engine_log, engine_request, engine_setup,
            jobs_all, job_cancel, jobs_clear,
            live_devices, live_start, live_stop, live_state,
            story_list, story_get, story_new, story_save, story_delete, story_script, story_from_script,
            story_write, story_render, ollama_models,
            settings_get, settings_set, copy_file, import_asset
        ])
        .build(tauri::generate_context!())
        .expect("error while building VoiceClone Studio")
        .run(|app, ev| {
            if let tauri::RunEvent::Exit = ev {
                if let Some(s) = app.try_state::<AppState>() {
                    s.live.stop();
                    s.engine.stop();
                }
            }
        });
}
