// crates/gui/ui/kit/tests/render.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { fmt, roundToStep, clampDepth, shapeFields, pointSummary, EFFECT_DEFAULTS, repeatFromField, inspectorKeyFor, headerHasSelected, WAVES_DEPTH_MAX } from "../render.js";

test("a_folded_header_marks_its_selected_member", () => {
  const g = { id: "g1", parts: ["p2", "p3"] };
  assert.equal(headerHasSelected(g, true, "p3"), true);
  assert.equal(headerHasSelected(g, false, "p3"), false, "an open group shows the member itself");
  assert.equal(headerHasSelected(g, true, "p1"), false);
  assert.equal(headerHasSelected(g, true, null), false);
});

test("the_wave_depth_slider_reaches_the_kernel_limit", () => {
  // Migrated v1 depths above 0.6 used to sit past the slider; the kernel
  // clamps at 0.95, so the slider does too.
  assert.equal(WAVES_DEPTH_MAX, 0.95);
});

test("chips_toggle_discretely_and_repeat_below_two_is_off", () => {
  assert.deepEqual(EFFECT_DEFAULTS.waves, { count: 6, depth: 0.15, style: "sine" });
  assert.equal(EFFECT_DEFAULTS.round, 0.1);
  assert.equal(EFFECT_DEFAULTS.rim, 0.15);
  assert.equal(repeatFromField("0"), null);
  assert.equal(repeatFromField("1"), null);
  assert.equal(repeatFromField(""), null);
  assert.equal(repeatFromField("4.7"), 5);
  assert.equal(repeatFromField("40"), 12);
  const p = { id: "p1", shape: { kind: "circle" }, effects: { waves: null, round: 0.1, rim: null, mirror: "none", repeat: null } };
  assert.equal(inspectorKeyFor(p), "p1|circle|0|1|0");
  p.effects.waves = EFFECT_DEFAULTS.waves;
  assert.equal(inspectorKeyFor(p), "p1|circle|1|1|0", "a chip switching on rebuilds the inspector");
});

test("a_points_shape_is_summed_up_in_points_and_loops", () => {
  const pt = { x: 0, y: 0, smooth: true };
  assert.equal(pointSummary({ kind: "points", control: [pt, pt, pt], loops: [[pt, pt, pt, pt]] }), "7 points in 2 loops");
  assert.equal(pointSummary({ kind: "points", control: [pt, pt, pt] }), "3 points in 1 loop");
  assert.equal(pointSummary({ kind: "circle" }), "0 points in 0 loops");
});

test("numbers_show_as_many_decimals_as_their_step", () => {
  assert.equal(fmt(0.123456, 0.01), "0.12");
  assert.equal(fmt(6, 1), "6");
  assert.equal(fmt(-0.5, 0.05), "-0.50");
});

test("typed_values_round_to_their_field_step", () => {
  assert.equal(roundToStep(0.123456, 0.01), 0.12);
  assert.equal(roundToStep(6.4, 1), 6);
  assert.equal(roundToStep(-0.5004, 0.05), -0.5);
  assert.equal(roundToStep(0.5, 0.01), 0.5);
});

test("depth_keeps_front_above_back_inside_the_range", () => {
  // Pushing back up past front pushes front along, up to the range.
  assert.deepEqual(clampDepth(0.1, -0.1, "back", 0.3), { front: 0.35, back: 0.3 });
  assert.deepEqual(clampDepth(0.1, -0.1, "back", 0.49), { front: 0.5, back: 0.45 });
  assert.deepEqual(clampDepth(0.1, -0.1, "front", -0.5), { front: -0.5, back: -0.55 });
  assert.deepEqual(clampDepth(0.1, -0.1, "front", 9), { front: 0.5, back: -0.1 });
  assert.deepEqual(clampDepth(0.1, -0.1, "back", -9), { front: 0.1, back: -2 });
});

test("every_shape_kind_lists_its_own_fields", () => {
  assert.deepEqual(shapeFields("circle"), []);
  assert.equal(shapeFields("polygon").map((f) => f[0]).join(","), "sides,rounding");
  assert.equal(shapeFields("star")[1][0], "inner");
  assert.deepEqual(shapeFields("contours"), []);
});

test("the_parts_list_key_follows_membership_and_the_group_options_follow_names", async () => {
  const { listKey, groupOptionsKey } = await import("../render.js");
  const d = { parts: [{ id: "p1" }, { id: "p2" }], groups: [{ id: "g1", name: "G", parts: ["p1", "p2"], mode: "add" }] };
  const before = listKey(d, []);
  d.groups[0].parts = ["p2"];
  assert.notEqual(listKey(d, []), before, "a part leaving its group rebuilds the list");
  assert.notEqual(listKey(d, ["g1"]), listKey(d, []), "folding rebuilds");
  const k1 = groupOptionsKey(d, "g1");
  d.groups[0].name = "Holes";
  assert.notEqual(groupOptionsKey(d, "g1"), k1, "a renamed group refreshes the options");
  assert.notEqual(groupOptionsKey(d, null), groupOptionsKey(d, "g1"), "the current group is part of the key");
});
