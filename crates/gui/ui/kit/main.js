// Bootstrap: the store, the front view, evaluation on every change, the
// document buttons and the keyboard.
import { createStore } from "./store.js";
import { createView } from "./view.js";
import { invoke, coalesced } from "./ipc.js";
import { render } from "./render.js";
import { mountPreviews } from "./preview.js";
import { toDesignUnits } from "./units.js";
import {
  makeRecent,
  autosave,
  startupDesign,
  hasAutosave,
  newDesign,
  importOldDesigner,
  parseDesign,
  fileStem,
  bindKeys,
} from "./files.js";

const $ = (id) => document.getElementById(id);

const DOC_FILTER = [{ name: "rhythr note design", extensions: ["rhsd"] }];
const RHS_FILTER = [{ name: "Rhythia skin", extensions: ["rhs"] }];
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
      // The view edits in design units; the kernel answers normalised.
      store.setEvaluated(toDesignUnits(ev));
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
  const view = createView($("kt-canvas"), store, {
    onEdit: () => {},
    onZoom: (pct) => ($("kt-view-reset").textContent = `${pct}%`),
  });
  $("kt-view-reset").addEventListener("click", view.resetView);
  const previews = mountPreviews(
    store,
    {
      toggle: $("kt-previews"),
      box: $("kt-preview-box"),
      splitter: $("kt-splitter"),
      still: $("kt-still"),
      loop: $("kt-preview"),
      error: $("kt-preview-error"),
    },
    invoke,
  );
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
    render(store);
    syncRecent();
    view.draw();
    const serial = store.serialize();
    if (serial !== lastSerial) {
      lastSerial = serial;
      autoSave();
      evaluate({ design: s.design });
      previews.designChanged(s.design);
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
  // What the backend read ({ json, migrated }) becomes the design. A file
  // converted from the old designer keeps no path: Save then asks for a new
  // file, so the old designer's document is never overwritten.
  async function openLoaded(path, loaded) {
    // Read and checked before asking, so a bad file never costs a prompt.
    const next = parseDesign(loaded.json);
    if (!(await discardOk("Open design"))) return;
    if (loaded.migrated) {
      store.load(next, null);
      recent.add(path);
      store.setStatus(`Opened and converted from the old designer: ${fileName(path)}; Save will ask for a new file`);
      return;
    }
    store.load(next, path);
    recent.add(path);
    store.setStatus(`Opened: ${path}`);
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
    const path = await dialog().open({ multiple: false, filters: DOC_FILTER });
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
  async function writeDoc(path) {
    const json = store.serialize();
    await invoke("kit_doc_save", { path, json });
    store.state.path = path;
    // An edit made while the write was in flight is still unsaved.
    if (store.serialize() === json) store.state.dirty = false;
    recent.add(path);
    store.setStatus(`Saved: ${path}`);
  }
  const defaultDocName = () => `${fileStem(store.state.design.name)}.rhsd`;
  const saveDoc = exclusive(async () => {
    let path = store.state.path;
    if (!path) {
      path = await dialog().save({ defaultPath: defaultDocName(), filters: DOC_FILTER });
      if (!path) return;
    }
    await writeDoc(path);
  });
  // Always asks, so a design with a file can be written to another one.
  const saveAsDoc = exclusive(async () => {
    const current = store.state.path;
    const path = await dialog().save({ defaultPath: current ? fileName(current) : defaultDocName(), filters: DOC_FILTER });
    if (!path) return;
    await writeDoc(path);
  });
  const importOld = exclusive(async () => {
    const next = await importOldDesigner(storage, invoke);
    if (!(await discardOk("From old designer"))) return;
    store.load(next, null);
    store.setStatus("Imported the note from the old Note Designer");
  });
  const exportRhs = exclusive(async () => {
    const path = await dialog().save({ defaultPath: `${fileStem(store.state.design.name)}.rhs`, filters: RHS_FILTER });
    if (!path) return;
    const out = await invoke("kit_export", { design: store.state.design, path });
    store.setStatus(`Exported: ${out || path}`);
  });
  $("kt-new").addEventListener("click", newDoc);
  $("kt-load").addEventListener("click", openDoc);
  $("kt-save").addEventListener("click", saveDoc);
  $("kt-save-as").addEventListener("click", saveAsDoc);
  $("kt-import-old").addEventListener("click", importOld);
  $("kt-export").addEventListener("click", exportRhs);
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
    { save: saveDoc, saveAs: saveAsDoc, open: openDoc, togglePreviews: previews.toggle },
    () => document.activeElement,
  );
  // The debounce must not hold back the last edit when the window goes.
  window.addEventListener("pagehide", autoSave.flush);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") autoSave.flush();
  });

  render(store);
  syncRecent();
  view.fit();
  evaluate({ design: store.state.design });
  // A remembered "on" state needs the starting design too; the subscriber
  // only sees later changes.
  previews.designChanged(store.state.design);
  window.kitStore = store;
}

boot().catch((e) => {
  const st = document.getElementById("kt-status");
  st.textContent = String(e);
  st.classList.add("err");
});
