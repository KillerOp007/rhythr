// crates/gui/ui/kit/tests/turntable-math.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import {
  clampOrbit, orbitEye, cameraMatrices, transformPoint, expandFaces, boxLines, ORBIT_LIMIT, ZOOM_MIN, ZOOM_MAX, DEFAULT_DISTANCE,
} from "../turntable-math.js";

const near = (a, b, eps = 1e-5) => assert.ok(Math.abs(a - b) < eps, `${a} vs ${b}`);

test("the_orbit_stays_within_sixty_degrees_and_the_zoom_range", () => {
  assert.deepEqual(clampOrbit({ yaw: 400, pitch: -90, dist: 0 }), { yaw: ORBIT_LIMIT, pitch: -ORBIT_LIMIT, dist: ZOOM_MIN });
  assert.deepEqual(clampOrbit({ yaw: 10, pitch: 5, dist: 100 }), { yaw: 10, pitch: 5, dist: ZOOM_MAX });
});

test("the_eye_starts_in_front_and_moves_right_and_up_with_the_orbit", () => {
  const front = orbitEye({ yaw: 0, pitch: 0, dist: DEFAULT_DISTANCE });
  near(front[0], 0);
  near(front[1], 0);
  near(front[2], DEFAULT_DISTANCE);
  const right = orbitEye({ yaw: 90, pitch: 0, dist: 2 });
  near(right[0], 2);
  near(right[2], 0);
  const above = orbitEye({ yaw: 0, pitch: 90, dist: 2 });
  near(above[1], 2);
});

test("the_front_camera_projects_the_centre_to_the_middle_and_the_front_nearer", () => {
  const { mvp } = cameraMatrices({ yaw: 0, pitch: 0, dist: 4.5 }, 1);
  const centre = transformPoint(mvp, [0, 0, 0]);
  near(centre[0], 0);
  near(centre[1], 0);
  // A point at the right edge of the box lands right of the centre, one
  // on the front face nearer (smaller depth) than one at the back.
  assert.ok(transformPoint(mvp, [1, 0, 0])[0] > 0.3);
  assert.ok(transformPoint(mvp, [0, 0, 0.5])[2] < transformPoint(mvp, [0, 0, -2])[2]);
  // Turned to the right, the right edge comes toward the camera.
  const turned = cameraMatrices({ yaw: 45, pitch: 0, dist: 4.5 }, 1).mvp;
  assert.ok(transformPoint(turned, [1, 0, 0])[2] < transformPoint(turned, [-1, 0, 0])[2]);
});

test("faces_expand_one_to_one_with_their_own_normal", () => {
  const positions = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]];
  const indices = [0, 1, 2, 0, 2, 1];
  const f = expandFaces(positions, indices);
  assert.equal(f.triangles, 2);
  assert.equal(f.positions.length, 18);
  // Counterclockwise seen from +z faces +z; the reversed one faces -z.
  near(f.normals[2], 1);
  near(f.normals[11], -1);
  // Vertex i of the output is index i of the input.
  assert.deepEqual([...f.positions.slice(3, 6)], [1, 0, 0]);
  assert.equal(boxLines().length, 24);
});
