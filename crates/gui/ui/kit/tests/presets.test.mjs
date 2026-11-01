// crates/gui/ui/kit/tests/presets.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { tileModel } from "../presets.js";

test("tile_model_lists_ids_names_and_part_counts_in_order", () => {
  const tiles = tileModel([
    { id: "target", name: "Target", design: { parts: [{}, {}, {}] } },
    { id: "hex-frame", name: "Hex frame", design: { parts: [{}, {}] } },
  ]);
  assert.deepEqual(tiles, [
    { id: "target", name: "Target", parts: 3 },
    { id: "hex-frame", name: "Hex frame", parts: 2 },
  ]);
});
