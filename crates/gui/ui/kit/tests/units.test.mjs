import test from "node:test";
import assert from "node:assert/strict";
import { toDesignUnits, boxExtent } from "../units.js";

const ev = (scale) => ({
  scale,
  levels: [-0.1, 0.1],
  parts: [{ id: "p1", shapes: [[[[1, 0], [0, 1], [-1, 0]]]] }],
  intervals: [{ z0: -0.1, z1: 0.1, solid: [[[[1, 0], [0, 1], [-1, 0]]]], glass: [] }],
  triangles: 10,
  mesh_error: null,
});

test("normalised_geometry_becomes_design_units_by_the_factor", () => {
  // A design of extent 2 comes back normalised with scale 0.5; in design
  // units the point at normalised 1 sits at 2 again.
  const d = toDesignUnits(ev(0.5));
  assert.deepEqual(d.parts[0].shapes[0][0][0], [2, 0]);
  assert.deepEqual(d.intervals[0].solid[0][0][1], [0, 2]);
  assert.equal(d.scale, 0.5, "the factor stays for the box overlay");
  assert.equal(d.triangles, 10);
  assert.equal(boxExtent(d), 2);
});

test("a_design_already_at_the_box_is_returned_as_is", () => {
  const e = ev(1);
  assert.equal(toDesignUnits(e), e);
  assert.equal(boxExtent(e), 1);
});

test("a_bad_factor_is_left_alone", () => {
  const e = ev(0);
  assert.equal(toDesignUnits(e), e);
  assert.equal(boxExtent({ scale: NaN }), 1);
});
