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
]);

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
    path: null,
    dirty: false,
    undo: [],
    redo: [],
    last: { label: null, at: -Infinity },
    ui: { previews: false },
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
      const merge = !DISCRETE.has(label) && label === s.last.label && t - s.last.at < COALESCE_MS;
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
    select(id) {
      s.selected = id;
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
    removePart(id) {
      api.commit("remove", (d) => {
        d.parts = d.parts.filter((p) => p.id !== id);
        for (const g of d.groups) g.parts = g.parts.filter((x) => x !== id);
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
    movePart(id, to) {
      api.commit("reorder", (d) => {
        const from = d.parts.findIndex((p) => p.id === id);
        if (from < 0) return;
        const [p] = d.parts.splice(from, 1);
        d.parts.splice(Math.max(0, Math.min(d.parts.length, to)), 0, p);
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
