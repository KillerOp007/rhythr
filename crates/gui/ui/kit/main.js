// Bootstrap: the store, the front view, evaluation on every change, the
// document buttons and the keyboard.
import { createStore } from "./store.js";
import { createView } from "./view.js";
import { invoke, coalesced } from "./ipc.js";
import { render, POINT_HELP } from "./render.js";
import { mountPreviews } from "./preview.js";
import { mountTurntable } from "./turntable.js";
import { renderHelp } from "./help.js";
import { mountPresets } from "./presets.js";
import {
  makeRecent,
  autosave,
  startupDesign,
  hasAutosave,
  newDesign,
  importOldDesigner,
  parseDesign,
  fileStem,
  docPathFor,
  glyphOf,
  bindKeys,
} from "./files.js";

const $ = (id) => document.getElementById(id);

// One file: Save writes a .rhs that the game imports and that carries the
// design, so Open takes it back. Plain .rhsd design files still open.
const OPEN_FILTER = [{ name: "Rhythia skin or rhythr design", extensions: ["rhs", "rhsd"] }];
const SAVE_FILTER = [{ name: "Rhythia skin", extensions: ["rhs"] }];
const SVG_FILTER = [{ name: "SVG image", extensions: ["svg"] }];
// The dialog plugin is read when a handler runs, never at load.
const dialog = () => window.__TAURI__.dialog;
const errText = (e) => String(e && e.message ? e.message : e);
const fileName = (path) => path.split(/[\\/]/).pop();

function webviewStorage() {
  try {
    return window.localStorage;
  } catch {
    return { getItem: () => null, setItem() {}, removeItem() {} };
  }
}

async function boot() {
  const storage = webviewStorage();
  const restored = hasAutosave(storage);
  const design = await startupDesign(storage, invoke);
  const store = createStore(design);
  store.state.dirty = restored;
  const recent = makeRecent(storage);
  const autoSave = autosave(storage, () => store.serialize());
  // The startup evaluate below is explicit, so the subscriber must see the
  // starting design as already sent.
  let lastSerial = store.serialize();
  // The status line is shared with the file actions. An evaluate clears only
  // what an evaluate wrote, so a good result never wipes a "Saved" message.
  let statusFromEval = false;
  let evalText = "";
  const evalStatus = (text) => {
    statusFromEval = true;
    evalText = text;
    store.setStatus(text, true);
  };
  const evaluate = coalesced(
    "kit_evaluate",
    (ev) => {
      store.setEvaluated(ev);
      $("kt-triangles").textContent = ev.triangles == null ? "" : `${ev.triangles} triangles`;
      if (ev.mesh_error) {
        evalStatus(ev.mesh_error);
      } else if (statusFromEval) {
        statusFromEval = false;
        store.setStatus("", false);
      }
    },
    (msg) => {
      // The count belongs to the last good result, which is no longer shown.
      $("kt-triangles").textContent = "";
      store.setEvalError(msg);
      evalStatus(msg);
    },
  );
  // What the panels and the canvas ask of the IPC owner; filled in below,
  // read when an event fires.
  const actions = {};
  const view = createView($("kt-canvas"), store, {
    onEdit: () => {},
    onZoom: (pct) => ($("kt-view-reset").textContent = `${pct}%`),
    // Double-click: a Points part opens for editing, any other converts.
    onDoubleClick: (part) => (part.shape.kind === "points" ? actions.editPoints(part.id, true) : actions.toPoints(part.id)),
  });
  $("kt-view-reset").addEventListener("click", view.resetView);
  // Snapping is remembered; the button mirrors the state.
  const SNAP_KEY = "rhythr.kit.snap";
  const setSnap = (on) => {
    store.setUi({ snap: on });
    $("kt-snap").classList.toggle("on", on);
    try {
      storage.setItem(SNAP_KEY, on ? "1" : "0");
    } catch {
      // Best effort: the toggle still works for this session.
    }
  };
  setSnap(storage.getItem(SNAP_KEY) !== "0");
  $("kt-snap").addEventListener("click", () => setSnap(!store.state.ui.snap));
  const previews = mountPreviews(
    store,
    {
      toggle: $("kt-previews"),
      box: $("kt-preview-box"),
      splitter: $("kt-splitter"),
      still: $("kt-still"),
      loop: $("kt-preview"),
      error: $("kt-preview-error"),
      figures: [$("kt-still-figure"), $("kt-loop-figure")],
    },
    invoke,
  );
  const turntable = mountTurntable(
    store,
    {
      toggle: $("kt-turntable"),
      figure: $("kt-turntable-figure"),
      canvas: $("kt-tt-canvas"),
      front: $("kt-tt-front"),
      threeQuarter: $("kt-tt-quarter"),
      shaded: $("kt-tt-shaded"),
      fade: $("kt-tt-fade"),
      note: $("kt-turntable-note"),
      box: $("kt-preview-box"),
      splitter: $("kt-splitter"),
    },
    invoke,
  );
  actions.toggleTurntable = turntable.toggle;
  // Help: the table of every control, as a modal; Esc closes it natively.
  const help = $("kt-help");
  renderHelp($("kt-help-body"));
  actions.help = () => {
    if (!help.open) help.showModal();
  };
  $("kt-help-btn").addEventListener("click", actions.help);
  $("kt-help-close").addEventListener("click", () => help.close());
  // A click on the backdrop closes it too.
  help.addEventListener("click", (e) => {
    if (e.target === help) help.close();
  });
  // The recent list is rebuilt from storage on every render, but the DOM
  // is touched only when it changed: an open dropdown survives the evaluate
  // results that arrive while it is open.
  let recentShown = "";
  function syncRecent() {
    const list = recent.list();
    const key = JSON.stringify(list);
    if (key === recentShown) return;
    recentShown = key;
    const sel = $("kt-recent");
    while (sel.options.length > 1) sel.remove(1);
    for (const p of list) {
      const o = document.createElement("option");
      o.value = p;
      o.textContent = p.split(/[\\/]/).pop();
      o.title = p;
      sel.add(o);
    }
    sel.disabled = list.length === 0;
    sel.value = "";
  }
  store.subscribe((s) => {
    render(store, actions);
    syncRecent();
    view.draw();
    const serial = store.serialize();
    if (serial !== lastSerial) {
      lastSerial = serial;
      autoSave();
      evaluate({ design: s.design });
      previews.designChanged(s.design);
      turntable.designChanged(s.design);
    }
    // Someone else wrote the status line: it is no longer ours to clear.
    if (statusFromEval && s.status.text !== evalText) statusFromEval = false;
    const st = $("kt-status");
    st.textContent = s.status.text;
    st.classList.toggle("err", s.status.error);
  });
  for (const b of document.querySelectorAll("[data-add]")) {
    b.addEventListener("click", () => store.addPart(b.dataset.add));
  }
  $("kt-color-add").addEventListener("click", () => store.commit("color-add", (d) => d.colors.push("#a2e0ff")));
  $("kt-undo").addEventListener("click", () => store.undo());
  $("kt-redo").addEventListener("click", () => store.redo());

  // Documents. One file action at a time: a second click or a held shortcut
  // must not stack dialogs or race two writes.
  const fail = (e) => store.setStatus(errText(e), true);
  let busy = false;
  const exclusive = (fn) => async (...args) => {
    if (busy) return;
    busy = true;
    try {
      await fn(...args);
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  };
  // Loading replaces the design and clears undo, so unsaved changes are
  // confirmed first. A dialog that cannot be shown aborts with its error.
  async function discardOk(title) {
    if (!store.state.dirty) return true;
    return dialog().ask("Discard the unsaved changes?", {
      title,
      kind: "warning",
      okLabel: "Discard",
      cancelLabel: "Cancel",
    });
  }
  // What the backend read ({ json, migrated }) becomes the design. Only a
  // skin archive keeps its path: a file converted from the old designer or
  // a plain .rhsd makes Save ask for a new .rhs, so nothing saved before is
  // overwritten with another format.
  async function openLoaded(path, loaded) {
    // Read and checked before asking, so a bad file never costs a prompt.
    const next = parseDesign(loaded.json);
    if (!(await discardOk("Open design"))) return;
    const keep = docPathFor(path, loaded.migrated);
    store.load(next, keep);
    recent.add(path);
    if (loaded.migrated) {
      store.setStatus(`Opened and converted from the old designer: ${fileName(path)}; Save will ask for a new .rhs file`);
    } else if (!keep) {
      store.setStatus(`Opened: ${fileName(path)}; Save will ask for a .rhs file`);
    } else {
      store.setStatus(`Opened: ${path}`);
    }
  }
  async function openPath(path) {
    await openLoaded(path, await invoke("kit_doc_load", { path }));
  }
  const newDoc = exclusive(async () => {
    if (!(await discardOk("New design"))) return;
    store.load(await newDesign(invoke), null);
    store.setStatus("");
  });
  const openDoc = exclusive(async () => {
    const path = await dialog().open({ multiple: false, filters: OPEN_FILTER });
    if (!path || Array.isArray(path)) return;
    await openPath(path);
  });
  // A recent entry the backend cannot read (moved, deleted, damaged) leaves
  // the list; the subscriber rebuilds the dropdown on the status update.
  const openRecent = exclusive(async (path) => {
    let loaded;
    try {
      loaded = await invoke("kit_doc_load", { path });
    } catch {
      recent.remove(path);
      store.setStatus(`${fileName(path)} could not be opened and was removed from the recent list`, true);
      return;
    }
    await openLoaded(path, loaded);
  });
  // Save bakes the skin with the design inside. A design the kernel refuses
  // (nothing visible, too detailed) is not written, because the game could
  // not load it; the autosave keeps the work meanwhile.
  async function writeDoc(path) {
    const json = store.serialize();
    let out;
    try {
      out = await invoke("kit_export", { design: store.state.design, path });
    } catch (e) {
      throw new Error(`Not saved, the skin would not load in the game: ${errText(e)}`);
    }
    store.state.path = out || path;
    // An edit made while the write was in flight is still unsaved.
    if (store.serialize() === json) store.state.dirty = false;
    recent.add(store.state.path);
    store.setStatus(`Saved: ${store.state.path}. Load it via Config in the main window or import it in the game.`);
  }
  const defaultDocName = () => `${fileStem(store.state.design.name)}.rhs`;
  const saveDoc = exclusive(async () => {
    let path = store.state.path;
    if (!path) {
      path = await dialog().save({ defaultPath: defaultDocName(), filters: SAVE_FILTER });
      if (!path) return;
    }
    await writeDoc(path);
  });
  // Always asks, so a design with a file can be written to another one.
  const saveAsDoc = exclusive(async () => {
    const current = store.state.path;
    const path = await dialog().save({ defaultPath: current ? fileName(current) : defaultDocName(), filters: SAVE_FILTER });
    if (!path) return;
    await writeDoc(path);
  });
  // Send to game writes the baked note into the game's own folders (one
  // profile, three files, see kit::game); the backend words every outcome,
  // success and refusal alike, so the status shows its sentence as is.
  const sendToGame = exclusive(async () => {
    const r = await invoke("kit_send_to_game", { design: store.state.design });
    store.setStatus(r.message);
  });
  // A preset is a complete design and replaces the note like a loaded file:
  // the same prompt about unsaved changes, no path, nothing dirty yet.
  actions.loadPreset = exclusive(async (preset) => {
    if (!(await discardOk("Preset"))) return;
    const next = parseDesign(JSON.stringify(preset.design));
    store.load(next, null);
    store.setStatus(`Preset ${preset.name}: ${next.parts.length} parts, all editable`);
  });
  mountPresets({ button: $("kt-presets"), panel: $("kt-presets-panel") }, { invoke, onPick: (p) => actions.loadPreset(p) });
  const importOld = exclusive(async () => {
    const next = await importOldDesigner(storage, invoke);
    if (!(await discardOk("From old designer"))) return;
    store.load(next, null);
    store.setStatus("Imported the note from the old Note Designer");
  });
  // Imports become parts: the backend resolves the file or the font once
  // and the outline is stored in the design, so the kernel never touches
  // either. Both land centred at the box size, like a shape from Add. An
  // SVG arrives as one layer per colour, each its own part, with strokes
  // outlined; a colour drawn on another comes as a Cut.
  const addSvg = exclusive(async () => {
    const path = await dialog().open({ multiple: false, filters: SVG_FILTER });
    if (!path || Array.isArray(path)) return;
    const layers = await invoke("kit_import_svg", { path });
    store.addLayers(layers, fileName(path).replace(/\.svg$/i, ""));
    if (layers.length === 1) {
      store.setStatus(`Added ${fileName(path)} as a part`);
    } else {
      const cuts = layers.filter((l) => l.mode === "cut").length;
      const cutNote = cuts ? `; ${cuts} drawn on another colour came in as Cut` : "";
      store.setStatus(`Added ${fileName(path)} as ${layers.length} parts, one per colour${cutNote}`);
    }
  });
  // Freeform: the kernel thins the part's outline to control points; the
  // part keeps its transform, mode, depth and effects.
  actions.toPoints = exclusive(async (id) => {
    const p = store.part(id);
    if (!p) return;
    const shape = await invoke("kit_shape_to_points", { shape: p.shape });
    store.updatePart(id, "shape", (x) => (x.shape = shape));
    store.setPointEdit(id);
    store.setStatus(`Editing points. ${POINT_HELP}`);
  });
  actions.editPoints = (id, on) => {
    store.setPointEdit(on ? id : null);
    store.setStatus(on ? `Editing points. ${POINT_HELP}` : "");
  };
  const glyphField = $("kt-glyph");
  const addGlyph = exclusive(async () => {
    const ch = glyphOf(glyphField.value);
    if (!ch) {
      glyphField.focus();
      store.setStatus("Type a letter, a digit or an emoji first", true);
      return;
    }
    const contours = await invoke("designer_import_glyph", { ch });
    store.addContours(contours, ch);
    store.setStatus(`Added "${ch}" as a part`);
  });
  $("kt-new").addEventListener("click", newDoc);
  $("kt-load").addEventListener("click", openDoc);
  $("kt-save").addEventListener("click", saveDoc);
  $("kt-save-as").addEventListener("click", saveAsDoc);
  $("kt-send").addEventListener("click", sendToGame);
  $("kt-import-old").addEventListener("click", importOld);
  $("kt-add-svg").addEventListener("click", addSvg);
  $("kt-add-glyph").addEventListener("click", addGlyph);
  glyphField.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (!e.repeat) addGlyph();
    }
  });
  $("kt-recent").addEventListener("change", (e) => {
    const sel = e.currentTarget;
    const path = sel.value;
    sel.value = "";
    // A focused select counts as typing, which would switch the shortcuts
    // off until the next click elsewhere.
    sel.blur();
    if (path) openRecent(path);
  });
  bindKeys(
    store,
    { save: saveDoc, saveAs: saveAsDoc, open: openDoc, togglePreviews: previews.toggle, toggleTurntable: turntable.toggle, help: actions.help },
    () => document.activeElement,
  );
  // The debounce must not hold back the last edit when the window goes.
  window.addEventListener("pagehide", autoSave.flush);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") autoSave.flush();
  });

  render(store, actions);
  syncRecent();
  view.fit();
  evaluate({ design: store.state.design });
  // A remembered "on" state needs the starting design too; the subscriber
  // only sees later changes.
  previews.designChanged(store.state.design);
  turntable.designChanged(store.state.design);
  window.kitStore = store;
}

boot().catch((e) => {
  const st = document.getElementById("kt-status");
  st.textContent = String(e);
  st.classList.add("err");
});
