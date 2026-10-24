// crates/gui/ui/kit/tests/preview.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { createPreviewScheduler, STILL_DELAY_MS } from "../preview.js";

function harness() {
  const calls = [];
  const timers = [];
  const h = {
    calls,
    timers,
    sched: null,
    fire() {
      const t = timers.splice(0);
      for (const fn of t) fn();
    },
  };
  h.sched = createPreviewScheduler({
    invoke: async (name, args) => {
      calls.push([name, args.still ? "still" : "loop", args.design.name]);
      return "data:image/png;base64,AAAA";
    },
    setTimer: (fn) => {
      timers.push(fn);
      return timers.length;
    },
    clearTimer: () => {},
    now: () => 0,
    onStill: () => {},
    onFrame: () => {},
    onError: () => {},
  });
  return h;
}

test("previews_off_makes_no_calls", async () => {
  const h = harness();
  h.sched.setEnabled(false);
  h.sched.designChanged({ name: "a" });
  h.fire();
  await h.sched.tick();
  assert.deepEqual(h.calls, []);
});

test("previews_on_render_the_still_after_the_delay_and_frames_on_tick", async () => {
  const h = harness();
  h.sched.setEnabled(true);
  h.sched.setVisible(true);
  h.sched.designChanged({ name: "a" });
  assert.deepEqual(h.calls, []);
  h.fire();
  await Promise.resolve();
  assert.deepEqual(h.calls, [["kit_preview", "still", "a"]]);
  await h.sched.tick();
  assert.deepEqual(h.calls[1], ["kit_preview", "loop", "a"]);
  h.sched.setEnabled(false);
  await h.sched.tick();
  assert.equal(h.calls.length, 2, "off stops the loop at once");
  h.sched.setEnabled(true);
  await h.sched.tick();
  assert.equal(h.calls.length, 3, "on resumes with the current design");
  assert.ok(STILL_DELAY_MS > 0);
});

// The de-duplication of repeated messages lives in mountPreviews; the
// scheduler must report every failing call and must not hand out an image.
test("a_failing_invoke_reaches_on_error_once_per_call_and_never_on_frame", async () => {
  const errors = [];
  let frames = 0;
  const sched = createPreviewScheduler({
    invoke: async () => {
      throw new Error("no gpu");
    },
    setTimer: () => 1,
    clearTimer: () => {},
    now: () => 0,
    onStill: () => frames++,
    onFrame: () => frames++,
    onError: (m) => errors.push(m),
  });
  sched.setEnabled(true);
  sched.designChanged({ name: "a" });
  await sched.tick();
  await sched.tick();
  assert.equal(errors.length, 2);
  assert.equal(errors[0], "Error: no gpu");
  assert.equal(frames, 0);
});
