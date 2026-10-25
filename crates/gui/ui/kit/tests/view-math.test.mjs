// crates/gui/ui/kit/tests/view-math.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import {
  makeView, toPx, toWorld, zoomAt, partRect, rectCorners, rectContains, handleAt, scaleFrom, angleFrom,
  unitToWorld, worldToUnit, nearestPoint, nearestSegment, snapUnit, depthShade,
  snapValue, snapPosition, snapSize, snapAngle,
} from "../view-math.js";

const size = { w: 800, h: 600 };
const near = (a, b, eps = 1e-6) => assert.ok(Math.abs(a - b) < eps, `${a} vs ${b}`);

test("px_and_world_round_trip_and_zoom_keeps_the_cursor_point", () => {
  const v = makeView();
  const [x, y] = toPx(v, size, [0.3, -0.2]);
  const [wx, wy] = toWorld(v, size, x, y);
  near(wx, 0.3);
  near(wy, -0.2);
  zoomAt(v, size, 500, 100, 2);
  const [ax, ay] = toWorld(v, size, 500, 100);
  const u = makeView();
  const [bx, by] = toWorld(u, size, 500, 100);
  near(ax, bx);
  near(ay, by);
});

test("a_rotated_rect_is_hit_in_its_own_frame", () => {
  // Half extents 1 by 0.2, turned 45 degrees: the point (0.6, 0.6) lies
  // on the long axis and is inside; (0.6, -0.6) is across it and outside.
  const r = partRect({ x: 0, y: 0, w: 1, h: 0.2, rotation: 45 }, 1);
  assert.ok(rectContains(r, [0.6, 0.6]));
  assert.ok(!rectContains(r, [0.6, -0.6]));
  // The scale maps document transforms onto the normalised drawing.
  const r2 = partRect({ x: 1, y: 0, w: 0.5, h: 0.5, rotation: 0 }, 0.5);
  assert.ok(rectContains(r2, [0.5, 0]));
  assert.ok(!rectContains(r2, [1, 0]));
});

test("corner_handles_follow_the_rotation", () => {
  const r = partRect({ x: 0, y: 0, w: 1, h: 0.5, rotation: 90 }, 1);
  const { corners } = rectCorners(r);
  // Local (1, 0.5) turned by 90 degrees is (-0.5, 1).
  near(corners[2][0], -0.5);
  near(corners[2][1], 1);
  assert.equal(handleAt(r, [-0.5, 1.0], 0.05), "ne");
  assert.equal(handleAt(r, [0, 0], 0.05), null);
});

test("scaling_is_about_the_centre_and_shift_keeps_aspect", () => {
  const r = partRect({ x: 0, y: 0, w: 1, h: 0.5, rotation: 0 }, 1);
  assert.deepEqual(scaleFrom(r, "e", [1.5, 0.1], false), { hw: 1.5, hh: 0.5 });
  const k = scaleFrom(r, "ne", [2, 0.5], true, Infinity);
  near(k.hw, 2);
  near(k.hh, 1);
  assert.equal(scaleFrom(r, "w", [0, 0], false).hw, 0.02, "never collapses");
  // At the size limit the proportions still hold: the common factor is
  // capped, not one side while the other keeps growing.
  const capped = scaleFrom(r, "ne", [2, 0.5], true);
  near(capped.hw, 1.5);
  near(capped.hh, 0.75);
  assert.deepEqual(scaleFrom(r, "e", [9, 0], false), { hw: 1.5, hh: 0.5 });
});

test("the_unit_frame_round_trips_through_scale_turn_and_move", () => {
  // The kernel's order: scale by w and h, turn, move. Unit (1, 0) on a part
  // half a unit wide turned 90 degrees and moved right lands at (0.5, 0.5).
  const r = partRect({ x: 0.5, y: 0, w: 0.5, h: 0.25, rotation: 90 }, 1);
  const w = unitToWorld(r, [1, 0]);
  near(w[0], 0.5);
  near(w[1], 0.5);
  const back = worldToUnit(r, w);
  near(back[0], 1);
  near(back[1], 0);
  const q = worldToUnit(r, [0.5 - 0.25, 0]);
  near(q[0], 0);
  near(q[1], 1, 1e-5);
  assert.deepEqual(snapUnit([0.26, -0.12], 0.05), [0.25, -0.1]);
});

test("the_nearest_point_and_segment_are_found_in_pixels", () => {
  const loops = [[[0, 0], [100, 0], [100, 100], [0, 100]], [[200, 200], [240, 200], [220, 240]]];
  assert.deepEqual(nearestPoint(loops, [98, 3], 9), { loop: 0, index: 1, dist: Math.hypot(2, 3) });
  assert.equal(nearestPoint(loops, [50, 50], 9), null);
  assert.equal(nearestPoint(loops, [222, 238], 9).loop, 1);
  // The top edge, halfway: a new point goes after point 0.
  const top = nearestSegment(loops, [50, 4]);
  assert.equal(top.loop, 0);
  assert.equal(top.index, 1);
  near(top.dist, 4);
  near(top.t, 0.5);
  // The closing edge from the last point back to the first: index 4 appends.
  const left = nearestSegment(loops, [3, 50]);
  assert.equal(left.index, 4);
  near(left.dist, 3);
  assert.equal(nearestSegment([[]], [0, 0]), null);
});

test("depth_shade_darkens_what_lies_below_the_top_and_lightens_a_dark_colour", () => {
  assert.equal(depthShade("#ffffff", 0.1, 0.1), "rgb(255,255,255)", "the top band keeps the colour");
  assert.equal(depthShade("#ffffff", -0.5, 0.1), "rgb(115,115,115)", "a full drop shades hardest");
  assert.equal(depthShade("#ffffff", 0.0, 0.1), "rgb(232,232,232)");
  assert.equal(depthShade("#ffffff", 0.4, 0.1), "rgb(255,255,255)", "nothing above the top is darkened");
  assert.equal(depthShade("#202020", -0.5, 0.1), "rgb(155,155,155)", "a dark colour lightens instead");
  assert.equal(depthShade("junk", -0.5, 0.1), "rgb(115,115,115)", "an unreadable colour counts as white");
});

test("the_rotation_handle_reads_degrees_from_the_top", () => {
  const r = partRect({ x: 0, y: 0, w: 1, h: 1, rotation: 0 }, 1);
  near(angleFrom(r, [0, 2]), 0);
  near(angleFrom(r, [-2, 0]), 90);
});

test("snapping_only_pulls_within_the_radius", () => {
  // Within 0.02 of a grid line: pulled; farther: left alone.
  assert.equal(snapValue(0.512, 0.02), 0.5);
  assert.equal(snapValue(0.525, 0.02), 0.525, "halfway between two lines, farther than the radius from both");
  // The box edge wins over the grid when it is nearer.
  assert.deepEqual(snapPosition(0.99, -0.012, 0.02), [1, 0]);
  assert.deepEqual(snapPosition(0.74, 0.3, 0.005), [0.74, 0.3]);
  assert.deepEqual(snapSize(0.49, 1.004, 0.02), [0.5, 1]);
  assert.equal(snapAngle(44, 3), 45);
  assert.equal(snapAngle(40, 3), 40);
  assert.equal(snapAngle(-178.5, 3), -180);
});

test("the_snap_radius_never_swallows_the_grid", async () => {
  const { snapTolerance, SNAP_GRID } = await import("../view-math.js");
  // At the default zoom 6 px would be 0.0335 of a 0.05 grid: capped so a
  // free band remains between the lines; zoomed in, the pixel radius rules.
  assert.equal(snapTolerance(179), SNAP_GRID * 0.4);
  near(snapTolerance(600), 0.01);
  assert.ok(snapTolerance(0) > 0 && snapTolerance(0) <= SNAP_GRID * 0.4);
});
