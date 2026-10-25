// crates/gui/ui/kit/render.js
// A panel is rebuilt only when its structure changes (parts added, removed or
// reordered, another part or shape kind selected, a colour added or removed)
// and is patched in place otherwise. A rebuild between mousedown and mouseup
// detaches the button under the pointer and the click never lands. Nothing in
// the DOM is a source of truth: a field the user is typing in keeps its text
// until it commits, every other widget is rewritten from state.
import { DEPTH_MIN, DEPTH_MAX, SHAPES, loopsOf, listRows } from "./store.js";

export const MIN_BAND = 0.05;

export function fmt(v, step) {
  const d = step >= 1 ? 0 : Math.max(0, Math.ceil(-Math.log10(step)));
  return Number(v).toFixed(d);
}

export function roundToStep(v, step) {
  return Number(fmt(v, step));
}

export function clampDepth(front, back, which, value) {
  const lo = DEPTH_MIN;
  const hi = DEPTH_MAX;
  let f = front;
  let b = back;
  if (which === "front") {
    f = Math.max(lo + MIN_BAND, Math.min(hi, value));
    if (b > f - MIN_BAND) b = f - MIN_BAND;
  } else {
    b = Math.max(lo, Math.min(hi - MIN_BAND, value));
    if (f < b + MIN_BAND) f = b + MIN_BAND;
  }
  const r = (x) => Math.round(x * 100) / 100;
  return { front: r(f), back: r(b) };
}

// [key, label, min, max, step]
const FIELDS = {
  circle: [],
  square: [["rounding", "Round corners", 0, 1, 0.01]],
  polygon: [["sides", "Sides", 3, 12, 1], ["rounding", "Round corners", 0, 1, 0.01]],
  star: [["points", "Points", 3, 12, 1], ["inner", "Inner size", 0.1, 0.9, 0.01]],
  ring: [["thickness", "Thickness", 0.05, 0.95, 0.01]],
  pill: [["aspect", "Roundness", 0.1, 1, 0.01]],
  contours: [],
  points: [],
};
export function shapeFields(kind) {
  return FIELDS[kind] || [];
}

const KIND_LABELS = { circle: "Disc", square: "Square", polygon: "Polygon", star: "Star", ring: "Ring", pill: "Pill", contours: "Outline (imported)", points: "Outline (points)" };

export const POINT_HELP = "Drag a point, click the outline to add one, double-click a point to switch smooth and corner, right-click or Delete removes it, Esc or a click elsewhere finishes.";

export const EFFECT_DEFAULTS = { waves: { count: 6, depth: 0.15, style: "sine" }, round: 0.1, rim: 0.15 };
export const REPEAT_MAX = 12;

// The Repeat field: below two copies there is nothing to repeat, so the
// effect is off (null); the kernel caps far higher, the editor at twelve.
export function repeatFromField(v) {
  const n = Math.round(Number(v));
  if (!Number.isFinite(n) || n < 2) return null;
  return Math.min(REPEAT_MAX, n);
}

// What the parts list was built for: the rows, their order, the folds and
// each part's group (a part leaving its group loses its indent).
export function listKey(design, folded) {
  return listRows(design, folded).map((r) => `${r.kind}:${r.id}:${r.folded ? 1 : 0}:${r.group || ""}`).join(",");
}

// What the Group dropdown shows: the groups by id and name, and the current
// one. Rebuilt only when this changes, never under an open dropdown.
export function groupOptionsKey(design, currentId) {
  return `${design.groups.map((g) => `${g.id}:${g.name}`).join("|")}|${currentId || ""}`;
}

// What the inspector was built for: the part, its shape kind and which
// chips are on (a chip switching on adds its fields, so it rebuilds).
export function inspectorKeyFor(p) {
  const e = p.effects || {};
  return `${p.id}|${p.shape.kind}|${e.waves ? 1 : 0}|${e.round != null ? 1 : 0}|${e.rim != null ? 1 : 0}`;
}

// "12 points in 2 loops": what the inspector says about a Points shape.
export function pointSummary(shape) {
  const loops = loopsOf(shape) || [];
  const n = loops.reduce((sum, l) => sum + l.length, 0);
  return `${n} point${n === 1 ? "" : "s"} in ${loops.length} loop${loops.length === 1 ? "" : "s"}`;
}

const $ = (id) => document.getElementById(id);

// What each panel was built for, and the handles to patch it with.
let partsKey = null;
let partRows = [];
let inspectorKey = null;
let inspectorId = null;
let inspectorWidgets = []; // each: (part) => void, rewrites its widget from state
let kindSelect = null;
let colorsKey = null;
let colorRows = [];
// Drag sources live outside the panels: a part drag survives the reorder
// rebuild that its own drop causes.
let dragPartId = null;
let dragColorIdx = -1;

function el(tag, attrs = {}, children = []) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v;
    else if (k === "text") e.textContent = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (v !== null && v !== undefined) e.setAttribute(k, v);
  }
  for (const c of children) e.append(c);
  return e;
}

function setText(node, text) {
  if (node.textContent !== text) node.textContent = text;
}

// Every text-like field remembers what it last showed. A field whose text
// differs from that is mid-edit and keeps its text; otherwise it follows state.
function setShown(input, text) {
  if (input.value !== text) input.value = text;
  input.dataset.shown = text;
}
function showText(input, text) {
  if (document.activeElement === input && input.value !== input.dataset.shown) return;
  setShown(input, text);
}

function slider(p, label, read, min, max, step, onInput) {
  const val = el("span", { class: "val", text: fmt(read(p), step) });
  const range = el("input", { type: "range", min, max, step, value: read(p) });
  range.addEventListener("input", () => {
    setText(val, fmt(range.value, step));
    onInput(Number(range.value));
  });
  inspectorWidgets.push((q) => {
    if (Number(range.value) !== read(q)) range.value = read(q);
    setText(val, fmt(read(q), step));
  });
  return el("div", { class: "kt-field" }, [el("label", { text: label }), range, val]);
}

// read(part) is the value the field shows. A commit rounds to the field's
// step, and changes nothing when the text is untouched, unusable or equal to
// the state; afterwards the field shows what the state kept.
function number(store, id, label, read, min, max, step, onCommit) {
  const input = el("input", { type: "number", min, max, step });
  setShown(input, fmt(read(store.part(id)), step));
  const commit = () => {
    const q = store.part(id);
    if (q && input.value !== input.dataset.shown && input.value.trim() !== "") {
      const v = Number(input.value);
      if (Number.isFinite(v)) {
        const next = roundToStep(Math.max(min, Math.min(max, v)), step);
        if (Math.abs(next - read(q)) > 1e-9) onCommit(next);
      }
    }
    const now = store.part(id);
    if (now) setShown(input, fmt(read(now), step));
  };
  input.addEventListener("change", commit);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") commit();
  });
  inspectorWidgets.push((q) => showText(input, fmt(read(q), step)));
  return el("div", { class: "kt-field" }, [el("label", { text: label }), input]);
}

function segmented(p, label, options, read, onPick) {
  const buttons = options.map(([v, text]) => [v, el("button", { class: "btn small", text, onclick: () => onPick(v) })]);
  const mark = (q) => {
    for (const [v, b] of buttons) b.classList.toggle("on", read(q) === v);
  };
  mark(p);
  inspectorWidgets.push(mark);
  return el("div", { class: "kt-field" }, [el("label", { text: label }), el("div", { class: "kt-seg" }, buttons.map(([, b]) => b))]);
}

// A group's header row: selects the group, folds its members, and takes a
// dropped part row into the group.
function groupRow(store, id) {
  const row = el("div", { class: "kt-part kt-group" });
  const fold = el("button", { class: "btn small ghost fold", onclick: (e) => { e.stopPropagation(); store.toggleFold(id); } });
  const name = el("span", { class: "name" });
  const mode = el("span", { class: "badge" });
  const count = el("span", { class: "count" });
  row.append(fold, name, mode, count);
  row.addEventListener("click", () => store.selectGroup(id));
  row.addEventListener("dragover", (e) => e.preventDefault());
  row.addEventListener("drop", (e) => {
    e.preventDefault();
    if (!dragPartId) return;
    const from = dragPartId;
    dragPartId = null;
    // A member dropped on its own header is already where it belongs.
    if (store.groupOf(from)?.id === id) return;
    store.addToGroup(from, id);
  });
  partRows.push({ kind: "group", id, row, fold, name, mode, count });
  return row;
}

function buildParts(store, root) {
  root.replaceChildren();
  partRows = [];
  for (const item of listRows(store.state.design, store.state.ui.folded)) {
    if (item.kind === "group") {
      root.append(groupRow(store, item.id));
      continue;
    }
    const id = item.id;
    const row = el("div", { class: `kt-part${item.group ? " member" : ""}`, draggable: "true" });
    const eye = el("button", { class: "btn small ghost",
      onclick: (e) => { e.stopPropagation(); store.updatePart(id, "visible", (x) => (x.visible = !x.visible)); } });
    const lock = el("button", { class: "btn small ghost",
      onclick: (e) => { e.stopPropagation(); store.updatePart(id, "lock", (x) => (x.locked = !x.locked)); } });
    const name = el("span", { class: "name" });
    const depth = el("span", { class: "depth", title: "Front depth. Parts at one depth merge into one surface; a higher Front stands out, a lower one lies deeper." });
    const mode = el("span", { class: "badge" });
    const material = el("span", { class: "badge" });
    row.append(eye, lock, name, depth, mode, material);
    row.addEventListener("click", () => store.select(id));
    row.addEventListener("dragstart", (e) => { dragPartId = id; row.classList.add("dragging"); e.dataTransfer.effectAllowed = "move"; });
    row.addEventListener("dragend", () => { dragPartId = null; row.classList.remove("dragging"); });
    row.addEventListener("dragover", (e) => e.preventDefault());
    row.addEventListener("drop", (e) => {
      e.preventDefault();
      if (!dragPartId || dragPartId === id) return;
      const to = store.state.design.parts.findIndex((x) => x.id === id);
      const from = dragPartId;
      dragPartId = null;
      store.movePart(from, to);
    });
    root.append(row);
    partRows.push({ kind: "part", id, row, eye, lock, name, depth, mode, material });
  }
}

function renderParts(store) {
  const s = store.state;
  const root = $("kt-parts");
  // Structure: which rows exist and in what order, folds and membership included.
  const key = listKey(s.design, s.ui.folded);
  if (key !== partsKey) {
    partsKey = key;
    buildParts(store, root);
  }
  for (const r of partRows) {
    if (r.kind === "group") {
      const g = s.design.groups.find((x) => x.id === r.id);
      if (!g) continue;
      const folded = s.ui.folded.includes(g.id);
      r.row.classList.toggle("selected", g.id === s.selectedGroup);
      setText(r.fold, folded ? "▸" : "▾");
      r.fold.setAttribute("title", folded ? "Show the group's parts" : "Hide the group's parts");
      setText(r.name, g.name);
      r.mode.classList.toggle("cut", g.mode === "cut");
      setText(r.mode, g.mode === "cut" ? "Cut" : "Add");
      setText(r.count, `${g.parts.length} part${g.parts.length === 1 ? "" : "s"}`);
      continue;
    }
    const p = store.part(r.id);
    if (!p) continue;
    r.row.classList.toggle("selected", p.id === s.selected);
    r.row.classList.toggle("hidden-part", !p.visible);
    setText(r.eye, p.visible ? "👁" : "◌");
    r.eye.setAttribute("title", p.visible ? "Hide" : "Show");
    setText(r.lock, p.locked ? "🔒" : "🔓");
    r.lock.setAttribute("title", p.locked ? "Unlock" : "Lock");
    setText(r.name, p.name);
    setText(r.depth, fmt(p.depth.front, 0.01));
    r.mode.classList.toggle("cut", p.mode === "cut");
    setText(r.mode, p.mode === "cut" ? "Cut" : "Add");
    r.material.classList.toggle("glass", p.material === "glass");
    setText(r.material, p.material === "glass" ? "Glass" : "Solid");
  }
}

function placeDepthBand(band, depth) {
  const span = DEPTH_MAX - DEPTH_MIN;
  const at = (z) => ((z - DEPTH_MIN) / span) * 100;
  band.style.left = `${at(depth.back)}%`;
  band.style.right = `${100 - at(depth.front)}%`;
}

function buildInspector(store, body, p, actions) {
  const id = p.id;
  const up = (label, patch) => store.updatePart(id, label, patch);
  const get = () => store.part(id);
  body.append(el("h2", { text: "Part" }));

  const name = el("input", { type: "text" });
  setShown(name, p.name);
  name.addEventListener("change", () => {
    const next = name.value.trim();
    if (next && next !== get()?.name) up("name", (x) => (x.name = next));
    const now = get();
    if (now) setShown(name, now.name);
  });
  inspectorWidgets.push((q) => showText(name, q.name));
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Name" }), name]));

  const kind = p.shape.kind;
  kindSelect = el("select", { "data-role": "kind" });
  for (const k of Object.keys(SHAPES)) kindSelect.append(el("option", { value: k, text: KIND_LABELS[k], selected: k === kind ? "" : null }));
  if (!(kind in SHAPES)) kindSelect.append(el("option", { value: kind, text: KIND_LABELS[kind] || kind, selected: "" }));
  const picker = kindSelect;
  picker.addEventListener("change", () => {
    if (picker.value !== kind) up("shape", (x) => (x.shape = SHAPES[picker.value]()));
  });
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Shape" }), picker]));
  if (kind === "contours") {
    const from = p.shape.label ? ` from ${p.shape.label}` : "";
    body.append(el("div", { class: "hint", text: `Imported outline${from}. Scale, turn, cut and rim it like any part; pick a shape above to replace it.` }));
  }
  for (const [key, label, min, max, step] of shapeFields(kind)) {
    body.append(slider(p, label, (q) => q.shape[key], min, max, step, (v) => up(`shape.${key}:${id}`, (x) => (x.shape[key] = v))));
  }
  // Freeform: a Points part offers its editing, every other shape the
  // conversion into one (the outline becomes points, the effects stay).
  if (kind === "points") {
    const info = el("div", { class: "hint" });
    const toggle = el("button", { class: "btn small", "data-role": "edit-points", onclick: () => actions.editPoints?.(id, store.state.ui.pointEdit !== id) });
    const sync = (q) => {
      const on = store.state.ui.pointEdit === id;
      setText(info, `${pointSummary(q.shape)}. ${on ? POINT_HELP : "Edit points to move, add and remove them; double-click the part does the same."}`);
      setText(toggle, on ? "Done" : "Edit points");
      toggle.classList.toggle("on", on);
    };
    sync(p);
    inspectorWidgets.push(sync);
    body.append(el("div", { class: "kt-field" }, [el("label", { text: "Points" }), toggle]), info);
  } else {
    body.append(el("div", { class: "kt-field" }, [
      el("label", { text: "Points" }),
      el("button", { class: "btn small ghost", "data-role": "to-points", text: "To points",
        title: "Turn the outline into points you can drag, add and remove (double-click the part does the same)",
        onclick: () => actions.toPoints?.(id) }),
    ]));
  }

  const pick = (key, label) => (v) => {
    if (get()?.[key] !== v) up(label, (x) => (x[key] = v));
  };
  body.append(segmented(p, "Mode", [["add", "Add"], ["cut", "Cut"]], (q) => q.mode, pick("mode", "mode")));
  body.append(segmented(p, "Material", [["solid", "Solid"], ["glass", "Glass"]], (q) => q.material, pick("material", "material")));
  // Group membership: none, an existing group, or a new one from this part.
  const groupSel = el("select", { "data-role": "group" });
  let groupsShown = null;
  const fillGroups = (q) => {
    const current = store.groupOf(q.id);
    const key = groupOptionsKey(store.state.design, current ? current.id : null);
    // An open dropdown is left alone: a rebuild under the pointer closes it.
    if (key === groupsShown || document.activeElement === groupSel) return;
    groupsShown = key;
    groupSel.replaceChildren(el("option", { value: "none", text: "None" }));
    for (const g of store.state.design.groups) groupSel.append(el("option", { value: g.id, text: g.name }));
    groupSel.append(el("option", { value: "new", text: "New group" }));
    groupSel.value = current ? current.id : "none";
  };
  fillGroups(p);
  groupSel.addEventListener("change", () => {
    const v = groupSel.value;
    // The choice is made: the dropdown may be rebuilt on the next render.
    groupSel.blur();
    if (v === "new") store.createGroup(id);
    else if (v === "none") store.removeFromGroup(id);
    else store.addToGroup(id, v);
  });
  inspectorWidgets.push(fillGroups);
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Group" }), groupSel]));

  body.append(el("h2", { text: "Position and size" }));
  const field = (label, read, min, max, write) => number(store, id, label, read, min, max, 0.01, (v) => up(`${write}:${id}`, (x) => (x.transform[write] = v)));
  body.append(field("X", (q) => q.transform.x, -1.5, 1.5, "x"));
  body.append(field("Y", (q) => q.transform.y, -1.5, 1.5, "y"));
  body.append(field("Width", (q) => q.transform.w, 0.02, 1.5, "w"));
  body.append(field("Height", (q) => q.transform.h, 0.02, 1.5, "h"));
  body.append(slider(p, "Rotation", (q) => q.transform.rotation, -180, 180, 1, (v) => up(`rotation:${id}`, (x) => (x.transform.rotation = v))));

  body.append(el("h2", { text: "Depth" }));
  const bar = el("div", { id: "kt-depth-bar" });
  const band = el("div", { class: "band" });
  bar.append(band, el("div", { class: "zero", style: `left:${((0 - DEPTH_MIN) / (DEPTH_MAX - DEPTH_MIN)) * 100}%` }));
  placeDepthBand(band, p.depth);
  inspectorWidgets.push((q) => placeDepthBand(band, q.depth));
  body.append(bar);
  // Front and Back push each other apart; an edit the clamp turns into the
  // current depth changes nothing.
  const depth = (which) => (v) => {
    const q = get();
    if (!q) return;
    const next = clampDepth(q.depth.front, q.depth.back, which, v);
    if (next.front !== q.depth.front || next.back !== q.depth.back) up(`depth:${id}`, (x) => (x.depth = next));
  };
  body.append(number(store, id, "Front", (q) => q.depth.front, DEPTH_MIN, DEPTH_MAX, 0.05, depth("front")));
  body.append(number(store, id, "Back", (q) => q.depth.back, DEPTH_MIN, DEPTH_MAX, 0.05, depth("back")));
  body.append(el("div", { class: "hint", text: "Front faces the player. The game's own disc spans 0.1 to -0.1." }));

  body.append(el("h2", { text: "Effects" }));
  // A chip: one button that switches the effect on (with its defaults) or
  // off; its fields follow while it is on. Switching rebuilds the panel
  // (the key carries the chip states), which is fine inside a click.
  const chip = (key, label, fields) => {
    const on = get()?.effects[key] != null;
    const btn = el("button", { class: `btn small${on ? " on" : ""}`, text: on ? "On" : "Off", "data-role": `chip-${key}`,
      onclick: () => up(`effect-${key}`, (x) => (x.effects[key] = x.effects[key] == null ? JSON.parse(JSON.stringify(EFFECT_DEFAULTS[key])) : null)) });
    body.append(el("div", { class: "kt-field" }, [el("label", { text: label }), btn]));
    if (on) for (const f of fields) body.append(f);
  };
  chip("waves", "Waves", [
    slider(p, "Count", (q) => q.effects.waves?.count ?? 6, 2, 16, 1, (v) => up(`waves.count:${id}`, (x) => { if (x.effects.waves) x.effects.waves.count = v; })),
    slider(p, "Depth", (q) => q.effects.waves?.depth ?? 0.15, 0.02, 0.6, 0.01, (v) => up(`waves.depth:${id}`, (x) => { if (x.effects.waves) x.effects.waves.depth = v; })),
    segmented(p, "Style", [["sine", "Sine"], ["spike", "Spike"], ["pulse", "Pulse"]], (q) => q.effects.waves?.style ?? "sine",
      (v) => { if (get()?.effects.waves?.style !== v) up(`waves.style:${id}`, (x) => { if (x.effects.waves) x.effects.waves.style = v; }); }),
  ]);
  chip("round", "Round", [slider(p, "Radius", (q) => q.effects.round ?? 0.1, 0.01, 0.5, 0.01, (v) => up(`round:${id}`, (x) => (x.effects.round = v)))]);
  chip("rim", "Rim", [slider(p, "Thickness", (q) => q.effects.rim ?? 0.15, 0.02, 0.5, 0.01, (v) => up(`rim:${id}`, (x) => (x.effects.rim = v)))]);
  body.append(el("div", { class: "hint", text: "Waves run inward from the part's edge, so a wavy part keeps its size. Round softens corners, Rim keeps only the outline band." }));

  body.append(el("h2", { text: "Symmetry" }));
  body.append(segmented(p, "Mirror", [["none", "None"], ["x", "X"], ["y", "Y"], ["both", "Both"]], (q) => q.effects.mirror,
    (v) => { if (get()?.effects.mirror !== v) up("mirror", (x) => (x.effects.mirror = v)); }));
  const repeat = el("input", { type: "number", min: 0, max: REPEAT_MAX, step: 1, "data-role": "repeat" });
  const repeatText = (q) => (q.effects.repeat == null ? "0" : String(q.effects.repeat));
  setShown(repeat, repeatText(p));
  const commitRepeat = () => {
    const q = get();
    if (!q) return;
    const next = repeatFromField(repeat.value);
    if (next !== q.effects.repeat) up("repeat", (x) => (x.effects.repeat = next));
    const now = get();
    if (now) setShown(repeat, repeatText(now));
  };
  repeat.addEventListener("change", commitRepeat);
  repeat.addEventListener("keydown", (e) => { if (e.key === "Enter") commitRepeat(); });
  inspectorWidgets.push((q) => showText(repeat, repeatText(q)));
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Repeat" }), repeat]));
  body.append(el("div", { class: "hint", text: "Mirror reflects the part across the note's axes; Repeat turns copies around the centre (2 to 12, 0 is off). Copies that overlap merge." }));

  body.append(el("div", { class: "kt-field" }, [
    el("button", { class: "btn small ghost", text: "Duplicate", onclick: () => store.duplicatePart(id) }),
    el("button", { class: "btn small ghost", text: "Remove", onclick: () => store.removePart(id) }),
  ]));
}

// The inspector of a selected group: name, mode, its parts, Ungroup.
function buildGroupInspector(store, body, g) {
  const id = g.id;
  const get = () => store.state.design.groups.find((x) => x.id === id);
  body.append(el("h2", { text: "Group" }));
  const name = el("input", { type: "text", "data-role": "group-name" });
  setShown(name, g.name);
  name.addEventListener("change", () => {
    const next = name.value.trim();
    if (next && next !== get()?.name) store.updateGroup(id, "group-name", (x) => (x.name = next));
    const now = get();
    if (now) setShown(name, now.name);
  });
  inspectorWidgets.push(() => {
    const now = get();
    if (now) showText(name, now.name);
  });
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Name" }), name]));
  const modeButtons = [["add", "Add"], ["cut", "Cut"]].map(([v, text]) => [v, el("button", { class: "btn small", text,
    onclick: () => { if (get()?.mode !== v) store.updateGroup(id, "group-mode", (x) => (x.mode = v)); } })]);
  const mark = () => {
    const now = get();
    for (const [v, b] of modeButtons) b.classList.toggle("on", now?.mode === v);
  };
  mark();
  inspectorWidgets.push(mark);
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Mode" }), el("div", { class: "kt-seg" }, modeButtons.map(([, b]) => b))]));
  body.append(el("div", { class: "hint", text: "A Cut part inside the group carves only this group's Add parts. A Cut group carves everything outside it with the shape of its Add parts." }));
  body.append(el("h2", { text: "Parts" }));
  for (const pid of g.parts) {
    const p = store.part(pid);
    if (!p) continue;
    body.append(el("div", { class: "kt-field" }, [
      el("label", { text: p.name }),
      el("button", { class: "btn small ghost", text: "Select", onclick: () => store.select(pid) }),
      el("button", { class: "btn small ghost", text: "Leave group", "data-role": "leave", onclick: () => store.removeFromGroup(pid) }),
    ]));
  }
  body.append(el("div", { class: "kt-field" }, [el("button", { class: "btn small ghost", text: "Ungroup", "data-role": "ungroup", onclick: () => store.ungroup(id) })]));
}

function renderInspector(store, actions) {
  const body = $("kt-inspector-body");
  const s = store.state;
  const g = s.selectedGroup ? s.design.groups.find((x) => x.id === s.selectedGroup) : null;
  const p = g ? null : store.part();
  // A group's key carries its members, so joins and leaves rebuild its list.
  const key = g ? `g|${g.id}|${g.parts.join(",")}` : p ? inspectorKeyFor(p) : "";
  if (key === inspectorKey) {
    const subject = p || g;
    if (subject) for (const sync of inspectorWidgets) sync(subject);
    return;
  }
  // The shape select rebuilds the panel it sits in; keyboard users stay on it.
  const focused = document.activeElement;
  const kindHadFocus = !!focused && !!focused.dataset && focused.dataset.role === "kind" && !!p && p.id === inspectorId;
  inspectorKey = key;
  inspectorId = p ? p.id : null;
  inspectorWidgets = [];
  kindSelect = null;
  body.replaceChildren();
  if (g) {
    buildGroupInspector(store, body, g);
    return;
  }
  if (!p) {
    body.append(el("div", { class: "hint", text: "Select a part, or add one from the toolbar." }));
    return;
  }
  buildInspector(store, body, p, actions);
  if (kindHadFocus && kindSelect) kindSelect.focus();
}

function buildColors(store, root) {
  const s = store.state;
  root.replaceChildren();
  colorRows = [];
  const count = s.design.colors.length;
  for (let i = 0; i < count; i++) {
    const row = el("div", { class: "kt-color-row", draggable: "true" });
    const pick = el("input", { type: "color" });
    const text = el("input", { type: "text", class: "hex" });
    const set = (v) => {
      const cur = s.design.colors[i];
      if (/^#[0-9a-fA-F]{6}$/.test(v) && cur !== undefined && v.toLowerCase() !== cur.toLowerCase()) {
        store.commit(`color:${i}`, (d) => (d.colors[i] = v.toLowerCase()));
      }
    };
    pick.addEventListener("input", () => set(pick.value));
    text.addEventListener("change", () => {
      set(text.value.trim());
      const now = s.design.colors[i];
      if (now !== undefined) setShown(text, now);
    });
    row.append(
      el("span", { class: "grip", text: "⋮⋮", title: "Drag to reorder: the notes play the colors in this order" }),
      pick,
      text,
      // The last colour cannot go: a note needs one.
      el("button", { class: "btn small ghost", text: "×", title: "Remove color", disabled: count <= 1 ? "" : null,
        onclick: () => { if (s.design.colors.length > 1) store.commit("color-remove", (d) => d.colors.splice(i, 1)); } }),
    );
    row.addEventListener("dragstart", () => { dragColorIdx = i; row.classList.add("dragging"); });
    row.addEventListener("dragend", () => { dragColorIdx = -1; row.classList.remove("dragging"); });
    row.addEventListener("dragover", (e) => e.preventDefault());
    row.addEventListener("drop", (e) => {
      e.preventDefault();
      if (dragColorIdx < 0 || dragColorIdx === i) return;
      const from = dragColorIdx;
      dragColorIdx = -1;
      store.commit("color-reorder", (d) => { const [c] = d.colors.splice(from, 1); d.colors.splice(i, 0, c); });
    });
    root.append(row);
    colorRows.push({ pick, text });
  }
}

function renderColors(store) {
  const colors = store.state.design.colors;
  const key = String(colors.length);
  if (key !== colorsKey) {
    colorsKey = key;
    buildColors(store, $("kt-colors"));
  }
  colors.forEach((hex, i) => {
    const r = colorRows[i];
    if (r.pick.value.toLowerCase() !== hex.toLowerCase()) r.pick.value = hex;
    showText(r.text, hex);
  });
}

// actions: { toPoints(id), editPoints(id, on) }, supplied by the owner of
// the IPC; the panels only call them.
export function render(store, actions = {}) {
  const s = store.state;
  renderParts(store);
  renderInspector(store, actions);
  renderColors(store);
  $("kt-undo").disabled = s.undo.length === 0;
  $("kt-redo").disabled = s.redo.length === 0;
  $("kt-previews").classList.toggle("on", s.ui.previews);
  document.title = `rhythr · Note Kit${s.dirty ? " *" : ""}${s.path ? ` · ${s.path.split(/[\\/]/).pop()}` : ""}`;
}
