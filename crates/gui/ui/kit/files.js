// Documents: autosave in the webview's storage so closing never loses
// work, a recent list, the old designer's autosave as an import, the file
// rules (one .rhs is both the game's skin and the editable design) and the
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
  const write = (l) => {
    try {
      storage.setItem(RECENT_KEY, JSON.stringify(l));
    } catch {
      // Best effort.
    }
    return l;
  };
  return {
    list: read,
    add(path) {
      return write([path, ...read().filter((p) => p !== path)].slice(0, RECENT_LIMIT));
    },
    // An entry that can no longer be opened (moved, deleted, damaged).
    remove(path) {
      return write(read().filter((p) => p !== path));
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

// The path a loaded file keeps for Save. Only a skin archive (.rhs) is
// written back in place: a converted old designer document must never be
// overwritten, and a plain .rhsd (either editor's design text) would turn
// into an archive under the wrong name, so Save asks for a new file.
export function docPathFor(path, migrated) {
  if (migrated || !/\.rhs$/i.test(String(path || ""))) return null;
  return path;
}

// The one character the glyph import outlines: the first code point of what
// was typed, so the part's label matches what lands on the canvas.
export function glyphOf(text) {
  return Array.from(String(text || "").trim())[0] || "";
}

// A design's name as a file name: the dialog's default and, on the Rust side,
// the skin name come from the file stem, so characters a file system
// refuses are replaced rather than left for the dialog to reject.
export function fileStem(name) {
  const s = String(name || "").replace(/[\\/:*?"<>|\u0000-\u001f]/g, "-").trim();
  return s && s !== "." && s !== ".." ? s : "my-note";
}

const TEXT_TYPES = new Set(["text", "number", "search", "email", "url", "password"]);
const inputType = (el) => String(el.type || "text").toLowerCase();

// Takes the keyboard for editing text: undo, delete and the letter keys
// belong to the field, not to the design.
export function isTextEntry(el) {
  if (!el) return false;
  if (el.isContentEditable || el.tagName === "TEXTAREA") return true;
  return el.tagName === "INPUT" && TEXT_TYPES.has(inputType(el));
}

// Takes the arrow keys as well: a slider, a colour input and a dropdown step
// with them natively, and they stay out of the part nudge while focused.
// Only text entry swallows the other shortcuts, so a slider that kept focus
// after a drag does not disable undo.
export function ownsArrows(el) {
  if (!el) return false;
  if (isTextEntry(el) || el.tagName === "SELECT") return true;
  return el.tagName === "INPUT" && (inputType(el) === "range" || inputType(el) === "color");
}

// activeElement() returns document.activeElement. Ctrl+S, Ctrl+Shift+S and
// Ctrl+O work from a focused text field, after blur() has committed what
// was typed (the field's change event fires synchronously); every other
// shortcut leaves a text field to its own editing.
export function bindKeys(store, actions, activeElement) {
  window.addEventListener("keydown", (e) => {
    const mod = e.ctrlKey || e.metaKey;
    // Autofill dispatches keydown events without a key.
    const key = String(e.key || "").toLowerCase();
    const active = activeElement();
    // A held key must not stack dialogs, repeat a removal or flip the previews.
    // Save as comes first: the plain branch below would take Ctrl+Shift+S too.
    if (mod && e.shiftKey && key === "s") {
      e.preventDefault();
      if (isTextEntry(active)) active.blur();
      if (!e.repeat) actions.saveAs();
      return;
    }
    if (mod && (key === "s" || key === "o")) {
      e.preventDefault();
      if (isTextEntry(active)) active.blur();
      if (!e.repeat) (key === "s" ? actions.save : actions.open)();
      return;
    }
    if (isTextEntry(active)) return;
    if (mod && key === "z" && !e.shiftKey) { e.preventDefault(); store.undo(); return; }
    if (mod && (key === "y" || (key === "z" && e.shiftKey))) { e.preventDefault(); store.redo(); return; }
    if (!mod && !e.altKey && key === "p") { if (!e.repeat) actions.togglePreviews(); return; }
    if (e.key === "Escape") { store.select(null); return; }
    const p = store.part();
    if (!p) return;
    if (e.key === "Delete" || e.key === "Backspace") {
      // Ctrl+Backspace is a word delete, not a part removal.
      if (mod) return;
      e.preventDefault();
      if (!e.repeat) store.removePart(p.id);
      return;
    }
    if (mod && key === "d") { e.preventDefault(); if (!e.repeat) store.duplicatePart(p.id); return; }
    if (mod || e.altKey || ownsArrows(active)) return;
    const step = e.shiftKey ? 0.1 : 0.01;
    const nudge = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] }[e.key];
    if (nudge && !p.locked) {
      e.preventDefault();
      const clamp = (v) => Math.max(-1.5, Math.min(1.5, Math.round(v * 1000) / 1000));
      const x = clamp(p.transform.x + nudge[0]);
      const y = clamp(p.transform.y + nudge[1]);
      // Already at the limit: no change, so no undo step and no evaluate.
      if (x === p.transform.x && y === p.transform.y) return;
      store.updatePart(p.id, `nudge:${p.id}`, (q) => {
        q.transform.x = x;
        q.transform.y = y;
      });
    }
  });
}
