// crates/gui/ui/kit/thumb.js
// A small picture of an evaluated design, drawn the way the front view
// draws it: the solid bands back to front with the depth shading, then the
// glass once at the game's alpha. The layer model is pure so a test can
// check it without a canvas; drawThumbnail paints it.

import { depthShade, GLASS_ALPHA } from "./view-math.js";

export { GLASS_ALPHA };
export const THUMB_PX = 96;

// Layers in paint order: { fill, alpha, polys }, polys in canvas pixels. One
// note unit is size / 2.4 pixels about the centre, so the box and a part
// that reaches a little past it both stay inside the picture; y points up
// in note units and down on the canvas.
export function thumbLayers(ev, colour, size = THUMB_PX) {
  const scale = size / 2.4;
  const px = ([x, y]) => [size / 2 + x * scale, size / 2 - y * scale];
  const polys = (shapes) => shapes.flat(1).map((contour) => contour.map(px));
  const top = ev.intervals.reduce((m, iv) => (iv.solid.length ? Math.max(m, iv.z1) : m), -Infinity);
  const layers = [];
  for (const iv of ev.intervals) {
    if (!iv.solid.length) continue;
    layers.push({ fill: depthShade(colour, iv.z1, top), alpha: 1, polys: polys(iv.solid) });
  }
  const glass = ev.intervals.flatMap((iv) => polys(iv.glass));
  if (glass.length) layers.push({ fill: colour, alpha: GLASS_ALPHA, polys: glass });
  return layers;
}

export function drawThumbnail(canvas, ev, colour) {
  canvas.width = THUMB_PX;
  canvas.height = THUMB_PX;
  const ctx = canvas.getContext("2d");
  ctx.clearRect(0, 0, THUMB_PX, THUMB_PX);
  for (const layer of thumbLayers(ev, colour)) {
    const p = new Path2D();
    for (const poly of layer.polys) {
      poly.forEach(([x, y], i) => (i ? p.lineTo(x, y) : p.moveTo(x, y)));
      p.closePath();
    }
    ctx.globalAlpha = layer.alpha;
    ctx.fillStyle = layer.fill;
    ctx.fill(p, "nonzero");
  }
  ctx.globalAlpha = 1;
}
