// Note designer: an outline editor whose output is a real mesh skin.
//
// Three layers of shaping, all funnelling into ONE closed outline:
//   1. sliders (base shape, rounding, hollow, depth)
//   2. modifiers (waves, twist) applied to the slider shape
//   3. freeform: the current outline frozen into draggable control
//      points, with radial symmetry so one edited wedge repeats
// The backend turns the outline into geometry and renders the preview
// with the actual renderer, so the picture is the truth, not a sketch.

const invoke = window.__TAURI__.core.invoke;
const dialog = window.__TAURI__.dialog;

const $ = (id) => document.getElementById(id);

// ------------------------------------------------------------ state

const P = {
  shape: "circle",
  n: 6,
  round: 0.3,
  hollow: true,
  thick: 0.22,
  depth: 0.05,
  waves: 0,
  waveDepth: 0.25,
  waveform: "sine",
  twist: 0,
};

/// The wave shape: soft, spiky or blocky, all with the same count/depth.
function waveFn(k) {
  const s = Math.sin(k);
  if (P.waveform === "spike") return (2 / Math.PI) * Math.asin(s);
  if (P.waveform === "pulse") return Math.tanh(3 * s) / Math.tanh(3);
  return s;
}
// Freeform mode: control points in the unit plane, each {x, y, smooth}.
// With symmetry s > 1 the points cover ONE wedge of 2π/s and the rest is
// generated; with s == 1 they are the whole outline.
let free = null;
let symmetry = 4;
let colors = ["#ffffff", "#a2e0ff", "#a2e0ff"]; // the game's Arctic

// ------------------------------------------------------------ outline

/// Slider outline: radius as a function of angle, sampled densely.
function paramOutline(samples = 160) {
  const pts = [];
  const tw = (P.twist * Math.PI) / 180;
  for (let i = 0; i < samples; i++) {
    const th = (i / samples) * Math.PI * 2;
    let r;
    if (P.shape === "circle") {
      r = 1;
    } else {
      // Regular N-gon radius, blended toward the circle by the rounding
      // slider. A square is the 4-gon turned 45° so its sides sit flat.
      const n = P.shape === "square" ? 4 : P.n;
      const off = P.shape === "square" ? Math.PI / 4 : 0;
      const a = ((th + off + tw) % (Math.PI * 2 / n) + Math.PI * 2 / n) % (Math.PI * 2 / n);
      const ngon = Math.cos(Math.PI / n) / Math.cos(a - Math.PI / n);
      r = ngon + (1 - ngon) * P.round;
    }
    if (P.waves > 0) {
      r *= 1 + P.waveDepth * waveFn(P.waves * (th + tw));
    }
    pts.push([r * Math.cos(th), r * Math.sin(th)]);
  }
  // Waves can push the radius past the unit cap; scale the whole shape
  // back so the drawn outline IS the exported one, never larger.
  const max = pts.reduce((m, p) => Math.max(m, Math.abs(p[0]), Math.abs(p[1])), 0);
  if (max > 1) {
    for (const p of pts) {
      p[0] /= max;
      p[1] /= max;
    }
  }
  return pts;
}

/// Expands the freeform control points through the symmetry, then samples
/// a closed Catmull-Rom through them (corner points pass straight lines).
function freeOutline() {
  const ctrl = [];
  for (let s = 0; s < symmetry; s++) {
    const a = (s * Math.PI * 2) / symmetry;
    const ca = Math.cos(a), sa = Math.sin(a);
    for (const p of free) {
      ctrl.push({ x: p.x * ca - p.y * sa, y: p.x * sa + p.y * ca, smooth: p.smooth });
    }
  }
  const n = ctrl.length;
  if (n < 3) return null;
  const out = [];
  const SEG = 10;
  for (let i = 0; i < n; i++) {
    const p0 = ctrl[(i + n - 1) % n];
    const p1 = ctrl[i];
    const p2 = ctrl[(i + 1) % n];
    const p3 = ctrl[(i + 2) % n];
    if (!p1.smooth && !p2.smooth) {
      out.push([p1.x, p1.y]);
      continue;
    }
    for (let k = 0; k < SEG; k++) {
      const t = k / SEG;
      // Catmull-Rom; a corner endpoint collapses its tangent so the curve
      // actually reaches the corner instead of rounding it away.
      const m1 = p1.smooth ? { x: (p2.x - p0.x) / 2, y: (p2.y - p0.y) / 2 } : { x: 0, y: 0 };
      const m2 = p2.smooth ? { x: (p3.x - p1.x) / 2, y: (p3.y - p1.y) / 2 } : { x: 0, y: 0 };
      const t2 = t * t, t3 = t2 * t;
      const h00 = 2 * t3 - 3 * t2 + 1, h10 = t3 - 2 * t2 + t;
      const h01 = -2 * t3 + 3 * t2, h11 = t3 - t2;
      out.push([
        h00 * p1.x + h10 * m1.x + h01 * p2.x + h11 * m2.x,
        h00 * p1.y + h10 * m1.y + h01 * p2.y + h11 * m2.y,
      ]);
    }
  }
  return out;
}

const outline = () => (free ? freeOutline() : paramOutline());

// ------------------------------------------------------------ canvas

const cv = $("dz-canvas");
const ctx = cv.getContext("2d");
// The canvas fills its box and follows the window: world coordinates map
// through a scale derived from the CURRENT size, so growing the window
// grows the note instead of leaving dead space.
let CW = 520, CH = 520, UNIT = 520 / 2.9;
function fitCanvas() {
  const box = $("dz-canvas-box");
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(200, box.clientWidth);
  const h = Math.max(200, box.clientHeight);
  cv.width = Math.round(w * dpr);
  cv.height = Math.round(h * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  CW = w;
  CH = h;
  UNIT = Math.min(w, h) / 2.9;
  draw();
}
new ResizeObserver(fitCanvas).observe($("dz-canvas-box"));
const toPx = (p) => [CW / 2 + p[0] * UNIT, CH / 2 - p[1] * UNIT];
const toWorld = (x, y) => [(x - CW / 2) / UNIT, -(y - CH / 2) / UNIT];

// The note's contours come from the BACKEND (designer_outline), which
// runs the identical cleanup, normalisation and rim inset the mesh build
// runs. A local port of that maths drifted from the real thing once
// already, which is exactly how the edited note and the previewed note
// stopped matching; one source of truth ends that class of bug.
let plan = null;
let planBusy = false;
let planAgain = false;

async function refreshPlan() {
  if (planBusy) {
    planAgain = true;
    return;
  }
  planBusy = true;
  try {
    plan = await invoke("designer_outline", { shape: shapePayload() });
  } catch (e) {
    plan = null;
    setStatus(String(e), true);
  } finally {
    planBusy = false;
    if (planAgain) {
      planAgain = false;
      refreshPlan();
    }
  }
  draw();
}
// The game's TRUE hit area in note units: hitbox half-extent 0.56875
// cells over the note's visual half-size 0.45 cells. The note itself is
// HARD-capped at the unit square (like every real skin), so a designed
// note can never outgrow the box that judges it.
const HITBOX = 0.56875 / 0.45;
const clampUnit = (x, y) => [
  Math.max(-1, Math.min(1, x)),
  Math.max(-1, Math.min(1, y)),
];

function draw() {
  ctx.clearRect(0, 0, CW, CH);
  const styles = getComputedStyle(document.documentElement);
  const accent = styles.getPropertyValue("--accent").trim() || "#2fd6d0";
  // Two reference squares. Solid: the hard note limit (the unit square
  // every real skin uses; nothing here may cross it). Dashed, outside:
  // the game's TRUE hit area, which is a quarter bigger than the note,
  // shown so the size relationship is visible rather than guessed.
  ctx.lineWidth = 1;
  ctx.strokeStyle = "rgba(139,152,169,0.35)";
  ctx.strokeRect(...toPx([-1, 1]), 2 * UNIT, 2 * UNIT);
  ctx.strokeStyle = "rgba(242,193,78,0.35)";
  ctx.setLineDash([4, 4]);
  ctx.strokeRect(...toPx([-HITBOX, HITBOX]), 2 * UNIT * HITBOX, 2 * UNIT * HITBOX);
  ctx.setLineDash([]);

  if (!plan || !plan.outer || plan.outer.length < 3) return;
  // The note as the export will have it: the backend's contours, filled
  // (or rimmed) in the FIRST colorset colour. Every slider changes THIS
  // drawing, and it is the same geometry the preview and the .rhs get.
  const path = (list) => {
    const [px, py] = toPx(list[0]);
    ctx.moveTo(px, py);
    for (const p of list.slice(1)) ctx.lineTo(...toPx(p));
    ctx.closePath();
  };
  // Fill, then cut: nonzero on each contour separately. The evenodd rule
  // cancelled the fill wherever a dragged outline overlapped itself,
  // which read as the colour vanishing in stripes.
  ctx.save();
  ctx.beginPath();
  path(plan.outer);
  ctx.fillStyle = colors[0] || "#ffffff";
  ctx.globalAlpha = 0.9;
  ctx.fill("nonzero");
  if (plan.inner) {
    ctx.globalCompositeOperation = "destination-out";
    ctx.globalAlpha = 1;
    ctx.beginPath();
    path(plan.inner);
    ctx.fill("nonzero");
  }
  ctx.restore();
  ctx.beginPath();
  path(plan.outer);
  ctx.strokeStyle = accent;
  ctx.lineWidth = 1.5;
  ctx.stroke();

  if (free) {
    // Symmetry wedge: the editable region.
    if (symmetry > 1) {
      ctx.lineWidth = 1;
      ctx.strokeStyle = "rgba(139,152,169,0.35)";
      ctx.beginPath();
      ctx.moveTo(...toPx([0, 0]));
      ctx.lineTo(...toPx([1.25, 0]));
      const a = (Math.PI * 2) / symmetry;
      ctx.moveTo(...toPx([0, 0]));
      ctx.lineTo(...toPx([1.25 * Math.cos(a), 1.25 * Math.sin(a)]));
      ctx.stroke();
    }
    const ps = plan?.scale || 1;
    for (const p of free) {
      const [x, y] = toPx([p.x * ps, p.y * ps]);
      ctx.beginPath();
      if (p.smooth) {
        ctx.arc(x, y, 5, 0, Math.PI * 2);
      } else {
        ctx.rect(x - 4.5, y - 4.5, 9, 9);
      }
      ctx.fillStyle = accent;
      ctx.fill();
      ctx.strokeStyle = "#0b0e12";
      ctx.stroke();
    }
  }
}

// ---- point editing (freeform only)

let dragging = -1;

function pointAt(mx, my) {
  if (!free) return -1;
  const ps = plan?.scale || 1;
  for (let i = 0; i < free.length; i++) {
    const [x, y] = toPx([free[i].x * ps, free[i].y * ps]);
    if (Math.hypot(mx - x, my - y) < 9) return i;
  }
  return -1;
}

/// Angle folded into the editable wedge, for inserting and dragging under
/// symmetry: whatever wedge the mouse is in, the edit lands in wedge 0.
function foldToWedge(x, y) {
  if (!free || symmetry <= 1) return [x, y];
  const a = Math.atan2(y, x);
  const wedge = (Math.PI * 2) / symmetry;
  const folded = ((a % wedge) + wedge) % wedge;
  const r = Math.hypot(x, y);
  return [r * Math.cos(folded), r * Math.sin(folded)];
}

cv.addEventListener("pointerdown", (e) => {
  if (!free) return;
  // Left button only: middle and side buttons were adding points too.
  if (e.button !== 0) return;
  const rect = cv.getBoundingClientRect();
  const mx = e.clientX - rect.left;
  const my = e.clientY - rect.top;
  const hit = pointAt(mx, my);
  if (hit >= 0) {
    dragging = hit;
    cv.setPointerCapture(e.pointerId);
    return;
  }
  // Insert only when the click actually lands NEAR the outline: a click
  // in empty space (or outside the note entirely) is not a request for a
  // point there.
  if (plan?.outer) {
    let best = Infinity;
    const n = plan.outer.length;
    for (let i = 0; i < n; i++) {
      const [ax, ay] = toPx(plan.outer[i]);
      const [bx, by] = toPx(plan.outer[(i + 1) % n]);
      const dx = bx - ax, dy = by - ay;
      const len2 = dx * dx + dy * dy || 1e-9;
      const t = Math.max(0, Math.min(1, ((mx - ax) * dx + (my - ay) * dy) / len2));
      const qx = ax + t * dx, qy = ay + t * dy;
      best = Math.min(best, Math.hypot(mx - qx, my - qy));
    }
    if (best > 18) return;
  }
  // Click near the outline: insert a point there (folded into the wedge).
  let [wx, wy] = toWorld(mx, my);
  const ps0 = plan?.scale || 1;
  wx /= ps0;
  wy /= ps0;
  [wx, wy] = foldToWedge(wx, wy);
  [wx, wy] = clampUnit(wx, wy);
  // Insert in angular order so the wedge stays a simple arc of points.
  // Angles normalised to [0, 2pi): atan2 goes negative below the x-axis,
  // and comparing raw values put every lower-half click at index 0,
  // scrambling the outline in no-symmetry mode.
  const norm = (a) => ((a % (Math.PI * 2)) + Math.PI * 2) % (Math.PI * 2);
  const ang = norm(Math.atan2(wy, wx));
  let at = free.length;
  for (let i = 0; i < free.length; i++) {
    if (norm(Math.atan2(free[i].y, free[i].x)) > ang) {
      at = i;
      break;
    }
  }
  free.splice(at, 0, { x: wx, y: wy, smooth: true });
  dragging = at;
  cv.setPointerCapture(e.pointerId);
  scheduleAll();
});

cv.addEventListener("pointermove", (e) => {
  if (dragging < 0 || !free) return;
  const rect = cv.getBoundingClientRect();
  const mx = e.clientX - rect.left;
  const my = e.clientY - rect.top;
  let [wx, wy] = toWorld(mx, my);
  {
    const ps1 = plan?.scale || 1;
    wx /= ps1;
    wy /= ps1;
  }
  // Clamp into the wedge instead of folding: folding teleports a point
  // sitting near the boundary to the wedge's far side the moment the
  // cursor slips across, which reads as the point jumping away.
  if (free && symmetry > 1) {
    const wedge = (Math.PI * 2) / symmetry;
    let a = Math.atan2(wy, wx);
    if (a < 0) a += Math.PI * 2;
    a = Math.min(a, wedge);
    const rr = Math.hypot(wx, wy);
    wx = rr * Math.cos(a);
    wy = rr * Math.sin(a);
  }
  [wx, wy] = clampUnit(wx, wy);
  free[dragging] = { ...free[dragging], x: wx, y: wy };
  scheduleAll();
});

cv.addEventListener("pointerup", () => (dragging = -1));

cv.addEventListener("dblclick", (e) => {
  if (!free) return;
  const rect = cv.getBoundingClientRect();
  const hit = pointAt(e.clientX - rect.left, e.clientY - rect.top);
  if (hit >= 0) {
    free[hit].smooth = !free[hit].smooth;
    scheduleAll();
  }
});

cv.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  if (!free || free.length <= 3) return;
  const rect = cv.getBoundingClientRect();
  const hit = pointAt(e.clientX - rect.left, e.clientY - rect.top);
  if (hit >= 0) {
    free.splice(hit, 1);
    // A removal invalidates whatever a concurrent left-drag was holding.
    dragging = -1;
    scheduleAll();
  }
});

// ------------------------------------------------------------ preview

function shapePayload() {
  const pts = outline();
  return {
    outline: pts,
    hollow: P.hollow ? P.thick : null,
    depth: P.depth,
    colors: colors.slice(),
  };
}

// The preview is GAMEPLAY: a nine-note snake over the whole grid on a
// steady beat, looping. A self-paced loop renders as fast as the backend
// returns frames and always reads the CURRENT shape, so edits show up in
// the running animation without any scheduling dance.
const LOOP_MS = 2700;
// The snake's centre-cell note sits exactly at the hit plane at 1700 ms:
// the still renders that moment, so it is always one big centred note.
const STILL_MS = 1700;
let anim0 = performance.now();
let animOn = true;
let stillTimer = null;
let stillBusy = false;

async function refreshStill() {
  if (stillBusy) return;
  stillBusy = true;
  try {
    const url = await invoke("designer_preview", { shape: shapePayload(), timeMs: STILL_MS });
    $("dz-still").src = url;
  } catch {
    // The animated loop already reports outline errors.
  } finally {
    stillBusy = false;
  }
}

async function animLoop() {
  let lastErr = "";
  while (true) {
    if (!animOn || document.hidden) {
      await new Promise((r) => setTimeout(r, 150));
      continue;
    }
    const t = (performance.now() - anim0) % LOOP_MS;
    try {
      const url = await invoke("designer_preview", { shape: shapePayload(), timeMs: t });
      $("dz-preview").src = url;
      if (lastErr) {
        lastErr = "";
        setStatus("");
      }
    } catch (e) {
      const msg = String(e);
      if (msg !== lastErr) {
        lastErr = msg;
        setStatus(msg, true);
      }
      await new Promise((r) => setTimeout(r, 250));
    }
  }
}

function setStatus(text, err) {
  const el = $("dz-status");
  el.textContent = text;
  el.classList.toggle("err", !!err);
}

function scheduleAll() {
  // The contours come from the backend and repaint on arrival (sub-ms
  // maths, coalesced during a drag); the animated preview reads the
  // current shape every frame on its own. The still re-renders shortly
  // after the last change.
  refreshPlan();
  clearTimeout(stillTimer);
  stillTimer = setTimeout(refreshStill, 250);
}

// ------------------------------------------------------------ controls

function bindSlider(id, get, set, fmt) {
  const el = $(id);
  el.addEventListener("input", () => {
    set(Number(el.value));
    $(`${id}-val`) && ($(`${id}-val`).textContent = fmt(Number(el.value)));
    scheduleAll();
  });
  $(`${id}-val`) && ($(`${id}-val`).textContent = fmt(get()));
}

function wireControls() {
  $("dz-shape").addEventListener("change", () => {
    P.shape = $("dz-shape").value;
    $("dz-n-row").hidden = P.shape !== "ngon";
    scheduleAll();
  });
  bindSlider("dz-n", () => P.n, (v) => (P.n = v), (v) => String(v));
  bindSlider("dz-round", () => P.round * 100, (v) => (P.round = v / 100), (v) => `${v}%`);
  bindSlider("dz-thick", () => P.thick * 100, (v) => (P.thick = v / 100), (v) => `${v}%`);
  bindSlider("dz-depth", () => P.depth * 100, (v) => (P.depth = v / 100), (v) => `${v}%`);
  bindSlider("dz-waves", () => P.waves, (v) => (P.waves = v), (v) => String(v));
  bindSlider("dz-wavedepth", () => P.waveDepth * 100, (v) => (P.waveDepth = v / 100), (v) => `${v}%`);
  bindSlider("dz-twist", () => P.twist, (v) => (P.twist = v), (v) => `${v}°`);
  $("dz-waveform").addEventListener("change", () => {
    P.waveform = $("dz-waveform").value;
    scheduleAll();
  });
  $("dz-hollow").addEventListener("change", () => {
    P.hollow = $("dz-hollow").checked;
    $("dz-thick-row").hidden = !P.hollow;
    scheduleAll();
  });

  $("dz-to-points").addEventListener("click", () => {
    // A five-wave shape frozen into four-fold symmetry is a different
    // shape: converting adopts the wave count as the symmetry when waves
    // are active, so the points reproduce what is on screen.
    if (P.waves >= 2 && P.waves <= 12) {
      const sel = $("dz-symmetry");
      const fit = [...sel.options].some((o) => Number(o.value) === P.waves)
        ? P.waves
        : 1;
      symmetry = fit;
      sel.value = String(fit);
    }
    // Freeze the current outline into a handful of control points per
    // wedge: dense sampling would give 160 handles nobody wants to drag.
    const pts = paramOutline(symmetry * 8);
    const per = pts.length / symmetry;
    free = pts.slice(0, per).map(([x, y]) => ({ x, y, smooth: true }));
    $("dz-back-params").hidden = false;
    $("dz-to-points").hidden = true;
    $("dz-symmetry").disabled = true; // point set is per-wedge now
    scheduleAll();
  });
  $("dz-back-params").addEventListener("click", () => {
    free = null;
    $("dz-back-params").hidden = true;
    $("dz-to-points").hidden = false;
    $("dz-symmetry").disabled = false;
    scheduleAll();
  });
  $("dz-symmetry").addEventListener("change", () => {
    symmetry = Number($("dz-symmetry").value);
    scheduleAll();
  });

  $("dz-color-add").addEventListener("click", () => {
    if (colors.length >= 12) return;
    colors.push("#ffffff");
    renderColors();
    scheduleAll();
  });
  $("dz-reset").addEventListener("click", () => {
    location.reload();
  });
  $("dz-save").addEventListener("click", async () => {
    const path = await dialog.save({
      defaultPath: "my-note.rhs",
      filters: [{ name: "Rhythia skin", extensions: ["rhs"] }],
    });
    if (!path) return;
    try {
      const saved = await invoke("designer_export", { shape: shapePayload(), path });
      setStatus(`Saved: ${saved} - load it via Config in the main window, or import it in the game.`);
    } catch (e) {
      setStatus(String(e), true);
    }
  });
}

let dragColorIdx = -1;

function renderColors() {
  const box = $("dz-colors");
  box.innerHTML = "";
  colors.forEach((c, i) => {
    const row = document.createElement("div");
    row.className = "dz-color-row";
    // The order IS the beat order in the preview loop, so it is worth
    // rearranging: drag a row where you want its colour to land.
    row.draggable = true;
    const grip = document.createElement("span");
    grip.className = "grip";
    grip.textContent = "⠿";
    grip.title = "Drag to reorder: the notes play the colors in this order";
    row.appendChild(grip);
    row.addEventListener("dragstart", () => {
      dragColorIdx = i;
      row.classList.add("dragging");
    });
    row.addEventListener("dragend", () => {
      dragColorIdx = -1;
      row.classList.remove("dragging");
    });
    row.addEventListener("dragover", (e) => e.preventDefault());
    row.addEventListener("drop", (e) => {
      e.preventDefault();
      if (dragColorIdx < 0 || dragColorIdx === i) return;
      const [moved] = colors.splice(dragColorIdx, 1);
      colors.splice(i, 0, moved);
      dragColorIdx = -1;
      renderColors();
      scheduleAll();
    });
    const pick = document.createElement("input");
    pick.type = "color";
    pick.value = c;
    const hex = document.createElement("input");
    hex.type = "text";
    hex.className = "mono hex";
    hex.value = c;
    const del = document.createElement("button");
    del.className = "btn small ghost";
    del.textContent = "✕";
    del.title = "Remove this color";
    pick.addEventListener("input", () => {
      colors[i] = pick.value;
      hex.value = pick.value;
      scheduleAll();
    });
    hex.addEventListener("change", () => {
      const v = hex.value.trim();
      if (/^#[0-9a-fA-F]{6}$/.test(v)) {
        colors[i] = v;
        pick.value = v;
        scheduleAll();
      } else {
        hex.value = colors[i];
      }
    });
    del.addEventListener("click", () => {
      if (colors.length <= 1) return;
      colors.splice(i, 1);
      renderColors();
      scheduleAll();
    });
    row.append(pick, hex, del);
    box.appendChild(row);
  });
}

wireControls();
renderColors();
$("dz-n-row").hidden = true;
fitCanvas();
refreshPlan();
animLoop();
