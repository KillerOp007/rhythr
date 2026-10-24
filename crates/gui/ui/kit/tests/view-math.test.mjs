// crates/gui/ui/kit/tests/view-math.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import {
  makeView, toPx, toWorld, zoomAt, partRect, rectCorners, rectContains, handleAt, scaleFrom, angleFrom,
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
  const k = scaleFrom(r, "ne", [2, 0.5], true);
  near(k.hw, 2);
  near(k.hh, 1);
  assert.equal(scaleFrom(r, "w", [0, 0], false).hw, 0.02, "never collapses");
});

test("the_rotation_handle_reads_degrees_from_the_top", () => {
  const r = partRect({ x: 0, y: 0, w: 1, h: 1, rotation: 0 }, 1);
  near(angleFrom(r, [0, 2]), 0);
  near(angleFrom(r, [-2, 0]), 90);
});
