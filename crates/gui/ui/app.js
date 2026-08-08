// rhythr desktop UI logic. Talks to the Rust backend via Tauri
// commands; all state lives in the backend, the UI re-renders from the
// StatusDto it returns.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const dialog = window.__TAURI__.dialog;
const opener = window.__TAURI__.opener;

const $ = (id) => document.getElementById(id);

// The quality slider's stops, as resolved by the backend. Recomputing the
// mapping here would be a second copy of crates/render/src/quality.rs to keep
// in step, and the first time they drifted the number on screen would stop
// describing the encode that actually runs.
let qualitySteps = [];

let status = null;          // last StatusDto from the backend
let timelineData = null;    // health graph + miss ticks
let currentMs = 0;          // scrubber position
let previewTimer = null;
let previewBusy = false;
let previewWanted = false;
let lastOutPath = null;
let rendering = false;
/// ffmpeg could not be executed at the last probe — nothing will encode.
let ffmpegMissing = false;
let autoDownloadTried = 0;  // map id of the last automatic download attempt

// ------------------------------------------------------------ formatting

function fmtTime(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/// mm:ss.ms — the clip fields need the milliseconds fmtTime rounds away.
function fmtClock(ms) {
  const t = Math.max(0, ms);
  const min = Math.floor(t / 60000);
  return `${min}:${((t - min * 60000) / 1000).toFixed(3).padStart(6, "0")}`;
}

/// "1:23.456", "1:23", "83.4" or "83" → ms; null when it reads as nothing.
function parseClock(text) {
  const m = String(text).trim().match(/^(?:(\d+):)?(\d+(?:[.,]\d+)?)$/);
  if (!m) return null;
  return (Number(m[1] || 0) * 60 + Number(m[2].replace(",", "."))) * 1000;
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
          "\n\nMost often this means the loaded map is not the one that was played — " +
          "try the chart this replay came from."
      )}">map may not match</span>`;
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
  // Downloading needs an online id. Without one the button could only ever
  // fail with "replay has no online map id", so it is not offered — and the
  // replay's readable map name is shown instead, which is what someone needs
  // to find the file themselves. It was parsed and carried all along and
  // displayed nowhere.
  const canDownload = !!(r && !m && r.map_id > 0);
  $("btn-map-dl").hidden = !canDownload;
  if (!m) {
    if (!r) {
      body.innerHTML = `<p class="hint">Auto-resolved from the replay</p>`;
    } else if (canDownload) {
      body.innerHTML = `<p class="hint">Map id <b>${r.map_id}</b>. Download it from rhythia.com, or browse for a local .sspm/.rhm</p>`;
    } else {
      const named = r.legacy_map_id
        ? `<b>${esc(r.legacy_map_id)}</b>`
        : "this replay's map";
      body.innerHTML =
        `<p class="hint">This replay carries no online map id, so it cannot be downloaded. Browse for ${named} as a local .sspm/.rhm file.</p>`;
    }
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

// ---------------------------------------------------- collapsible sources

const CARD_STORE = "rhythr.cards.collapsed";

function collapsedCards() {
  try {
    const v = JSON.parse(localStorage.getItem(CARD_STORE) || "[]");
    return new Set(Array.isArray(v) ? v.filter((x) => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function initOptionalCards() {
  const stored = collapsedCards();
  const first = localStorage.getItem(CARD_STORE) === null;
  for (const card of document.querySelectorAll(".card.optional")) {
    // First run: fold the optional sources so the rail opens compact.
    if (first || stored.has(card.id)) card.classList.add("collapsed");
    const title = card.querySelector(".card-head h2");
    if (!title) continue;
    title.tabIndex = 0;
    title.setAttribute("role", "button");
    const toggle = () => {
      card.classList.toggle("collapsed");
      const now = collapsedCards();
      card.classList.contains("collapsed") ? now.add(card.id) : now.delete(card.id);
      try {
        localStorage.setItem(CARD_STORE, JSON.stringify([...now]));
      } catch { /* nothing we can do, and nothing worth saying */ }
    };
    title.addEventListener("click", toggle);
    title.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        toggle();
      }
    });
  }
  syncOptionalCards();
}

/// A card that now holds something real opens itself once — a loaded ghost
/// or background should never be hidden behind a fold the user forgot.
///
/// The Game card is the exception and runs the other way round. It used to
/// open on SUCCESS, which is backwards: a connected game needs no attention,
/// while a failed detect leaves the app rendering with approximated skins and
/// no hit sounds. The message saying so was written into a folded card, and
/// since only a success ever wrote the fold state, a machine where detection
/// keeps failing re-folded it on every single launch. So: open it while the
/// game is NOT connected.
let lastCardFilled = {};
function syncOptionalCards() {
  const filled = {
    "card-ghost": !!status?.ghost,
    "card-background": !!status?.settings?.background,
    "card-game": status ? !status.game_ok : false,
  };
  for (const [id, has] of Object.entries(filled)) {
    const card = $(id);
    // Auto-open only on the EDGE from empty to filled — the moment something
    // arrives — not on every status refresh. Refreshing on every one meant a
    // card the user deliberately folded (a loaded ghost they are done with,
    // or a game that stays unconnected) sprang back open each time, so it
    // could never be kept shut.
    const justFilled = has && !lastCardFilled[id];
    if (justFilled && card && card.classList.contains("collapsed")) {
      card.classList.remove("collapsed");
      const now = collapsedCards();
      now.delete(id);
      try {
        localStorage.setItem(CARD_STORE, JSON.stringify([...now]));
      } catch { /* ignore */ }
    }
  }
  lastCardFilled = filled;
}

/// Trims a path from the middle, keeping the root and the file name — the
/// two ends that tell you where something is. CSS ellipsis alone can only
/// cut one end, and the rtl trick that cuts the front visibly moves the
/// leading slash to the back.
function shortenPath(path, max = 46) {
  if (path.length <= max) return path;
  const sep = path.includes("\\") ? "\\" : "/";
  const parts = path.split(sep);
  const name = parts.pop() || "";
  const root = parts.slice(0, 3).join(sep);
  const short = `${root}${sep}…${sep}${name}`;
  return short.length < path.length ? short : `…${sep}${name}`;
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
    <div class="src-meta src-path" title="${esc(path)}">${esc(shortenPath(path))}</div>`;
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
  // The rows are focusable but carry no role="button": that would turn the ✕
  // inside them into a presentational child and hide deleting from a reader.
  $("preset-list").innerHTML = names.length
    ? names
        .map(
          (n) =>
            `<li data-preset="${esc(n)}" tabindex="0" title="Apply this preset"><span class="name">${esc(n)}</span><button class="del" title="Delete preset">✕</button></li>`,
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
  // The bounds are in the fields; the label keeps the one thing they do
  // not say, which is how long the clip actually is.
  $("clip-label").textContent = clip ? fmtTime(clip[1] - clip[0]) : "";
  writeClipFields(false);
  renderClipSuggestions();
  drawScrubber();
}

/// The clip bounds are editable, so a refresh must not overwrite what is
/// being typed — except right after a commit, when the value that actually
/// applies has to land in the field the user still has focus in.
function writeClipFields(force) {
  const clip = status?.clip;
  const put = (id, ms, whole) => {
    const el = $(id);
    if (!el || (!force && document.activeElement === el)) return;
    el.value = ms == null ? "" : fmtClock(ms);
    el.placeholder = fmtClock(whole);
  };
  put("clip-in", clip ? clip[0] : null, 0);
  put("clip-out", clip ? clip[1] : null, timelineData?.length_ms || 0);
}

/// One nudge: a frame at the rate this will render at, a second with Shift.
/// A clip that cannot be moved by single frames cannot be cut on the beat.
function clipStep(big) {
  return big ? 1000 : 1000 / (status?.settings?.fps || 60);
}

/// Moves one edge of the clip and leaves the other where it is, creating
/// the clip from the full run if there was none.
async function applyClipBound(isIn, ms) {
  const len = timelineData?.length_ms || 0;
  const cur = status?.clip;
  let s = cur ? cur[0] : 0;
  let e = cur ? cur[1] : len;
  // The backend rejects anything under half a second: hold the edited edge
  // there instead of bouncing an error off the user.
  if (isIn) s = Math.min(Math.max(0, ms), e - 500);
  else e = Math.max(Math.min(len, ms), s + 500);
  try {
    await call(() => invoke("set_clip", { startMs: s, endMs: e }));
  } catch (err) {
    loadNote(String(err));
  }
  writeClipFields(true);
  // Frame-exact only means something if you can see the frame.
  seekTo(isIn ? s : e);
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
    .map((p) => `<li data-path="${esc(p)}" role="button" tabindex="0" title="${esc(p)}">${esc(p.split(/[\\/]/).pop())}</li>`)
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
  // A field being typed into owns its value until the change lands.
  const put = (id, v) => {
    const el = $(id);
    if (document.activeElement !== el) el.value = v;
  };
  // Resolution and frame rate fall back to their custom fields for anything
  // the lists don't hold; picking "Custom…" keeps them open even though the
  // stored value is still one of the presets.
  const res = `${s.width}x${s.height}`;
  const resSel = $("set-res");
  const resCustom =
    resSel.value === "custom" || ![...resSel.options].some((o) => o.value === res);
  resSel.value = resCustom ? "custom" : res;
  $("res-custom").hidden = !resCustom;
  put("set-res-w", String(s.width));
  put("set-res-h", String(s.height));
  const fpsSel = $("set-fps");
  const fpsCustom =
    fpsSel.value === "custom" || ![...fpsSel.options].some((o) => o.value === String(s.fps));
  fpsSel.value = fpsCustom ? "custom" : String(s.fps);
  $("set-fps-custom").hidden = !fpsCustom;
  put("set-fps-custom", String(s.fps));
  const presetSel = $("set-preset");
  if (![...presetSel.options].some((o) => o.value === s.preset)) {
    const opt = document.createElement("option");
    opt.value = s.preset;
    opt.textContent = s.preset;
    presetSel.appendChild(opt);
  }
  presetSel.value = s.preset;
  $("set-quality").value = String(s.quality);
  paintQuality();
  $("set-tcpfeed").checked = !!s.tcp_feed;
  if (s.transport_note) $("bench-note").textContent = s.transport_note;
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
  // Survives the card being folded, which is where the warning used to go
  // and stay.
  const headChip = $("game-head-chip");
  if (headChip) headChip.hidden = ok !== false;
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

function seekTo(ms) {
  currentMs = Math.min(timelineData?.length_ms || 0, Math.max(0, ms));
  $("scrub-time").textContent = fmtTime(currentMs);
  drawScrubber();
  schedulePreview();
}

function scrubTo(clientX) {
  const canvas = $("scrubber");
  const rect = canvas.getBoundingClientRect();
  const frac = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
  seekTo(frac * (timelineData?.length_ms || 0));
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

/// The history is global — Ctrl+Z works anywhere and the HUD tab pushes
/// steps of its own — so the buttons follow can_undo/can_redo rather than
/// the editor, which would hide them in the very moment something became
/// undoable. The editor keeps them in place while it is open.
function syncHistoryButtons() {
  const undo = $("btn-hud-undo");
  const redo = $("btn-hud-redo");
  undo.hidden = !(hudEditOn || status?.can_undo);
  redo.hidden = !(hudEditOn || status?.can_redo);
  undo.disabled = !status?.can_undo;
  redo.disabled = !status?.can_redo;
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
    syncHistoryButtons();
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

// A render runs for minutes and people alt-tab away, so the end of one has
// to be visible from outside the window. The title is the one channel a
// webview always owns; it goes back to normal the moment the window is
// looked at again. No focus stealing, no dialog.
const BASE_TITLE = document.title;
let titleNotice = false;
let notifyAsked = false;

function setWindowNotice(text) {
  if (document.hasFocus()) return;
  titleNotice = true;
  document.title = `${text} — ${BASE_TITLE}`;
  // A real notification on top, where the webview has them and the user has
  // already allowed them. Optional by design: it must never throw.
  try {
    if (window.Notification && Notification.permission === "granted") {
      new Notification(BASE_TITLE, { body: text });
    }
  } catch { /* no notifications in this webview — the title said it */ }
}

function clearWindowNotice() {
  if (!titleNotice) return;
  titleNotice = false;
  document.title = BASE_TITLE;
}

/// Asked when a render starts, never at launch: that is the one moment the
/// question makes sense, and the window still has focus to answer it in.
function askNotifyPermission() {
  if (notifyAsked) return;
  notifyAsked = true;
  try {
    if (window.Notification && Notification.permission === "default") {
      const p = Notification.requestPermission();
      if (p && typeof p.catch === "function") p.catch(() => {});
    }
  } catch { /* nothing to ask */ }
}

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
      // If the dialog itself fails (a missing permission once cost us this
      // whole prompt), the safe default is to KEEP the existing file, never
      // to overwrite it silently — losing a video is worse than a stray
      // "name (2).mp4".
      let overwrite = false;
      try {
        overwrite = await dialog.ask(
          `${name} already exists in the output folder.\n\nReplace it?`,
          { title: "File already there", kind: "warning", okLabel: "Replace", cancelLabel: "Keep both" }
        );
      } catch (e) {
        console.error("overwrite dialog failed, keeping the existing file", e);
      }
      keepExisting = !overwrite;
    }
  } catch {
    // Path not resolvable yet (no folder set) — let start_render report it.
  }
  try {
    lastOutPath = await invoke("start_render", { keepExisting });
    setRenderingUi(true);
    askNotifyPermission();
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
    $("render-text").classList.remove("done");
    $("render-text").textContent =
      `${pct.toFixed(0)}% — frame ${done.toLocaleString()} / ${total.toLocaleString()}` +
      ` · ${fps.toFixed(0)} fps · ETA ${fmtTime(eta_secs * 1000)}`;
  });
  listen("render-done", (e) => {
    setRenderingUi(false);
    updateRenderButton();
    lastOutPath = e.payload.path;
    // The timing stays on screen after the render rather than scrolling past
    // during it: "why was that one slower than the last" is a question you
    // only think to ask once it is over.
    $("render-text").classList.add("done");
    // Clear any error tooltip left from a previous failed render; a Done line
    // carrying the last error's hover text is worse than none.
    $("render-text").title = "";
    // Everything that could explain the speed, on screen: with no console on
    // Windows, two finished renders side by side is the only way to see what
    // differed between them.
    $("render-text").textContent =
      `Done — ${e.payload.path}\n${e.payload.timing}\n${e.payload.detail}`;
    $("btn-open-out").hidden = false;
    setWindowNotice("Render finished");
  });
  listen("render-cancelled", () => {
    setRenderingUi(false);
    updateRenderButton();
    $("render-text").classList.remove("done");
    $("render-text").title = "";
    $("render-text").textContent = "Cancelled.";
  });
  listen("render-error", (e) => {
    setRenderingUi(false);
    updateRenderButton();
    // Full text, wrapped, selectable, and in the tooltip as well: ffmpeg
    // puts its reason at the END of the message, which is exactly what a
    // single ellipsised line threw away.
    $("render-text").classList.add("done");
    $("render-text").textContent = `Error: ${e.payload}`;
    $("render-text").title = e.payload;
    setWindowNotice("Render failed");
  });
}

/// The results screen is only appended when the clip reaches the end of the
/// run (video.rs: `end_ms >= run_end - 500`). Nothing said so, and two of the
/// four clips the app itself suggests stop mid-run — so the app offered a
/// chip that quietly turned off a setting the app defaults to on.
function updateResultsAvailability() {
  const sel = $("set-results");
  const note = $("results-note");
  if (!sel || !note) return;
  const clip = status?.clip;
  const r = status?.replay;
  if (!clip || !r) {
    sel.disabled = false;
    note.hidden = true;
    return;
  }
  const runEnd = r.failed ? r.fail_time_ms : r.length_ms;
  const reachesEnd = clip[1] >= runEnd - 500;
  sel.disabled = !reachesEnd;
  note.hidden = reachesEnd;
  if (!reachesEnd) {
    note.textContent =
      "Your clip stops before the end of the run, so there is no results screen to show. Extend the clip to the end to get one.";
  }
}

function updateRenderButton() {
  const ready = !!(status?.replay && status?.map);
  const btn = $("btn-render");
  btn.disabled = !ready || rendering || ffmpegMissing;
  btn.title = ffmpegMissing
    ? "ffmpeg could not be run — set its path under Advanced, or install it"
    : "";
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
      // runMs is SONG time; a speed mod compresses the video to runMs/speed
      // of wall clock, so it produces that many fewer output frames. Ignoring
      // speed over-estimated the time by exactly the speed factor.
      const speed = status.replay.speed || 1;
      const frames = (runMs / speed / 1000) * (status.settings.fps || 60);
      const est = frames / fps + (clip ? 0 : status.settings.results_secs || 0);
      readyText = `Ready to render ${what} (~${fmtTime(est * 1000)} at last speed)`;
    }
    // ffmpeg is checked here too: `ready` is only replay+map, while the
    // button is also disabled without ffmpeg, so this line used to promise a
    // render next to a button that could not start one.
    // A Ready line must not keep an old error's hover text.
    $("render-text").title = "";
    $("render-text").textContent = ffmpegMissing
      ? "ffmpeg could not be run. Set its path under Advanced, or install it."
      : ready
        ? readyText
        : status?.replay
          ? "No map yet. Download it, or browse for the file."
          : "Load a replay to render";
    // Which file this will produce was invisible until the render finished.
    // Resolved by the backend so it matches exactly what gets written.
    if (ready) showTargetFile();
    else setTargetFile("");
    updateResultsAvailability();
  }
}

/// Shows the file a render would write, next to the ready text.
function setTargetFile(path) {
  const el = $("render-target");
  if (!el) return;
  el.textContent = path ? path.split(/[\\/]/).pop() : "";
  el.title = path || "";
  el.hidden = !path;
}

let targetSeq = 0;
async function showTargetFile() {
  const mine = ++targetSeq;
  try {
    const planned = await invoke("planned_output_path");
    if (mine === targetSeq) setTargetFile(planned.path);
  } catch {
    if (mine === targetSeq) setTargetFile("");
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
  syncOptionalCards();
  renderHudTab();
  renderOutputTab();
  renderClipRow();
  updateRenderButton();
  syncHistoryButtons();

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
          // The map attempt already painted its failure over the preview;
          // clear it, or a successful skin load still reads as an error.
          $("preview-msg").hidden = true;
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
    // Deleting a preset asks; saving over one did not, and destroys exactly
    // as much.
    // hasOwnProperty, not `in`: `in` walks the prototype chain, so a preset
    // named "toString" or "constructor" would falsely read as existing and
    // pop a replace prompt for a preset that is not there.
    const existing = Object.prototype.hasOwnProperty.call(
      status?.settings?.presets || {},
      name,
    );
    if (existing) {
      const ok = await dialog.ask(`Replace the preset "${name}" with the current layout?`, {
        title: "Replace preset",
        kind: "warning",
        okLabel: "Replace",
        cancelLabel: "Cancel",
      });
      if (!ok) return;
    }
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
  $("preset-list").addEventListener("keydown", (e) => {
    if (e.key !== " " && e.key !== "Enter") return;
    // The ✕ is a real button and already answers Enter and Space itself.
    const li = e.target.closest("li[data-preset]");
    if (!li || e.target.closest(".del")) return;
    e.preventDefault();
    li.click();
  });

  // Clip range.
  $("btn-clip-in").addEventListener("click", () => setClipEdge(true));
  $("btn-clip-out").addEventListener("click", () => setClipEdge(false));
  const wireClipField = (id, isIn) => {
    const el = $(id);
    el.addEventListener("keydown", (e) => {
      const dir = e.key === "ArrowUp" ? 1 : e.key === "ArrowDown" ? -1 : 0;
      if (!dir) return;
      e.preventDefault();
      const base = parseClock(el.value) ?? (isIn ? 0 : timelineData?.length_ms || 0);
      const next = base + dir * clipStep(e.shiftKey);
      // Written before the round trip, so a held arrow key keeps counting
      // from where it already is instead of from a stale field.
      el.value = fmtClock(next);
      applyClipBound(isIn, next);
    });
    el.addEventListener("change", () => {
      const ms = parseClock(el.value);
      // Unreadable: put the bound that actually applies back in the field.
      if (ms == null) {
        writeClipFields(true);
        return;
      }
      // Blurring after a nudge fires this with the value the nudge already
      // set; re-sending it would seek the preview a second time.
      const cur = status?.clip;
      const now = cur ? (isIn ? cur[0] : cur[1]) : null;
      if (now != null && Math.abs(now - ms) < 0.5) return;
      applyClipBound(isIn, ms);
    });
  };
  wireClipField("clip-in", true);
  wireClipField("clip-out", false);
  $("btn-clip-clear").addEventListener("click", async () => {
    await call(() => invoke("clear_clip"));
    renderClipRow();
  });

  $("recent-list").addEventListener("click", (e) => {
    const li = e.target.closest("li[data-path]");
    if (li) loadPath(li.dataset.path);
  });
  $("recent-list").addEventListener("keydown", (e) => {
    if (e.key !== " " && e.key !== "Enter") return;
    const li = e.target.closest("li[data-path]");
    if (!li) return;
    e.preventDefault();
    li.click();
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
    if ($("set-res").value === "custom") {
      // Nothing changes until a number is entered — just open the fields.
      $("res-custom").hidden = false;
      $("set-res-w").focus();
      $("set-res-w").select();
      return;
    }
    const [w, h] = $("set-res").value.split("x").map(Number);
    pushOutput({ width: w, height: h });
  });
  // h.264 in yuv420p cannot encode an odd side: round here rather than let
  // ffmpeg refuse the job minutes in. The bounds are the backend's own.
  const fitEven = (v, lo, hi) => Math.min(hi, Math.max(lo, Math.round(v / 2) * 2));
  const pushRes = () => {
    const w = Number($("set-res-w").value.trim());
    const h = Number($("set-res-h").value.trim());
    if (!Number.isFinite(w) || !Number.isFinite(h) || w < 1 || h < 1) {
      $("set-res-w").value = String(status?.settings?.width ?? 1920);
      $("set-res-h").value = String(status?.settings?.height ?? 1080);
      return;
    }
    const fw = fitEven(w, 320, 7680);
    const fh = fitEven(h, 240, 4320);
    $("set-res-w").value = String(fw);
    $("set-res-h").value = String(fh);
    pushOutput({ width: fw, height: fh });
  };
  $("set-res-w").addEventListener("change", pushRes);
  $("set-res-h").addEventListener("change", pushRes);
  $("set-fps").addEventListener("change", () => {
    if ($("set-fps").value === "custom") {
      $("set-fps-custom").hidden = false;
      $("set-fps-custom").focus();
      $("set-fps-custom").select();
      return;
    }
    pushOutput({ fps: Number($("set-fps").value) });
  });
  $("set-fps-custom").addEventListener("change", () => {
    const v = Math.round(Number($("set-fps-custom").value.trim()));
    if (!Number.isFinite(v) || v < 1) {
      $("set-fps-custom").value = String(status?.settings?.fps ?? 60);
      return;
    }
    const fps = Math.min(240, Math.max(24, v));
    $("set-fps-custom").value = String(fps);
    pushOutput({ fps });
  });
  $("set-preset").addEventListener("change", () => pushOutput({ preset: $("set-preset").value }));
  $("set-tcpfeed").addEventListener("change", () =>
    pushOutput({ tcp_feed: $("set-tcpfeed").checked }));
  $("btn-bench-transport").addEventListener("click", async () => {
    const btn = $("btn-bench-transport");
    const note = $("bench-note");
    btn.disabled = true;
    const was = btn.textContent;
    btn.textContent = "Measuring…";
    note.textContent = "Pushing frames through each transport. A few seconds.";
    try {
      const line = await invoke("benchmark_transport");
      // The benchmark writes tcp_feed/socket size into settings; pull the
      // status back so the Fast-frame-transport checkbox reflects a run that
      // may have chosen the pipe, instead of still showing its old state.
      applyStatus(await invoke("get_status"));
      note.textContent = line;
    } catch (e) {
      note.textContent = `Could not measure: ${e}`;
    } finally {
      btn.textContent = was;
      btn.disabled = false;
    }
  });
  $("set-quality").addEventListener("input", paintQuality);
  $("set-quality").addEventListener("change", () => pushOutput({ quality: Number($("set-quality").value) }));
  $("set-encoder").addEventListener("change", () => {
    pushOutput({ encoder: $("set-encoder").value });
    paintQuality();
  });
  $("set-results").addEventListener("change", () => pushOutput({ results_secs: Number($("set-results").value) }));
  $("set-mblur").addEventListener("change", () => pushOutput({ motion_blur: Number($("set-mblur").value) }));
  $("set-musicvol").addEventListener("input", () => { $("musicvol-val").textContent = `${$("set-musicvol").value}%`; });
  $("set-musicvol").addEventListener("change", () => pushOutput({ music_volume: Number($("set-musicvol").value) }));
  $("set-hitvol").addEventListener("input", () => { $("hitvol-val").textContent = `${$("set-hitvol").value}%`; });
  $("set-hitvol").addEventListener("change", () => pushOutput({ hitsound_volume: Number($("set-hitvol").value) }));
  $("set-filename").addEventListener("change", () => pushOutput({ file_name: $("set-filename").value }));
  $("set-ffmpeg").addEventListener("change", async () => {
    await pushOutput({ ffmpeg: $("set-ffmpeg").value });
    // Re-probe: now that the gate actually blocks rendering, a corrected
    // path has to lift it without a restart.
    initEncoders();
  });
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

  $("btn-diagnostics").addEventListener("click", async () => {
    const p = await dialog.save({
      defaultPath: "rhythr-diagnostics.txt",
      filters: [{ name: "Text", extensions: ["txt"] }],
    });
    if (!p) return;
    try {
      const written = await invoke("write_diagnostics", { path: p });
      loadNote(`Diagnostics written: ${written}`);
    } catch (e) {
      loadNote(String(e));
    }
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

/// The render button has ONE owner (updateRenderButton); this just asks it
/// to re-evaluate after a probe. Setting `disabled` here directly both
/// ignored whether a replay was loaded and was undone by the next status
/// update.
function updateRenderReady() {
  updateRenderButton();
}

// Higher is better on this slider — that inversion is the whole point of it,
// so the hint has to keep saying what the number costs.
function paintQuality() {
  const el = $("set-quality");
  if (!el) return;
  const q = Number(el.value);
  $("quality-val").textContent = String(q);
  const step = qualitySteps.find((s) => s.q === q);
  const hint = $("quality-hint");
  if (!step || !hint) return;
  const enc = $("set-encoder")?.value || "auto";
  const native =
    enc === "x264"
      ? `CRF ${step.x264}`
      : enc === "auto"
        ? `CRF ${step.x264} / hardware ${step.hardware}`
        : `quantiser ${step.hardware}`;
  hint.textContent = `${step.hint} · ${native}`;
}

async function initEncoders() {
  try {
    const probe = await invoke("probe_encoders");
    qualitySteps = probe.quality_steps || [];
    const list = probe.available;
    const sel = $("set-encoder");
    const labels = {
      auto: "Auto (fastest available)",
      x264: "x264 (software)",
      nvenc: "NVENC (NVIDIA)",
      qsv: "Quick Sync (Intel)",
      // VAAPI covers AMD and Intel on Linux; AMD on Windows is AMF, which is
      // why naming AMD on the VAAPI entry alone used to be misleading.
      vaapi: "VAAPI (AMD/Intel, Linux)",
      amf: "AMF (AMD)",
    };
    sel.innerHTML = list.map((e) => `<option value="${e}">${labels[e] || e}</option>`).join("");
    const saved = status?.settings?.encoder || "auto";
    if (list.includes(saved)) {
      sel.value = saved;
    } else if (list.length) {
      // e.g. settings from another machine — keep backend and UI in agreement.
      sel.value = "auto";
      pushOutput({ encoder: "auto" });
    }
    // Only now: the hint names the value THIS encoder will be given, so
    // drawing it before the dropdown had been set described the wrong one.
    paintQuality();
    // With no list at all (ffmpeg not runnable) the saved choice stands:
    // overwriting it here would lose the user's encoder because of a broken
    // path they are about to fix.
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
        ? `Hardware encoder: ${hw.map((e) => labels[e]?.split(" (")[0] || e).join(", ")}`
        : "Software encoding (x264)";
    }
    updateRenderReady();
    // Say WHY a hardware encoder is missing (e.g. nvenc wants a newer
    // NVIDIA driver) — otherwise "only x264" looks like a bug.
    const note = $("encoder-note");
    const reasons = Object.entries(probe.unavailable || {})
      .filter(([e]) => e !== "vaapi" || hw.length === 0) // vaapi absence on Windows is normal
      .map(([e, r]) => `${labels[e]?.split(" (")[0] || e}: ${r}`);
    if (note) {
      note.textContent = hw.length === 0 && reasons.length ? reasons.join("  ·  ") : "";
      note.hidden = !note.textContent;
    }
  } catch { /* probing is best-effort */ }
}

// ------------------------------------------------------------ UI scale

// The webview's own zoom shortcuts are off, so the size of the interface is
// ours to offer: style.css measures everything against the root font size,
// and --ui-scale is the factor on it.
const SCALE_STORE = "rhythr.ui.scale";
const SCALE_MIN = 0.8;
const SCALE_MAX = 1.6;
const SCALE_STEP = 0.1;
let uiScale = 1;

function setUiScale(v, announce) {
  uiScale = Math.min(SCALE_MAX, Math.max(SCALE_MIN, Math.round(v * 20) / 20));
  document.documentElement.style.setProperty("--ui-scale", String(uiScale));
  const pct = Math.round(uiScale * 100);
  $("set-uiscale").value = String(pct);
  $("uiscale-val").textContent = `${pct}%`;
  // The scrubber is a canvas — its bitmap only follows the new box on redraw.
  drawScrubber();
  try {
    localStorage.setItem(SCALE_STORE, String(uiScale));
  } catch { /* nothing we can do, and nothing worth saying */ }
  if (announce) loadNote(`Interface size ${pct}%`);
}

function initUiScale() {
  const stored = Number(localStorage.getItem(SCALE_STORE));
  setUiScale(Number.isFinite(stored) && stored > 0 ? stored : 1);
  $("set-uiscale").addEventListener("input", () =>
    setUiScale(Number($("set-uiscale").value) / 100));
  document.addEventListener("keydown", (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    // Zoom belongs to the window, not to whichever field holds focus.
    if (e.key === "=" || e.key === "+") setUiScale(uiScale + SCALE_STEP, true);
    else if (e.key === "-" || e.key === "_") setUiScale(uiScale - SCALE_STEP, true);
    else if (e.key === "0") setUiScale(1, true);
    else return;
    e.preventDefault();
  });
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
  initUiScale();
  window.addEventListener("focus", clearWindowNotice);
  // Insurance: a webview that swallows the window focus event must not be
  // able to leave the title stuck on an old render.
  document.addEventListener("pointerdown", clearWindowNotice);
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
  initOptionalCards();
  const st = await invoke("get_status");
  await applyStatus(st);
  initEncoders();
  setTimeout(initUpdater, 2500);
  autoConnectGame();
});
