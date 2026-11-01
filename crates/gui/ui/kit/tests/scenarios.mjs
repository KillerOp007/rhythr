// Headless scenario runner for the Note Designer window. It starts the browser
// harness (harness.mjs, real kernel through the kit_eval_json example),
// drives kit.html in headless Chrome through playwright-core and prints one
// PASS or FAIL line per check, grouped by scenario. Not a unit test (no
// `.test.` in the name, so `node --test` leaves it alone) and nothing here
// ships: the build script skips `tests/`.
//
//   cargo build -p rhythia-render --example kit_eval_json
//   node crates/gui/ui/kit/tests/scenarios.mjs [group ...]
//
// Groups: files imports points depth turntable help groups effects snap.
// Environment: PLAYWRIGHT_CORE (directory of playwright-core; default the
// npx cache used so far), CHROME (binary; default /opt/google/chrome/chrome),
// PORT (harness port, default 8793), SHOTS (directory for screenshots,
// default none). Exit code 1 when any check fails.
import { createRequire } from "node:module";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { spawn } from "node:child_process";

const HERE = path.dirname(new URL(import.meta.url).pathname);
const ROOT = path.resolve(HERE, "../../../../..");
const HARNESS = path.join(HERE, "harness.mjs");
const BIN = path.join(ROOT, "target/debug/examples/kit_eval_json");
const PW = process.env.PLAYWRIGHT_CORE || "/home/killer/.npm/_npx/9833c18b2d85bc59/node_modules/playwright-core";
const CHROME = process.env.CHROME || "/opt/google/chrome/chrome";
const PORT = Number(process.env.PORT || 8793);
const SHOTS = process.env.SHOTS || "";
const URL_KIT = `http://127.0.0.1:${PORT}/kit.html`;
const WORK = path.join(os.tmpdir(), "rhythr-kit-scenarios");
const HARNESS_WORK = path.join(os.tmpdir(), "rhythr-kit-harness");
// A stand-in for the game's data folder; the harness child inherits it.
const GAME_DIR = path.join(WORK, "game");
process.env.RHYTHR_GAME_DATA_DIR = GAME_DIR;

if (!fs.existsSync(BIN)) {
  console.error(`missing ${BIN}: run cargo build -p rhythia-render --example kit_eval_json first`);
  process.exit(2);
}
const require = createRequire(import.meta.url);
const { chromium } = require(PW);

// ---- fixtures: design texts and small stored zip archives

const V2_DISC = JSON.stringify({
  version: 2, name: "note", colors: ["#ffffff", "#a2e0ff", "#a2e0ff"],
  parts: [{ id: "p1", name: "Disc", visible: true, locked: false, shape: { kind: "circle" }, transform: { x: 0, y: 0, w: 1, h: 1, rotation: 0 }, mode: "add", depth: { front: 0.1, back: -0.1 }, material: "solid", effects: { waves: null, round: null, rim: null, mirror: "none", repeat: null } }],
  groups: [],
});
const V1_CIRCLE = '{"P":{"shape":"circle","n":4,"round":0,"hollow":false,"thick":0.2,"depth":0.05,"waves":0,"waveDepth":0,"waveform":"sine","twist":0},"free":null,"symmetry":4,"mirror":false,"colors":["#ffffff"],"imported":null}';
const V1_POLYGON = '{"P":{"shape":"ngon","n":6,"round":0.3,"hollow":true,"thick":0.22,"depth":0.05,"waves":5,"waveDepth":0.25,"waveform":"spike","twist":30},"free":null,"symmetry":4,"mirror":false,"colors":["#ff5d6c","#2fd6d0"],"imported":null}';

const CRC_TABLE = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
// A zip with stored entries, enough for the kit's archive reader.
function zipStored(entries) {
  const parts = [];
  const central = [];
  let offset = 0;
  for (const [name, content] of entries) {
    const data = Buffer.isBuffer(content) ? content : Buffer.from(content, "utf8");
    const n = Buffer.from(name, "utf8");
    const crc = crc32(data);
    const lh = Buffer.alloc(30);
    lh.writeUInt32LE(0x04034b50, 0);
    lh.writeUInt16LE(20, 4);
    lh.writeUInt32LE(crc, 14);
    lh.writeUInt32LE(data.length, 18);
    lh.writeUInt32LE(data.length, 22);
    lh.writeUInt16LE(n.length, 26);
    const ch = Buffer.alloc(46);
    ch.writeUInt32LE(0x02014b50, 0);
    ch.writeUInt16LE(20, 4);
    ch.writeUInt16LE(20, 6);
    ch.writeUInt32LE(crc, 16);
    ch.writeUInt32LE(data.length, 20);
    ch.writeUInt32LE(data.length, 24);
    ch.writeUInt16LE(n.length, 28);
    ch.writeUInt32LE(offset, 42);
    central.push(Buffer.concat([ch, n]));
    parts.push(lh, n, data);
    offset += 30 + n.length + data.length;
  }
  const cd = Buffer.concat(central);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(cd.length, 12);
  eocd.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, cd, eocd]);
}
// The entries of a zip the kit wrote (stored or deflated).
function zipEntries(file) {
  const b = fs.readFileSync(file);
  let eocd = b.length - 22;
  while (eocd >= 0 && b.readUInt32LE(eocd) !== 0x06054b50) eocd--;
  if (eocd < 0) return [];
  const count = b.readUInt16LE(eocd + 10);
  let off = b.readUInt32LE(eocd + 16);
  const out = [];
  for (let i = 0; i < count; i++) {
    if (b.readUInt32LE(off) !== 0x02014b50) break;
    const method = b.readUInt16LE(off + 10);
    const csize = b.readUInt32LE(off + 20);
    const nlen = b.readUInt16LE(off + 28);
    const elen = b.readUInt16LE(off + 30);
    const clen = b.readUInt16LE(off + 32);
    const lho = b.readUInt32LE(off + 42);
    const name = b.toString("utf8", off + 46, off + 46 + nlen);
    const start = lho + 30 + b.readUInt16LE(lho + 26) + b.readUInt16LE(lho + 28);
    out.push({
      name,
      data: () => {
        const raw = b.subarray(start, start + csize);
        return method === 8 ? zlib.inflateRawSync(raw) : raw;
      },
    });
    off += 46 + nlen + elen + clen;
  }
  return out;
}
const entryNames = (file) => zipEntries(file).map((e) => e.name);
const designIn = (file) => JSON.parse(zipEntries(file).find((e) => e.name === "rhythr/design.json").data().toString("utf8"));

function writeFixtures() {
  fs.rmSync(WORK, { recursive: true, force: true });
  fs.mkdirSync(WORK, { recursive: true });
  fs.writeFileSync(path.join(WORK, "d.json"), V2_DISC);
  fs.writeFileSync(path.join(WORK, "old.rhsd"), V1_CIRCLE);
  fs.writeFileSync(path.join(WORK, "old.rhs"), zipStored([["config", "{}"], ["rhythr/design.json", V1_POLYGON]]));
  fs.writeFileSync(path.join(WORK, "plain.rhs"), zipStored([["config", "{}"], ["noteSkin/note.mtl", "newmtl Material.001\n"]]));
  fs.writeFileSync(path.join(WORK, "badge.svg"), "<svg/>");
  fs.writeFileSync(path.join(WORK, "cloud.svg"), "<svg/>");
  for (const f of ["s1.rhs", "s2.rhs", "s3.rhs", "r4.rhs"]) fs.rmSync(path.join(HARNESS_WORK, f), { force: true });
  // The game folder: framework.json names the active profile, which has a
  // few of the game's keys so the copy can be checked key by key.
  fs.mkdirSync(GAME_DIR, { recursive: true });
  fs.writeFileSync(path.join(GAME_DIR, "framework.json"), '{"CurrentGameProfile":{"Value":"p1.json"},"DataPath":{"Value":""}}');
  fs.writeFileSync(path.join(GAME_DIR, "p1.json"), '{"ApproachRate":{"Value":3.25},"NoteSkin":{"Value":"x"},"ColorSet":{"Value":"y"},"FpsCap":{"Value":360}}');
}

// ---- harness lifecycle

function startHarness() {
  return new Promise((resolve, reject) => {
    const proc = spawn(process.execPath, [HARNESS], { env: { ...process.env, PORT: String(PORT) }, stdio: ["ignore", "pipe", "pipe"] });
    let text = "";
    const onData = (d) => {
      text += d;
      if (text.includes("kit harness on")) resolve(proc);
    };
    proc.stdout.on("data", onData);
    proc.stderr.on("data", onData);
    proc.on("exit", (code) => reject(new Error(`harness exited with ${code}: ${text}`)));
    setTimeout(() => reject(new Error(`harness did not start: ${text}`)), 10000);
  });
}

// ---- page helpers shared by every group

const near = (a, b, eps) => Math.abs(a - b) <= eps;

function helpers(page, out, group) {
  const check = (name, ok, info = "") => out.push(`${ok ? "PASS" : "FAIL"} ${group}: ${name}${info ? "  ::  " + info : ""}`);
  const settle = async (ms = 450) => {
    await page.waitForTimeout(ms);
    await page.waitForFunction(() => !document.getElementById("kt-status").textContent.includes("…"));
  };
  const S = () => page.evaluate(() => {
    const s = window.kitStore.state;
    const q = (sel) => document.querySelector(sel);
    const text = (sel) => (q(sel) ? q(sel).textContent : "");
    return {
      status: text("#kt-status"),
      err: q("#kt-status").classList.contains("err"),
      path: s.path,
      dirty: s.dirty,
      selected: s.selected,
      selectedGroup: s.selectedGroup,
      pointEdit: s.ui.pointEdit,
      pointSel: s.ui.pointSel,
      snap: s.ui.snap,
      folded: s.ui.folded.slice(),
      turntable: s.ui.turntable,
      previews: s.ui.previews,
      evalError: s.evalError,
      undo: s.undo.length,
      groups: JSON.parse(JSON.stringify(s.design.groups)),
      parts: s.design.parts.map((p) => ({
        id: p.id, name: p.name, kind: p.shape.kind, mode: p.mode, visible: p.visible, locked: p.locked, label: p.shape.label,
        x: p.transform.x, y: p.transform.y, w: p.transform.w, h: p.transform.h, rotation: p.transform.rotation, front: p.depth.front,
        effects: JSON.parse(JSON.stringify(p.effects)),
        n: p.shape.kind === "points" ? p.shape.control.length : null,
        loops: p.shape.kind === "points" ? (p.shape.loops || []).length : null,
        control: p.shape.kind === "points" ? p.shape.control : null,
      })),
      triangles: text("#kt-triangles"),
      recent: [...document.querySelectorAll("#kt-recent option")].map((o) => o.value).filter(Boolean),
      inspector: text("#kt-inspector-body"),
      rows: [...document.querySelectorAll("#kt-parts .kt-part")].map((r) => ({
        group: r.classList.contains("kt-group"), member: r.classList.contains("member"), selected: r.classList.contains("selected"),
        name: r.querySelector(".name").textContent, depth: r.querySelector(".depth") ? r.querySelector(".depth").textContent : null,
        mode: r.querySelector(".badge") ? r.querySelector(".badge").textContent : null, count: r.querySelector(".count") ? r.querySelector(".count").textContent : null,
      })),
      editBtn: text('[data-role="edit-points"]') || null,
      toPointsBtn: Boolean(q('[data-role="to-points"]')),
      groupNameField: Boolean(q('[data-role="group-name"]')),
      figureHidden: q("#kt-turntable-figure").hidden,
      bandHidden: q("#kt-preview-box").hidden,
      stillHidden: q("#kt-still-figure").hidden,
      ttBtnOn: q("#kt-turntable").classList.contains("on"),
      note: text("#kt-turntable-note"),
      noteErr: q("#kt-turntable-note").classList.contains("err"),
      meshCalls: window.__harness.calls.filter((c) => c === "kit_mesh").length,
      calls: window.__harness.calls.slice(),
      helpOpen: q("#kt-help").open,
      helpSections: [...document.querySelectorAll("#kt-help-body h3")].map((h) => h.textContent),
      helpRows: document.querySelectorAll("#kt-help-body tr").length,
      canvas: { w: q("#kt-tt-canvas").width, h: q("#kt-tt-canvas").height },
      snapBtnOn: q("#kt-snap").classList.contains("on"),
      scrollW: document.documentElement.scrollWidth,
    };
  });
  const dlg = (o) => page.evaluate((o) => Object.assign(window.__harness.dialog, o), o);
  const click = (id) => page.click(`#${id}`);
  const keys = (combo) => page.locator("body").press(combo);
  const store = (code) => page.evaluate(`(() => { const s = window.kitStore; ${code} })()`);
  // Page coordinates of a point in a part's unit frame (view at 100 percent,
  // unpanned), and the same in canvas-local pixels for sampling.
  const geo = (id, ux, uy) => page.evaluate(([id, ux, uy]) => {
    const c = document.getElementById("kt-canvas");
    const w = c.clientWidth, h = c.clientHeight;
    const u = Math.min(w, h) / 2.9;
    const p = window.kitStore.part(id);
    const t = p.transform;
    const rot = (t.rotation * Math.PI) / 180;
    const x = ux * t.w, y = uy * t.h;
    const wx = t.x + x * Math.cos(rot) - y * Math.sin(rot);
    const wy = t.y + x * Math.sin(rot) + y * Math.cos(rot);
    const lx = w / 2 + wx * u, ly = h / 2 - wy * u;
    const r = c.getBoundingClientRect();
    return { page: [r.left + c.clientLeft + lx, r.top + c.clientTop + ly], local: [lx, ly], unit: u };
  }, [id, ux, uy]);
  const pixel = (local) => page.evaluate(([x, y]) => {
    const c = document.getElementById("kt-canvas");
    const dpr = window.devicePixelRatio || 1;
    const d = c.getContext("2d").getImageData(Math.round(x * dpr), Math.round(y * dpr), 1, 1).data;
    return [d[0], d[1], d[2], d[3]];
  }, local);
  const drag = async (from, to, opts = {}) => {
    await page.mouse.move(from[0], from[1]);
    await page.mouse.down();
    const steps = 6;
    for (let i = 1; i <= steps; i++) await page.mouse.move(from[0] + ((to[0] - from[0]) * i) / steps, from[1] + ((to[1] - from[1]) * i) / steps, opts);
    await page.mouse.up();
  };
  const evaluated = (id) => page.evaluate((id) => {
    const p = window.kitStore.state.evaluated.parts.find((x) => x.id === id);
    if (!p) return null;
    const flat = p.shapes.flat(3);
    const radii = p.shapes.flat(2).map((v) => Math.hypot(v[0], v[1]));
    return { shapes: p.shapes.length, contours: p.shapes[0].length, extent: Math.max(...flat.map((v) => Math.abs(v))), minRadius: Math.min(...radii) };
  }, id);
  const idxOf = (control, x, y) => control.reduce((best, q, i) => (Math.hypot(q.x - x, q.y - y) < Math.hypot(control[best].x - x, control[best].y - y) ? i : best), 0);
  // HTML drag and drop between list rows: the part row named `fromName`
  // dropped on the group header named `toHeader`.
  const dndRow = (fromName, toHeader) => page.evaluate(([fromName, toHeader]) => {
    const rows = [...document.querySelectorAll("#kt-parts .kt-part")];
    const from = rows.find((r) => !r.classList.contains("kt-group") && r.querySelector(".name").textContent === fromName);
    const to = rows.find((r) => r.classList.contains("kt-group") && r.querySelector(".name").textContent === toHeader);
    if (!from || !to) return false;
    const dt = new DataTransfer();
    from.dispatchEvent(new DragEvent("dragstart", { bubbles: true, cancelable: true, dataTransfer: dt }));
    to.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: dt }));
    to.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
    from.dispatchEvent(new DragEvent("dragend", { bubbles: true, cancelable: true, dataTransfer: dt }));
    return true;
  }, [fromName, toHeader]);
  // Inspector controls by their field label.
  const setRange = (label, value) => page.evaluate(([label, value]) => {
    const field = [...document.querySelectorAll("#kt-inspector-body .kt-field")].find((f) => f.querySelector("label") && f.querySelector("label").textContent === label);
    const r = field && field.querySelector('input[type="range"]');
    if (!r) return false;
    r.value = String(value);
    r.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  }, [label, value]);
  const clickSeg = (label, text) => page.evaluate(([label, text]) => {
    const field = [...document.querySelectorAll("#kt-inspector-body .kt-field")].find((f) => f.querySelector("label") && f.querySelector("label").textContent === label);
    const b = field && [...field.querySelectorAll("button")].find((x) => x.textContent === text);
    if (!b) return false;
    b.click();
    return true;
  }, [label, text]);
  const shot = async (name) => {
    if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `${name}.png`) });
  };
  return { check, settle, S, dlg, click, keys, store, geo, pixel, drag, evaluated, idxOf, dndRow, setRange, clickSeg, shot };
}

// ---- scenario groups

async function files({ page, h }) {
  const { check, settle, S, dlg, click, keys } = h;
  let s = await S();
  const tb = await page.evaluate(() => ({
    exportGone: !document.getElementById("kt-export"),
    importOldGone: !document.getElementById("kt-import-old"),
    save: document.getElementById("kt-save").textContent,
    saveGhost: document.getElementById("kt-save").classList.contains("ghost"),
    hint: document.getElementById("kt-canvas-hint").textContent.includes("one .rhs file"),
  }));
  check("toolbar", tb.exportGone && tb.importOldGone && tb.save === "Save .rhs" && !tb.saveGhost && s.scrollW <= 1100 && tb.hint, JSON.stringify(tb));

  await dlg({ save: `${HARNESS_WORK}/s1.rhs` });
  await keys("Control+s");
  await settle();
  s = await S();
  const e1 = fs.existsSync(`${HARNESS_WORK}/s1.rhs`) ? entryNames(`${HARNESS_WORK}/s1.rhs`) : [];
  check("save writes an archive with the design inside", s.path === `${HARNESS_WORK}/s1.rhs` && !s.dirty && s.status.startsWith("Saved: ") && e1.includes("rhythr/design.json") && e1.includes("noteSkin/s1.obj") && s.recent[0] === `${HARNESS_WORK}/s1.rhs`, `${s.status} | ${e1.join(",")}`);

  await dlg({ save: null });
  const m1 = fs.statSync(`${HARNESS_WORK}/s1.rhs`).mtimeMs;
  await page.waitForTimeout(30);
  await page.click('[data-add="square"]');
  await settle();
  await keys("Control+s");
  await settle();
  s = await S();
  check("save in place", s.status.startsWith("Saved: ") && !s.dirty && designIn(`${HARNESS_WORK}/s1.rhs`).parts.length === 2 && fs.statSync(`${HARNESS_WORK}/s1.rhs`).mtimeMs > m1, s.status);

  await dlg({ ask: true });
  await click("kt-new");
  await settle();
  s = await S();
  check("new", s.parts.length === 1 && s.path === null, JSON.stringify(s.parts.map((p) => p.id)));
  await dlg({ open: `${HARNESS_WORK}/s1.rhs` });
  await click("kt-load");
  await settle();
  s = await S();
  check("open the saved skin", s.parts.length === 2 && s.path === `${HARNESS_WORK}/s1.rhs` && s.status === `Opened: ${HARNESS_WORK}/s1.rhs` && !s.dirty, s.status);

  await dlg({ open: `${WORK}/old.rhs`, save: `${HARNESS_WORK}/s2.rhs` });
  await click("kt-load");
  await settle();
  s = await S();
  check("old designer skin converts without a path", s.parts.length === 1 && s.parts[0].kind === "polygon" && s.path === null && s.status.includes("converted from an older design format") && s.status.includes("new .rhs file"), `${s.status} | ${JSON.stringify(s.parts.map((p) => p.kind))}`);
  await keys("Control+s");
  await settle();
  s = await S();
  check("save after conversion asks", s.path === `${HARNESS_WORK}/s2.rhs` && s.status.startsWith("Saved: ") && fs.existsSync(`${HARNESS_WORK}/s2.rhs`), s.status);

  await dlg({ open: `${WORK}/d.json` });
  await click("kt-load");
  await settle();
  s = await S();
  check("v2 text opens without a path", s.parts.length === 1 && s.parts[0].kind === "circle" && s.path === null && s.status === "Opened: d.json; Save will ask for a .rhs file", s.status);
  await dlg({ open: `${WORK}/old.rhsd` });
  await click("kt-load");
  await settle();
  s = await S();
  check("old rhsd converts", s.parts.length === 1 && s.parts[0].kind === "circle" && s.path === null && s.status.includes("converted from an older design format: old.rhsd"), s.status);

  await dlg({ open: `${WORK}/plain.rhs` });
  await click("kt-load");
  await settle();
  s = await S();
  check("a plain game skin is refused", s.err && s.status.includes("no editable design") && s.parts.length === 1 && s.parts[0].kind === "circle", s.status);

  await dlg({ open: `${HARNESS_WORK}/s1.rhs` });
  await click("kt-load");
  await settle();
  const m2 = fs.statSync(`${HARNESS_WORK}/s1.rhs`).mtimeMs;
  await h.store(`s.commit("visible", (d) => d.parts.forEach((p) => (p.visible = false)))`);
  await settle();
  await keys("Control+s");
  await settle();
  s = await S();
  check("an unbuildable design is not saved", s.err && s.status.startsWith("Not saved, the skin would not load in the game: nothing to show") && fs.statSync(`${HARNESS_WORK}/s1.rhs`).mtimeMs === m2 && s.dirty, s.status);
  await keys("Control+z");
  await settle();
  s = await S();
  check("undo restores visibility", s.parts.every((p) => p.visible) && !s.evalError);

  await dlg({ save: `${HARNESS_WORK}/s3.rhs` });
  await keys("Control+Shift+s");
  await settle();
  s = await S();
  check("save as", s.path === `${HARNESS_WORK}/s3.rhs` && designIn(`${HARNESS_WORK}/s3.rhs`).parts.length === 2 && s.status.startsWith("Saved: ") && s.recent[0] === `${HARNESS_WORK}/s3.rhs`, s.status);

  await page.reload();
  await page.waitForFunction(() => window.kitStore && window.kitStore.state.evaluated);
  await dlg({ open: `${HARNESS_WORK}/s3.rhs`, ask: true });
  await click("kt-load");
  await settle();
  s = await S();
  check("reopen after a reload", s.parts.length === 2 && s.path === `${HARNESS_WORK}/s3.rhs` && !s.dirty && !s.evalError && /\d+ triangles/.test(s.triangles), `${s.status} | ${s.triangles}`);
  check("no kit_doc_save calls", !s.calls.includes("kit_doc_save"));
}

async function imports({ page, h }) {
  const { check, settle, S, dlg, click, evaluated } = h;
  await page.fill("#kt-glyph", "A");
  await page.press("#kt-glyph", "Enter");
  await settle();
  let s = await S();
  let last = s.parts[s.parts.length - 1];
  check("a letter becomes a part", s.parts.length === 2 && last.kind === "contours" && last.name === "A" && s.selected === last.id && s.status === 'Added "A" as a part' && !s.evalError && s.inspector.includes("Imported outline from A"), s.status);
  await page.fill("#kt-glyph", "?");
  await click("kt-add-glyph");
  await settle();
  s = await S();
  check("no font error", s.err && s.status.includes("no installed font") && s.parts.length === 2, s.status);
  await page.fill("#kt-glyph", "   ");
  await click("kt-add-glyph");
  await settle();
  s = await S();
  check("empty field error", s.err && s.status.startsWith("Type a letter") && s.parts.length === 2, s.status);
  await page.fill("#kt-glyph", "\u{1F600}xy");
  await page.press("#kt-glyph", "Enter");
  await settle();
  s = await S();
  check("an emoji is one character", s.parts.length === 3 && s.parts[2].name === "\u{1F600}", s.status);
  await page.fill("#kt-glyph", "B");
  await page.focus("#kt-glyph");
  await page.press("#kt-glyph", "Delete");
  await page.press("#kt-glyph", "Control+z");
  await settle();
  s = await S();
  check("the text field keeps its keys", s.parts.length === 3, JSON.stringify(s.parts.map((p) => p.name)));

  await dlg({ open: `${WORK}/d.json` });
  await click("kt-add-svg");
  await settle();
  s = await S();
  last = s.parts[s.parts.length - 1];
  let geo = await evaluated(last.id);
  check("an svg becomes a part with its hole", s.parts.length === 4 && last.kind === "contours" && last.name === "d.json" && geo.shapes === 1 && geo.contours === 2 && near(geo.extent, 1, 0.01) && !s.evalError, JSON.stringify(geo));
  await h.store(`s.updatePart("${last.id}", "w", (p) => { p.transform.w = 1.4; p.transform.h = 1.4; })`);
  await settle();
  geo = await evaluated(last.id);
  check("a scaled svg is cut at the box", geo.extent <= 1.0001 && geo.extent > 0.99, `extent ${geo.extent}`);

  await dlg({ open: `${WORK}/badge.svg` });
  await click("kt-add-svg");
  await settle();
  s = await S();
  const n = s.parts.length;
  check("a two-colour svg comes as two parts, the figure cut", n === 6 && s.parts[n - 2].name === "badge 1" && s.parts[n - 2].mode === "add" && s.parts[n - 1].name === "badge 2" && s.parts[n - 1].mode === "cut" && s.parts[n - 1].label === "badge, colour #ffffff" && s.selected === s.parts[n - 1].id && s.status.includes("2 parts, one per colour") && s.status.includes("Cut") && s.inspector.includes("colour #ffffff"), s.status);
  await dlg({ open: `${WORK}/cloud.svg` });
  await click("kt-add-svg");
  await settle();
  s = await S();
  check("a one-colour svg is one part", s.parts.length === 7 && s.parts[6].name === "cloud" && s.parts[6].label === "cloud" && s.status === "Added cloud.svg as a part", s.status);
}

async function points({ page, h }) {
  const { check, settle, S, dlg, click, keys, geo, drag, idxOf } = h;
  await page.click('[data-add="square"]');
  await settle();
  await h.store(`s.updatePart("p2", "scale:p2", (p) => { p.transform.w = 0.5; p.transform.h = 0.5; }); s.updatePart("p2", "depth:p2", (p) => (p.depth = { front: 0.3, back: -0.1 }));`);
  await settle();
  await page.click('[data-role="to-points"]');
  await settle();
  let s = await S();
  const sq = s.parts[1];
  check("to points", sq.kind === "points" && sq.n === 4 && sq.loops === 0 && sq.control.every((q) => !q.smooth) && s.pointEdit === "p2" && s.editBtn === "Done" && s.status.startsWith("Editing points") && sq.front === 0.3, JSON.stringify({ n: sq.n, pe: s.pointEdit, btn: s.editBtn }));

  const i0 = idxOf(sq.control, -1, -1);
  let g = await geo("p2", -1, -1);
  await drag(g.page, [g.page[0] + 40, g.page[1]]);
  await settle();
  s = await S();
  const moved = s.parts[1].control[i0];
  const expected = -1 + 40 / (g.unit * 0.5);
  // Snapping may pull the point to the 0.05 grid of the part's frame.
  check("drag a point in the part's frame", near(moved.x, expected, 0.06) && near(moved.y, -1, 0.01) && s.parts[1].x === 0 && s.parts[1].n === 4 && s.pointSel && s.pointSel.index === i0, `x ${moved.x} expected ${expected.toFixed(3)}`);
  await keys("Control+z");
  await settle();
  s = await S();
  check("undo restores the point", near(s.parts[1].control[i0].x, -1, 1e-5) && s.pointEdit === "p2");

  g = await geo("p2", 0, -1);
  await page.mouse.click(g.page[0], g.page[1]);
  await settle();
  s = await S();
  const iN = idxOf(s.parts[1].control, 0, -1);
  check("a click on the outline inserts a corner", s.parts[1].n === 5 && near(s.parts[1].control[iN].x, 0, 0.03) && near(s.parts[1].control[iN].y, -1, 0.03) && s.parts[1].control[iN].smooth === false && s.pointSel && s.pointSel.index === iN, JSON.stringify(s.parts[1].control[iN]));
  g = await geo("p2", s.parts[1].control[iN].x, s.parts[1].control[iN].y);
  await page.mouse.dblclick(g.page[0], g.page[1]);
  await settle();
  s = await S();
  check("double-click toggles smooth", s.parts[1].control[iN].smooth === true && s.parts[1].n === 5);
  await page.mouse.click(g.page[0], g.page[1], { button: "right" });
  await settle();
  s = await S();
  check("right-click removes", s.parts[1].n === 4 && !s.parts[1].control.some((q) => near(q.x, 0, 0.03) && near(q.y, -1, 0.03)));

  g = await geo("p2", -1, -1);
  await page.mouse.click(g.page[0], g.page[1]);
  await settle();
  await keys("Delete");
  await settle();
  s = await S();
  check("delete removes the point, not the part", s.parts.length === 2 && s.parts[1].n === 3 && s.pointEdit === "p2");
  g = await geo("p2", s.parts[1].control[0].x, s.parts[1].control[0].y);
  await page.mouse.click(g.page[0], g.page[1]);
  await keys("Delete");
  await settle();
  s = await S();
  check("three points stay", s.parts.length === 2 && s.parts[1].n === 3);
  const before = s.parts[1].control[0].x;
  await keys("Shift+ArrowRight");
  await settle();
  s = await S();
  check("shift arrow nudges the point", near(s.parts[1].control[0].x, before + 0.1, 1e-6) && s.parts[1].x === 0);
  await keys("Escape");
  await settle();
  s = await S();
  check("escape ends editing", s.pointEdit === null && s.editBtn === "Edit points" && s.selected === "p2");
  await keys("Delete");
  await settle();
  s = await S();
  check("delete then removes the part", s.parts.length === 1);
  await keys("Control+z");
  await settle();

  g = await geo("p1", 0.9, 0);
  await page.mouse.dblclick(g.page[0], g.page[1]);
  await settle();
  s = await S();
  check("double-click converts a disc and opens editing", s.parts[0].kind === "points" && s.parts[0].n >= 12 && s.parts[0].n <= 32 && s.parts[0].control.every((q) => q.smooth) && s.pointEdit === "p1" && s.selected === "p1", `n ${s.parts[0].n}`);
  g = await geo("p1", -1.3, 0.3);
  await page.mouse.click(g.page[0], g.page[1]);
  await settle();
  s = await S();
  check("a click away ends editing", s.pointEdit === null && s.parts[0].x === 0 && s.parts.length === 2);

  await h.store(`s.updatePart("p2", "x", (p) => { p.shape.loops = [[{x:-0.3,y:-0.3,smooth:false},{x:-0.3,y:0.3,smooth:false},{x:0.3,y:0.3,smooth:false}]]; })`);
  await settle();
  await dlg({ save: `${HARNESS_WORK}/r4.rhs`, ask: true });
  await keys("Control+s");
  await settle();
  s = await S();
  const saved = fs.existsSync(`${HARNESS_WORK}/r4.rhs`) ? designIn(`${HARNESS_WORK}/r4.rhs`) : null;
  check("save keeps every loop", s.status.startsWith("Saved: ") && saved && saved.parts[1].shape.kind === "points" && saved.parts[1].shape.loops.length === 1 && saved.parts[0].shape.control.length === s.parts[0].n, s.status.slice(0, 60));
  await click("kt-new");
  await settle();
  await dlg({ open: `${HARNESS_WORK}/r4.rhs` });
  await click("kt-load");
  await settle();
  s = await S();
  check("open brings the points back", s.parts.length === 2 && s.parts[0].kind === "points" && s.parts[1].kind === "points" && s.parts[1].loops === 1 && !s.evalError);

  await h.store(`s.updatePart("p2", "lock", (p) => (p.locked = true)); s.select("p2"); s.setPointEdit("p2");`);
  await settle();
  s = await S();
  g = await geo("p2", s.parts[1].control[0].x, s.parts[1].control[0].y);
  const lockedBefore = s.parts[1].control[0].x;
  await drag(g.page, [g.page[0] + 40, g.page[1]]);
  await settle();
  s = await S();
  check("a locked part keeps its points", s.parts[1].control[0].x === lockedBefore && s.pointEdit === "p2");
  await h.store(`s.updatePart("p2", "lock", (p) => (p.locked = false)); s.setPointEdit(null);`);
  await page.fill("#kt-glyph", "A");
  await page.press("#kt-glyph", "Enter");
  await settle();
  await page.click('[data-role="to-points"]');
  await settle();
  s = await S();
  check("a letter converts too", s.parts[2].kind === "points" && s.parts[2].n === 3 && s.pointEdit === "p3");
  await h.shot("scenarios-points");
}

async function depth({ page, h }) {
  const { check, settle, S, geo, pixel } = h;
  await page.click('[data-add="square"]');
  await settle();
  await h.store(`s.updatePart("p2", "scale:p2", (p) => { p.transform.w = 0.5; p.transform.h = 0.5; }); s.updatePart("p2", "depth:p2", (p) => (p.depth = { front: 0.3, back: -0.1 }));`);
  await settle();
  const s = await S();
  const pc = await pixel((await geo("p1", 0, 0)).local);
  const pr = await pixel((await geo("p1", 0.8, 0)).local);
  check("depth numbers in the list", s.rows.map((r) => r.depth).join(",") === "0.10,0.30", JSON.stringify(s.rows));
  check("the raised part keeps the colour, the lower one draws darker", pc[0] === 255 && pc[3] === 255 && pr[0] > 195 && pr[0] < 220 && pr[0] === pr[1], `centre ${pc} rim ${pr}`);
  const hints = await page.evaluate(() => document.getElementById("kt-canvas-hint").textContent.includes("Double-click a part") && document.querySelector("#kt-parts-pane .hint").textContent.includes("Front depth"));
  check("hints mention points and depth", hints);
}

async function turntable({ page, h }) {
  const { check, settle, S, keys } = h;
  const stats = () => page.evaluate(() => {
    const c = document.getElementById("kt-tt-canvas");
    const gl = c.getContext("webgl2");
    const px = new Uint8Array(c.width * c.height * 4);
    gl.readPixels(0, 0, c.width, c.height, gl.RGBA, gl.UNSIGNED_BYTE, px);
    let lit = 0, sum = 0, left = 0, right = 0;
    for (let i = 0; i < px.length; i += 4) {
      const v = (px[i] + px[i + 1] + px[i + 2]) / 3;
      if (v > 40) {
        lit++;
        sum += v;
        if ((i / 4) % c.width < c.width / 2) left++;
        else right++;
      }
    }
    return { lit, mean: lit ? sum / lit : 0, left, right };
  });
  let s = await S();
  check("off by default", !s.turntable && s.figureHidden && s.bandHidden && s.meshCalls === 0);
  await keys("3");
  await settle(900);
  s = await S();
  const p0 = await stats();
  check("on by key, one mesh call, pixels", s.turntable && !s.figureHidden && !s.bandHidden && s.stillHidden && s.ttBtnOn && s.meshCalls === 1 && !s.noteErr && s.canvas.w > 50 && p0.lit > 500 && p0.mean > 150, JSON.stringify({ calls: s.meshCalls, p0 }));
  const box = await page.evaluate(() => { const r = document.getElementById("kt-tt-canvas").getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; });
  await page.mouse.move(box.x, box.y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(box.x + i * 15, box.y + i * 5);
  await page.mouse.up();
  await page.waitForTimeout(150);
  const p1 = await stats();
  check("a drag turns the note", Math.abs(p1.lit - p0.lit) > 50 || Math.abs(p1.left - p1.right) > Math.abs(p0.left - p0.right) + 20, JSON.stringify({ p0, p1 }));
  await page.click("#kt-tt-front");
  await page.waitForTimeout(100);
  const p2 = await stats();
  check("front snaps back", Math.abs(p2.lit - p0.lit) < 30 && Math.abs(p2.mean - p0.mean) < 3);
  await page.click("#kt-tt-quarter");
  await page.waitForTimeout(100);
  const p3 = await stats();
  check("three-quarter differs", Math.abs(p3.lit - p0.lit) > 30 || Math.abs(p3.mean - p0.mean) > 3);
  await page.click("#kt-tt-shaded");
  await page.waitForTimeout(100);
  const flat = await stats();
  check("flat is brighter than shaded", flat.mean > p3.mean && flat.mean > 240, JSON.stringify({ shaded: p3.mean, flat: flat.mean }));
  await page.fill("#kt-tt-fade", "0.4");
  await page.dispatchEvent("#kt-tt-fade", "input");
  await page.waitForTimeout(100);
  const faded = await stats();
  check("fade dims the note", faded.mean < flat.mean - 40);
  await page.fill("#kt-tt-fade", "1");
  await page.dispatchEvent("#kt-tt-fade", "input");
  await page.click("#kt-tt-shaded");
  await page.click('[data-add="square"]');
  await settle(900);
  s = await S();
  check("edits refetch the mesh", s.meshCalls >= 2 && !s.noteErr, `calls ${s.meshCalls}`);
  await h.store(`s.updatePart("p1", "visible", (p) => (p.visible = false)); s.updatePart("p2", "visible", (p) => (p.visible = false));`);
  await settle(900);
  s = await S();
  const pg = await stats();
  check("a refused design keeps the picture and says why", s.noteErr && s.note.includes("nothing to show") && pg.lit > 500, s.note);
  await keys("Control+z");
  await keys("Control+z");
  await settle(900);
  s = await S();
  check("recovery clears the caption", !s.noteErr && s.note.startsWith("Turned view"));
  await keys("p");
  await settle();
  s = await S();
  check("previews and 3D share the band", s.previews && s.turntable && !s.bandHidden && !s.stillHidden && !s.figureHidden);
  await page.click("#kt-turntable");
  await settle();
  s = await S();
  check("3D off keeps the band for the previews", !s.turntable && s.previews && !s.bandHidden && s.figureHidden && !s.ttBtnOn);
  await keys("p");
  await settle();
  s = await S();
  check("previews off hides the band", s.bandHidden);
  await keys("3");
  await settle();
  await page.reload();
  await page.waitForFunction(() => window.kitStore && window.kitStore.state.evaluated);
  await settle(900);
  s = await S();
  check("the state is remembered", s.turntable && !s.figureHidden && s.meshCalls >= 1);
  await h.shot("scenarios-turntable");
}

async function help({ page, h }) {
  const { check, S, keys } = h;
  await page.click("#kt-help-btn");
  let s = await S();
  check("help opens with every section", s.helpOpen && s.helpSections.join("|") === "Toolbar|Parts list|Canvas (front view)|Point editing|3D (turned view)|Groups and effects|Inspector|Keyboard" && s.helpRows > 50, JSON.stringify(s.helpSections));
  const before = s.parts.length;
  await keys("Delete");
  s = await S();
  check("keys stay with the dialog", s.parts.length === before);
  await keys("Escape");
  s = await S();
  check("escape closes", !s.helpOpen);
  await keys("F1");
  s = await S();
  check("F1 opens", s.helpOpen);
  await page.click("#kt-help-close");
  await keys("?");
  s = await S();
  check("question mark opens", s.helpOpen);
  await keys("Escape");
  await page.click("#kt-glyph");
  await page.keyboard.press("3");
  s = await S();
  check("typing 3 in a field is typing", (await page.inputValue("#kt-glyph")) === "3" && !s.turntable);
}

async function groups({ page, h }) {
  const { check, settle, S, geo, pixel, keys, dndRow, clickSeg } = h;
  await page.click('[data-add="square"]');
  await settle();
  await page.click('[data-add="star"]');
  await settle();
  await h.store(`s.updatePart("p2", "scale:p2", (p) => { p.transform.w = 0.3; p.transform.h = 0.3; }); s.updatePart("p3", "scale:p3", (p) => { p.transform.x = 0.5; p.transform.w = 0.3; p.transform.h = 0.3; });`);
  await settle();
  await page.selectOption('select[data-role="group"]', "new");
  await settle();
  let s = await S();
  const header = s.rows.find((r) => r.group);
  check("a new group from the inspector", s.groups.length === 1 && s.groups[0].name === "Group 1" && s.groups[0].parts.join() === "p3" && s.selectedGroup === "g1" && s.selected === null && header && header.name === "Group 1" && header.count === "1 part" && header.selected && s.groupNameField, JSON.stringify({ groups: s.groups, rows: s.rows }));

  check("drop on the header", await dndRow("Square", "Group 1"));
  await settle();
  s = await S();
  check("the dropped part joins and sits next to its group", s.groups[0].parts.join() === "p3,p2" && s.parts.map((p) => p.id).join() === "p1,p3,p2" && s.rows.map((r) => `${r.group ? "g" : "p"}:${r.name}${r.member ? "*" : ""}`).join() === "p:Disc,p:Star*,p:Square*,g:Group 1" && s.rows.find((r) => r.group).count === "2 parts", JSON.stringify(s.rows));

  await page.fill('[data-role="group-name"]', "Holes");
  await page.press('[data-role="group-name"]', "Enter");
  await settle();
  check("name the group", await clickSeg("Mode", "Cut"));
  await settle();
  s = await S();
  const centre = await pixel((await geo("p1", 0, 0)).local);
  const edge = await pixel((await geo("p1", -0.7, 0)).local);
  check("a cut group carves the disc", s.groups[0].name === "Holes" && s.groups[0].mode === "cut" && s.rows.find((r) => r.group).mode === "Cut" && centre[0] < 200 && edge[0] === 255 && !s.evalError, `centre ${centre} edge ${edge} ${s.evalError}`);
  await h.shot("scenarios-groups");

  const serialBefore = await page.evaluate(() => window.kitStore.serialize());
  const undoBefore = s.undo;
  const evalBefore = s.calls.filter((c) => c === "kit_evaluate").length;
  await page.click("#kt-parts .kt-group .fold");
  await settle();
  s = await S();
  const serialAfter = await page.evaluate(() => window.kitStore.serialize());
  const evalAfter = s.calls.filter((c) => c === "kit_evaluate").length;
  check("folding hides the members and changes nothing", s.folded.join() === "g1" && s.rows.length === 2 && !s.rows.some((r) => r.member) && serialAfter === serialBefore && s.undo === undoBefore && evalAfter === evalBefore, JSON.stringify({ rows: s.rows, evalBefore, evalAfter }));
  await page.click("#kt-parts .kt-group .fold");
  await settle();
  s = await S();
  check("unfold brings them back", s.folded.length === 0 && s.rows.length === 4);
  // A member selected inside a folded group: its header carries the mark.
  await h.store(`s.select("p3")`);
  await page.click("#kt-parts .kt-group .fold");
  await settle();
  const markedFolded = await page.evaluate(() => document.querySelector("#kt-parts .kt-group").classList.contains("has-selected"));
  await page.click("#kt-parts .kt-group .fold");
  await settle();
  const markedOpen = await page.evaluate(() => document.querySelector("#kt-parts .kt-group").classList.contains("has-selected"));
  check("a folded header marks its selected member", markedFolded && !markedOpen, JSON.stringify({ markedFolded, markedOpen }));
  await h.store(`s.selectGroup("g1")`);
  await settle();

  await page.click('[data-role="leave"]');
  await settle();
  s = await S();
  check("leave group", s.groups[0].parts.length === 1 && s.selectedGroup === "g1" && s.inspector.includes("Leave group"), JSON.stringify(s.groups));
  await page.click('[data-role="ungroup"]');
  await settle();
  s = await S();
  check("ungroup keeps the parts", s.groups.length === 0 && s.selectedGroup === null && s.parts.length === 3);
  await keys("Control+z");
  await keys("Control+z");
  await settle();
  s = await S();
  check("undo brings the group back with both parts", s.groups.length === 1 && s.groups[0].parts.length === 2 && s.rows.some((r) => r.group), JSON.stringify(s.groups));
  await page.click("#kt-parts .kt-part:not(.kt-group)");
  await settle();
  s = await S();
  check("a part row selects the part, not the group", s.selected !== null && s.selectedGroup === null);
}

async function effects({ page, h }) {
  const { check, settle, S, evaluated, setRange, clickSeg } = h;
  await page.click('[data-add="square"]');
  await settle();
  let s = await S();
  const undo0 = s.undo;
  await page.click('[data-role="chip-waves"]');
  await settle();
  s = await S();
  const hasCount = await page.evaluate(() => [...document.querySelectorAll("#kt-inspector-body label")].some((l) => l.textContent === "Count"));
  check("waves on with its defaults", JSON.stringify(s.parts[1].effects.waves) === JSON.stringify({ count: 6, depth: 0.15, style: "sine" }) && hasCount && s.undo === undo0 + 1, JSON.stringify(s.parts[1].effects));
  check("the count slider", await setRange("Count", 8));
  await settle();
  s = await S();
  check("count follows the slider in one step", s.parts[1].effects.waves.count === 8 && s.undo === undo0 + 2);
  const depthMax = await page.evaluate(() => {
    const f = [...document.querySelectorAll("#kt-inspector-body .kt-field")].find((x) => x.querySelector("label") && x.querySelector("label").textContent === "Depth");
    const r = f && f.querySelector('input[type="range"]');
    return r ? r.max : null;
  });
  check("the depth slider reaches the kernel's limit", depthMax === "0.95", String(depthMax));
  check("the style pick", await clickSeg("Style", "Spike"));
  await settle();
  check("the second style pick", await clickSeg("Style", "Pulse"));
  await settle();
  s = await S();
  check("style picks are separate undo steps", s.parts[1].effects.waves.style === "pulse" && s.undo === undo0 + 4, JSON.stringify({ undo: s.undo, undo0 }));
  await page.click('[data-role="chip-round"]');
  await settle();
  s = await S();
  check("round on leaves waves alone", s.parts[1].effects.round === 0.1 && s.parts[1].effects.waves.count === 8);
  await page.click('[data-role="chip-round"]');
  await settle();
  s = await S();
  check("round off", s.parts[1].effects.round === null && s.parts[1].effects.waves.count === 8);
  await page.click('[data-role="chip-waves"]');
  await page.click('[data-role="chip-rim"]');
  await settle();
  s = await S();
  let geo = await evaluated("p2");
  check("rim keeps the outline band", s.parts[1].effects.rim === 0.15 && geo.shapes === 1 && geo.contours === 2 && !s.evalError, JSON.stringify(geo));
  await page.click('[data-role="chip-rim"]');
  // Small enough that four copies around the centre do not touch (copies
  // that overlap merge into one shape).
  await h.store(`s.updatePart("p2", "move:p2", (p) => { p.transform.y = 0.5; p.transform.w = 0.2; p.transform.h = 0.2; })`);
  await settle();
  check("mirror x", await clickSeg("Mirror", "X"));
  await settle();
  s = await S();
  geo = await evaluated("p2");
  check("mirror makes a second copy", s.parts[1].effects.mirror === "x" && geo.shapes === 2, JSON.stringify(geo));
  await clickSeg("Mirror", "None");
  await page.fill('[data-role="repeat"]', "4");
  await page.press('[data-role="repeat"]', "Enter");
  await settle();
  s = await S();
  geo = await evaluated("p2");
  check("repeat four turns four copies", s.parts[1].effects.repeat === 4 && geo.shapes === 4, JSON.stringify(geo));
  await page.fill('[data-role="repeat"]', "1");
  await page.press('[data-role="repeat"]', "Enter");
  await settle();
  s = await S();
  check("repeat below two is off", s.parts[1].effects.repeat === null && (await page.inputValue('[data-role="repeat"]')) === "0");
  await h.store(`s.select("p1")`);
  await settle();
  await page.click('[data-role="chip-waves"]');
  await settle();
  s = await S();
  geo = await evaluated("p1");
  check("waves run inward", s.parts[0].effects.waves && geo.extent <= 1.001 && geo.minRadius < 0.9, JSON.stringify(geo));
  await h.shot("scenarios-effects");
}

async function snap({ page, h }) {
  const { check, settle, S, geo, drag, keys, idxOf } = h;
  let s = await S();
  check("snap on by default", s.snap === true && s.snapBtnOn);
  let g = await geo("p1", 0.2, 0.2);
  await drag(g.page, [g.page[0] + 0.512 * g.unit, g.page[1]]);
  await settle();
  s = await S();
  check("a move snaps to the grid", s.parts[0].x === 0.5 && s.parts[0].y === 0, `x ${s.parts[0].x} y ${s.parts[0].y}`);
  await keys("Control+z");
  await settle();
  await page.click("#kt-snap");
  s = await S();
  check("snap off", s.snap === false && !s.snapBtnOn);
  g = await geo("p1", 0.2, 0.2);
  await drag(g.page, [g.page[0] + 0.512 * g.unit, g.page[1]]);
  await settle();
  s = await S();
  check("without snap the move is free", near(s.parts[0].x, 0.512, 0.01) && s.parts[0].x !== 0.5, `x ${s.parts[0].x}`);
  await keys("Control+z");
  await settle();
  await page.click("#kt-snap");
  g = await geo("p1", 1, 0);
  const target = await geo("p1", 0.49, 0);
  await drag(g.page, target.page);
  await settle();
  s = await S();
  check("a size snaps to half the box", s.parts[0].w === 0.5 && s.parts[0].h === 1, `w ${s.parts[0].w}`);
  await keys("Control+z");
  await settle();
  // The rotate handle sits 24 px above the top edge; drag it to 44 degrees.
  const top = await geo("p1", 0, 1);
  const handle = [top.page[0], top.page[1] - 24];
  const a = (134 * Math.PI) / 180;
  const to = await geo("p1", 1.2 * Math.cos(a), 1.2 * Math.sin(a));
  await drag(handle, to.page);
  await settle();
  s = await S();
  check("a turn snaps to 15 degree steps", s.parts[0].rotation === 45, `rotation ${s.parts[0].rotation}`);
  await keys("Control+z");
  await settle();
  await page.click('[data-role="to-points"]');
  await settle();
  s = await S();
  const i = idxOf(s.parts[0].control, 1, 0);
  g = await geo("p1", s.parts[0].control[i].x, s.parts[0].control[i].y);
  const dest = await geo("p1", 0.26, 0);
  await drag(g.page, dest.page);
  await settle();
  s = await S();
  check("a point snaps to the frame's grid", s.parts[0].control[i].x === 0.25 && s.parts[0].control[i].y === 0, JSON.stringify(s.parts[0].control[i]));
  await keys("Escape");
  await page.click("#kt-snap");
  await page.reload();
  await page.waitForFunction(() => window.kitStore && window.kitStore.state.evaluated);
  s = await S();
  check("the snap state is remembered", s.snap === false && !s.snapBtnOn);
  await page.click("#kt-snap");
  s = await S();
  check("snap back on", s.snap === true && s.snapBtnOn);
}

async function send({ page, h }) {
  const { check, settle, S, click } = h;
  const objPath = path.join(GAME_DIR, "skins", "notes", "rhythr-kit.obj");
  const mtlPath = path.join(GAME_DIR, "skins", "notes", "rhythr-kit.mtl");
  const csPath = path.join(GAME_DIR, "skins", "colorsets", "rhythr-kit.txt");
  const profilePath = path.join(GAME_DIR, "rhythr-kit.json");
  await click("kt-send");
  await settle();
  let s = await S();
  const obj = fs.existsSync(objPath) ? fs.readFileSync(objPath, "utf8") : "";
  const profile = fs.existsSync(profilePath) ? JSON.parse(fs.readFileSync(profilePath, "utf8")) : null;
  check(
    "first send creates files and profile",
    s.status.startsWith("Sent to the game: ") && s.status.includes("Pick the profile rhythr-kit") && !s.err && obj.split("\n")[1] === "mtllib rhythr-kit.mtl" && fs.existsSync(mtlPath) && fs.existsSync(csPath) && profile !== null && String(profile.NoteSkin.Value).endsWith("rhythr-kit.obj") && String(profile.ColorSet.Value).endsWith("rhythr-kit.txt") && profile.ApproachRate.Value === 3.25 && profile.FpsCap.Value === 360,
    `${s.status} | ${obj.split("\n")[1]} | ${JSON.stringify(profile)}`,
  );
  const before = fs.readFileSync(profilePath, "utf8");
  await click("kt-send");
  await settle();
  s = await S();
  check("second send leaves the profile alone", s.status.startsWith("Sent to the game. Apply the profile") && fs.readFileSync(profilePath, "utf8") === before, s.status);
  check("the document is untouched", s.path === null && s.dirty === false, JSON.stringify({ path: s.path, dirty: s.dirty }));
  const mtime = fs.statSync(objPath).mtimeMs;
  await h.store(`s.updatePart("p1", "visible", (p) => (p.visible = false))`);
  await settle();
  await click("kt-send");
  await settle();
  s = await S();
  check("a refused design is not sent", s.err && s.status.startsWith("Not sent, the skin would not load in the game") && fs.statSync(objPath).mtimeMs === mtime, s.status);
}

async function presets({ page, h }) {
  const { check, settle, S, dlg, click, keys } = h;
  const panelHidden = () => page.evaluate(() => document.getElementById("kt-presets-panel").hidden);
  const openPanel = async () => {
    await click("kt-presets");
    await page.waitForSelector("#kt-presets-panel .kt-preset");
    await settle(200);
  };
  const pick = async (id) => {
    await page.click(`#kt-presets-panel .kt-preset[data-id="${id}"]`);
    await settle();
  };
  await openPanel();
  const tiles = await page.evaluate(() => [...document.querySelectorAll("#kt-presets-panel .kt-preset span")].map((el) => el.textContent));
  check("the panel lists five tiles in order", tiles.join("|") === "Target|Hex frame|Crosshair|Diamond frame|Notched ring", tiles.join("|"));
  // In a wide window the toolbar is one row and the Presets button sits far
  // right; the panel must not run past the window's edge (the fifth tile
  // was cut off on the owner's screen).
  await keys("Escape");
  await page.setViewportSize({ width: 1700, height: 760 });
  await settle(200);
  await openPanel();
  const box = await page.evaluate(() => {
    const r = document.getElementById("kt-presets-panel").getBoundingClientRect();
    const b = document.getElementById("kt-presets").getBoundingClientRect();
    return { left: Math.round(r.left), right: Math.round(r.right), width: Math.round(r.width), button: Math.round(b.left), innerWidth: window.innerWidth };
  });
  check("the panel stays inside the window", box.button > 900 && box.right <= box.innerWidth && box.left >= 0 && box.width > 300, JSON.stringify(box));
  await keys("Escape");
  await page.setViewportSize({ width: 1100, height: 760 });
  await settle(200);
  await openPanel();
  await page.waitForTimeout(600);
  const drawn = await page.evaluate(() =>
    [...document.querySelectorAll("#kt-presets-panel canvas")].map((c) => {
      const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
      let n = 0;
      for (let i = 3; i < d.length; i += 4) if (d[i] > 0) n++;
      return n;
    }),
  );
  check("thumbnails are drawn", drawn.length === 5 && drawn.every((n) => n > 200), JSON.stringify(drawn));
  await pick("target");
  let s = await S();
  const name = await page.evaluate(() => window.kitStore.state.design.name);
  check(
    "a preset replaces a clean note",
    s.parts.map((p) => p.name).join() === "Outer ring,Inner ring,Dot" && s.path === null && s.dirty === false && name === "Target" && s.status.startsWith("Preset Target: 3 parts") && (await panelHidden()),
    `${s.status} | ${s.parts.map((p) => p.name).join()}`,
  );
  await h.store(`s.updatePart("p1", "move", (p) => (p.transform.x = 0.2))`);
  await settle();
  await dlg({ ask: false });
  await openPanel();
  await pick("hex-frame");
  s = await S();
  check("a dirty note asks first", s.parts.length === 3 && s.dirty === true, JSON.stringify({ parts: s.parts.length, dirty: s.dirty }));
  await dlg({ ask: true });
  await openPanel();
  await pick("hex-frame");
  s = await S();
  check("after Discard the preset loads", s.parts.length === 2 && s.parts[0].name === "Frame" && s.dirty === false, s.parts.map((p) => p.name).join());
  await openPanel();
  const openBefore = !(await panelHidden());
  await keys("Escape");
  const closedByEsc = await panelHidden();
  check("escape closes the panel", openBefore && closedByEsc && (await S()).selected !== null, JSON.stringify({ openBefore, closedByEsc }));
  await openPanel();
  await page.mouse.click(600, 700);
  await settle(100);
  check("an outside click closes the panel", await panelHidden());
  await page.click('[data-role="to-points"]');
  await settle();
  const editing = (await S()).pointEdit;
  await openPanel();
  await pick("crosshair");
  s = await S();
  check("a preset picked during point editing ends it", editing !== null && s.pointEdit === null && s.parts.length === 3 && s.parts[1].name === "Ticks", JSON.stringify({ editing, after: s.pointEdit }));
}

const GROUPS = { files, imports, points, depth, turntable, help, groups, effects, snap, send, presets };

// ---- main

const wanted = process.argv.slice(2).filter((a) => GROUPS[a]);
const run = wanted.length ? wanted : Object.keys(GROUPS);
writeFixtures();
if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
const out = [];
let harness = null;
let browser = null;
try {
  harness = await startHarness();
  browser = await chromium.launch({ executablePath: CHROME, headless: true, args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
  for (const name of run) {
    // A fresh context per group: clean storage, no state carried over.
    const context = await browser.newContext({ viewport: { width: 1100, height: 760 } });
    const page = await context.newPage();
    page.on("pageerror", (e) => out.push(`PAGEERROR ${name}: ${e.message}`));
    try {
      await page.goto(URL_KIT);
      await page.evaluate(() => localStorage.clear());
      await page.reload();
      await page.waitForFunction(() => window.kitStore && window.kitStore.state.evaluated);
      await GROUPS[name]({ page, h: helpers(page, out, name) });
    } catch (e) {
      out.push(`SCRIPT ERROR ${name}: ${e.stack || e}`);
    } finally {
      await context.close();
    }
  }
} finally {
  if (browser) await browser.close();
  if (harness) harness.kill();
}
console.log(out.join("\n"));
const passed = out.filter((l) => l.startsWith("PASS")).length;
const failed = out.length - passed;
console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed ? 1 : 0);
