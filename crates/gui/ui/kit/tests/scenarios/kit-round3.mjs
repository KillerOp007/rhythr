// Round 3 scenarios for the Note Kit window against the browser harness
// (crates/gui/ui/kit/tests/harness.mjs on 127.0.0.1:8791): one headless
// Chrome page, closed at the end. Prints PASS/FAIL lines.
import { createRequire } from "node:module";
import fs from "node:fs";
import { execFileSync } from "node:child_process";
const require = createRequire(import.meta.url);
const { chromium } = require("/home/killer/.npm/_npx/9833c18b2d85bc59/node_modules/playwright-core");

const WORK = "/tmp/rhythr-kit-harness";
const FIX = "/tmp/claude-1000/-home-killer/97bec339-56d0-4336-bb87-f39cbf17ad26/scratchpad/kit-roundtrip";
const SHOT = "/home/killer/.playwright-mcp";
const out = [];
const check = (name, ok, info = "") => out.push(`${ok ? "PASS" : "FAIL"} ${name}${info ? "  ::  " + info : ""}`);
const entries = (file) => execFileSync("python3", ["-c", `import zipfile,sys;print("\\n".join(zipfile.ZipFile(sys.argv[1]).namelist()))`, file]).toString().trim().split("\n");
const designIn = (file) => JSON.parse(execFileSync("python3", ["-c", `import zipfile,sys;sys.stdout.write(zipfile.ZipFile(sys.argv[1]).read("rhythr/design.json").decode())`, file]).toString());
for (const f of ["s1.rhs", "s2.rhs", "s3.rhs"]) fs.rmSync(`${WORK}/${f}`, { force: true });

const browser = await chromium.launch({ executablePath: "/opt/google/chrome/chrome", headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1000, height: 660 } });
  page.on("pageerror", (e) => out.push(`PAGEERROR ${e.message}`));
  page.on("console", (m) => { if (m.type() === "error") out.push(`CONSOLE ${m.text()}`); });
  page.on("response", (r) => { if (r.status() === 404) out.push(`404 ${r.url()}`); });
  await page.goto("http://127.0.0.1:8791/kit.html");
  await page.waitForFunction(() => window.kitStore && window.kitStore.state.evaluated);
  const settle = async (ms = 400) => {
    await page.waitForTimeout(ms);
    await page.waitForFunction(() => !document.getElementById("kt-status").textContent.includes("…"));
  };
  const S = () => page.evaluate(() => {
    const s = window.kitStore.state;
    return {
      status: document.getElementById("kt-status").textContent,
      err: document.getElementById("kt-status").classList.contains("err"),
      path: s.path,
      dirty: s.dirty,
      parts: s.design.parts.map((p) => ({ id: p.id, name: p.name, kind: p.shape.kind, visible: p.visible, w: p.transform.w })),
      selected: s.selected,
      evalError: s.evalError,
      triangles: document.getElementById("kt-triangles").textContent,
      recent: [...document.querySelectorAll("#kt-recent option")].map((o) => o.value).filter(Boolean),
      inspector: document.getElementById("kt-inspector-body").textContent,
    };
  });
  const dlg = (o) => page.evaluate((o) => Object.assign(window.__harness.dialog, o), o);
  const click = (id) => page.click(`#${id}`);
  const keys = (combo) => page.locator("body").press(combo);

  // A. Toolbar: one primary Save, no Export, nothing overflows at the minimum width.
  const tb = await page.evaluate(() => ({
    exportGone: !document.getElementById("kt-export"),
    save: document.getElementById("kt-save").textContent,
    saveGhost: document.getElementById("kt-save").classList.contains("ghost"),
    scrollW: document.documentElement.scrollWidth,
    glyph: !!document.getElementById("kt-glyph"),
    svg: !!document.getElementById("kt-add-svg"),
    hint: document.getElementById("kt-canvas-hint").textContent.includes("one .rhs file"),
  }));
  check("A toolbar", tb.exportGone && tb.save === "Save .rhs" && !tb.saveGhost && tb.scrollW <= 1000 && tb.glyph && tb.svg && tb.hint, JSON.stringify(tb));

  // B. Ctrl+S on a fresh design asks for a .rhs and writes an archive with the design inside.
  await dlg({ save: `${WORK}/s1.rhs` });
  await keys("Control+s");
  await settle();
  let s = await S();
  const e1 = fs.existsSync(`${WORK}/s1.rhs`) ? entries(`${WORK}/s1.rhs`) : [];
  check("B save writes archive", s.path === `${WORK}/s1.rhs` && !s.dirty && s.status.startsWith("Saved: ") && e1.includes("rhythr/design.json") && e1.includes("noteSkin/s1.obj") && s.recent[0] === `${WORK}/s1.rhs`, `${s.status} | ${e1.join(",")}`);

  // C. An edit then Ctrl+S saves in place without a dialog.
  await dlg({ save: null });
  const m1 = fs.statSync(`${WORK}/s1.rhs`).mtimeMs;
  await page.waitForTimeout(30);
  await page.click('[data-add="square"]');
  await settle();
  await keys("Control+s");
  await settle();
  s = await S();
  const d1 = designIn(`${WORK}/s1.rhs`);
  check("C save in place", s.status.startsWith("Saved: ") && !s.dirty && d1.parts.length === 2 && fs.statSync(`${WORK}/s1.rhs`).mtimeMs > m1, `${s.status} | parts in file ${d1.parts.length}`);

  // D. New, then Open brings the saved skin back with its path.
  await dlg({ ask: true });
  await click("kt-new");
  await settle();
  s = await S();
  check("D new", s.parts.length === 1 && s.path === null, JSON.stringify(s.parts));
  await dlg({ open: `${WORK}/s1.rhs` });
  await click("kt-load");
  await settle();
  s = await S();
  check("D open saved skin", s.parts.length === 2 && s.path === `${WORK}/s1.rhs` && s.status === `Opened: ${WORK}/s1.rhs` && !s.dirty, `${s.status} | ${JSON.stringify(s.parts)}`);

  // E. An old designer skin converts, keeps no path, Save asks for a new file.
  await dlg({ open: `${FIX}/old.rhs`, save: `${WORK}/s2.rhs` });
  await click("kt-load");
  await settle();
  s = await S();
  check("E old designer skin converts", s.parts.length === 1 && s.parts[0].kind === "polygon" && s.path === null && s.status.includes("converted from the old designer") && s.status.includes("new .rhs file"), `${s.status} | ${JSON.stringify(s.parts)}`);
  await keys("Control+s");
  await settle();
  s = await S();
  check("E save after conversion asks", s.path === `${WORK}/s2.rhs` && s.status.startsWith("Saved: ") && fs.existsSync(`${WORK}/s2.rhs`), s.status);

  // F. A plain design text file (version 2, as the beta's .rhsd) opens but keeps no path;
  // the old designer's .rhsd converts like its skin does.
  await dlg({ open: `${FIX}/d.json` });
  await click("kt-load");
  await settle();
  s = await S();
  check("F v2 text opens without path", s.parts.length === 1 && s.parts[0].kind === "circle" && s.path === null && s.status === "Opened: d.json; Save will ask for a .rhs file", s.status);
  await dlg({ open: `${FIX}/old.rhsd` });
  await click("kt-load");
  await settle();
  s = await S();
  check("F old rhsd converts", s.parts.length === 1 && s.parts[0].kind === "circle" && s.path === null && s.status.includes("converted from the old designer: old.rhsd"), s.status);

  // G. A skin made by the game (no design inside) is refused and nothing changes.
  await dlg({ open: `${FIX}/plain.rhs` });
  await click("kt-load");
  await settle();
  s = await S();
  check("G plain game skin refused", s.err && s.status.includes("no editable design") && s.parts.length === 1 && s.parts[0].kind === "circle", s.status);

  // H. Letter import via Enter in the field: a selected contours part named after the character.
  await dlg({ open: `${WORK}/s1.rhs` });
  await click("kt-load");
  await settle();
  await page.fill("#kt-glyph", "A");
  await page.press("#kt-glyph", "Enter");
  await settle();
  s = await S();
  const last = s.parts[s.parts.length - 1];
  check("H letter becomes a part", s.parts.length === 3 && last.kind === "contours" && last.name === "A" && s.selected === last.id && s.status === 'Added "A" as a part' && !s.evalError && /\d+ triangles/.test(s.triangles) && s.inspector.includes("Imported outline from A"), `${s.status} | ${s.triangles} | ${JSON.stringify(last)}`);

  // I. Glyph errors: no font, nothing typed.
  await page.fill("#kt-glyph", "?");
  await click("kt-add-glyph");
  await settle();
  s = await S();
  check("I no font error", s.err && s.status.includes("no installed font") && s.parts.length === 3, s.status);
  await page.fill("#kt-glyph", "   ");
  await click("kt-add-glyph");
  await settle();
  s = await S();
  check("I empty field error", s.err && s.status.startsWith("Type a letter") && s.parts.length === 3, s.status);

  // J. An emoji outside the basic plane is one character; the rest of the text is ignored.
  await page.fill("#kt-glyph", "\u{1F600}xy");
  await page.press("#kt-glyph", "Enter");
  await settle();
  s = await S();
  check("J emoji label", s.parts.length === 4 && s.parts[3].name === "\u{1F600}" && s.status === 'Added "\u{1F600}" as a part', s.status);

  // K. Typing in the glyph field never edits the design: Delete and Ctrl+Z stay in the field.
  await page.fill("#kt-glyph", "B");
  await page.focus("#kt-glyph");
  await page.press("#kt-glyph", "Delete");
  await page.press("#kt-glyph", "Control+z");
  await settle();
  s = await S();
  check("K text field keeps the keys", s.parts.length === 4, JSON.stringify(s.parts.map((p) => p.name)));

  // L. SVG import becomes a part with its hole; scaled past the box it is cut at the box.
  await dlg({ open: `${FIX}/d.json` });
  await click("kt-add-svg");
  await settle();
  s = await S();
  const svgPart = s.parts[s.parts.length - 1];
  const geo = await page.evaluate((id) => {
    const ev = window.kitStore.state.evaluated;
    const p = ev.parts.find((x) => x.id === id);
    const ext = (shapes) => Math.max(...shapes.flat(3).map((v) => Math.abs(v)));
    return { shapes: p ? p.shapes.length : 0, contours: p ? p.shapes[0].length : 0, extent: p ? ext(p.shapes) : null };
  }, svgPart.id);
  check("L svg becomes a part with a hole", s.parts.length === 5 && svgPart.kind === "contours" && svgPart.name === "d.json" && geo.shapes === 1 && geo.contours === 2 && Math.abs(geo.extent - 1) < 0.01 && !s.evalError, `${s.status} | ${JSON.stringify(geo)}`);
  await page.evaluate((id) => window.kitStore.updatePart(id, "w:" + id, (p) => { p.transform.w = 1.4; p.transform.h = 1.4; }), svgPart.id);
  await settle();
  const geo2 = await page.evaluate((id) => {
    const p = window.kitStore.state.evaluated.parts.find((x) => x.id === id);
    return p ? Math.max(...p.shapes.flat(3).map((v) => Math.abs(v))) : null;
  }, svgPart.id);
  check("L scaled svg cut at the box", geo2 !== null && geo2 <= 1.0001 && geo2 > 0.99, `extent ${geo2}`);

  // M. Nothing visible: Save is refused with the reason, the file is untouched, autosave keeps the work.
  const m2 = fs.statSync(`${WORK}/s1.rhs`).mtimeMs;
  await page.evaluate(() => window.kitStore.commit("visible", (d) => d.parts.forEach((p) => (p.visible = false))));
  await settle();
  await keys("Control+s");
  await settle();
  s = await S();
  check("M unbuildable design not saved", s.err && s.status.startsWith("Not saved, the skin would not load in the game: nothing to show") && fs.statSync(`${WORK}/s1.rhs`).mtimeMs === m2 && s.dirty, s.status);
  await keys("Control+z");
  await settle();
  s = await S();
  check("M undo restores visibility", s.parts.every((p) => p.visible) && !s.evalError, JSON.stringify(s.parts.map((p) => p.visible)));

  // N. Save as writes a second file and the path follows it.
  await dlg({ save: `${WORK}/s3.rhs` });
  await keys("Control+Shift+s");
  await settle();
  s = await S();
  const d3 = fs.existsSync(`${WORK}/s3.rhs`) ? designIn(`${WORK}/s3.rhs`) : { parts: [] };
  check("N save as", s.path === `${WORK}/s3.rhs` && d3.parts.length === 5 && d3.parts.filter((p) => p.shape.kind === "contours").length === 3 && s.status.startsWith("Saved: ") && s.recent[0] === `${WORK}/s3.rhs`, `${s.status} | ${d3.parts.length} parts`);

  // O. The saved skin opens in a fresh page exactly as saved.
  await page.goto("http://127.0.0.1:8791/kit.html");
  await page.waitForFunction(() => window.kitStore && window.kitStore.state.evaluated);
  await dlg({ open: `${WORK}/s3.rhs`, ask: true });
  await click("kt-load");
  await settle();
  s = await S();
  check("O reopen after reload", s.parts.length === 5 && s.path === `${WORK}/s3.rhs` && !s.dirty && !s.evalError && /\d+ triangles/.test(s.triangles), `${s.status} | ${s.triangles}`);

  await page.screenshot({ path: `${SHOT}/kit-round3.png` });
  const calls = await page.evaluate(() => window.__harness.calls.filter((c) => c === "kit_doc_save").length);
  check("P no kit_doc_save calls", calls === 0, `${calls}`);
} catch (e) {
  out.push(`SCRIPT ERROR ${e.stack || e}`);
} finally {
  await browser.close();
}
console.log(out.join("\n"));
