# Final fix wave (one dispatch) after the whole-branch review of 536688b..a97af7c

Every item is a controller ruling. Do them in order; after each step the node tests (`node --test crates/gui/ui/kit/tests/*.test.mjs`) and, for Rust steps, `cargo test -p rhythia-gui` stay green; `scripts/preflight.sh` at the end. No em or en dashes anywhere. Nothing under `docs/superpowers/`.

## 1. Rust: the native title, poisoned locks, the kit destroy handler (`crates/gui/src/main.rs`)

Tauri 2 installs no document-title handler, so `document.title` written by `render.js` never reaches the native title bar; the dirty `*` and the file name are invisible. In `open_kit_window`, add `.on_document_title_changed(|w, title| { let _ = w.set_title(&title); })` to the `WebviewWindowBuilder` chain (the method exists on `WebviewWindowBuilder` in Tauri 2.11; if the closure signature differs, read `tauri-2.11.5/src/webview/webview_window.rs` in the cargo registry and use the one that compiles) with a one-line comment stating why.

The three destroy-handler sites that clear skins with `if let Ok(mut skin) = ...lock()` become `let mut skin = ...lock().unwrap_or_else(|p| p.into_inner()); *skin = None;` (the previews recover poisoned locks the same way; a poisoned skin mutex must not survive a renderer drop). The kit window's destroy handler also clears `designer_skin` when it drops the renderer (a designer frame queued after its window's Destroyed would otherwise meet a new device).

Check: `cargo build -p rhythia-gui` and `cargo test -p rhythia-gui`.

## 2. Rust: migration flag and the editor's document (`crates/gui/src/kit_commands.rs`)

`load_document(text) -> Result<Loaded, String>` with `#[derive(Serialize)] pub struct Loaded { pub json: String, pub migrated: bool }` (`migrated` true when the old designer path ran); `kit_doc_load` returns `Loaded`; `kit_migrate_v1` keeps returning the JSON string (it is always a migration). Update the existing tests (`load_document(...).unwrap().json`), add assertions that the v1 fixture gives `migrated: true` and a v2 document `false`. Add `a_part_built_by_the_editor_parses`: the six shape literals the editor writes (`{"kind":"circle"}`, `{"kind":"square","rounding":0}`, `{"kind":"polygon","sides":6,"rounding":0}`, `{"kind":"star","points":5,"inner":0.5}`, `{"kind":"ring","thickness":0.28}`, `{"kind":"pill","aspect":0.5}`) each inside a full part object exactly as `store.js`'s `newPart` writes it (`id`, `name`, `visible`, `locked`, `shape`, `transform {x,y,w,h,rotation}`, `mode "add"`, `depth {front:0.1, back:-0.1}`, `material "solid"`, `effects {waves:null, round:null, rim:null, mirror:"none", repeat:null}`) inside a design with `version 2`, `name`, `colors`, `parts`, `groups: []` must deserialise as `Design` and evaluate without error.

## 3. Front-end files: converted loads, Save as, recent cleanup (`crates/gui/ui/kit/files.js`, `main.js`, `kit.html`)

- `openPath` reads `{ json, migrated }`; a migrated load calls `store.load(next, null)` (no path, so Save asks for a new file) and sets the status "Opened and converted from the old designer: <file name>; Save will ask for a new file" in the normal style; a plain load keeps the path as today.
- "Save as": a toolbar button `kt-save-as` ("Save as") next to Save, and Ctrl+Shift+S in `bindKeys` (checked before the plain Ctrl+S branch; it blurs a text-entry field first like Ctrl+S), calling `actions.saveAs()` which always shows the save dialog with `defaultPath` = the current file name when a path is set, else `<design name>.rhsd`, then writes, sets the path, clears dirty, adds to recent, status `Saved: <path>`.
- `makeRecent` gains `remove(path)`; when `kit_doc_load` rejects for a recent entry, the entry is removed and the status says "<file name> could not be opened and was removed from the recent list" in the error style.
- Tests in `tests/files.test.mjs`: Ctrl+Shift+S calls `saveAs` even with a path set and Ctrl+S calls `save`; `remove` drops the entry and leaves the others.

## 4. Preview errors get their own place (`crates/gui/ui/kit/preview.js`, `kit.html`, `kit.css`, `tests/preview.test.mjs`)

The status line has three writers and the preview one fails under composition (verified: a repeated preview error stayed hidden after an evaluate error replaced and cleared the line). Preview errors move into the preview pane: an element `<div id="kt-preview-error" class="hint err" hidden></div>` inside `#kt-preview-box` above the row; `kit.css` styles `#kt-preview-error { color: var(--danger); }`. `preview.js` exports a pure `createErrorReporter(setText)` that keeps `{ still: "", loop: "" }`, `report(source, msg)` sets the source's text and calls `setText(still || loop)` only when the shown text changes, `clear(source)` clears that source and updates likewise; `mountPreviews` wires it to the element (`setText` writes `textContent` and toggles `hidden`), `onStill`/`onFrame` clear their source, errors report their source; `lastPreviewError` and every `store.setStatus` call in `preview.js` are removed. The loop: wrap the body of the `while (true)` in try/catch (an exception in `tick` or a callback must not kill the loop); `tick` returns `false` on a failed call and `true` otherwise; the loop sleeps 250 ms after a failed tick, 150 ms when previews are off or the document is hidden, else `max(FRAME_MS - took, 1)`. Tests: `createErrorReporter` (reports once per distinct text, a repeated text does not call `setText` again, `clear` of one source leaves the other visible, a re-report after a clear shows again); the scheduler test asserts `tick()` resolves `false` when `invoke` rejects and `true` on success.

## 5. Store polish (`crates/gui/ui/kit/store.js`, `view.js`, `tests/store.test.mjs`)

- `addPart` and `duplicatePart` set `s.selected` inside the commit's mutate closure so each emits once (keep the return values).
- `endGesture()`: resets `s.last` to the empty marker so the next commit of the same label starts a new undo step; `view.js`'s `end` handler calls `store.endGesture()` when a gesture was active (two separate drags on one part within 350 ms are then two steps). Test: two `updatePart("p1", "move:p1", ...)` commits separated by `endGesture()` inside the window produce two undo steps; without it one.
- Test `an_evaluate_error_keeps_the_last_good_geometry`: `setEvaluated(ev)` then `setEvalError("x")` leaves `state.evaluated === ev`.

## 6. Wheel delta mode (`crates/gui/ui/kit/view.js`)

Before computing the factor: `let dy = e.deltaY; if (e.deltaMode === 1) dy *= 16; else if (e.deltaMode === 2) dy *= size.h;` with a comment (WebView2 reports pixels, WebKitGTK may report lines). Syntax check only.

## 7. Checks and commits

`node --experimental-default-type=module --check` on every module you touched, `node --test crates/gui/ui/kit/tests/*.test.mjs`, `cargo test -p rhythia-gui`, `cargo build -p rhythia-gui`, `scripts/preflight.sh`. Two commits: (a) items 1 and 2 as "Kit window: the native title follows the document, converted loads are flagged, locks recover", (b) items 3 to 6 as "Kit window: Save as, converted loads keep the old file, preview errors in the pane, one emit per add, wheel delta modes". Both with the trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
