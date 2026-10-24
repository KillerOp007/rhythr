// crates/gui/ui/kit/render.js
// Panels are rewritten from state on every change. Nothing in the DOM
// is a source of truth; a focused input keeps its own text until it
// commits, everything else is replaced.
import { DEPTH_MIN, DEPTH_MAX, SHAPES } from "./store.js";

export const MIN_BAND = 0.05;

export function fmt(v, step) {
  const d = step >= 1 ? 0 : Math.max(0, Math.ceil(-Math.log10(step)));
  return Number(v).toFixed(d);
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

const KIND_LABELS = { circle: "Disc", square: "Square", polygon: "Polygon", star: "Star", ring: "Ring", pill: "Pill", contours: "Outline (imported)", points: "Outline (freehand)" };

const $ = (id) => document.getElementById(id);

// Live widgets of the current inspector build, so a skipped rebuild can still
// refresh the fields that did not have focus (a depth edit moves the other end).
let numberFields = [];
let depthBand = null;
// What each panel was last built from. A render whose state shows the same
// thing leaves the DOM alone: an evaluate result must not replace the slider
// being dragged or the row being dragged.
let partsSig = null;
let inspectorSig = null;
let inspectorId = null;
let colorsSig = null;
// True while a slider's own input event is being committed: its value is
// already on screen, and rebuilding would end the drag under the pointer.
let sliderCommit = false;
// Drag sources live outside the panels: a render between dragstart and drop
// (an evaluate result) replaces the rows but not the drag in progress.
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

function slider(label, value, min, max, step, onInput) {
  const val = el("span", { class: "val", text: fmt(value, step) });
  const range = el("input", { type: "range", min, max, step, value });
  range.addEventListener("input", () => {
    val.textContent = fmt(range.value, step);
    sliderCommit = true;
    try {
      onInput(Number(range.value));
    } finally {
      sliderCommit = false;
    }
  });
  return el("div", { class: "kt-field" }, [el("label", { text: label }), range, val]);
}

// read(part) is the value the field shows; after a commit the field shows
// what the state kept (clamped, or untouched when the text was unusable).
function number(store, id, label, read, min, max, step, onCommit) {
  const input = el("input", { type: "number", min, max, step, value: fmt(read(store.part(id)), step) });
  const show = () => {
    const q = store.part(id);
    if (q) input.value = fmt(read(q), step);
  };
  const commit = () => {
    const q = store.part(id);
    const v = Number(input.value);
    if (q && input.value.trim() !== "" && Number.isFinite(v)) {
      const next = Math.max(min, Math.min(max, v));
      if (Math.abs(next - read(q)) > 1e-9) onCommit(next);
    }
    show();
  };
  input.addEventListener("change", commit);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") commit();
  });
  numberFields.push({ input, read, step });
  return el("div", { class: "kt-field" }, [el("label", { text: label }), input]);
}

function segmented(label, options, current, onPick) {
  const seg = el("div", { class: "kt-seg" }, options.map(([v, text]) =>
    el("button", { class: `btn small${v === current ? " on" : ""}`, text, onclick: () => onPick(v) })));
  return el("div", { class: "kt-field" }, [el("label", { text: label }), seg]);
}

function renderParts(store) {
  const s = store.state;
  const root = $("kt-parts");
  const sig = JSON.stringify([s.selected, s.design.parts.map((p) => [p.id, p.name, p.visible, p.locked, p.mode, p.material])]);
  if (sig === partsSig) return;
  partsSig = sig;
  root.replaceChildren();
  for (const p of s.design.parts) {
    const row = el("div", { class: `kt-part${p.id === s.selected ? " selected" : ""}${p.visible ? "" : " hidden-part"}`, draggable: "true" });
    row.append(
      el("button", { class: "btn small ghost", title: p.visible ? "Hide" : "Show", text: p.visible ? "👁" : "◌",
        onclick: (e) => { e.stopPropagation(); store.updatePart(p.id, "visible", (x) => (x.visible = !x.visible)); } }),
      el("button", { class: "btn small ghost", title: p.locked ? "Unlock" : "Lock", text: p.locked ? "🔒" : "🔓",
        onclick: (e) => { e.stopPropagation(); store.updatePart(p.id, "lock", (x) => (x.locked = !x.locked)); } }),
      el("span", { class: "name", text: p.name }),
      el("span", { class: `badge${p.mode === "cut" ? " cut" : ""}`, text: p.mode === "cut" ? "Cut" : "Add" }),
      el("span", { class: `badge${p.material === "glass" ? " glass" : ""}`, text: p.material === "glass" ? "Glass" : "Solid" }),
    );
    row.addEventListener("click", () => store.select(p.id));
    row.addEventListener("dragstart", (e) => { dragPartId = p.id; row.classList.add("dragging"); e.dataTransfer.effectAllowed = "move"; });
    row.addEventListener("dragend", () => { dragPartId = null; row.classList.remove("dragging"); });
    row.addEventListener("dragover", (e) => e.preventDefault());
    row.addEventListener("drop", (e) => {
      e.preventDefault();
      if (!dragPartId || dragPartId === p.id) return;
      const to = s.design.parts.findIndex((x) => x.id === p.id);
      const id = dragPartId;
      dragPartId = null;
      store.movePart(id, to);
    });
    root.append(row);
  }
}

function placeDepthBand(band, depth) {
  const span = DEPTH_MAX - DEPTH_MIN;
  const at = (z) => ((z - DEPTH_MIN) / span) * 100;
  band.style.left = `${at(depth.back)}%`;
  band.style.right = `${100 - at(depth.front)}%`;
}

function renderInspector(store) {
  const body = $("kt-inspector-body");
  const p = store.part();
  const sig = p ? JSON.stringify(p) : "";
  if (sig === inspectorSig) return;
  if (sliderCommit) {
    inspectorSig = sig;
    return;
  }
  const focused = document.activeElement;
  const typing = focused && body.contains(focused) && focused.tagName === "INPUT" && focused.type !== "range";
  if (typing && p && inspectorId === p.id) {
    // The typed text stays; the other fields and the depth bar follow the state.
    for (const f of numberFields) if (f.input !== focused) f.input.value = fmt(f.read(p), f.step);
    if (depthBand) placeDepthBand(depthBand, p.depth);
    inspectorSig = sig;
    return;
  }
  inspectorSig = sig;
  inspectorId = p ? p.id : null;
  body.replaceChildren();
  numberFields = [];
  depthBand = null;
  if (!p) {
    body.append(el("div", { class: "hint", text: "Select a part, or add one from the toolbar." }));
    return;
  }
  const id = p.id;
  const up = (label, patch) => store.updatePart(id, label, patch);
  body.append(el("h2", { text: "Part" }));
  const name = el("input", { type: "text", value: p.name });
  name.addEventListener("change", () => {
    const next = name.value.trim();
    if (next && next !== store.part(id)?.name) up("name", (x) => (x.name = next));
    name.value = store.part(id)?.name ?? name.value;
  });
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Name" }), name]));
  const kind = p.shape.kind;
  const kindSelect = el("select", {});
  for (const k of Object.keys(SHAPES)) kindSelect.append(el("option", { value: k, text: KIND_LABELS[k], selected: k === kind ? "" : null }));
  if (!(kind in SHAPES)) kindSelect.append(el("option", { value: kind, text: KIND_LABELS[kind] || kind, selected: "" }));
  kindSelect.addEventListener("change", () => up("shape", (x) => (x.shape = SHAPES[kindSelect.value]())));
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Shape" }), kindSelect]));
  for (const [key, label, min, max, step] of shapeFields(kind)) {
    body.append(slider(label, p.shape[key], min, max, step, (v) => up(`shape.${key}:${id}`, (x) => (x.shape[key] = v))));
  }
  body.append(segmented("Mode", [["add", "Add"], ["cut", "Cut"]], p.mode, (v) => up("mode", (x) => (x.mode = v))));
  body.append(segmented("Material", [["solid", "Solid"], ["glass", "Glass"]], p.material, (v) => up("material", (x) => (x.material = v))));
  body.append(el("h2", { text: "Position and size" }));
  const t = p.transform;
  body.append(number(store, id, "X", (q) => q.transform.x, -1.5, 1.5, 0.01, (v) => up(`x:${id}`, (x) => (x.transform.x = v))));
  body.append(number(store, id, "Y", (q) => q.transform.y, -1.5, 1.5, 0.01, (v) => up(`y:${id}`, (x) => (x.transform.y = v))));
  body.append(number(store, id, "Width", (q) => q.transform.w, 0.02, 1.5, 0.01, (v) => up(`w:${id}`, (x) => (x.transform.w = v))));
  body.append(number(store, id, "Height", (q) => q.transform.h, 0.02, 1.5, 0.01, (v) => up(`h:${id}`, (x) => (x.transform.h = v))));
  body.append(slider("Rotation", t.rotation, -180, 180, 1, (v) => up(`rotation:${id}`, (x) => (x.transform.rotation = v))));
  body.append(el("h2", { text: "Depth" }));
  const bar = el("div", { id: "kt-depth-bar" });
  depthBand = el("div", { class: "band" });
  placeDepthBand(depthBand, p.depth);
  bar.append(depthBand);
  bar.append(el("div", { class: "zero", style: `left:${((0 - DEPTH_MIN) / (DEPTH_MAX - DEPTH_MIN)) * 100}%` }));
  body.append(bar);
  body.append(number(store, id, "Front", (q) => q.depth.front, DEPTH_MIN, DEPTH_MAX, 0.05, (v) => up(`depth:${id}`, (x) => (x.depth = clampDepth(x.depth.front, x.depth.back, "front", v)))));
  body.append(number(store, id, "Back", (q) => q.depth.back, DEPTH_MIN, DEPTH_MAX, 0.05, (v) => up(`depth:${id}`, (x) => (x.depth = clampDepth(x.depth.front, x.depth.back, "back", v)))));
  body.append(el("div", { class: "hint", text: "Front faces the player. The game's own disc spans 0.1 to -0.1." }));
  body.append(el("div", { class: "kt-field" }, [
    el("button", { class: "btn small ghost", text: "Duplicate", onclick: () => store.duplicatePart(id) }),
    el("button", { class: "btn small ghost", text: "Remove", onclick: () => store.removePart(id) }),
  ]));
}

function renderColors(store) {
  const s = store.state;
  const root = $("kt-colors");
  const colors = s.design.colors;
  const sig = colors.join(",");
  if (sig === colorsSig) return;
  colorsSig = sig;
  const focused = document.activeElement;
  if (focused && root.contains(focused) && focused.tagName === "INPUT" && root.children.length === colors.length) {
    // A picker being dragged or a hex being typed must survive the commit it
    // caused: the other rows follow the state, the focused input stays.
    colors.forEach((hex, i) => {
      for (const input of root.children[i].querySelectorAll("input")) if (input !== focused) input.value = hex;
    });
    return;
  }
  root.replaceChildren();
  colors.forEach((hex, i) => {
    const row = el("div", { class: "kt-color-row", draggable: "true" });
    const pick = el("input", { type: "color", value: hex });
    const text = el("input", { type: "text", class: "hex", value: hex });
    const set = (v) => {
      if (/^#[0-9a-fA-F]{6}$/.test(v)) store.commit(`color:${i}`, (d) => (d.colors[i] = v.toLowerCase()));
      text.value = s.design.colors[i] ?? text.value;
    };
    pick.addEventListener("input", () => set(pick.value));
    text.addEventListener("change", () => set(text.value.trim()));
    row.append(
      el("span", { class: "grip", text: "⋮⋮", title: "Drag to reorder: the notes play the colors in this order" }),
      pick,
      text,
      el("button", { class: "btn small ghost", text: "×", title: "Remove color",
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
  });
}

export function render(store) {
  const s = store.state;
  renderParts(store);
  renderInspector(store);
  renderColors(store);
  $("kt-undo").disabled = s.undo.length === 0;
  $("kt-redo").disabled = s.redo.length === 0;
  $("kt-previews").classList.toggle("on", s.ui.previews);
  document.title = `rhythr · Note Kit${s.dirty ? " *" : ""}${s.path ? ` · ${s.path.split(/[\\/]/).pop()}` : ""}`;
}
