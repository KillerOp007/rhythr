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
