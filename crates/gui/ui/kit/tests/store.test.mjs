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

test("serialize_writes_the_kit_format", () => {
  const s = createStore(design());
  const d = JSON.parse(s.serialize());
  assert.equal(d.version, 2);
  assert.deepEqual(d.parts[0].shape, { kind: "circle" });
  assert.deepEqual(d.parts[0].effects, { waves: null, round: null, rim: null, mirror: "none", repeat: null });
  assert.deepEqual(d.parts[0].depth, { front: 0.1, back: -0.1 });
});
