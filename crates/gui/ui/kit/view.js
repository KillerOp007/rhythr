// The front view: an orthographic Canvas 2D drawing of what the kernel
// produced (normalised, note units), each part's own outline, and the
// selected part's handles. The only place geometry is edited.
import {
  makeView, unit, toPx, toWorld, zoomAt, partRect, rectCorners, rectContains, handleAt, scaleFrom, angleFrom, HITBOX,
} from "./view-math.js";

const GLASS_ALPHA = 0.57;
const HANDLE_PX = 7;
const ROTATE_PX = 24;

export function createView(canvas, store, hooks) {
  const ctx = canvas.getContext("2d");
  const view = makeView();
  const size = { w: 520, h: 520 };
  let hatch = null;
  let spaceDown = false;
  let gesture = null;

  // A hidden pane reports a zero box; fitting or zooming against it would
  // put NaN into the view, so both wait for a real size.
  function boxHasArea() {
    const box = canvas.parentElement;
    return box.clientWidth > 0 && box.clientHeight > 0;
  }

  function fit() {
    if (!boxHasArea()) return;
    const box = canvas.parentElement;
    const dpr = window.devicePixelRatio || 1;
    size.w = Math.max(200, box.clientWidth);
    size.h = Math.max(200, box.clientHeight);
    canvas.width = Math.round(size.w * dpr);
    canvas.height = Math.round(size.h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    draw();
  }

  function path(shapes) {
    const p = new Path2D();
    for (const shape of shapes) {
      for (const contour of shape) {
        contour.forEach((pt, i) => {
          const [x, y] = toPx(view, size, pt);
          if (i === 0) p.moveTo(x, y);
          else p.lineTo(x, y);
        });
        p.closePath();
      }
    }
    return p;
  }

  function hatchPattern() {
    if (hatch) return hatch;
    const c = document.createElement("canvas");
    c.width = 8;
    c.height = 8;
    const g = c.getContext("2d");
    g.strokeStyle = "rgba(255, 120, 90, 0.5)";
    g.lineWidth = 1.5;
    g.beginPath();
    g.moveTo(0, 8);
    g.lineTo(8, 0);
    g.stroke();
    hatch = ctx.createPattern(c, "repeat");
    return hatch;
  }

  function box() {
    const a = toPx(view, size, [-1, 1]);
    const b = toPx(view, size, [1, -1]);
    ctx.strokeStyle = "rgba(255,255,255,0.35)";
    ctx.lineWidth = 1;
    ctx.setLineDash([]);
    ctx.strokeRect(a[0], a[1], b[0] - a[0], b[1] - a[1]);
    const ha = toPx(view, size, [-HITBOX, HITBOX]);
    const hb = toPx(view, size, [HITBOX, -HITBOX]);
    ctx.setLineDash([4, 4]);
    ctx.strokeStyle = "rgba(255,255,255,0.18)";
    ctx.strokeRect(ha[0], ha[1], hb[0] - ha[0], hb[1] - ha[1]);
    ctx.setLineDash([]);
  }

  function draw() {
    const s = store.state;
    ctx.clearRect(0, 0, size.w, size.h);
    box();
    const ev = s.evaluated;
    const colour = (s.design.colors && s.design.colors[0]) || "#ffffff";
    if (ev) {
      // Outer contours run counterclockwise and holes clockwise, so the
      // nonzero rule cuts the holes without any extra bookkeeping.
      ctx.fillStyle = colour;
      for (const iv of ev.intervals) ctx.fill(path(iv.solid), "nonzero");
      ctx.globalAlpha = GLASS_ALPHA;
      for (const iv of ev.intervals) ctx.fill(path(iv.glass), "nonzero");
      ctx.globalAlpha = 1;
      for (const part of s.design.parts) {
        const geom = ev.parts.find((g) => g.id === part.id);
        if (!geom || !part.visible) continue;
        const p = path(geom.shapes);
        const sel = part.id === s.selected;
        if (part.mode === "cut") {
          ctx.fillStyle = hatchPattern();
          ctx.fill(p, "nonzero");
          ctx.setLineDash([5, 3]);
          ctx.strokeStyle = sel ? "rgba(255,140,110,0.95)" : "rgba(255,140,110,0.6)";
        } else {
          ctx.setLineDash([]);
          ctx.strokeStyle = sel ? "rgba(120,200,255,0.95)" : "rgba(255,255,255,0.25)";
        }
        ctx.lineWidth = sel ? 1.5 : 1;
        ctx.stroke(p);
        ctx.setLineDash([]);
      }
    }
    const part = store.part();
    if (part && ev) handles(part, ev.scale);
  }

  function handles(part, scale) {
    const r = partRect(part.transform, scale);
    const g = rectCorners(r);
    ctx.strokeStyle = "rgba(120,200,255,0.8)";
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 3]);
    ctx.beginPath();
    g.corners.forEach((c, i) => {
      const [x, y] = toPx(view, size, c);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.closePath();
    ctx.stroke();
    ctx.setLineDash([]);
    if (part.locked) return;
    ctx.fillStyle = "#fff";
    for (const c of [...g.corners, ...g.edges]) {
      const [x, y] = toPx(view, size, c);
      ctx.fillRect(x - 3.5, y - 3.5, 7, 7);
    }
    const [tx, ty] = toPx(view, size, g.top);
    const [cx, cy] = toPx(view, size, [r.cx, r.cy]);
    const len = Math.hypot(tx - cx, ty - cy) || 1;
    const rx = tx + ((tx - cx) / len) * ROTATE_PX;
    const ry = ty + ((ty - cy) / len) * ROTATE_PX;
    ctx.beginPath();
    ctx.moveTo(tx, ty);
    ctx.lineTo(rx, ry);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(rx, ry, 5, 0, Math.PI * 2);
    ctx.fill();
  }

  function rotateHandleHit(part, scale, mx, my) {
    const r = partRect(part.transform, scale);
    const g = rectCorners(r);
    const [tx, ty] = toPx(view, size, g.top);
    const [cx, cy] = toPx(view, size, [r.cx, r.cy]);
    const len = Math.hypot(tx - cx, ty - cy) || 1;
    const rx = tx + ((tx - cx) / len) * ROTATE_PX;
    const ry = ty + ((ty - cy) / len) * ROTATE_PX;
    return Math.hypot(mx - rx, my - ry) <= HANDLE_PX + 2;
  }

  function pick(wp) {
    // Top of the list draws on top, so it is picked first.
    const parts = store.state.design.parts;
    const scale = store.state.evaluated ? store.state.evaluated.scale : 1;
    for (let i = parts.length - 1; i >= 0; i--) {
      const p = parts[i];
      if (!p.visible) continue;
      if (rectContains(partRect(p.transform, scale), wp)) return p;
    }
    return null;
  }

  canvas.addEventListener("pointerdown", (e) => {
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    canvas.setPointerCapture(e.pointerId);
    if (e.button === 1 || spaceDown) {
      gesture = { kind: "pan", mx, my, cx: view.cx, cy: view.cy };
      return;
    }
    if (e.button !== 0) return;
    const wp = toWorld(view, size, mx, my);
    const s = store.state;
    const scale = s.evaluated ? s.evaluated.scale : 1;
    const sel = store.part();
    if (sel && !sel.locked && s.evaluated) {
      const r = partRect(sel.transform, scale);
      if (rotateHandleHit(sel, scale, mx, my)) {
        gesture = { kind: "rotate", id: sel.id };
        return;
      }
      const h = handleAt(r, wp, HANDLE_PX / unit(view, size));
      if (h) {
        gesture = { kind: "scale", id: sel.id, handle: h, r };
        return;
      }
    }
    const hit = pick(wp);
    if (!hit) {
      store.select(null);
      return;
    }
    if (hit.id !== s.selected) store.select(hit.id);
    if (!hit.locked) {
      gesture = { kind: "move", id: hit.id, wp, x: hit.transform.x, y: hit.transform.y, scale };
    }
  });

  canvas.addEventListener("pointermove", (e) => {
    if (!gesture || !canvas.hasPointerCapture(e.pointerId)) return;
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    if (gesture.kind === "pan") {
      const u = unit(view, size);
      view.cx = gesture.cx - (mx - gesture.mx) / u;
      view.cy = gesture.cy + (my - gesture.my) / u;
      draw();
      return;
    }
    const wp = toWorld(view, size, mx, my);
    const scale = store.state.evaluated ? store.state.evaluated.scale : 1;
    // Labels carry the part id: undo coalescing merges by label, and a
    // gesture on one part must never merge into a gesture on another.
    if (gesture.kind === "move") {
      const dx = (wp[0] - gesture.wp[0]) / gesture.scale;
      const dy = (wp[1] - gesture.wp[1]) / gesture.scale;
      store.updatePart(gesture.id, `move:${gesture.id}`, (p) => {
        p.transform.x = clamp(gesture.x + dx, -1.5, 1.5);
        p.transform.y = clamp(gesture.y + dy, -1.5, 1.5);
      });
    } else if (gesture.kind === "scale") {
      // Shift keeps the proportions on corners only; an edge handle moves one axis.
      const keep = e.shiftKey && gesture.handle.length === 2;
      const { hw, hh } = scaleFrom(gesture.r, gesture.handle, wp, keep);
      store.updatePart(gesture.id, `scale:${gesture.id}`, (p) => {
        p.transform.w = clamp(hw / scale, 0.02, 1.5);
        p.transform.h = clamp(hh / scale, 0.02, 1.5);
      });
    } else if (gesture.kind === "rotate") {
      const part = store.part(gesture.id);
      const r = partRect(part.transform, scale);
      let a = angleFrom(r, wp);
      if (e.shiftKey) a = Math.round(a / 15) * 15;
      a = ((((a + 180) % 360) + 360) % 360) - 180;
      store.updatePart(gesture.id, `rotate:${gesture.id}`, (p) => (p.transform.rotation = Math.round(a * 10) / 10));
    }
    hooks.onEdit();
  });

  // Every button takes pointer capture on pointerdown (a right click too,
  // which starts no gesture), so every release gives it back.
  const end = (e) => {
    if (canvas.hasPointerCapture(e.pointerId)) canvas.releasePointerCapture(e.pointerId);
    gesture = null;
  };
  canvas.addEventListener("pointerup", end);
  canvas.addEventListener("pointercancel", end);

  canvas.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      if (!boxHasArea()) return;
      const rect = canvas.getBoundingClientRect();
      zoomAt(view, size, e.clientX - rect.left, e.clientY - rect.top, e.deltaY < 0 ? 1.15 : 1 / 1.15);
      hooks.onZoom(Math.round(view.z * 100));
      draw();
    },
    { passive: false },
  );

  window.addEventListener("keydown", (e) => {
    if (e.code === "Space" && !isTyping()) {
      spaceDown = true;
      e.preventDefault();
    }
  });
  window.addEventListener("keyup", (e) => {
    if (e.code === "Space") spaceDown = false;
  });

  function resetView() {
    view.z = 1;
    view.cx = 0;
    view.cy = 0;
    hooks.onZoom(100);
    draw();
  }

  new ResizeObserver(fit).observe(canvas.parentElement);
  return { draw, fit, resetView };
}

export function isTyping() {
  const t = document.activeElement && document.activeElement.tagName;
  return t === "INPUT" || t === "SELECT" || t === "TEXTAREA";
}

const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
