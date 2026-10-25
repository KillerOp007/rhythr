// crates/gui/ui/kit/tests/help.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { HELP, helpMarkdown } from "../help.js";

test("every_section_lists_controls_with_an_action", () => {
  assert.ok(HELP.length >= 6);
  for (const s of HELP) {
    assert.ok(s.title && s.rows.length >= 3, s.title);
    for (const row of s.rows) {
      assert.equal(row.length, 2, `${s.title}: ${row}`);
      assert.ok(row[0].trim() && row[1].trim(), `${s.title}: ${row}`);
      assert.ok(/[.]$/.test(row[1]), `${s.title}: "${row[1]}" ends without a full stop`);
    }
  }
});

test("the_keyboard_section_names_every_shortcut_the_window_binds", () => {
  const text = HELP.find((s) => s.title === "Keyboard").rows.map((r) => r[0]).join(" ");
  for (const key of ["Ctrl+Z", "Ctrl+Y", "Ctrl+S", "Ctrl+Shift+S", "Ctrl+O", "Ctrl+D", "Delete", "Backspace", "Arrows", "Esc", "P", "3", "F1", "Space"]) {
    assert.ok(text.includes(key), `missing ${key}`);
  }
});

test("the_table_covers_groups_effects_and_snapping", () => {
  const titles = HELP.map((s) => s.title);
  assert.ok(titles.includes("Groups and effects"));
  const all = HELP.flatMap((s) => s.rows.map((r) => r[0])).join(" ");
  for (const control of ["Snap", "Group", "Fold", "Ungroup", "Waves", "Round", "Rim", "Mirror", "Repeat", "Leave group"]) {
    assert.ok(all.includes(control), `missing ${control}`);
  }
});

test("the_markdown_mirrors_the_sections", () => {
  const md = helpMarkdown();
  assert.ok(md.includes("**Toolbar**"));
  assert.ok(md.includes("| Control | What it does |"));
  const rows = HELP.reduce((n, s) => n + s.rows.length, 0);
  assert.equal(md.split("\n").filter((l) => l.startsWith("| ") && !l.startsWith("| Control")).length, rows);
});
