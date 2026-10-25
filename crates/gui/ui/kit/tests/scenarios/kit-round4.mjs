// Round 4 scenarios for the Note Kit window against the browser harness
// (crates/gui/ui/kit/tests/harness.mjs on 127.0.0.1:8791): one headless
// Chrome page, closed at the end. Prints PASS/FAIL lines.
import { createRequire } from "node:module";
import fs from "node:fs";
import { execFileSync } from "node:child_process";
const require = createRequire(import.meta.url);
const { chromium } = require("/home/killer/.npm/_npx/9833c18b2d85bc59/node_modules/playwright-core");

const WORK = "/tmp/rhythr-kit-harness";
const FIX = "/tmp/claude-1000/-home-killer/97bec339-56d0-4336-bb87-f39cbf17ad26/scratchpad/kit-round4-fixtures";
const SHOT = "/home/killer/.playwright-mcp";
fs.mkdirSync(FIX, { recursive: true });
fs.writeFileSync(`${FIX}/badge.svg`, "<svg/>");
fs.writeFileSync(`${FIX}/cloud.svg`, "<svg/>");
fs.rmSync(`${WORK}/r4.rhs`, { force: true });
const out = [];
const check = (name, ok, info = "") => out.push(`${ok ? "PASS" : "FAIL"} ${name}${info ? "  ::  " + info : ""}`);
const designIn = (file) => JSON.parse(execFileSync("python3", ["-c", `import zipfile,sys;sys.stdout.write(zipfile.ZipFile(sys.argv[1]).read("rhythr/design.json").decode())`, file]).toString());
const near = (a, b, eps) => Math.abs(a - b) <= eps;

const browser = await chromium.launch({ executablePath: "/opt/google/chrome/chrome", headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 720 } });
  page.on("pageerror", (e) => out.push(`PAGEERROR ${e.message}`));
  page.on("console", (m) => { if (m.type() === "error" && !m.text().includes("favicon")) out.push(`CONSOLE ${m.text()}`); });
  await page.goto("http://127.0.0.1:8791/kit.html");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.waitForFunction(() => window.kitStore && window.kitStore.state.evaluated);
  const settle = async (ms = 450) => {
    await page.waitForTimeout(ms);
    await page.waitForFunction(() => !document.getElementById("kt-status").textContent.includes("…"));
  };
  const S = () => page.evaluate(() => {
    const s = window.kitStore.state;
    return {
      status: document.getElementById("kt-status").textContent,
      err: document.getElementById("kt-status").classList.contains("err"),
      path: s.path,
      parts: s.design.parts.map((p) => ({ id: p.id, name: p.name, kind: p.shape.kind, mode: p.mode, label: p.shape.label,
        n: p.shape.kind === "points" ? p.shape.control.length : null, loops: p.shape.kind === "points" ? (p.shape.loops || []).length : null,
        control: p.shape.kind === "points" ? p.shape.control : null, x: p.transform.x, front: p.depth.front, locked: p.locked })),
      selected: s.selected,
      pointEdit: s.ui.pointEdit,
      pointSel: s.ui.pointSel,
      evalError: s.evalError,
      undo: s.undo.length,
      rows: [...document.querySelectorAll("#kt-parts .kt-part")].map((r) => ({ name: r.querySelector(".name").textContent, depth: r.querySelector(".depth").textContent, mode: r.querySelector(".badge").textContent })),
      inspector: document.getElementById("kt-inspector-body").textContent,
      editBtn: document.querySelector('[data-role="edit-points"]')?.textContent || null,
      toPointsBtn: !!document.querySelector('[data-role="to-points"]'),
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

  // A. A two-colour SVG comes in as two parts in paint order, the figure cut.
  await dlg({ open: `${FIX}/badge.svg` });
  await click("kt-add-svg");
  await settle();
  let s = await S();
  check("A two-colour svg as two parts", s.parts.length === 3 && s.parts[1].name === "badge 1" && s.parts[1].mode === "add" && s.parts[2].name === "badge 2" && s.parts[2].mode === "cut" && s.parts[2].label === "badge, colour #ffffff" && s.selected === "p3" && s.status.includes("2 parts, one per colour") && s.status.includes("Cut") && s.rows[2].mode === "Cut", `${s.status} | ${JSON.stringify(s.rows)}`);
  check("A inspector names the colour", s.inspector.includes("colour #ffffff") && s.toPointsBtn, s.inspector.slice(0, 160));

  // B. A one-colour SVG is one part with the plain name.
  await dlg({ open: `${FIX}/cloud.svg` });
  await click("kt-add-svg");
  await settle();
  s = await S();
  check("B one-colour svg", s.parts.length === 4 && s.parts[3].name === "cloud" && s.parts[3].label === "cloud" && s.status === "Added cloud.svg as a part", s.status);

  // C. Depth shows in the list and on the canvas: a raised square keeps the
  // colour, the disc below it draws darker.
  await dlg({ ask: true });
  await click("kt-new");
  await settle();
  await page.click('[data-add="square"]');
  await settle();
  await store(`s.updatePart("p2", "scale:p2", (p) => { p.transform.w = 0.5; p.transform.h = 0.5; }); s.updatePart("p2", "depth:p2", (p) => (p.depth = { front: 0.3, back: -0.1 }));`);
  await settle();
  s = await S();
  const centre = (await geo("p1", 0, 0)).local;
  const rim = (await geo("p1", 0.8, 0)).local;
  const pc = await pixel(centre);
  const pr = await pixel(rim);
  check("C depth badges", s.rows.map((r) => r.depth).join(",") === "0.10,0.30", JSON.stringify(s.rows));
  check("C raised part keeps the colour, lower draws darker", pc[0] === 255 && pc[3] === 255 && pr[0] > 195 && pr[0] < 220 && pr[0] === pr[1], `centre ${pc} rim ${pr}`);
  const hintOk = await page.evaluate(() => document.getElementById("kt-canvas-hint").textContent.includes("Double-click a part") && document.querySelector("#kt-parts-pane .hint").textContent.includes("Front depth"));
  check("C hints mention points and depth", hintOk);

  // D. To points from the inspector: the square becomes four corners and
  // point editing starts.
  await store(`s.select("p2")`);
  await page.click('[data-role="to-points"]');
  await settle();
  s = await S();
  const sq = s.parts[1];
  check("D to points", sq.kind === "points" && sq.n === 4 && sq.loops === 0 && sq.control.every((q) => !q.smooth) && s.pointEdit === "p2" && s.editBtn === "Done" && s.status.startsWith("Editing points") && s.parts[1].front === 0.3, JSON.stringify({ n: sq.n, pe: s.pointEdit, btn: s.editBtn, status: s.status.slice(0, 40) }));

  // The loop's point order is the kernel's; points are found by position.
  const idxOf = (control, x, y) => control.reduce((best, q, i) => (Math.hypot(q.x - x, q.y - y) < Math.hypot(control[best].x - x, control[best].y - y) ? i : best), 0);

  // E. Dragging a point moves it in the part's frame, the part itself stays.
  let i0 = idxOf(sq.control, -1, -1);
  let g = await geo("p2", -1, -1);
  await drag(g.page, [g.page[0] + 40, g.page[1]]);
  await settle();
  s = await S();
  const moved = s.parts[1].control[i0];
  const expected = -1 + 40 / (g.unit * 0.5);
  check("E drag a point", near(moved.x, expected, 0.03) && near(moved.y, -1, 0.01) && s.parts[1].x === 0 && s.parts[1].n === 4 && s.pointSel && s.pointSel.index === i0, `x ${moved.x} expected ${expected.toFixed(3)} part x ${s.parts[1].x}`);
  await keys("Control+z");
  await settle();
  s = await S();
  check("E undo restores the point", near(s.parts[1].control[i0].x, -1, 1e-5) && s.pointEdit === "p2", JSON.stringify(s.parts[1].control[i0]));

  // F. A click on the outline inserts a point there (a corner between corners).
  g = await geo("p2", 0, -1);
  await page.mouse.click(g.page[0], g.page[1]);
  await settle();
  s = await S();
  let iN = idxOf(s.parts[1].control, 0, -1);
  check("F insert on the outline", s.parts[1].n === 5 && near(s.parts[1].control[iN].x, 0, 0.03) && near(s.parts[1].control[iN].y, -1, 0.03) && s.parts[1].control[iN].smooth === false && s.pointSel && s.pointSel.index === iN, JSON.stringify(s.parts[1].control[iN]));

  // G. Double-click switches smooth and corner; H. right-click removes.
  g = await geo("p2", s.parts[1].control[iN].x, s.parts[1].control[iN].y);
  await page.mouse.dblclick(g.page[0], g.page[1]);
  await settle();
  s = await S();
  check("G double-click toggles smooth", s.parts[1].control[iN].smooth === true && s.parts[1].n === 5, JSON.stringify(s.parts[1].control[iN]));
  await page.mouse.click(g.page[0], g.page[1], { button: "right" });
  await settle();
  s = await S();
  check("H right-click removes", s.parts[1].n === 4 && idxOf(s.parts[1].control, 0, -1) !== -1 && !s.parts[1].control.some((q) => near(q.x, 0, 0.03) && near(q.y, -1, 0.03)), `n ${s.parts[1].n}`);

  // I. Delete removes the selected point, never the part; a loop keeps three.
  g = await geo("p2", -1, -1);
  await page.mouse.click(g.page[0], g.page[1]);
  await settle();
  await keys("Delete");
  await settle();
  s = await S();
  check("I delete removes the point", s.parts.length === 2 && s.parts[1].n === 3 && s.pointEdit === "p2" && !s.parts[1].control.some((q) => near(q.x, -1, 0.03) && near(q.y, -1, 0.03)), `parts ${s.parts.length} n ${s.parts[1].n}`);
  g = await geo("p2", s.parts[1].control[0].x, s.parts[1].control[0].y);
  await page.mouse.click(g.page[0], g.page[1]);
  await keys("Delete");
  await settle();
  s = await S();
  check("I three points stay", s.parts.length === 2 && s.parts[1].n === 3, `n ${s.parts[1].n}`);

  // J. Shift+arrow nudges the selected point by a tenth.
  const before = s.parts[1].control[0].x;
  await keys("Shift+ArrowRight");
  await settle();
  s = await S();
  check("J shift arrow nudges a point", near(s.parts[1].control[0].x, before + 0.1, 1e-6) && s.parts[1].x === 0, `${before} -> ${s.parts[1].control[0].x}`);

  // K. Escape ends editing; Delete then removes the part as before.
  await keys("Escape");
  await settle();
  s = await S();
  check("K escape ends editing", s.pointEdit === null && s.editBtn === "Edit points" && s.selected === "p2", `${s.pointEdit} ${s.editBtn}`);
  await keys("Delete");
  await settle();
  s = await S();
  check("K delete removes the part", s.parts.length === 1, `parts ${s.parts.length}`);
  await keys("Control+z");
  await settle();
  s = await S();
  check("K undo brings it back", s.parts.length === 2 && s.parts[1].kind === "points");

  // L. Double-click a disc converts it and opens editing on it.
  g = await geo("p1", 0.9, 0);
  await page.mouse.dblclick(g.page[0], g.page[1]);
  await settle();
  s = await S();
  check("L double-click converts", s.parts[0].kind === "points" && s.parts[0].n >= 12 && s.parts[0].n <= 32 && s.parts[0].control.every((q) => q.smooth) && s.pointEdit === "p1" && s.selected === "p1", `n ${s.parts[0].n} pe ${s.pointEdit}`);

  // M. A click away from the outline ends editing and moves nothing.
  g = await geo("p1", -1.3, 0.3);
  await page.mouse.click(g.page[0], g.page[1]);
  await settle();
  s = await S();
  check("M click away ends editing", s.pointEdit === null && s.parts[0].x === 0 && s.parts.length === 2, `pe ${s.pointEdit}`);

  // N. Save and open keep every loop.
  await store(`s.updatePart("p2", "x", (p) => { p.shape.loops = [[{x:-0.3,y:-0.3,smooth:false},{x:-0.3,y:0.3,smooth:false},{x:0.3,y:0.3,smooth:false}]]; })`);
  await settle();
  await dlg({ save: `${WORK}/r4.rhs`, ask: true });
  await keys("Control+s");
  await settle();
  s = await S();
  const saved = fs.existsSync(`${WORK}/r4.rhs`) ? designIn(`${WORK}/r4.rhs`) : null;
  check("N save keeps loops", s.status.startsWith("Saved: ") && saved && saved.parts[1].shape.kind === "points" && saved.parts[1].shape.loops.length === 1 && saved.parts[0].shape.control.length === s.parts[0].n, s.status.slice(0, 60));
  await click("kt-new");
  await settle();
  await dlg({ open: `${WORK}/r4.rhs` });
  await click("kt-load");
  await settle();
  s = await S();
  check("N open brings the points back", s.parts.length === 2 && s.parts[0].kind === "points" && s.parts[1].kind === "points" && s.parts[1].loops === 1 && !s.evalError, `${JSON.stringify(s.parts.map((p) => [p.kind, p.n, p.loops]))} ${s.evalError}`);

  // O. A locked part shows its points but takes no drag.
  await store(`s.updatePart("p2", "lock", (p) => (p.locked = true)); s.select("p2"); s.setPointEdit("p2");`);
  await settle();
  s = await S();
  g = await geo("p2", s.parts[1].control[0].x, s.parts[1].control[0].y);
  const lockedBefore = s.parts[1].control[0].x;
  await drag(g.page, [g.page[0] + 40, g.page[1]]);
  await settle();
  s = await S();
  check("O locked part keeps its points", s.parts[1].control[0].x === lockedBefore && s.pointEdit === "p2", `${lockedBefore} -> ${s.parts[1].control[0].x}`);
  await store(`s.updatePart("p2", "lock", (p) => (p.locked = false)); s.setPointEdit(null);`);

  // P. The letter import still lands as a part; To points converts it too.
  await page.fill("#kt-glyph", "A");
  await page.press("#kt-glyph", "Enter");
  await settle();
  s = await S();
  check("P letter import", s.parts.length === 3 && s.parts[2].kind === "contours" && s.parts[2].name === "A", JSON.stringify(s.parts[2]));
  await page.click('[data-role="to-points"]');
  await settle();
  s = await S();
  check("P letter to points", s.parts[2].kind === "points" && s.parts[2].n === 3 && s.pointEdit === "p3", `n ${s.parts[2].n}`);
  await page.screenshot({ path: `${SHOT}/kit-round4.png` });

  // Q. The design keeps evaluating without errors throughout.
  check("Q no evaluate error at the end", !s.evalError && !s.err, `${s.evalError} ${s.status}`);
} finally {
  await browser.close();
}
console.log(out.join("\n"));
const fails = out.filter((l) => !l.startsWith("PASS")).length;
console.log(`\n${out.filter((l) => l.startsWith("PASS")).length} passed, ${fails} failed or noted`);
