/* VoiceClone Studio — UI logic (vanilla JS, Tauri v2 globals). */
const T = window.__TAURI__;
const invoke = (cmd, args) => T.core.invoke(cmd, args);
const fileUrl = (p) => (p ? T.core.convertFileSrc(p) : "");
const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

const PRESETS = {
  man: ["Tamil man", "ஆண்", "👨"], girl: ["Tamil girl", "பெண்", "👩"], boy: ["Tamil boy", "பையன்", "🧑"],
  aunty: ["Aunty", "ஆன்ட்டி", "👩‍🦱"], uncle: ["Uncle", "மாமா", "👨‍🦳"], grandpa: ["Grandpa", "தாத்தா", "👴"],
  grandma: ["Grandma", "பாட்டி", "👵"], child_boy: ["Child (boy)", "சிறுவன்", "👦"], child_girl: ["Child (girl)", "சிறுமி", "👧"],
};
const ACCENTS = ["#ffc83c", "#4a9eff", "#7be07b", "#ff6cb0", "#ff5a5a", "#40d0e0", "#ffa040", "#c070ff"];

let voices = [];
let cur = null;          // project being edited
let curRenderJob = null;
let saveTimer = null;
const player = new Audio();

// ------------------------------------------------------------------ helpers
function toast(msg, err = false, ms = 3500) {
  const t = $("#toast");
  t.textContent = msg;
  t.className = "toast" + (err ? " err" : "");
  clearTimeout(t._h);
  t._h = setTimeout(() => t.classList.add("hidden"), ms);
}
async function api(method, path, body) {
  return invoke("engine_request", { method, path, body: body ?? null });
}
function fmtTime(s) {
  if (s == null || !isFinite(s)) return "";
  s = Math.max(0, Math.round(s));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60), r = s % 60;
  if (m < 60) return `${m}m ${String(r).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}
async function pickFile(filters) {
  return T.dialog.open({ multiple: false, directory: false, filters });
}
const AUDIO_FILTER = [{ name: "Audio", extensions: ["wav", "mp3", "m4a", "flac", "ogg", "aac", "opus", "webm", "mp4"] }];
const IMAGE_FILTER = [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }];

function play(path) {
  if (!path) return;
  player.src = fileUrl(path) + "?t=" + Date.now();
  player.play().catch((e) => toast("Cannot play: " + e, true));
}

/** Submit an engine job and resolve with its result when finished. */
async function engineJob(path, body) {
  const job = await api("POST", path, body);
  refreshJobs();
  return waitJob(job.id);
}
function waitJob(id) {
  return new Promise((resolve, reject) => {
    const tick = async () => {
      try {
        const j = await api("GET", `/jobs/${id}`);
        if (j.status === "done") return resolve(j.result);
        if (j.status === "error") return reject(new Error(j.error || "failed"));
        if (j.status === "cancelled") return reject(new Error("cancelled"));
      } catch (e) { /* engine busy; retry */ }
      setTimeout(tick, 700);
    };
    tick();
  });
}

function modal(title, html, okText = "OK", showCancel = true) {
  return new Promise((resolve) => {
    $("#mTitle").textContent = title;
    $("#mBody").innerHTML = html;
    $("#mOk").textContent = okText;
    $("#mCancel").classList.toggle("hidden", !showCancel);
    const d = $("#modal");
    d.onclose = () => resolve(d.returnValue === "ok");
    d.showModal();
  });
}

// ------------------------------------------------------------------ navigation
$$("nav button").forEach((b) => b.addEventListener("click", () => go(b.dataset.page)));
function go(page) {
  $$("nav button").forEach((b) => b.classList.toggle("active", b.dataset.page === page));
  $$(".page").forEach((p) => p.classList.toggle("active", p.id === "page-" + page));
  if (page === "voices") loadVoices();
  if (page === "live") loadDevices();
  if (page === "settings") loadSettings();
  if (page === "jobs") refreshJobs();
}
$("#enginePill").addEventListener("click", () => go("settings"));

// ------------------------------------------------------------------ engine status
let engineReady = false;
async function pollEngine() {
  try {
    const s = await invoke("engine_status");
    const pill = $("#enginePill");
    pill.classList.remove("ok", "bad");
    if (s.status === "running") {
      pill.classList.add("ok");
      $("#engineText").textContent = "AI engine ready";
      if (!engineReady) { engineReady = true; loadVoices(); }
    } else if (!s.installed) {
      pill.classList.add("bad");
      $("#engineText").textContent = s.status === "installing" ? "Installing engine…" : "Engine not installed";
    } else {
      $("#engineText").textContent = s.status === "installing" ? "Installing engine…" : "Engine " + s.status;
      if (s.status === "stopped" || s.status === "setup failed") pill.classList.add("bad");
      engineReady = false;
    }
    $("#setupCard").classList.toggle("hidden", s.status === "running" || (s.installed && s.status !== "installing" && s.status !== "setup failed"));
    if (s.status === "installing" || s.status === "setup failed") showSetupProgress(s.status);
    $("#engInfo") && ($("#engInfo").innerHTML = `Status: <b>${esc(s.status)}</b><br>Engine folder: ${esc(s.dir)}<br>Data folder: ${esc(s.data_dir)}`);
  } catch (e) { /* ignore */ }
  setTimeout(pollEngine, 2500);
}

// ------------------------------------------------------------------ jobs + dock
let lastJobs = [];
async function refreshJobs() {
  try { lastJobs = await invoke("jobs_all"); } catch { return; }
  const running = lastJobs.filter((j) => j.status === "running" || j.status === "queued");
  const badge = $("#jobBadge");
  badge.textContent = running.length;
  badge.classList.toggle("hidden", running.length === 0);
  const jobHtml = (j, compact) => {
    const pct = j.status === "done" ? 100 : (j.percent ?? Math.round((j.progress || 0) * 100));
    const eta = j.paused ? "⏸ paused" : j.eta_sec != null ? `~${fmtTime(j.eta_sec)} left` : j.status === "running" ? "estimating…" : "";
    const right = j.status === "running" || j.status === "queued"
      ? (j.status === "running" ? `<button class="ghost small" data-${j.paused ? "resume" : "pause"}="${j.id}">${j.paused ? "▶ Resume" : "⏸ Pause"}</button>` : "") +
        `<button class="ghost small" data-cancel="${j.id}">Cancel</button>` : `<span class="st">${j.status}</span>`;
    return `<div class="job ${j.status}${j.paused ? " paused" : ""}">
      <div class="job-top"><span class="job-title">${esc(j.title)}</span><span class="job-pct">${pct.toFixed ? pct.toFixed(pct < 10 ? 1 : 0) : pct}%</span></div>
      <div class="progress"><div style="width:${pct}%"></div></div>
      ${j.steps && j.steps.length ? `<div class="job-step">Step ${j.steps.filter((x) => x.status === "done").length + 1} of ${j.steps.filter((x) => x.status !== "skipped").length}: ${esc((j.steps.find((x) => x.status === "running") || {}).label || "")}</div>` : ""}
      <div class="job-sub"><span>${esc(j.error || j.stage || "")}</span><span>${compact ? eta : `${eta} ${j.elapsed_sec ? "· " + fmtTime(j.elapsed_sec) + " elapsed" : ""}`}</span></div>
      ${compact ? "" : `<div class="actions">${right}${j.result && j.result.path ? `<button class="ghost small" data-open="${esc(j.result.path)}">Show file</button>` : ""}</div>`}
      ${compact ? `<div class="actions right" style="margin-top:4px">${right}</div>` : ""}
    </div>`;
  };
  if ($("#page-jobs").classList.contains("active")) {
    $("#jobList").innerHTML = lastJobs.length ? lastJobs.map((j) => jobHtml(j, false)).join("") : `<div class="empty">No jobs yet.</div>`;
  }
  const dock = $("#dock");
  // the story page already shows full details for the render being watched
  const watching = curRenderJob && $("#page-studio").classList.contains("active") && !$("#studioEditor").classList.contains("hidden");
  const dockJobs = running.filter((j) => !(watching && j.id === curRenderJob));
  if (dockJobs.length) {
    dock.innerHTML = `<div class="dock-head"><span>Running in background</span><span>${dockJobs.length} job${dockJobs.length > 1 ? "s" : ""}</span></div>` +
      dockJobs.slice(0, 3).map((j) => jobHtml(j, true)).join("");
    dock.classList.remove("hidden");
  } else dock.classList.add("hidden");
  updateRenderPanel();
}
document.addEventListener("click", async (ev) => {
  const c = ev.target.closest("[data-cancel]");
  if (c) { await invoke("job_cancel", { id: c.dataset.cancel }).catch((e) => toast(e, true)); refreshJobs(); }
  const pz = ev.target.closest("[data-pause]");
  if (pz) { await invoke("job_pause", { id: pz.dataset.pause }).catch((e) => toast(e, true)); toast("⏸ Paused"); refreshJobs(); }
  const rs = ev.target.closest("[data-resume]");
  if (rs) { await invoke("job_resume", { id: rs.dataset.resume }).catch((e) => toast(e, true)); toast("▶ Resumed"); refreshJobs(); }
  const o = ev.target.closest("[data-open]");
  if (o) T.opener.revealItemInDir(o.dataset.open).catch(() => {});
});
$("#jobsClear").addEventListener("click", async () => { await invoke("jobs_clear"); refreshJobs(); });
setInterval(refreshJobs, 1000);

// ------------------------------------------------------------------ voices
async function loadVoices() {
  try { voices = await api("GET", "/voices"); } catch { return; }
  const list = $("#voiceList");
  list.innerHTML = voices.length ? voices.map((v) => `
    <div class="voice">
      <div class="voice-top"><b>${esc(v.name)}</b><span class="tag ${v.kind}">${v.kind === "designed" ? "AI character" : "cloned"} · ${esc(v.language)}</span></div>
      <p>${esc(v.ref_text || "")}</p>
      <div class="actions">
        <button class="ghost small" data-vplay="${v.id}">▶ Sample</button>
        <button class="ghost small" data-vexport="${v.id}">⬇ Download</button>
        <button class="ghost small" data-vrename="${v.id}">✎ Edit</button>
        <button class="ghost small danger" data-vdel="${v.id}">Delete</button>
      </div>
    </div>`).join("") : `<div class="empty">No voices yet — clone one or create the Tamil character voices.</div>`;
  const opts = `<option value="">— choose a voice —</option>` + voices.map((v) => `<option value="${v.id}">${esc(v.name)}${v.kind === "designed" ? " (AI)" : ""}</option>`).join("");
  $$(".voice-select").forEach((s) => { const val = s.value; s.innerHTML = opts; s.value = val || (voices[0]?.id ?? ""); });
  if (cur) renderCharacters();
}
$("#voiceList").addEventListener("click", async (ev) => {
  const b = ev.target.closest("button"); if (!b) return;
  const id = b.dataset.vplay || b.dataset.vexport || b.dataset.vrename || b.dataset.vdel;
  const v = voices.find((x) => x.id === id); if (!v) return;
  try {
    if (b.dataset.vplay) play((await api("GET", `/voices/${id}/sample`)).path);
    if (b.dataset.vexport) {
      const dest = await T.dialog.save({ defaultPath: `${v.name}.vcpt`, filters: [{ name: "Voice checkpoint", extensions: ["vcpt"] }] });
      if (dest) { await api("POST", `/voices/${id}/export`, { path: dest }); toast("Checkpoint saved"); }
    }
    if (b.dataset.vrename) {
      const ok = await modal("Edit voice", `<label>Name<input id="mvName" value="${esc(v.name)}"></label>
        <label>Transcript of the reference clip <span class="hint">(fixing mistakes here improves the clone)</span><textarea id="mvText" rows="3">${esc(v.ref_text)}</textarea></label>`, "Save");
      if (ok) { await api("PATCH", `/voices/${id}`, { name: $("#mvName").value, ref_text: $("#mvText").value }); loadVoices(); }
    }
    if (b.dataset.vdel && (await modal("Delete voice?", `<p>Delete <b>${esc(v.name)}</b>? Download it first if you may need it again.</p>`, "Delete"))) {
      await api("DELETE", `/voices/${id}`); loadVoices();
    }
  } catch (e) { toast(String(e), true); }
});
let cloneFile = null;
$("#cPick").addEventListener("click", async () => {
  const f = await pickFile(AUDIO_FILTER);
  if (f) { cloneFile = f; $("#cFile").textContent = f.split(/[\\/]/).pop(); if (!$("#cName").value) $("#cName").value = f.split(/[\\/]/).pop().replace(/\.\w+$/, ""); }
});
$("#cGo").addEventListener("click", async () => {
  if (!cloneFile) return toast("Choose an audio file first", true);
  const b = $("#cGo"); b.disabled = true;
  try {
    const r = await engineJob("/voices/clone", { name: $("#cName").value || "My voice", path: cloneFile, language: $("#cLang").value, ref_text: $("#cText").value || null });
    toast(`Voice "${r.name}" saved`); loadVoices();
  } catch (e) { toast("Clone failed: " + e.message, true, 6000); }
  b.disabled = false;
});
$("#presetGo").addEventListener("click", async () => {
  try { await engineJob("/voices/presets", { keys: [] }); toast("Character voices ready"); loadVoices(); }
  catch (e) { toast(String(e.message || e), true); }
});
$("#vImport").addEventListener("click", async () => {
  const f = await pickFile([{ name: "Voice checkpoint", extensions: ["vcpt"] }]);
  if (!f) return;
  try { const v = await api("POST", "/voices/import", { path: f }); toast(`Imported "${v.name}"`); loadVoices(); }
  catch (e) { toast(String(e), true); }
});

// ------------------------------------------------------------------ TTS
$("#tText").addEventListener("input", () => { $("#tCount").textContent = `${$("#tText").value.length} characters`; });
let lastTts = null;
$("#tGo").addEventListener("click", async () => {
  const text = $("#tText").value.trim();
  if (!text) return toast("Type something to speak", true);
  const b = $("#tGo"); b.disabled = true;
  try {
    const r = await engineJob("/tts", { text, voice_id: $("#tVoice").value || null, language: $("#tLang").value, mode: $("#tMode").value, speed: $("#tSpeed").value ? +$("#tSpeed").value : null });
    lastTts = r.path; $("#tAudio").src = fileUrl(r.path) + "?t=" + Date.now(); $("#tOut").classList.remove("hidden"); $("#tAudio").play();
  } catch (e) { toast("Speech failed: " + e.message, true, 6000); }
  b.disabled = false;
});
$("#tSave").addEventListener("click", () => saveCopy(lastTts, "speech.wav", "wav"));

// ------------------------------------------------------------------ Convert
let convFile = null, lastConv = null;
$("#vPick").addEventListener("click", async () => { const f = await pickFile(AUDIO_FILTER); if (f) { convFile = f; $("#vFile").textContent = f.split(/[\\/]/).pop(); } });
$("#vGo").addEventListener("click", async () => {
  if (!convFile) return toast("Choose a recording first", true);
  if (!$("#vVoice").value) return toast("Choose a target voice", true);
  const b = $("#vGo"); b.disabled = true;
  try {
    const r = await engineJob("/convert", { voice_id: $("#vVoice").value, input: convFile, steps: +$("#vSteps").value });
    lastConv = r.path; $("#vAudio").src = fileUrl(r.path) + "?t=" + Date.now(); $("#vOut").classList.remove("hidden");
  } catch (e) { toast("Conversion failed: " + e.message, true, 6000); }
  b.disabled = false;
});
$("#vSave").addEventListener("click", () => saveCopy(lastConv, "converted.wav", "wav"));
async function saveCopy(src, name, ext) {
  if (!src) return;
  const dest = await T.dialog.save({ defaultPath: name, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
  if (dest) { await invoke("copy_file", { src, dest }); toast("Saved"); }
}

// ------------------------------------------------------------------ Live
async function loadDevices() {
  try {
    const d = await invoke("live_devices");
    $("#lIn").innerHTML = d.inputs.map((n) => `<option ${n === d.default_input ? "selected" : ""}>${esc(n)}</option>`).join("");
    $("#lOut").innerHTML = d.outputs.map((n) => `<option ${n === d.default_output ? "selected" : ""}>${esc(n)}</option>`).join("");
  } catch (e) { toast("Audio devices: " + e, true); }
}
$("#lGate").addEventListener("input", () => { $("#lGateV").textContent = $("#lGate").value + " dB"; });
$("#lStart").addEventListener("click", async () => {
  if (!$("#lVoice").value) return toast("Choose a voice", true);
  try {
    await invoke("live_start", { params: { voice_id: $("#lVoice").value, input: $("#lIn").value, output: $("#lOut").value,
      block_time: +$("#lBlock").value, diffusion_steps: +$("#lSteps").value, gate_db: +$("#lGate").value, output_gain: 1.0 } });
    $("#lStart").disabled = true; $("#lStop").disabled = false; pollLive();
  } catch (e) { toast(String(e), true, 6000); }
});
$("#lStop").addEventListener("click", async () => { await invoke("live_stop"); $("#lStart").disabled = false; $("#lStop").disabled = true; });
async function pollLive() {
  const s = await invoke("live_state");
  const st = $("#lStatus");
  st.textContent = s.error ? "Error: " + s.error : s.running ? (s.status === "running" ? "● Live" : s.status) : "Idle";
  st.classList.toggle("on", s.running && s.status === "running");
  $("#lMeter").style.width = Math.max(0, Math.min(100, (s.input_db + 70) * 1.6)) + "%";
  $("#lInfer").textContent = s.infer_ms ? Math.round(s.infer_ms) : "–";
  $("#lLat").textContent = s.latency_ms ? (s.latency_ms / 1000).toFixed(2) + "s" : "–";
  $("#lDrop").textContent = s.dropped;
  if (s.running) setTimeout(pollLive, 150);
  else { $("#lStart").disabled = false; $("#lStop").disabled = true; }
}

// ------------------------------------------------------------------ Studio: home
function presetChips() {
  const def = ["grandma", "child_boy", "child_girl", "aunty"];
  $("#wChars").innerHTML = Object.entries(PRESETS).map(([k, [en, ta, e]]) =>
    `<span class="chip ${def.includes(k) ? "on" : ""}" data-k="${k}">${e} ${en} · ${ta}</span>`).join("");
  $$("#wChars .chip").forEach((c) => c.addEventListener("click", () => c.classList.toggle("on")));
}
async function loadStories() {
  const list = await invoke("story_list");
  $("#storyList").innerHTML = list.length ? list.map((s) => `
    <div class="story-card" data-open-story="${s.id}">
      <h3>${esc(s.title)}</h3><p>${esc(s.logline || "")}</p>
      <div class="meta"><span>${s.scenes} scenes</span><span>${s.lines} lines</span>${s.output ? "<span>🎬 rendered</span>" : ""}</div>
    </div>`).join("") : `<div class="empty">No stories yet. Write one with AI or paste a script above.</div>`;
}
$("#storyList").addEventListener("click", (ev) => { const c = ev.target.closest("[data-open-story]"); if (c) openStory(c.dataset.openStory); });
$("#wGo").addEventListener("click", async () => {
  const idea = $("#wIdea").value.trim();
  if (!idea) return toast("Describe your story idea first", true);
  const b = $("#wGo"); b.disabled = true;
  const provider = $("#wProvider").value;
  $("#wStatus").textContent = provider === "claude" ? "Claude is writing your story… (about a minute)" : "The local model is writing your story… (1–3 minutes)";
  try {
    const p = await invoke("story_write", { req: { idea, minutes: +$("#wMinutes").value, genre: $("#wGenre").value,
      characters: $$("#wChars .chip.on").map((c) => c.dataset.k), provider } });
    $("#wStatus").textContent = "";
    openStory(p.id);
  } catch (e) { $("#wStatus").textContent = ""; toast(String(e), true, 8000); }
  b.disabled = false;
});
$("#pasteGo").addEventListener("click", async () => {
  const text = $("#pasteScript").value.trim();
  if (!text) return toast("Paste a script first", true);
  try { const p = await invoke("story_from_script", { text, id: null }); openStory(p.id); } catch (e) { toast(String(e), true); }
});
$("#newBlank").addEventListener("click", async () => { const p = await invoke("story_new", { title: "புதிய கதை" }); openStory(p.id); });
$("#scriptHelp").addEventListener("click", () => modal("Script format", `<pre>${esc(`# Title of the story
> One-line summary (optional)

@Name = preset | look (optional, English, for the face)
   presets: man, girl, boy, aunty, uncle, grandpa, grandma, child_boy, child_girl

[Scene: Scene title | visual description for the background (English)]
Name: Tamil dialogue line
Name (emotion): Another line — emotion like happy, sad, excited, whisper
Narrator: Narration line (shown over the full scene)

A plain paragraph without "Name:" is read by the Narrator.
Unknown names become new characters automatically (பாட்டி → grandma, தாத்தா → grandpa …).`)}</pre>`, "Got it", false));

// ------------------------------------------------------------------ Studio: editor
async function openStory(id) {
  cur = await invoke("story_get", { id });
  $("#studioHome").classList.add("hidden");
  $("#studioEditor").classList.remove("hidden");
  go("studio");
  curRenderJob = lastJobs.find((j) => j.kind === "story" && j.status === "running" && (j.project_id === cur.id || j.title.endsWith(cur.title)))?.id || null;
  renderEditor();
}
$("#edBack").addEventListener("click", () => { flushSave(); cur = null; $("#studioEditor").classList.add("hidden"); $("#studioHome").classList.remove("hidden"); loadStories(); });

function scheduleSave() {
  if (!cur || curRenderJob) return;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(flushSave, 600);
  updateStats();
}
async function flushSave() {
  clearTimeout(saveTimer);
  if (!cur || curRenderJob) return;
  try { cur = await invoke("story_save", { project: cur }); } catch (e) { toast("Save failed: " + e, true); }
}
const charById = (id) => cur.characters.find((c) => c.id === id);
const charColor = (id) => ACCENTS[Math.max(0, cur.characters.findIndex((c) => c.id === id)) % ACCENTS.length];
function clearLineAssets(pred, audioToo = true) {
  cur.scenes.forEach((s) => s.lines.forEach((l) => { if (pred(l)) { if (audioToo) { l.audio = null; l.duration = 0; } l.talking_video = null; } }));
}

function renderEditor() {
  $("#edTitle").value = cur.title;
  $("#edLogline").value = cur.logline || "";
  setSeg("#edFormat", cur.format);
  setSeg("#edStyle", cur.style);
  $("#edTalking").checked = cur.talking_faces;
  setSeg("#edAnim", cur.cinematic === false ? "fast" : "cinematic");
  animHint();
  $("#edSubs").checked = cur.subtitles;
  $("#edMinimax").checked = cur.use_minimax;
  $("#edMusicName").textContent = cur.music ? cur.music.split(/[\\/]/).pop() : "None";
  $("#edMusicVol").value = cur.music_volume;
  $("#edBgName").textContent = cur.background_image ? cur.background_image.split(/[\\/]/).pop() : "Auto (AI scenes)";
  renderCharacters();
  renderScenes();
  updateStats();
  showResult();
  updateRenderPanel();
}
function setSeg(sel, v) { $$(sel + " button").forEach((b) => b.classList.toggle("on", b.dataset.v === v)); }
$$("#edFormat button").forEach((b) => b.addEventListener("click", () => {
  if (cur.format !== b.dataset.v) { cur.format = b.dataset.v; cur.scenes.forEach((s) => { s.image = null; s.clip = null; }); }
  setSeg("#edFormat", cur.format); renderScenes(); scheduleSave();
}));
$$("#edStyle button").forEach((b) => b.addEventListener("click", async () => {
  if (cur.style === b.dataset.v) return;
  const repaint = cur.characters.some((c) => c.face_image) || cur.scenes.some((s) => s.image);
  cur.style = b.dataset.v; setSeg("#edStyle", cur.style);
  if (repaint && (await modal("Repaint images?", "<p>Repaint all faces and scenes in the new look on the next render?</p>", "Repaint"))) {
    cur.characters.forEach((c) => { if (!c.face_image?.includes("uploads")) c.face_image = null; });
    cur.scenes.forEach((s) => { s.image = null; s.clip = null; });
    clearLineAssets(() => true, false);
    renderCharacters(); renderScenes();
  }
  scheduleSave();
}));
$("#edTitle").addEventListener("input", (e) => { cur.title = e.target.value; scheduleSave(); });
$("#edLogline").addEventListener("input", (e) => { cur.logline = e.target.value; scheduleSave(); });
$("#edTalking").addEventListener("change", (e) => { cur.talking_faces = e.target.checked; scheduleSave(); });
function animHint() {
  $("#edAnimHint").textContent = cur.cinematic === false
    ? "Fast: characters cut out into the scene with lip-synced faces. A 2-minute story renders in about 15–25 minutes."
    : "Cinematic: every shot is animated (gestures, body and scene motion) and lip-synced. Realistic but slow: about 1–2 minutes per shot.";
}
$$("#edAnim button").forEach((b) => b.addEventListener("click", () => {
  cur.cinematic = b.dataset.v === "cinematic"; setSeg("#edAnim", b.dataset.v); animHint(); scheduleSave();
}));
$("#edSubs").addEventListener("change", (e) => { cur.subtitles = e.target.checked; scheduleSave(); });
$("#edMinimax").addEventListener("change", async (e) => {
  cur.use_minimax = e.target.checked;
  if (cur.use_minimax) {
    const s = await invoke("settings_get");
    if (!s.has_minimax_key) toast("Add your MiniMax API key in Settings to use H3 scenes", true, 6000);
  }
  scheduleSave();
});
$("#edMusicVol").addEventListener("input", (e) => { cur.music_volume = +e.target.value; scheduleSave(); });
$("#edMusicPick").addEventListener("click", async () => {
  const f = await pickFile(AUDIO_FILTER); if (!f) return;
  cur.music = await invoke("import_asset", { projectId: cur.id, src: f, kind: "music" });
  $("#edMusicName").textContent = f.split(/[\\/]/).pop(); scheduleSave();
});
$("#edMusicClear").addEventListener("click", () => { cur.music = null; $("#edMusicName").textContent = "None"; scheduleSave(); });
$("#edBgPick").addEventListener("click", async () => {
  const f = await pickFile(IMAGE_FILTER); if (!f) return;
  cur.background_image = await invoke("import_asset", { projectId: cur.id, src: f, kind: "background" });
  $("#edBgName").textContent = f.split(/[\\/]/).pop(); scheduleSave();
});
$("#edBgClear").addEventListener("click", () => { cur.background_image = null; $("#edBgName").textContent = "Auto (AI scenes)"; scheduleSave(); });
$("#edDelete").addEventListener("click", async () => {
  if (!(await modal("Delete story?", `<p>Delete <b>${esc(cur.title)}</b> and all its generated files?</p>`, "Delete"))) return;
  await invoke("story_delete", { id: cur.id }); cur = null;
  $("#studioEditor").classList.add("hidden"); $("#studioHome").classList.remove("hidden"); loadStories();
});
$("#edScriptBtn").addEventListener("click", async () => {
  await flushSave();
  const text = await invoke("story_script", { id: cur.id });
  const ok = await modal("Story script", `<p class="muted small">Copy this to share the story, or edit it and apply. Characters keep their faces and voices.</p>
    <textarea id="mScript" rows="18" class="mono">${esc(text)}</textarea>
    <div class="actions"><button type="button" class="ghost" id="mCopy">Copy to clipboard</button></div>`, "Apply changes");
  if (ok) {
    const newText = $("#mScript").value;
    if (newText !== text) { cur = await invoke("story_from_script", { text: newText, id: cur.id }); renderEditor(); toast("Script applied"); }
  }
});
document.addEventListener("click", (e) => {
  if (e.target.id === "mCopy") { navigator.clipboard.writeText($("#mScript").value); toast("Copied"); }
});

// ---- characters
function renderCharacters() {
  if (!cur) return;
  const vOpts = (sel) => `<option value="">Auto (character voice)</option>` +
    voices.map((v) => `<option value="${v.id}" ${v.id === sel ? "selected" : ""}>${esc(v.name)}</option>`).join("");
  $("#charList").innerHTML = cur.characters.map((c) => `
    <div class="char" data-cid="${c.id}">
      <div class="char-top">
        ${c.face_image ? `<img class="face" src="${fileUrl(c.face_image)}" style="border-color:${charColor(c.id)}">` :
          `<div class="face" style="border-color:${charColor(c.id)}">${c.is_narrator ? "📖" : PRESETS[c.preset]?.[2] || "🙂"}</div>`}
        <div style="flex:1;min-width:0">
          <input class="char-name" value="${esc(c.name)}" data-f="name">
          <select data-f="preset">${Object.entries(PRESETS).map(([k, [en, ta]]) => `<option value="${k}" ${k === c.preset ? "selected" : ""}>${en} · ${ta}</option>`).join("")}</select>
        </div>
      </div>
      <select data-f="voice_id">${vOpts(c.voice_id)}</select>
      <label class="switch" style="margin:2px 0"><input type="checkbox" data-f="is_narrator" ${c.is_narrator ? "checked" : ""}><span></span>Narrator (no face on screen)</label>
      <textarea data-f="look" placeholder="Look (English): age, face, hair, clothes…">${esc(c.look)}</textarea>
      <div class="char-btns">
        <button class="ghost small" data-act="voice">▶ Voice</button>
        <button class="ghost small" data-act="genface" ${c.is_narrator ? "disabled" : ""}>🎨 ${c.face_image ? "New face" : "Make face"}</button>
        <button class="ghost small" data-act="upface" ${c.is_narrator ? "disabled" : ""}>⬆ Photo</button>
        <button class="ghost small danger" data-act="del">✕</button>
      </div>
    </div>`).join("") || `<div class="empty">No characters yet.</div>`;
}
$("#charList").addEventListener("change", (ev) => {
  const el = ev.target, card = el.closest("[data-cid]"); if (!card || !el.dataset.f) return;
  const c = charById(card.dataset.cid), f = el.dataset.f;
  c[f] = el.type === "checkbox" ? el.checked : el.value || (f === "voice_id" ? null : "");
  if (f === "voice_id" || f === "preset") clearLineAssets((l) => l.character_id === c.id);
  if (f === "is_narrator" || f === "preset") renderCharacters();
  renderScenes(); scheduleSave();
});
$("#charList").addEventListener("input", (ev) => {
  const el = ev.target, card = el.closest("[data-cid]"); if (!card || !["name", "look"].includes(el.dataset.f)) return;
  charById(card.dataset.cid)[el.dataset.f] = el.value; scheduleSave();
});
$("#charList").addEventListener("click", async (ev) => {
  const b = ev.target.closest("[data-act]"); if (!b) return;
  const c = charById(b.closest("[data-cid]").dataset.cid);
  try {
    if (b.dataset.act === "voice") {
      const vid = c.voice_id || voices.find((v) => v.preset === c.preset && v.kind === "designed")?.id;
      if (!vid) return toast("Create the character voices first (Voices page)", true);
      play((await api("GET", `/voices/${vid}/sample`)).path);
    }
    if (b.dataset.act === "genface") {
      b.disabled = true; b.textContent = "Painting…";
      await flushSave();
      const presets = await api("GET", "/presets");
      const r = await engineJob("/images", { prompt: c.look || presets[c.preset].face, kind: "portrait", style: cur.style,
        width: 1024, height: 1024, seed: Math.floor(Math.random() * 1e9), out: null });
      c.face_image = r.path; clearLineAssets((l) => l.character_id === c.id, false);
      renderCharacters(); scheduleSave();
    }
    if (b.dataset.act === "upface") {
      const f = await pickFile(IMAGE_FILTER); if (!f) return;
      c.face_image = await invoke("import_asset", { projectId: cur.id, src: f, kind: "face" });
      clearLineAssets((l) => l.character_id === c.id, false); renderCharacters(); scheduleSave();
    }
    if (b.dataset.act === "del") {
      const used = cur.scenes.some((s) => s.lines.some((l) => l.character_id === c.id));
      if (used && !(await modal("Remove character?", `<p>${esc(c.name)} has lines. Remove the character and their lines?</p>`, "Remove"))) return;
      cur.characters = cur.characters.filter((x) => x.id !== c.id);
      cur.scenes.forEach((s) => (s.lines = s.lines.filter((l) => l.character_id !== c.id)));
      renderCharacters(); renderScenes(); scheduleSave();
    }
  } catch (e) { toast(String(e.message || e), true); renderCharacters(); }
});
$("#addChar").addEventListener("click", () => {
  cur.characters.push({ id: rid(), name: "New character", preset: "man", voice_id: null, look: "", face_image: null, is_narrator: false });
  renderCharacters(); scheduleSave();
});
const rid = () => Math.random().toString(16).slice(2, 12);

// ---- scenes & lines
function renderScenes() {
  if (!cur) return;
  const tall = cur.format === "shorts";
  const spk = (sel) => cur.characters.map((c) => `<option value="${c.id}" ${c.id === sel ? "selected" : ""}>${esc(c.name)}</option>`).join("");
  $("#sceneList").innerHTML = cur.scenes.map((s, si) => `
    <div class="scene" data-si="${si}">
      <div class="scene-head">
        ${s.image ? `<img class="scene-img ${tall ? "tall" : ""}" src="${fileUrl(s.image)}">` : `<div class="scene-img ${tall ? "tall" : ""}">Painted on render</div>`}
        <div>
          <input class="scene-title char-name" data-sf="title" value="${esc(s.title)}" placeholder="Scene title (shown as a card)">
          <textarea class="visual" data-sf="visual" placeholder="Background description for the AI painter (English)">${esc(s.visual)}</textarea>
        </div>
        <div class="char-btns" style="flex-direction:column">
          <button class="ghost small" data-sact="paint">🎨 Paint</button>
          <button class="ghost small" data-sact="up">⬆ Image</button>
          <button class="ghost small" data-sact="upv">⬆ Video</button>
          <button class="ghost small danger" data-sact="del">✕</button>
        </div>
      </div>
      <div class="lines">
        ${s.lines.map((l, li) => `
          <div class="line" data-li="${li}">
            <select data-lf="character_id" style="border-left:3px solid ${charColor(l.character_id)}">${spk(l.character_id)}</select>
            <input data-lf="emotion" value="${esc(l.emotion)}" placeholder="emotion">
            <textarea data-lf="text" rows="1">${esc(l.text)}</textarea>
            <div class="line-btns">
              <button class="ghost small" data-lact="play" title="${l.audio ? "Play" : "Generate and play"}">${l.audio ? "▶" : "🔊"}</button>
              <button class="ghost small" data-lact="up" title="Move up">↑</button>
              <button class="ghost small" data-lact="del" title="Delete">✕</button>
            </div>
          </div>`).join("")}
        <button class="ghost small add-line" data-sact="addline">+ Line</button>
      </div>
    </div>`).join("") || `<div class="empty">No scenes yet.</div>`;
  $$("#sceneList textarea[data-lf=text]").forEach(autoGrow);
}
function autoGrow(t) { t.style.height = "auto"; t.style.height = Math.min(200, t.scrollHeight + 2) + "px"; }
$("#sceneList").addEventListener("input", (ev) => {
  const el = ev.target, sc = el.closest("[data-si]"); if (!sc) return;
  const s = cur.scenes[+sc.dataset.si];
  if (el.dataset.sf) { s[el.dataset.sf] = el.value; if (el.dataset.sf === "visual") { s.image = null; s.clip = null; } }
  const ln = el.closest("[data-li]");
  if (ln && el.dataset.lf) {
    const l = s.lines[+ln.dataset.li];
    l[el.dataset.lf] = el.value;
    if (el.dataset.lf === "text") { l.audio = null; l.talking_video = null; l.duration = 0; autoGrow(el); }
  }
  scheduleSave();
});
$("#sceneList").addEventListener("change", (ev) => {
  const el = ev.target; if (el.dataset.lf !== "character_id") return;
  const s = cur.scenes[+el.closest("[data-si]").dataset.si], l = s.lines[+el.closest("[data-li]").dataset.li];
  l.character_id = el.value; l.audio = null; l.talking_video = null; l.duration = 0;
  el.style.borderLeftColor = charColor(el.value); scheduleSave();
});
$("#sceneList").addEventListener("click", async (ev) => {
  const b = ev.target.closest("button"); if (!b) return;
  const si = +b.closest("[data-si]").dataset.si, s = cur.scenes[si];
  try {
    if (b.dataset.sact === "addline") {
      const last = s.lines[s.lines.length - 1];
      s.lines.push({ id: rid(), character_id: last?.character_id || cur.characters[0]?.id || "", text: "", emotion: "", audio: null, duration: 0, talking_video: null });
      renderScenes(); scheduleSave();
    }
    if (b.dataset.sact === "del" && (await modal("Delete scene?", `<p>Delete "${esc(s.title)}" and its ${s.lines.length} lines?</p>`, "Delete"))) {
      cur.scenes.splice(si, 1); renderScenes(); scheduleSave();
    }
    if (b.dataset.sact === "paint") {
      b.disabled = true; b.textContent = "Painting…";
      const [w, h] = cur.format === "shorts" ? [768, 1344] : [1344, 768];
      const r = await engineJob("/images", { prompt: s.visual || s.title, kind: "scene", style: cur.style, width: w, height: h, seed: Math.floor(Math.random() * 1e9), out: null });
      s.image = r.path; s.clip = null; renderScenes(); scheduleSave();
    }
    if (b.dataset.sact === "up") {
      const f = await pickFile(IMAGE_FILTER); if (!f) return;
      s.image = await invoke("import_asset", { projectId: cur.id, src: f, kind: "scene" }); s.clip = null; renderScenes(); scheduleSave();
    }
    if (b.dataset.sact === "upv") {
      const f = await pickFile([{ name: "Video", extensions: ["mp4", "mov", "webm", "mkv"] }]); if (!f) return;
      s.clip = await invoke("import_asset", { projectId: cur.id, src: f, kind: "clip" }); toast("Scene video set — it plays behind this scene"); scheduleSave();
    }
    const ln = b.closest("[data-li]");
    if (ln) {
      const li = +ln.dataset.li, l = s.lines[li];
      if (b.dataset.lact === "del") { s.lines.splice(li, 1); renderScenes(); scheduleSave(); }
      if (b.dataset.lact === "up" && li > 0) { [s.lines[li - 1], s.lines[li]] = [s.lines[li], s.lines[li - 1]]; renderScenes(); scheduleSave(); }
      if (b.dataset.lact === "play") {
        if (!l.audio) {
          if (!l.text.trim()) return;
          b.textContent = "…"; b.disabled = true;
          const c = charById(l.character_id);
          const vid = c?.voice_id || voices.find((v) => v.preset === c?.preset && v.kind === "designed")?.id || null;
          const r = await engineJob("/tts", { text: l.text, voice_id: vid, language: cur.language, mode: "auto", out: null });
          l.audio = r.path; l.duration = r.duration; l.talking_video = null; scheduleSave(); renderScenes();
        }
        play(l.audio);
      }
    }
  } catch (e) { toast(String(e.message || e), true); renderScenes(); }
});
$("#addScene").addEventListener("click", () => {
  cur.scenes.push({ id: rid(), title: `காட்சி ${cur.scenes.length + 1}`, visual: "", image: null, clip: null, lines: [] });
  renderScenes(); scheduleSave();
});
function updateStats() {
  if (!cur) return;
  const lines = cur.scenes.reduce((a, s) => a + s.lines.length, 0);
  const chars = cur.scenes.reduce((a, s) => a + s.lines.reduce((b, l) => b + l.text.length, 0), 0);
  const voiced = cur.scenes.reduce((a, s) => a + s.lines.filter((l) => l.audio).length, 0);
  const est = chars / 13 + cur.scenes.length * 2.4 + lines * 0.3 + 4;
  $("#edStats").innerHTML = `${cur.scenes.length} scenes · ${lines} lines · ${voiced}/${lines} voiced<br>Estimated video length: ~${fmtTime(est)}`;
}

// ---- render
$("#edRender").addEventListener("click", async () => {
  await flushSave();
  if (!cur.scenes.some((s) => s.lines.length)) return toast("Add some dialogue first", true);
  try {
    curRenderJob = await invoke("story_render", { id: cur.id });
    $("#resultPanel").classList.add("hidden");
    refreshJobs();
    toast("Rendering started — it keeps going in the background");
  } catch (e) { toast(String(e), true, 6000); }
});
$("#rCancel").addEventListener("click", () => curRenderJob && invoke("job_cancel", { id: curRenderJob }));
$("#rPause").addEventListener("click", async () => {
  if (!curRenderJob) return;
  const j = lastJobs.find((x) => x.id === curRenderJob);
  try {
    await invoke(j && j.paused ? "job_resume" : "job_pause", { id: curRenderJob });
    toast(j && j.paused ? "▶ Resumed" : "⏸ Paused — the GPU is free until you resume");
    refreshJobs();
  } catch (e) { toast(String(e), true); }
});
let lastGpu = null, gpuTimer = 0;
async function pollGpu() {
  if (Date.now() - gpuTimer < 3000) return;
  gpuTimer = Date.now();
  try { lastGpu = await api("GET", "/gpu"); } catch { lastGpu = null; }
}
function setTaskbar(progress, status) {
  try { T.window.getCurrentWindow().setProgressBar({ status, progress: Math.round(progress) }); } catch {}
}
const stepTime = (st) => {
  const now = Date.now() / 1000;
  if (st.status === "done" && st.started && st.finished) return st.finished - st.started < 1 ? "cached ✓" : `took ${fmtTime(st.finished - st.started)}`;
  if (st.status === "running" && st.started) {
    const el = now - st.started;
    const left = st.progress > 0.08 && el > 3 ? el * (1 - st.progress) / st.progress : null;
    return `${fmtTime(el)}${left != null ? ` · ~${fmtTime(left)} left` : ""}`;
  }
  if (st.status === "skipped") return "skipped";
  return st.estimate >= 1 ? `~${fmtTime(st.estimate)}` : "cached";
};
async function updateRenderPanel() {
  if (!cur || !curRenderJob) { $("#renderPanel").classList.add("hidden"); setEditorLocked(false); return; }
  const j = lastJobs.find((x) => x.id === curRenderJob);
  if (!j) return;
  $("#renderPanel").classList.remove("hidden");
  setEditorLocked(true);
  pollGpu();
  const pct = j.percent || 0;
  $("#rPct").textContent = pct.toFixed(pct < 10 ? 1 : 0) + "%";
  $("#rArc").style.strokeDashoffset = String(326.7 * (1 - pct / 100));
  $("#rBar").style.width = pct + "%";
  $("#renderPanel").classList.toggle("paused", !!j.paused);
  $("#rPause").textContent = j.paused ? "▶ Resume" : "⏸ Pause";
  $("#rPause").classList.toggle("primary", !!j.paused);
  $("#rStage").textContent = j.paused ? "⏸ Paused — click Resume to continue (nothing is lost)" : (j.stage || "Working…");
  const finishAt = j.eta_sec != null ? new Date(Date.now() + j.eta_sec * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : null;
  const stats = [
    `<span>Elapsed <b>${fmtTime(j.elapsed_sec)}</b></span>`,
    `<span>Time left <b>${j.eta_sec != null ? "~" + fmtTime(j.eta_sec) : "estimating…"}</b></span>`,
    finishAt ? `<span>Done by <b>${finishAt}</b></span>` : "",
    `<span>Step <b>${(j.steps || []).filter((x) => x.status === "done").length + 1} / ${(j.steps || []).filter((x) => x.status !== "skipped").length}</b></span>`,
    lastGpu && lastGpu.vram_total_gb ? `<span>GPU <b>${(lastGpu.vram_total_gb - lastGpu.vram_free_gb).toFixed(1)} / ${lastGpu.vram_total_gb} GB</b></span>` : "",
  ];
  $("#rStats").innerHTML = stats.join("");
  $("#rSteps").innerHTML = (j.steps || []).map((st) => `
    <li class="rp-step ${st.status}">
      <span class="rp-icon">${st.status === "done" ? "✓" : st.status === "running" ? "" : st.status === "skipped" ? "–" : ""}</span>
      <div style="min-width:0"><div class="rp-step-label">${esc(st.label)}</div>
        ${st.status === "running" ? `<div class="rp-step-detail">${esc(st.detail || "Working…")}</div><div class="progress"><div style="width:${(st.progress * 100).toFixed(1)}%"></div></div>` : ""}</div>
      <span class="rp-step-time">${stepTime(st)}${st.status === "running" ? `<br><b>${(st.progress * 100).toFixed(0)}%</b>` : ""}</span>
    </li>`).join("");
  if (j.preview) {
    const img = $("#rPreview");
    const src = fileUrl(j.preview);
    if (img.dataset.src !== src) { img.dataset.src = src; img.src = src; }
    img.classList.remove("hidden"); $("#rPreviewEmpty").classList.add("hidden");
  }
  $("#rFacts").innerHTML = (j.facts || []).map(([k, v]) => `<div><b>${esc(v)}</b><span>${esc(k)}</span></div>`).join("");
  const log = $("#rLog");
  const atBottom = log.scrollTop + log.clientHeight >= log.scrollHeight - 10;
  log.innerHTML = (j.events || []).map((e) => `<div>${esc(e)}</div>`).join("");
  if (atBottom) log.scrollTop = log.scrollHeight;
  setTaskbar(pct, j.paused ? "paused" : "normal");
  if (j.status !== "running") {
    const done = j.status === "done";
    curRenderJob = null;
    setTaskbar(0, "none");
    $("#renderPanel").classList.add("hidden");
    setEditorLocked(false);
    cur = await invoke("story_get", { id: cur.id });
    renderEditor();
    if (done) toast("🎉 Video ready!");
    else if (j.status === "error") toast("Render failed: " + j.error, true, 9000);
  }
}
function setEditorLocked(lock) {
  $("#studioEditor .editor-grid").style.opacity = lock ? 0.55 : 1;
  $("#studioEditor .editor-grid").style.pointerEvents = lock ? "none" : "";
  $("#edRender").disabled = lock;
}
const projDir = () => (cur.output || "").replace(/[\\/][^\\/]*$/, "");
const tFmt = (t) => `${Math.floor(t / 60)}:${String(Math.floor(t % 60)).padStart(2, "0")}`;
const tParse = (v) => { const m = String(v).match(/(\d+):(\d{1,2})/); return m ? +m[1] * 60 + +m[2] : (+v || 0); };
const lineById = (id) => { for (const s of cur.scenes) for (const l of s.lines) if (l.id === id) return [s, l]; return [null, null]; };

function showResult() {
  if (!cur?.output) { $("#resultPanel").classList.add("hidden"); return; }
  $("#resultPanel").classList.remove("hidden");
  const vid = $("#resVideo");
  const src = fileUrl(cur.output) + "?t=" + cur.updated;
  if (vid.dataset.src !== src) { vid.dataset.src = src; vid.src = src; }
  const img = $("#resThumb");
  img.onerror = () => img.classList.add("hidden");
  img.onload = () => img.classList.remove("hidden");
  img.src = fileUrl(projDir() + "/thumbnail.jpg") + "?t=" + cur.updated;
  const v = (cur.versions || []).find((x) => x.path === cur.output);
  $("#resInfo").textContent = (cur.format === "shorts" ? "YouTube Shorts 1080×1920" : "YouTube 1920×1080") + " · H.264 · −14 LUFS" + (v ? ` · version ${v.version}` : "");
  $("#resDesc").value = cur.description || "";
  renderTimeline();
  renderVersions();
  fixChips();
}

function renderTimeline() {
  const dir = projDir();
  const shots = (cur.timeline || []).filter((s) => ["dialogue", "narration", "insert", "question"].includes(s.kind));
  $("#resTimeline").innerHTML = shots.map((s) => {
    const [sc, l] = lineById(s.line_id);
    const thumb = s.kind === "narration" ? sc?.image : s.kind === "question" ? null : (l?.image || `${dir}/keyframes/${s.line_id}.png`);
    const icon = s.kind === "question" ? "❓" : s.kind === "narration" ? "📖" : "🎬";
    return `<div class="shot" data-start="${s.start}" data-line="${s.line_id || ""}" data-kind="${s.kind}">
      ${thumb ? `<img src="${fileUrl(thumb)}" onerror="this.outerHTML='<div class=noimg>${icon}</div>'">` : `<div class="noimg">${icon}</div>`}
      <span class="t">${tFmt(s.start)}</span>
      <div class="who">${esc(s.hook ? "🔥 " + s.label : s.label)}</div><div class="what">${esc(s.text || "")}</div></div>`;
  }).join("") || `<div class="muted small">The shot timeline appears after the next render.</div>`;
}
$("#resTimeline").addEventListener("click", (ev) => {
  const c = ev.target.closest(".shot"); if (!c) return;
  const t = +c.dataset.start;
  $("#resVideo").currentTime = t + 0.05;
  $("#fixAt").value = tFmt(t);
  const kind = c.dataset.kind;
  $$(".shot-menu").forEach((m) => m.remove());
  const menu = document.createElement("div");
  menu.className = "shot-menu";
  const [, l] = lineById(c.dataset.line);
  const btn = (label, req) => `<button class="ghost small" data-req="${esc(req)}">${label}</button>`;
  menu.innerHTML = `<b>Fix the shot at ${tFmt(t)}${l?.text ? ": “" + esc(l.text.slice(0, 50)) + "…”" : ""}</b>` +
    (kind === "dialogue" ? btn("👄 Fix lip-sync", "fix the lip-sync") + btn("🔁 New animation", "animate again") +
      btn("🗣 New voice take", "record the voice again") + btn("🐢 Slower", "say it slower") + btn("✏️ Edit words", "change the words") +
      btn("🎨 Change look", "change the look") : "") +
    (kind === "narration" ? btn("🗣 New voice take", "record the voice again") + btn("✏️ Edit words", "change the words") : "") +
    (kind === "insert" ? btn("🔁 New animation", "animate again") : "") +
    btn("🖼 Repaint background", "change the background") + btn("🗑 Remove", "delete this line");
  $("#resTimeline").after(menu);
  menu.addEventListener("click", (e) => {
    const b = e.target.closest("[data-req]"); if (!b) return;
    $("#fixReq").value = b.dataset.req;
    previewFix();
    menu.remove();
  });
});
$("#resVideo").addEventListener("timeupdate", () => {
  const t = $("#resVideo").currentTime;
  let active = null;
  $$("#resTimeline .shot").forEach((c) => { if (+c.dataset.start <= t) active = c; c.classList.remove("active"); });
  if (active) { active.classList.add("active"); }
});

function fixChips() {
  const people = cur.characters.filter((c) => !c.is_narrator).map((c) => c.name);
  const ex = [
    "fix the lip-sync", "say it slower", `change ${people[people.length - 1] || "Appa"}'s shirt to blue`,
    `give ${people[0] || "Paati"} grey hair`, "make the background daytime", "animate again with a big smile",
  ];
  $("#fixChips").innerHTML = ex.map((x) => `<span class="chip" data-x="${esc(x)}">${esc(x)}</span>`).join("");
}
$("#fixChips").addEventListener("click", (e) => { const c = e.target.closest("[data-x]"); if (c) { $("#fixReq").value = c.dataset.x; $("#fixReq").focus(); } });
$("#fixNow").addEventListener("click", () => { $("#fixAt").value = tFmt($("#resVideo").currentTime); });

let currentPlan = null;
async function previewFix() {
  const req = $("#fixReq").value.trim();
  if (!req) return toast("Type what you want to change", true);
  try {
    currentPlan = await invoke("story_fix_preview", { id: cur.id, at: tParse($("#fixAt").value), request: req });
    renderPlan();
  } catch (e) { toast(String(e), true, 7000); }
}
$("#fixPreview").addEventListener("click", previewFix);
$("#fixReq").addEventListener("keydown", (e) => { if (e.key === "Enter") previewFix(); });

function renderPlan() {
  const pl = currentPlan, box = $("#fixPlan");
  const field = { text: ["New words (Tamil)", "text"], look: ["New look (English: hair, clothes, colours…)", "look"],
    scene: ["New background description (English)", "visual"], animate: ["Motion / action (English)", "motion"],
    emotion: ["Motion / action (English)", "motion"] }[pl.action];
  const icons = { lipsync: "👄", text: "✏️", voice: "🗣", animate: "🔁", look: "🎨", scene: "🖼", emotion: "🎭", delete: "🗑" };
  box.innerHTML = `
    <div class="sum">${icons[pl.action] || "✨"} ${esc(pl.summary)}</div>
    <div class="muted small">Shot: ${esc(pl.target)}</div>
    ${field ? `<label>${field[0]}${pl.needs ? ' <span class="hint" style="color:var(--warn)">— please fill this in</span>' : ""}
      <textarea id="planField" rows="2">${esc(pl[field[1]] || "")}</textarea></label>` : ""}
    ${pl.action === "emotion" ? `<label>Emotion <input id="planEmotion" value="${esc(pl.emotion || "")}"></label>` : ""}
    <div class="meta"><span>${pl.shots} shot${pl.shots > 1 ? "s" : ""} regenerated</span><span>everything else reused</span><span>about ${fmtTime(pl.estimate_sec)}</span></div>
    <div class="actions"><button class="primary" id="planApply">✅ Apply &amp; regenerate</button><button class="ghost" id="planQueue">Apply, render later</button><button class="ghost" id="planCancel">Cancel</button></div>`;
  box.classList.remove("hidden");
  if (field) $("#planField").focus();
  $("#planCancel").onclick = () => box.classList.add("hidden");
  const apply = async (render) => {
    if (field) pl[field[1]] = $("#planField").value;
    if ($("#planEmotion")) pl.emotion = $("#planEmotion").value;
    try {
      const r = await invoke("story_fix_apply", { id: cur.id, plan: pl, render });
      box.classList.add("hidden");
      $("#fixReq").value = "";
      cur = await invoke("story_get", { id: cur.id });
      if (r.job) { curRenderJob = r.job; refreshJobs(); toast(`${r.note} — regenerating only that part`); }
      else { toast(`${r.note} — saved. Add more fixes, then click Render video.`); }
      renderEditor();
    } catch (e) { toast(String(e), true, 7000); }
  };
  $("#planApply").onclick = () => apply(true);
  $("#planQueue").onclick = () => apply(false);
}

function renderVersions() {
  const vs = [...(cur.versions || [])].reverse();
  $("#resVersions").innerHTML = vs.map((v) => `
    <div class="version ${v.path === cur.output ? "current" : ""}">
      <span class="v">v${v.version}</span><span class="n" title="${esc(v.note)}">${esc(v.note)} · ${new Date(v.time * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })}</span>
      <button class="ghost small" data-play="${esc(v.path)}">▶</button>
      <button class="ghost small" data-dl="${esc(v.path)}">⬇</button>
      ${v.path === cur.output ? "" : `<button class="ghost small" data-use="${esc(v.path)}">Use</button>`}
    </div>`).join("") || `<div class="muted small">No versions yet.</div>`;
}
$("#resVersions").addEventListener("click", async (e) => {
  const b = e.target.closest("button"); if (!b) return;
  if (b.dataset.play) { $("#resVideo").src = fileUrl(b.dataset.play) + "?v=" + Date.now(); $("#resVideo").play(); }
  if (b.dataset.dl) downloadFile(b.dataset.dl, b.dataset.dl.split(/[\\/]/).pop(), "mp4");
  if (b.dataset.use) { cur = await invoke("story_use_version", { id: cur.id, path: b.dataset.use }); renderEditor(); toast("Switched to that version"); }
});

async function downloadFile(src, name, ext) {
  const dest = await T.dialog.save({ defaultPath: name, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
  if (dest) { await invoke("copy_file", { src, dest }); toast("Saved ✓"); }
}
async function exportAndSave(kind, ext, label) {
  try {
    toast(kind === "shorts" ? "Making the Shorts teaser… (about a minute)" : `Preparing ${label}…`, false, 2500);
    const path = await invoke("story_export", { id: cur.id, kind });
    await downloadFile(path, path.split(/[\\/]/).pop(), ext);
  } catch (e) { toast(String(e), true, 7000); }
}
$("#resSave").addEventListener("click", () => downloadFile(cur.output, `${cur.title}.mp4`, "mp4"));
$("#resThumbSave").addEventListener("click", () => downloadFile(projDir() + "/thumbnail.jpg", `${cur.title} thumbnail.jpg`, "jpg"));
$("#resSrt").addEventListener("click", () => exportAndSave("srt", "srt", "subtitles"));
$("#resMp3").addEventListener("click", () => exportAndSave("mp3", "mp3", "audio"));
$("#resShorts").addEventListener("click", () => exportAndSave("shorts", "mp4", "Shorts teaser"));
$("#resOpen").addEventListener("click", () => T.opener.revealItemInDir(cur.output));
$("#resDescCopy").addEventListener("click", () => { navigator.clipboard.writeText($("#resDesc").value); toast("Description copied — paste it into YouTube"); });

// ------------------------------------------------------------------ settings
async function loadSettings() {
  const s = await invoke("settings_get");
  $("#sWriter").value = s.writer; $("#sAnthropic").value = s.anthropic_api_key; $("#sMinimax").value = s.minimax_api_key;
  $("#sMmModel").value = s.minimax_model; $("#sTray").checked = s.close_to_tray; $("#sAuto").checked = s.autostart_engine;
  $("#engDir").textContent = s.engine_dir || "Built-in";
  $("#sIntro").textContent = s.intro_video ? s.intro_video.split(/[\\/]/).pop() : "No intro";
  $("#sOutro").textContent = s.outro_video ? s.outro_video.split(/[\\/]/).pop() : "No outro";
  try {
    const c = await invoke("cache_info");
    const gb = (c.total_bytes / 1e9).toFixed(2);
    const k = c.kinds;
    $("#sCache").textContent = `${gb} GB reused across stories — ${k.tts.files} voice lines, ${k.images.files} images, ${k.clips.files} animated shots, ${k.sync.files} lip-syncs, ${k.segments.files} composed shots.`;
  } catch {}
  try {
    const models = await invoke("ollama_models");
    $("#sOllama").innerHTML = `<option value="">Auto</option>` + models.map((m) => `<option ${m === s.ollama_model ? "selected" : ""}>${esc(m)}</option>`).join("");
  } catch { $("#sOllama").innerHTML = `<option value="">Ollama not running</option>`; }
  refreshLog();
}
async function refreshLog() {
  if (!$("#page-settings").classList.contains("active")) return;
  const log = await invoke("engine_log");
  const el = $("#engLog"), atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 20;
  el.textContent = log.slice(-200).join("\n");
  if (atBottom) el.scrollTop = el.scrollHeight;
  setTimeout(refreshLog, 1500);
}
$("#sSave").addEventListener("click", async () => {
  await invoke("settings_set", { patch: { writer: $("#sWriter").value, ollama_model: $("#sOllama").value, anthropic_api_key: $("#sAnthropic").value,
    minimax_api_key: $("#sMinimax").value, minimax_model: $("#sMmModel").value, close_to_tray: $("#sTray").checked, autostart_engine: $("#sAuto").checked } });
  $("#wProvider").value = $("#sWriter").value;
  toast("Settings saved"); loadSettings();
});
$("#engInstall").addEventListener("click", async () => {
  try { await invoke("engine_setup", { talkingFaces: $("#engTalking").checked, cinematic: true }); toast("Installing — this downloads several GB and can take 20–60 minutes"); }
  catch (e) { toast(String(e), true); }
});
$("#engRestart").addEventListener("click", async () => { await invoke("engine_restart"); toast("Restarting engine…"); });
$("#engUnload").addEventListener("click", async () => { await api("POST", "/gpu/unload").catch(() => {}); toast("GPU memory will be freed"); });
$("#engData").addEventListener("click", async () => { const s = await invoke("engine_status"); T.opener.openPath(s.data_dir); });

// ------------------------------------------------------------------ ChatGPT prompt builder
function buildStoryPrompt(o) {
  const v = (x, d) => (x && x.trim() ? x.trim() : d);
  return STORY_PROMPT_TEMPLATE
    .replace(/^TITLE \/ IDEA  : .*$/m, `TITLE / IDEA  : ${v(o.title, "AI chooses a fresh, catchy title")}`)
    .replace(/^MY STORY      : .*$/m, `MY STORY      : ${v((o.story || "").replace(/\s+/g, " "), "(empty - invent an original story from the title)")}`)
    .replace(/^LENGTH        : .*$/m, `LENGTH        : ${o.length}`)
    .replace(/^AUDIENCE      : .*$/m, `AUDIENCE      : ${o.audience}`)
    .replace(/^MOOD \/ GENRE  : .*$/m, `MOOD / GENRE  : ${v(o.mood, "AI chooses")}`)
    .replace(/^SETTING       : .*$/m, `SETTING       : ${v(o.setting, "AI chooses")}`)
    .replace(/^CHARACTERS    : .*$/m, `CHARACTERS    : ${v(o.characters, "AI chooses")}`)
    .replace(/^LESSON        : .*$/m, `LESSON        : ${v(o.lesson, "AI chooses")}`);
}
// Quick copy: the fixed master template with the user's title and story inserted - never rewritten by an AI.
$("#qCopy").addEventListener("click", async () => {
  const title = $("#qTitle").value, story = $("#qStory").value;
  if (!title.trim() && !story.trim()) return toast("Type a title or your story first", true);
  await navigator.clipboard.writeText(buildStoryPrompt({ title, story, length: "2 minutes", audience: "children 4–10 and families" }));
  toast("📋 Prompt copied — paste it into ChatGPT, Claude or Gemini");
});
$("#builderOpen").addEventListener("click", async () => {
  const d = $("#modal");
  d.classList.add("wide");
  const html = `
    <div class="builder-steps">
      <div><b>1 · Describe</b>Your title is enough; everything else is optional.</div>
      <div><b>2 · Copy &amp; paste</b>Paste the prompt into ChatGPT, Claude or Gemini.</div>
      <div><b>3 · Bring it back</b>Click <i>Copy code</i> on the AI's answer, paste it here and create.</div>
    </div>
    <div class="builder">
      <label class="full">Story title or idea <span class="hint">(Tamil or English)</span><input id="bTitle" placeholder="e.g. நிலாவுடன் பேசிய குட்டி முயல்" value="${esc($("#qTitle").value || "")}"></label>
      <label class="full">Your story in 2–3 lines <span class="hint">(optional — the AI keeps your plot exactly)</span><textarea id="bStory" rows="3">${esc($("#qStory").value || "")}</textarea></label>
      <label>Length <select id="bLen"><option value="1 minute (YouTube Shorts)">1 min · Shorts</option><option value="2 minutes" selected>2 min</option><option value="3 minutes">3 min</option><option value="5 minutes">5 min</option></select></label>
      <label>Audience <select id="bAud"><option>children 4–10 and families</option><option>toddlers 2–5 (very simple words)</option><option>older kids 8–14</option><option>whole family / adults</option></select></label>
      <label>Mood <select id="bMood"><option value="">AI chooses</option><option>funny</option><option>heart-touching</option><option>adventure</option><option>mystery</option><option>moral story</option><option>festival special</option><option>bedtime story (calm)</option></select></label>
      <label>Setting <input id="bSet" placeholder="AI chooses — e.g. village, forest, school"></label>
      <label>Characters <input id="bChars" placeholder="AI chooses — e.g. a rabbit, a parrot and grandma"></label>
      <label>Lesson <input id="bLesson" placeholder="AI chooses — e.g. sharing, honesty"></label>
      <label class="full">Prompt preview <textarea id="bPreview" class="mono" readonly></textarea></label>
    </div>
    <div class="actions">
      <button type="button" class="primary" id="bCopy">📋 Copy prompt</button>
      <button type="button" class="ghost" id="bGpt">Open ChatGPT</button>
      <button type="button" class="ghost" id="bClaude">Open Claude</button>
      <button type="button" class="ghost" id="bGemini">Open Gemini</button>
    </div>
    <p class="muted small">Tip: always copy the AI's answer with its <b>Copy code</b> button. Then click <b>Create story from script</b> and check the characters' voices before rendering.</p>`;
  const done = modal("✨ ChatGPT prompt builder", html, "Close", false);
  const opts = () => ({ title: $("#bTitle").value, story: $("#bStory").value, length: $("#bLen").value, audience: $("#bAud").value, mood: $("#bMood").value,
    setting: $("#bSet").value, characters: $("#bChars").value, lesson: $("#bLesson").value });
  const refresh = () => { $("#bPreview").value = buildStoryPrompt(opts()); };
  ["#bTitle", "#bStory", "#bLen", "#bAud", "#bMood", "#bSet", "#bChars", "#bLesson"].forEach((k) => $(k).addEventListener("input", refresh));
  refresh();
  $("#bCopy").onclick = async () => {
    if (!$("#bTitle").value.trim() && !$("#bStory").value.trim()) { toast("Type a title or your story first", true); return; }
    await navigator.clipboard.writeText(buildStoryPrompt(opts()));
    toast("📋 Prompt copied — paste it into ChatGPT");
  };
  const open = (url) => T.opener.openUrl(url).catch(() => toast("Open " + url + " in your browser"));
  $("#bGpt").onclick = () => open("https://chatgpt.com/");
  $("#bClaude").onclick = () => open("https://claude.ai/new");
  $("#bGemini").onclick = () => open("https://gemini.google.com/app");
  await done;
  d.classList.remove("wide");
});

// ------------------------------------------------------------------ first-run setup
const SETUP_STEPS_BASE = 8;
async function showSetupProgress(status) {
  $("#setupProgress").classList.remove("hidden");
  $("#setupGo").disabled = status === "installing";
  const log = await invoke("engine_log");
  const steps = log.filter((l) => l.startsWith("==> "));
  const total = SETUP_STEPS_BASE + ($("#setupTalking").checked ? 4 : 0) + ($("#setupCine").checked ? 2 : 0);
  const done = Math.min(steps.length, total);
  const failed = log.some((l) => l.includes("SETUP FAILED"));
  const ok = log.some((l) => l.includes("SETUP COMPLETE"));
  const pct = ok ? 100 : Math.max(2, Math.round((done - 0.5) / total * 100));
  $("#setupBar").style.width = pct + "%";
  $("#setupStage").textContent = failed ? "Setup failed — see the messages below, then click Set up again"
    : ok ? "✓ Setup complete — starting the engine…" : `Step ${done} of ~${total}: ${(steps[steps.length - 1] || "==> Starting").slice(4)}`;
  $("#setupMeta").textContent = `${pct}% · large downloads can look paused for a few minutes — that's normal`;
  const el = $("#setupLog");
  el.textContent = log.slice(-60).join("\n");
  el.scrollTop = el.scrollHeight;
  if (failed) $("#setupGo").disabled = false;
}
$("#setupGo").addEventListener("click", async () => {
  try {
    await invoke("engine_setup", { talkingFaces: $("#setupTalking").checked, cinematic: $("#setupCine").checked });
    showSetupProgress("installing");
    toast("Setup started — you can leave this window open or minimise it");
  } catch (e) { toast(String(e), true, 8000); }
});
async function pickEngineDir() {
  const d = await T.dialog.open({ directory: true, multiple: false, title: "Choose the engine folder (contains vcstudio and .venv)" });
  if (!d) return;
  await invoke("settings_set", { patch: { engine_dir: d } });
  toast("Engine folder saved — restarting the app engine…");
  setTimeout(() => invoke("engine_restart"), 300);
  loadSettings();
}
$("#setupHave").addEventListener("click", async () => { await pickEngineDir(); toast("Restart VoiceClone Studio to use the new engine folder", false, 6000); });
$("#engDirPick").addEventListener("click", async () => { await pickEngineDir(); toast("Restart VoiceClone Studio to use the new engine folder", false, 6000); });
$("#engDirClear").addEventListener("click", async () => { await invoke("settings_set", { patch: { engine_dir: "" } }); loadSettings(); toast("Using the built-in engine after the next restart"); });

async function pickBumper(key) {
  const f = await pickFile([{ name: "Video", extensions: ["mp4", "mov", "mkv", "webm"] }]);
  if (!f) return;
  await invoke("settings_set", { patch: { [key]: f } }); loadSettings(); toast("Saved — added to every new render");
}
$("#sIntroPick").addEventListener("click", () => pickBumper("intro_video"));
$("#sOutroPick").addEventListener("click", () => pickBumper("outro_video"));
$("#sIntroClear").addEventListener("click", async () => { await invoke("settings_set", { patch: { intro_video: "" } }); loadSettings(); });
$("#sOutroClear").addEventListener("click", async () => { await invoke("settings_set", { patch: { outro_video: "" } }); loadSettings(); });
$("#sCacheClear").addEventListener("click", async () => {
  if (!(await modal("Clear cache?", "<p>Stories keep their own files. Clearing only removes the shared reuse cache, so the next renders may take longer.</p>", "Clear"))) return;
  await invoke("cache_clear"); loadSettings(); toast("Cache cleared");
});

// ------------------------------------------------------------------ boot
(async function boot() {
  presetChips();
  loadStories();
  pollEngine();
  try { const s = await invoke("settings_get"); $("#wProvider").value = s.writer; } catch {}
  refreshJobs();
})();
