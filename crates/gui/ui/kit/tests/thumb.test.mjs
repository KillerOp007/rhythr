// crates/gui/ui/kit/tests/thumb.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { thumbLayers, THUMB_PX, GLASS_ALPHA } from "../thumb.js";
import { depthShade } from "../view-math.js";

// One shape with one square contour of half-extent h, in note units.
const square = (h) => [[[[-h, -h], [h, -h], [h, h], [-h, h]]]];

function evaluation() {
  return {
    levels: [],
    parts: [],
    intervals: [
      { z0: -0.4, z1: -0.2, solid: square(1), glass: [] },
      { z0: -0.1, z1: 0.1, solid: square(0.5), glass: [] },
      { z0: -0.1, z1: 0.1, solid: [], glass: square(0.2) },
    ],
  };
}

test("thumb_layers_shade_the_bands_and_end_with_the_glass", () => {
  const layers = thumbLayers(evaluation(), "#ffffff");
  assert.equal(layers.length, 3);
  assert.equal(layers[1].fill, depthShade("#ffffff", 0.1, 0.1));
  assert.notEqual(layers[0].fill, layers[1].fill, "the lower band draws darker");
  assert.equal(layers[0].alpha, 1);
  assert.equal(layers[2].alpha, GLASS_ALPHA);
  assert.equal(layers[2].fill, "#ffffff");
  for (const layer of layers) {
    for (const poly of layer.polys) {
      for (const [x, y] of poly) {
        assert.ok(x >= 0 && x <= THUMB_PX && y >= 0 && y <= THUMB_PX, `${x},${y}`);
      }
    }
  }
});

test("thumb_layers_map_the_unit_box_inside_the_canvas_with_y_up", () => {
  const [layer] = thumbLayers({ levels: [], parts: [], intervals: [{ z0: 0, z1: 0.1, solid: square(1), glass: [] }] }, "#a2e0ff", 96);
  const ys = layer.polys[0].map((p) => p[1]);
  const xs = layer.polys[0].map((p) => p[0]);
  assert.ok(Math.min(...xs) > 0 && Math.max(...xs) < 96);
  // The first point is the bottom left corner in note units, so it lands
  // low on the canvas (pixel y grows downward).
  assert.ok(layer.polys[0][0][1] > 48);
  assert.ok(Math.max(...ys) - Math.min(...ys) > 60, "the box fills most of the thumbnail");
});
