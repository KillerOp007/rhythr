// crates/gui/ui/kit/tests/ipc.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { coalesced } from "../ipc.js";

test("an_error_keeps_the_last_good_geometry", async () => {
  // Three requests while one is in flight: only the newest runs after,
  // and an error reaches onError without touching onResult.
  const calls = [];
  let resolveFirst;
  const fake = (name, args) =>
    new Promise((resolve, reject) => {
      calls.push(args.design);
      if (args.design === "bad") reject(new Error("nothing to show"));
      else if (calls.length === 1) resolveFirst = resolve;
      else resolve(`ok:${args.design}`);
    });
  const results = [];
  const errors = [];
  const run = coalesced("kit_evaluate", (r) => results.push(r), (e) => errors.push(e), fake);
  run({ design: "a" });
  run({ design: "b" });
  run({ design: "c" });
  resolveFirst("ok:a");
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(calls, ["a", "c"]);
  assert.deepEqual(results, ["ok:a", "ok:c"]);
  run({ design: "bad" });
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(results, ["ok:a", "ok:c"], "a failed call leaves the last result alone");
  assert.equal(errors.length, 1);
  assert.match(errors[0], /nothing to show/);
});
