// Analyze window: the replay at full window size with overlays, live
// playback at any speed, and an options drawer that hides without losing
// a single setting. Standalone: it talks to the backend directly.
"use strict";

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
const dialog = window.__TAURI__.dialog;
const $ = (id) => document.getElementById(id);

const ACCENT = "#2fd6d0";
const GHOST = "#ff9c41";
const MUTED = "#8b98a9";

/// Overlay strokes must survive any skin: on a bright background the
/// light defaults vanish, so the whole palette flips dark, the same
/// rule the HUD meters follow.
function olPalette() {
  // From status, not the analysis payload: a mid-session skin swap
  // refreshes it through the normal status poll. The fallback engines
  // show the user's custom background (native deliberately keeps the
  // skin one). With an image behind the picture, keep the light set.
  const light =
    !!status?.light_background && (engine === "native" || !status?.settings?.background);
  return light
    ? {
        box: "rgba(18,22,28,0.9)",
        boxMiss: "rgba(190,24,40,0.95)",
        approach: "rgba(45,55,66,0.55)",
        raw: "#0e1116",
        main: "#0a7d76",
        ghost: "#a85800",
        selFill: "rgba(10,125,118,0.15)",
      }
    : {
        box: "rgba(228,235,243,0.92)",
        boxMiss: "rgba(255,93,108,0.9)",
        approach: "rgba(170,185,200,0.5)",
        raw: "#ffffff",
        main: ACCENT,
        ghost: GHOST,
        selFill: "rgba(47,214,208,0.12)",
      };
}
const DANGER = "#ff5d6c";
const WARN = "#f2c14e";
const OK = "#58d68b";

// Everything the drawer edits lives here, so hiding the drawer can never
// drop a setting: the panel is rebuilt from this state when it reopens.
const opt = {
  path: true,
  raw: true,
  gameCursor: true,
  notes: true,
  markers: false,
  hitboxes: true,
  heatmap: false,
  pathWindow: 600,
  linger: 350,
  quality: "auto",
  immersive: true,
  audio: true,
  audioVol: 20,
  section: "overlays",
};

// Closing the window used to forget every one of those, so a preferred
// overlay set, path window, linger and volume had to be dialled in again on
// each open. Persisted locally; unknown or malformed stored values are
// ignored rather than trusted.
const OPT_STORE = "rhythr.analyze.opt";

function loadOpt() {
  let saved;
  try {
    saved = JSON.parse(localStorage.getItem(OPT_STORE) || "{}");
  } catch {
    return;
  }
  if (!saved || typeof saved !== "object") return;
  for (const [k, v] of Object.entries(saved)) {
    if (!(k in opt)) continue;
    if (typeof v !== typeof opt[k]) continue;
    opt[k] = v;
  }
}

let saveOptTimer = null;
function saveOpt() {
  clearTimeout(saveOptTimer);
  saveOptTimer = setTimeout(() => {
    try {
      localStorage.setItem(OPT_STORE, JSON.stringify(opt));
    } catch {
      /* private mode or a full quota: not worth bothering the user */
    }
  }, 400);
}

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

// ── song audio ──────────────────────────────────────────────
// The map's own music through WebAudio, rate-locked to the analyzer
// clock. Resampling, not time-stretching: changing the speed bends the
// pitch like a record, which is what makes a spot findable by ear.
const audioUrl = () =>
  navigator.userAgent.includes("Windows") ? "http://rhaudio.localhost/song" : "rhaudio://localhost/song";

// performance.now() at the last playback-clock write: the follower
/// extrapolates the staircase clock between writes.
let clockWall = 0;

const snd = {
  ctx: null,
  gain: null,
  buf: null,
  src: null,
  key: "", // dataKey the buffer belongs to
  state: "off", // off | loading | ready | failed: <why>
  startPos: 0,
  startedAt: 0,
  rate: 1,
  driftStrikes: 0,
};

function sndCtx() {
  if (!snd.ctx) {
    try {
      snd.ctx = new (window.AudioContext || window.webkitAudioContext)();
    } catch {
      snd.state = "failed: no AudioContext";
    }
  }
  return snd.ctx;
}

function sndLoad() {
  // The audio belongs to the MAP: a replay or ghost change must not
  // re-download and re-decode the same song.
  const key = status?.map?.path || "";
  if (!key || (snd.key === key && snd.state !== "off" && !snd.state.startsWith("failed"))) return;
  sndStop();
  snd.key = key;
  snd.buf = null;
  if (!sndCtx()) return;
  snd.state = "loading";
  fetch(audioUrl())
    .then((r) => (r.ok ? r.arrayBuffer() : Promise.reject(new Error("map has no audio"))))
    .then((bytes) => (snd.key === key ? snd.ctx.decodeAudioData(bytes) : null))
    .then((buf) => {
      if (!buf || snd.key !== key) return;
      snd.buf = buf;
      snd.state = "ready";
      sndDiagUpdate();
    })
    .catch((e) => {
      if (snd.key === key) snd.state = `failed: ${e?.message || e}`;
      sndDiagUpdate();
    });
}

/// The replay's speed mod as every engine clock applies it (same clamp
/// as the native clock in live.rs and the baked-in segment speed).
function replaySpeed() {
  return clamp(status?.replay?.speed || 1, 0.25, 3);
}

/// The run's hit window in song-time ms, the mirror of
/// `rhythia_sim::hitreg::hit_window_ms`. The game misses a note once
/// `ms > note_t + 55 * speed`, so an unhit note's box lives exactly that
/// long. This was a hardcoded 80 here while the renderer had already moved
/// to the real formula, which drifted the overlay away from the picture on
/// every run that is not 1.45x.
function hitWindowMs() {
  return 55 * clamp(status?.replay?.speed || 1, 0.01, 4);
}

function sndRate() {
  return Math.max(0.001, play.factor * replaySpeed());
}

function sndPos() {
  return snd.startPos + (snd.ctx.currentTime - snd.startedAt) * snd.rate;
}

function sndStop() {
  if (!snd.src) return;
  try {
    snd.src.stop();
  } catch {}
  try {
    snd.src.disconnect();
  } catch {}
  snd.src = null;
}

function sndStart(tSec) {
  sndStop();
  if (!snd.buf || tSec >= snd.buf.duration) return;
  if (!snd.gain) {
    snd.gain = snd.ctx.createGain();
    snd.gain.connect(snd.ctx.destination);
  }
  snd.gain.gain.value = (opt.audioVol ?? 70) / 100;
  const src = snd.ctx.createBufferSource();
  src.buffer = snd.buf;
  const rate = sndRate();
  src.playbackRate.value = rate;
  src.connect(snd.gain);
  src.start(0, Math.max(0, tSec));
  snd.src = src;
  snd.rate = rate;
  snd.startPos = Math.max(0, tSec);
  snd.startedAt = snd.ctx.currentTime;
  snd.driftStrikes = 0;
}

/// One follower for every engine: reads the shared clock (currentMs,
/// play.on, factor) and keeps the audio locked to it. Called from live
/// ticks, the transport handlers and a slow safety interval.
function sndFollow() {
  if (!opt.audio || snd.state !== "ready" || !play.on || document.hidden) {
    sndStop();
    return;
  }
  if (snd.ctx.state === "suspended") return; // resumes on the next play gesture
  // The clock is a staircase (one write per displayed frame); compare
  // against its extrapolation, not the stale last step, or slow frame
  // rates read as drift and restart the music in a loop.
  const ext = clockWall ? currentMs + (performance.now() - clockWall) * sndRate() : currentMs;
  const t = Math.min(ext, runEnd()) / 1000;
  if (!snd.src) {
    sndStart(t);
    return;
  }
  const rate = sndRate();
  if (Math.abs(rate - snd.rate) > 1e-6) {
    // Rebase first so the drift math stays truthful, then bend the pitch.
    snd.startPos = sndPos();
    snd.startedAt = snd.ctx.currentTime;
    snd.rate = rate;
    snd.src.playbackRate.value = rate;
  }
  // Two strikes before a restart: a single stale reading (an in-flight
  // tick around a seek, a segment handover) must not yank the music.
  if (Math.abs(sndPos() - t) > 0.08) {
    if (snd.driftStrikes >= 1) {
      sndStart(t);
    } else {
      snd.driftStrikes++;
    }
  } else {
    snd.driftStrikes = 0;
  }
}

function sndVolume(v) {
  opt.audioVol = v;
  saveOpt();
  if (snd.gain) snd.gain.gain.value = v / 100;
  const s = $("an-vol");
  if (s && Number(s.value) !== v) s.value = String(v);
}

function sndStateText() {
  if (!opt.audio) return "off";
  if (snd.state === "ready") return `ready · ${snd.buf ? Math.round(snd.buf.duration) + "s" : ""}`;
  return snd.state;
}

/// Targeted update: drawSection() must NOT rebuild mid-drag, so state
/// transitions patch only this one value.
function sndDiagUpdate() {
  const el = document.getElementById("an-audio-kv");
  if (el) el.textContent = sndStateText();
}
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
let engine = "video"; // "native" | "video" | "stream"
// Live engine: the picture is painted by the GPU BEHIND this webview;
// this page only draws overlays and controls on a transparent body.
const liveState = { active: false, tick: null, ended: false, key: "", seekTarget: null, seekWall: 0 };
// Short trace of what the playback engine did last, visible under
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
// Segment pipeline, YouTube-style: playback NEVER waits for a render.
// Play starts instantly on the frame stream; rendered video segments are
// produced in the background and take over seamlessly, with the next one
// always buffering while the current one plays.
const seg = {
  token: 0, // latest requested token
  preparing: false,
  preparingFrom: -1,
  switchOnReady: false,
  spanSetting: 12000, // song ms per segment at 1x
  current: null, // {url, startMs, spanMs, outFps}, loaded in <video>
  next: null, // buffered follow-up
};

const videoUrl = (token) => {
  const q = `v=${token}`;
  return navigator.userAgent.includes("Windows")
    ? `http://rhvideo.localhost/seg.mp4?${q}`
    : `rhvideo://localhost/seg.mp4?${q}`;
};

/// A media element refuses a playbackRate below this and quietly clamps
/// it back up, and the segment engine READS the playhead off that
/// element, so a slower speed would run the whole clock too fast. This is
/// the video path's own floor, not the analyzer's: below it the frame
/// stream (no rate limit at all) carries playback instead.
const VIDEO_MIN_RATE = 0.0625;

/// Whether a rendered segment can serve the speed that was asked for.
const segmentsUsable = () => engine === "video" && play.factor >= VIDEO_MIN_RATE;

/// Frames per SONG second so the played result still shows ~60 frames a
/// second at the chosen speed: 60 at 1x, 240 at 0.25x. Every displayed
/// frame is a real rendered frame: no duplicates, no stutter.
function segmentFps() {
  return Math.round(clamp(60 / clamp(play.factor, VIDEO_MIN_RATE, 4), 60, 480));
}

/// The video's duration equals its song span, so the element's rate IS
/// the playback speed.
function videoRate() {
  return clamp(play.factor, VIDEO_MIN_RATE, 4);
}

/// Song time per segment: capped by render effort (~900 frames), so slow
/// motion prepares shorter stretches instead of taking forever.
function segmentSpan() {
  const frames = 900;
  return clamp((frames / segmentFps()) * 1000, 1500, seg.spanSetting);
}

const entryCovers = (e, t) => e && t >= e.startMs - 1 && t < e.startMs + e.spanMs - 60;
const segmentCovers = (t) => entryCovers(seg.current, t);

function showPrep(on, text) {
  $("an-prep").hidden = !on;
  if (text) $("an-prep-text").textContent = text;
  if (!on) $("an-prep-fill").style.width = "0%";
}

/// Kicks off a background render. Playback keeps running on whatever is
/// on screen; the pill is a quiet notice, never a blocker.
function requestSegment(fromMs, switchOnReady) {
  const from = Math.round(fromMs);
  if (seg.preparing && Math.abs(seg.preparingFrom - from) < 500) {
    if (switchOnReady) seg.switchOnReady = true;
    return;
  }
  seg.preparing = true;
  seg.preparingFrom = from;
  seg.switchOnReady = switchOnReady;
  showPrep(true, `Rendering ${(segmentSpan() / 1000).toFixed(0)} s…`);
  invoke("prepare_segment", {
    startMs: from,
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
      tr(`prep failed: ${e}`);
    });
}

// Overlay geometry on the video's own frame grid, so the boxes never lag
// the picture. Kept sorted; nothing crosses IPC during playback.
let segGeo = { times: [], list: [] };

async function primeSegmentGeometry(entry) {
  const stepMs = 1000 / entry.outFps;
  const count = Math.min(1200, Math.round(entry.spanMs / stepMs));
  const times = [];
  for (let i = 0; i < count; i++) times.push(entry.startMs + i * stepMs);
  const merged = segGeo.times.map((t, i) => [t, segGeo.list[i]]);
  for (let i = 0; i < times.length; i += 300) {
    const chunk = times.slice(i, i + 300);
    try {
      const got = await invoke("frame_geometry_batch", { times: chunk });
      chunk.forEach((t, j) => merged.push([t, got[j]]));
    } catch (e) {
      break;
    }
  }
  merged.sort((a, b) => a[0] - b[0]);
  // Keep a bounded window around the segments in flight.
  const trimmed = merged.slice(-3600);
  segGeo = { times: trimmed.map((x) => x[0]), list: trimmed.map((x) => x[1]) };
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

function freeEntry(e) {
  if (e?.url) URL.revokeObjectURL(e.url);
}

function stopVideoElement() {
  const v = $("an-video");
  clearTimeout(watchdog);
  v.pause();
  v.hidden = true;
  document.body.classList.remove("an-playing");
}

function dropSegment() {
  seg.preparing = false;
  seg.switchOnReady = false;
  seg.preparingFrom = -1;
  freeEntry(seg.current);
  freeEntry(seg.next);
  seg.current = null;
  seg.next = null;
  segGeo = { times: [], list: [] };
  showPrep(false);
  stopVideoElement();
  const v = $("an-video");
  v.removeAttribute("src");
  v.load();
  invoke("cancel_segment").catch(() => {});
}

/// A finished segment: fetch it into memory, prime its geometry, then
/// either take over playback, buffer as the follow-up, or wait for Play.
async function onSegmentReady(info) {
  tr(`ready t=${info.token}`);
  if (info.token !== seg.token) return;
  let entry;
  try {
    const res = await fetch(videoUrl(info.token));
    if (!res.ok) throw new Error(`segment ${res.status}`);
    const blob = await res.blob();
    if (info.token !== seg.token) return;
    entry = {
      url: URL.createObjectURL(blob),
      startMs: info.startMs,
      spanMs: info.spanMs,
      outFps: info.outFps,
    };
    tr(`blob ${(blob.size / 1048576).toFixed(1)}MB`);
  } catch (e) {
    seg.preparing = false;
    showPrep(false);
    tr(`blob failed: ${e}`);
    return;
  }
  seg.preparing = false;
  showPrep(false);
  await primeSegmentGeometry(entry);
  const playingVideo = play.on && !$("an-video").hidden;
  if (seg.switchOnReady && play.on && segmentsUsable() && entryCovers(entry, currentMs)) {
    seg.switchOnReady = false;
    freeEntry(seg.current);
    seg.current = entry;
    play.gen++; // stop the stream loop; the video takes over mid-flight
    loadAndPlayCurrent();
  } else if (playingVideo) {
    freeEntry(seg.next);
    seg.next = entry;
  } else {
    freeEntry(seg.current);
    seg.current = entry;
  }
}

function loadAndPlayCurrent() {
  const v = $("an-video");
  v.src = seg.current.url;
  v.load();
  startVideoPlayback();
}

// The video holds `span` song-seconds, so video seconds ARE song seconds.
async function startVideoPlayback() {
  const v = $("an-video");
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
  v.currentTime = clamp(
    (currentMs - seg.current.startMs) / 1000 / replaySpeed(),
    0,
    Math.max(0, (seg.current.spanMs / replaySpeed() - 40) / 1000),
  );
  v.playbackRate = videoRate();
  // Do not await play(): some engines never resolve that promise even
  // though playback starts, which would leave the overlay loop dead.
  v.play().catch((e) => fallbackToStreaming(String(e)));
  // Watchdog: only real movement counts as working playback.
  const t0 = v.currentTime;
  clearTimeout(watchdog);
  watchdog = setTimeout(() => {
    if (play.on && engine === "video" && !v.hidden && v.currentTime <= t0 + 0.01) {
      fallbackToStreaming("video did not advance");
    }
  }, 1500);
  videoTick();
}

/// Streaming takes over RIGHT NOW (seek outside the buffer, view change);
/// a rendered segment for the new position arrives in the background.
function switchToStreamingNow() {
  stopVideoElement();
  play.gen++;
  startStreaming();
}

/// The overlay follows the video's own clock: it can never drift from
/// the picture.
function videoTick() {
  const v = $("an-video");
  if (!play.on || v.hidden) return;
  // The segment file has the replay's speed mod baked in (video.rs:
  // one video second holds `speed` song seconds). Map back to song time.
  currentMs = seg.current.startMs + v.currentTime * 1000 * replaySpeed();
  clockWall = performance.now();
  updateTime();
  drawScrub();
  syncOverlayToVideo();
  const geo = geometryNear(currentMs);
  if (geo) lastFrame = geo;
  const cv = $("an-canvas");
  cv.getContext("2d").clearRect(0, 0, cv.width, cv.height);
  drawOverlay();
  refreshLive();
  sndFollow();
  // Buffer the follow-up EARLY: the moment this segment starts playing.
  if (!seg.next && !seg.preparing) {
    const end = seg.current.startMs + seg.current.spanMs;
    if (end < runEnd() - 100) requestSegment(end, false);
  }
  if (v.ended) {
    if (entryCovers(seg.next, currentMs + 30)) {
      // Gapless handover to the buffered follow-up.
      freeEntry(seg.current);
      seg.current = seg.next;
      seg.next = null;
      loadAndPlayCurrent();
      return;
    }
    // Not buffered yet: keep moving on the frame stream, switch back
    // when the render lands.
    requestSegment(currentMs, true);
    switchToStreamingNow();
    return;
  }
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

/// Cursor position (world units) at an arbitrary song time, interpolated
/// between recorded frames.
function cursorAt(a, ms) {
  const ft = a.frames.t;
  const i = lastIndexLE(ft, ms);
  if (i < 0) return null;
  if (i + 1 < ft.length && ft[i + 1] > ft[i]) {
    const k = clamp((ms - ft[i]) / (ft[i + 1] - ft[i]), 0, 1);
    return [
      a.frames.x[i] + (a.frames.x[i + 1] - a.frames.x[i]) * k,
      a.frames.y[i] + (a.frames.y[i + 1] - a.frames.y[i]) * k,
    ];
  }
  return [a.frames.x[i], a.frames.y[i]];
}

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

// ------------------------------------------------------------ UI scale

// The interface size belongs to the whole app, not to the analyzer: the
// main window owns the control, and both windows share an origin, so the
// stored factor is simply readable here. Not an entry in opt: a window
// that opened at another size than the one the user set is a bug.
const SCALE_STORE = "rhythr.ui.scale";
const SCALE_MIN = 0.8;
const SCALE_MAX = 1.6;

function applyUiScale() {
  let v = 1;
  try {
    const stored = Number(localStorage.getItem(SCALE_STORE));
    if (Number.isFinite(stored) && stored > 0) v = clamp(stored, SCALE_MIN, SCALE_MAX);
  } catch {
    /* private mode or a wiped store: the default size is a fine answer */
  }
  document.documentElement.style.setProperty("--ui-scale", String(v));
}

// At load, not at DOMContentLoaded: the window should open at the user's
// size rather than lay itself out twice in front of them.
applyUiScale();

// The scale can change in the main window while this one is open; the
// storage event is how the other documents of an origin hear about it.
window.addEventListener("storage", (e) => {
  if (e.key != null && e.key !== SCALE_STORE) return;
  applyUiScale();
  // Canvases take their bitmap from their CSS box and only follow it on a
  // redraw, and the stage observer stays silent when nothing but the
  // controls inside it changed size.
  drawScrub();
  refreshLive();
});

// ------------------------------------------------------------ preview

function schedulePreview() {
  if (engine === "native") return; // the live thread paints stills too
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
  // CORS-mode load keeps the canvas origin-clean (the scheme handler
  // sends Access-Control-Allow-Origin: *). The overlay snapshot reads
  // the canvas back, and one tainted frame would poison it for good.
  img.crossOrigin = "anonymous";
  img.src = url;
  await img.decode();
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
      transportNote = `Frame channel fell back to "${transport}": ${e}`;
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
  clockWall = performance.now();
  liveState.seekTarget = currentMs;
  liveState.seekWall = performance.now();
  updateTime();
  drawScrub();
  queueMicrotask(sndFollow);
  if (engine === "native") {
    invoke("live_cmd", { cmd: "seek", value: currentMs }).catch(() => {});
    return;
  }
  if (play.on && segmentsUsable()) {
    if (segmentCovers(currentMs) && !$("an-video").hidden) {
      // Inside the buffered stretch: instant.
      $("an-video").currentTime = (currentMs - seg.current.startMs) / 1000 / replaySpeed();
      return;
    }
    if (entryCovers(seg.next, currentMs)) {
      // The follow-up covers it: promote it now.
      freeEntry(seg.current);
      seg.current = seg.next;
      seg.next = null;
      play.gen++;
      loadAndPlayCurrent();
      return;
    }
    // Outside every buffer: keep moving on frames, render from HERE.
    requestSegment(currentMs, true);
    switchToStreamingNow();
    return;
  }
  if (play.on) {
    // Bump the generation first, the way every other restart does: without
    // it the streaming loop already running keeps its own generation valid
    // and a second one starts beside it. A looped miss seeks on every wrap,
    // so the loops would pile up for as long as the loop runs.
    play.gen++;
    startStreaming();
    return;
  }
  cancelPrefetch();
  schedulePreview();
}

function updateTime() {
  $("an-time").textContent = fmtMsFull(currentMs) + (fps ? `  [${Math.round(fps)} fps]` : "");
  $("an-total").textContent = fmtMs(runEnd());
  enforceLoop();
}

// ------------------------------------------------------- miss navigation
//
// Studying a mistake is what this window is for, and it could only be done
// by hunting for the moment on the scrub bar. These walk the run's misses
// in order and can loop the one you are on.

/// Miss times in chart order, cached per loaded analysis.
let missTimes = [];
/// How much of the approach to show before the note, and how long to keep
/// watching after it, when jumping to or looping a miss.
const MISS_LEAD_MS = 900;
const MISS_TAIL_MS = 400;
let missLoop = null; // {from, to} while looping
let inLoopJump = false;

function collectMisses() {
  const notes = data?.main?.notes || [];
  missTimes = notes.filter((n) => !n.hit).map((n) => n.t).sort((x, y) => x - y);
  const has = missTimes.length > 0;
  for (const id of ["an-miss-prev", "an-miss-next", "an-miss-loop"]) {
    const el = $(id);
    if (el) el.disabled = !has;
  }
  setMissLoop(false);
}

/// The miss the playhead is at or heading into: the one a loop should
/// repeat, and the anchor "previous"/"next" step away from.
function currentMissIndex() {
  if (!missTimes.length) return -1;
  let best = 0;
  for (let i = 0; i < missTimes.length; i++) {
    if (missTimes[i] - MISS_LEAD_MS <= currentMs) best = i;
    else break;
  }
  return best;
}

function gotoMiss(dir) {
  if (!missTimes.length) return;
  const here = currentMissIndex();
  // Treat "already sitting on this miss" as being on it, so one press of
  // prev goes to the previous one rather than re-seeking to the same spot.
  const onIt = Math.abs(currentMs - (missTimes[here] - MISS_LEAD_MS)) < 60;
  let next = dir > 0 ? here + 1 : here - (onIt ? 1 : 0);
  if (!onIt && missTimes[here] - MISS_LEAD_MS > currentMs) {
    // The playhead sits before this miss's lead-in, so it IS the next one;
    // there is nothing earlier to step back to.
    if (dir < 0) return;
    next = here;
  }
  next = clamp(next, 0, missTimes.length - 1);
  const target = missTimes[next];
  // Re-anchor the loop on the DESTINATION before seeking, not after. Seeking
  // first fires updateTime() -> enforceLoop(), and with the loop still on the
  // old miss and playback running, the new position lies outside it, so
  // enforceLoop yanks the playhead straight back to the old miss, and the
  // jump silently does nothing. Moving the loop first means the seek lands
  // inside it.
  if (missLoop) {
    missLoop = { from: Math.max(0, target - MISS_LEAD_MS), to: target + MISS_TAIL_MS };
  }
  seek(Math.max(0, target - MISS_LEAD_MS));
  showChrome();
}

function setMissLoop(on) {
  const btn = $("an-miss-loop");
  if (!on || !missTimes.length) {
    missLoop = null;
  } else {
    const t = missTimes[currentMissIndex()];
    missLoop = { from: Math.max(0, t - MISS_LEAD_MS), to: t + MISS_TAIL_MS };
  }
  if (btn) {
    btn.setAttribute("aria-pressed", missLoop ? "true" : "false");
    btn.classList.toggle("on", !!missLoop);
  }
}

/// Wraps the playhead back to the start of the looped miss. Guarded because
/// seek() calls updateTime() again.
function enforceLoop() {
  if (!missLoop || inLoopJump || !play.on) return;
  if (currentMs >= missLoop.to || currentMs < missLoop.from - 50) {
    inLoopJump = true;
    seek(missLoop.from);
    inLoopJump = false;
  }
}

// ------------------------------------------------------------ overlay

/// Fits the canvas into the stage at the frame's aspect ratio, in real
/// device pixels: the backend renders at exactly this size, so nothing
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
  if (engine === "native") {
    // The picture is painted BEHIND the webview by the live thread and the
    // canvas is overlay-only; its position and size belong to the tick.
    // Falling through blitted `currentBitmap` (the one still fetched at
    // t=0 before the native engine took over) over the running picture,
    // and syncCanvas() then moved and resized the canvas to fit that old
    // frame. Clicking the playfield while paused showed the start of the
    // map on top of the current one, shifted.
    const cv = $("an-canvas");
    const ctx = cv.getContext("2d");
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, cv.width, cv.height);
    drawOverlay();
    return;
  }
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
  // Native mode has no bitmap: the GPU paints the picture; the canvas
  // carries overlays alone.
  if (!data || !lastFrame) return;
  if (engine !== "native" && !currentBitmap) return;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  // Geometry arrives in the frame's own pixels; the canvas IS that size,
  // except when a resize outran the renderer.
  ctx.scale(cv.width / lastFrame.w, cv.height / lastFrame.h);
  ctx.lineJoin = "round";
  ctx.lineCap = "round";
  const t = currentMs;

  const pal = olPalette();
  // Strokes are drawn in frame pixels: scale them with the frame or a 4K
  // window gets hairlines nobody can see.
  const lw = Math.max(1.8, lastFrame.h / 420);
  lastFrame.sides.forEach((side, si) => {
    const a = si === 0 ? data.main : data.ghost;
    const color = si === 0 ? pal.main : pal.ghost;
    if (!a) return;
    // The game's barrier: the visible cursor (and every hit test) is
    // clamped to the field: raw tablet recordings go beyond it, but
    // "outside the field" never happened on screen. Draw the truth.
    const cb = (si === 0 ? data.cursor_bound : data.ghost_cursor_bound) ?? 1.36875;
    const cpx = (wx, wy) => projectPx(side, clamp(wx, -cb, cb), clamp(wy, -cb, cb));

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
        // After the note reaches the plane the box freezes there; the
        // verdict fades out over the linger so stacked past judgements
        // read as past, not as one confusing simultaneous scene.
        const judging = n && t >= n.t;
        let fade = 1;
        if (judging) {
          const resolution = n.hit ? Math.max(n.hit_ms ?? n.t, n.t) : n.t + hitWindowMs();
          const age = t - resolution;
          fade = age > 0 ? Math.max(0.3, 1 - age / Math.max(opt.linger, 1)) : 1;
          ctx.globalAlpha = 0.85 * fade;
        }
        // Approaching notes stay NEUTRAL: the verdict colours the box
        // only once the note reaches the plane. Painting the outcome
        // early reads like the analyzer marking un-reached notes as hit.
        ctx.strokeStyle = sel ? pal.main : !judging ? pal.approach : hit ? pal.box : pal.boxMiss;
        ctx.lineWidth = sel ? lw + 1 : judging ? lw : Math.max(1.2, lw * 0.75);
        ctx.setLineDash(judging && !hit ? [lw * 3, lw * 2.2] : []);
        pathFrom(ctx, q.pts);
        ctx.stroke();
        ctx.setLineDash([]);
        if (sel) {
          ctx.fillStyle = pal.selFill;
          ctx.fill();
        }
        // Verdict marker: where the cursor actually was at the deciding
        // moment (the recorded hit frame, or the note time for a miss).
        // THIS answers "was I really inside?", not the moving picture.
        if (judging) {
          const rm = n.hit ? (n.hit_ms ?? n.t) : n.t;
          const c = cursorAt(a, rm);
          const cp = c && cpx(c[0], c[1]);
          if (cp) {
            // Same fade as the box: the dot must never outlive its box
            // visually and float in empty space.
            ctx.globalAlpha = 0.9 * fade;
            // For a miss, a thin line shows HOW FAR the cursor was from
            // the area at the deciding moment.
            if (!hit) {
              const cxm = (q.pts[0][0] + q.pts[1][0] + q.pts[2][0] + q.pts[3][0]) / 4;
              const cym = (q.pts[0][1] + q.pts[1][1] + q.pts[2][1] + q.pts[3][1]) / 4;
              ctx.beginPath();
              ctx.moveTo(cp[0], cp[1]);
              ctx.lineTo(cxm, cym);
              ctx.lineWidth = Math.max(1, lw * 0.4);
              ctx.strokeStyle = pal.boxMiss;
              ctx.setLineDash([lw * 1.5, lw * 1.5]);
              ctx.stroke();
              ctx.setLineDash([]);
            }
            const r = Math.max(6, lw * 2.4);
            ctx.beginPath();
            ctx.arc(cp[0], cp[1], r, 0, Math.PI * 2);
            ctx.fillStyle = hit ? pal.main : pal.boxMiss;
            ctx.fill();
            // Contrast ring so the dot reads on any ground (including
            // the cursor sprite it often sits on).
            ctx.lineWidth = Math.max(1.5, lw * 0.6);
            ctx.strokeStyle = pal.raw;
            ctx.stroke();
          }
        }
        ctx.globalAlpha = 1;
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
      ctx.lineWidth = Math.max(1.7, lw * 0.8);
      ctx.beginPath();
      let started = false;
      for (let j = lo; j <= hi; j++) {
        if (j > lo && ft[j] - ft[j - 1] > 500) started = false;
        const p = cpx(a.frames.x[j], a.frames.y[j]);
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
        const p = cpx(a.frames.x[j], a.frames.y[j]);
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
        p = cpx(
          a.frames.x[hi] + (a.frames.x[hi + 1] - a.frames.x[hi]) * k,
          a.frames.y[hi] + (a.frames.y[hi + 1] - a.frames.y[hi]) * k,
        );
      } else if (hi >= 0) {
        p = cpx(a.frames.x[hi], a.frames.y[hi]);
      }
      if (p) {
        const arm = Math.max(8, lw * 4.5);
        ctx.strokeStyle = pal.raw;
        ctx.lineWidth = Math.max(1.5, lw * 0.7);
        ctx.beginPath();
        ctx.moveTo(p[0] - arm, p[1]);
        ctx.lineTo(p[0] + arm, p[1]);
        ctx.moveTo(p[0], p[1] - arm);
        ctx.lineTo(p[0], p[1] + arm);
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
  if (best >= 0) {
    // Clicking a note is documented as the way to inspect it, but the
    // inspector only ever redrew if the drawer already happened to be open
    // on that tab, so for most people the click did nothing visible.
    opt.section = "notes";
    saveOpt();
    renderNav();
    toggleOptions(true);
  } else if (opt.section === "notes") {
    drawSection();
  }
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

// The backend keys frames by round(from + step*k): the frontend must use
// the exact same base and step or every request misses the cache.
function prefetch(fromMs) {
  invoke("prefetch_frames", { fromMs, stepMs: frameStep(), count: 45 }).catch(() => {});
}

/// Stops the background renderer: it must not keep working for a
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
  if (on && opt.audio) {
    // This runs inside a real user gesture (click/space), the only
    // place the autoplay policy lets the AudioContext start.
    sndLoad();
    if (snd.ctx?.state === "suspended") snd.ctx.resume().catch(() => {});
  }
  queueMicrotask(sndFollow);
  $("an-play").textContent = on ? "⏸" : "▶";
  document.body.classList.toggle("an-immersive", on && opt.immersive);
  clearTimeout(stillTimer);
  if (!on && engine === "native") {
    invoke("live_cmd", { cmd: "pause" }).catch(() => {});
    return;
  }
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
  if (engine === "native") {
    invoke("live_cmd", { cmd: "play" }).catch((e) => msg(String(e)));
    return;
  }
  if (segmentsUsable()) {
    if (segmentCovers(currentMs)) {
      loadAndPlayCurrent();
      return;
    }
    // Start NOW on frames; the rendered segment takes over when it lands.
    requestSegment(currentMs, true);
    startStreaming();
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

/// The video element never became ready: this platform cannot play the
/// prepared segment, so switch to frames for the rest of the session.
function fallbackToStreaming(why) {
  tr(`fallback: ${why}`);
  clearTimeout(watchdog);
  engine = "stream";
  transportNote = `Segment playback unavailable (${why}). Using single frames.`;
  dropSegment();
  const v = $("an-video");
  v.hidden = true;
  document.body.classList.remove("an-playing");
  if (play.on) startStreaming();
}

async function pump(gen) {
  if (gen !== play.gen || !play.on) return;
  if (renderBusy || status?.rendering) {
    setPlaying(false);
    msg("Paused: a video render is using the renderer.");
    return;
  }
  const step = frameStep();
  // Time comes from the wall clock, quantized to the frame grid the
  // prefetcher renders, so playback keeps real time even if a frame is
  // slow, and every request hits a ready image.
  const elapsed = performance.now() - play.startWall;
  // ONLY the wall clock advances the song. Flooring this at "one more per
  // iteration" would make a 144 Hz display play 2.4x too fast.
  const kw = Math.round(
    (elapsed * play.factor * clamp(status?.replay?.speed || 1, 0.25, 3)) / step,
  );
  if (kw <= play.k) {
    // Same grid point: wait for the next one instead of re-rendering it.
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
  clockWall = performance.now();
  updateTime();
  drawScrub();
  const t0 = performance.now();
  try {
    await showFrame(currentMs);
    loopFails = 0;
  } catch (e) {
    // Never spin on a broken frame channel: say what happened and stop.
    if (++loopFails >= 3) {
      setPlaying(false);
      msg(`Playback stopped: ${e}`);
      return;
    }
  }
  const dt = performance.now() - t0;
  fps = fps ? fps * 0.9 + (1000 / Math.max(1, dt)) * 0.1 : 1000 / Math.max(1, dt);
  // Displayed frames per second of wall clock: that is what "smooth"
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
  // Keep frames and geometry a second ahead of the playhead, on the
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
  if (engine === "native") {
    invoke("live_cmd", { cmd: "pause" }).catch(() => {});
    let i = lastIndexLE(ft, currentMs);
    if (!(dir < 0 && i >= 0 && ft[i] < currentMs)) i += dir;
    const t = ft[clamp(i, 0, ft.length - 1)];
    currentMs = t;
    clockWall = performance.now();
    liveState.seekTarget = t;
    liveState.seekWall = performance.now();
    updateTime();
    drawScrub();
    invoke("live_cmd", { cmd: "seek", value: t }).catch(() => {});
    return;
  }
  if (play.on) setPlaying(false);
  let i = lastIndexLE(ft, currentMs);
  if (!(dir < 0 && i >= 0 && ft[i] < currentMs)) i += dir;
  seek(ft[clamp(i, 0, ft.length - 1)]);
}

let speedTimer = null;

function setSpeed(v) {
  // 0.01x is what the live engine accepts (live.rs clamps its speed
  // command to 0.01..4) and what the slider and the number box offer.
  const next = clamp(v, 0.01, 4);
  const changed = next !== play.factor;
  play.factor = next;
  $("an-speed").value = String(Math.round(play.factor * 100));
  $("an-speed-num").value = String(Math.round(play.factor * 100) / 100);
  if (!changed) return;
  queueMicrotask(sndFollow);
  if (engine === "native") {
    invoke("live_cmd", { cmd: "speed", value: play.factor }).catch(() => {});
    return;
  }
  // A segment is rendered FOR one speed (the slower it is, the more
  // frames per song second), so a new speed needs a new segment.
  // Debounced: dragging the slider must not start a dozen renders.
  clearTimeout(speedTimer);
  // Buffered segments were rendered for the old speed: their frame
  // density no longer matches.
  freeEntry(seg.next);
  seg.next = null;
  if (play.on) {
    if (!$("an-video").hidden) {
      if (play.factor < VIDEO_MIN_RATE) {
        // The element cannot go this slow, and its clock is the playhead,
        // so hand the run back to the frame stream, which has no such floor.
        dropSegment();
        switchToStreamingNow();
        return;
      }
      $("an-video").playbackRate = videoRate(); // instant, until the re-render lands
    } else {
      // The stream clock counts wall time in steps of the CURRENT speed,
      // so a new step rescales everything already played unless the
      // measurement restarts here.
      play.startWall = performance.now();
      play.startMs = Math.round(currentMs);
      play.k = -1;
      lastPrefetchK = -1e9;
    }
    speedTimer = setTimeout(() => {
      if (play.on && segmentsUsable()) requestSegment(currentMs, true);
    }, 500);
  } else {
    freeEntry(seg.current);
    seg.current = null;
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
    // Nothing on the replay said what any of it meant; this is the key.
    const pal = olPalette();
    const swatch = (css, shape) =>
      `<span class="an-swatch an-sw-${shape}" style="--sw:${css}"></span>`;
    html += card(
      "What you are looking at",
      `<div class="an-legend">
        ${swatch(pal.main, "line")}<span>Cursor path: where the cursor actually was, clamped to the field barrier the game enforces.</span>
        ${data.ghost ? `${swatch(pal.ghost, "line")}<span>The ghost run's path.</span>` : ""}
        ${swatch(pal.raw, "dot")}<span>Raw cursor: the recorded position before that clamp. Tablets record beyond the field; no hit ever happened out there.</span>
        ${swatch(pal.approach, "box")}<span>A note's hit area on approach. Neutral on purpose: nothing has been decided yet.</span>
        ${swatch(pal.box, "box")}<span>The note was taken. The box freezes at the plane and stays until the hit, plus the linger time you set.</span>
        ${swatch(pal.boxMiss, "box")}<span>The note was missed. It survives to the end of the hit window, then the line shows how far the cursor stayed away.</span>
        ${swatch(pal.main, "dot")}<span>Verdict dot: where the cursor sat at the exact moment the note resolved.</span>
      </div>`,
    );
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
      <label class="hint an-slider">Verdict boxes stay
        <input type="range" id="opt-linger" min="0" max="1000" step="50" value="${opt.linger}">
        <span>${opt.linger === 0 ? "instant" : (opt.linger / 1000).toFixed(2) + "s"}</span></label>
      <div class="an-title" style="margin-top:10px">Rendered picture</div>
      <div class="an-toggles">
        <label class="an-tog"><input type="checkbox" data-view="gameCursor"${opt.gameCursor ? " checked" : ""}> Game cursor</label>
        <label class="an-tog"><input type="checkbox" data-view="notes"${opt.notes ? " checked" : ""}> Notes</label>
      </div>
      <p class="hint">Hide the game's cursor to study the raw recorded one, or the notes to see nothing but hit areas. Changes re-render the picture.</p>
      <p class="hint">Hitboxes show the game's TRUE hit area (a fixed square, larger than the visual note) and follow each note in. At the hit plane the box freezes (the game judges in 2D, so cursor vs. box is only comparable there) and lingers briefly with a dot marking exactly where the cursor was at the deciding moment: dot inside the box = hit, outside = miss. Works on skins that fade notes out early (half ghost).</p>`,
    );
  } else if (opt.section === "cursor") {
    const c = a.cursor;
    const t = a.speed_series.t;
    const i = Math.max(0, lastIndexLE(t, currentMs));
    html += card(
      "Speed",
      kv("Now", `<span id="an-live-speed">${t.length ? `${fmt1(a.speed_series.v[i])} cells/s` : "-"}</span>`) +
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
        kv("Bias", `${fmt2(a.direction_bias.dx)} / ${fmt2(a.direction_bias.dy)} cells`, "Mean hit offset from note centres (the arrow)"),
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
              `<a class="an-jump" data-t="${s.start_ms}">${fmtMs(s.start_ms)}-${fmtMs(s.end_ms)} · ${fmt1(s.acc_pct)}% · UR ${Math.round(s.ur)} · ${s.misses} miss</a>`,
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
              `<a class="an-jump" data-t="${n.t}" data-note="${n.i}">${fmtMs(n.t)} · ${fmt2(n.near_dist)} cells away</a>`,
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
              : kv("Closest approach", n.near_dist != null ? `${fmt2(n.near_dist)} cells` : "-")) +
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
        v === "clean" ? "no integrity signals" : v === "notice" ? "signals: take a look" : "strong signals"
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
        `<p class="hint an-foot">Signals are hints derived from the recording: context, not verdicts. Absolute input devices (graphics tablets) naturally produce teleport-like jumps when the pen re-enters hover range: on tablet plays, movement signals here are expected and are NOT evidence of cheating.</p>`,
    );
    html += card("Recording rate", kv("Frame delta", `${fmt1(a.frame_deltas.avg_ms)} ms avg · ${fmt1(a.frame_deltas.median_ms)} ms median`) + `<canvas class="an-graph" data-hist="delta"></canvas>`);
  } else if (opt.section === "export") {
    html += card(
      "Save the analysis",
      `<div class="an-actions">
        <button class="btn small" id="exp-card">Analysis card (PNG)</button>
        <button class="btn small ghost" id="exp-json">JSON</button>
        <button class="btn small ghost" id="exp-csv">CSV</button>
        <button class="btn small ghost" id="exp-snap">Overlay snapshot (F8)</button>
      </div><p class="hint">The card is a shareable summary; JSON and CSV carry the per-note data. The overlay snapshot saves exactly what you see (picture plus hitboxes/path), ideal for bug reports.</p>`,
    );
  } else if (opt.section === "view") {
    // Every shortcut in one place: they were only discoverable by hovering
    // the right button, and half of them have no button at all.
    html += card(
      "Keyboard",
      `<div class="an-keys">
        ${[
          ["Space", "Play / pause"],
          ["← →", "Step one frame"],
          ["Shift + ← →", "Jump one second"],
          [", .", "Previous / next miss"],
          ["L", "Loop the current miss"],
          ["O", "Options drawer"],
          ["F8", "Save an overlay snapshot"],
          ["Esc", "Close the drawer"],
        ]
          .map(([k, what]) => `<kbd>${k}</kbd><span>${what}</span>`)
          .join("")}
      </div>`,
    );
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
      </div><p class="hint">Frames render at the window's own pixel size: nothing is scaled, so Native is the sharpest AND the cheapest way to fill the window. Auto starts there and steps down once if playback can't hold ~55 fps${
        fps ? ` (currently ${Math.round(fps)} fps at ${scalePct()}%)` : ""
      }.</p>`,
    );
    html += card(
      "While playing",
      `<label class="an-tog"><input type="checkbox" data-opt="immersive"${opt.immersive ? " checked" : ""}> Hide the controls during playback</label>
       <p class="hint">They come back the moment you move the mouse or pause.</p>`,
    );
    html += card(
      "Song audio",
      `<label class="an-tog"><input type="checkbox" data-opt="audio"${opt.audio ? " checked" : ""}> Play the map's music</label>
       <p class="hint">The music follows the playback clock. Slowing down bends the pitch down with it. Find a spot by ear, the way you can't mid-run. Volume lives in the playbar.</p>`,
    );
    html += card(
      "Diagnostics",
      kv("Build", status?.build || "?") +
        kv("Engine", engine === "native" ? "live GPU (native)" : engine === "video" ? "rendered video" : "single frames") +
        kv("Still frames", transport) +
        kv("Playback", !$("an-video").hidden ? `video · ${seg.current?.outFps ?? "?"} fps/song-s` : seg.preparing ? "rendering…" : loopFps ? `stream · ${Math.round(loopFps)} fps` : "idle") +
        kv("Buffered", seg.current ? `${fmtMs(seg.current.startMs)}+${(seg.current.spanMs / 1000).toFixed(0)}s${seg.next ? ` · next ${fmtMs(seg.next.startMs)}+${(seg.next.spanMs / 1000).toFixed(0)}s` : ""}` : "-") +
        kv("Render size", `${lastRenderH}p at ${scalePct()}%`) +
        `<div class="an-kv"><span>Audio</span><b id="an-audio-kv">${sndStateText()}</b></div>` +
        `<div class="an-list"><span class="hint">${esc(trace.join(" · ") || "-")}</span></div>` +
        (transportNote ? `<p class="hint">${esc(transportNote)}</p>` : "") +
        `<p class="hint">If playback stalls, this tells us where. "fetch" is the fast path; the window falls back on its own if a platform blocks it.</p>`,
    );
    html += card(
      "Shortcuts",
      `<div class="an-list"><span>Space: play / pause</span><span>← / →: one frame</span><span>Shift + ← / →: one second</span><span>O: options · Esc: hide options</span><span>F8: save overlay snapshot</span><span>Click a note: inspect it</span></div>`,
    );
  }

  body.innerHTML = html;
  wireSection();
}

/// Per-frame update of the drawer: canvases and live readouts only.
/// Rebuilding the DOM here would swallow clicks and abort slider drags.
function refreshLive() {
  if ($("an-options").hidden || !data) return;
  redrawGraphs();
  const live = $("an-live-speed");
  if (live) {
    const t = data.main.speed_series.t;
    const i = Math.max(0, lastIndexLE(t, currentMs));
    live.textContent = t.length ? `${fmt1(data.main.speed_series.v[i])} cells/s` : "-";
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
  body.querySelectorAll("input[data-view]").forEach((cb) => {
    cb.addEventListener("change", async () => {
      opt[cb.dataset.view] = cb.checked;
      saveOpt();
      if (engine === "native") {
        invoke("live_cmd", {
          cmd: "view",
          hideCursor: !opt.gameCursor,
          hideNotes: !opt.notes,
        }).catch((e) => msg(String(e)));
        // Persist too: a session restart rebuilds from the stored flags.
        invoke("set_analyze_view", {
          hideCursor: !opt.gameCursor,
          hideNotes: !opt.notes,
        }).catch(() => {});
        return;
      }
      try {
        status = await invoke("set_analyze_view", {
          hideCursor: !opt.gameCursor,
          hideNotes: !opt.notes,
        });
      } catch (e) {
        msg(String(e));
        return;
      }
      // Everything rendered so far shows the old look.
      geoCache.clear();
      freeEntry(seg.next);
      seg.next = null;
      const playingVideo = play.on && !$("an-video").hidden;
      freeEntry(seg.current);
      seg.current = null;
      if (playingVideo) {
        requestSegment(currentMs, true);
        switchToStreamingNow();
      } else if (play.on) {
        requestSegment(currentMs, true);
      } else {
        schedulePreview();
      }
    });
  });
  body.querySelectorAll("input[data-opt]").forEach((cb) => {
    cb.addEventListener("change", () => {
      opt[cb.dataset.opt] = cb.checked;
      saveOpt();
      if (cb.dataset.opt === "audio") {
        if (cb.checked) {
          sndLoad();
          if (snd.ctx?.state === "suspended") snd.ctx.resume().catch(() => {});
        }
        sndFollow();
        sndDiagUpdate();
      }
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
      saveOpt();
      win.nextElementSibling.textContent = `${(opt.pathWindow / 1000).toFixed(2)}s`;
      drawFrame();
    });
  }
  const lin = body.querySelector("#opt-linger");
  if (lin) {
    lin.addEventListener("input", () => {
      opt.linger = Number(lin.value);
      saveOpt();
      lin.nextElementSibling.textContent =
        opt.linger === 0 ? "instant" : `${(opt.linger / 1000).toFixed(2)}s`;
      // 0 must mean INSTANT for the engine, but the backend treats 0 as
      // "use default", so send 1 ms instead (visually identical).
      const ms = Math.max(1, opt.linger);
      invoke("live_cmd", { cmd: "linger", value: ms }).catch(() => {});
      invoke("set_analyze_linger", { ms }).catch(() => {});
      geoCache.clear();
      drawFrame();
    });
  }
  body.querySelectorAll("input[data-q]").forEach((rb) => {
    rb.addEventListener("change", () => {
      // Kept as a string so the stored value always has the same type as
      // the default; every reader coerces with Number()/String() anyway.
      opt.quality = rb.dataset.q;
      saveOpt();
      autoScale = 100;
      autoNextCheck = 1200;
      lastRenderH = 0;
      syncRenderSize();
    });
  });
  $("exp-card")?.addEventListener("click", exportCard);
  $("exp-snap")?.addEventListener("click", snapOverlay);
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
    msgFlash(`Saved: ${p}`);
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
    msgFlash(`Saved: ${p}`);
  } catch (e) {
    msg(String(e));
  }
}

/// Composites the current picture and the overlay canvas into one PNG:
/// exactly what the eye sees, portable and test-friendly (F8).
let snapBusy = false;
async function snapOverlay() {
  if (!lastFrame || snapBusy) return;
  snapBusy = true;
  try {
    // Freeze the moment FIRST: the overlay pixels now, the still for the
    // same clock value. During playback the clock moves while the frame
    // fetch runs, and a late picture under a fresh overlay lies.
    const t0 = currentMs;
    const ov = document.createElement("canvas");
    ov.width = lastFrame.w;
    ov.height = lastFrame.h;
    ov.getContext("2d").drawImage($("an-canvas"), 0, 0);
    const c = document.createElement("canvas");
    c.width = lastFrame.w;
    c.height = lastFrame.h;
    const x = c.getContext("2d");
    x.fillStyle = "#000";
    x.fillRect(0, 0, c.width, c.height);
    try {
      // Native mode: ask the live engine itself. Its picture (skin
      // background, live resolution) is what the screen shows; the
      // preview pipeline would paint the custom background instead.
      // fetch + ImageBitmap keeps the canvas origin-clean: an <img>
      // without CORS would taint it and toDataURL throws SecurityError.
      let blob = null;
      if (engine === "native") {
        try {
          const bytes = await invoke("live_still");
          blob = new Blob([bytes], { type: "image/png" });
        } catch {}
      }
      if (!blob) {
        const r = await fetch(frameUrl(t0));
        if (r.ok) blob = await r.blob();
      }
      if (blob) {
        const bmp = await createImageBitmap(blob);
        // Aspect-fit, never stretch: the preview still and the live
        // frame can differ in resolution (and defensively, in aspect).
        const s = Math.min(c.width / bmp.width, c.height / bmp.height);
        const dw = bmp.width * s;
        const dh = bmp.height * s;
        x.drawImage(bmp, (c.width - dw) / 2, (c.height - dh) / 2, dw, dh);
        bmp.close?.();
      }
    } catch {
      // No still available (render busy): overlay on black still helps.
    }
    x.drawImage(ov, 0, 0, c.width, c.height);
    let dataUrl;
    try {
      dataUrl = c.toDataURL("image/png");
    } catch {
      msg("Overlay snapshot unavailable: the frame channel on this system taints the canvas.");
      return;
    }
    let p = null;
    try {
      p = await invoke("overlay_snap_target");
    } catch {}
    if (!p) {
      p = await dialog.save({
        defaultPath: `${expBase()} - overlay.png`,
        filters: [{ name: "PNG image", extensions: ["png"] }],
      });
      if (!p) return;
    }
    try {
      await invoke("save_data_url_png", { path: p, dataUrl });
      msg("Overlay snapshot saved.");
      clearTimeout(flashTimer);
      flashTimer = setTimeout(() => msg(""), 1800);
    } catch (e) {
      msg(String(e));
    }
  } finally {
    snapBusy = false;
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
  x.fillText(`${data.player} · ${data.map_title || "run"}`, 48, 72);
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
    msgFlash(`Saved: ${p}`);
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
  `${status?.replay?.path}|${status?.ghost?.path || ""}|${status?.map?.path}|${status?.config?.path || ""}|${status?.settings?.width}x${status?.settings?.height}`;

async function refresh() {
  try {
    status = await invoke("get_status");
  } catch (e) {
    msg(String(e));
    return;
  }
  const title = status?.replay
    ? `${status.replay.player} · ${status.map?.song_name || status.map?.title || ""}`
    : "No replay loaded";
  $("an-title").textContent = status?.build ? `${title}   ·   build ${status.build}` : title;
  if (!status?.replay || !status?.map) {
    data = null;
    dataKey = "";
    geoCache.clear();
    $("an-chip").hidden = true;
    setPlaying(false);
    msg("Load a replay (and its map) in the main window. This view follows it.");
    drawSection();
    return;
  }
  const key = sourceKey();
  if (key === dataKey && data) return;
  await loadData(key);
  if (engine === "native" && liveState.key !== key) {
    // Sources changed: restart the live thread against the new replay.
    liveState.active = false;
    const ok = await bootNative();
    if (!ok) fallbackFromNative("could not restart");
  } else if (engine !== "native") {
    // Native may have been unavailable when the window opened (no replay
    // loaded yet) or the engine died: sources changed, so try again.
    // On platforms without native support this returns false instantly.
    await bootNative();
  }
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
  collectMisses();
  // Geometry describes THIS replay on THIS field: never reuse any of it.
  geoCache.clear();
  segGeo = { times: [], list: [] };
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
  if (opt.audio) sndLoad();
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
    // drawFrame(), not drawOverlay(): in native and video mode the canvas is
    // overlay-only and drawOverlay() does not clear it, so toggling the drawer
    // while paused stacked a fresh overlay over the old one and it darkened
    // each time. drawFrame() clears first in every mode, then draws the
    // overlay.
    drawFrame();
    drawScrub();
  });
}

// ------------------------------------------------------------ boot

// A silent exception used to kill the playback loop without a trace;
// surface it instead.
window.addEventListener("error", (e) => msg(`Error: ${e.message}`));
window.addEventListener("unhandledrejection", (e) => msg(`Error: ${e.reason}`));

/// One live-tick: the native engine's clock and per-side geometry. The
/// canvas paints ONLY overlays; the picture sits behind the webview.
function onLiveTick(tk) {
  // Ticks in flight from before a seek carry the OLD clock: letting
  // them through rewinds the UI and yanks the audio back for a beat.
  if (liveState.seekTarget != null) {
    if (Math.abs(tk.t - liveState.seekTarget) < 250 || performance.now() - liveState.seekWall > 300) {
      liveState.seekTarget = null;
    } else {
      return;
    }
  }
  liveState.tick = tk;
  currentMs = tk.t;
  clockWall = performance.now();
  const wasPlaying = play.on;
  play.on = tk.playing;
  if (wasPlaying !== tk.playing) {
    $("an-play").textContent = tk.playing ? "⏸" : "▶";
    document.body.classList.toggle("an-immersive", tk.playing && opt.immersive);
  }
  fps = tk.fps;
  updateTime();
  drawScrub();
  // Overlay canvas covers the letterboxed frame rect (CSS px).
  const dpr = window.devicePixelRatio || 1;
  const cv = $("an-canvas");
  const [rx, ry, rw, rh] = tk.rect;
  cv.style.left = `${rx / dpr}px`;
  cv.style.top = `${ry / dpr}px`;
  cv.style.width = `${rw / dpr}px`;
  cv.style.height = `${rh / dpr}px`;
  if (cv.width !== tk.fw || cv.height !== tk.fh) {
    cv.width = tk.fw;
    cv.height = tk.fh;
  }
  lastFrame = { w: tk.fw, h: tk.fh, sides: tk.sides };
  sndFollow();
  const ctx = cv.getContext("2d");
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, cv.width, cv.height);
  drawOverlay();
  refreshLive();
}

let nativeWired = false;

/// The live engine died (or refused to restart): hand the window back to
/// the frame/segment engines and strip every native-mode override, or the
/// page stays transparent with no picture behind it.
function fallbackFromNative(reason) {
  const wasNative = engine === "native";
  engine = "video";
  liveState.active = false;
  liveState.key = "";
  document.body.classList.remove("an-native");
  document.documentElement.classList.remove("an-native");
  document.body.classList.remove("an-immersive");
  play.on = false;
  $("an-play").textContent = "▶";
  if (!wasNative) return;
  tr(`native fallback: ${reason}`);
  msg("");
  syncRenderSize()
    .then(() => schedulePreview())
    .catch(() => {});
}

function wireNativeOnce() {
  if (nativeWired) return;
  nativeWired = true;
  listen("live-tick", (e) => onLiveTick(e.payload)).catch(() => {});
  listen("live-ended", () => {
    play.on = false;
    $("an-play").textContent = "▶";
    document.body.classList.remove("an-immersive");
  }).catch(() => {});
  listen("live-error", (e) => {
    tr(`live error: ${e.payload}`);
    fallbackFromNative(String(e.payload));
  }).catch(() => {});
  // Window resizes reach the render thread as physical pixels.
  let rzTimer = null;
  window.addEventListener("resize", () => {
    if (engine !== "native") return;
    clearTimeout(rzTimer);
    rzTimer = setTimeout(() => {
      // No size args: the backend reads the window's true physical size
      // itself: innerWidth*dpr is off by one at fractional Windows DPI.
      invoke("live_cmd", { cmd: "resize" }).catch(() => {});
    }, 120);
  });
}

async function bootNative() {
  try {
    const ok = await invoke("start_live_session");
    if (!ok) return false;
  } catch (e) {
    tr(`native failed: ${e}`);
    return false;
  }
  engine = "native";
  // Whatever still was fetched before this point belongs to a dead path
  // now. Keeping it around is how a t=0 frame survived to be blitted over
  // the live picture later.
  currentBitmap?.close?.();
  currentBitmap = null;
  liveState.active = true;
  liveState.key = sourceKey();
  document.body.classList.add("an-native");
  document.documentElement.classList.add("an-native");
  // The frame path is unused in native mode.
  $("an-video").hidden = true;
  wireNativeOnce();
  tr("native engine");
  return true;
}

window.addEventListener("DOMContentLoaded", async () => {
  // Before anything reads opt: restore what the last session chose.
  loadOpt();
  // The restored checkboxes/slider only change the picture if the backend is
  // told too: the change handlers do that, but they never fire at boot, so
  // without this a reopened window shows "Notes off" while a rendered segment
  // still has notes. Sync the backend to the restored state once.
  invoke("set_analyze_view", { hideCursor: !opt.gameCursor, hideNotes: !opt.notes }).catch(() => {});
  invoke("set_analyze_linger", { ms: opt.linger }).catch(() => {});
  $("an-gear").addEventListener("click", () => toggleOptions());
  $("an-close").addEventListener("click", () => toggleOptions(false));
  $("an-play").addEventListener("click", () => setPlaying(!play.on));
  $("an-back").addEventListener("click", () => stepFrame(-1));
  $("an-fwd").addEventListener("click", () => stepFrame(1));
  $("an-miss-prev").addEventListener("click", () => gotoMiss(-1));
  $("an-miss-next").addEventListener("click", () => gotoMiss(1));
  $("an-miss-loop").addEventListener("click", () => setMissLoop(!missLoop));
  $("an-speed").addEventListener("input", () => setSpeed(Number($("an-speed").value) / 100));
  $("an-speed-num").addEventListener("change", () => {
    const v = parseFloat(String($("an-speed-num").value).replace(",", "."));
    setSpeed(Number.isFinite(v) && v > 0 ? v : play.factor);
  });
  $("an-speed-reset").addEventListener("click", () => setSpeed(1));
  // The slider's markup default is only a fallback; the stored choice wins.
  $("an-vol").value = String(opt.audioVol);
  $("an-vol").addEventListener("input", () => sndVolume(Number($("an-vol").value)));
  $("an-canvas").addEventListener("click", pickNote);
  // Native mode: the canvas is click-transparent (it spans the window
  // under the floating controls), so picking listens on the stage.
  $("stage").addEventListener("click", (e) => {
    if (engine !== "native") return;
    if (e.target !== $("stage") && e.target !== $("an-canvas")) return;
    pickNote(e);
  });
  $("an-secnav").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-sec]");
    if (!b) return;
    opt.section = b.dataset.sec;
    saveOpt();
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
  // Scrubbing pauses while you drag, but it used to leave playback off
  // afterwards, so finding a spot always cost an extra press of space.
  let resumeAfterScrub = false;
  $("an-scrub").addEventListener("pointerdown", (e) => {
    $("an-scrub").setPointerCapture(e.pointerId);
    resumeAfterScrub = play.on;
    setPlaying(false);
    scrubSeek(e);
  });
  $("an-scrub").addEventListener("pointermove", (e) => {
    if (e.buttons & 1) scrubSeek(e);
  });
  $("an-scrub").addEventListener("pointerup", () => {
    if (resumeAfterScrub) setPlaying(true);
    resumeAfterScrub = false;
  });
  $("an-scrub").addEventListener("pointercancel", () => {
    resumeAfterScrub = false;
  });

  // Hand focus back after a drag, so the very next key press is a playback
  // key again instead of another nudge of the slider the user just let go of.
  document.addEventListener("pointerup", (e) => {
    const el = e.target;
    if (el && el.tagName === "INPUT" && (el.type || "").toLowerCase() === "range") {
      el.blur();
    }
  });
  document.addEventListener("mousemove", showChrome);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden && play.on) setPlaying(false);
  });
  // A range slider is an <input> too, so treating every input as "typing"
  // meant one click on the volume or speed slider silently killed space and
  // the arrow keys for the rest of the session.
  const TEXT_INPUTS = new Set([
    "text", "number", "search", "url", "email", "password", "tel", "date", "time",
  ]);
  document.addEventListener("keydown", (e) => {
    const t = document.activeElement;
    const inputType = t && t.tagName === "INPUT" ? (t.type || "text").toLowerCase() : "";
    const typing =
      t && (t.tagName === "TEXTAREA" || t.isContentEditable || TEXT_INPUTS.has(inputType));
    // A focused control owns the keys it normally handles: a slider and a
    // radio take the arrows, a checkbox and a radio take space. Swallowing
    // those for playback would make the drawer unusable by keyboard.
    const onSlider = inputType === "range";
    const takesSpace = inputType === "checkbox" || inputType === "radio";
    const takesArrows = onSlider || inputType === "radio";
    if (e.key === "Escape") {
      toggleOptions(false);
      return;
    }
    if (e.key === "F8") {
      e.preventDefault();
      snapOverlay();
      return;
    }
    if (typing || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.code === "Space") {
      if (takesSpace) return;
      e.preventDefault();
      setPlaying(!play.on);
      showChrome();
    } else if (e.key === "ArrowLeft") {
      if (takesArrows) return;
      e.preventDefault();
      if (e.shiftKey) seek(currentMs - 1000);
      else stepFrame(-1);
      showChrome();
    } else if (e.key === "ArrowRight") {
      if (takesArrows) return;
      e.preventDefault();
      if (e.shiftKey) seek(currentMs + 1000);
      else stepFrame(1);
      showChrome();
    } else if (e.key === "," || e.key === "PageUp") {
      e.preventDefault();
      gotoMiss(-1);
    } else if (e.key === "." || e.key === "PageDown") {
      e.preventDefault();
      gotoMiss(1);
    } else if (e.key.toLowerCase() === "l") {
      setMissLoop(!missLoop);
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

  // The playbar folds to a second row on a narrow window; the render pill
  // rides above whatever height it ends up with instead of landing on the
  // transport.
  new ResizeObserver(() => {
    $("stage").style.setProperty("--playbar-h", `${$("an-playbar").offsetHeight}px`);
  }).observe($("an-playbar"));

  // The main window drives which replay is loaded; follow its changes.
  listen("sources-changed", () => {
    geoCache.clear();
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
  // finishing mid-playback, say). The event above does the real work.
  setInterval(() => {
    if (!play.on) refresh();
  }, 5000);

  setSpeed(1);
  // Native boots from refresh() once the sources are known: booting
  // here too would tear the engine down and rebuild it immediately.
  await syncRenderSize();
  await refresh();
  toggleOptions(true);
});

setInterval(sndFollow, 100);

window.addEventListener("beforeunload", () => {
  play.on = false;
  sndStop();
  snd.ctx?.close().catch(() => {});
  cancelPrefetch();
  invoke("cancel_segment").catch(() => {});
  invoke("set_preview_quality", { height: 720 }).catch(() => {});
});
