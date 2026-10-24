// Documents: autosave in the webview's storage so closing never loses
// work, a recent list, the old designer's autosave as an import, and the
// keyboard. Storage, invoke and window are injected or read inside the
// functions, so this module loads under node without a DOM.
export const DOC_KEY = "rhythr.kit.doc";
export const RECENT_KEY = "rhythr.kit.recent";
export const OLD_DOC_KEY = "rhythr.designer.doc";
export const RECENT_LIMIT = 6;
export const AUTOSAVE_MS = 600;

export function makeRecent(storage) {
  const read = () => {
    try {
      const v = JSON.parse(storage.getItem(RECENT_KEY) || "[]");
      return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
    } catch {
      return [];
    }
  };
  return {
    list: read,
    add(path) {
      const l = [path, ...read().filter((p) => p !== path)].slice(0, RECENT_LIMIT);
      try {
        storage.setItem(RECENT_KEY, JSON.stringify(l));
      } catch {
        // Best effort.
      }
      return l;
    },
  };
}

// Debounced: the returned function schedules a write of serialize(). Its
// flush() writes a pending one at once, for the window closing inside the
// debounce.
export function autosave(storage, serialize) {
  let timer = null;
  const write = () => {
    timer = null;
    try {
      storage.setItem(DOC_KEY, serialize());
    } catch {
      // Storage full or unavailable: autosave is best effort.
    }
  };
  const schedule = () => {
    clearTimeout(timer);
    timer = setTimeout(write, AUTOSAVE_MS);
  };
  schedule.flush = () => {
    if (timer === null) return;
    clearTimeout(timer);
    write();
  };
  return schedule;
}

export function parseDesign(text) {
  const d = JSON.parse(text);
  if (!d || d.version !== 2 || !Array.isArray(d.parts)) throw new Error("not a kit design");
  if (!Array.isArray(d.groups)) d.groups = [];
  if (!Array.isArray(d.colors) || !d.colors.length) d.colors = ["#ffffff"];
  return d;
}

export async function newDesign(invoke) {
  return parseDesign(await invoke("kit_default_doc"));
}

// Whether startupDesign will restore a saved design. The autosave does not
// say whether it was ever written to a file, so a restored one counts as
// unsaved work.
export function hasAutosave(storage) {
  try {
    parseDesign(storage.getItem(DOC_KEY));
    return true;
  } catch {
    return false;
  }
}

export async function startupDesign(storage, invoke) {
  try {
    const saved = storage.getItem(DOC_KEY);
    if (saved) return parseDesign(saved);
  } catch {
    // A damaged autosave is not worth a dialog; start fresh.
  }
  return newDesign(invoke);
}

export async function importOldDesigner(storage, invoke) {
  const old = storage.getItem(OLD_DOC_KEY);
  if (!old) throw new Error("nothing saved in the old Note Designer on this machine");
  return parseDesign(await invoke("kit_migrate_v1", { json: old }));
}

// A design's name as a file name: the dialog's default and, on the Rust side,
// the skin name come from the file stem, so characters a file system
// refuses are replaced rather than left for the dialog to reject.
export function fileStem(name) {
  const s = String(name || "").replace(/[\\/:*?"<>|\u0000-\u001f]/g, "-").trim();
  return s && s !== "." && s !== ".." ? s : "my-note";
}

// Ctrl+S and Ctrl+O work from a focused field (they commit nothing
// destructive); every other key, undo included, belongs to the field's own
// text while one has focus.
export function bindKeys(store, actions, isTyping) {
  window.addEventListener("keydown", (e) => {
    const mod = e.ctrlKey || e.metaKey;
    // Autofill dispatches keydown events without a key.
    const key = String(e.key || "").toLowerCase();
    // A held key must not stack dialogs, repeat a removal or flip the previews.
    if (mod && key === "s") { e.preventDefault(); if (!e.repeat) actions.save(); return; }
    if (mod && key === "o") { e.preventDefault(); if (!e.repeat) actions.open(); return; }
    if (isTyping()) return;
    if (mod && key === "z" && !e.shiftKey) { e.preventDefault(); store.undo(); return; }
    if (mod && (key === "y" || (key === "z" && e.shiftKey))) { e.preventDefault(); store.redo(); return; }
    if (!mod && !e.altKey && key === "p") { if (!e.repeat) actions.togglePreviews(); return; }
    if (e.key === "Escape") { store.select(null); return; }
    const p = store.part();
    if (!p) return;
    if (e.key === "Delete" || e.key === "Backspace") { e.preventDefault(); if (!e.repeat) store.removePart(p.id); return; }
    if (mod && key === "d") { e.preventDefault(); if (!e.repeat) store.duplicatePart(p.id); return; }
    if (mod || e.altKey) return;
    const step = e.shiftKey ? 0.1 : 0.01;
    const nudge = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] }[e.key];
    if (nudge && !p.locked) {
      e.preventDefault();
      store.updatePart(p.id, `nudge:${p.id}`, (x) => {
        x.transform.x = Math.max(-1.5, Math.min(1.5, Math.round((x.transform.x + nudge[0]) * 1000) / 1000));
        x.transform.y = Math.max(-1.5, Math.min(1.5, Math.round((x.transform.y + nudge[1]) * 1000) / 1000));
      });
    }
  });
}
