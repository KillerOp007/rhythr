// Analyze mode: replay analytics, live playback with variable speed, and
// exact overlays on top of the preview frame. Loaded after app.js — shares
// its top-level bindings ($, invoke, status, currentMs, schedulePreview,
// runPreview, drawScrubber, timelineData) via the global lexical scope.
"use strict";

(() => {
  const ACCENT = "#2fd6d0";
  const GHOST = "#ff9c41";
  const MUTED = "#8b98a9";
  const DANGER = "#ff5d6c";
  const WARN = "#f2c14e";
  const OK = "#58d68b";

  let entered = false;
  let data = null; // AnalysisDto from the backend
  let dataKey = "";
  let loading = false;
  let lastFrame = null; // PreviewFrameDto (sides + dims)
  let lastFrameT = 0;
  let selNote = -1; // selected note index (main side)
  let pathWindow = 600;
  const overlays = { path: true, raw: true, markers: false, hitboxes: true, heatmap: false };
  const play = { on: false, factor: 1, last: 0, gen: 0 };
  let heatCanvases = { main: null, ghost: null };

  // ------------------------------------------------------------ helpers

  const fmt1 = (v) => (Math.round(v * 10) / 10).toLocaleString("en-US");
  const fmt2 = (v) => (Math.round(v * 100) / 100).toLocaleString("en-US");
  const fmtMs = (ms) => {
    const t = Math.max(0, ms) / 1000;
    const m = Math.floor(t / 60);
    return `${m}:${String(Math.floor(t % 60)).padStart(2, "0")}`;
  };
  const fmtMsFull = (ms) => {
    const t = Math.max(0, ms) / 1000;
    const m = Math.floor(t / 60);
    return `${m}:${String(Math.floor(t % 60)).padStart(2, "0")}.${String(Math.floor((t % 1) * 1000)).padStart(3, "0")}`;
  };
  const esc2 = (s) =>
    String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

  const runEnd = () => timelineData?.length_ms || status?.replay?.length_ms || 0;

  // notes[] holds only ATTEMPTED notes; `i` is the map-wide index, so
  // positional indexing is wrong — always resolve by id.
  const noteById = (i) => data?.main?.notes.find((n) => n.i === i);

  function seek(t) {
    currentMs = Math.max(0, Math.min(t, runEnd()));
    updateTimeLabel();
    drawScrubber();
    schedulePreview();
  }

  function updateTimeLabel() {
    $("play-time").textContent = fmtMsFull(currentMs);
  }

  // Binary search: index of the last element <= t.
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

  // ------------------------------------------------------------ projection

  // Column-major 4×4 from the backend; world points sit on the hit plane
  // (z = 0), so the z column drops out.
  function projectPx(side, wx, wy, previewW, previewH) {
    const m = side.m;
    const x = m[0][0] * wx + m[1][0] * wy + m[3][0];
    const y = m[0][1] * wx + m[1][1] * wy + m[3][1];
    const w = m[0][3] * wx + m[1][3] * wy + m[3][3];
    if (w <= 1e-6) return null;
    return [(x / w * 0.5 + 0.5) * side.w + side.x, (0.5 - y / w * 0.5) * previewH];
  }

  function gridToWorld(gx, gy) {
    return [gx - 1.0, 1.0 - gy];
  }

  // ------------------------------------------------------------ overlay

  function syncOverlayCanvas() {
    const img = $("preview-img");
    const cv = $("analyze-overlay");
    if (!entered || img.hidden) {
      cv.hidden = true;
      return false;
    }
    cv.hidden = false;
    cv.style.left = `${img.offsetLeft}px`;
    cv.style.top = `${img.offsetTop}px`;
    cv.style.width = `${img.clientWidth}px`;
    cv.style.height = `${img.clientHeight}px`;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(img.clientWidth * dpr));
    const h = Math.max(1, Math.round(img.clientHeight * dpr));
    if (cv.width !== w || cv.height !== h) {
      cv.width = w;
      cv.height = h;
    }
    return true;
  }

  function sideData(i) {
    if (i === 0) return { a: data?.main, color: ACCENT, heat: "main" };
    return { a: data?.ghost, color: GHOST, heat: "ghost" };
  }

  function buildHeatCanvas(hm, color) {
    const c = document.createElement("canvas");
    c.width = hm.size;
    c.height = hm.size;
    const ctx = c.getContext("2d");
    const img = ctx.createImageData(hm.size, hm.size);
    const [r, g, b] = [parseInt(color.slice(1, 3), 16), parseInt(color.slice(3, 5), 16), parseInt(color.slice(5, 7), 16)];
    for (let i = 0; i < hm.counts.length; i++) {
      img.data[i * 4] = r;
      img.data[i * 4 + 1] = g;
      img.data[i * 4 + 2] = b;
      img.data[i * 4 + 3] = Math.round(hm.counts[i] * 0.8);
    }
    ctx.putImageData(img, 0, 0);
    return c;
  }

  function drawOverlay(t) {
    if (!syncOverlayCanvas()) return;
    const cv = $("analyze-overlay");
    const ctx = cv.getContext("2d");
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, cv.width, cv.height);
    if (!data || !lastFrame) return;
    const img = $("preview-img");
    const dpr = window.devicePixelRatio || 1;
    // preview px → canvas device px
    const sx = (img.clientWidth * dpr) / lastFrame.w;
    const sy = (img.clientHeight * dpr) / lastFrame.h;
    ctx.scale(sx, sy);
    ctx.lineJoin = "round";
    ctx.lineCap = "round";

    lastFrame.sides.forEach((side, i) => {
      const { a, color, heat } = sideData(i);
      if (!a) return;
      const P = (wx, wy) => projectPx(side, wx, wy, lastFrame.w, lastFrame.h);

      // clip to this side's viewport so split halves don't bleed over
      ctx.save();
      ctx.beginPath();
      ctx.rect(side.x, 0, side.w, lastFrame.h);
      ctx.clip();

      if (overlays.heatmap) {
        const hc = heatCanvases[heat];
        if (hc) {
          const e = a.heatmap.extent;
          const tl = P(-e, e);
          const tr = P(e, e);
          const bl = P(-e, -e);
          if (tl && tr && bl) {
            ctx.save();
            // affine map of the unit heat image onto the projected square
            ctx.transform(
              (tr[0] - tl[0]) / hc.width,
              (tr[1] - tl[1]) / hc.width,
              (bl[0] - tl[0]) / hc.height,
              (bl[1] - tl[1]) / hc.height,
              tl[0],
              tl[1],
            );
            ctx.drawImage(hc, 0, 0);
            ctx.restore();
          }
        }
      }

      if (overlays.hitboxes) {
        for (const n of a.notes) {
          if (Math.abs(n.t - t) > 400) continue;
          const [wx, wy] = gridToWorld(n.gx, n.gy);
          const p0 = P(wx - 0.5, wy + 0.5);
          const p1 = P(wx + 0.5, wy - 0.5);
          if (!p0 || !p1) continue;
          ctx.strokeStyle = n.i === selNote && i === 0 ? ACCENT : n.hit ? "rgba(180,195,210,0.55)" : "rgba(255,93,108,0.8)";
          ctx.lineWidth = n.i === selNote && i === 0 ? 2.5 : 1.3;
          ctx.setLineDash(n.hit ? [] : [5, 4]);
          ctx.strokeRect(p0[0], p0[1], p1[0] - p0[0], p1[1] - p0[1]);
          ctx.setLineDash([]);
        }
      }

      const ft = a.frames.t;
      const lo = Math.max(0, lastIndexLE(ft, t - pathWindow));
      const hi = lastIndexLE(ft, t);
      if (overlays.path && hi > lo) {
        ctx.strokeStyle = color;
        ctx.globalAlpha = 0.75;
        ctx.lineWidth = 1.6;
        ctx.beginPath();
        let started = false;
        for (let j = lo; j <= hi; j++) {
          // never draw across a pause gap
          if (j > lo && ft[j] - ft[j - 1] > 500) started = false;
          const p = P(a.frames.x[j], a.frames.y[j]);
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

      if (overlays.markers && hi > lo) {
        ctx.fillStyle = color;
        ctx.globalAlpha = 0.9;
        for (let j = lo; j <= hi; j++) {
          const p = P(a.frames.x[j], a.frames.y[j]);
          if (!p) continue;
          ctx.beginPath();
          ctx.arc(p[0], p[1], 1.7, 0, Math.PI * 2);
          ctx.fill();
        }
        ctx.globalAlpha = 1;
      }

      if (overlays.raw) {
        // interpolated raw cursor position at t — the recorded truth
        let px = null;
        if (hi >= 0 && hi + 1 < ft.length && ft[hi + 1] > ft[hi]) {
          const k = Math.min(1, Math.max(0, (t - ft[hi]) / (ft[hi + 1] - ft[hi])));
          px = P(
            a.frames.x[hi] + (a.frames.x[hi + 1] - a.frames.x[hi]) * k,
            a.frames.y[hi] + (a.frames.y[hi + 1] - a.frames.y[hi]) * k,
          );
        } else if (hi >= 0) {
          px = P(a.frames.x[hi], a.frames.y[hi]);
        }
        if (px) {
          ctx.strokeStyle = "#ffffff";
          ctx.lineWidth = 1.4;
          ctx.beginPath();
          ctx.moveTo(px[0] - 7, px[1]);
          ctx.lineTo(px[0] + 7, px[1]);
          ctx.moveTo(px[0], px[1] - 7);
          ctx.lineTo(px[0], px[1] + 7);
          ctx.stroke();
        }
      }
      ctx.restore();
    });
  }

  // ---------------------------------------------------------- note picking

  function pickNote(ev) {
    if (!data || !lastFrame) return;
    const img = $("preview-img");
    const rect = $("analyze-overlay").getBoundingClientRect();
    const mx = ((ev.clientX - rect.left) / rect.width) * lastFrame.w;
    const my = ((ev.clientY - rect.top) / rect.height) * lastFrame.h;
    let best = -1;
    let bestD = 40 * (lastFrame.w / Math.max(1, img.clientWidth)); // ~40 css px
    for (const n of data.main.notes) {
      if (Math.abs(n.t - lastFrameT) > 600) continue;
      const [wx, wy] = gridToWorld(n.gx, n.gy);
      const p = projectPx(lastFrame.sides[0], wx, wy, lastFrame.w, lastFrame.h);
      if (!p) continue;
      const d = Math.hypot(p[0] - mx, p[1] - my);
      if (d < bestD) {
        bestD = d;
        best = n.i;
      }
    }
    selNote = best;
    renderInspector();
    drawOverlay(lastFrameT);
  }

  // ------------------------------------------------------------ playback

  function setPlaying(on) {
    if (on && (!entered || !data)) return;
    play.on = on;
    play.gen++; // any in-flight pump loop belongs to an old generation now
    $("btn-play").textContent = on ? "⏸" : "▶";
    if (on) {
      if (currentMs >= runEnd() - 1) currentMs = 0;
      play.last = performance.now();
      pump(play.gen);
    }
  }

  async function pump(gen) {
    if (gen !== play.gen || !play.on || !entered) return;
    if (rendering) {
      // A video render owns the pipeline — pause instead of hammering it.
      setPlaying(false);
      return;
    }
    const now = performance.now();
    const sp = Math.max(0.25, Math.min(3, status?.replay?.speed || 1));
    currentMs = Math.min(currentMs + (now - play.last) * play.factor * sp, runEnd());
    play.last = now;
    updateTimeLabel();
    drawScrubber();
    const t0 = performance.now();
    try {
      previewWanted = true;
      await runPreview(); // renders + calls onFrame → overlay redraw
    } catch (e) {
      /* keep the loop alive on a transient error */
    }
    if (performance.now() - t0 < 8) {
      await new Promise((r) => setTimeout(r, 8));
    }
    if (gen !== play.gen) return;
    if (currentMs >= runEnd()) {
      setPlaying(false);
      return;
    }
    pump(gen);
  }

  function stepFrame(dir) {
    if (!data) return;
    const ft = data.main.frames.t;
    if (!ft.length) return;
    let i = lastIndexLE(ft, currentMs);
    // Between two frames the preview shows frame i — stepping back goes
    // TO i first instead of skipping over it.
    if (!(dir < 0 && i >= 0 && ft[i] < currentMs)) {
      i += dir;
    }
    i = Math.max(0, Math.min(ft.length - 1, i));
    seek(ft[i]);
  }

  // ------------------------------------------------------------ graphs

  function graphCtx(canvas) {
    const dpr = window.devicePixelRatio || 1;
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
    }
    const ctx = canvas.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    return { ctx, w, h };
  }

  // Single-series time graph with playhead; click = seek, hover = readout.
  function drawSeries(canvas, series, opts = {}) {
    const { ctx, w, h } = graphCtx(canvas);
    const t = series?.t || [];
    const v = series?.v || [];
    if (t.length < 2) {
      ctx.fillStyle = MUTED;
      ctx.font = "11px system-ui";
      ctx.fillText("not enough data", 8, h / 2);
      return;
    }
    const t0 = 0;
    const t1 = runEnd() || t[t.length - 1];
    let vMax = opts.vMax ?? Math.max(...v) * 1.12;
    if (!(vMax > 0)) vMax = 1;
    const X = (tt) => ((tt - t0) / Math.max(1, t1 - t0)) * (w - 2) + 1;
    const Y = (vv) => h - 3 - (Math.min(vv, vMax) / vMax) * (h - 10);
    // recessive grid: two horizontal lines
    ctx.strokeStyle = "rgba(139,152,169,0.15)";
    ctx.lineWidth = 1;
    for (const f of [0.33, 0.66]) {
      ctx.beginPath();
      ctx.moveTo(0, h - 3 - f * (h - 10));
      ctx.lineTo(w, h - 3 - f * (h - 10));
      ctx.stroke();
    }
    const color = opts.color || ACCENT;
    ctx.beginPath();
    ctx.moveTo(X(t[0]), Y(v[0]));
    for (let i = 1; i < t.length; i++) ctx.lineTo(X(t[i]), Y(v[i]));
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.8;
    ctx.stroke();
    // soft fill to the baseline
    ctx.lineTo(X(t[t.length - 1]), h);
    ctx.lineTo(X(t[0]), h);
    ctx.closePath();
    ctx.fillStyle = color + "22";
    ctx.fill();
    // playhead
    const px = X(currentMs);
    ctx.strokeStyle = "rgba(255,255,255,0.7)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(px, 0);
    ctx.lineTo(px, h);
    ctx.stroke();
    if (opts.marker) {
      const mx = X(opts.marker.t);
      ctx.fillStyle = WARN;
      ctx.beginPath();
      ctx.arc(mx, Y(opts.marker.v), 3, 0, Math.PI * 2);
      ctx.fill();
    }
    canvas.dataset.t1 = String(t1);
  }

  function seriesSeekHandler(canvas) {
    canvas.addEventListener("click", (e) => {
      const t1 = Number(canvas.dataset.t1 || 0);
      if (!t1) return;
      const r = canvas.getBoundingClientRect();
      seek(((e.clientX - r.left) / r.width) * t1);
    });
    canvas.classList.add("an-clickable");
  }

  function drawHist(canvas, counts, opts = {}) {
    const { ctx, w, h } = graphCtx(canvas);
    if (!counts?.length) return;
    const max = Math.max(...counts, 1);
    const bw = w / counts.length;
    ctx.fillStyle = (opts.color || ACCENT) + "cc";
    for (let i = 0; i < counts.length; i++) {
      const bh = (counts[i] / max) * (h - 12);
      if (counts[i] > 0) ctx.fillRect(i * bw + 0.5, h - bh - 2, Math.max(1, bw - 1), bh);
    }
    if (opts.zeroAt != null) {
      const zx = opts.zeroAt * w;
      ctx.strokeStyle = "rgba(255,255,255,0.55)";
      ctx.beginPath();
      ctx.moveTo(zx, 0);
      ctx.lineTo(zx, h);
      ctx.stroke();
    }
    ctx.fillStyle = MUTED;
    ctx.font = "10px system-ui";
    if (opts.left) ctx.fillText(opts.left, 4, 10);
    if (opts.right) {
      const tw = ctx.measureText(opts.right).width;
      ctx.fillText(opts.right, w - tw - 4, 10);
    }
  }

  function drawScatter(canvas, notes) {
    const { ctx, w, h } = graphCtx(canvas);
    const cx = w / 2;
    const cy = h / 2;
    const scale = Math.min(w, h) / 2.6; // ±1.3 cells visible
    // hitbox square ±0.5
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
      const px = cx + nt.off_x * scale;
      const py = cy - nt.off_y * scale; // world +y up
      ctx.beginPath();
      ctx.arc(px, py, 2.2, 0, Math.PI * 2);
      ctx.fill();
      sx += nt.off_x;
      sy += nt.off_y;
      n++;
    }
    ctx.globalAlpha = 1;
    if (n > 3) {
      // bias arrow from centre
      const bx = cx + (sx / n) * scale * 4;
      const by = cy - (sy / n) * scale * 4;
      ctx.strokeStyle = WARN;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.lineTo(bx, by);
      ctx.stroke();
      const ang = Math.atan2(by - cy, bx - cx);
      ctx.beginPath();
      ctx.moveTo(bx, by);
      ctx.lineTo(bx - 6 * Math.cos(ang - 0.4), by - 6 * Math.sin(ang - 0.4));
      ctx.moveTo(bx, by);
      ctx.lineTo(bx - 6 * Math.cos(ang + 0.4), by - 6 * Math.sin(ang + 0.4));
      ctx.stroke();
    }
  }

  // ------------------------------------------------------------ panels

  function kv(label, value, title) {
    return `<div class="an-kv" ${title ? `title="${esc2(title)}"` : ""}><span>${label}</span><b>${value}</b></div>`;
  }

  function card(title, inner, extra = "") {
    return `<div class="an-card"><div class="an-title">${title}${extra}</div>${inner}</div>`;
  }

  function sevChip(sev) {
    const cls = sev === "warn" ? "bad" : sev === "notice" ? "warn" : "info";
    return `<span class="chip ${cls}">${sev}</span>`;
  }

  function renderPanels() {
    const body = $("an-body");
    if (!data) {
      body.innerHTML = `<p class="hint">${loading ? "Analyzing replay…" : "Load a replay to analyze it."}</p>`;
      return;
    }
    const a = data.main;
    const vcls = a.verdict === "clean" ? "ok" : a.verdict === "notice" ? "warn" : "bad";

    let html = "";
    html += `<div class="an-verdict"><span class="chip ${vcls}">${a.verdict === "clean" ? "no integrity signals" : a.verdict === "notice" ? "signals — take a look" : "strong signals"}</span>
      <span class="hint">${esc2(data.player)} · ${a.meta.hits}/${a.meta.hits + a.meta.misses} hits</span></div>`;

    html += card(
      "Overlays",
      `<div class="an-toggles">
        ${["path", "raw", "markers", "hitboxes", "heatmap"]
          .map(
            (k) =>
              `<label class="an-tog"><input type="checkbox" data-ov="${k}" ${overlays[k] ? "checked" : ""}>
               ${{ path: "Cursor path", raw: "Raw cursor", markers: "Frame markers", hitboxes: "Note hitboxes", heatmap: "Heatmap" }[k]}</label>`,
          )
          .join("")}
      </div>
      <label class="hint an-slider">Path window <input type="range" id="an-window" min="200" max="3000" step="100" value="${pathWindow}">
      <span id="an-window-val">${(pathWindow / 1000).toFixed(1)}s</span></label>`,
    );

    const c = a.cursor;
    html += card(
      "Cursor",
      kv("Speed now", `<span id="an-live-speed">–</span>`) +
        kv("Average / p95", `${fmt1(c.avg_speed)} / ${fmt1(c.p95_speed)} cells/s`) +
        kv("Max", `<a class="an-jump" data-t="${c.max_speed.t}">${fmt1(c.max_speed.v)} cells/s @ ${fmtMs(c.max_speed.t)}</a>`) +
        kv("Max accel", `<a class="an-jump" data-t="${c.max_accel.t}">${fmt1(c.max_accel.v)} cells/s²</a>`) +
        kv("Path / optimal", `${fmt1(c.total_path_cells)} / ${fmt1(c.optimal_path_cells)} cells`) +
        kv("Efficiency", `${fmt1(c.efficiency_pct)}%`, "Shortest possible route through all notes vs. what the cursor actually travelled") +
        kv("Moving", `${fmt1(c.moving_pct)}% of the time`),
    );

    html += card("Speed over time", `<canvas id="an-speed" class="an-graph"></canvas>`);

    html += card(
      "Aim",
      `<canvas id="an-scatter" class="an-scatter"></canvas>` +
        kv("Bias", `${fmt2(a.direction_bias.dx)} / ${fmt2(a.direction_bias.dy)} cells (${a.direction_bias.dx > 0 ? "right" : "left"}/${a.direction_bias.dy > 0 ? "high" : "low"})`, "Mean hit offset from note centres — the arrow in the scatter") +
        kv("Snap vs flow", `${Math.round(a.snap_flow.snap_pct)}% / ${Math.round(a.snap_flow.flow_pct)}%`, "Snappy jumps vs. smooth continuous aim between notes") +
        kv("Overshoot", `${fmt1(a.overshoot.rate_pct)}% of approaches, avg ${fmt2(a.overshoot.avg_cells)} cells${a.overshoot.worst ? ` — <a class="an-jump" data-t="${a.overshoot.worst.t}">worst</a>` : ""}`) +
        kv("Micro-jitter", `${fmt2(a.jitter.rms_cells * 100)} cells·10⁻²`, "RMS deviation from the smoothed path while moving"),
    );

    const tm = a.timing;
    html += card(
      "Timing",
      kv("Unstable rate", `${fmt1(tm.ur)}`) +
        kv("Mean / median", `${fmt1(tm.mean_err_ms)} / ${fmt1(tm.median_err_ms)} ms`) +
        kv("Drift", `${tm.drift_ms_per_min >= 0 ? "+" : ""}${fmt1(tm.drift_ms_per_min)} ms/min`, "Positive = hitting later as the run goes on") +
        `<canvas id="an-timing-hist" class="an-graph"></canvas>
         <div class="an-halves"><span></span><span>1st half</span><span>2nd half</span>
           <span>Acc</span><span>${fmt1(tm.first_half.acc_pct)}%</span><span>${fmt1(tm.second_half.acc_pct)}%</span>
           <span>UR</span><span>${fmt1(tm.first_half.ur)}</span><span>${fmt1(tm.second_half.ur)}</span>
           <span>Speed</span><span>${fmt1(tm.first_half.avg_speed)}</span><span>${fmt1(tm.second_half.avg_speed)}</span>
         </div>`,
    );

    html += card("Consistency (rolling UR)", `<canvas id="an-rollur" class="an-graph"></canvas>`);

    const ms = a.misses;
    const worstMisses = a.notes
      .filter((n) => !n.hit && n.near_dist != null)
      .sort((x, y) => x.near_dist - y.near_dist)
      .slice(0, 8);
    html += card(
      "Misses",
      kv("Total", `${ms.count}`) +
        (ms.count
          ? kv("Barely / lost", `${Math.round(ms.barely_pct)}% / ${Math.round(ms.lost_pct)}%`, "Barely: cursor came within 0.65 cells · lost: never within 1.2 cells") +
            kv("Context", `${ms.on_fast_jumps} on fast jumps · ${ms.on_streams} in streams · ${ms.other} other`) +
            `<div class="an-list">${worstMisses
              .map(
                (n) =>
                  `<a class="an-jump" data-t="${n.t}" data-note="${n.i}">${fmtMs(n.t)} — ${fmt2(n.near_dist)} cells away</a>`,
              )
              .join("")}</div>`
          : ""),
    );

    const sec = [...a.sections].sort((x, y) => x.acc_pct - y.acc_pct).slice(0, 6);
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

    html += card("Note inspector", `<div id="an-inspector"><p class="hint">Click a note box in the preview (or a miss above).</p></div>`);

    if (data.ghost) {
      const g = data.ghost;
      html += card(
        `Ghost race — <span class="an-main">${esc2(data.player)}</span> vs <span class="an-ghost">${esc2(data.ghost_player || "ghost")}</span>`,
        `<div class="an-halves an-vs"><span></span><span class="an-main">${esc2(data.player)}</span><span class="an-ghost">${esc2(data.ghost_player || "ghost")}</span>
          <span>Acc</span><span>${fmt1((a.meta.hits / Math.max(1, a.meta.hits + a.meta.misses)) * 100)}%</span><span>${fmt1((g.meta.hits / Math.max(1, g.meta.hits + g.meta.misses)) * 100)}%</span>
          <span>UR</span><span>${fmt1(a.timing.ur)}</span><span>${fmt1(g.timing.ur)}</span>
          <span>Avg speed</span><span>${fmt1(a.cursor.avg_speed)}</span><span>${fmt1(g.cursor.avg_speed)}</span>
          <span>Efficiency</span><span>${fmt1(a.cursor.efficiency_pct)}%</span><span>${fmt1(g.cursor.efficiency_pct)}%</span>
          <span>Misses</span><span>${a.meta.misses}</span><span>${g.meta.misses}</span>
        </div>
        <div class="an-title" style="margin-top:8px">Cursor distance between runs</div>
        <canvas id="an-ghostdist" class="an-graph"></canvas>`,
      );
    }

    html += card(
      "Integrity signals",
      (a.signals.length
        ? a.signals
            .map(
              (s) => `<div class="an-signal">${sevChip(s.severity)}<div><b>${esc2(s.title)}</b>
                <p class="hint">${esc2(s.detail)}</p>
                ${s.times.length ? `<div class="an-list an-inline">${s.times.map((t) => `<a class="an-jump" data-t="${t}">${fmtMs(t)}</a>`).join("")}</div>` : ""}
              </div></div>`,
            )
            .join("")
        : `<p class="hint">Nothing unusual found in this replay's data.</p>`) +
        `<p class="hint an-foot">Signals are hints derived from the recording — context, not verdicts.</p>`,
    );

    html += card(
      "Export",
      `<div class="an-actions">
        <button class="btn small" id="an-export-card">Analysis card (PNG)</button>
        <button class="btn small ghost" id="an-export-json">JSON</button>
        <button class="btn small ghost" id="an-export-csv">CSV</button>
      </div>`,
    );

    body.innerHTML = html;
    wirePanels();
    drawGraphs();
    renderInspector();
  }

  function wirePanels() {
    $("an-body").querySelectorAll("input[data-ov]").forEach((cb) => {
      cb.addEventListener("change", () => {
        overlays[cb.dataset.ov] = cb.checked;
        drawOverlay(lastFrameT);
      });
    });
    const win = $("an-window");
    if (win) {
      win.addEventListener("input", () => {
        pathWindow = Number(win.value);
        $("an-window-val").textContent = `${(pathWindow / 1000).toFixed(1)}s`;
        drawOverlay(lastFrameT);
      });
    }
    for (const id of ["an-speed", "an-rollur", "an-ghostdist"]) {
      const cv = $(id);
      if (cv) seriesSeekHandler(cv);
    }
    const sc = $("an-scatter");
    if (sc) {
      sc.addEventListener("click", (e) => {
        // pick nearest hit note in scatter space
        if (!data) return;
        const r = sc.getBoundingClientRect();
        const scale = Math.min(r.width, r.height) / 2.6;
        const ox = (e.clientX - r.left - r.width / 2) / scale;
        const oy = -(e.clientY - r.top - r.height / 2) / scale;
        let best = -1;
        let bd = 0.12;
        for (const n of data.main.notes) {
          if (n.off_x == null) continue;
          const d = Math.hypot(n.off_x - ox, n.off_y - oy);
          if (d < bd) {
            bd = d;
            best = n.i;
          }
        }
        const picked = best >= 0 ? noteById(best) : null;
        if (picked) {
          selNote = best;
          renderInspector();
          seek(picked.t);
        }
      });
      sc.classList.add("an-clickable");
    }
    $("an-export-json")?.addEventListener("click", () => exportJson());
    $("an-export-csv")?.addEventListener("click", () => exportCsv());
    $("an-export-card")?.addEventListener("click", () => exportCard());
  }

  function drawGraphs() {
    if (!data) return;
    const a = data.main;
    const sp = $("an-speed");
    if (sp) drawSeries(sp, a.speed_series, { marker: { t: a.cursor.max_speed.t, v: a.cursor.max_speed.v } });
    const th = $("an-timing-hist");
    if (th) {
      drawHist(th, a.timing.hist, {
        zeroAt: -a.timing.hist_start_ms / (a.timing.hist.length * a.timing.hist_bin_ms),
        left: "early",
        right: "late",
      });
    }
    const ru = $("an-rollur");
    if (ru) drawSeries(ru, a.rolling_ur);
    const sc = $("an-scatter");
    if (sc) drawScatter(sc, a.notes);
    const gd = $("an-ghostdist");
    if (gd && data.ghost_distance) drawSeries(gd, data.ghost_distance, { color: GHOST });
    // live speed readout at the playhead
    const live = $("an-live-speed");
    if (live) {
      const t = a.speed_series.t;
      const i = Math.max(0, lastIndexLE(t, currentMs));
      live.textContent = t.length ? `${fmt1(a.speed_series.v[i])} cells/s` : "–";
    }
  }

  function renderInspector() {
    const el = $("an-inspector");
    if (!el) return;
    const n = selNote >= 0 ? noteById(selNote) : null;
    if (!n) {
      el.innerHTML = `<p class="hint">Click a note box in the preview (or a miss above).</p>`;
      return;
    }
    el.innerHTML =
      kv("Note", `#${n.i + 1} @ <a class="an-jump" data-t="${n.t}">${fmtMs(n.t)}</a>`) +
      kv("Result", n.hit ? `<span class="an-ok">hit</span>` : `<span class="an-bad">miss</span>`) +
      (n.hit
        ? kv("Timing", `${n.err_ms >= 0 ? "+" : ""}${fmt1(n.err_ms)} ms (${n.err_ms >= 0 ? "late" : "early"})`) +
          kv("Hit offset", `${fmt2(n.dist)} cells from centre (${fmt2(n.off_x)}, ${fmt2(n.off_y)})`)
        : kv("Closest approach", n.near_dist != null ? `${fmt2(n.near_dist)} cells` : "–")) +
      kv("Approach speed", `${fmt1(n.approach_v)} cells/s`);
  }

  // ------------------------------------------------------------ exports

  async function pickSave(defName, ext, name) {
    return dialog.save({
      defaultPath: defName,
      filters: [{ name, extensions: [ext] }],
    });
  }

  function exportBase() {
    const raw = `${data?.player || "player"} - ${status?.map?.song_name || "run"} - analysis`;
    return raw.replace(/[\\/:*?"<>|]/g, "-");
  }

  async function exportJson() {
    if (!data) return;
    const p = await pickSave(`${exportBase()}.json`, "json", "JSON");
    if (!p) return;
    // frames are huge and reproducible — exports carry the findings
    const strip = ({ frames, ...rest }) => rest;
    const out = {
      ...data,
      main: strip(data.main),
      ghost: data.ghost ? strip(data.ghost) : null,
    };
    await call(() => invoke("save_text_file", { path: p, contents: JSON.stringify(out, null, 2) }));
    loadNote(`Analysis JSON saved — ${p}`);
  }

  async function exportCsv() {
    if (!data) return;
    const p = await pickSave(`${exportBase()}.csv`, "csv", "CSV");
    if (!p) return;
    const rows = [
      "side,note,time_ms,grid_x,grid_y,hit,hit_ms,err_ms,off_x,off_y,dist_cells,near_dist_cells,approach_speed",
    ];
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
    await call(() => invoke("save_text_file", { path: p, contents: rows.join("\n") }));
    loadNote(`Analysis CSV saved — ${p}`);
  }

  async function exportCard() {
    if (!data) return;
    const p = await pickSave(`${exportBase()}.png`, "png", "PNG image");
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
    x.fillText(
      `${a.meta.hits}/${a.meta.hits + a.meta.misses} hits · analyzed with rhythr`,
      48,
      102,
    );
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
    // timing histogram strip
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
      const bh = (cnt / maxC) * hh;
      if (cnt > 0) x.fillRect(48 + i * bw, hy + hh - bh, Math.max(1, bw - 2), bh);
    });
    x.strokeStyle = "rgba(255,255,255,0.6)";
    const zx = 48 + (-a.timing.hist_start_ms / (a.timing.hist.length * a.timing.hist_bin_ms)) * hw;
    x.beginPath();
    x.moveTo(zx, hy);
    x.lineTo(zx, hy + hh);
    x.stroke();
    x.fillStyle = a.verdict === "clean" ? OK : a.verdict === "notice" ? WARN : DANGER;
    x.font = "600 16px system-ui";
    x.fillText(
      a.verdict === "clean" ? "● no integrity signals" : `● integrity: ${a.verdict}`,
      48,
      600,
    );
    await call(() => invoke("save_data_url", { path: p, dataUrl: c.toDataURL("image/png") }));
    loadNote(`Analysis card saved — ${p}`);
  }

  // ------------------------------------------------------------ lifecycle

  const sourceKey = () =>
    `${status?.replay?.path}|${status?.ghost?.path || status?.ghost?.file_name || ""}|${status?.map?.path}`;

  async function ensureData() {
    const key = sourceKey();
    if (data && key === dataKey) return;
    if (loading) return;
    loading = true;
    data = null;
    selNote = -1;
    renderPanels();
    let fresh = null;
    try {
      fresh = await invoke("analysis_data");
    } catch (e) {
      loading = false;
      $("an-body").innerHTML = `<p class="hint">Analysis failed: ${esc2(String(e))}</p>`;
      return;
    }
    loading = false;
    // The sources may have changed while we were computing — this result
    // belongs to the old pair, throw it away and start over.
    if (sourceKey() !== key) {
      ensureData();
      return;
    }
    data = fresh;
    dataKey = key;
    heatCanvases = {
      main: buildHeatCanvas(data.main.heatmap, ACCENT),
      ghost: data.ghost ? buildHeatCanvas(data.ghost.heatmap, GHOST) : null,
    };
    renderPanels();
    drawOverlay(lastFrameT);
  }

  window.rhythrAnalyze = {
    active: () => entered,
    enter() {
      entered = true;
      if (typeof hudEditOn !== "undefined" && hudEditOn) $("btn-edit-hud").click();
      $("play-row").hidden = false;
      updateTimeLabel();
      ensureData();
      schedulePreview();
    },
    leave() {
      setPlaying(false);
      entered = false;
      $("play-row").hidden = true;
      $("analyze-overlay").hidden = true;
    },
    onStatus() {
      const key = sourceKey();
      if (key !== dataKey) {
        data = null;
        if (entered) {
          if (!st?.replay || !st?.map) {
            window.rhythrAnalyze.leave();
            document.querySelector('.tab[data-tab="output"]')?.click();
          } else {
            ensureData();
          }
        }
      }
    },
    onResize() {
      if (!entered) return;
      drawOverlay(lastFrameT);
      drawGraphs();
    },
    onFrame(frameDto, t) {
      lastFrame = frameDto;
      lastFrameT = t;
      updateTimeLabel();
      // wait for the img to actually show the new frame before measuring
      requestAnimationFrame(() => {
        drawOverlay(t);
        drawGraphs();
      });
    },
  };

  // ------------------------------------------------------------ input

  document.addEventListener("DOMContentLoaded", () => {
    $("an-body").addEventListener("click", (e) => {
      const j = e.target.closest("a.an-jump");
      if (!j) return;
      if (j.dataset.note != null) {
        selNote = Number(j.dataset.note);
        renderInspector();
      }
      seek(Number(j.dataset.t));
    });
    $("btn-play").addEventListener("click", () => setPlaying(!play.on));
    $("btn-step-back").addEventListener("click", () => stepFrame(-1));
    $("btn-step-fwd").addEventListener("click", () => stepFrame(1));
    $("play-speed").addEventListener("change", () => {
      play.factor = Number($("play-speed").value);
    });
    $("analyze-overlay").addEventListener("click", pickNote);
    document.addEventListener("keydown", (e) => {
      if (!entered || e.ctrlKey || e.metaKey || e.altKey) return;
      const t = document.activeElement;
      const typing =
        t && (t.tagName === "TEXTAREA" || t.isContentEditable || t.tagName === "INPUT" || t.tagName === "SELECT");
      if (typing) return;
      if (e.code === "Space") {
        e.preventDefault();
        setPlaying(!play.on);
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        if (e.shiftKey) seek(currentMs - 1000);
        else stepFrame(-1);
      } else if (e.key === "ArrowRight") {
        e.preventDefault();
        if (e.shiftKey) seek(currentMs + 1000);
        else stepFrame(1);
      }
    });
  });
})();
