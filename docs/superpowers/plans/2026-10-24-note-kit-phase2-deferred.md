# Deferred minors from the Phase 2 task reviews (for the final whole-branch review)

Each item was seen by a task reviewer, judged Minor or out of scope, and parked. The final reviewer triages: FIX NOW (worth the one fix wave before the testbuild), RECORD (document for Phase 3), NOT A DEFECT.

## Task 1 (kit_commands.rs)
- A part id containing a banned substring would get the generic sentence (ids are `p1` style; fails safe).
- One stderr line per evaluate on a persistently unbuildable design.
- "levels and intervals disagree" passes through (cannot happen from a real Evaluation).
- "design file damaged: {serde}" and "not a design file: {serde}" carry serde wording.
- `kit_doc_save` does not check `version`; the editor cannot produce another one.
- Non-atomic write (`std::fs::write`); same as the old designer.
- `skin_name_from`: `.RHS` uppercase not stripped; 64 leading spaces give "note".

## Task 2 (previews, window, build script)
- The destroy handlers clear the skins with `if let Ok` while the previews recover poisoned locks with `into_inner`; a poisoned skin mutex would survive a renderer drop.
- The kit destroy handler drops the renderer when no designer window exists but does not clear `designer_skin` (a designer preview queued on a worker after its window was destroyed; pre-existing shape).
- Test path literals joined with `/` (works on Windows, not component-wise).
- The GPU test compares a white and a red still; a black-frame check is implicit.

## Task 3 (store.js)
- Coalescing is keyed on the label only; gestures now carry the part id, so this is closed for gestures; inspector sliders also carry the id.
- `fixSelection` leaves nothing selected after undoing the removal of the last part.
- No-op commits (unknown id) still add undo steps and set dirty.
- `addPart` and `duplicatePart` emit twice (the first emit has a stale selection).

## Task 4 (view-math.js)
- `MIN_HALF` clamps only the dragged axis.
- `angleFrom` must be wrapped by the caller (view.js does).

## Task 5 (view.js, kit.html, kit.css, ipc.js, main.js)
- Wheel ignores `deltaMode`; a line-mode webview (deltaY about 3) zooms very slowly; WebView2 reports pixels, WebKitGTK may report lines: multiply line deltas by 16, page deltas by the box height.
- DPR is read only in `fit()`; a monitor change without a size change leaves the old density.
- The hatch pattern is anchored to the canvas origin and swims while panning.
- `hooks.onZoom` is required but the brief's interface named only `onEdit`.
- Fractional layout can put the bitmap under 0.5 px off the CSS box.
- Rotate and scale reference the part centre and factor captured at pointerdown; the drawn part may rescale about the origin when the normalisation factor changes mid-drag (inherent to auto-normalisation).
- Space `preventDefault` blocks Space on a focused button (protective: it would re-click "Star").

## Task 6 (render.js)
- `kindSelect.focus()` after a kind rebuild lacks `{ preventScroll: true }`.
- `roundToStep` rounds to the step's decimals, not to multiples of the step (consistent with `fmt`); rename or document.
- A focused kind select counts as typing (`isTyping` includes SELECT), so Delete, Ctrl+Z and nudges stay off until a click elsewhere; decide between blurring it after a change (as `kt-recent` does) and keeping arrow stepping.
- Mid-edit text in a field survives a same-structure load (Ctrl+O runs before the typing check) and commits into the new design on blur.
- Undo of a kind change while a field has focus rebuilds and drops focus (acceptable per ruling).
- `document.title` may not reach the native title bar in Tauri 2 (the window has a fixed `.title(...)`); check in the testbuild whether the `*` dirty marker shows.
- Arrow keys on the focused kind select step the kind (each a discrete undo step); fine.

## Task 7 (preview.js)
- No error backoff: a persistent preview failure retries the backend at about 30 Hz (the old designer waited 250 ms after an error); sleep 250 ms after a failed tick.
- The loop IIFE is unguarded: a throwing `onError` (for example a throwing render) kills the loop silently until reload; wrap the body in try/catch.
- Switching previews off leaves a preview error on the status line (nothing clears it); clear in `apply(false)`.
- The de-duplication and clear-only-if-ours logic lives in `mountPreviews` and is untested; a pure `createErrorReporter(store)` would allow a test.
- `visible` starts true without reading `document.hidden`; a hidden document with previews on wakes every 33 ms (idle at 150 ms when hidden).
- Splitter: a plain click freezes the responsive default height; no `pointercancel`; the 0.7 bound ignores the window chrome.
- Still versus frame error flicker when only one of them fails.
- `String(e)` gives an "Error:" prefix for Error objects (Tauri rejects with strings, so no effect).

## Task 8 (files.js, main.js, kit.html, main.rs)
- No "Save as": a design with a path always overwrites it (add Ctrl+Shift+S and a toolbar entry).
- A deleted recent file shows the Rust error and stays in the list (drop the entry on an unreadable file).
- The implementer's Playwright smoke with a mocked `window.__TAURI__` is not committed; it is the right headless check for this window (a script under `crates/gui/ui/kit/tests/` that is not run by `node --test`, documented in the guide or a README).

## Cross-task
- `isTyping` (view.js) is shared by the keyboard map (files.js) and the inspector focus rules (render.js); it counts SELECT as typing.
- The status line has three writers (evaluate, previews, file actions) with two ownership flags (`statusFromEval`, `lastPreviewError`); a small status owner abstraction would simplify Phase 3.
- Plan amendments pending at the end of the phase (one commit): the `DISCRETE` set and the gesture label scheme in Task 3 and 5 text, the node test command (`node --test crates/gui/ui/kit/tests/*.test.mjs`), the recursive build script, the capability entry, the render layer's rebuild-on-structure rule, the preview error de-duplication, the key order in `bindKeys`.
