// Analyze window: the replay at full window size with overlays, live
// playback at any speed, and an options drawer that hides without losing
// a single setting. Standalone — it talks to the backend directly.
"use strict";

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
const dialog = window.__TAURI__.dialog;
const $ = (id) => document.getElementById(id);

const ACCENT = "#2fd6d0";
const GHOST = "#ff9c41";
const MUTED = "#8b98a9";
const DANGER = "#ff5d6c";
const WARN = "#f2c14e";
const OK = "#58d68b";

// Everything the drawer edits lives here, so hiding the drawer can never
// drop a setting — the panel is rebuilt from this state when it reopens.
const opt = {
  path: true,
  raw: true,
  markers: false,
  hitboxes: true,
  heatmap: false,
  pathWindow: 600,
  quality: 1080,
  immersive: true,
  section: "overlays",
};

let status = null;
let data = null;
let dataKey = "";
let loading = false;
let timeline = null;
let currentMs = 0;
let lastFrame = null;
let selNote = -1;
let busy = false;
let wanted = false;
let previewTimer = null;
const play = { on: false, factor: 1, last: 0, gen: 0 };
let heatCanvases = { main: null, ghost: null };
let hideChromeTimer = null;
// Set by render events; status polling alone lags a whole render behind.
let renderBusy = false;

// ------------------------------------------------------------ helpers

const fmt1 = (v) => (Math.round(v * 10) / 10).toLocaleString("en-US");
const fmt2 = (v) => (Math.round(v * 100) / 100).toLocaleString("en-US");
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
const esc = (s) =>
  String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

const fmtMs = (ms) => {
  const t = Math.max(0, ms) / 1000;
  return `${Math.floor(t / 60)}:${String(Math.floor(t % 60)).padStart(2, "0")}`;
};
const fmtMsFull = (ms) => {
  const t = Math.max(0, ms) / 1000;
  return `${Math.floor(t / 60)}:${String(Math.floor(t % 60)).padStart(2, "0")}.${String(
    Math.floor((t % 1) * 1000),
  ).padStart(3, "0")}`;
};

const runEnd = () => timeline?.length_ms || status?.replay?.length_ms || 0;
const noteById = (i) => data?.main?.notes.find((n) => n.i === i);

function lastIndexLE(arr, t) {
  let lo = 0;
  let hi = arr.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (arr[mid] <= t) lo = mid + 1;
    else hi = mid;
  }
  return lo - 1;
}

function msg(text) {
  const el = $("an-msg");
  if (!text) {
    el.hidden = true;
    return;
  }
  el.textContent = text;
  el.hidden = false;
}

// ------------------------------------------------------------ preview

function schedulePreview() {
  if (!status?.replay || !status?.map) return;
  wanted = true;
  clearTimeout(previewTimer);
  previewTimer = setTimeout(runPreview, 40);
}

async function runPreview() {
  if (busy || !wanted) return;
  wanted = false;
  busy = true;
  try {
    const d = await invoke("preview_analyze", { timeMs: currentMs });
    lastFrame = d;
    const img = $("an-img");
    img.src = d.img;
    msg("");
    updateTime();
    requestAnimationFrame(() => {
      drawOverlay();
      refreshLive();
    });
  } catch (e) {
    msg(String(e));
  } finally {
    busy = false;
    if (wanted) runPreview();
  }
}

function seek(t) {
  currentMs = clamp(t, 0, runEnd());
  updateTime();
  drawScrub();
  schedulePreview();
}

function updateTime() {
  $("an-time").textContent = fmtMsFull(currentMs);
  $("an-total").textContent = fmtMs(runEnd());
}

// ------------------------------------------------------------ overlay

function syncCanvas() {
  const img = $("an-img");
  const cv = $("an-overlay");
  if (!img.naturalWidth) return false;
  const r = img.getBoundingClientRect();
  const s = $("stage").getBoundingClientRect();
  cv.style.left = `${r.left - s.left}px`;
  cv.style.top = `${r.top - s.top}px`;
  cv.style.width = `${r.width}px`;
  cv.style.height = `${r.height}px`;
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(r.width * dpr));
  const h = Math.max(1, Math.round(r.height * dpr));
  if (cv.width !== w || cv.height !== h) {
    cv.width = w;
    cv.height = h;
  }
  return true;
}

function projectPx(side, wx, wy) {
  const m = side.m;
  const x = m[0][0] * wx + m[1][0] * wy + m[3][0];
  const y = m[0][1] * wx + m[1][1] * wy + m[3][1];
  const w = m[0][3] * wx + m[1][3] * wy + m[3][3];
  if (w <= 1e-6) return null;
  return [(x / w) * 0.5 * side.w + 0.5 * side.w + side.x, (0.5 - (y / w) * 0.5) * lastFrame.h];
}

function buildHeatCanvas(hm, color) {
  const c = document.createElement("canvas");
  c.width = hm.size;
  c.height = hm.size;
  const ctx = c.getContext("2d");
  const img = ctx.createImageData(hm.size, hm.size);
  const r = parseInt(color.slice(1, 3), 16);
  const g = parseInt(color.slice(3, 5), 16);
  const b = parseInt(color.slice(5, 7), 16);
  for (let i = 0; i < hm.counts.length; i++) {
    img.data[i * 4] = r;
    img.data[i * 4 + 1] = g;
    img.data[i * 4 + 2] = b;
    img.data[i * 4 + 3] = Math.round(hm.counts[i] * 0.8);
  }
  ctx.putImageData(img, 0, 0);
  return c;
}

function pathFrom(ctx, pts) {
  ctx.beginPath();
  ctx.moveTo(pts[0][0], pts[0][1]);
  for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i][0], pts[i][1]);
  ctx.closePath();
}

function drawOverlay() {
  const cv = $("an-overlay");
  const ctx = cv.getContext("2d");
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, cv.width, cv.height);
  if (!syncCanvas() || !data || !lastFrame) return;
  const img = $("an-img");
  const dpr = window.devicePixelRatio || 1;
  const r = img.getBoundingClientRect();
  ctx.scale((r.width * dpr) / lastFrame.w, (r.height * dpr) / lastFrame.h);
  ctx.lineJoin = "round";
  ctx.lineCap = "round";
  const t = currentMs;

  lastFrame.sides.forEach((side, si) => {
    const a = si === 0 ? data.main : data.ghost;
    const color = si === 0 ? ACCENT : GHOST;
    if (!a) return;

    // Two clips: the side's viewport (split halves must never bleed into
    // each other) and, for the field-bound layers, the playfield border.
    const clipSide = () => {
      ctx.beginPath();
      ctx.rect(side.x, 0, side.w, lastFrame.h);
      ctx.clip();
    };
    ctx.save();
    clipSide();
    pathFrom(ctx, side.field);
    ctx.clip();

    if (opt.heatmap) {
      const hc = heatCanvases[si === 0 ? "main" : "ghost"];
      if (hc) {
        // Projection is perspective (and rotates under SpinCamera), so a
        // single affine would skew the map. Tile it: each cell gets its
        // own transform from its own projected corners.
        const e = a.heatmap.extent;
        const N = 8;
        const step = (2 * e) / N;
        const sw = hc.width / N;
        const sh = hc.height / N;
        for (let gy = 0; gy < N; gy++) {
          for (let gx = 0; gx < N; gx++) {
            const x0 = -e + gx * step;
            const y1 = e - gy * step;
            const tl = projectPx(side, x0, y1);
            const tr = projectPx(side, x0 + step, y1);
            const bl = projectPx(side, x0, y1 - step);
            if (!tl || !tr || !bl) continue;
            ctx.save();
            ctx.transform(
              (tr[0] - tl[0]) / sw,
              (tr[1] - tl[1]) / sw,
              (bl[0] - tl[0]) / sh,
              (bl[1] - tl[1]) / sh,
              tl[0],
              tl[1],
            );
            // Half-pixel overlap hides seams between the tiles.
            ctx.drawImage(hc, gx * sw, gy * sh, sw, sh, 0, 0, sw + 0.5, sh + 0.5);
            ctx.restore();
          }
        }
      }
    }

    if (opt.hitboxes) {
      // Notes may poke past the border (the render draws them un-clipped),
      // so their boxes only obey the viewport clip.
      ctx.restore();
      ctx.save();
      clipSide();
      // The backend hands us the note quads exactly as the renderer draws
      // them, so the box sits on the approaching note and shrinks with it.
      for (const q of side.notes) {
        const n = si === 0 ? noteById(q.i) : a.notes.find((x) => x.i === q.i);
        const sel = si === 0 && q.i === selNote;
        const hit = n ? n.hit : true;
        ctx.strokeStyle = sel ? ACCENT : hit ? "rgba(180,195,210,0.6)" : "rgba(255,93,108,0.85)";
        ctx.lineWidth = sel ? 2.4 : 1.3;
        ctx.setLineDash(hit ? [] : [5, 4]);
        pathFrom(ctx, q.pts);
        ctx.stroke();
        ctx.setLineDash([]);
        if (sel) {
          ctx.fillStyle = "rgba(47,214,208,0.12)";
          ctx.fill();
        }
      }
      ctx.restore();
      ctx.save();
      clipSide();
      pathFrom(ctx, side.field);
      ctx.clip();
    }

    const ft = a.frames.t;
    const lo = Math.max(0, lastIndexLE(ft, t - opt.pathWindow));
    const hi = lastIndexLE(ft, t);

    if (opt.path && hi > lo) {
      ctx.strokeStyle = color;
      ctx.globalAlpha = 0.75;
      ctx.lineWidth = 1.7;
      ctx.beginPath();
      let started = false;
      for (let j = lo; j <= hi; j++) {
        if (j > lo && ft[j] - ft[j - 1] > 500) started = false;
        const p = projectPx(side, a.frames.x[j], a.frames.y[j]);
        if (!p) continue;
        if (!started) {
          ctx.moveTo(p[0], p[1]);
          started = true;
        } else {
          ctx.lineTo(p[0], p[1]);
        }
      }
      ctx.stroke();
      ctx.globalAlpha = 1;
    }

    if (opt.markers && hi > lo) {
      ctx.fillStyle = color;
      ctx.globalAlpha = 0.9;
      for (let j = lo; j <= hi; j++) {
        const p = projectPx(side, a.frames.x[j], a.frames.y[j]);
        if (!p) continue;
        ctx.beginPath();
        ctx.arc(p[0], p[1], 1.8, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.globalAlpha = 1;
    }

    if (opt.raw) {
      let p = null;
      if (hi >= 0 && hi + 1 < ft.length && ft[hi + 1] > ft[hi]) {
        const k = clamp((t - ft[hi]) / (ft[hi + 1] - ft[hi]), 0, 1);
        p = projectPx(
          side,
          a.frames.x[hi] + (a.frames.x[hi + 1] - a.frames.x[hi]) * k,
          a.frames.y[hi] + (a.frames.y[hi + 1] - a.frames.y[hi]) * k,
        );
      } else if (hi >= 0) {
        p = projectPx(side, a.frames.x[hi], a.frames.y[hi]);
      }
      if (p) {
        ctx.strokeStyle = "#ffffff";
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.moveTo(p[0] - 8, p[1]);
        ctx.lineTo(p[0] + 8, p[1]);
        ctx.moveTo(p[0], p[1] - 8);
        ctx.lineTo(p[0], p[1] + 8);
        ctx.stroke();
      }
    }
    ctx.restore();
  });
}

function pickNote(ev) {
  if (!data || !lastFrame) return;
  const cv = $("an-overlay");
  const rect = cv.getBoundingClientRect();
  const mx = ((ev.clientX - rect.left) / rect.width) * lastFrame.w;
  const my = ((ev.clientY - rect.top) / rect.height) * lastFrame.h;
  const side = lastFrame.sides[0];
  let best = -1;
  let bestD = Infinity;
  for (const q of side.notes) {
    const cx = (q.pts[0][0] + q.pts[2][0]) / 2;
    const cy = (q.pts[0][1] + q.pts[2][1]) / 2;
    const halfW = Math.abs(q.pts[1][0] - q.pts[0][0]) / 2 + 6;
    const halfH = Math.abs(q.pts[2][1] - q.pts[1][1]) / 2 + 6;
    if (Math.abs(mx - cx) <= halfW && Math.abs(my - cy) <= halfH) {
      const d = Math.hypot(mx - cx, my - cy);
      if (d < bestD) {
        bestD = d;
        best = q.i;
      }
    }
  }
  selNote = best;
  drawOverlay();
  if (opt.section === "notes") drawSection();
}

// ------------------------------------------------------------ scrubber

function drawScrub() {
  const cv = $("an-scrub");
  const dpr = window.devicePixelRatio || 1;
  const w = cv.clientWidth;
  const h = cv.clientHeight;
  if (!w || !h) return;
  if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) {
    cv.width = Math.round(w * dpr);
    cv.height = Math.round(h * dpr);
  }
  const ctx = cv.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const len = runEnd();
  if (!len) return;
  ctx.fillStyle = "rgba(139,152,169,0.18)";
  ctx.fillRect(0, h / 2 - 2, w, 4);
  if (timeline?.miss_times?.length) {
    ctx.fillStyle = "rgba(255,93,108,0.75)";
    for (const t of timeline.miss_times) {
      ctx.fillRect((t / len) * w, h / 2 - 8, 1.2, 16);
    }
  }
  ctx.fillStyle = ACCENT;
  ctx.fillRect((currentMs / len) * w - 1, 2, 2, h - 4);
}

// ------------------------------------------------------------ playback

function setPlaying(on) {
  if (on && (!data || !status?.replay)) return;
  play.on = on;
  play.gen++;
  $("an-play").textContent = on ? "⏸" : "▶";
  document.body.classList.toggle("an-immersive", on && opt.immersive);
  if (on) {
    if (currentMs >= runEnd() - 1) currentMs = 0;
    play.last = performance.now();
    pump(play.gen);
  }
}

async function pump(gen) {
  if (gen !== play.gen || !play.on) return;
  if (renderBusy || status?.rendering) {
    setPlaying(false);
    msg("Paused — a video render is using the renderer.");
    return;
  }
  const now = performance.now();
  const sp = clamp(status?.replay?.speed || 1, 0.25, 3);
  currentMs = Math.min(currentMs + (now - play.last) * play.factor * sp, runEnd());
  play.last = now;
  updateTime();
  drawScrub();
  const t0 = performance.now();
  try {
    wanted = true;
    await runPreview();
  } catch (e) {
    /* transient — keep the loop alive */
  }
  if (performance.now() - t0 < 8) await new Promise((r) => setTimeout(r, 8));
  if (gen !== play.gen) return;
  if (currentMs >= runEnd()) {
    setPlaying(false);
    return;
  }
  pump(gen);
}

function stepFrame(dir) {
  const ft = data?.main?.frames?.t;
  if (!ft?.length) return;
  setPlaying(false);
  let i = lastIndexLE(ft, currentMs);
  if (!(dir < 0 && i >= 0 && ft[i] < currentMs)) i += dir;
  seek(ft[clamp(i, 0, ft.length - 1)]);
}

function setSpeed(v) {
  play.factor = clamp(v, 0.01, 4);
  $("an-speed").value = String(Math.round(play.factor * 100));
  $("an-speed-num").value = String(Math.round(play.factor * 100) / 100);
}

// ------------------------------------------------------------ options

const SECTIONS = [
  ["overlays", "Overlays"],
  ["cursor", "Cursor"],
  ["timing", "Timing"],
  ["misses", "Misses"],
  ["notes", "Note"],
  ["ghost", "Ghost"],
  ["integrity", "Integrity"],
  ["export", "Export"],
  ["view", "View"],
];

function kv(label, value, title) {
  return `<div class="an-kv"${title ? ` title="${esc(title)}"` : ""}><span>${label}</span><b>${value}</b></div>`;
}
function card(title, inner) {
  return `<div class="an-card"><div class="an-title">${title}</div>${inner}</div>`;
}

function renderNav() {
  const nav = $("an-secnav");
  nav.innerHTML = SECTIONS.filter(([id]) => id !== "ghost" || data?.ghost)
    .map(([id, label]) => `<button data-sec="${id}"${opt.section === id ? ' class="active"' : ""}>${label}</button>`)
    .join("");
}

function drawSection() {
  const body = $("an-secbody");
  if ($("an-options").hidden) return;
  if (!data) {
    body.innerHTML = `<p class="hint">${loading ? "Analyzing replay…" : "Load a replay in the main window."}</p>`;
    return;
  }
  if (opt.section === "ghost" && !data.ghost) opt.section = "overlays";
  const a = data.main;
  let html = "";

  if (opt.section === "overlays") {
    html += card(
      "Show on the replay",
      `<div class="an-toggles">
        ${[
          ["path", "Cursor path"],
          ["raw", "Raw cursor"],
          ["markers", "Frame markers"],
          ["hitboxes", "Note hitboxes"],
          ["heatmap", "Heatmap"],
        ]
          .map(
            ([k, label]) =>
              `<label class="an-tog"><input type="checkbox" data-opt="${k}"${opt[k] ? " checked" : ""}> ${label}</label>`,
          )
          .join("")}
      </div>
      <label class="hint an-slider">Path window
        <input type="range" id="opt-window" min="100" max="4000" step="50" value="${opt.pathWindow}">
        <span>${(opt.pathWindow / 1000).toFixed(2)}s</span></label>
      <p class="hint">Hitboxes follow each note as it flies in — they are the note's own hit area, so they grow toward the hit plane and stay inside the field.</p>`,
    );
  } else if (opt.section === "cursor") {
    const c = a.cursor;
    const t = a.speed_series.t;
    const i = Math.max(0, lastIndexLE(t, currentMs));
    html += card(
      "Speed",
      kv("Now", `<span id="an-live-speed">${t.length ? `${fmt1(a.speed_series.v[i])} cells/s` : "–"}</span>`) +
        kv("Average / p95", `${fmt1(c.avg_speed)} / ${fmt1(c.p95_speed)} cells/s`) +
        kv("Max", `<a class="an-jump" data-t="${c.max_speed.t}">${fmt1(c.max_speed.v)} @ ${fmtMs(c.max_speed.t)}</a>`) +
        kv("Max accel", `<a class="an-jump" data-t="${c.max_accel.t}">${fmt1(c.max_accel.v)} cells/s²</a>`) +
        `<canvas class="an-graph" data-series="speed"></canvas>`,
    );
    html += card(
      "Movement",
      kv("Path / optimal", `${fmt1(c.total_path_cells)} / ${fmt1(c.optimal_path_cells)} cells`) +
        kv("Efficiency", `${fmt1(c.efficiency_pct)}%`, "Shortest route through all notes vs. what the cursor travelled") +
        kv("Moving", `${fmt1(c.moving_pct)}% of the time`) +
        kv("Overshoot", `${fmt1(a.overshoot.rate_pct)}% · avg ${fmt2(a.overshoot.avg_cells)} cells${a.overshoot.worst ? ` · <a class="an-jump" data-t="${a.overshoot.worst.t}">worst</a>` : ""}`) +
        kv("Snap vs flow", `${Math.round(a.snap_flow.snap_pct)}% / ${Math.round(a.snap_flow.flow_pct)}%`) +
        kv("Micro-jitter", `${fmt2(a.jitter.rms_cells * 100)} cells·10⁻²`),
    );
    html += card(
      "Aim placement",
      `<canvas class="an-scatter"></canvas>` +
        kv("Bias", `${fmt2(a.direction_bias.dx)} / ${fmt2(a.direction_bias.dy)} cells`, "Mean hit offset from note centres — the arrow"),
    );
  } else if (opt.section === "timing") {
    const tm = a.timing;
    html += card(
      "Hit timing",
      kv("Unstable rate", fmt1(tm.ur)) +
        kv("Mean / median", `${fmt1(tm.mean_err_ms)} / ${fmt1(tm.median_err_ms)} ms`) +
        kv("Drift", `${tm.drift_ms_per_min >= 0 ? "+" : ""}${fmt1(tm.drift_ms_per_min)} ms/min`, "Positive = hitting later as the run goes on") +
        `<canvas class="an-graph" data-hist="timing"></canvas>`,
    );
    html += card(
      "First vs second half",
      `<div class="an-halves"><span></span><span>1st</span><span>2nd</span>
        <span>Acc</span><span>${fmt1(tm.first_half.acc_pct)}%</span><span>${fmt1(tm.second_half.acc_pct)}%</span>
        <span>UR</span><span>${fmt1(tm.first_half.ur)}</span><span>${fmt1(tm.second_half.ur)}</span>
        <span>Speed</span><span>${fmt1(tm.first_half.avg_speed)}</span><span>${fmt1(tm.second_half.avg_speed)}</span></div>`,
    );
    html += card("Consistency (rolling UR)", `<canvas class="an-graph" data-series="rollur"></canvas>`);
    const sec = [...a.sections].sort((x, y) => x.acc_pct - y.acc_pct).slice(0, 8);
    if (sec.length) {
      html += card(
        "Toughest sections",
        `<div class="an-list">${sec
          .map(
            (s) =>
              `<a class="an-jump" data-t="${s.start_ms}">${fmtMs(s.start_ms)}–${fmtMs(s.end_ms)} · ${fmt1(s.acc_pct)}% · UR ${Math.round(s.ur)} · ${s.misses} miss</a>`,
          )
          .join("")}</div>`,
      );
    }
  } else if (opt.section === "misses") {
    const ms = a.misses;
    const worst = a.notes
      .filter((n) => !n.hit && n.near_dist != null)
      .sort((x, y) => x.near_dist - y.near_dist)
      .slice(0, 20);
    html += card(
      "Summary",
      kv("Total", `${ms.count}`) +
        (ms.count
          ? kv("Barely / lost", `${Math.round(ms.barely_pct)}% / ${Math.round(ms.lost_pct)}%`, "Barely: within 0.65 cells · lost: never within 1.2") +
            kv("Context", `${ms.on_fast_jumps} fast jumps · ${ms.on_streams} streams · ${ms.other} other`)
          : ""),
    );
    if (worst.length) {
      html += card(
        "Closest calls",
        `<div class="an-list">${worst
          .map(
            (n) =>
              `<a class="an-jump" data-t="${n.t}" data-note="${n.i}">${fmtMs(n.t)} — ${fmt2(n.near_dist)} cells away</a>`,
          )
          .join("")}</div>`,
      );
    }
  } else if (opt.section === "notes") {
    const n = selNote >= 0 ? noteById(selNote) : null;
    html += card(
      "Note inspector",
      n
        ? kv("Note", `#${n.i + 1} @ <a class="an-jump" data-t="${n.t}">${fmtMs(n.t)}</a>`) +
            kv("Result", n.hit ? `<span class="an-ok">hit</span>` : `<span class="an-bad">miss</span>`) +
            (n.hit
              ? kv("Timing", `${n.err_ms >= 0 ? "+" : ""}${fmt1(n.err_ms)} ms (${n.err_ms >= 0 ? "late" : "early"})`) +
                kv("Hit offset", `${fmt2(n.dist)} cells (${fmt2(n.off_x)}, ${fmt2(n.off_y)})`)
              : kv("Closest approach", n.near_dist != null ? `${fmt2(n.near_dist)} cells` : "–")) +
            kv("Approach speed", `${fmt1(n.approach_v)} cells/s`)
        : `<p class="hint">Click a note in the replay to inspect it. Pause first (Space) and step with ← / → to catch a single frame.</p>`,
    );
  } else if (opt.section === "ghost" && data.ghost) {
    const g = data.ghost;
    html += card(
      `${esc(data.player)} vs ${esc(data.ghost_player || "ghost")}`,
      `<div class="an-halves an-vs"><span></span><span class="an-main">${esc(data.player)}</span><span class="an-ghost">${esc(data.ghost_player || "ghost")}</span>
        <span>Acc</span><span>${fmt1((a.meta.hits / Math.max(1, a.meta.hits + a.meta.misses)) * 100)}%</span><span>${fmt1((g.meta.hits / Math.max(1, g.meta.hits + g.meta.misses)) * 100)}%</span>
        <span>UR</span><span>${fmt1(a.timing.ur)}</span><span>${fmt1(g.timing.ur)}</span>
        <span>Avg speed</span><span>${fmt1(a.cursor.avg_speed)}</span><span>${fmt1(g.cursor.avg_speed)}</span>
        <span>Efficiency</span><span>${fmt1(a.cursor.efficiency_pct)}%</span><span>${fmt1(g.cursor.efficiency_pct)}%</span>
        <span>Misses</span><span>${a.meta.misses}</span><span>${g.meta.misses}</span></div>`,
    );
    html += card("Cursor distance between runs", `<canvas class="an-graph" data-series="ghostdist"></canvas>`);
  } else if (opt.section === "integrity") {
    const v = a.verdict;
    html += card(
      "Signals",
      `<div class="an-verdict"><span class="chip ${v === "clean" ? "ok" : v === "notice" ? "warn" : "bad"}">${
        v === "clean" ? "no integrity signals" : v === "notice" ? "signals — take a look" : "strong signals"
      }</span></div>` +
        (a.signals.length
          ? a.signals
              .map(
                (s) => `<div class="an-signal"><span class="chip ${
                  s.severity === "warn" ? "bad" : s.severity === "notice" ? "warn" : "info"
                }">${s.severity}</span><div><b>${esc(s.title)}</b><p class="hint">${esc(s.detail)}</p>${
                  s.times.length
                    ? `<div class="an-list an-inline">${s.times.map((t) => `<a class="an-jump" data-t="${t}">${fmtMs(t)}</a>`).join("")}</div>`
                    : ""
                }</div></div>`,
              )
              .join("")
          : `<p class="hint">Nothing unusual found in this replay's data.</p>`) +
        `<p class="hint an-foot">Signals are hints derived from the recording — context, not verdicts.</p>`,
    );
    html += card("Recording rate", kv("Frame delta", `${fmt1(a.frame_deltas.avg_ms)} ms avg · ${fmt1(a.frame_deltas.median_ms)} ms median`) + `<canvas class="an-graph" data-hist="delta"></canvas>`);
  } else if (opt.section === "export") {
    html += card(
      "Save the analysis",
      `<div class="an-actions">
        <button class="btn small" id="exp-card">Analysis card (PNG)</button>
        <button class="btn small ghost" id="exp-json">JSON</button>
        <button class="btn small ghost" id="exp-csv">CSV</button>
      </div><p class="hint">The card is a shareable summary; JSON and CSV carry the per-note data for your own analysis.</p>`,
    );
  } else if (opt.section === "view") {
    html += card(
      "Preview quality",
      `<div class="an-toggles">
        ${[720, 1080, 1440]
          .map(
            (q) =>
              `<label class="an-tog"><input type="radio" name="q" data-q="${q}"${opt.quality === q ? " checked" : ""}> ${q}p</label>`,
          )
          .join("")}
      </div><p class="hint">Higher is sharper on a big screen and slower to play back. The main window's preview uses the same setting.</p>`,
    );
    html += card(
      "While playing",
      `<label class="an-tog"><input type="checkbox" data-opt="immersive"${opt.immersive ? " checked" : ""}> Hide the controls during playback</label>
       <p class="hint">They come back the moment you move the mouse or pause.</p>`,
    );
    html += card(
      "Shortcuts",
      `<div class="an-list"><span>Space — play / pause</span><span>← / → — one frame</span><span>Shift + ← / → — one second</span><span>O — options · Esc — hide options</span><span>Click a note — inspect it</span></div>`,
    );
  }

  body.innerHTML = html;
  wireSection();
}

/// Per-frame update of the drawer: canvases and live readouts only —
/// rebuilding the DOM here would swallow clicks and abort slider drags.
function refreshLive() {
  if ($("an-options").hidden || !data) return;
  redrawGraphs();
  const live = $("an-live-speed");
  if (live) {
    const t = data.main.speed_series.t;
    const i = Math.max(0, lastIndexLE(t, currentMs));
    live.textContent = t.length ? `${fmt1(data.main.speed_series.v[i])} cells/s` : "–";
  }
}

function redrawGraphs() {
  const body = $("an-secbody");
  body.querySelectorAll("canvas[data-series]").forEach((cv) => {
    const key = cv.dataset.series;
    const s =
      key === "speed" ? data.main.speed_series : key === "rollur" ? data.main.rolling_ur : data.ghost_distance;
    drawSeries(cv, s, { color: key === "ghostdist" ? GHOST : ACCENT });
  });
  body.querySelectorAll("canvas[data-hist]").forEach((cv) => {
    if (cv.dataset.hist === "timing") {
      const tm = data.main.timing;
      drawHist(cv, tm.hist, {
        zeroAt: -tm.hist_start_ms / (tm.hist.length * tm.hist_bin_ms),
        left: "early",
        right: "late",
      });
    } else {
      drawHist(cv, data.main.frame_deltas.hist, { left: "0 ms", right: "40+ ms" });
    }
  });
  const sc = body.querySelector(".an-scatter");
  if (sc) drawScatter(sc, data.main.notes);
}

function wireSection() {
  const body = $("an-secbody");
  body.querySelectorAll("input[data-opt]").forEach((cb) => {
    cb.addEventListener("change", () => {
      opt[cb.dataset.opt] = cb.checked;
      if (cb.dataset.opt === "immersive") {
        document.body.classList.toggle("an-immersive", play.on && opt.immersive);
      }
      drawOverlay();
    });
  });
  const win = body.querySelector("#opt-window");
  if (win) {
    win.addEventListener("input", () => {
      opt.pathWindow = Number(win.value);
      win.nextElementSibling.textContent = `${(opt.pathWindow / 1000).toFixed(2)}s`;
      drawOverlay();
    });
  }
  body.querySelectorAll("input[data-q]").forEach((rb) => {
    rb.addEventListener("change", async () => {
      opt.quality = Number(rb.dataset.q);
      try {
        status = await invoke("set_preview_quality", { height: opt.quality });
        schedulePreview();
      } catch (e) {
        msg(String(e));
      }
    });
  });
  $("exp-card")?.addEventListener("click", exportCard);
  $("exp-json")?.addEventListener("click", exportJson);
  $("exp-csv")?.addEventListener("click", exportCsv);
  body.querySelectorAll("canvas[data-series]").forEach((cv) => {
    cv.addEventListener("click", (e) => {
      const t1 = runEnd();
      if (!t1) return;
      const r = cv.getBoundingClientRect();
      seek(((e.clientX - r.left) / r.width) * t1);
    });
    cv.style.cursor = "crosshair";
  });
  redrawGraphs();
}

// ------------------------------------------------------------ graphs

function gctx(cv) {
  const dpr = window.devicePixelRatio || 1;
  const w = cv.clientWidth;
  const h = cv.clientHeight;
  if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) {
    cv.width = Math.round(w * dpr);
    cv.height = Math.round(h * dpr);
  }
  const ctx = cv.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  return { ctx, w, h };
}

function drawSeries(cv, series, o = {}) {
  const { ctx, w, h } = gctx(cv);
  const t = series?.t || [];
  const v = series?.v || [];
  if (t.length < 2) {
    ctx.fillStyle = MUTED;
    ctx.font = "11px system-ui";
    ctx.fillText("not enough data", 6, h / 2);
    return;
  }
  const t1 = runEnd() || t[t.length - 1];
  const vMax = Math.max(...v) * 1.12 || 1;
  const X = (tt) => (tt / Math.max(1, t1)) * (w - 2) + 1;
  const Y = (vv) => h - 3 - (Math.min(vv, vMax) / vMax) * (h - 10);
  ctx.strokeStyle = "rgba(139,152,169,0.15)";
  ctx.lineWidth = 1;
  for (const f of [0.33, 0.66]) {
    ctx.beginPath();
    ctx.moveTo(0, h - 3 - f * (h - 10));
    ctx.lineTo(w, h - 3 - f * (h - 10));
    ctx.stroke();
  }
  const color = o.color || ACCENT;
  ctx.beginPath();
  ctx.moveTo(X(t[0]), Y(v[0]));
  for (let i = 1; i < t.length; i++) ctx.lineTo(X(t[i]), Y(v[i]));
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.8;
  ctx.stroke();
  ctx.lineTo(X(t[t.length - 1]), h);
  ctx.lineTo(X(t[0]), h);
  ctx.closePath();
  ctx.fillStyle = color + "22";
  ctx.fill();
  ctx.strokeStyle = "rgba(255,255,255,0.7)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(X(currentMs), 0);
  ctx.lineTo(X(currentMs), h);
  ctx.stroke();
}

function drawHist(cv, counts, o = {}) {
  const { ctx, w, h } = gctx(cv);
  if (!counts?.length) return;
  const max = Math.max(...counts, 1);
  const bw = w / counts.length;
  ctx.fillStyle = (o.color || ACCENT) + "cc";
  counts.forEach((c, i) => {
    if (c > 0) ctx.fillRect(i * bw + 0.5, h - (c / max) * (h - 12) - 2, Math.max(1, bw - 1), (c / max) * (h - 12));
  });
  if (o.zeroAt != null) {
    ctx.strokeStyle = "rgba(255,255,255,0.55)";
    ctx.beginPath();
    ctx.moveTo(o.zeroAt * w, 0);
    ctx.lineTo(o.zeroAt * w, h);
    ctx.stroke();
  }
  ctx.fillStyle = MUTED;
  ctx.font = "10px system-ui";
  if (o.left) ctx.fillText(o.left, 4, 10);
  if (o.right) ctx.fillText(o.right, w - ctx.measureText(o.right).width - 4, 10);
}

function drawScatter(cv, notes) {
  const { ctx, w, h } = gctx(cv);
  const cx = w / 2;
  const cy = h / 2;
  const scale = Math.min(w, h) / 2.6;
  ctx.strokeStyle = "rgba(180,195,210,0.5)";
  ctx.lineWidth = 1.2;
  ctx.strokeRect(cx - 0.5 * scale, cy - 0.5 * scale, scale, scale);
  ctx.strokeStyle = "rgba(139,152,169,0.25)";
  ctx.beginPath();
  ctx.moveTo(cx, 4);
  ctx.lineTo(cx, h - 4);
  ctx.moveTo(4, cy);
  ctx.lineTo(w - 4, cy);
  ctx.stroke();
  let sx = 0;
  let sy = 0;
  let n = 0;
  ctx.fillStyle = ACCENT;
  ctx.globalAlpha = 0.45;
  for (const nt of notes) {
    if (nt.off_x == null) continue;
    ctx.beginPath();
    ctx.arc(cx + nt.off_x * scale, cy - nt.off_y * scale, 2.2, 0, Math.PI * 2);
    ctx.fill();
    sx += nt.off_x;
    sy += nt.off_y;
    n++;
  }
  ctx.globalAlpha = 1;
  if (n > 3) {
    const bx = cx + (sx / n) * scale * 4;
    const by = cy - (sy / n) * scale * 4;
    ctx.strokeStyle = WARN;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(bx, by);
    ctx.stroke();
  }
}

// ------------------------------------------------------------ exports

const expBase = () =>
  `${data?.player || "player"} - ${status?.map?.song_name || "run"} - analysis`.replace(/[\\/:*?"<>|]/g, "-");

async function exportJson() {
  const p = await dialog.save({ defaultPath: `${expBase()}.json`, filters: [{ name: "JSON", extensions: ["json"] }] });
  if (!p) return;
  const strip = ({ frames, ...rest }) => rest;
  const out = { ...data, main: strip(data.main), ghost: data.ghost ? strip(data.ghost) : null };
  try {
    await invoke("save_text_file", { path: p, contents: JSON.stringify(out, null, 2) });
    msgFlash(`Saved — ${p}`);
  } catch (e) {
    msg(String(e));
  }
}

async function exportCsv() {
  const p = await dialog.save({ defaultPath: `${expBase()}.csv`, filters: [{ name: "CSV", extensions: ["csv"] }] });
  if (!p) return;
  const rows = ["side,note,time_ms,grid_x,grid_y,hit,hit_ms,err_ms,off_x,off_y,dist_cells,near_dist_cells,approach_speed"];
  const emit = (side, notes) => {
    for (const n of notes) {
      rows.push(
        [
          side,
          n.i + 1,
          n.t,
          n.gx,
          n.gy,
          n.hit ? 1 : 0,
          n.hit_ms ?? "",
          n.err_ms != null ? n.err_ms.toFixed(2) : "",
          n.off_x != null ? n.off_x.toFixed(4) : "",
          n.off_y != null ? n.off_y.toFixed(4) : "",
          n.dist != null ? n.dist.toFixed(4) : "",
          n.near_dist != null ? n.near_dist.toFixed(4) : "",
          n.approach_v.toFixed(2),
        ].join(","),
      );
    }
  };
  emit("main", data.main.notes);
  if (data.ghost) emit("ghost", data.ghost.notes);
  try {
    await invoke("save_text_file", { path: p, contents: rows.join("\n") });
    msgFlash(`Saved — ${p}`);
  } catch (e) {
    msg(String(e));
  }
}

async function exportCard() {
  const p = await dialog.save({ defaultPath: `${expBase()}.png`, filters: [{ name: "PNG image", extensions: ["png"] }] });
  if (!p) return;
  const a = data.main;
  const c = document.createElement("canvas");
  c.width = 1200;
  c.height = 630;
  const x = c.getContext("2d");
  x.fillStyle = "#0b0e12";
  x.fillRect(0, 0, 1200, 630);
  x.fillStyle = "#e8edf3";
  x.font = "600 34px system-ui";
  x.fillText(`${data.player} — ${data.map_title || "run"}`, 48, 72);
  x.fillStyle = MUTED;
  x.font = "16px system-ui";
  x.fillText(`${a.meta.hits}/${a.meta.hits + a.meta.misses} hits · analyzed with rhythr`, 48, 102);
  const stat = (label, value, col, row) => {
    const sx = 48 + col * 280;
    const sy = 170 + row * 96;
    x.fillStyle = MUTED;
    x.font = "14px system-ui";
    x.fillText(label, sx, sy);
    x.fillStyle = "#e8edf3";
    x.font = "600 28px system-ui";
    x.fillText(value, sx, sy + 34);
  };
  stat("Unstable rate", fmt1(a.timing.ur), 0, 0);
  stat("Mean error", `${fmt1(a.timing.mean_err_ms)} ms`, 1, 0);
  stat("Max speed", `${fmt1(a.cursor.max_speed.v)} cells/s`, 2, 0);
  stat("Avg speed", `${fmt1(a.cursor.avg_speed)} cells/s`, 3, 0);
  stat("Efficiency", `${fmt1(a.cursor.efficiency_pct)}%`, 0, 1);
  stat("Overshoot", `${fmt1(a.overshoot.rate_pct)}%`, 1, 1);
  stat("Snap / flow", `${Math.round(a.snap_flow.snap_pct)} / ${Math.round(a.snap_flow.flow_pct)}`, 2, 1);
  stat("Misses", `${a.meta.misses}`, 3, 1);
  const hy = 420;
  const hw = 1104;
  const hh = 130;
  x.fillStyle = MUTED;
  x.font = "14px system-ui";
  x.fillText("Hit timing (early ↔ late)", 48, hy - 10);
  const maxC = Math.max(...a.timing.hist, 1);
  const bw = hw / a.timing.hist.length;
  x.fillStyle = ACCENT;
  a.timing.hist.forEach((cnt, i) => {
    if (cnt > 0) x.fillRect(48 + i * bw, hy + hh - (cnt / maxC) * hh, Math.max(1, bw - 2), (cnt / maxC) * hh);
  });
  x.strokeStyle = "rgba(255,255,255,0.6)";
  const zx = 48 + (-a.timing.hist_start_ms / (a.timing.hist.length * a.timing.hist_bin_ms)) * hw;
  x.beginPath();
  x.moveTo(zx, hy);
  x.lineTo(zx, hy + hh);
  x.stroke();
  x.fillStyle = a.verdict === "clean" ? OK : a.verdict === "notice" ? WARN : DANGER;
  x.font = "600 16px system-ui";
  x.fillText(a.verdict === "clean" ? "● no integrity signals" : `● integrity: ${a.verdict}`, 48, 600);
  try {
    await invoke("save_data_url", { path: p, dataUrl: c.toDataURL("image/png") });
    msgFlash(`Saved — ${p}`);
  } catch (e) {
    msg(String(e));
  }
}

let flashTimer = null;
function msgFlash(text) {
  msg(text);
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => msg(""), 2600);
}

// ------------------------------------------------------------ data

const sourceKey = () =>
  `${status?.replay?.path}|${status?.ghost?.path || ""}|${status?.map?.path}|${status?.settings?.width}x${status?.settings?.height}`;

async function refresh() {
  try {
    status = await invoke("get_status");
  } catch (e) {
    msg(String(e));
    return;
  }
  const title = status?.replay
    ? `${status.replay.player} — ${status.map?.song_name || status.map?.title || ""}`
    : "No replay loaded";
  $("an-title").textContent = title;
  if (!status?.replay || !status?.map) {
    data = null;
    dataKey = "";
    $("an-chip").hidden = true;
    setPlaying(false);
    msg("Load a replay (and its map) in the main window — this view follows it.");
    drawSection();
    return;
  }
  const key = sourceKey();
  if (key === dataKey && data) return;
  await loadData(key);
}

async function loadData(key) {
  if (loading) return;
  loading = true;
  data = null;
  selNote = -1;
  $("an-chip").hidden = true;
  drawSection();
  msg("Analyzing replay…");
  let fresh = null;
  let tl = null;
  try {
    [fresh, tl] = await Promise.all([invoke("analysis_data"), invoke("timeline", { samples: 400 })]);
  } catch (e) {
    loading = false;
    msg(`Analysis failed: ${e}`);
    return;
  }
  loading = false;
  if (sourceKey() !== key) {
    refresh();
    return;
  }
  data = fresh;
  timeline = tl;
  dataKey = key;
  heatCanvases = {
    main: buildHeatCanvas(data.main.heatmap, ACCENT),
    ghost: data.ghost ? buildHeatCanvas(data.ghost.heatmap, GHOST) : null,
  };
  const v = data.main.verdict;
  const chip = $("an-chip");
  chip.hidden = false;
  chip.className = `chip ${v === "clean" ? "ok" : v === "notice" ? "warn" : "bad"}`;
  chip.textContent = v === "clean" ? "no integrity signals" : v === "notice" ? "signals" : "strong signals";
  msg("");
  currentMs = clamp(currentMs, 0, runEnd());
  renderNav();
  drawSection();
  drawScrub();
  schedulePreview();
}

// ------------------------------------------------------------ chrome

function showChrome() {
  document.body.classList.remove("an-immersive");
  clearTimeout(hideChromeTimer);
  if (play.on && opt.immersive) {
    hideChromeTimer = setTimeout(() => {
      if (play.on && opt.immersive) document.body.classList.add("an-immersive");
    }, 1800);
  }
}

function toggleOptions(show) {
  const el = $("an-options");
  const next = show ?? el.hidden;
  el.hidden = !next;
  if (next) {
    renderNav();
    drawSection();
  }
  requestAnimationFrame(() => {
    drawOverlay();
    drawScrub();
  });
}

// ------------------------------------------------------------ boot

window.addEventListener("DOMContentLoaded", async () => {
  $("an-gear").addEventListener("click", () => toggleOptions());
  $("an-close").addEventListener("click", () => toggleOptions(false));
  $("an-play").addEventListener("click", () => setPlaying(!play.on));
  $("an-back").addEventListener("click", () => stepFrame(-1));
  $("an-fwd").addEventListener("click", () => stepFrame(1));
  $("an-speed").addEventListener("input", () => setSpeed(Number($("an-speed").value) / 100));
  $("an-speed-num").addEventListener("change", () => {
    const v = parseFloat(String($("an-speed-num").value).replace(",", "."));
    setSpeed(Number.isFinite(v) && v > 0 ? v : play.factor);
  });
  $("an-speed-reset").addEventListener("click", () => setSpeed(1));
  $("an-overlay").addEventListener("click", pickNote);
  $("an-secnav").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-sec]");
    if (!b) return;
    opt.section = b.dataset.sec;
    renderNav();
    drawSection();
  });
  $("an-secbody").addEventListener("click", (e) => {
    const j = e.target.closest("a.an-jump");
    if (!j) return;
    if (j.dataset.note != null) selNote = Number(j.dataset.note);
    seek(Number(j.dataset.t));
  });
  const scrubSeek = (e) => {
    const r = $("an-scrub").getBoundingClientRect();
    seek(clamp((e.clientX - r.left) / r.width, 0, 1) * runEnd());
  };
  $("an-scrub").addEventListener("pointerdown", (e) => {
    $("an-scrub").setPointerCapture(e.pointerId);
    setPlaying(false);
    scrubSeek(e);
  });
  $("an-scrub").addEventListener("pointermove", (e) => {
    if (e.buttons & 1) scrubSeek(e);
  });

  document.addEventListener("mousemove", showChrome);
  document.addEventListener("keydown", (e) => {
    const t = document.activeElement;
    const typing = t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);
    if (e.key === "Escape") {
      toggleOptions(false);
      return;
    }
    if (typing || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.code === "Space") {
      e.preventDefault();
      setPlaying(!play.on);
      showChrome();
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      if (e.shiftKey) seek(currentMs - 1000);
      else stepFrame(-1);
      showChrome();
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      if (e.shiftKey) seek(currentMs + 1000);
      else stepFrame(1);
      showChrome();
    } else if (e.key.toLowerCase() === "o") {
      toggleOptions();
    }
  });

  new ResizeObserver(() => {
    drawOverlay();
    drawScrub();
  }).observe($("stage"));
  $("an-img").addEventListener("load", () => drawOverlay());

  // The main window drives which replay is loaded; follow its changes.
  listen("sources-changed", () => refresh()).catch(() => {});
  listen("render-stage", () => {
    renderBusy = true;
    setPlaying(false);
  }).catch(() => {});
  for (const ev of ["render-done", "render-cancelled", "render-error"]) {
    listen(ev, () => {
      renderBusy = false;
      msg("");
      refresh();
      schedulePreview();
    }).catch(() => {});
  }
  // Backstop for anything that changes without an event (a render
  // finishing mid-playback, say) — the event above does the real work.
  setInterval(() => {
    if (!play.on) refresh();
  }, 5000);

  setSpeed(1);
  try {
    status = await invoke("set_preview_quality", { height: opt.quality });
  } catch (e) {
    /* falls back to whatever the main window uses */
  }
  await refresh();
  toggleOptions(true);
});

window.addEventListener("beforeunload", () => {
  play.on = false;
  invoke("set_preview_quality", { height: 720 }).catch(() => {});
});
