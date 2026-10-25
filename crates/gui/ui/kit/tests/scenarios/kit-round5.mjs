// Round 5 scenarios for the Note Kit window against the browser harness
// (crates/gui/ui/kit/tests/harness.mjs on 127.0.0.1:8791): the turned view
// and the help table, one headless Chrome page, closed at the end.
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { chromium } = require("/home/killer/.npm/_npx/9833c18b2d85bc59/node_modules/playwright-core");

const SHOT = "/home/killer/.playwright-mcp";
const out = [];
const check = (name, ok, info = "") => out.push(`${ok ? "PASS" : "FAIL"} ${name}${info ? "  ::  " + info : ""}`);

const browser = await chromium.launch({ executablePath: "/opt/google/chrome/chrome", headless: true, args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 760 } });
  page.on("pageerror", (e) => out.push(`PAGEERROR ${e.message}`));
  page.on("console", (m) => { if (m.type() === "error" && !m.text().includes("404")) out.push(`CONSOLE ${m.text()}`); });
  await page.goto("http://127.0.0.1:8791/kit.html");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.waitForFunction(() => window.kitStore && window.kitStore.state.evaluated);
  const settle = async (ms = 500) => {
    await page.waitForTimeout(ms);
    await page.waitForFunction(() => !document.getElementById("kt-status").textContent.includes("…"));
  };
  const S = () => page.evaluate(() => {
    const s = window.kitStore.state;
    const fig = document.getElementById("kt-turntable-figure");
    return {
      status: document.getElementById("kt-status").textContent,
      turntable: s.ui.turntable,
      previews: s.ui.previews,
      figureHidden: fig.hidden,
      bandHidden: document.getElementById("kt-preview-box").hidden,
      stillHidden: document.getElementById("kt-still-figure").hidden,
      btnOn: document.getElementById("kt-turntable").classList.contains("on"),
      note: document.getElementById("kt-turntable-note").textContent,
      noteErr: document.getElementById("kt-turntable-note").classList.contains("err"),
      meshCalls: window.__harness.calls.filter((c) => c === "kit_mesh").length,
      helpOpen: document.getElementById("kt-help").open,
      helpSections: [...document.querySelectorAll("#kt-help-body h3")].map((h) => h.textContent),
      helpRows: document.querySelectorAll("#kt-help-body tr").length,
      canvas: { w: document.getElementById("kt-tt-canvas").width, h: document.getElementById("kt-tt-canvas").height },
    };
  });
  // Pixel statistics of the turned view: how many pixels differ from the
  // background, and the mean brightness of those, by reading the WebGL
  // buffer (preserveDrawingBuffer keeps it).
  const stats = () => page.evaluate(() => {
    const c = document.getElementById("kt-tt-canvas");
    const gl = c.getContext("webgl2");
    const px = new Uint8Array(c.width * c.height * 4);
    gl.readPixels(0, 0, c.width, c.height, gl.RGBA, gl.UNSIGNED_BYTE, px);
    let lit = 0;
    let sum = 0;
    let left = 0;
    let right = 0;
    for (let i = 0; i < px.length; i += 4) {
      const v = (px[i] + px[i + 1] + px[i + 2]) / 3;
      if (v > 40) {
        lit++;
        sum += v;
        const x = (i / 4) % c.width;
        if (x < c.width / 2) left++;
        else right++;
      }
    }
    return { lit, mean: lit ? sum / lit : 0, left, right, w: c.width, h: c.height };
  });
  const keys = (combo) => page.locator("body").press(combo);
  const store = (code) => page.evaluate(`(() => { const s = window.kitStore; ${code} })()`);

  // A. Off by default: no mesh call, no band.
  let s = await S();
  check("A off by default", !s.turntable && s.figureHidden && s.bandHidden && s.meshCalls === 0, JSON.stringify({ t: s.turntable, calls: s.meshCalls }));

  // B. The 3 key turns it on: the band appears without the previews, one
  // mesh call, pixels appear.
  await keys("3");
  await settle(900);
  s = await S();
  let p0 = await stats();
  check("B on by key", s.turntable && !s.figureHidden && !s.bandHidden && s.stillHidden && s.btnOn && s.meshCalls === 1 && !s.noteErr && s.canvas.w > 50, JSON.stringify({ calls: s.meshCalls, hidden: s.figureHidden, still: s.stillHidden, canvas: s.canvas }));
  check("B something is drawn", p0.lit > 500 && p0.mean > 150, JSON.stringify(p0));
  await page.screenshot({ path: `${SHOT}/kit-round5-front.png` });

  // C. Dragging turns the note: the picture changes and gets wider on one side.
  const box = await page.evaluate(() => { const r = document.getElementById("kt-tt-canvas").getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; });
  await page.mouse.move(box.x, box.y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(box.x + i * 15, box.y + i * 5);
  await page.mouse.up();
  await page.waitForTimeout(150);
  const orbit1 = await page.evaluate(() => null);
  let p1 = await stats();
  check("C drag turns the note", Math.abs(p1.lit - p0.lit) > 50 || Math.abs(p1.left - p1.right) > Math.abs(p0.left - p0.right) + 20, JSON.stringify({ before: p0, after: p1 }));
  await page.screenshot({ path: `${SHOT}/kit-round5-turned.png` });

  // D. Front snaps back to the first picture; 3/4 differs from it.
  await page.click("#kt-tt-front");
  await page.waitForTimeout(100);
  const p2 = await stats();
  check("D front snaps back", Math.abs(p2.lit - p0.lit) < 30 && Math.abs(p2.mean - p0.mean) < 3, JSON.stringify({ p0, p2 }));
  await page.click("#kt-tt-quarter");
  await page.waitForTimeout(100);
  const p3 = await stats();
  check("D three-quarter differs", Math.abs(p3.lit - p0.lit) > 30 || Math.abs(p3.mean - p0.mean) > 3, JSON.stringify(p3));

  // E. Shaded off draws flat (the mean goes to the full colour), Fade
  // lowers the brightness.
  await page.click("#kt-tt-shaded");
  await page.waitForTimeout(100);
  const flat = await stats();
  check("E flat is brighter and uniform", flat.mean > p3.mean && flat.mean > 240, JSON.stringify({ shaded: p3.mean, flat: flat.mean }));
  await page.fill("#kt-tt-fade", "0.4");
  await page.dispatchEvent("#kt-tt-fade", "input");
  await page.waitForTimeout(100);
  const faded = await stats();
  check("E fade dims the note", faded.mean < flat.mean - 40, JSON.stringify({ flat: flat.mean, faded: faded.mean }));
  await page.fill("#kt-tt-fade", "1");
  await page.dispatchEvent("#kt-tt-fade", "input");
  await page.click("#kt-tt-shaded");

  // F. A design change fetches a new mesh; a deeper part shows at an angle.
  await page.click('[data-add="square"]');
  await settle();
  await store(`s.updatePart("p2", "scale:p2", (p) => { p.transform.w = 0.5; p.transform.h = 0.5; }); s.updatePart("p2", "depth:p2", (p) => (p.depth = { front: 0.45, back: -0.1 }));`);
  await settle(900);
  s = await S();
  check("F edits refetch the mesh", s.meshCalls >= 3 && !s.noteErr, `calls ${s.meshCalls} ${s.note.slice(0, 40)}`);

  // G. A design the kernel refuses keeps the last note and says why.
  await store(`s.updatePart("p1", "visible", (p) => (p.visible = false)); s.updatePart("p2", "visible", (p) => (p.visible = false));`);
  await settle(900);
  s = await S();
  const pg = await stats();
  check("G refusal keeps the picture", s.noteErr && s.note.includes("nothing to show") && pg.lit > 500, `${s.note} lit ${pg.lit}`);
  await keys("Control+z");
  await keys("Control+z");
  await settle(900);
  s = await S();
  check("G recovery clears the caption", !s.noteErr && s.note.startsWith("Turned view"), s.note.slice(0, 40));

  // H. Previews and 3D share the band: previews on adds figures, 3D off
  // keeps the band while previews stay on.
  await keys("p");
  await settle();
  s = await S();
  check("H both on", s.previews && s.turntable && !s.bandHidden && !s.stillHidden && !s.figureHidden);
  await page.click("#kt-turntable");
  await settle();
  s = await S();
  check("H 3D off keeps the band for the previews", !s.turntable && s.previews && !s.bandHidden && s.figureHidden && !s.btnOn);
  await keys("p");
  await settle();
  s = await S();
  check("H previews off hides the band", s.bandHidden);

  // I. The state is remembered across a reload.
  await keys("3");
  await settle();
  await page.reload();
  await page.waitForFunction(() => window.kitStore && window.kitStore.state.evaluated);
  await settle(900);
  s = await S();
  check("I remembered on", s.turntable && !s.figureHidden && s.meshCalls >= 1, JSON.stringify({ t: s.turntable, calls: s.meshCalls }));

  // J. Help: button, F1 and ? open the table; Esc closes it; keys stay
  // with the dialog while it is open.
  await page.click("#kt-help-btn");
  s = await S();
  check("J help opens with every section", s.helpOpen && s.helpSections.join("|") === "Toolbar|Parts list|Canvas (front view)|Point editing|3D (turned view)|Inspector|Keyboard" && s.helpRows > 40, JSON.stringify(s.helpSections));
  const partsBefore = await page.evaluate(() => window.kitStore.state.design.parts.length);
  await keys("Delete");
  const partsAfter = await page.evaluate(() => window.kitStore.state.design.parts.length);
  check("J keys stay with the dialog", partsBefore === partsAfter, `${partsBefore} -> ${partsAfter}`);
  await keys("Escape");
  s = await S();
  check("J escape closes", !s.helpOpen);
  await keys("F1");
  s = await S();
  check("J F1 opens", s.helpOpen);
  await page.click("#kt-help-close");
  await keys("?");
  s = await S();
  check("J question mark opens", s.helpOpen);
  await page.screenshot({ path: `${SHOT}/kit-round5-help.png` });
  await keys("Escape");

  // K. The 3 key inside the glyph field types, it does not toggle.
  await page.click("#kt-glyph");
  await page.keyboard.press("3");
  s = await S();
  const typed = await page.inputValue("#kt-glyph");
  check("K typing 3 in a field is typing", typed === "3" && s.turntable, `field "${typed}" turntable ${s.turntable}`);
} finally {
  await browser.close();
}
console.log(out.join("\n"));
const fails = out.filter((l) => !l.startsWith("PASS")).length;
console.log(`\n${out.filter((l) => l.startsWith("PASS")).length} passed, ${fails} failed or noted`);
