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
  quality: "auto",
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
let shownMs = -1; // song time currently on screen
let fps = 0;
const play = { on: false, factor: 1, last: 0, gen: 0, startWall: 0, startMs: 0, k: 0 };

// Frames travel through a custom URI scheme: the webview decodes PNG
// bytes natively instead of parsing a base64 string out of an IPC reply.
const frameUrl = (t) => {
  const q = `t=${Math.round(t)}`;
  const win = navigator.userAgent.includes("Windows");
  return win ? `http://rhframe.localhost/f.png?${q}` : `rhframe://localhost/f.png?${q}`;
};
let heatCanvases = { main: null, ghost: null };
let currentBitmap = null;
let lastRenderH = 0;
// Auto mode measures the first second of playback and drops the render
// scale once if the machine cannot hold ~55 fps. It never changes back
// mid-playback (a resize rebuilds the GPU pipeline).
let autoScale = 100;
let playbackScale = false;
let autoNextCheck = 1200;
let loopFps = 0;
let lastTick = 0;
let lastPrefetchK = -1e9;
// Which playback engine this machine can actually use. "video" plays a
// rendered segment (smooth at any size); "stream" pushes single frames
// (works everywhere, costs more per pixel). Chosen automatically.
let engine = "video";
// Short trace of what the playback engine did last — visible under
// View -> Diagnostics, so a problem report says where it went wrong.
const trace = [];
function tr(s) {
  trace.push(s);
  if (trace.length > 6) trace.shift();
}
let loopFails = 0;

// Playback runs on a real video the renderer produced: the webview
// decodes it in hardware, which is the only way to get a genuinely
// smooth picture at full window size. Stills (paused, stepping,
// scrubbing) keep using the on-demand renderer, which is exact and
// instant.
const seg = {
  token: 0,
  startMs: 0,
  spanMs: 0,
  outFps: 60,
  ready: false,
  preparing: false,
  wantPlay: false,
  spanSetting: 12000, // song ms per segment
};

const videoUrl = (token) => {
  const q = `v=${token}`;
  return navigator.userAgent.includes("Windows")
    ? `http://rhvideo.localhost/seg.mp4?${q}`
    : `rhvideo://localhost/seg.mp4?${q}`;
};

/// Frames per SONG second the renderer must produce so that playing the
/// result at `speed` still shows ~60 frames a second: at 1x that is 60,
/// at 0.25x it is 240. Every displayed frame is then a real rendered
/// frame — no duplicates, no stutter.
function segmentFps() {
  return Math.round(clamp(60 / clamp(play.factor, 0.0625, 4), 60, 480));
}

/// What the video element runs at. Its duration equals the song span, so
/// the rate IS the playback speed.
function videoRate() {
  return clamp(play.factor, 0.0625, 4);
}

/// How much song time to prepare: enough to be useful, capped so the
/// wait stays short at slow speeds (they need far more frames).
function segmentSpan() {
  const frames = 900; // ~10 s of rendering on a mid-range GPU
  return clamp((frames / segmentFps()) * 1000, 1500, seg.spanSetting);
}

function segmentCovers(t) {
  return seg.ready && t >= seg.startMs - 1 && t < seg.startMs + seg.spanMs - 60;
}

function showPrep(on, text) {
  $("an-prep").hidden = !on;
  if (text) $("an-prep-text").textContent = text;
  if (!on) $("an-prep-fill").style.width = "0%";
}

function requestSegment(fromMs, autoPlay) {
  seg.ready = false;
  seg.preparing = true;
  seg.wantPlay = autoPlay;
  showPrep(true, `Preparing ${(segmentSpan() / 1000).toFixed(1)} s at ${play.factor}×…`);
  invoke("prepare_segment", {
    startMs: Math.round(fromMs),
    spanMs: segmentSpan(),
    height: lastRenderH || 720,
    outFps: segmentFps(),
  })
    .then((token) => {
      seg.token = token;
    })
    .catch((e) => {
      seg.preparing = false;
      showPrep(false);
      msg(`Could not prepare playback: ${e}`);
    });
}

// Overlay geometry for the whole segment, on the video's own frame grid
// so the boxes never lag the picture. Fetched once per segment; nothing
// crosses the IPC boundary during playback.
let segGeo = { times: [], list: [] };

async function primeSegmentGeometry() {
  const stepMs = 1000 / seg.outFps;
  const count = Math.min(1200, Math.round(seg.spanMs / stepMs));
  const times = [];
  for (let i = 0; i < count; i++) times.push(seg.startMs + i * stepMs);
  segGeo = { times: [], list: [] };
  for (let i = 0; i < times.length; i += 300) {
    const chunk = times.slice(i, i + 300);
    try {
      const got = await invoke("frame_geometry_batch", { times: chunk });
      segGeo.times.push(...chunk);
      segGeo.list.push(...got);
    } catch (e) {
      break;
    }
  }
}

function geometryNear(t) {
  const a = segGeo.times;
  if (!a.length) return null;
  let lo = 0;
  let hi = a.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (a[mid] < t) lo = mid + 1;
    else hi = mid;
  }
  const cand = lo > 0 && Math.abs(a[lo - 1] - t) < Math.abs(a[lo] - t) ? lo - 1 : lo;
  return segGeo.list[cand] || null;
}

function dropSegment() {
  seg.ready = false;
  seg.preparing = false;
  seg.wantPlay = false;
  segGeo = { times: [], list: [] };
  showPrep(false);
  const v = $("an-video");
  v.pause();
  v.removeAttribute("src");
  v.load();
  if (segObjectUrl) {
    URL.revokeObjectURL(segObjectUrl);
    segObjectUrl = null;
  }
  invoke("cancel_segment").catch(() => {});
}

// The video holds `span` song-seconds at `outFps` frames each, so its
// duration is exactly the song span: video seconds ARE song seconds.
async function startVideoPlayback() {
  const v = $("an-video");
  // Wait for real data: some webviews accept the source and then never
  // decode anything, which would look like a frozen picture.
  if (v.readyState < 2) {
    const ok = await new Promise((resolve) => {
      let done = false;
      const finish = (val) => {
        if (done) return;
        done = true;
        clearTimeout(t);
        v.removeEventListener("loadeddata", onOk);
        v.removeEventListener("error", onErr);
        resolve(val);
      };
      const onOk = () => finish(true);
      const onErr = () => finish(false);
      const t = setTimeout(() => finish(false), 5000);
      v.addEventListener("loadeddata", onOk);
      v.addEventListener("error", onErr);
    });
    if (!ok) {
      fallbackToStreaming(v.error ? `decoder error ${v.error.code}` : "no data");
      return;
    }
  }
  tr("video play");
  document.body.classList.add("an-playing");
  v.hidden = false;
  $("an-canvas").style.left = "";
  v.currentTime = clamp((currentMs - seg.startMs) / 1000, 0, Math.max(0, (seg.spanMs - 40) / 1000));
  v.playbackRate = videoRate();
  // Do not await play(): some engines never resolve that promise even
  // though playback starts, which would leave the overlay loop dead.
  v.play().catch((e) => fallbackToStreaming(String(e)));
  // Watchdog: an engine can accept the source, decode a frame and still
  // never advance. Only real movement counts as working playback.
  const t0 = v.currentTime;
  clearTimeout(watchdog);
  watchdog = setTimeout(() => {
    if (play.on && engine === "video" && !v.hidden && v.currentTime <= t0 + 0.01) {
      fallbackToStreaming("video did not advance");
    }
  }, 1500);
  videoTick();
}

/// The overlay follows the video's own clock — one callback per decoded
/// frame, so it can never drift from the picture.
function videoTick() {
  const v = $("an-video");
  if (!play.on || v.hidden) return;
  currentMs = seg.startMs + v.currentTime * 1000;
  updateTime();
  drawScrub();
  syncOverlayToVideo();
  const geo = geometryNear(currentMs);
  if (geo) lastFrame = geo;
  const cv = $("an-canvas");
  cv.getContext("2d").clearRect(0, 0, cv.width, cv.height);
  drawOverlay();
  refreshLive();
  // Prepare the next stretch while this one still plays.
  if (!seg.preparing && v.duration && v.currentTime > v.duration * 0.6) {
    requestSegment(seg.startMs + seg.spanMs, false);
  }
  if (v.ended) {
    if (seg.ready && seg.startMs > currentMs - 10) {
      // The next stretch arrived — carry straight on.
      startVideoPlayback();
      return;
    }
    if (!seg.preparing) requestSegment(currentMs, true);
    return;
  }
  // Plain rAF: requestVideoFrameCallback exists on some engines but
  // never fires there, which would stop the overlay after one frame.
  requestAnimationFrame(videoTick);
}

/// The overlay canvas must sit exactly on the video picture.
function syncOverlayToVideo() {
  const v = $("an-video");
  const cv = $("an-canvas");
  const r = v.getBoundingClientRect();
  const s = $("stage").getBoundingClientRect();
  cv.style.left = `${r.left - s.left}px`;
  cv.style.top = `${r.top - s.top}px`;
  cv.style.width = `${r.width}px`;
  cv.style.height = `${r.height}px`;
  const w = v.videoWidth || 1;
  const h = v.videoHeight || 1;
  if (cv.width !== w || cv.height !== h) {
    cv.width = w;
    cv.height = h;
  }
}
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

// Geometry for upcoming frames, fetched in batches so playback needs no
// IPC round trip per frame.
const geoCache = new Map();
let geoPending = null;

async function geometryFor(t) {
  const key = Math.round(t);
  const hit = geoCache.get(key);
  if (hit) return hit;
  // Ask for the time the entry is keyed by, or the cached geometry would
  // belong to a slightly different moment than the frame.
  const g = await invoke("frame_geometry", { timeMs: key });
  geoCache.set(key, g);
  return g;
}

function primeGeometry(fromMs, step, count) {
  if (geoPending) return;
  const times = [];
  for (let k = 0; k < count; k++) {
    const key = Math.round(fromMs + step * k);
    if (!geoCache.has(key)) times.push(key);
  }
  if (!times.length) return;
  geoPending = invoke("frame_geometry_batch", { times })
    .then((list) => {
      list.forEach((g, i) => geoCache.set(times[i], g));
      // Keep the map from growing without bound over a long session.
      if (geoCache.size > 600) {
        const keys = [...geoCache.keys()].sort((a, b) => Math.abs(a - fromMs) - Math.abs(b - fromMs));
        keys.slice(400).forEach((k) => geoCache.delete(k));
      }
    })
    .catch(() => {})
    .finally(() => {
      geoPending = null;
    });
}

// Frame transport, in order of preference. Any of them can fail on a
// platform we cannot test here, so the window degrades instead of
// freezing: fetch -> plain <img> (no CORS involved) -> the IPC data URL
// that the main window has always used.
let transport = "fetch";
let transportNote = "";
let frameFails = 0;

function withTimeout(promise, ms, onTimeout) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      onTimeout?.();
      reject(new Error(`timed out after ${ms} ms`));
    }, ms);
    promise.then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        clearTimeout(timer);
        reject(e);
      },
    );
  });
}

async function bitmapFromUrl(url) {
  const img = new Image();
  img.src = url;
  await img.decode();
  // Tainted canvases are fine here: the frame canvas is never read back.
  return createImageBitmap(img);
}

async function loadBitmap(t) {
  if (transport === "fetch") {
    const ctl = new AbortController();
    const res = await withTimeout(fetch(frameUrl(t), { signal: ctl.signal }), 5000, () =>
      ctl.abort(),
    );
    if (!res.ok) throw new Error(`frame ${res.status}: ${await res.text()}`);
    return createImageBitmap(await res.blob());
  }
  if (transport === "img") {
    return withTimeout(bitmapFromUrl(frameUrl(t)), 5000);
  }
  const url = await withTimeout(invoke("preview", { timeMs: t }), 8000);
  return bitmapFromUrl(url);
}

/// Fetches a frame and decodes it off the main thread, degrading the
/// transport if this platform cannot serve the current one.
async function fetchBitmap(t) {
  try {
    const bmp = await loadBitmap(t);
    frameFails = 0;
    return bmp;
  } catch (e) {
    frameFails++;
    if (frameFails >= 2 && transport !== "ipc") {
      transport = transport === "fetch" ? "img" : "ipc";
      transportNote = `Frame channel fell back to "${transport}" — ${e}`;
      frameFails = 0;
      msgFlash(transportNote);
      return loadBitmap(t);
    }
    throw e;
  }
}

let frameReq = 0;

async function showFrame(t) {
  const req = ++frameReq;
  const [bmp, geo] = await Promise.all([fetchBitmap(t), geometryFor(t)]);
  // A slower earlier request must not paint over a newer frame.
  if (req !== frameReq) {
    bmp.close?.();
    return;
  }
  lastFrame = geo;
  currentBitmap?.close?.();
  currentBitmap = bmp;
  shownMs = t;
  msg("");
  updateTime();
  drawFrame();
  refreshLive();
}

async function runPreview() {
  if (busy || !wanted) return;
  wanted = false;
  busy = true;
  try {
    await showFrame(currentMs);
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
  if (play.on && segmentCovers(currentMs) && !$("an-video").hidden) {
    // Inside the prepared stretch: instant, no re-render.
    $("an-video").currentTime = (currentMs - seg.startMs) / 1000;
    return;
  }
  if (play.on) {
    requestSegment(currentMs, true);
    return;
  }
  cancelPrefetch();
  schedulePreview();
}

function updateTime() {
  $("an-time").textContent = fmtMsFull(currentMs) + (fps ? `  [${Math.round(fps)} fps]` : "");
  $("an-total").textContent = fmtMs(runEnd());
}

// ------------------------------------------------------------ overlay

/// Fits the canvas into the stage at the frame's aspect ratio, in real
/// device pixels — the backend renders at exactly this size, so nothing
/// is scaled and everything stays sharp.
function syncCanvas() {
  const cv = $("an-canvas");
  cv.style.left = "";
  cv.style.top = "";
  const stage = $("stage").getBoundingClientRect();
  if (!currentBitmap || !stage.width || !stage.height) return false;
  const ar = currentBitmap.width / currentBitmap.height;
  let cssW = stage.width;
  let cssH = cssW / ar;
  if (cssH > stage.height) {
    cssH = stage.height;
    cssW = cssH * ar;
  }
  cv.style.width = `${Math.floor(cssW)}px`;
  cv.style.height = `${Math.floor(cssH)}px`;
  const w = currentBitmap.width;
  const h = currentBitmap.height;
  if (cv.width !== w || cv.height !== h) {
    cv.width = w;
    cv.height = h;
  }
  return true;
}

function drawFrame() {
  if (play.on && !$("an-video").hidden) {
    // Video mode: the picture comes from the video element, the canvas
    // carries the overlay only.
    const cv = $("an-canvas");
    cv.getContext("2d").clearRect(0, 0, cv.width, cv.height);
    drawOverlay();
    return;
  }
  if (!syncCanvas()) return;
  const ctx = $("an-canvas").getContext("2d");
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.drawImage(currentBitmap, 0, 0);
  drawOverlay();
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
  const cv = $("an-canvas");
  const ctx = cv.getContext("2d");
  if (!data || !lastFrame || !currentBitmap) return;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  // Geometry arrives in the frame's own pixels; the canvas IS that size,
  // except when a resize outran the renderer.
  ctx.scale(cv.width / lastFrame.w, cv.height / lastFrame.h);
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
  const cv = $("an-canvas");
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
  drawFrame(); // the merged canvas has no clear of its own
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

/// Song ms per displayed frame at the current speed (60 fps grid).
function frameStep() {
  const sp = clamp(status?.replay?.speed || 1, 0.25, 3);
  return (1000 / 60) * play.factor * sp;
}

/// Asks the backend to render at the stage's real pixel size (capped),
/// so no frame is scaled and playback stays cheap.
/// Full size for still frames, the learned scale while playing.
function scalePct() {
  if (opt.quality !== "auto") return Number(opt.quality);
  return playbackScale ? autoScale : 100;
}

async function syncRenderSize() {
  const stage = $("stage").getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  // Capped: the preview pipeline is shared with the main window, and a
  // 4K stage would make ITS preview expensive too.
  const want = Math.round(clamp(stage.height * dpr * (scalePct() / 100), 400, 1600));
  if (want === lastRenderH) return;
  lastRenderH = want;
  try {
    status = await invoke("set_preview_quality", { height: want });
    geoCache.clear();
    schedulePreview();
  } catch (e) {
    /* keep the current size */
  }
}

// The backend keys frames by round(from + step*k) — the frontend must use
// the exact same base and step or every request misses the cache.
function prefetch(fromMs) {
  invoke("prefetch_frames", { fromMs, stepMs: frameStep(), count: 45 }).catch(() => {});
}

/// Stops the background renderer — it must not keep working for a
/// playhead that moved on.
function cancelPrefetch() {
  invoke("cancel_prefetch").catch(() => {});
}

let stillTimer = null;
let watchdog = null;

function setPlaying(on) {
  if (on && (!data || !status?.replay)) return;
  play.on = on;
  play.gen++;
  $("an-play").textContent = on ? "⏸" : "▶";
  document.body.classList.toggle("an-immersive", on && opt.immersive);
  clearTimeout(stillTimer);
  if (!on) {
    cancelPrefetch();
    const v = $("an-video");
    v.pause();
    v.hidden = true;
    document.body.classList.remove("an-playing");
    showPrep(false);
    // Back to the exact, full-resolution still.
    if (opt.quality === "auto" && autoScale < 100) {
      stillTimer = setTimeout(() => {
        playbackScale = false;
        lastRenderH = 0;
        syncRenderSize();
      }, 300);
    }
    schedulePreview();
    return;
  }
  if (currentMs >= runEnd() - 1) currentMs = 0;
  if (engine === "video") {
    if (segmentCovers(currentMs)) startVideoPlayback();
    else requestSegment(currentMs, true);
    return;
  }
  startStreaming();
}

/// Frame-by-frame engine: works on any platform, but every pixel travels
/// through the image decoder, so it is the fallback.
function startStreaming() {
  tr("stream start");
  play.startWall = performance.now();
  play.startMs = Math.round(currentMs);
  play.k = -1;
  fps = 0;
  loopFps = 0;
  lastTick = 0;
  lastPrefetchK = -1e9;
  loopFails = 0;
  autoNextCheck = 1200; // first verdict after ~1.2 s of playback
  prefetch(play.startMs);
  primeGeometry(play.startMs, frameStep(), 45);
  pump(play.gen);
}

/// The video element never became ready — this platform cannot play the
/// prepared segment, so switch to frames for the rest of the session.
function fallbackToStreaming(why) {
  tr(`fallback: ${why}`);
  clearTimeout(watchdog);
  engine = "stream";
  transportNote = `Segment playback unavailable (${why}) — using single frames.`;
  dropSegment();
  const v = $("an-video");
  v.hidden = true;
  document.body.classList.remove("an-playing");
  if (play.on) startStreaming();
}

/// A prepared segment landed: load it and (if we were waiting) play.
let segObjectUrl = null;

async function onSegmentReady(info) {
  tr(`ready t=${info.token}`);
  if (info.token !== seg.token) return;
  seg.startMs = info.startMs;
  seg.spanMs = info.spanMs;
  seg.outFps = info.outFps;
  const v = $("an-video");
  // The media decoder cannot reach a custom URI scheme on every platform,
  // so the segment is handed over as an in-memory object instead — a few
  // MB, and it plays identically everywhere.
  try {
    const res = await fetch(videoUrl(info.token));
    if (!res.ok) throw new Error(`segment ${res.status}`);
    const blob = await res.blob();
    if (info.token !== seg.token) return;
    if (segObjectUrl) URL.revokeObjectURL(segObjectUrl);
    segObjectUrl = URL.createObjectURL(blob);
    tr(`blob ${(blob.size / 1048576).toFixed(1)}MB`);
    v.src = segObjectUrl;
    v.load();
  } catch (e) {
    seg.preparing = false;
    showPrep(false);
    setPlaying(false);
    msg(`Could not load the prepared video: ${e}`);
    return;
  }
  seg.ready = true;
  seg.preparing = false;
  await primeSegmentGeometry();
  showPrep(false);
  if (seg.wantPlay && play.on) {
    seg.wantPlay = false;
    startVideoPlayback();
  } else if (seg.wantPlay) {
    seg.wantPlay = false;
  }
}

async function pump(gen) {
  if (gen !== play.gen || !play.on) return;
  if (renderBusy || status?.rendering) {
    setPlaying(false);
    msg("Paused — a video render is using the renderer.");
    return;
  }
  const step = frameStep();
  // Time comes from the wall clock, quantized to the frame grid the
  // prefetcher renders — so playback keeps real time even if a frame is
  // slow, and every request hits a ready image.
  const elapsed = performance.now() - play.startWall;
  // ONLY the wall clock advances the song. Flooring this at "one more per
  // iteration" would make a 144 Hz display play 2.4x too fast.
  const kw = Math.round(
    (elapsed * play.factor * clamp(status?.replay?.speed || 1, 0.25, 3)) / step,
  );
  if (kw <= play.k) {
    // Same grid point — wait for the next one instead of re-rendering it.
    requestAnimationFrame(() => pump(gen));
    return;
  }
  // A window that was hidden (rAF frozen) or a machine that fell far
  // behind must resync rather than jump seconds ahead.
  if (kw - play.k > 45) {
    play.startWall = performance.now();
    play.startMs = Math.round(currentMs);
    play.k = -1;
    lastPrefetchK = -1e9;
    requestAnimationFrame(() => pump(gen));
    return;
  }
  const k = kw;
  play.k = k;
  currentMs = Math.min(play.startMs + k * step, runEnd());
  updateTime();
  drawScrub();
  const t0 = performance.now();
  try {
    await showFrame(currentMs);
    loopFails = 0;
  } catch (e) {
    // Never spin on a broken frame channel — say what happened and stop.
    if (++loopFails >= 3) {
      setPlaying(false);
      msg(`Playback stopped — ${e}`);
      return;
    }
  }
  const dt = performance.now() - t0;
  fps = fps ? fps * 0.9 + (1000 / Math.max(1, dt)) * 0.1 : 1000 / Math.max(1, dt);
  // Displayed frames per second of wall clock — that is what "smooth"
  // means, and what the Auto scale must judge.
  const now2 = performance.now();
  if (lastTick) {
    const inst = 1000 / Math.max(1, now2 - lastTick);
    loopFps = loopFps ? loopFps * 0.9 + inst * 0.1 : inst;
  }
  lastTick = now2;
  // Auto resolution. Judged by WALL TIME, not by a frame count: at 3 fps
  // a 60-frame threshold would take twenty seconds to notice anything.
  // Cost scales with the pixel count, so the correction is sqrt-based and
  // may repeat until the picture actually moves.
  if (opt.quality === "auto" && play.on && gen === play.gen) {
    const wall = performance.now() - play.startWall;
    if (wall > autoNextCheck && loopFps) {
      autoNextCheck = wall + 2500;
      if (loopFps < 45 && autoScale > 25) {
        const factor = Math.sqrt(clamp(loopFps / 55, 0.16, 1));
        autoScale = clamp(Math.round(autoScale * factor), 25, 100);
        playbackScale = true;
        lastRenderH = 0;
        tr(`auto ${autoScale}% (${Math.round(loopFps)} fps)`);
        syncRenderSize();
        loopFps = 0; // re-measure at the new size
        lastTick = 0;
      }
    }
  }
  // Keep frames and geometry a second ahead of the playhead — on the
  // same grid points the playback will ask for.
  // Distance-based: a slow frame can skip right over any fixed multiple.
  if (k - lastPrefetchK >= 20) {
    lastPrefetchK = k;
    prefetch(play.startMs + (k + 15) * step);
    primeGeometry(play.startMs + k * step, step, 45);
  }
  if (gen !== play.gen) return;
  if (currentMs >= runEnd()) {
    setPlaying(false);
    return;
  }
  requestAnimationFrame(() => pump(gen));
}

function stepFrame(dir) {
  const ft = data?.main?.frames?.t;
  if (!ft?.length) return;
  if (play.on) setPlaying(false);
  let i = lastIndexLE(ft, currentMs);
  if (!(dir < 0 && i >= 0 && ft[i] < currentMs)) i += dir;
  seek(ft[clamp(i, 0, ft.length - 1)]);
}

let speedTimer = null;

function setSpeed(v) {
  const next = clamp(v, 0.0625, 4);
  const changed = next !== play.factor;
  play.factor = next;
  $("an-speed").value = String(Math.round(play.factor * 100));
  $("an-speed-num").value = String(Math.round(play.factor * 100) / 100);
  if (!changed) return;
  // A segment is rendered FOR one speed (the slower it is, the more
  // frames per song second), so a new speed needs a new segment.
  // Debounced: dragging the slider must not start a dozen renders.
  clearTimeout(speedTimer);
  if (play.on) {
    $("an-video").playbackRate = videoRate(); // instant, until the new one lands
    speedTimer = setTimeout(() => {
      if (play.on) requestSegment(currentMs, true);
    }, 500);
  } else {
    seg.ready = false;
  }
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
      "Render resolution",
      `<div class="an-toggles">
        ${[
          ["auto", "Auto"],
          [100, "Native"],
          [70, "70%"],
          [50, "Half"],
        ]
          .map(
            ([q, label]) =>
              `<label class="an-tog"><input type="radio" name="q" data-q="${q}"${String(opt.quality) === String(q) ? " checked" : ""}> ${label}</label>`,
          )
          .join("")}
      </div><p class="hint">Frames render at the window's own pixel size — nothing is scaled, so Native is the sharpest AND the cheapest way to fill the window. Auto starts there and steps down once if playback can't hold ~55 fps${
        fps ? ` (currently ${Math.round(fps)} fps at ${scalePct()}%)` : ""
      }.</p>`,
    );
    html += card(
      "While playing",
      `<label class="an-tog"><input type="checkbox" data-opt="immersive"${opt.immersive ? " checked" : ""}> Hide the controls during playback</label>
       <p class="hint">They come back the moment you move the mouse or pause.</p>`,
    );
    html += card(
      "Diagnostics",
      kv("Build", status?.build || "?") +
        kv("Engine", engine === "video" ? "rendered video" : "single frames") +
        kv("Still frames", transport) +
        kv("Playback", seg.ready ? `${seg.outFps} fps/song-s` : seg.preparing ? "preparing…" : loopFps ? `${Math.round(loopFps)} fps` : "idle") +
        kv("Segment", seg.ready ? `${fmtMs(seg.startMs)} + ${(seg.spanMs / 1000).toFixed(1)}s` : "–") +
        kv("Render size", `${lastRenderH}p at ${scalePct()}%`) +
        `<div class="an-list"><span class="hint">${esc(trace.join(" · ") || "—")}</span></div>` +
        (transportNote ? `<p class="hint">${esc(transportNote)}</p>` : "") +
        `<p class="hint">If playback stalls, this tells us where. "fetch" is the fast path; the window falls back on its own if a platform blocks it.</p>`,
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
      drawFrame();
    });
  });
  const win = body.querySelector("#opt-window");
  if (win) {
    win.addEventListener("input", () => {
      opt.pathWindow = Number(win.value);
      win.nextElementSibling.textContent = `${(opt.pathWindow / 1000).toFixed(2)}s`;
      drawFrame();
    });
  }
  body.querySelectorAll("input[data-q]").forEach((rb) => {
    rb.addEventListener("change", () => {
      opt.quality = rb.dataset.q === "auto" ? "auto" : Number(rb.dataset.q);
      autoScale = 100;
      autoNextCheck = 1200;
      lastRenderH = 0;
      syncRenderSize();
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
  $("an-title").textContent = status?.build ? `${title}   ·   build ${status.build}` : title;
  if (!status?.replay || !status?.map) {
    data = null;
    dataKey = "";
    geoCache.clear();
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
  // Geometry describes THIS replay on THIS field — never reuse it.
  geoCache.clear();
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

// A silent exception used to kill the playback loop without a trace;
// surface it instead.
window.addEventListener("error", (e) => msg(`Error: ${e.message}`));
window.addEventListener("unhandledrejection", (e) => msg(`Error: ${e.reason}`));

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
  $("an-canvas").addEventListener("click", pickNote);
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
  document.addEventListener("visibilitychange", () => {
    if (document.hidden && play.on) setPlaying(false);
  });
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

  // A resized window wants frames at its new pixel size: re-request the
  // render resolution (debounced) and repaint what we have meanwhile.
  let resizeTimer = null;
  new ResizeObserver(() => {
    drawFrame();
    drawScrub();
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(syncRenderSize, 250);
  }).observe($("stage"));

  // The main window drives which replay is loaded; follow its changes.
  listen("sources-changed", () => {
    dropSegment();
    refresh();
  }).catch(() => {});
  listen("segment-progress", (e) => {
    if (e.payload?.token !== seg.token) return;
    $("an-prep-fill").style.width = `${e.payload.pct}%`;
  }).catch(() => {});
  listen("segment-ready", (e) => onSegmentReady(e.payload)).catch(() => {});
  listen("segment-error", (e) => {
    if (e.payload?.token !== seg.token) return;
    seg.preparing = false;
    showPrep(false);
    setPlaying(false);
    msg(`Could not prepare playback: ${e.payload.message}`);
  }).catch(() => {});
  $("an-prep-cancel").addEventListener("click", () => {
    dropSegment();
    setPlaying(false);
  });
  listen("render-stage", () => {
    renderBusy = true;
    setPlaying(false);
    dropSegment();
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
  await syncRenderSize();
  await refresh();
  toggleOptions(true);
});

window.addEventListener("beforeunload", () => {
  play.on = false;
  cancelPrefetch();
  invoke("cancel_segment").catch(() => {});
  invoke("set_preview_quality", { height: 720 }).catch(() => {});
});
