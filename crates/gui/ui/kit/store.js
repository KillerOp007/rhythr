// crates/gui/ui/kit/store.js
// One store: the design, the editor state and undo. Every change goes
// through commit(label, mutate); the render layer redraws from state, so
// panels and state can never disagree.

export const DEPTH_MIN = -2.0;
export const DEPTH_MAX = 0.5;
export const NEW_DEPTH = { front: 0.1, back: -0.1 };
export const UNDO_LIMIT = 100;
// A continuous gesture (a slider drag, a key held down) is one undo step:
// commits with the same label inside this window merge into the first.
export const COALESCE_MS = 350;
// Structural edits and button clicks are discrete actions: two quick removes
// (or toggles) are two undo steps, never one. Only slider, field and picker
// gestures coalesce.
const DISCRETE = new Set([
  "add", "remove", "duplicate", "reorder",
  "visible", "lock", "mode", "material", "shape", "name",
  "color-add", "color-remove", "color-reorder",
  "point-add", "point-remove", "point-smooth",
  "group", "group-add", "group-remove", "ungroup", "group-mode", "group-name",
  "effect-waves", "effect-round", "effect-rim", "mirror", "repeat", "waves.style",
]);
// A label may carry the part id after a colon (`waves.style:p2`); the
// discrete rule reads the part before it.
const isDiscrete = (label) => DISCRETE.has(label) || DISCRETE.has(String(label).split(":")[0]);

// A Points shape holds its first loop in `control` and the rest in `loops`;
// the editor treats them as one list of loops.
export function loopsOf(shape) {
  if (!shape || shape.kind !== "points" || !Array.isArray(shape.control)) return null;
  return [shape.control, ...(Array.isArray(shape.loops) ? shape.loops : [])];
}

function setLoops(shape, loops) {
  shape.control = loops[0] || [];
  shape.loops = loops.slice(1);
}

// A part leaves whatever group holds it; a group left empty goes with it.
function leaveGroup(d, id) {
  for (const g of d.groups) g.parts = g.parts.filter((x) => x !== id);
  d.groups = d.groups.filter((g) => g.parts.length > 0);
}

// Members stay contiguous in the parts array: a joining part moves to sit
// directly above the group's topmost member, so the list shows the group
// in one block whatever order the parts had.
function placeNextToGroup(d, id, g) {
  const idx = d.parts.findIndex((p) => p.id === id);
  const others = g.parts.filter((x) => x !== id).map((x) => d.parts.findIndex((p) => p.id === x)).filter((i) => i >= 0);
  if (idx < 0 || !others.length) return;
  const top = Math.max(...others);
  const [p] = d.parts.splice(idx, 1);
  // After the removal the old top member sits one lower when it was above
  // the moved part; either way the part lands directly above it.
  d.parts.splice(idx < top ? top : top + 1, 0, p);
}

// The parts list in DOM order (the list is column-reverse, so the last row
// draws at the top): parts bottom to top, a group's header after its
// topmost member, members hidden while the group is folded.
export function listRows(design, folded = []) {
  const groupOf = (id) => design.groups.find((g) => g.parts.includes(id)) || null;
  const top = new Map();
  design.parts.forEach((p, i) => {
    const g = groupOf(p.id);
    if (g) top.set(g.id, i);
  });
  const rows = [];
  design.parts.forEach((p, i) => {
    const g = groupOf(p.id);
    const isFolded = Boolean(g) && folded.includes(g.id);
    if (!isFolded) rows.push({ kind: "part", id: p.id, group: g ? g.id : null });
    if (g && top.get(g.id) === i) rows.push({ kind: "group", id: g.id, name: g.name, mode: g.mode, folded: isFolded, count: g.parts.length });
  });
  return rows;
}

export const SHAPES = {
  circle: () => ({ kind: "circle" }),
  square: () => ({ kind: "square", rounding: 0 }),
  polygon: () => ({ kind: "polygon", sides: 6, rounding: 0 }),
  star: () => ({ kind: "star", points: 5, inner: 0.5 }),
  ring: () => ({ kind: "ring", thickness: 0.28 }),
  pill: () => ({ kind: "pill", aspect: 0.5 }),
};
const NAMES = { circle: "Disc", square: "Square", polygon: "Polygon", star: "Star", ring: "Ring", pill: "Pill" };

export function newPart(id, kind) {
  return {
    id,
    name: NAMES[kind] || "Part",
    visible: true,
    locked: false,
    shape: SHAPES[kind](),
    transform: { x: 0, y: 0, w: 1, h: 1, rotation: 0 },
    mode: "add",
    depth: { ...NEW_DEPTH },
    material: "solid",
    effects: { waves: null, round: null, rim: null, mirror: "none", repeat: null },
  };
}

function topId(design) {
  return design.parts.length ? design.parts[design.parts.length - 1].id : null;
}

export function createStore(design, opts = {}) {
  const now = opts.now || (() => Date.now());
  const listeners = new Set();
  const s = {
    design,
    selected: topId(design),
    // A selected group (its header row) excludes a selected part.
    selectedGroup: null,
    path: null,
    dirty: false,
    undo: [],
    redo: [],
    last: { label: null, at: -Infinity },
    // pointEdit: the id of the Points part whose points are being edited, or
    // null; pointSel: the selected point as { loop, index }, or null; folded:
    // the group ids whose members the list hides (never in the document);
    // snap: whether moves, sizes, turns and points pull to the grid.
    ui: { previews: false, turntable: false, pointEdit: null, pointSel: null, folded: [], snap: true },
    // The last good evaluation stays on screen through an error, so the
    // handles never vanish under the cursor.
    evaluated: null,
    evalError: null,
    status: { text: "", error: false },
  };
  const emit = () => {
    for (const l of listeners) l(s);
  };
  const snap = () => JSON.stringify(s.design);
  const noLast = () => ({ label: null, at: -Infinity });
  function fixSelection() {
    if (s.selected && !s.design.parts.some((p) => p.id === s.selected)) s.selected = topId(s.design);
    if (s.selectedGroup && !s.design.groups.some((g) => g.id === s.selectedGroup)) s.selectedGroup = null;
    s.ui.folded = s.ui.folded.filter((id) => s.design.groups.some((g) => g.id === id));
    // Point editing follows its part: gone (undo, remove) or no longer a
    // Points shape, and the mode ends; a selected point past the loop's end
    // (undo of an insert) is dropped.
    if (s.ui.pointEdit) {
      const p = s.design.parts.find((x) => x.id === s.ui.pointEdit);
      const loops = p ? loopsOf(p.shape) : null;
      if (!loops) {
        s.ui.pointEdit = null;
        s.ui.pointSel = null;
      } else if (s.ui.pointSel) {
        const { loop, index } = s.ui.pointSel;
        if (!loops[loop] || index >= loops[loop].length) s.ui.pointSel = null;
      }
    }
  }
  const api = {
    state: s,
    subscribe(fn) {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    commit(label, mutate) {
      const before = snap();
      mutate(s.design);
      const t = now();
      const merge = !isDiscrete(label) && label === s.last.label && t - s.last.at < COALESCE_MS;
      if (!merge) {
        s.undo.push(before);
        if (s.undo.length > UNDO_LIMIT) s.undo.shift();
        s.redo.length = 0;
      }
      s.last = { label, at: t };
      s.dirty = true;
      fixSelection();
      emit();
    },
    undo() {
      const b = s.undo.pop();
      if (b === undefined) return false;
      s.redo.push(snap());
      s.design = JSON.parse(b);
      s.last = noLast();
      s.dirty = true;
      fixSelection();
      emit();
      return true;
    },
    redo() {
      const n = s.redo.pop();
      if (n === undefined) return false;
      s.undo.push(snap());
      s.design = JSON.parse(n);
      s.last = noLast();
      s.dirty = true;
      fixSelection();
      emit();
      return true;
    },
    // A released pointer ends the gesture: the next commit starts a new undo
    // step even inside the coalescing window, so two quick drags on one part
    // undo separately. Nothing on screen changes, so nothing is emitted.
    endGesture() {
      s.last = noLast();
    },
    // Point editing belongs to the part being edited: choosing another part
    // or a group ends it, so the canvas and the inspector never disagree.
    select(id) {
      s.selected = id;
      s.selectedGroup = null;
      if (s.ui.pointEdit && s.ui.pointEdit !== id) {
        s.ui.pointEdit = null;
        s.ui.pointSel = null;
      }
      emit();
    },
    selectGroup(id) {
      s.selectedGroup = id;
      if (id) {
        s.selected = null;
        s.ui.pointEdit = null;
        s.ui.pointSel = null;
      }
      emit();
    },
    nextGroupId() {
      const ids = new Set(s.design.groups.map((g) => g.id));
      let n = 1;
      while (ids.has(`g${n}`)) n++;
      return `g${n}`;
    },
    groupOf(id) {
      return s.design.groups.find((g) => g.parts.includes(id)) || null;
    },
    // A new group holds one part and is selected, so it can be named and
    // given its mode at once; more parts join from their inspector or by
    // a drop on the group's row.
    createGroup(partId, name) {
      let id = null;
      api.commit("group", (d) => {
        if (!d.parts.some((p) => p.id === partId)) return;
        // Leave first: a sole member's old group dissolves here, and the
        // new group takes the smallest id and name nobody uses after that.
        leaveGroup(d, partId);
        const ids = new Set(d.groups.map((g) => g.id));
        const names = new Set(d.groups.map((g) => g.name));
        let n = 1;
        while (ids.has(`g${n}`) || names.has(`Group ${n}`)) n++;
        id = `g${n}`;
        d.groups.push({ id, name: String(name || "").trim() || `Group ${n}`, parts: [partId], mode: "add" });
        s.selectedGroup = id;
        s.selected = null;
        s.ui.pointEdit = null;
        s.ui.pointSel = null;
      });
      return id;
    },
    // A part already in the group, an unknown part or an unknown group is
    // nothing to do: checked before the commit, so no undo step and no
    // dirty flag come from a drop on the part's own header.
    addToGroup(partId, groupId) {
      const g = s.design.groups.find((x) => x.id === groupId);
      if (!g || !s.design.parts.some((p) => p.id === partId) || g.parts.includes(partId)) return;
      api.commit("group-add", (d) => {
        leaveGroup(d, partId);
        const target = d.groups.find((x) => x.id === groupId);
        if (!target) return;
        target.parts.push(partId);
        placeNextToGroup(d, partId, target);
      });
    },
    removeFromGroup(partId) {
      api.commit("group-remove", (d) => leaveGroup(d, partId));
    },
    ungroup(groupId) {
      api.commit("ungroup", (d) => {
        d.groups = d.groups.filter((g) => g.id !== groupId);
      });
    },
    updateGroup(groupId, label, patch) {
      api.commit(label, (d) => {
        const g = d.groups.find((x) => x.id === groupId);
        if (g) patch(g);
      });
    },
    // Folding is editor state: no commit, no undo step, no evaluate.
    toggleFold(groupId) {
      s.ui.folded = s.ui.folded.includes(groupId) ? s.ui.folded.filter((x) => x !== groupId) : [...s.ui.folded, groupId];
      emit();
    },
    part(id = s.selected) {
      return s.design.parts.find((p) => p.id === id) || null;
    },
    nextId() {
      const ids = new Set(s.design.parts.map((p) => p.id));
      let n = 1;
      while (ids.has(`p${n}`)) n++;
      return `p${n}`;
    },
    // The selection is set inside the commit, so its one emit already shows
    // the new part selected.
    addPart(kind) {
      const id = api.nextId();
      api.commit("add", (d) => {
        d.parts.push(newPart(id, kind));
        s.selected = id;
      });
      return id;
    },
    // An imported outline (an SVG's filled paths or a glyph) as a new part,
    // named after its source and selected like any Add.
    addContours(contours, label) {
      const id = api.nextId();
      const name = String(label || "").trim() || "Outline";
      api.commit("add", (d) => {
        const p = newPart(id, "circle");
        p.name = name;
        p.shape = { kind: "contours", contours, label: name };
        d.parts.push(p);
        s.selected = id;
      });
      return id;
    },
    // An SVG's colours, one part each in paint order: the top colour of the
    // file ends on top of the list and is selected. Each layer brings its
    // mode (a colour drawn on another is a Cut).
    addLayers(layers, label) {
      const base = String(label || "").trim() || "Outline";
      const ids = [];
      if (!layers.length) return ids;
      api.commit("add", (d) => {
        layers.forEach((l, i) => {
          const id = api.nextId();
          const p = newPart(id, "circle");
          const one = layers.length === 1;
          p.name = one ? base : `${base} ${i + 1}`;
          p.shape = { kind: "contours", contours: l.contours, label: one || !l.paint ? base : `${base}, colour ${l.paint}` };
          p.mode = l.mode === "cut" ? "cut" : "add";
          d.parts.push(p);
          ids.push(id);
        });
        s.selected = ids[ids.length - 1];
      });
      return ids;
    },
    // Point editing. Coordinates are in the part's own unit frame (the
    // transform is applied afterwards, so a point dragged on a turned part
    // lands where the pointer is).
    setPointEdit(id) {
      s.ui.pointEdit = id;
      s.ui.pointSel = null;
      emit();
    },
    selectPoint(sel) {
      s.ui.pointSel = sel;
      emit();
    },
    pointLoops(id = s.ui.pointEdit) {
      const p = api.part(id);
      return p ? loopsOf(p.shape) : null;
    },
    movePoint(id, loop, index, x, y) {
      api.updatePart(id, `point:${id}`, (p) => {
        const l = loopsOf(p.shape);
        const q = l && l[loop] && l[loop][index];
        if (q) {
          q.x = x;
          q.y = y;
        }
      });
    },
    // Inserted before `index` in the loop; smooth unless both neighbours are
    // corners, so a point on a straight edge keeps the edge straight.
    insertPoint(id, loop, index, x, y) {
      let placed = null;
      api.updatePart(id, "point-add", (p) => {
        const l = loopsOf(p.shape);
        if (!l || !l[loop]) return;
        const pts = l[loop];
        const at = Math.max(0, Math.min(pts.length, index));
        const prev = pts[(at + pts.length - 1) % pts.length];
        const next = pts[at % pts.length];
        const smooth = !(prev && next && !prev.smooth && !next.smooth);
        pts.splice(at, 0, { x, y, smooth });
        placed = { loop, index: at };
        s.ui.pointSel = placed;
      });
      return placed;
    },
    // A loop keeps three points; at three, removing a point drops the loop
    // unless it is the part's last one, which stays.
    removePoint(id, loop, index) {
      let done = false;
      api.updatePart(id, "point-remove", (p) => {
        const l = loopsOf(p.shape);
        if (!l || !l[loop] || index >= l[loop].length) return;
        if (l[loop].length > 3) {
          l[loop].splice(index, 1);
        } else if (l.length > 1) {
          l.splice(loop, 1);
          setLoops(p.shape, l);
        } else {
          return;
        }
        done = true;
        s.ui.pointSel = null;
      });
      return done;
    },
    togglePoint(id, loop, index) {
      api.updatePart(id, "point-smooth", (p) => {
        const l = loopsOf(p.shape);
        const q = l && l[loop] && l[loop][index];
        if (q) q.smooth = !q.smooth;
      });
    },
    removePart(id) {
      api.commit("remove", (d) => {
        d.parts = d.parts.filter((p) => p.id !== id);
        leaveGroup(d, id);
      });
    },
    duplicatePart(id) {
      const src = api.part(id);
      if (!src) return null;
      const nid = api.nextId();
      api.commit("duplicate", (d) => {
        const copy = JSON.parse(JSON.stringify(src));
        copy.id = nid;
        copy.name = `${src.name} copy`;
        const i = d.parts.findIndex((p) => p.id === id);
        d.parts.splice(i + 1, 0, copy);
        // The copy sits right above its source, so it joins the same group
        // and the block stays contiguous.
        const g = d.groups.find((x) => x.parts.includes(id));
        if (g) g.parts.push(nid);
        s.selected = nid;
      });
      return nid;
    },
    updatePart(id, label, patch) {
      api.commit(label, (d) => {
        const p = d.parts.find((x) => x.id === id);
        if (p) patch(p);
      });
    },
    // A row dropped on another row lands next to it and takes that row's
    // group (none, when the target is in none), so a group's block stays
    // contiguous whatever is dragged where.
    movePart(id, to) {
      api.commit("reorder", (d) => {
        const from = d.parts.findIndex((p) => p.id === id);
        if (from < 0) return;
        const target = d.parts[Math.max(0, Math.min(d.parts.length - 1, to))];
        const [p] = d.parts.splice(from, 1);
        d.parts.splice(Math.max(0, Math.min(d.parts.length, to)), 0, p);
        const was = d.groups.find((g) => g.parts.includes(id)) || null;
        const next = target && target.id !== id ? d.groups.find((g) => g.parts.includes(target.id)) || null : was;
        if (next === was) return;
        leaveGroup(d, id);
        const again = next && d.groups.find((g) => g.id === next.id);
        if (again) again.parts.push(id);
      });
    },
    load(next, path = null) {
      s.design = next;
      s.path = path;
      s.dirty = false;
      s.undo.length = 0;
      s.redo.length = 0;
      s.last = noLast();
      s.selected = topId(next);
      // A loaded design (a file, a preset) may reuse the ids of the old one,
      // so no editing state carries over: group selection, folds and point
      // editing all belong to the design that is gone.
      s.selectedGroup = null;
      s.ui.pointEdit = null;
      s.ui.pointSel = null;
      s.ui.folded = [];
      emit();
    },
    setEvaluated(ev) {
      s.evaluated = ev;
      s.evalError = null;
      emit();
    },
    setEvalError(msg) {
      s.evalError = msg;
      emit();
    },
    setStatus(text, error = false) {
      s.status = { text, error };
      emit();
    },
    setUi(patch) {
      Object.assign(s.ui, patch);
      emit();
    },
    serialize() {
      return JSON.stringify(s.design);
    },
  };
  return api;
}
