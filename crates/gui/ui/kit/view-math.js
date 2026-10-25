// crates/gui/ui/kit/view-math.js
// Pure maths for the front view: a screen-space zoom and pan over note
// units (half-extent 1 is the box), part rectangles in the part's own
// frame, hit tests and handle geometry. No DOM here, so it is testable.

// The game's true hit area over the note's visual half-size.
export const HITBOX = 0.56875 / 0.45;
export const MIN_HALF = 0.02;

export function makeView() {
  return { z: 1, cx: 0, cy: 0 };
}

export function unit(view, size) {
  return (Math.min(size.w, size.h) / 2.9) * view.z;
}

export function toPx(view, size, p) {
  const u = unit(view, size);
  return [size.w / 2 + (p[0] - view.cx) * u, size.h / 2 - (p[1] - view.cy) * u];
}

export function toWorld(view, size, x, y) {
  const u = unit(view, size);
  return [(x - size.w / 2) / u + view.cx, -(y - size.h / 2) / u + view.cy];
}

// The point under the cursor stays under the cursor.
export function zoomAt(view, size, x, y, factor) {
  const [wx, wy] = toWorld(view, size, x, y);
  view.z = Math.max(0.4, Math.min(12, view.z * factor));
  const u = unit(view, size);
  view.cx = wx - (x - size.w / 2) / u;
  view.cy = wy + (y - size.h / 2) / u;
  return view;
}

// A part's rectangle in drawing units: the document transform scaled by
// the kernel's normalisation factor, so handles sit on the drawn shape.
export function partRect(t, scale) {
  return { cx: t.x * scale, cy: t.y * scale, hw: t.w * scale, hh: t.h * scale, rot: (t.rotation * Math.PI) / 180 };
}

export function rectCorners(r) {
  const c = Math.cos(r.rot);
  const s = Math.sin(r.rot);
  const at = (dx, dy) => [r.cx + dx * c - dy * s, r.cy + dx * s + dy * c];
  return {
    corners: [at(-r.hw, -r.hh), at(r.hw, -r.hh), at(r.hw, r.hh), at(-r.hw, r.hh)],
    edges: [at(0, -r.hh), at(r.hw, 0), at(0, r.hh), at(-r.hw, 0)],
    top: at(0, r.hh),
  };
}

export function toLocal(r, p) {
  const c = Math.cos(-r.rot);
  const s = Math.sin(-r.rot);
  const dx = p[0] - r.cx;
  const dy = p[1] - r.cy;
  return [dx * c - dy * s, dx * s + dy * c];
}

export function rectContains(r, p) {
  const [lx, ly] = toLocal(r, p);
  return Math.abs(lx) <= r.hw && Math.abs(ly) <= r.hh;
}

// Corners then edges, in the order rectCorners returns them.
export const HANDLES = ["sw", "se", "ne", "nw", "s", "e", "n", "w"];

export function handleAt(r, p, tolerance) {
  const g = rectCorners(r);
  const all = [...g.corners, ...g.edges];
  for (let i = 0; i < all.length; i++) {
    if (Math.hypot(all[i][0] - p[0], all[i][1] - p[1]) <= tolerance) return HANDLES[i];
  }
  return null;
}

export const MAX_HALF = 1.5;

// Scaling about the centre: the pointer's local coordinate sets the half
// extent on the dragged axis; a corner drags both. With the proportions
// kept, the size limit caps the common factor, so the shape stops growing
// as a whole instead of one side stopping while the other goes on.
export function scaleFrom(r, handle, p, keepAspect, maxHalf = MAX_HALF) {
  const [lx, ly] = toLocal(r, p);
  let hw = r.hw;
  let hh = r.hh;
  if (handle.includes("e") || handle.includes("w")) hw = Math.max(MIN_HALF, Math.abs(lx));
  if (handle.includes("n") || handle.includes("s")) hh = Math.max(MIN_HALF, Math.abs(ly));
  if (keepAspect && r.hw > 0 && r.hh > 0) {
    const k = Math.min(Math.max(hw / r.hw, hh / r.hh), maxHalf / r.hw, maxHalf / r.hh);
    hw = r.hw * k;
    hh = r.hh * k;
  }
  return { hw: Math.min(hw, maxHalf), hh: Math.min(hh, maxHalf) };
}

// The rotation handle sits above the top edge; straight up is zero.
export function angleFrom(r, p) {
  return (Math.atan2(p[1] - r.cy, p[0] - r.cx) * 180) / Math.PI - 90;
}

// A part's unit frame (where its points live: half-extent 1 before the
// transform) to the world and back, the kernel's order: scale, turn, move.
export function unitToWorld(r, q) {
  const c = Math.cos(r.rot);
  const s = Math.sin(r.rot);
  const x = q[0] * r.hw;
  const y = q[1] * r.hh;
  return [r.cx + x * c - y * s, r.cy + x * s + y * c];
}

export function worldToUnit(r, p) {
  const [lx, ly] = toLocal(r, p);
  return [lx / (r.hw || 1e-9), ly / (r.hh || 1e-9)];
}

// The nearest of the loops' points (pixel positions) within `radius`
// pixels of m, or null.
export function nearestPoint(loopsPx, m, radius) {
  let best = null;
  loopsPx.forEach((loop, li) => {
    loop.forEach((p, i) => {
      const d = Math.hypot(p[0] - m[0], p[1] - m[1]);
      if (d <= radius && (!best || d < best.dist)) best = { loop: li, index: i, dist: d };
    });
  });
  return best;
}

// The nearest closed-loop segment to m (pixels): the loop, the index a new
// point takes to land on that segment (after its start point), the distance
// and the position along the segment. Null for empty loops.
export function nearestSegment(loopsPx, m) {
  let best = null;
  loopsPx.forEach((loop, li) => {
    const n = loop.length;
    for (let i = 0; i < n; i++) {
      const a = loop[i];
      const b = loop[(i + 1) % n];
      const dx = b[0] - a[0];
      const dy = b[1] - a[1];
      const len2 = dx * dx + dy * dy || 1e-9;
      const t = Math.max(0, Math.min(1, ((m[0] - a[0]) * dx + (m[1] - a[1]) * dy) / len2));
      const d = Math.hypot(a[0] + t * dx - m[0], a[1] + t * dy - m[1]);
      if (!best || d < best.dist) best = { loop: li, index: i + 1, dist: d, t };
    }
  });
  return best;
}

// Snaps a unit-frame coordinate to a grid of `step`.
export function snapUnit(q, step) {
  return [Math.round(q[0] / step) * step, Math.round(q[1] / step) * step];
}

// Snapping: a value is pulled to the nearest grid line or anchor when that
// lies within `tol` (world units, the pixel radius over the view's unit);
// anything farther passes through, so a deliberate placement is never
// fought. Results carry three decimals like every other edit.
export const SNAP_GRID = 0.05;
export const SNAP_PX = 6;
export function snapValue(v, tol, anchors = [], grid = SNAP_GRID) {
  let best = v;
  let bd = Infinity;
  for (const c of [Math.round(v / grid) * grid, ...anchors]) {
    const d = Math.abs(v - c);
    if (d <= tol && d < bd) {
      bd = d;
      best = c;
    }
  }
  // Plus zero: a value rounded to the grid from below would be minus zero.
  return Math.round(best * 1000) / 1000 + 0;
}
// A part's centre: the grid plus the box centre and edges.
export function snapPosition(x, y, tol) {
  return [snapValue(x, tol, [-1, 0, 1]), snapValue(y, tol, [-1, 0, 1])];
}
// Half extents: the grid plus half the box and the full box.
export function snapSize(hw, hh, tol) {
  return [snapValue(hw, tol, [0.5, 1]), snapValue(hh, tol, [0.5, 1])];
}
export function snapAngle(deg, tolDeg = 3) {
  const s = Math.round(deg / 15) * 15;
  return Math.abs(deg - s) <= tolDeg ? s : deg;
}

// What the front view fills a depth band with: the highest band keeps the
// note colour, lower bands fade toward dark (or toward light, for a dark
// colour) by how far they lie below the top, so a raised or recessed part
// reads as such head-on. A drop of DEPTH_FULL_SHADE note units is the
// strongest shade.
export const DEPTH_FULL_SHADE = 0.6;
export function depthShade(hex, z, top) {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(String(hex || ""));
  const rgb = m ? [1, 2, 3].map((i) => parseInt(m[i], 16)) : [255, 255, 255];
  const drop = Math.max(0, (top ?? z) - z);
  const t = Math.min(1, drop / DEPTH_FULL_SHADE) * 0.55;
  const luma = (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]) / 255;
  const toward = luma < 0.35 ? 255 : 0;
  const out = rgb.map((v) => Math.round(v + (toward - v) * t));
  return `rgb(${out[0]},${out[1]},${out[2]})`;
}
