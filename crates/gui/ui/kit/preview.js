// The previews are optional: hidden, they cost nothing (no calls, no
// loop); shown, the still re-renders shortly after the last change and
// the gameplay loop runs through the real renderer at about 30 fps.
export const STILL_DELAY_MS = 250;
export const LOOP_MS = 2700;
export const FRAME_MS = 33;
export const PREVIEWS_KEY = "rhythr.kit.previews";
export const PREVIEW_H_KEY = "rhythr.kit.previewH";

export function createPreviewScheduler(io) {
  let enabled = false;
  let visible = true;
  let design = null;
  let stillTimer = null;
  let stillBusy = false;
  let stillAgain = false;
  let frameBusy = false;
  const t0 = io.now();

  async function still() {
    if (!enabled || !design) return;
    if (stillBusy) {
      stillAgain = true;
      return;
    }
    stillBusy = true;
    try {
      io.onStill(await io.invoke("kit_preview", { design, timeMs: 0, still: true }));
    } catch (e) {
      io.onError(String(e));
    } finally {
      stillBusy = false;
      if (stillAgain) {
        stillAgain = false;
        still();
      }
    }
  }

  return {
    setEnabled(on) {
      enabled = on;
      if (on && design) {
        io.clearTimer(stillTimer);
        stillTimer = io.setTimer(still, STILL_DELAY_MS);
      }
    },
    setVisible(v) {
      visible = v;
    },
    designChanged(d) {
      design = d;
      if (!enabled) return;
      io.clearTimer(stillTimer);
      stillTimer = io.setTimer(still, STILL_DELAY_MS);
    },
    // One gameplay frame; the caller paces the calls.
    async tick() {
      if (!enabled || !visible || !design || frameBusy) return;
      frameBusy = true;
      try {
        const timeMs = (io.now() - t0) % LOOP_MS;
        io.onFrame(await io.invoke("kit_preview", { design, timeMs, still: false }));
      } catch (e) {
        io.onError(String(e));
      } finally {
        frameBusy = false;
      }
    },
  };
}

export function mountPreviews(store, els, invoke) {
  // A failing frame must not rewrite the panels thirty times a second, and a
  // recovered preview must not leave its old error on screen. Only the
  // status text this module wrote is ever cleared.
  let lastPreviewError = "";
  function clearPreviewError() {
    if (!lastPreviewError) return;
    if (store.state.status.text === lastPreviewError) store.setStatus("");
    lastPreviewError = "";
  }
  const sched = createPreviewScheduler({
    invoke,
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (t) => clearTimeout(t),
    now: () => performance.now(),
    onStill: (url) => {
      els.still.src = url;
      clearPreviewError();
    },
    onFrame: (url) => {
      els.loop.src = url;
      clearPreviewError();
    },
    onError: (msg) => {
      if (msg === lastPreviewError) return;
      lastPreviewError = msg;
      store.setStatus(msg, true);
    },
  });
  let saved = false;
  try {
    saved = localStorage.getItem(PREVIEWS_KEY) === "1";
  } catch {
    saved = false;
  }
  function apply(on) {
    store.setUi({ previews: on });
    els.box.hidden = !on;
    els.splitter.hidden = !on;
    sched.setEnabled(on);
    try {
      localStorage.setItem(PREVIEWS_KEY, on ? "1" : "0");
    } catch {
      // Best effort: the toggle still works for this session.
    }
  }
  els.toggle.addEventListener("click", () => apply(!store.state.ui.previews));
  document.addEventListener("visibilitychange", () => sched.setVisible(!document.hidden));
  (async function loop() {
    while (true) {
      const t = performance.now();
      await sched.tick();
      const took = performance.now() - t;
      await new Promise((r) => setTimeout(r, Math.max(FRAME_MS - took, store.state.ui.previews ? 1 : 150)));
    }
  })();
  // The splitter from the old designer, same stored height.
  const bar = els.splitter;
  const box = els.box;
  try {
    const h = Number(localStorage.getItem(PREVIEW_H_KEY));
    if (h >= 120 && h <= window.innerHeight * 0.7) box.style.flexBasis = `${h}px`;
  } catch {
    // Default balance.
  }
  let startY = 0;
  let startH = 0;
  bar.addEventListener("pointerdown", (e) => {
    startY = e.clientY;
    startH = box.getBoundingClientRect().height;
    bar.setPointerCapture(e.pointerId);
  });
  bar.addEventListener("pointermove", (e) => {
    if (!bar.hasPointerCapture(e.pointerId)) return;
    box.style.flexBasis = `${Math.max(120, Math.min(window.innerHeight * 0.7, startH - (e.clientY - startY)))}px`;
  });
  bar.addEventListener("pointerup", (e) => {
    if (!bar.hasPointerCapture(e.pointerId)) return;
    bar.releasePointerCapture(e.pointerId);
    try {
      localStorage.setItem(PREVIEW_H_KEY, String(Math.round(box.getBoundingClientRect().height)));
    } catch {
      // Best effort.
    }
  });
  apply(saved);
  return { apply, designChanged: (d) => sched.designChanged(d), toggle: () => apply(!store.state.ui.previews) };
}
