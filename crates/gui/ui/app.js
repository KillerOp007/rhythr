// rhythr desktop UI logic. Talks to the Rust backend via Tauri
// commands; all state lives in the backend, the UI re-renders from the
// StatusDto it returns.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const dialog = window.__TAURI__.dialog;
const opener = window.__TAURI__.opener;

const $ = (id) => document.getElementById(id);

let status = null;          // last StatusDto from the backend
let timelineData = null;    // health graph + miss ticks
let currentMs = 0;          // scrubber position
let previewTimer = null;
let previewBusy = false;
let previewWanted = false;
let lastOutPath = null;
let rendering = false;
let autoDownloadTried = 0;  // map id of the last automatic download attempt

// ------------------------------------------------------------ formatting

function fmtTime(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function fmtSpeed(v) {
  const r = Math.round(v * 100) / 100;
  return `${r}x`;
}

function esc(text) {
  return String(text ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

// ------------------------------------------------------------ source cards

function renderReplayCard() {
  const body = $("replay-body");
  const r = status?.replay;
  if (!r) {
    body.innerHTML = `<p class="hint">Drop a <code>.rhr</code> file anywhere</p>`;
    return;
  }
  const date = r.unix_ms ? new Date(r.unix_ms).toLocaleString() : "";
  const mods = r.mods.length ? r.mods.map((m) => m.replace(/^mod_/, "")).join(", ") : "none";
  let chip = "";
  if (r.verify) {
    chip = r.verify.consistent
      ? `<span class="chip ok" title="rhythr's own consistency check — not an official Rhythia score verification.">verified</span>`
      : `<span class="chip bad" title="${esc(r.verify.problems.join("\n"))}">inconsistent — possibly modified</span>`;
  }
  const outcome = r.failed
    ? `<span class="chip bad">failed at ${fmtTime(r.fail_time_ms)}</span>`
    : `<span class="chip info">passed</span>`;
  body.innerHTML = `
    <div class="src-title">${esc(r.player)}</div>
    <div class="src-meta">
      ${esc(r.file_name)}<br>
      <b>${r.accuracy_pct.toFixed(2)}%</b> · ${r.hits} hits · ${r.misses} misses<br>
      speed <b>${fmtSpeed(r.speed)}</b> · mods: ${esc(mods)}<br>
      ${esc(date)}
    </div>
    ${outcome} ${chip}`;
}

function renderMapCard() {
  const body = $("map-body");
  const m = status?.map;
  const r = status?.replay;
  $("btn-map-dl").hidden = !(r && !m);
  if (!m) {
    body.innerHTML = r
      ? `<p class="hint">Map id <b>${r.map_id}</b> — download from rhythia.com or browse a local .sspm/.rhm</p>`
      : `<p class="hint">Auto-resolved from the replay</p>`;
    return;
  }
  const src = { local: "local file", cache: "cached download", downloaded: "downloaded" }[m.source] || m.source;
  const warn = m.hash_mismatch
    ? `<span class="chip warn" title="The online map changed since this replay was recorded.">map updated since replay</span>`
    : "";
  body.innerHTML = `
    <div class="src-title">${esc(m.title || m.song_name)}</div>
    <div class="src-meta">
      ${m.note_count.toLocaleString()} notes · ${fmtTime(m.duration_ms)}<br>
      audio ${m.has_audio ? "✓" : "—"} · cover ${m.has_cover ? "✓" : "—"}
    </div>
    <span class="chip info">${esc(src)}</span> ${warn}`;
}

function renderConfigCard() {
  const body = $("config-body");
  const path = status?.config?.path;
  $("btn-config-clear").hidden = !path;
  if (!path) {
    body.innerHTML = `<p class="hint">Optional <code>.rhs</code> — defaults otherwise</p>`;
    return;
  }
  const name = path.split(/[\\/]/).pop();
  body.innerHTML = `
    <div class="src-title">${esc(name)}</div>
    <div class="src-meta">${esc(path)}</div>`;
}

function renderRecent() {
  const list = status?.settings?.recent_replays || [];
  $("card-recent").hidden = list.length === 0;
  $("recent-list").innerHTML = list
    .map((p) => `<li data-path="${esc(p)}" title="${esc(p)}">${esc(p.split(/[\\/]/).pop())}</li>`)
    .join("");
}

// ------------------------------------------------------------ HUD tab

const HUD_GROUPS = [
  { title: "Header", items: [
    ["song_info", "Title & clock", "“Watching … play …”"],
    ["song_progress", "Song progress bar", ""],
  ]},
  { title: "Left panel", items: [
    ["combo_ring", "Combo ring", "shape-shifting progress ring"],
    ["pauses", "Pauses", ""],
    ["grade", "Grade", "SS / S / A …"],
    ["accuracy", "Accuracy", ""],
  ]},
  { title: "Right panel", items: [
    ["score", "Score", ""],
    ["points", "Points (RP)", ""],
    ["misses", "Misses", ""],
    ["notes", "Notes", "hit / total counter"],
  ]},
  { title: "Playfield", items: [
    ["health_bar", "Health bar", ""],
    ["combo_text", "Centre combo number", ""],
    ["miss_marker", "Miss marker", "red X on missed notes"],
    ["speed_label", "Speed label", "S-notation under health"],
  ]},
];

function renderHudTab() {
  const wrap = $("hud-groups");
  const base = status?.config?.base_hud || {};
  const eff = status?.config?.effective_hud || {};
  const overrides = status?.settings?.hud_overrides || {};
  wrap.innerHTML = HUD_GROUPS.map((g) => `
    <div class="hud-group-title">${g.title}</div>
    ${g.items.map(([key, name, sub]) => {
      const on = eff[key];
      const modified = key in overrides && overrides[key] !== base[key];
      return `
        <div class="hud-row" data-key="${key}" data-on="${on ? 1 : 0}" role="switch"
             aria-checked="${on}" tabindex="0"
             title="${modified ? "Overridden — config says " + (base[key] ? "on" : "off") : "Click to toggle"}">
          <span class="name">${name}${sub ? `<small>${sub}</small>` : ""}</span>
          ${modified ? `<span class="dot mod"></span>` : ""}
          <span class="switch"></span>
        </div>`;
    }).join("")}`).join("");

  wrap.querySelectorAll(".hud-row").forEach((row) => {
    const toggle = async () => {
      const key = row.dataset.key;
      const next = !(eff[key]);
      // An override matching the config baseline is just removed.
      const value = next === base[key] ? null : next;
      await call(() => invoke("set_hud_override", { key, value }));
      schedulePreview();
    };
    row.addEventListener("click", toggle);
    row.addEventListener("keydown", (e) => {
      if (e.key === " " || e.key === "Enter") { e.preventDefault(); toggle(); }
    });
  });
}

// ------------------------------------------------------------ output tab

function renderOutputTab() {
  const s = status?.settings;
  if (!s) return;
  const res = `${s.width}x${s.height}`;
  const resSel = $("set-res");
  if (![...resSel.options].some((o) => o.value === res)) {
    const opt = document.createElement("option");
    opt.value = res;
    opt.textContent = `${s.width} × ${s.height}`;
    resSel.appendChild(opt);
  }
  resSel.value = res;
  $("set-fps").value = String(s.fps);
  $("set-crf").value = String(s.crf);
  $("crf-val").textContent = String(s.crf);
  $("set-encoder").value = s.encoder;
  $("set-results").value = String(Math.round(s.results_secs));
  $("set-mblur").value = String(s.motion_blur);
  $("set-musicvol").value = String(s.music_volume);
  $("musicvol-val").textContent = `${s.music_volume}%`;
  $("set-hitvol").value = String(s.hitsound_volume);
  $("hitvol-val").textContent = `${s.hitsound_volume}%`;
  $("set-outdir").value = s.output_dir || "";
  $("set-filename").value = s.file_name || "";
  $("set-assets").value = s.game_assets || "";
  $("set-ffmpeg").value = s.ffmpeg || "";
  if (status?.replay && !s.file_name) {
    invoke("suggest_file_name").then((n) => { $("set-filename").placeholder = n; });
  }
}

function assetsNote(text) {
  const el = $("assets-note");
  el.textContent = text || "";
  el.hidden = !text;
}

async function applyGameAssets(path) {
  const fromExe = path.toLowerCase().endsWith(".exe");
  assetsNote(fromExe ? "Extracting skin assets from the game… (a few seconds)" : "");
  try {
    await call(() => invoke("set_game_assets", { path }));
    assetsNote(fromExe ? "Assets extracted — built-in skins now render with the real textures." : "");
    schedulePreview();
  } catch (e) {
    assetsNote(String(e));
  }
}

async function pushOutput(update) {
  await call(() => invoke("set_output", { update }));
}

// ------------------------------------------------------------ scrubber

function drawScrubber() {
  const canvas = $("scrubber");
  const ctx = canvas.getContext("2d");
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth, h = canvas.clientHeight;
  if (canvas.width !== w * dpr) { canvas.width = w * dpr; canvas.height = h * dpr; }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  if (!timelineData) return;
  const { length_ms, health, miss_times, fail_ms } = timelineData;

  // Health area graph.
  ctx.beginPath();
  ctx.moveTo(0, h);
  health.forEach((v, i) => {
    const x = (w * (i + 1)) / health.length;
    ctx.lineTo(x, h - v * (h - 8) - 2);
  });
  ctx.lineTo(w, h);
  ctx.closePath();
  ctx.fillStyle = "rgba(47, 214, 208, 0.16)";
  ctx.fill();
  ctx.beginPath();
  health.forEach((v, i) => {
    const x = (w * (i + 1)) / health.length;
    const y = h - v * (h - 8) - 2;
    i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
  });
  ctx.strokeStyle = "rgba(47, 214, 208, 0.7)";
  ctx.lineWidth = 1.2;
  ctx.stroke();

  // Miss ticks.
  ctx.fillStyle = "rgba(255, 93, 108, 0.85)";
  for (const t of miss_times) {
    const x = (t / length_ms) * w;
    ctx.fillRect(x - 0.75, 2, 1.5, h - 4);
  }
  // Fail point.
  if (fail_ms != null) {
    const x = (fail_ms / length_ms) * w;
    ctx.fillStyle = "#ff5d6c";
    ctx.fillRect(x - 1, 0, 2, h);
  }
  // Playhead.
  const px = (currentMs / length_ms) * w;
  ctx.fillStyle = "#e8edf4";
  ctx.fillRect(px - 1, 0, 2, h);
}

function scrubTo(clientX) {
  const canvas = $("scrubber");
  const rect = canvas.getBoundingClientRect();
  const frac = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
  currentMs = frac * (timelineData?.length_ms || 0);
  $("scrub-time").textContent = fmtTime(currentMs);
  drawScrubber();
  schedulePreview();
}

function initScrubber() {
  const canvas = $("scrubber");
  let dragging = false;
  canvas.addEventListener("pointerdown", (e) => {
    dragging = true;
    canvas.setPointerCapture(e.pointerId);
    scrubTo(e.clientX);
  });
  canvas.addEventListener("pointermove", (e) => { if (dragging) scrubTo(e.clientX); });
  canvas.addEventListener("pointerup", () => { dragging = false; });
  new ResizeObserver(drawScrubber).observe(canvas);
}

// ------------------------------------------------------------ preview

function schedulePreview() {
  if (!status?.replay || !status?.map || rendering) return;
  previewWanted = true;
  clearTimeout(previewTimer);
  previewTimer = setTimeout(runPreview, 140);
}

async function runPreview() {
  if (previewBusy) return;           // re-queued when the current one lands
  if (!previewWanted) return;
  previewWanted = false;
  previewBusy = true;
  try {
    const url = await invoke("preview", { timeMs: currentMs });
    const img = $("preview-img");
    img.src = url;
    img.hidden = false;
    $("dropzone").hidden = true;
    $("btn-frame").hidden = false;
    $("preview-msg").hidden = true;
  } catch (e) {
    showPreviewMsg(String(e));
  } finally {
    previewBusy = false;
    if (previewWanted) runPreview();
  }
}

function showPreviewMsg(text) {
  const el = $("preview-msg");
  el.textContent = text;
  el.hidden = false;
}

// ------------------------------------------------------------ render flow

function setRenderingUi(on) {
  rendering = on;
  $("btn-render").hidden = on;
  $("btn-cancel").hidden = !on;
  $("btn-frame").disabled = on;
  $("render-progress-track").hidden = !on;
  if (!on) $("render-progress-fill").style.width = "0%";
}

async function startRender() {
  $("btn-open-out").hidden = true;
  try {
    lastOutPath = await invoke("start_render");
    setRenderingUi(true);
    $("render-text").textContent = "Starting…";
  } catch (e) {
    $("render-text").textContent = String(e);
  }
}

function initRenderEvents() {
  listen("render-stage", (e) => {
    if (rendering) $("render-text").textContent = e.payload;
  });
  listen("render-progress", (e) => {
    const { done, total, fps, eta_secs } = e.payload;
    const pct = total ? (100 * done) / total : 0;
    $("render-progress-fill").style.width = `${pct.toFixed(1)}%`;
    $("render-text").textContent =
      `${pct.toFixed(0)}% — frame ${done.toLocaleString()} / ${total.toLocaleString()}` +
      ` · ${fps.toFixed(0)} fps · ETA ${fmtTime(eta_secs * 1000)}`;
  });
  listen("render-done", (e) => {
    setRenderingUi(false);
    updateRenderButton();
    lastOutPath = e.payload;
    $("render-text").textContent = `Done — ${e.payload}`;
    $("btn-open-out").hidden = false;
  });
  listen("render-cancelled", () => {
    setRenderingUi(false);
    updateRenderButton();
    $("render-text").textContent = "Cancelled.";
  });
  listen("render-error", (e) => {
    setRenderingUi(false);
    updateRenderButton();
    $("render-text").textContent = `Error: ${e.payload}`;
  });
}

function updateRenderButton() {
  const ready = !!(status?.replay && status?.map);
  $("btn-render").disabled = !ready || rendering;
  if (!rendering) {
    let readyText = "Ready to render the full run";
    const fps = status?.settings?.last_render_fps || 0;
    if (ready && fps > 1) {
      const runMs = status.replay.failed ? status.replay.fail_time_ms : status.replay.length_ms;
      const frames = (runMs / 1000) * (status.settings.fps || 60);
      const est = frames / fps + (status.settings.results_secs || 0);
      readyText = `Ready to render the full run (~${fmtTime(est * 1000)} at last speed)`;
    }
    $("render-text").textContent = ready
      ? readyText
      : status?.replay
        ? "Map missing — download or browse one"
        : "Load a replay to render";
  }
}

// ------------------------------------------------------------ status glue

async function call(fn) {
  try {
    const st = await fn();
    // Await: applyStatus ends with updateRenderButton writing the status
    // line — callers that print their own message must come after it.
    if (st) await applyStatus(st);
    return st;
  } catch (e) {
    showPreviewMsg(String(e));
    throw e;
  }
}

async function applyStatus(st) {
  const hadPair = !!(status?.replay && status?.map);
  const replayChanged = status?.replay?.path !== st.replay?.path;
  const mapChanged = status?.map?.path !== st.map?.path;
  status = st;
  renderReplayCard();
  renderMapCard();
  renderConfigCard();
  renderRecent();
  renderHudTab();
  renderOutputTab();
  updateRenderButton();

  // A page (re)load during an active render must show the rendering state.
  if (st.rendering && !rendering) setRenderingUi(true);

  // A replay without its map: fetch it from rhythia.com right away —
  // once per map id, so a failure (offline, unpublished map) falls back
  // to the manual Download/Browse buttons instead of looping.
  if (st.replay && !st.map && st.replay.map_id > 0 && autoDownloadTried !== st.replay.map_id) {
    autoDownloadTried = st.replay.map_id;
    $("map-body").innerHTML = `<p class="hint">Downloading map from rhythia.com…</p>`;
    invoke("download_map")
      .then(async (st2) => { await applyStatus(st2); loadNote("Map downloaded."); })
      .catch((e) => {
        $("map-body").innerHTML =
          `<p class="hint">Automatic download failed: ${esc(String(e))} — try Download again or Browse a local file.</p>`;
        $("btn-map-dl").hidden = false;
      });
  }

  const hasPair = !!(st.replay && st.map);
  $("scrub-row").hidden = !hasPair;
  if (hasPair && (replayChanged || mapChanged || !hadPair)) {
    currentMs = Math.min(15000, (st.replay.length_ms || 0) / 2);
    timelineData = await invoke("timeline", { samples: 600 }).catch(() => null);
    $("scrub-len").textContent = fmtTime(timelineData?.length_ms || 0);
    $("scrub-time").textContent = fmtTime(currentMs);
    drawScrubber();
    schedulePreview();
  } else if (!hasPair) {
    timelineData = null;
    $("preview-img").hidden = true;
    $("btn-frame").hidden = true;
    $("dropzone").hidden = false;
  }
}

// ------------------------------------------------------------ file loading

function loadNote(text) {
  // The render bar is always visible — use it as the load status line
  // (unless a render is writing progress there).
  if (!rendering) $("render-text").textContent = text;
}

async function loadPath(path) {
  const name = path.split(/[\\/]/).pop();
  const lower = path.toLowerCase();
  try {
    if (lower.endsWith(".rhr")) {
      await call(() => invoke("load_replay", { path }));
      loadNote(`Loaded replay: ${name}`);
    } else if (lower.endsWith(".sspm") || lower.endsWith(".rhm") || lower.endsWith(".json")) {
      await call(() => invoke("load_map", { path }));
      loadNote(`Loaded map: ${name}`);
    } else if (lower.endsWith(".rhs")) {
      await call(() => invoke("load_config", { path }));
      loadNote(`Loaded skin: ${name}`);
      schedulePreview();
    } else {
      loadNote(`Unsupported file type: ${name}`);
      showPreviewMsg(`Unsupported file type: ${name}`);
    }
  } catch (e) {
    // Surface the reason where it is always visible; don't let one bad
    // file abort the rest of a multi-file drop.
    loadNote(`Could not load ${name}: ${e}`);
  }
}

function initDragDrop() {
  listen("tauri://drag-enter", () => { $("drop-overlay").hidden = false; });
  listen("tauri://drag-leave", () => { $("drop-overlay").hidden = true; });
  listen("tauri://drag-drop", async (e) => {
    $("drop-overlay").hidden = true;
    // Replays first: loading a replay may swap the auto-resolved map, so a
    // map dropped in the same gesture must land after it.
    const rank = (p) => (p.toLowerCase().endsWith(".rhr") ? 0 : 1);
    const paths = [...(e.payload.paths || [])].sort((a, b) => rank(a) - rank(b));
    for (const p of paths) await loadPath(p);
  });
  // Second app instance (e.g. double-clicked .rhr) forwards its file here.
  listen("open-replay", (e) => loadPath(e.payload));
}

// ------------------------------------------------------------ wiring

function initControls() {
  $("btn-replay").addEventListener("click", async () => {
    const p = await dialog.open({ filters: [{ name: "Rhythia replay", extensions: ["rhr"] }] });
    if (p) await loadPath(p);
  });
  $("btn-map").addEventListener("click", async () => {
    const p = await dialog.open({ filters: [{ name: "Map", extensions: ["sspm", "rhm", "json"] }] });
    if (p) await loadPath(p);
  });
  $("btn-map-dl").addEventListener("click", async () => {
    $("btn-map-dl").disabled = true;
    $("map-body").innerHTML = `<p class="hint">Downloading from rhythia.com…</p>`;
    try {
      await call(() => invoke("download_map"));
    } catch (e) {
      $("map-body").innerHTML = `<p class="hint">${esc(String(e))}</p>`;
    } finally {
      $("btn-map-dl").disabled = false;
    }
  });
  $("btn-config").addEventListener("click", async () => {
    const p = await dialog.open({ filters: [{ name: "Skin config", extensions: ["rhs"] }] });
    if (p) await loadPath(p);
  });
  $("btn-config-clear").addEventListener("click", () => call(() => invoke("clear_config")).then(schedulePreview));

  $("recent-list").addEventListener("click", (e) => {
    const li = e.target.closest("li[data-path]");
    if (li) loadPath(li.dataset.path);
  });

  // Tabs.
  document.querySelectorAll(".tab").forEach((tab) => {
    tab.addEventListener("click", () => {
      document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("active", t === tab));
      $("tab-output").hidden = tab.dataset.tab !== "output";
      $("tab-hud").hidden = tab.dataset.tab !== "hud";
    });
  });

  // Output settings.
  $("set-res").addEventListener("change", () => {
    const [w, h] = $("set-res").value.split("x").map(Number);
    pushOutput({ width: w, height: h });
  });
  $("set-fps").addEventListener("change", () => pushOutput({ fps: Number($("set-fps").value) }));
  $("set-crf").addEventListener("input", () => { $("crf-val").textContent = $("set-crf").value; });
  $("set-crf").addEventListener("change", () => pushOutput({ crf: Number($("set-crf").value) }));
  $("set-encoder").addEventListener("change", () => pushOutput({ encoder: $("set-encoder").value }));
  $("set-results").addEventListener("change", () => pushOutput({ results_secs: Number($("set-results").value) }));
  $("set-mblur").addEventListener("change", () => pushOutput({ motion_blur: Number($("set-mblur").value) }));
  $("set-musicvol").addEventListener("input", () => { $("musicvol-val").textContent = `${$("set-musicvol").value}%`; });
  $("set-musicvol").addEventListener("change", () => pushOutput({ music_volume: Number($("set-musicvol").value) }));
  $("set-hitvol").addEventListener("input", () => { $("hitvol-val").textContent = `${$("set-hitvol").value}%`; });
  $("set-hitvol").addEventListener("change", () => pushOutput({ hitsound_volume: Number($("set-hitvol").value) }));
  $("set-filename").addEventListener("change", () => pushOutput({ file_name: $("set-filename").value }));
  $("set-ffmpeg").addEventListener("change", () => pushOutput({ ffmpeg: $("set-ffmpeg").value }));
  $("btn-outdir").addEventListener("click", async () => {
    const p = await dialog.open({ directory: true });
    if (p) pushOutput({ output_dir: p });
  });
  $("btn-assets").addEventListener("click", async () => {
    const p = await dialog.open({ directory: true });
    if (p) await applyGameAssets(p);
  });
  $("btn-assets-exe").addEventListener("click", async () => {
    const p = await dialog.open({ filters: [{ name: "Rhythia game", extensions: ["exe"] }] });
    if (p) await applyGameAssets(p);
  });
  $("btn-assets-auto").addEventListener("click", async () => {
    assetsNote("Searching Steam installation…");
    const exe = await invoke("detect_game").catch(() => null);
    if (!exe) {
      assetsNote("Game not found in the usual Steam folders — use “From rhythia.exe…” instead.");
      return;
    }
    await applyGameAssets(exe);
  });

  $("btn-hud-reset").addEventListener("click", async () => {
    await call(() => invoke("reset_hud_overrides"));
    schedulePreview();
  });

  $("btn-render").addEventListener("click", startRender);
  $("btn-cancel").addEventListener("click", () => invoke("cancel_render"));
  $("btn-open-out").addEventListener("click", () => {
    if (lastOutPath) {
      opener.revealItemInDir(lastOutPath).catch((e) => {
        $("render-text").textContent = `Could not open file manager: ${e}`;
      });
    }
  });

  $("btn-frame").addEventListener("click", async () => {
    const suggested = `frame-${fmtTime(currentMs).replace(":", "m")}s.png`;
    const p = await dialog.save({
      defaultPath: suggested,
      filters: [{ name: "PNG image", extensions: ["png"] }],
    });
    if (!p) return;
    $("btn-frame").disabled = true;
    try {
      await invoke("export_frame", { timeMs: currentMs, path: p });
      $("render-text").textContent = `Frame saved — ${p}`;
    } catch (e) {
      showPreviewMsg(String(e));
    } finally {
      $("btn-frame").disabled = false;
    }
  });
}

async function initEncoders() {
  try {
    const probe = await invoke("probe_encoders");
    const list = probe.available;
    const sel = $("set-encoder");
    const labels = {
      auto: "Auto (fastest available)",
      x264: "x264 (software)",
      nvenc: "NVENC (NVIDIA)",
      qsv: "Quick Sync (Intel)",
      vaapi: "VAAPI (AMD/Intel)",
    };
    sel.innerHTML = list.map((e) => `<option value="${e}">${labels[e] || e}</option>`).join("");
    const saved = status?.settings?.encoder || "auto";
    if (list.includes(saved)) {
      sel.value = saved;
    } else {
      // e.g. settings from another machine — keep backend and UI in agreement.
      sel.value = "auto";
      pushOutput({ encoder: "auto" });
    }
    const hw = list.filter((e) => e !== "auto" && e !== "x264");
    $("topbar-info").textContent = hw.length
      ? `Hardware encoder: ${hw.map((e) => labels[e]?.split(" ")[0] || e).join(", ")}`
      : "Software encoding (x264)";
    // Say WHY a hardware encoder is missing (e.g. nvenc wants a newer
    // NVIDIA driver) — otherwise "only x264" looks like a bug.
    const note = $("encoder-note");
    const reasons = Object.entries(probe.unavailable || {})
      .filter(([e]) => e !== "vaapi" || hw.length === 0) // vaapi absence on Windows is normal
      .map(([e, r]) => `${labels[e]?.split(" ")[0] || e}: ${r}`);
    if (note) {
      note.textContent = hw.length === 0 && reasons.length ? reasons.join("  ·  ") : "";
      note.hidden = !note.textContent;
    }
  } catch { /* probing is best-effort */ }
}

// ------------------------------------------------------------ boot

async function initUpdater() {
  // Non-blocking: check GitHub for a newer release; the user decides.
  try {
    const update = await window.__TAURI__.updater.check();
    if (!update) return;
    $("update-text").textContent = `Update ${update.version} is available.`;
    $("update-banner").hidden = false;
    $("btn-update-later").onclick = () => { $("update-banner").hidden = true; };
    $("btn-update").onclick = async () => {
      $("btn-update").disabled = true;
      let got = 0;
      try {
        await update.downloadAndInstall((e) => {
          if (e.event === "Progress") {
            got += e.data.chunkLength;
            $("update-text").textContent = `Downloading update… ${(got / 1048576).toFixed(0)} MB`;
          } else if (e.event === "Finished") {
            $("update-text").textContent = "Installing…";
          }
        });
        await window.__TAURI__.process.relaunch();
      } catch (err) {
        $("update-text").textContent = `Update failed: ${err}`;
        $("btn-update").disabled = false;
      }
    };
  } catch { /* offline or first run — try again next launch */ }
}

window.addEventListener("DOMContentLoaded", async () => {
  initControls();
  initScrubber();
  initDragDrop();
  initRenderEvents();
  const st = await invoke("get_status");
  await applyStatus(st);
  initEncoders();
  setTimeout(initUpdater, 2500);
});
