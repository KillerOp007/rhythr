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
    if (r.verify.consistent) {
      chip = `<span class="chip ok" title="rhythr's own consistency check — not an official Rhythia score verification.">verified</span>`;
    } else if (r.verify.wrong_map) {
      // Blaming the replay here was wrong: the usual cause is a map file
      // that simply is not the chart this run was played on.
      chip = `<span class="chip warn" title="${esc(
        "The recorded hits do not line up with this chart:\n" +
          r.verify.problems.join("\n") +
          "\n\nLoad the map this replay was played on."
      )}">map doesn't match</span>`;
    } else {
      chip = `<span class="chip bad" title="${esc(r.verify.problems.join("\n"))}">inconsistent — possibly modified</span>`;
    }
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
    body.innerHTML = `<p class="hint">Optional <code>.rhs</code> or the game's <code>config.json</code> — defaults otherwise</p>`;
    return;
  }
  const name = path.split(/[\\/]/).pop();
  body.innerHTML = `
    <div class="src-title">${esc(name)}</div>
    <div class="src-meta src-path" title="${esc(path)}">${esc(path)}</div>`;
}

function renderGhostCard() {
  const g = status?.ghost;
  $("btn-ghost-clear").hidden = !g;
  const body = $("ghost-body");
  if (!g) {
    body.innerHTML = `<p class="hint">Optional second replay of the same map: the video splits into two side-by-side runs, each with its own HUD and results. Needs the same speed mod; other mods may differ.</p>`;
    return;
  }
  const warn = g.same_map ? "" : `<span class="chip warn">may be a different map</span>`;
  body.innerHTML = `
    <div class="src-title" style="color:#ff8c3d">${esc(g.player)}</div>
    <div class="src-meta">${esc(g.file_name)}</div>
    <span class="chip info">ghost active</span> ${warn}`;
}

function renderPresetsCard() {
  // Typing a name must survive status refreshes.
  if (document.activeElement === $("preset-name")) return;
  const names = Object.keys(status?.settings?.presets || {});
  $("preset-list").innerHTML = names.length
    ? names
        .map(
          (n) =>
            `<li data-preset="${esc(n)}" title="Apply this preset"><span class="name">${esc(n)}</span><button class="del" title="Delete preset">✕</button></li>`,
        )
        .join("")
    : `<li style="cursor:default">No presets yet — set up a look and hit Save.</li>`;
}

// ------------------------------------------------------------ clip range

// A temporary range while a scrubber handle is being dragged.
let tempClip = null;

function renderClipRow() {
  const clip = status?.clip;
  $("btn-clip-clear").hidden = !clip;
  $("clip-label").textContent = clip
    ? `${fmtTime(clip[0])}–${fmtTime(clip[1])} (${fmtTime(clip[1] - clip[0])})`
    : "";
  renderClipSuggestions();
  drawScrubber();
}

// Suggestions computed from the run itself — offered, never imposed: a
// click sets the range, the handles stay free to move.
function computeClipSuggestions() {
  if (!timelineData) return [];
  const { length_ms, miss_times, fail_ms } = timelineData;
  const out = [];
  const misses = miss_times.filter((t) => t > 0 && t < length_ms).sort((a, b) => a - b);
  // Longest clean streak.
  const marks = [0, ...misses, length_ms];
  let best = [0, 0];
  for (let i = 1; i < marks.length; i++) {
    if (marks[i] - marks[i - 1] > best[1] - best[0]) best = [marks[i - 1], marks[i]];
  }
  if (best[1] - best[0] > 8000) {
    const s = Math.max(0, best[0] + 300);
    const e = Math.min(length_ms, best[1] - 100);
    out.push({ label: `Best streak (${fmtTime(e - s)})`, why: "Longest run without a miss", start: s, end: e });
  }
  // Densest miss window.
  if (misses.length >= 3) {
    const W = 20000;
    let bs = 0;
    let bc = 0;
    for (const t of misses) {
      const c = misses.filter((u) => u >= t && u < t + W).length;
      if (c > bc) {
        bc = c;
        bs = t;
      }
    }
    out.push({
      label: "Toughest part",
      why: `${bc} misses in 20 seconds`,
      start: Math.max(0, bs - 3000),
      end: Math.min(length_ms, bs + W),
    });
  }
  // The fail, or the finish.
  if (fail_ms != null) {
    out.push({
      label: "The fail",
      why: "The last seconds before the run ended",
      start: Math.max(0, fail_ms - 18000),
      end: Math.min(length_ms, fail_ms + 1500),
    });
  } else if (length_ms > 25000) {
    out.push({ label: "Finish", why: "The last 20 seconds", start: length_ms - 20000, end: length_ms });
  }
  return out;
}

function renderClipSuggestions() {
  const el = $("clip-suggest");
  const sug = computeClipSuggestions();
  el.innerHTML = sug
    .map(
      (s, i) =>
        `<span class="chip info" data-sug="${i}" title="${esc(s.why)} — just a suggestion: set it, then move the handles however you like">${esc(s.label)}</span>`,
    )
    .join(" ");
  el.querySelectorAll("[data-sug]").forEach((c) =>
    c.addEventListener("click", async () => {
      const s = sug[Number(c.dataset.sug)];
      try {
        await call(() => invoke("set_clip", { startMs: s.start, endMs: s.end }));
        renderClipRow();
        schedulePreview();
      } catch (e) {
        loadNote(String(e));
      }
    }),
  );
}

async function setClipEdge(isIn) {
  const len = timelineData?.length_ms || 0;
  const cur = status?.clip;
  let s = cur ? cur[0] : 0;
  let e = cur ? cur[1] : len;
  if (isIn) {
    s = currentMs;
    if (e <= s + 500) e = len;
  } else {
    e = currentMs;
    if (s >= e - 500) s = 0;
  }
  try {
    await call(() => invoke("set_clip", { startMs: s, endMs: e }));
    renderClipRow();
  } catch (err) {
    loadNote(String(err));
  }
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

function renderBackgroundCard() {
  const p = status?.settings?.background;
  $("btn-bg-clear").hidden = !p;
  const body = $("bg-body");
  // Re-rendering would yank the dim slider out from under an active drag —
  // but only skip while the CONTENT is unchanged (the slider keeps focus
  // after use, and a background dropped then must still show up).
  if (
    body.contains(document.activeElement) &&
    document.activeElement?.type === "range" &&
    body.dataset.bgPath === (p || "")
  ) {
    return;
  }
  body.dataset.bgPath = p || "";
  if (!p) {
    body.innerHTML = `<p class="hint">Optional image or video shown behind the gameplay instead of the skin background (videos play muted and looped). The results screen keeps its own look. Drop a file here or browse.</p>`;
    return;
  }
  const s = status.settings;
  const dur = status?.bg_video_duration;
  const row = (id, label, min, max, step, value, valText) => `
    <label class="hint" style="display:flex;align-items:center;gap:8px;margin-top:6px">
      <span style="width:44px">${label}</span>
      <input type="range" id="${id}" min="${min}" max="${max}" step="${step}" value="${value}" style="flex:1">
      <span id="${id}-val" style="width:44px;text-align:right">${valText}</span>
    </label>`;
  body.innerHTML = `
    <div class="src-meta">${esc(p.split(/[\\/]/).pop())}</div>
    <span class="chip info">background active</span>
    ${row("bg-dim", "Dim", 0, 100, 1, s.background_dim ?? 60, `${s.background_dim ?? 60}%`)}
    ${row("bg-zoom", "Zoom", 100, 300, 1, s.background_zoom ?? 100, `${s.background_zoom ?? 100}%`)}
    ${row("bg-offx", "Pos X", -50, 50, 1, s.background_off_x ?? 0, `${s.background_off_x ?? 0}%`)}
    ${row("bg-offy", "Pos Y", -50, 50, 1, s.background_off_y ?? 0, `${s.background_off_y ?? 0}%`)}
    ${dur ? row("bg-start", "Start", 0, Math.max(0.1, dur - 0.5).toFixed(1), 0.1, s.background_start ?? 0, fmtTime((s.background_start ?? 0) * 1000)) : ""}
    ${dur ? `
    <label class="hint" style="display:flex;align-items:center;gap:8px;margin-top:6px"
           title="Only matters when you render a clip: should the video be at the position it would have reached since 0:00 of the song, or restart at the clip start?">
      <span style="width:44px">Timing</span>
      <select id="bg-sync" style="flex:1">
        <option value="song">Follow song position</option>
        <option value="clip">Restart at clip start</option>
      </select>
    </label>` : ""}`;
  // Every slider debounces into its backend patch and refreshes the
  // preview; the value label updates instantly.
  const wire = (id, fmt, push) => {
    const sl = $(id);
    if (!sl) return;
    let timer = null;
    const send = async () => {
      await call(push);
      schedulePreview();
    };
    sl.addEventListener("input", () => {
      $(`${id}-val`).textContent = fmt(sl.value);
      clearTimeout(timer);
      timer = setTimeout(send, 60);
    });
    sl.addEventListener("change", () => {
      clearTimeout(timer);
      send();
    });
  };
  wire("bg-dim", (v) => `${v}%`, () => invoke("set_background_dim", { pct: Number($("bg-dim").value) }));
  wire("bg-zoom", (v) => `${v}%`, () =>
    invoke("set_background_transform", { patch: { zoom: Number($("bg-zoom").value) } }));
  wire("bg-offx", (v) => `${v}%`, () =>
    invoke("set_background_transform", { patch: { off_x: Number($("bg-offx").value) } }));
  wire("bg-offy", (v) => `${v}%`, () =>
    invoke("set_background_transform", { patch: { off_y: Number($("bg-offy").value) } }));
  wire("bg-start", (v) => fmtTime(v * 1000), () =>
    invoke("set_background_transform", { patch: { start: Number($("bg-start").value) } }));
  const sync = $("bg-sync");
  if (sync) {
    sync.value = (s.background_sync_song ?? true) ? "song" : "clip";
    sync.addEventListener("change", async () => {
      await call(() => invoke("set_background_transform", {
        patch: { sync_song: sync.value === "song" },
      }));
      schedulePreview();
    });
  }
}

// Settings entry for a draggable overlay extra ("error"/"aim" kept their
// historic short keys; the race extras use their settings name directly).
function meterSettings(key) {
  const field = key === "error" ? "error_meter" : key === "aim" ? "aim_meter" : key;
  return status?.settings?.[field] || {};
}

function meterRow(key, label, m) {
  const opts = !m.enabled ? "" : `
    <div class="meter-opts">
      <label>Size <input type="range" data-meter="${key}" data-prop="scale" min="40" max="250" step="1" value="${Math.round(m.scale * 100)}"></label>
      <label>Opacity <input type="range" data-meter="${key}" data-prop="alpha" min="10" max="100" step="1" value="${Math.round(m.alpha * 100)}"></label>
      <div class="sub">Move and resize it with Edit HUD on.</div>
    </div>`;
  return `
    <div class="hud-row meter-toggle" data-meter-key="${key}" data-on="${m.enabled ? 1 : 0}" role="switch"
         aria-checked="${m.enabled}" tabindex="0">
      <span class="name">${label}</span>
      <span class="switch"></span>
    </div>${opts}`;
}

function renderHudTab() {
  const wrap = $("hud-groups");
  // Rebuilding the DOM would yank a slider out from under an active drag —
  // the slider itself is the source of truth then, skip the re-render.
  if (wrap.contains(document.activeElement) && document.activeElement?.type === "range") {
    return;
  }
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
    }).join("")}`).join("")
    + `<div class="hud-group-title">Extras (not in the game)</div>`
    + meterRow("error", "Hit error bar (early/late ms)", status?.settings?.error_meter || {})
    + meterRow("aim", "Aim accuracy (cursor vs. note centre)", status?.settings?.aim_meter || {})
    + (status?.ghost
      ? `<div class="hud-group-title">Ghost race</div>`
        + meterRow("race_delta", "Racing delta (score lead, bar + results graph)", status?.settings?.race_delta || {})
      : "");

  wrap.querySelectorAll(".meter-toggle").forEach((row) => {
    const key = row.dataset.meterKey;
    const toggle = async () => {
      const cur = meterSettings(key);
      await invoke("mark_undo").catch(() => {});
      await call(() => invoke("set_meter", { key, patch: { enabled: !cur.enabled } }));
      schedulePreview();
    };
    row.addEventListener("click", toggle);
    row.addEventListener("keydown", (e) => {
      if (e.key === " " || e.key === "Enter") { e.preventDefault(); toggle(); }
    });
  });
  wrap.querySelectorAll(".meter-opts input[type=range]").forEach((sl) => {
    let timer = null;
    let gestured = false;
    let chain = Promise.resolve();
    const push = (commit) => {
      chain = chain.then(async () => {
        const patch = {};
        patch[sl.dataset.prop] = Number(sl.value) / 100;
        if (commit) {
          await call(() => invoke("set_meter", { key: sl.dataset.meter, patch, commit: true }));
        } else {
          await invoke("set_meter", { key: sl.dataset.meter, patch, commit: false }).catch(() => {});
        }
        schedulePreview();
      }).catch(() => {});
      return chain;
    };
    // Live while sliding; the whole slide is one undo step, committed on
    // release ("change").
    sl.addEventListener("input", () => {
      if (!gestured) {
        gestured = true;
        chain = chain.then(() => invoke("mark_undo")).catch(() => {});
      }
      clearTimeout(timer);
      timer = setTimeout(() => push(false), 60);
    });
    sl.addEventListener("change", () => {
      clearTimeout(timer);
      gestured = false;
      push(true);
    });
  });

  wrap.querySelectorAll(".hud-row:not(.meter-toggle)").forEach((row) => {
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
  $("set-ffmpeg").value = s.ffmpeg || "";
  if (status?.replay && !s.file_name) {
    invoke("suggest_file_name").then((n) => { $("set-filename").placeholder = n; });
  }
}

// In-flight connect progress; status refreshes re-render the card with no
// note, which must not wipe it.
let gameNote = null;

function setGameNote(note) {
  gameNote = note;
  renderGameCard();
}

function renderGameCard(note) {
  const body = $("game-body");
  const ok = status?.game_ok;
  const path = status?.settings?.game_assets;
  note = note ?? gameNote;
  let html = "";
  if (ok) {
    html += `<span class="chip ok">game connected</span>`;
    html += `<div class="src-meta" style="margin-top:6px" title="${esc(path || "")}">Built-in skins use the exact textures and colors.</div>`;
  } else {
    html += `<span class="chip warn">not connected</span>`;
    html += `<div class="src-meta" style="margin-top:6px">Built-in skins are approximated until rhythr reads your Rhythia install. Use Detect, or Locate the game's executable.</div>`;
  }
  if (note) html += `<div class="src-meta" style="margin-top:6px">${esc(note)}</div>`;
  body.innerHTML = html;
}

async function applyGameAssets(path) {
  setGameNote("Extracting assets from the game… (a few seconds)");
  try {
    await call(() => invoke("set_game_assets", { path }));
    setGameNote(null);
    schedulePreview();
  } catch (e) {
    setGameNote(null);
    renderGameCard(String(e));
  }
}

// On startup, connect the game by itself: users otherwise never find the
// button and wonder why their skin looks approximated.
async function autoConnectGame() {
  if (status?.game_ok) return;
  setGameNote("Searching your Steam libraries…");
  const exe = await invoke("detect_game").catch(() => null);
  if (!exe) {
    setGameNote(null);
    renderGameCard("Not found automatically — if the game is installed somewhere unusual, click Locate… and pick its executable.");
    return;
  }
  await applyGameAssets(exe);
}

async function pushOutput(update) {
  await call(() => invoke("set_output", { update }));
  schedulePreview();
}

// ------------------------------------------------------------ scrubber

function drawScrubber() {
  const canvas = $("scrubber");
  const ctx = canvas.getContext("2d");
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth, h = canvas.clientHeight;
  if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
    canvas.width = w * dpr;
    canvas.height = h * dpr;
  }
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
  // Clip range: dim the outside, mark the in/out handles.
  const clip = tempClip || status?.clip;
  if (clip) {
    const x0 = (clip[0] / length_ms) * w;
    const x1 = (clip[1] / length_ms) * w;
    ctx.fillStyle = "rgba(5, 7, 10, 0.55)";
    ctx.fillRect(0, 0, x0, h);
    ctx.fillRect(x1, 0, w - x1, h);
    ctx.fillStyle = "#2fd6d0";
    ctx.fillRect(x0 - 1.5, 0, 3, h);
    ctx.fillRect(x1 - 1.5, 0, 3, h);
    ctx.fillRect(x0 - 1.5, 0, 8, 5);
    ctx.fillRect(x1 - 6.5, 0, 8, 5);
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

// Meter geometry in preview-image pixels, mirroring hud.rs. In a ghost
// split each side is half the image wide and holds its own position.
function meterSides(imgW) {
  return status?.ghost
    ? [{ off: 0, w: imgW / 2, gk: null }, { off: imgW / 2, w: imgW / 2, gk: "ghost" }]
    : [{ off: 0, w: imgW, gk: null }];
}

function meterPos(m, side) {
  const x = side.gk && m.ghost_x != null ? m.ghost_x : m.x;
  const y = side.gk && m.ghost_y != null ? m.ghost_y : m.y;
  return { x, y };
}

function meterBox(key, m, side, imgH) {
  const h = imgH;
  const p = meterPos(m, side);
  const cx = side.off + p.x * side.w;
  const cy = p.y * imgH;
  if (key === "error") {
    const hw = h * 0.16 * (m.scale || 1);
    const th = h * 0.016 * (m.scale || 1) * 1.5;
    return { x: cx - hw, y: cy - th * 1.5, w: hw * 2, h: th * 3 };
  }
  // Race widgets are FULL-frame (side = whole image). refd = min(w, h)
  // is the image height for every landscape/split frame we preview.
  const refd = Math.min(side.w, h);
  if (key === "race_delta") {
    const s = m.scale || 1;
    return { x: cx - refd * 0.23 * s, y: cy - refd * 0.062 * s, w: refd * 0.46 * s, h: refd * 0.125 * s };
  }
  const half = h * 0.065 * (m.scale || 1);
  return { x: cx - half, y: cy - half, w: half * 2, h: half * 2 };
}

// ------------------------------------------------ HUD drag editor
// The hitboxes come from the RENDERER (bounds of the vertices it actually
// draws), so box and pixels can never drift apart — the lesson from the
// old meter-drag offset bug. The frontend only maps frame pixels onto the
// displayed image. Positions save immediately; the render always matches
// the live preview, whether the switch stays on or not.
let hudEditOn = false;
let hudDrag = null;
let hudRefreshQueued = false;

// Snap grid for the editor: cell size as a fraction of the frame height
// (square cells). Persisted locally — a pure editor preference.
const GRID_STEPS = [0, 1 / 48, 1 / 24, 1 / 12, 1 / 6];
const GRID_NAMES = ["Off", "Fine", "Small", "Medium", "Large"];
let gridStep = Number(localStorage.getItem("hud-grid")) || 0;

// Magnet snap for one axis: whichever of the box's two edges or its
// centre sits closest to a grid line wins. A big element aligns its edge
// with a line instead of hovering centred between two of them.
function snapAxisDelta(lo, hi, step) {
  let best = 0;
  let bestAbs = Infinity;
  for (const v of [lo, hi, (lo + hi) / 2]) {
    const d = Math.round(v / step) * step - v;
    if (Math.abs(d) < bestAbs) {
      bestAbs = Math.abs(d);
      best = d;
    }
  }
  return best;
}

// Snap displacement for a box in frame pixels; [0, 0] with the grid off.
function snapBoxDelta(x0, y0, x1, y1, frameH) {
  if (!gridStep || !frameH) return [0, 0];
  const s = frameH * gridStep;
  return [snapAxisDelta(x0, x1, s), snapAxisDelta(y0, y1, s)];
}

// Paints the grid over the preview image (its own child so the edit
// boxes stay unclipped and the lines stop at the frame's edges).
function drawEditGrid() {
  const layer = document.getElementById("hud-edit-layer");
  if (!layer) return;
  let ov = document.getElementById("hud-grid-overlay");
  const img = $("preview-img");
  if (!hudEditOn || !gridStep || !img.naturalHeight) {
    if (ov) ov.remove();
    return;
  }
  if (!ov) {
    ov = document.createElement("div");
    ov.id = "hud-grid-overlay";
    ov.style.position = "absolute";
    ov.style.pointerEvents = "none";
    layer.prepend(ov);
  }
  const r = img.getBoundingClientRect();
  const wr = $("preview-wrap").getBoundingClientRect();
  const cell = r.height * gridStep;
  ov.style.left = `${r.left - wr.left}px`;
  ov.style.top = `${r.top - wr.top}px`;
  ov.style.width = `${r.width}px`;
  ov.style.height = `${r.height}px`;
  ov.style.backgroundImage =
    "linear-gradient(to right, rgba(47, 214, 208, 0.16) 1px, transparent 1px)," +
    "linear-gradient(to bottom, rgba(47, 214, 208, 0.16) 1px, transparent 1px)";
  ov.style.backgroundSize = `${cell}px ${cell}px`;
}

function flushHudRefresh() {
  if (!hudRefreshQueued) return;
  hudRefreshQueued = false;
  refreshHudBoxes();
}

async function refreshHudBoxes() {
  // A preview can land mid-drag; rebuilding the layer then would remove
  // the captured box and kill the drag. Defer until the drag ends.
  if (hudDrag) {
    hudRefreshQueued = true;
    return;
  }
  const layer = $("hud-edit-layer");
  if (!hudEditOn || !status?.replay || !status?.map) {
    layer.innerHTML = "";
    layer.style.backgroundImage = "";
    return;
  }
  let boxes;
  try {
    boxes = await invoke("hud_layout", { timeMs: currentMs });
  } catch {
    return;
  }
  const img = $("preview-img");
  const r = img.getBoundingClientRect();
  const wr = $("preview-wrap").getBoundingClientRect();
  const sx = r.width / (img.naturalWidth || 1);
  const sy = r.height / (img.naturalHeight || 1);
  layer.innerHTML = "";
  for (const b of boxes) {
    const el = document.createElement("div");
    el.className = "hud-edit-box";
    el.dataset.key = b.key;
    const pad = 3;
    el.style.left = `${r.left - wr.left + b.x0 * sx - pad}px`;
    el.style.top = `${r.top - wr.top + b.y0 * sy - pad}px`;
    el.style.width = `${(b.x1 - b.x0) * sx + pad * 2}px`;
    el.style.height = `${(b.y1 - b.y0) * sy + pad * 2}px`;
    el.title = b.key.replace("_", " ");
    // The element's true frame-pixel size (no editor padding) — the snap
    // math aligns real edges with grid lines, not the dashed outline.
    el.dataset.fw = String(b.x1 - b.x0);
    el.dataset.fh = String(b.y1 - b.y0);
    const grip = document.createElement("div");
    grip.className = "hud-resize";
    grip.title = "Drag to resize";
    el.appendChild(grip);
    layer.appendChild(el);
  }
  // Meters join the editor as real boxes too (client-side geometry —
  // they have no renderer hitbox): drag to move, corner grip to resize.
  const iw = img.naturalWidth || 1;
  const ih = img.naturalHeight || 1;
  const addMeter = (key, m, side) => {
    const b = meterBox(key, m, side, ih);
    const el = document.createElement("div");
    el.className = "hud-edit-box meter";
    el.dataset.meterKey = key;
    el.dataset.gk = side.gk || "";
    el.dataset.sideOff = String(side.off);
    el.dataset.sideW = String(side.w);
    el.dataset.fw = String(b.w);
    el.dataset.fh = String(b.h);
    const pad = 3;
    el.style.left = `${r.left - wr.left + b.x * sx - pad}px`;
    el.style.top = `${r.top - wr.top + b.y * sy - pad}px`;
    el.style.width = `${b.w * sx + pad * 2}px`;
    el.style.height = `${b.h * sy + pad * 2}px`;
    el.title =
      key === "error" ? "hit error bar" : key === "aim" ? "aim accuracy" : "racing delta";
    const grip = document.createElement("div");
    grip.className = "hud-resize";
    grip.title = "Drag to resize";
    el.appendChild(grip);
    layer.appendChild(el);
  };
  for (const key of ["error", "aim"]) {
    const m = meterSettings(key);
    if (m?.enabled) for (const side of meterSides(iw)) addMeter(key, m, side);
  }
  const rd = meterSettings("race_delta");
  if (status?.ghost && rd?.enabled) addMeter("race_delta", rd, { off: 0, w: iw, gk: null });
  drawEditGrid();
}

function initHudEdit() {
  const wrap = $("preview-wrap");
  const layer = document.createElement("div");
  layer.id = "hud-edit-layer";
  wrap.appendChild(layer);

  $("btn-edit-hud").addEventListener("click", () => {
    hudEditOn = !hudEditOn;
    $("btn-edit-hud").classList.toggle("active", hudEditOn);
    $("btn-hud-reset").hidden = !hudEditOn;
    $("btn-hud-grid").hidden = !hudEditOn;
    $("btn-hud-undo").hidden = !hudEditOn;
    $("btn-hud-redo").hidden = !hudEditOn;
    if (!hudEditOn) gridMenu.hidden = true;
    refreshHudBoxes();
  });

  const doHistory = async (cmd) => {
    if (cmd === "undo_layout" ? !status?.can_undo : !status?.can_redo) return;
    // A focused slider would pin the HUD tab to its stale value.
    if (document.activeElement?.type === "range") document.activeElement.blur();
    try {
      const st = await invoke(cmd);
      await applyStatus(st);
      schedulePreview();
      refreshHudBoxes();
    } catch (e) {
      loadNote(String(e));
    }
  };
  $("btn-analyze").addEventListener("click", () => {
    call(() => invoke("open_analyze_window"));
  });
  $("btn-hud-undo").addEventListener("click", () => doHistory("undo_layout"));
  $("btn-hud-redo").addEventListener("click", () => doHistory("redo_layout"));
  document.addEventListener("keydown", (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    const t = document.activeElement;
    const typing = t && (t.tagName === "TEXTAREA" || t.isContentEditable
      || (t.tagName === "INPUT" && !["range", "checkbox", "button"].includes(t.type)));
    if (typing) return;
    const k = e.key.toLowerCase();
    if (k === "z" && !e.shiftKey) {
      e.preventDefault();
      doHistory("undo_layout");
    } else if (k === "y" || (k === "z" && e.shiftKey)) {
      e.preventDefault();
      doHistory("redo_layout");
    }
  });

  // Snap-grid picker: a small dropdown under the Grid button.
  const gridMenu = document.createElement("div");
  gridMenu.id = "hud-grid-menu";
  gridMenu.hidden = true;
  GRID_NAMES.forEach((name, i) => {
    const b = document.createElement("button");
    b.textContent = name;
    b.dataset.i = i;
    gridMenu.appendChild(b);
  });
  wrap.appendChild(gridMenu);
  const gridLabel = () => {
    const i = GRID_STEPS.indexOf(gridStep);
    $("btn-hud-grid").textContent = `Grid: ${GRID_NAMES[i >= 0 ? i : 0]}`;
    gridMenu.querySelectorAll("button").forEach((b) => {
      b.classList.toggle("active", Number(b.dataset.i) === (i >= 0 ? i : 0));
    });
  };
  gridLabel();
  $("btn-hud-grid").addEventListener("click", (e) => {
    const btn = e.currentTarget.getBoundingClientRect();
    const wr = wrap.getBoundingClientRect();
    gridMenu.style.left = `${btn.left - wr.left}px`;
    gridMenu.style.top = `${btn.bottom - wr.top + 6}px`;
    gridMenu.hidden = !gridMenu.hidden;
  });
  gridMenu.addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    gridStep = GRID_STEPS[Number(b.dataset.i)] || 0;
    localStorage.setItem("hud-grid", String(gridStep));
    gridLabel();
    gridMenu.hidden = true;
    drawEditGrid();
  });
  document.addEventListener("pointerdown", (e) => {
    if (!gridMenu.hidden && !gridMenu.contains(e.target) && e.target.id !== "btn-hud-grid") {
      gridMenu.hidden = true;
    }
  });

  $("btn-hud-reset").addEventListener("click", async () => {
    try {
      const st = await invoke("reset_hud_layout");
      await applyStatus(st);
      schedulePreview();
      refreshHudBoxes();
    } catch (e) {
      showPreviewMsg(String(e));
    }
  });

  layer.addEventListener("pointerdown", (e) => {
    const box = e.target.closest(".hud-edit-box");
    if (!box) return;
    e.preventDefault();
    box.setPointerCapture(e.pointerId);
    box.classList.add("dragging");
    const meterKey = box.dataset.meterKey || null;
    if (e.target.closest(".hud-resize")) {
      // Corner handle: resize about the box centre.
      const br = box.getBoundingClientRect();
      const cx = br.left + br.width / 2;
      const cy = br.top + br.height / 2;
      hudDrag = {
        box,
        key: box.dataset.key,
        meterKey,
        mode: "resize",
        cx,
        cy,
        startDist: Math.max(8, Math.hypot(e.clientX - cx, e.clientY - cy)),
        baseScale: meterKey
          ? meterSettings(meterKey).scale ?? 1
          : status?.settings?.hud_scales?.[box.dataset.key] ?? 1,
        origLeft: parseFloat(box.style.left),
        origTop: parseFloat(box.style.top),
        origW: box.offsetWidth,
        origH: box.offsetHeight,
        factor: 1,
      };
      return;
    }
    hudDrag = {
      box,
      key: box.dataset.key,
      meterKey,
      mode: "move",
      startX: e.clientX,
      startY: e.clientY,
      origLeft: parseFloat(box.style.left),
      origTop: parseFloat(box.style.top),
    };
  });
  // The backend payload for the gesture's current geometry — shared by the
  // live (uncommitted) pushes and the final commit on release.
  const dragPayload = (d) => {
    if (d.mode === "resize") {
      const scale = d.total ?? d.baseScale;
      return d.meterKey
        ? { cmd: "set_meter", args: { key: d.meterKey, patch: { scale } } }
        : { cmd: "set_hud_scale", args: { key: d.key, scale } };
    }
    // Box centre (wrap px) → frame px → normalised to the HUD's frame,
    // which is HALF the preview in a ghost split.
    const img = $("preview-img");
    const r = img.getBoundingClientRect();
    const wr = $("preview-wrap").getBoundingClientRect();
    const bx = parseFloat(d.box.style.left) + d.box.offsetWidth / 2;
    const by = parseFloat(d.box.style.top) + d.box.offsetHeight / 2;
    let fx = ((bx + wr.left - r.left) / r.width) * (img.naturalWidth || 1);
    let fy = ((by + wr.top - r.top) / r.height) * (img.naturalHeight || 1);
    {
      const bw = parseFloat(d.box.dataset.fw) || 0;
      const bh = parseFloat(d.box.dataset.fh) || 0;
      const [dx, dy] = snapBoxDelta(
        fx - bw / 2,
        fy - bh / 2,
        fx + bw / 2,
        fy + bh / 2,
        img.naturalHeight || 0,
      );
      fx += dx;
      fy += dy;
    }
    if (d.meterKey) {
      // Meter: normalise within its side (a ghost half stores its own
      // position via ghost_x/ghost_y).
      const off = parseFloat(d.box.dataset.sideOff) || 0;
      const sw = parseFloat(d.box.dataset.sideW) || img.naturalWidth || 1;
      const nx = Math.min(1, Math.max(0, (fx - off) / sw));
      const ny = Math.min(1, Math.max(0, fy / (img.naturalHeight || 1)));
      const patch = d.box.dataset.gk ? { ghost_x: nx, ghost_y: ny } : { x: nx, y: ny };
      return { cmd: "set_meter", args: { key: d.meterKey, patch } };
    }
    const vpW = status?.ghost ? (img.naturalWidth || 1) / 2 : img.naturalWidth || 1;
    return {
      cmd: "set_hud_position",
      args: { key: d.key, x: fx / vpW, y: fy / (img.naturalHeight || 1) },
    };
  };

  // Throttled uncommitted pushes keep the preview frame tracking the box
  // under the cursor mid-drag.
  const livePush = (d) => {
    const now = performance.now();
    if (d.pushBusy || (d.lastPush && now - d.lastPush < 90)) return;
    d.lastPush = now;
    d.pushBusy = true;
    const { cmd, args } = dragPayload(d);
    d.chain = d.chain
      .then(() => invoke(cmd, { ...args, commit: false }))
      .then(() => schedulePreview())
      .catch(() => {})
      .finally(() => {
        d.pushBusy = false;
      });
  };

  layer.addEventListener("pointermove", (e) => {
    if (!hudDrag) return;
    const d = hudDrag;
    // The undo snapshot marks the state at gesture start, once per gesture.
    if (!d.gate) {
      d.gate = invoke("mark_undo").catch(() => {});
      d.chain = d.gate;
    }
    if (d.mode === "resize") {
      d.moved = true;
      // Distance from the centre sets the scale; clamp like the backend.
      const dist = Math.max(8, Math.hypot(e.clientX - d.cx, e.clientY - d.cy));
      const total = Math.min(2.5, Math.max(0.4, d.baseScale * (dist / d.startDist)));
      d.factor = total / d.baseScale;
      d.total = total;
      d.box.style.width = `${d.origW * d.factor}px`;
      d.box.style.height = `${d.origH * d.factor}px`;
      d.box.style.left = `${d.origLeft + (d.origW - d.origW * d.factor) / 2}px`;
      d.box.style.top = `${d.origTop + (d.origH - d.origH * d.factor) / 2}px`;
      livePush(d);
      return;
    }
    d.moved = true;
    let nl = d.origLeft + e.clientX - d.startX;
    let nt = d.origTop + e.clientY - d.startY;
    if (gridStep) {
      // Magnet-snap the element's REAL edges/centre in frame space, live.
      const img = $("preview-img");
      const r = img.getBoundingClientRect();
      const wr = $("preview-wrap").getBoundingClientRect();
      const nw = img.naturalWidth || 1;
      const nh = img.naturalHeight || 1;
      const cxF = ((nl + d.box.offsetWidth / 2 + wr.left - r.left) / r.width) * nw;
      const cyF = ((nt + d.box.offsetHeight / 2 + wr.top - r.top) / r.height) * nh;
      const bw = parseFloat(d.box.dataset.fw) || 0;
      const bh = parseFloat(d.box.dataset.fh) || 0;
      const [dx, dy] = snapBoxDelta(cxF - bw / 2, cyF - bh / 2, cxF + bw / 2, cyF + bh / 2, nh);
      nl += dx * (r.width / nw);
      nt += dy * (r.height / nh);
    }
    d.box.style.left = `${nl}px`;
    d.box.style.top = `${nt}px`;
    livePush(d);
  });
  layer.addEventListener("pointerup", async (e) => {
    if (!hudDrag) return;
    const d = hudDrag;
    hudDrag = null;
    d.box.classList.remove("dragging");
    if (!d.moved) {
      flushHudRefresh();
      return;
    }
    try {
      if (d.chain) await d.chain;
      const { cmd, args } = dragPayload(d);
      const st = await invoke(cmd, { ...args, commit: true });
      await applyStatus(st);
      schedulePreview();
    } catch (err) {
      showPreviewMsg(String(err));
    }
    flushHudRefresh();
  });
  // Touch panning or a lost capture cancels the gesture: abandon the drag
  // without saving a position.
  const abortDrag = () => {
    if (!hudDrag) return;
    hudDrag.box.classList.remove("dragging");
    hudDrag = null;
    flushHudRefresh();
  };
  layer.addEventListener("pointercancel", abortDrag);
  layer.addEventListener("lostpointercapture", abortDrag);
}

function initScrubber() {
  const canvas = $("scrubber");
  let dragging = false;
  let clipDrag = null; // "in" | "out" while a clip handle is being dragged
  const fracAt = (clientX) => {
    const rect = canvas.getBoundingClientRect();
    return Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
  };
  canvas.addEventListener("pointerdown", (e) => {
    const clip = status?.clip;
    if (clip && timelineData) {
      // Grab an in/out handle when the pointer is on it.
      const rect = canvas.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const x0 = (clip[0] / timelineData.length_ms) * rect.width;
      const x1 = (clip[1] / timelineData.length_ms) * rect.width;
      const which = Math.abs(x - x0) < 7 ? "in" : Math.abs(x - x1) < 7 ? "out" : null;
      if (which) {
        clipDrag = which;
        tempClip = [...clip];
        canvas.setPointerCapture(e.pointerId);
        return;
      }
    }
    dragging = true;
    canvas.setPointerCapture(e.pointerId);
    scrubTo(e.clientX);
  });
  canvas.addEventListener("pointermove", (e) => {
    if (clipDrag && timelineData) {
      const t = fracAt(e.clientX) * timelineData.length_ms;
      if (clipDrag === "in") tempClip[0] = Math.min(t, tempClip[1] - 500);
      else tempClip[1] = Math.max(t, tempClip[0] + 500);
      drawScrubber();
      return;
    }
    if (dragging) scrubTo(e.clientX);
  });
  canvas.addEventListener("pointerup", async () => {
    if (clipDrag) {
      const range = tempClip;
      clipDrag = null;
      tempClip = null;
      try {
        await call(() => invoke("set_clip", { startMs: range[0], endMs: range[1] }));
      } catch (err) {
        loadNote(String(err));
      }
      renderClipRow();
      return;
    }
    dragging = false;
  });
  // A cancelled gesture (touch pan, lost capture) must not leave a
  // half-dragged clip handle behind.
  const abortScrub = () => {
    dragging = false;
    clipDrag = null;
    tempClip = null;
    drawScrubber();
  };
  canvas.addEventListener("pointercancel", abortScrub);
  canvas.addEventListener("lostpointercapture", abortScrub);
  new ResizeObserver(drawScrubber).observe(canvas);
}

// ------------------------------------------------------------ preview

function schedulePreview() {
  if (!status?.replay || !status?.map || rendering) return;
  previewWanted = true;
  clearTimeout(previewTimer);
  previewTimer = setTimeout(runPreview, 60);
}

async function runPreview() {
  if (previewBusy) return;           // re-queued when the current one lands
  if (!previewWanted) return;
  previewWanted = false;
  previewBusy = true;
  try {
    const img = $("preview-img");
    const url = await invoke("preview", { timeMs: currentMs });
    img.src = url;
    img.hidden = false;
    $("dropzone").hidden = true;
    $("preview-tools").hidden = false;
    $("preview-msg").hidden = true;
    // Wait for layout before measuring the img rect for the edit boxes.
    requestAnimationFrame(() => {
      syncPreviewBox();
      refreshHudBoxes();
    });
  } catch (e) {
    showPreviewMsg(String(e));
  } finally {
    previewBusy = false;
    if (previewWanted) runPreview();
  }
}

function syncPreviewBox() {
  const wrap = $("preview-wrap");
  const img = $("preview-img");
  const center = $("center");
  if (img.hidden || !img.naturalWidth || !img.naturalHeight) {
    wrap.style.maxHeight = "";
    wrap.style.maxWidth = "";
    return;
  }
  const ar = img.naturalWidth / img.naturalHeight;
  const availW = center.clientWidth;
  // Height the column can spare: everything except the other rows.
  let others = 0;
  for (const el of center.children) {
    if (el !== wrap && !el.hidden) others += el.offsetHeight + 8;
  }
  const availH = Math.max(160, center.clientHeight - others);
  const h = Math.min(availH, availW / ar);
  wrap.style.maxHeight = `${Math.ceil(h) + 2}px`;
  wrap.style.maxWidth = `${Math.ceil(h * ar) + 2}px`;
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
  let keepExisting = false;
  try {
    // Rendering twice used to replace the first video without a word.
    const planned = await invoke("planned_output_path");
    if (planned.exists) {
      const name = planned.path.split(/[\\/]/).pop();
      const overwrite = await dialog.ask(
        `${name} already exists in the output folder.\n\nReplace it?`,
        { title: "File already there", kind: "warning", okLabel: "Replace", cancelLabel: "Keep both" }
      );
      keepExisting = !overwrite;
    }
  } catch {
    // Path not resolvable yet (no folder set) — let start_render report it.
  }
  try {
    lastOutPath = await invoke("start_render", { keepExisting });
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
    const clip = status?.clip;
    const what = clip ? `the clip (${fmtTime(clip[1] - clip[0])})` : "the full run";
    let readyText = `Ready to render ${what}`;
    const fps = status?.settings?.last_render_fps || 0;
    if (ready && fps > 1) {
      const runMs = clip
        ? clip[1] - clip[0]
        : status.replay.failed
          ? status.replay.fail_time_ms
          : status.replay.length_ms;
      const frames = (runMs / 1000) * (status.settings.fps || 60);
      const est = frames / fps + (clip ? 0 : status.settings.results_secs || 0);
      readyText = `Ready to render ${what} (~${fmtTime(est * 1000)} at last speed)`;
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
  if (st.build) $("app-ver").title = `build ${st.build}`;
  renderReplayCard();
  renderGhostCard();
  renderBackgroundCard();
  renderPresetsCard();
  renderMapCard();
  renderConfigCard();
  renderGameCard();
  renderRecent();
  renderHudTab();
  renderOutputTab();
  renderClipRow();
  updateRenderButton();
  $("btn-hud-undo").disabled = !st.can_undo;
  $("btn-hud-redo").disabled = !st.can_redo;

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
  $("clip-row").hidden = !hasPair;
  $("btn-analyze").hidden = !hasPair;
  if (hasPair && (replayChanged || mapChanged || !hadPair)) {
    currentMs = Math.min(15000, (st.replay.length_ms || 0) / 2);
    timelineData = await invoke("timeline", { samples: 600 }).catch(() => null);
    $("scrub-len").textContent = fmtTime(timelineData?.length_ms || 0);
    // The clip suggestions come from the timeline — refresh them now
    // that it exists (the render pass above ran before this fetch).
    renderClipRow();
    $("scrub-time").textContent = fmtTime(currentMs);
    drawScrubber();
    schedulePreview();
  } else if (!hasPair) {
    timelineData = null;
    $("preview-img").hidden = true;
    $("preview-tools").hidden = true;
    $("dropzone").hidden = false;
    // Boxes from the previous pair would float interactively over the
    // dropzone otherwise (the no-pair path clears the layer).
    refreshHudBoxes();
  }
}

// ------------------------------------------------------------ file loading

function loadNote(text) {
  // The render bar is always visible — use it as the load status line
  // (unless a render is writing progress there).
  if (!rendering) $("render-text").textContent = text;
}

// A replay dropped straight onto the Ghost race card. Without a main
// replay there is nothing to race yet — it becomes YOUR replay, with a
// hint (a lone replay renders as a normal video).
async function dropGhost(path) {
  const name = path.split(/[\\/]/).pop();
  if (!status?.replay) {
    await loadPath(path);
    // loadPath swallows its own errors — only claim success if the
    // replay actually landed.
    if (status?.replay) {
      loadNote("Loaded as your replay — a ghost race needs a second one. Drop it on Ghost race, otherwise this renders as a normal video.");
    }
    return;
  }
  try {
    await call(() => invoke("load_ghost", { path }));
    loadNote(`Ghost replay loaded: ${name}`);
    schedulePreview();
  } catch (e) {
    loadNote(String(e));
  }
}

async function loadPath(path) {
  const name = path.split(/[\\/]/).pop();
  const lower = path.toLowerCase();
  try {
    if (lower.endsWith(".rhr")) {
      await call(() => invoke("load_replay", { path }));
      loadNote(`Loaded replay: ${name}`);
    } else if (lower.endsWith(".sspm") || lower.endsWith(".rhm")) {
      await call(() => invoke("load_map", { path }));
      loadNote(`Loaded map: ${name}`);
    } else if (lower.endsWith(".json")) {
      // Ambiguous: a cached map is JSON, and so is the game's own
      // config.json. Try the map first, fall back to reading it as a skin.
      try {
        await call(() => invoke("load_map", { path }));
        loadNote(`Loaded map: ${name}`);
      } catch (mapErr) {
        try {
          await call(() => invoke("load_config", { path }));
          loadNote(`Loaded skin config: ${name}`);
          schedulePreview();
        } catch {
          throw mapErr;
        }
      }
    } else if (lower.endsWith(".rhs")) {
      await call(() => invoke("load_config", { path }));
      loadNote(`Loaded skin: ${name}`);
      schedulePreview();
    } else {
      // Anything else might be a background image/video — the backend
      // classifies by content and rejects what neither the image decoder
      // nor ffmpeg can read.
      try {
        await call(() => invoke("set_background", { path }));
        loadNote(`Background set: ${name}`);
        schedulePreview();
      } catch {
        loadNote(`Unsupported file type: ${name}`);
        showPreviewMsg(`Unsupported file type: ${name}`);
      }
    }
  } catch (e) {
    // Surface the reason where it is always visible; don't let one bad
    // file abort the rest of a multi-file drop.
    loadNote(`Could not load ${name}: ${e}`);
  }
}

function initDragDrop() {
  // Drop targets: hovering a dragged file over these cards lights them up
  // and routes the drop there instead of the default middle-drop path.
  const dropTargets = () => [
    { el: document.getElementById("card-ghost"), kind: "ghost" },
    { el: document.getElementById("card-background"), kind: "background" },
  ];
  let hoverTarget = null;
  const clearHover = () => {
    document.querySelectorAll(".card.drop-target").forEach((c) => c.classList.remove("drop-target"));
    hoverTarget = null;
  };
  const hitTarget = (pos) => {
    if (!pos) return null;
    const scale = window.devicePixelRatio || 1;
    // elementFromPoint respects the sources rail's scroll clipping — a
    // card scrolled out of view must not catch drops through whatever
    // covers its unclipped rect.
    const el = document.elementFromPoint(pos.x / scale, pos.y / scale);
    const card = el?.closest?.("#card-ghost, #card-background");
    if (!card) return null;
    return dropTargets().find((t) => t.el === card) || null;
  };
  const trackHover = (e) => {
    const t = hitTarget(e.payload?.position);
    if (t?.kind !== hoverTarget?.kind) {
      clearHover();
      if (t) {
        t.el.classList.add("drop-target");
        hoverTarget = t;
      }
    }
    // The full-screen overlay would cover the card highlight.
    $("drop-overlay").hidden = !!t;
  };
  listen("tauri://drag-enter", trackHover);
  listen("tauri://drag-over", trackHover);
  listen("tauri://drag-leave", () => { $("drop-overlay").hidden = true; clearHover(); });
  listen("tauri://drag-drop", async (e) => {
    $("drop-overlay").hidden = true;
    const target = hitTarget(e.payload?.position);
    clearHover();
    // Replays first: loading a replay may swap the auto-resolved map, so a
    // map dropped in the same gesture must land after it.
    const rank = (p) => (p.toLowerCase().endsWith(".rhr") ? 0 : 1);
    let paths = [...(e.payload.paths || [])].sort((a, b) => rank(a) - rank(b));
    if (target?.kind === "ghost") {
      // ALL replays in the gesture go through the ghost route: two runs
      // dropped together become main + ghost instead of the second one
      // silently replacing the first.
      const rhrs = paths.filter((p) => p.toLowerCase().endsWith(".rhr"));
      for (const p of rhrs) await dropGhost(p);
      paths = paths.filter((p) => !rhrs.includes(p));
    } else if (target?.kind === "background" && paths.length) {
      const p = paths[0];
      try {
        await call(() => invoke("set_background", { path: p }));
        loadNote(`Background set: ${p.split(/[\\/]/).pop()}`);
        schedulePreview();
        paths = paths.slice(1);
      } catch {
        loadNote(`Unsupported background: ${p.split(/[\\/]/).pop()}`);
        paths = paths.slice(1);
      }
    }
    for (const p of paths) await loadPath(p);
  });
  // Second app instance (e.g. double-clicked .rhr) forwards its file here.
  listen("open-replay", (e) => loadPath(e.payload));
}

// ------------------------------------------------------------ wiring

function initControls() {
  const pickReplay = async () => {
    const p = await dialog.open({ filters: [{ name: "Rhythia replay", extensions: ["rhr"] }] });
    if (p) await loadPath(p);
  };
  $("btn-replay").addEventListener("click", pickReplay);
  // The empty-state panel invited a drop but did nothing on a click, which
  // is the first thing anyone tries.
  $("btn-drop-open").addEventListener("click", pickReplay);
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
    const p = await dialog.open({
      filters: [{ name: "Skin config", extensions: ["rhs", "json"] }],
    });
    if (p) await loadPath(p);
  });
  $("btn-config-clear").addEventListener("click", () => call(() => invoke("clear_config")).then(schedulePreview));
  $("btn-ghost").addEventListener("click", async () => {
    const p = await dialog.open({ filters: [{ name: "Rhythia replay", extensions: ["rhr"] }] });
    if (!p) return;
    try {
      await call(() => invoke("load_ghost", { path: p }));
      loadNote("Ghost replay loaded.");
      schedulePreview();
    } catch (e) { loadNote(String(e)); }
  });
  $("btn-ghost-clear").addEventListener("click", () => call(() => invoke("clear_ghost")).then(schedulePreview));
  $("btn-bg").addEventListener("click", async () => {
    const p = await dialog.open({
      filters: [
        { name: "Image or video", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "gif", "mp4", "webm", "mkv", "mov", "avi", "m4v", "wmv", "flv", "ts"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
    if (!p) return;
    try {
      await call(() => invoke("set_background", { path: p }));
      loadNote("Background set.");
      schedulePreview();
    } catch (e) { loadNote(String(e)); }
  });
  $("btn-bg-clear").addEventListener("click", () =>
    call(() => invoke("set_background", { path: null })).then(schedulePreview));

  // Layout presets.
  $("btn-preset-save").addEventListener("click", async () => {
    const name = $("preset-name").value.trim();
    try {
      await call(() => invoke("save_preset", { name }));
      $("preset-name").value = "";
      $("preset-name").blur();
      renderPresetsCard();
      loadNote(`Preset saved: ${name}`);
    } catch (e) {
      loadNote(String(e));
    }
  });
  $("preset-list").addEventListener("click", async (e) => {
    const li = e.target.closest("li[data-preset]");
    if (!li) return;
    const name = li.dataset.preset;
    try {
      if (e.target.closest(".del")) {
        const ok = await dialog.ask(`Delete the preset "${name}"?`, {
          title: "Delete preset",
          kind: "warning",
          okLabel: "Delete",
          cancelLabel: "Keep",
        });
        if (!ok) return;
        await call(() => invoke("delete_preset", { name }));
        renderPresetsCard();
        loadNote(`Preset deleted: ${name}`);
        return;
      }
      await call(() => invoke("apply_preset", { name }));
      loadNote(`Preset applied: ${name}`);
      schedulePreview();
      refreshHudBoxes();
    } catch (err) {
      loadNote(String(err));
    }
  });

  // Clip range.
  $("btn-clip-in").addEventListener("click", () => setClipEdge(true));
  $("btn-clip-out").addEventListener("click", () => setClipEdge(false));
  $("btn-clip-clear").addEventListener("click", async () => {
    await call(() => invoke("clear_clip"));
    renderClipRow();
  });

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
  $("btn-game-exe").addEventListener("click", async () => {
    // No extension filter: the native Linux build has no .exe suffix.
    const p = await dialog.open({ title: "Select the Rhythia executable" });
    if (p) await applyGameAssets(p);
  });
  $("btn-game-detect").addEventListener("click", async () => {
    setGameNote("Searching your Steam libraries…");
    const exe = await invoke("detect_game").catch(() => null);
    if (!exe) {
      setGameNote(null);
      renderGameCard("Not found in any Steam library — click Locate… and pick the game's executable.");
      return;
    }
    await applyGameAssets(exe);
  });

  $("btn-hud-config-reset").addEventListener("click", async () => {
    // This throws away every dragged position and size, which is often the
    // result of a long session with the editor.
    const ok = await dialog.ask(
      "Put every HUD element back to the loaded config?\n\nDragged positions and sizes are lost.",
      { title: "Reset HUD", kind: "warning", okLabel: "Reset", cancelLabel: "Cancel" }
    );
    if (!ok) return;
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

  // The thumbnail button opens a platform-format menu; picking an entry
  // renders the card in that size.
  $("btn-frame").addEventListener("click", () => {
    $("thumb-menu").hidden = !$("thumb-menu").hidden;
  });
  document.addEventListener("click", (e) => {
    if (!$("thumb-wrap").contains(e.target)) $("thumb-menu").hidden = true;
  });
  $("thumb-menu").addEventListener("click", async (e) => {
    const item = e.target.closest("button[data-w]");
    if (!item) return;
    $("thumb-menu").hidden = true;
    const w = Number(item.dataset.w);
    const h = Number(item.dataset.h);
    const raw = status?.replay
      ? `${status.replay.player} - ${status?.map?.song_name || "run"} - card`
      : "score-card";
    const base = raw.replace(/[\\/:*?"<>|]/g, "-");
    const p = await dialog.save({
      defaultPath: `${base}.png`,
      filters: [{ name: "PNG image", extensions: ["png"] }],
    });
    if (!p) return;
    $("btn-frame").disabled = true;
    try {
      await invoke("export_card", { path: p, width: w, height: h });
      $("render-text").textContent = `Score card saved — ${p}`;
    } catch (e2) {
      showPreviewMsg(String(e2));
    } finally {
      $("btn-frame").disabled = false;
    }
  });
}

/// ffmpeg could not be executed at the last probe.
let ffmpegMissing = false;

/// Keeps the render button honest about whether a render can even start.
function updateRenderReady() {
  const btn = $("btn-render");
  if (!btn) return;
  btn.disabled = ffmpegMissing;
  btn.title = ffmpegMissing
    ? "ffmpeg could not be run — set its path under Advanced, or install it"
    : "";
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
    // No ffmpeg means no render at all. Say it here, not after the user has
    // sat through one — and point at the setting that fixes it.
    ffmpegMissing = !!probe.ffmpeg_missing;
    if (ffmpegMissing) {
      $("topbar-info").innerHTML =
        `<span class="chip bad" title="${esc(
          `Tried to run: ${probe.ffmpeg}\n\nSet the path under Advanced, or install ffmpeg.`
        )}">ffmpeg not found — rendering unavailable</span>`;
    } else {
      $("topbar-info").textContent = hw.length
        ? `Hardware encoder: ${hw.map((e) => labels[e]?.split(" ")[0] || e).join(", ")}`
        : "Software encoding (x264)";
    }
    updateRenderReady();
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
    // Package installs can't replace themselves. An AUR install gets the
    // honest hint (the update arrives through the AUR helper); deb/rpm
    // get pointed at the release downloads.
    const channel = await invoke("update_channel");
    if (channel !== "self") {
      if (channel === "aur") {
        $("update-text").textContent =
          `Update ${update.version} is available — update via your AUR helper (rhythr-bin).`;
        $("btn-update").textContent = "Release notes";
      } else {
        $("btn-update").textContent = "Open download page";
      }
      $("btn-update").onclick = () => invoke("open_releases_page");
      return;
    }
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
  window.__TAURI__.app.getVersion().then((v) => { $("app-ver").textContent = `v${v}`; });
  $("preview-img").draggable = false;
  // The rAF below the src swap can race the decode; the load event is the
  // reliable moment to fit the box around the fresh frame.
  $("preview-img").addEventListener("load", () => {
    syncPreviewBox();
    requestAnimationFrame(() => refreshHudBoxes());
  });
  new ResizeObserver(() => {
    syncPreviewBox();
    requestAnimationFrame(() => refreshHudBoxes());
  }).observe($("center"));
  initControls();
  initScrubber();
  initHudEdit();
  initDragDrop();
  initRenderEvents();
  const st = await invoke("get_status");
  await applyStatus(st);
  initEncoders();
  setTimeout(initUpdater, 2500);
  autoConnectGame();
});
