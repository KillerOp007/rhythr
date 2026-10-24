// The kernel returns geometry normalised to the note box (the game's hard
// size cap). The editor works in design units instead: a part keeps its
// drawn size and position while it is moved, hidden or resized, and the
// box the game will scale the design into is drawn as an overlay that
// follows the largest part. Normalised coordinates divided by the factor
// are design coordinates.
export function toDesignUnits(ev) {
  const scale = Number(ev && ev.scale);
  if (!Number.isFinite(scale) || scale <= 0 || scale === 1) return ev;
  const k = 1 / scale;
  const shapes = (ss) => ss.map((shape) => shape.map((contour) => contour.map((p) => [p[0] * k, p[1] * k])));
  return {
    ...ev,
    parts: ev.parts.map((p) => ({ id: p.id, shapes: shapes(p.shapes) })),
    intervals: ev.intervals.map((iv) => ({ z0: iv.z0, z1: iv.z1, solid: shapes(iv.solid), glass: shapes(iv.glass) })),
  };
}

// The note box in design units: the extent the game maps onto 1.0.
export function boxExtent(ev) {
  const scale = Number(ev && ev.scale);
  if (!Number.isFinite(scale) || scale <= 0) return 1;
  return 1 / scale;
}
