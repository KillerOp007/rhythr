// crates/gui/ui/kit/tests/files.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import * as files from "../files.js";
import {
  makeRecent,
  startupDesign,
  isTextEntry,
  ownsArrows,
  bindKeys,
  docPathFor,
  glyphOf,
  RECENT_LIMIT,
  DOC_KEY,
} from "../files.js";
import { createStore } from "../store.js";

function storage(init = {}) {
  const m = new Map(Object.entries(init));
  return { getItem: (k) => (m.has(k) ? m.get(k) : null), setItem: (k, v) => m.set(k, String(v)), removeItem: (k) => m.delete(k) };
}

test("only_a_skin_archive_keeps_its_path_for_save", () => {
  assert.equal(docPathFor("/p/note.rhs", false), "/p/note.rhs");
  assert.equal(docPathFor("C:\\skins\\Note.RHS", false), "C:\\skins\\Note.RHS");
  // A converted old designer file and a plain design text file make Save ask.
  assert.equal(docPathFor("/p/note.rhs", true), null);
  assert.equal(docPathFor("/p/note.rhsd", false), null);
  assert.equal(docPathFor("", false), null);
  assert.equal(docPathFor(null, false), null);
});

test("the_glyph_import_takes_the_first_character_typed", () => {
  assert.equal(glyphOf(" A "), "A");
  assert.equal(glyphOf("AB"), "A");
  // An emoji outside the basic plane is one code point, not two halves.
  assert.equal(glyphOf("\u{1F600}x"), "\u{1F600}");
  assert.equal(glyphOf("   "), "");
  assert.equal(glyphOf(null), "");
});

test("recent_keeps_the_newest_first_without_duplicates", () => {
  const r = makeRecent(storage());
  for (let i = 0; i < RECENT_LIMIT + 3; i++) r.add(`/p/${i}.rhsd`);
  r.add("/p/3.rhsd");
  const l = r.list();
  assert.equal(l.length, RECENT_LIMIT);
  assert.equal(l[0], "/p/3.rhsd");
  assert.equal(new Set(l).size, l.length);
});

test("recent_remove_drops_the_entry_and_keeps_the_others", () => {
  const r = makeRecent(storage());
  r.add("/p/a.rhsd");
  r.add("/p/b.rhsd");
  r.add("/p/c.rhsd");
  const l = r.remove("/p/b.rhsd");
  assert.deepEqual(l, ["/p/c.rhsd", "/p/a.rhsd"]);
  assert.deepEqual(r.list(), ["/p/c.rhsd", "/p/a.rhsd"]);
  r.remove("/p/missing.rhsd");
  assert.deepEqual(r.list(), ["/p/c.rhsd", "/p/a.rhsd"]);
});

test("startup_prefers_the_autosave_and_falls_back_to_the_default", async () => {
  const calls = [];
  const invoke = async (name) => {
    calls.push(name);
    return JSON.stringify({ version: 2, name: "default", colors: ["#ffffff"], parts: [], groups: [] });
  };
  const saved = JSON.stringify({ version: 2, name: "mine", colors: ["#ffffff"], parts: [], groups: [] });
  assert.equal((await startupDesign(storage({ [DOC_KEY]: saved }), invoke)).name, "mine");
  assert.deepEqual(calls, []);
  assert.equal((await startupDesign(storage({ [DOC_KEY]: "{broken" }), invoke)).name, "default");
  assert.deepEqual(calls, ["kit_default_doc"]);
});

test("the_module_exports_no_old_designer_import", () => {
  // The v1 window is gone, and with it the import of its autosave; old
  // files still convert through Open.
  assert.equal(files.importOldDesigner, undefined);
  assert.equal(files.OLD_DOC_KEY, undefined);
});

test("text_entry_and_arrow_ownership_are_told_apart", () => {
  const cases = [
    [{ tagName: "INPUT", type: "range" }, false, true],
    [{ tagName: "INPUT", type: "color" }, false, true],
    [{ tagName: "INPUT", type: "number" }, true, true],
    [{ tagName: "INPUT", type: "text" }, true, true],
    [{ tagName: "INPUT" }, true, true],
    [{ tagName: "INPUT", type: "checkbox" }, false, false],
    [{ tagName: "TEXTAREA" }, true, true],
    [{ tagName: "DIV", isContentEditable: true }, true, true],
    [{ tagName: "SELECT" }, false, true],
    [{ tagName: "BUTTON" }, false, false],
    [null, false, false],
  ];
  for (const [el, text, arrows] of cases) {
    const name = el ? `${el.tagName} ${el.type || ""}` : "null";
    assert.equal(isTextEntry(el), text, `isTextEntry ${name}`);
    assert.equal(ownsArrows(el), arrows, `ownsArrows ${name}`);
  }
});

// bindKeys against a fake window and a fake focused element.
function rig(active) {
  const store = createStore({ version: 2, name: "t", colors: ["#ffffff"], parts: [], groups: [] });
  store.addPart("circle");
  const calls = [];
  let handler;
  globalThis.window = { addEventListener: (_type, h) => (handler = h) };
  const focus = { active };
  bindKeys(
    store,
    {
      save: () => calls.push("save"),
      saveAs: () => calls.push("saveAs"),
      open: () => calls.push("open"),
      togglePreviews: () => calls.push("preview"),
    },
    () => focus.active,
  );
  const press = (key, o = {}) => handler({ key, preventDefault() {}, ...o });
  return { store, calls, press };
}

test("a_slider_that_kept_focus_keeps_the_design_shortcuts_but_keeps_its_arrows", () => {
  const { store, calls, press } = rig({ tagName: "INPUT", type: "range" });
  press("ArrowRight");
  assert.equal(store.part().transform.x, 0);
  press("p");
  assert.deepEqual(calls, ["preview"]);
  press("z", { ctrlKey: true });
  assert.equal(store.state.design.parts.length, 0);
  press("y", { ctrlKey: true });
  assert.equal(store.state.design.parts.length, 1);
  // The store leaves nothing selected after an undo took the part away.
  store.select("p1");
  press("Delete");
  assert.equal(store.state.design.parts.length, 0);
});

test("a_text_field_keeps_its_keys_and_is_blurred_before_save_and_open", () => {
  const calls = [];
  const field = { tagName: "INPUT", type: "number", blur: () => calls.push("blur") };
  const r = rig(field);
  r.press("z", { ctrlKey: true });
  r.press("Delete");
  r.press("p");
  r.press("ArrowLeft");
  assert.equal(r.store.state.design.parts.length, 1);
  assert.equal(r.store.part().transform.x, 0);
  assert.deepEqual(r.calls, []);
  r.press("s", { ctrlKey: true });
  r.press("o", { ctrlKey: true });
  assert.deepEqual(calls, ["blur", "blur"]);
  assert.deepEqual(r.calls, ["save", "open"]);
});

test("ctrl_shift_s_saves_as_even_with_a_path_and_ctrl_s_saves", () => {
  const blurs = [];
  const field = { tagName: "INPUT", type: "text", blur: () => blurs.push("blur") };
  const r = rig(field);
  r.store.state.path = "/p/mine.rhsd";
  r.press("S", { ctrlKey: true, shiftKey: true });
  assert.deepEqual(r.calls, ["saveAs"]);
  r.press("s", { ctrlKey: true });
  assert.deepEqual(r.calls, ["saveAs", "save"]);
  assert.deepEqual(blurs, ["blur", "blur"], "both blur the text field first");
  r.press("S", { ctrlKey: true, shiftKey: true, repeat: true });
  assert.deepEqual(r.calls, ["saveAs", "save"], "a held key does not stack dialogs");
});

test("a_nudge_at_the_limit_commits_nothing_and_ctrl_backspace_keeps_the_part", () => {
  const { store, press } = rig({ tagName: "BODY" });
  store.updatePart("p1", "test", (p) => (p.transform.x = 1.5));
  const depth = store.state.undo.length;
  press("ArrowRight");
  assert.equal(store.part().transform.x, 1.5);
  assert.equal(store.state.undo.length, depth);
  press("ArrowLeft");
  assert.equal(store.part().transform.x, 1.49);
  press("Backspace", { ctrlKey: true });
  assert.equal(store.state.design.parts.length, 1);
  press("Backspace");
  assert.equal(store.state.design.parts.length, 0);
});
