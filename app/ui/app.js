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
      $("#engineText").textContent = "Engine not installed";
    } else {
      $("#engineText").textContent = s.status === "installing" ? "Installing engine…" : "Engine " + s.status;
      if (s.status === "stopped" || s.status === "setup failed") pill.classList.add("bad");
      engineReady = false;
    }
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
    const eta = j.eta_sec != null ? `~${fmtTime(j.eta_sec)} left` : j.status === "running" ? "estimating…" : "";
    const right = j.status === "running" || j.status === "queued"
      ? `<button class="ghost small" data-cancel="${j.id}">Cancel</button>` : `<span class="st">${j.status}</span>`;
    return `<div class="job ${j.status}">
      <div class="job-top"><span class="job-title">${esc(j.title)}</span><span class="job-pct">${pct.toFixed ? pct.toFixed(pct < 10 ? 1 : 0) : pct}%</span></div>
      <div class="progress"><div style="width:${pct}%"></div></div>
      <div class="job-sub"><span>${esc(j.error || j.stage || "")}</span><span>${compact ? eta : `${eta} ${j.elapsed_sec ? "· " + fmtTime(j.elapsed_sec) + " elapsed" : ""}`}</span></div>
      ${compact ? "" : `<div class="actions">${right}${j.result && j.result.path ? `<button class="ghost small" data-open="${esc(j.result.path)}">Show file</button>` : ""}</div>`}
      ${compact ? `<div class="actions right" style="margin-top:4px">${right}</div>` : ""}
    </div>`;
  };
  if ($("#page-jobs").classList.contains("active")) {
    $("#jobList").innerHTML = lastJobs.length ? lastJobs.map((j) => jobHtml(j, false)).join("") : `<div class="empty">No jobs yet.</div>`;
  }
  const dock = $("#dock");
  if (running.length) {
    dock.innerHTML = `<div class="dock-head"><span>Running in background</span><span>${running.length} job${running.length > 1 ? "s" : ""}</span></div>` +
      running.slice(0, 3).map((j) => jobHtml(j, true)).join("");
    dock.classList.remove("hidden");
  } else dock.classList.add("hidden");
  updateRenderPanel();
}
document.addEventListener("click", async (ev) => {
  const c = ev.target.closest("[data-cancel]");
  if (c) { await invoke("job_cancel", { id: c.dataset.cancel }).catch((e) => toast(e, true)); refreshJobs(); }
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
  curRenderJob = lastJobs.find((j) => j.kind === "story" && j.status === "running" && j.title.endsWith(cur.title))?.id || null;
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
async function updateRenderPanel() {
  if (!cur || !curRenderJob) { $("#renderPanel").classList.add("hidden"); setEditorLocked(false); return; }
  const j = lastJobs.find((x) => x.id === curRenderJob);
  if (!j) return;
  $("#renderPanel").classList.remove("hidden");
  setEditorLocked(true);
  $("#rStage").textContent = `${j.stage} — ${j.percent.toFixed(1)}%`;
  $("#rBar").style.width = j.percent + "%";
  $("#rMeta").textContent = [j.eta_sec != null ? `about ${fmtTime(j.eta_sec)} left` : "estimating time…", `${fmtTime(j.elapsed_sec)} elapsed`].join(" · ");
  if (j.status !== "running") {
    const done = j.status === "done";
    curRenderJob = null;
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
function showResult() {
  if (!cur?.output) { $("#resultPanel").classList.add("hidden"); return; }
  $("#resultPanel").classList.remove("hidden");
  $("#resVideo").src = fileUrl(cur.output) + "?t=" + cur.updated;
  const thumb = cur.output.replace(/[^\\/]+$/, "thumbnail.jpg");
  const img = $("#resThumb");
  img.onerror = () => img.classList.add("hidden");
  img.onload = () => img.classList.remove("hidden");
  img.src = fileUrl(thumb) + "?t=" + cur.updated;
  $("#resInfo").textContent = (cur.format === "shorts" ? "YouTube Shorts 1080×1920" : "YouTube 1920×1080") + " · H.264 · −14 LUFS";
  $("#resDesc").value = cur.description || "";
}
$("#resSave").addEventListener("click", async () => {
  const dest = await T.dialog.save({ defaultPath: `${cur.title}.mp4`, filters: [{ name: "MP4 video", extensions: ["mp4"] }] });
  if (dest) { await invoke("copy_file", { src: cur.output, dest }); toast("Video saved"); }
});
$("#resThumbSave").addEventListener("click", async () => {
  const src = cur.output.replace(/[^\\/]+$/, "thumbnail.jpg");
  const dest = await T.dialog.save({ defaultPath: `${cur.title} thumbnail.jpg`, filters: [{ name: "JPEG", extensions: ["jpg"] }] });
  if (dest) { await invoke("copy_file", { src, dest }); toast("Thumbnail saved"); }
});
$("#resOpen").addEventListener("click", () => T.opener.revealItemInDir(cur.output));
$("#resDescCopy").addEventListener("click", () => { navigator.clipboard.writeText($("#resDesc").value); toast("Description copied — paste it into YouTube"); });

// ------------------------------------------------------------------ settings
async function loadSettings() {
  const s = await invoke("settings_get");
  $("#sWriter").value = s.writer; $("#sAnthropic").value = s.anthropic_api_key; $("#sMinimax").value = s.minimax_api_key;
  $("#sMmModel").value = s.minimax_model; $("#sTray").checked = s.close_to_tray; $("#sAuto").checked = s.autostart_engine;
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
  try { await invoke("engine_setup", { talkingFaces: $("#engTalking").checked }); toast("Installing — this downloads several GB and can take 10–30 minutes"); }
  catch (e) { toast(String(e), true); }
});
$("#engRestart").addEventListener("click", async () => { await invoke("engine_restart"); toast("Restarting engine…"); });
$("#engUnload").addEventListener("click", async () => { await api("POST", "/gpu/unload").catch(() => {}); toast("GPU memory will be freed"); });
$("#engData").addEventListener("click", async () => { const s = await invoke("engine_status"); T.opener.openPath(s.data_dir); });

// ------------------------------------------------------------------ boot
(async function boot() {
  presetChips();
  loadStories();
  pollEngine();
  try { const s = await invoke("settings_get"); $("#wProvider").value = s.writer; } catch {}
  refreshJobs();
})();
