// crates/gui/ui/kit/tests/store.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { createStore, newPart, COALESCE_MS, UNDO_LIMIT } from "../store.js";

function design() {
  return { version: 2, name: "t", colors: ["#ffffff"], parts: [newPart("p1", "circle")], groups: [] };
}

test("a_drag_is_one_undo_step", () => {
  let t = 1000;
  const s = createStore(design(), { now: () => t });
  for (let i = 0; i < 200; i++) {
    s.updatePart("p1", "move", (p) => (p.transform.x += 0.001));
    t += 10;
  }
  assert.equal(s.state.undo.length, 1);
  t += COALESCE_MS + 1;
  s.updatePart("p1", "move", (p) => (p.transform.x += 0.1));
  assert.equal(s.state.undo.length, 2);
  s.updatePart("p1", "rotate", (p) => (p.transform.rotation = 10));
  assert.equal(s.state.undo.length, 3, "a different label never merges");
  assert.ok(s.undo());
  assert.equal(s.part("p1").transform.rotation, 0);
  assert.ok(s.redo());
  assert.equal(s.part("p1").transform.rotation, 10);
});

test("add_duplicate_remove_keep_ids_unique_and_selection_valid", () => {
  const s = createStore(design());
  const id = s.addPart("star");
  assert.equal(id, "p2");
  assert.equal(s.state.selected, "p2");
  const copy = s.duplicatePart("p2");
  assert.equal(copy, "p3");
  assert.equal(s.part("p3").name, "Star copy");
  assert.equal(s.state.design.parts.map((p) => p.id).join(","), "p1,p2,p3");
  s.removePart("p3");
  assert.equal(s.state.selected, "p2", "selection falls back to the top part");
  s.state.design.groups.push({ id: "g", name: "g", parts: ["p1", "p2"], mode: "add" });
  s.removePart("p2");
  assert.deepEqual(s.state.design.groups[0].parts, ["p1"]);
  assert.ok(s.undo());
  assert.equal(s.state.design.parts.length, 2);
});

test("add_and_duplicate_emit_once_with_the_new_part_selected", () => {
  const s = createStore(design());
  const seen = [];
  s.subscribe((st) => seen.push(st.selected));
  assert.equal(s.addPart("star"), "p2");
  assert.deepEqual(seen, ["p2"], "one emit, already showing the new part");
  assert.equal(s.duplicatePart("p2"), "p3");
  assert.deepEqual(seen, ["p2", "p3"]);
});

test("end_gesture_splits_two_drags_inside_the_coalescing_window", () => {
  // Without endGesture: two quick drags on one part merge into one step.
  let t = 1000;
  const a = createStore(design(), { now: () => t });
  a.updatePart("p1", "move:p1", (p) => (p.transform.x = 0.1));
  t += 10;
  a.updatePart("p1", "move:p1", (p) => (p.transform.x = 0.2));
  assert.equal(a.state.undo.length, 1);
  // With it between them (the pointer was released): two steps.
  t = 1000;
  const b = createStore(design(), { now: () => t });
  b.updatePart("p1", "move:p1", (p) => (p.transform.x = 0.1));
  b.endGesture();
  t += 10;
  b.updatePart("p1", "move:p1", (p) => (p.transform.x = 0.2));
  assert.equal(b.state.undo.length, 2);
  assert.ok(b.undo());
  assert.equal(b.part("p1").transform.x, 0.1);
});

test("an_imported_outline_becomes_a_selected_part_with_its_source_as_name", () => {
  const s = createStore(design());
  const outline = [[[-1, -1], [1, -1], [0, 1]]];
  const id = s.addContours(outline, " A ");
  const p = s.part(id);
  assert.equal(p.shape.kind, "contours");
  assert.deepEqual(p.shape.contours, outline);
  assert.equal(p.shape.label, "A");
  assert.equal(p.name, "A");
  assert.deepEqual(p.transform, { x: 0, y: 0, w: 1, h: 1, rotation: 0 });
  assert.equal(s.state.selected, id);
  assert.equal(s.state.undo.length, 1);
  assert.equal(s.addContours(outline, "").length > 0 && s.part().name, "Outline");
  s.undo();
  s.undo();
  assert.equal(s.part(id), null);
});

test("svg_layers_become_parts_in_order_with_their_modes", () => {
  const s = createStore(design());
  const sq = [[[-1, -1], [1, -1], [1, 1], [-1, 1]]];
  const inner = [[[-0.4, -0.4], [0.4, -0.4], [0.4, 0.4], [-0.4, 0.4]]];
  const ids = s.addLayers([{ contours: sq, mode: "add", paint: "#ff4500" }, { contours: inner, mode: "cut", paint: "#ffffff" }], "badge.svg ");
  assert.deepEqual(ids, ["p2", "p3"]);
  assert.equal(s.part("p2").name, "badge.svg 1");
  assert.equal(s.part("p3").name, "badge.svg 2");
  assert.equal(s.part("p2").mode, "add");
  assert.equal(s.part("p3").mode, "cut");
  assert.equal(s.part("p3").shape.kind, "contours");
  assert.deepEqual(s.part("p3").shape.contours, inner);
  assert.equal(s.part("p3").shape.label, "badge.svg, colour #ffffff");
  assert.equal(s.state.selected, "p3", "the top colour is selected");
  assert.equal(s.state.undo.length, 1, "one import, one undo step");
  // A single colour keeps the plain name and label.
  const [one] = s.addLayers([{ contours: sq, mode: "add", paint: "#000000" }], "cloud");
  assert.equal(s.part(one).name, "cloud");
  assert.equal(s.part(one).shape.label, "cloud");
  assert.deepEqual(s.addLayers([], "x"), []);
  assert.equal(s.state.undo.length, 2, "nothing to add is no step");
});

function pointsDesign() {
  const d = design();
  const pt = (x, y, smooth = true) => ({ x, y, smooth });
  d.parts.push({
    ...newPart("p2", "circle"),
    id: "p2",
    shape: { kind: "points", control: [pt(-1, -1, false), pt(1, -1, false), pt(1, 1, false), pt(-1, 1, false)], loops: [[pt(-0.5, -0.5), pt(-0.5, 0.5), pt(0.5, 0.5)]] },
  });
  return d;
}

test("point_edits_change_the_loops_and_a_loop_keeps_three_points", () => {
  const s = createStore(pointsDesign());
  s.setPointEdit("p2");
  assert.equal(s.state.ui.pointEdit, "p2");
  assert.equal(s.pointLoops().length, 2);
  s.movePoint("p2", 0, 1, 1.2, -0.9);
  assert.deepEqual(s.part("p2").shape.control[1], { x: 1.2, y: -0.9, smooth: false });
  // Inserted between two corners: a corner, so the straight edge stays straight.
  const placed = s.insertPoint("p2", 0, 1, 0, -1);
  assert.deepEqual(placed, { loop: 0, index: 1 });
  assert.deepEqual(s.state.ui.pointSel, placed);
  assert.equal(s.part("p2").shape.control.length, 5);
  assert.equal(s.part("p2").shape.control[1].smooth, false);
  // Between a smooth point and anything: smooth.
  s.insertPoint("p2", 1, 1, -0.5, 0);
  assert.equal(s.part("p2").shape.loops[0][1].smooth, true);
  s.togglePoint("p2", 1, 1);
  assert.equal(s.part("p2").shape.loops[0][1].smooth, false);
  // Removing down to three points on the second loop, then the loop itself.
  assert.ok(s.removePoint("p2", 1, 1));
  assert.equal(s.part("p2").shape.loops[0].length, 3);
  assert.ok(s.removePoint("p2", 1, 0), "a three-point loop goes as a whole");
  assert.deepEqual(s.part("p2").shape.loops, []);
  assert.equal(s.state.ui.pointSel, null);
  // The last loop never drops below three points.
  for (let i = 0; i < 2; i++) assert.ok(s.removePoint("p2", 0, 0));
  assert.equal(s.part("p2").shape.control.length, 3);
  assert.equal(s.removePoint("p2", 0, 0), false);
  assert.equal(s.part("p2").shape.control.length, 3);
  // Discrete edits are separate undo steps; a drag coalesces.
  const steps = s.state.undo.length;
  s.movePoint("p2", 0, 0, 0.1, 0.1);
  s.movePoint("p2", 0, 0, 0.2, 0.2);
  assert.equal(s.state.undo.length, steps + 1);
});

test("point_editing_ends_when_its_part_changes_shape_or_goes", () => {
  const s = createStore(pointsDesign());
  s.setPointEdit("p2");
  s.selectPoint({ loop: 1, index: 2 });
  // Undo of an insert can leave the selection past the loop's end.
  s.insertPoint("p2", 1, 3, 0, 0);
  s.selectPoint({ loop: 1, index: 3 });
  s.undo();
  assert.equal(s.state.ui.pointEdit, "p2");
  assert.equal(s.state.ui.pointSel, null, "a selection past the end is dropped");
  s.updatePart("p2", "shape", (p) => (p.shape = { kind: "circle" }));
  assert.equal(s.state.ui.pointEdit, null, "no longer a Points part");
  s.setPointEdit("p2");
  s.removePart("p2");
  assert.equal(s.state.ui.pointEdit, null);
});

test("an_evaluate_error_keeps_the_last_good_geometry", () => {
  const s = createStore(design());
  const ev = { scale: 1, levels: [], parts: [], intervals: [], triangles: 0, mesh_error: null };
  s.setEvaluated(ev);
  s.setEvalError("x");
  assert.equal(s.state.evaluated, ev);
  assert.equal(s.state.evalError, "x");
});

test("move_reorders_within_bounds", () => {
  const s = createStore(design());
  s.addPart("ring");
  s.addPart("pill");
  s.movePart("p3", 0);
  assert.equal(s.state.design.parts[0].id, "p3");
  s.movePart("p3", 99);
  assert.equal(s.state.design.parts[2].id, "p3");
});

test("undo_is_bounded_and_load_resets_history", () => {
  let t = 0;
  const s = createStore(design(), { now: () => (t += COALESCE_MS + 1) });
  for (let i = 0; i < UNDO_LIMIT + 20; i++) s.updatePart("p1", "x", (p) => (p.transform.x = i));
  assert.equal(s.state.undo.length, UNDO_LIMIT);
  s.load(design(), "/tmp/a.rhsd");
  assert.equal(s.state.undo.length, 0);
  assert.equal(s.state.path, "/tmp/a.rhsd");
  assert.equal(s.state.dirty, false);
});

test("toggles_never_merge", () => {
  // Discrete clicks are separate undo steps even inside the coalescing window.
  const labels = ["visible", "lock", "mode", "material", "shape", "name", "color-add", "color-remove", "color-reorder"];
  for (const label of labels) {
    const s = createStore(design(), { now: () => 1000 });
    s.commit(label, (d) => (d.name = "a"));
    s.commit(label, (d) => (d.name = "b"));
    assert.equal(s.state.undo.length, 2, `${label} merged`);
  }
  // A picker drag is one gesture: same label, one step.
  let t = 1000;
  const s = createStore(design(), { now: () => t });
  s.commit("color:0", (d) => (d.colors[0] = "#111111"));
  t += 10;
  s.commit("color:0", (d) => (d.colors[0] = "#222222"));
  assert.equal(s.state.undo.length, 1);
  t += 10;
  s.commit("color:1", (d) => d.colors.push("#333333"));
  assert.equal(s.state.undo.length, 2, "another colour is another gesture");
});

test("serialize_writes_the_kit_format", () => {
  const s = createStore(design());
  const d = JSON.parse(s.serialize());
  assert.equal(d.version, 2);
  assert.deepEqual(d.parts[0].shape, { kind: "circle" });
  assert.deepEqual(d.parts[0].effects, { waves: null, round: null, rim: null, mirror: "none", repeat: null });
  assert.deepEqual(d.parts[0].depth, { front: 0.1, back: -0.1 });
});
