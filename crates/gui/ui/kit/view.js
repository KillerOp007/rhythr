// The front view: an orthographic Canvas 2D drawing of what the kernel
// produced, each part's own outline, and the selected part's handles. The
// only place geometry is edited. Design units are note units: the note
// box is fixed at the unit square, the kernel cuts every part to it and
// never rescales anything, so a part keeps its drawn size whatever moves
// or hides, and the outlines show exactly what the note keeps.
//
// Depth shows head-on: every band of the note is filled from the back
// forward, the highest band in the note colour and lower bands shaded by
// how far they lie below it, so a raised or recessed part reads as such.
//
// Point editing (a Points part, entered by double-click or the inspector):
// the part's control points draw as handles in the part's own frame; drag
// moves one, a click on the outline inserts one, double-click switches
// smooth and corner, right-click removes, a click elsewhere finishes.
import {
  makeView, unit, toPx, toWorld, zoomAt, partRect, rectCorners, rectContains, handleAt, scaleFrom, angleFrom, HITBOX,
  unitToWorld, worldToUnit, nearestPoint, nearestSegment, snapUnit, depthShade,
  snapValue, snapPosition, snapSize, snapAngle, snapTolerance,
} from "./view-math.js";
import { loopsOf } from "./store.js";

const GLASS_ALPHA = 0.57;
const HANDLE_PX = 7;
const ROTATE_PX = 24;
const POINT_PX = 9;
const SEGMENT_PX = 12;
const POINT_SNAP = 0.05;

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

  // Pointer positions in the canvas's own client box: the 1 px border sits
  // outside it, and the bitmap is sized to it, so they line up exactly.
  function local(e) {
    const rect = canvas.getBoundingClientRect();
    return [e.clientX - rect.left - canvas.clientLeft, e.clientY - rect.top - canvas.clientTop];
  }

  function fit() {
    if (!boxHasArea()) return;
    const dpr = window.devicePixelRatio || 1;
    size.w = Math.max(200, canvas.clientWidth);
    size.h = Math.max(200, canvas.clientHeight);
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

  // The note box, fixed: what lies inside is the note, a part past the
  // edge is cut there. The dashed square is the game's true hit area.
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

  // The part whose points are being edited, if it is still a visible Points
  // part (the store ends the mode when the part goes or changes shape).
  function editingPart() {
    const id = store.state.ui.pointEdit;
    if (!id) return null;
    const p = store.part(id);
    return p && p.visible && loopsOf(p.shape) ? p : null;
  }

  // The control points in pixels, loop by loop, through the part's transform.
  function pointLoopsPx(part) {
    const r = partRect(part.transform, 1);
    return (loopsOf(part.shape) || []).map((loop) => loop.map((q) => toPx(view, size, unitToWorld(r, [q.x, q.y]))));
  }

  function draw() {
    const s = store.state;
    ctx.clearRect(0, 0, size.w, size.h);
    box();
    const ev = s.evaluated;
    const colour = (s.design.colors && s.design.colors[0]) || "#ffffff";
    // The last good evaluation is kept through an error so the outlines
    // stay under the cursor, but its fills are stale (a part hidden since
    // would still show as solid), so an error shows outlines only.
    const stale = Boolean(s.evalError);
    if (ev && !stale) {
      // Bands come back to front. Outer contours run counterclockwise and
      // holes clockwise, so the nonzero rule cuts the holes without any
      // extra bookkeeping; a higher band paints over a lower one, so what
      // shows at any spot is the band nearest the player there.
      const top = ev.intervals.reduce((m, iv) => (iv.solid.length ? Math.max(m, iv.z1) : m), -Infinity);
      for (const iv of ev.intervals) {
        if (!iv.solid.length) continue;
        ctx.fillStyle = depthShade(colour, iv.z1, top);
        ctx.fill(path(iv.solid), "nonzero");
      }
      // The kernel lists a glass part in every band it spans, and the
      // renderer does not stack those layers: one path, one fill. Windings
      // of separate bands add up, so a band's hole stays open only where no
      // other band has glass.
      const glass = new Path2D();
      for (const iv of ev.intervals) glass.addPath(path(iv.glass));
      ctx.fillStyle = colour;
      ctx.globalAlpha = GLASS_ALPHA;
      ctx.fill(glass, "nonzero");
      ctx.globalAlpha = 1;
    }
    if (ev) {
      // A member of a Cut group removes material like a Cut part and draws
      // like one; a selected group lights up every member's outline.
      const cutGroups = new Set(s.design.groups.filter((g) => g.mode === "cut").flatMap((g) => g.parts));
      const selectedGroup = s.selectedGroup ? s.design.groups.find((g) => g.id === s.selectedGroup) : null;
      const groupSel = selectedGroup ? new Set(selectedGroup.parts) : null;
      for (const part of s.design.parts) {
        const geom = ev.parts.find((g) => g.id === part.id);
        if (!geom || !part.visible) continue;
        const p = path(geom.shapes);
        const sel = part.id === s.selected || (groupSel !== null && groupSel.has(part.id));
        if (part.mode === "cut" || cutGroups.has(part.id)) {
          if (!stale) {
            ctx.fillStyle = hatchPattern();
            ctx.fill(p, "nonzero");
          }
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
    const editing = editingPart();
    if (editing) {
      drawPoints(editing);
      return;
    }
    // A hidden part stays selected (the inspector is where it comes back),
    // but shows no handles: nothing on the canvas belongs to it.
    const part = store.part();
    if (part && part.visible) handles(part);
  }

  function drawPoints(part) {
    const loopsPx = pointLoopsPx(part);
    const loops = loopsOf(part.shape);
    const sel = store.state.ui.pointSel;
    // The control polygon, faint: where a click inserts a point.
    ctx.setLineDash([3, 3]);
    ctx.strokeStyle = "rgba(120,200,255,0.45)";
    ctx.lineWidth = 1;
    for (const loop of loopsPx) {
      ctx.beginPath();
      loop.forEach(([x, y], i) => (i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)));
      ctx.closePath();
      ctx.stroke();
    }
    ctx.setLineDash([]);
    loopsPx.forEach((loop, li) => {
      loop.forEach(([x, y], i) => {
        const q = loops[li][i];
        if (sel && sel.loop === li && sel.index === i) {
          ctx.beginPath();
          ctx.arc(x, y, POINT_PX, 0, Math.PI * 2);
          ctx.strokeStyle = "rgba(120,200,255,0.95)";
          ctx.lineWidth = 1.5;
          ctx.stroke();
        }
        // Round for smooth, square for a corner, as the old designer drew them.
        ctx.beginPath();
        if (q.smooth) ctx.arc(x, y, 4.5, 0, Math.PI * 2);
        else ctx.rect(x - 4, y - 4, 8, 8);
        ctx.fillStyle = part.locked ? "rgba(120,200,255,0.5)" : "#78c8ff";
        ctx.fill();
        ctx.strokeStyle = "rgba(0,0,0,0.7)";
        ctx.lineWidth = 1;
        ctx.stroke();
      });
    });
  }

  function handles(part) {
    const r = partRect(part.transform, 1);
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
    // White handles with a dark rim stay visible over a white fill.
    ctx.fillStyle = "#fff";
    ctx.strokeStyle = "rgba(0,0,0,0.6)";
    for (const c of [...g.corners, ...g.edges]) {
      const [x, y] = toPx(view, size, c);
      ctx.fillRect(x - 3.5, y - 3.5, 7, 7);
      ctx.strokeRect(x - 3.5, y - 3.5, 7, 7);
    }
    const [tx, ty] = toPx(view, size, g.top);
    const [cx, cy] = toPx(view, size, [r.cx, r.cy]);
    const len = Math.hypot(tx - cx, ty - cy) || 1;
    const rx = tx + ((tx - cx) / len) * ROTATE_PX;
    const ry = ty + ((ty - cy) / len) * ROTATE_PX;
    ctx.strokeStyle = "rgba(120,200,255,0.8)";
    ctx.beginPath();
    ctx.moveTo(tx, ty);
    ctx.lineTo(rx, ry);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(rx, ry, 5, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = "rgba(0,0,0,0.6)";
    ctx.stroke();
  }

  function rotateHandleHit(part, mx, my) {
    const r = partRect(part.transform, 1);
    const g = rectCorners(r);
    const [tx, ty] = toPx(view, size, g.top);
    const [cx, cy] = toPx(view, size, [r.cx, r.cy]);
    const len = Math.hypot(tx - cx, ty - cy) || 1;
    const rx = tx + ((tx - cx) / len) * ROTATE_PX;
    const ry = ty + ((ty - cy) / len) * ROTATE_PX;
    return Math.hypot(mx - rx, my - ry) <= HANDLE_PX + 2;
  }

  function pick(wp) {
    // Top of the list is picked first.
    const parts = store.state.design.parts;
    for (let i = parts.length - 1; i >= 0; i--) {
      const p = parts[i];
      if (!p.visible) continue;
      if (rectContains(partRect(p.transform, 1), wp)) return p;
    }
    return null;
  }

  // The WebView's own menu offers Reload, which would discard the design.
  // In point editing a right click on a point removes it.
  canvas.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    const editing = editingPart();
    if (!editing || editing.locked) return;
    const hit = nearestPoint(pointLoopsPx(editing), local(e), POINT_PX);
    if (hit) store.removePoint(editing.id, hit.loop, hit.index);
  });

  // Point editing: a point under the pointer is picked up, the outline
  // near the pointer gets a new point that is picked up at once, anything
  // else ends the editing (and does nothing more, so a part never moves by
  // the click that finished editing another).
  function pointDown(editing, mx, my, wp) {
    const loopsPx = pointLoopsPx(editing);
    const hit = nearestPoint(loopsPx, [mx, my], POINT_PX);
    if (hit) {
      store.selectPoint({ loop: hit.loop, index: hit.index });
      if (!editing.locked) gesture = { kind: "point", id: editing.id, loop: hit.loop, index: hit.index };
      return;
    }
    const seg = nearestSegment(loopsPx, [mx, my]);
    if (seg && seg.dist <= SEGMENT_PX && !editing.locked) {
      const r = partRect(editing.transform, 1);
      const [ux, uy] = worldToUnit(r, wp);
      const placed = store.insertPoint(editing.id, seg.loop, seg.index, round3(ux), round3(uy));
      if (placed) gesture = { kind: "point", id: editing.id, loop: placed.loop, index: placed.index };
      return;
    }
    store.setPointEdit(null);
  }

  canvas.addEventListener("pointerdown", (e) => {
    // A press that starts no gesture must not leave the last one running.
    gesture = null;
    const [mx, my] = local(e);
    canvas.setPointerCapture(e.pointerId);
    if (e.button === 1 || spaceDown) {
      gesture = { kind: "pan", mx, my, cx: view.cx, cy: view.cy };
      return;
    }
    if (e.button !== 0) return;
    const wp = toWorld(view, size, mx, my);
    const editing = editingPart();
    if (editing) {
      pointDown(editing, mx, my, wp);
      return;
    }
    const s = store.state;
    const sel = store.part();
    if (sel && !sel.locked && sel.visible) {
      const r = partRect(sel.transform, 1);
      if (rotateHandleHit(sel, mx, my)) {
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
      gesture = { kind: "move", id: hit.id, wp, x: hit.transform.x, y: hit.transform.y };
    }
  });

  canvas.addEventListener("pointermove", (e) => {
    if (!gesture || !canvas.hasPointerCapture(e.pointerId)) return;
    const [mx, my] = local(e);
    if (gesture.kind === "pan") {
      const u = unit(view, size);
      view.cx = gesture.cx - (mx - gesture.mx) / u;
      view.cy = gesture.cy + (my - gesture.my) / u;
      draw();
      return;
    }
    // The part can vanish mid-drag (undo, delete): the gesture ends with it.
    const part = store.part(gesture.id);
    if (!part) {
      gesture = null;
      return;
    }
    const wp = toWorld(view, size, mx, my);
    // Snapping pulls within a pixel radius, measured in world units here
    // and capped so a free band stays between the grid lines; Shift keeps
    // its own meanings (proportions, 15 degree steps, the grid in point
    // editing) whether snapping is on or off.
    const tol = snapTolerance(unit(view, size));
    const snapping = Boolean(store.state.ui.snap);
    // Design units throughout: a world delta is a document delta. Labels
    // carry the part id: undo coalescing merges by label, and a gesture on
    // one part must never merge into a gesture on another.
    if (gesture.kind === "point") {
      // Into the part's own frame, so the point lands under the pointer on
      // a turned or stretched part too; the snap radius moves into that
      // frame with it.
      const r = partRect(part.transform, 1);
      let q = worldToUnit(r, wp);
      if (e.shiftKey) q = snapUnit(q, POINT_SNAP);
      else if (snapping) q = [snapValue(q[0], tol / (r.hw || 1), [], POINT_SNAP), snapValue(q[1], tol / (r.hh || 1), [], POINT_SNAP)];
      store.movePoint(gesture.id, gesture.loop, gesture.index, round3(clamp(q[0], -4, 4)), round3(clamp(q[1], -4, 4)));
    } else if (gesture.kind === "move") {
      const dx = wp[0] - gesture.wp[0];
      const dy = wp[1] - gesture.wp[1];
      let nx = gesture.x + dx;
      let ny = gesture.y + dy;
      if (snapping) [nx, ny] = snapPosition(nx, ny, tol);
      store.updatePart(gesture.id, `move:${gesture.id}`, (p) => {
        p.transform.x = clamp(nx, -1.5, 1.5);
        p.transform.y = clamp(ny, -1.5, 1.5);
      });
    } else if (gesture.kind === "scale") {
      // Shift keeps the proportions on corners only; an edge handle moves one axis.
      const keep = e.shiftKey && gesture.handle.length === 2;
      let { hw, hh } = scaleFrom(gesture.r, gesture.handle, wp, keep);
      if (snapping && !keep) [hw, hh] = snapSize(hw, hh, tol);
      store.updatePart(gesture.id, `scale:${gesture.id}`, (p) => {
        p.transform.w = clamp(hw, 0.02, 1.5);
        p.transform.h = clamp(hh, 0.02, 1.5);
      });
    } else if (gesture.kind === "rotate") {
      const r = partRect(part.transform, 1);
      let a = angleFrom(r, wp);
      if (e.shiftKey) a = Math.round(a / 15) * 15;
      else if (snapping) a = snapAngle(a, 3);
      a = ((((a + 180) % 360) + 360) % 360) - 180;
      store.updatePart(gesture.id, `rotate:${gesture.id}`, (p) => (p.transform.rotation = Math.round(a * 10) / 10));
    }
    hooks.onEdit();
  });

  // Every button takes pointer capture on pointerdown (a right click too,
  // which starts no gesture), so every release gives it back. A gesture
  // that ends is one undo step; the next drag, however quick, is another.
  const end = (e) => {
    if (canvas.hasPointerCapture(e.pointerId)) canvas.releasePointerCapture(e.pointerId);
    if (gesture) store.endGesture();
    gesture = null;
  };
  canvas.addEventListener("pointerup", end);
  canvas.addEventListener("pointercancel", end);

  // Double-click: on a point it switches smooth and corner; on a part it
  // asks for point editing (the owner converts the shape when needed).
  canvas.addEventListener("dblclick", (e) => {
    if (e.button !== 0) return;
    const [mx, my] = local(e);
    const editing = editingPart();
    if (editing) {
      const hit = nearestPoint(pointLoopsPx(editing), [mx, my], POINT_PX);
      if (hit && !editing.locked) store.togglePoint(editing.id, hit.loop, hit.index);
      return;
    }
    const hit = pick(toWorld(view, size, mx, my));
    if (hit && !hit.locked && hooks.onDoubleClick) hooks.onDoubleClick(hit);
  });

  canvas.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      if (!e.deltaY || !boxHasArea()) return;
      // The delta in pixels: WebView2 reports pixels, WebKitGTK may report
      // lines (deltaMode 1) or pages (2), which would otherwise zoom far too
      // slowly.
      let dy = e.deltaY;
      if (e.deltaMode === 1) dy *= 16;
      else if (e.deltaMode === 2) dy *= size.h;
      // Proportional to the wheel delta, so a trackpad's many small events
      // zoom as far as one mouse notch does; one event never moves more
      // than a factor of two.
      const factor = clamp(Math.exp(-dy * 0.0015), 0.5, 2);
      const [mx, my] = local(e);
      zoomAt(view, size, mx, my, factor);
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
  // A Space release that lands in another window never reaches keyup here.
  window.addEventListener("blur", () => {
    spaceDown = false;
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
const round3 = (v) => Math.round(v * 1000) / 1000;
