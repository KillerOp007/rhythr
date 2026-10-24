// crates/gui/ui/kit/tests/preview.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { createPreviewScheduler, createErrorReporter, STILL_DELAY_MS } from "../preview.js";

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

// The de-duplication of repeated messages lives in createErrorReporter; the
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

// The loop backs off after a failed frame, so tick says whether it failed,
// and each error names its source for the reporter.
test("tick_resolves_false_when_invoke_rejects_and_true_on_success", async () => {
  let fail = true;
  const errors = [];
  const sched = createPreviewScheduler({
    invoke: async () => {
      if (fail) throw "no gpu";
      return "data:image/png;base64,AAAA";
    },
    setTimer: () => 1,
    clearTimer: () => {},
    now: () => 0,
    onStill: () => {},
    onFrame: () => {},
    onError: (m, source) => errors.push([m, source]),
  });
  sched.setEnabled(true);
  sched.designChanged({ name: "a" });
  assert.equal(await sched.tick(), false);
  assert.deepEqual(errors, [["no gpu", "loop"]]);
  fail = false;
  assert.equal(await sched.tick(), true);
  assert.equal(errors.length, 1);
});

test("the_error_reporter_shows_each_distinct_text_once_per_source", () => {
  const shown = [];
  const r = createErrorReporter((t) => shown.push(t));
  // Reports once per distinct text; a repeated text does not call setText again.
  r.report("loop", "no gpu");
  r.report("loop", "no gpu");
  r.report("loop", "no gpu");
  assert.deepEqual(shown, ["no gpu"]);
  r.report("loop", "too detailed");
  assert.deepEqual(shown, ["no gpu", "too detailed"]);
  // The still's error takes precedence; clearing it leaves the loop's visible.
  r.report("still", "still failed");
  assert.deepEqual(shown, ["no gpu", "too detailed", "still failed"]);
  r.clear("still");
  assert.deepEqual(shown, ["no gpu", "too detailed", "still failed", "too detailed"]);
  // Clearing a source with nothing to clear changes nothing on screen.
  r.clear("still");
  assert.equal(shown.length, 4);
  r.clear("loop");
  assert.deepEqual(shown.slice(4), [""]);
  // A re-report after a clear shows again.
  r.report("loop", "too detailed");
  assert.deepEqual(shown.slice(4), ["", "too detailed"]);
});

test("clearing_one_source_leaves_the_other_visible", () => {
  const shown = [];
  const r = createErrorReporter((t) => shown.push(t));
  r.report("loop", "loop failed");
  r.report("still", "still failed");
  r.clear("loop");
  assert.deepEqual(shown, ["loop failed", "still failed"], "the still's text was already showing");
  r.report("loop", "loop failed");
  r.clear("still");
  assert.deepEqual(shown, ["loop failed", "still failed", "loop failed"]);
});
