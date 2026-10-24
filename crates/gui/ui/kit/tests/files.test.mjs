// crates/gui/ui/kit/tests/files.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { makeRecent, startupDesign, importOldDesigner, RECENT_LIMIT, DOC_KEY, OLD_DOC_KEY } from "../files.js";

function storage(init = {}) {
  const m = new Map(Object.entries(init));
  return { getItem: (k) => (m.has(k) ? m.get(k) : null), setItem: (k, v) => m.set(k, String(v)), removeItem: (k) => m.delete(k) };
}

test("recent_keeps_the_newest_first_without_duplicates", () => {
  const r = makeRecent(storage());
  for (let i = 0; i < RECENT_LIMIT + 3; i++) r.add(`/p/${i}.rhsd`);
  r.add("/p/3.rhsd");
  const l = r.list();
  assert.equal(l.length, RECENT_LIMIT);
  assert.equal(l[0], "/p/3.rhsd");
  assert.equal(new Set(l).size, l.length);
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

test("the_old_designer_autosave_migrates_through_the_backend", async () => {
  const old = '{"P":{},"free":null,"symmetry":4,"mirror":false,"colors":["#ffffff"]}';
  const invoke = async (name, args) => {
    assert.equal(name, "kit_migrate_v1");
    assert.equal(args.json, old);
    return JSON.stringify({ version: 2, name: "migrated", colors: ["#ffffff"], parts: [], groups: [] });
  };
  assert.equal((await importOldDesigner(storage({ [OLD_DOC_KEY]: old }), invoke)).name, "migrated");
  await assert.rejects(importOldDesigner(storage(), invoke), /nothing saved/);
});
