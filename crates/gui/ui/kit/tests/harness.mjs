// Browser harness for the Note Kit window: serves crates/gui/ui/ as the
// desktop app would and stands in for the Tauri IPC, so kit.html can be
// driven in a plain browser (by hand or with Playwright) against the real
// kernel. kit_evaluate runs the kit_eval_json example; documents are real
// files under a temp directory; previews and the export are canned.
//
//   cargo build -p rhythia-render --example kit_eval_json
//   node crates/gui/ui/kit/tests/harness.mjs      (PORT=8791 by default)
//   open http://127.0.0.1:8791/kit.html
//
// window.__harness.dialog.{open,save,ask} are the canned dialog answers;
// window.__harness.calls lists every command invoked. This directory is
// skipped by the build (build_support skips "tests"), so nothing here ships.
import http from "node:http";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";

const HERE = path.dirname(new URL(import.meta.url).pathname);
const ROOT = path.resolve(HERE, "../../../../..");
const UI = path.join(ROOT, "crates/gui/ui");
const BIN = path.join(ROOT, "target/debug/examples/kit_eval_json");
const WORK = path.join(os.tmpdir(), "rhythr-kit-harness");
const LOG = path.join(WORK, "harness.log");
const PORT = Number(process.env.PORT || 8791);
fs.mkdirSync(WORK, { recursive: true });
fs.writeFileSync(LOG, "");

const PNG_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

const SHIM = `<script>
(() => {
  const call = async (name, args) => {
    const r = await fetch("/invoke/" + name, {
      method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(args || {}),
    });
    const text = await r.text();
    if (!r.ok) throw new Error(text);
    return text ? JSON.parse(text) : null;
  };
  window.__harness = { dialog: { open: null, save: null, ask: true }, calls: [] };
  window.__TAURI__ = {
    core: { invoke: (name, args) => { window.__harness.calls.push(name); return call(name, args); } },
    dialog: {
      open: async () => window.__harness.dialog.open,
      save: async () => window.__harness.dialog.save,
      ask: async () => window.__harness.dialog.ask,
    },
  };
})();
</script>`;

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
  ".ttf": "font/ttf",
};

const log = (line) => fs.appendFileSync(LOG, `${new Date().toISOString().slice(11, 23)} ${line}\n`);

function run(args, stdin) {
  return new Promise((resolve, reject) => {
    const p = spawn(BIN, args, { stdio: ["pipe", "pipe", "pipe"] });
    let out = "";
    let err = "";
    p.stdout.on("data", (d) => (out += d));
    p.stderr.on("data", (d) => (err += d));
    p.on("error", reject);
    p.on("close", (code) => (code === 0 ? resolve(out.trim()) : reject(new Error(err.trim() || `exit ${code}`))));
    p.stdin.end(stdin == null ? undefined : stdin);
  });
}

const body = (req) =>
  new Promise((resolve) => {
    let s = "";
    req.on("data", (d) => (s += d));
    req.on("end", () => resolve(s));
  });

async function invoke(name, args) {
  switch (name) {
    case "kit_evaluate": {
      const out = JSON.parse(await run([], JSON.stringify(args.design)));
      if (out.error) throw new Error(out.error);
      return out;
    }
    case "kit_default_doc":
      return await run(["--default"]);
    case "kit_preview":
      return PNG_1x1;
    case "kit_doc_load":
      return { json: fs.readFileSync(args.path, "utf8"), migrated: false };
    case "kit_doc_save":
      fs.writeFileSync(args.path, args.json);
      return args.path;
    case "kit_export":
      return args.path;
    default:
      throw new Error(`harness: no handler for ${name}`);
  }
}

http
  .createServer(async (req, res) => {
    const url = new URL(req.url, "http://x");
    if (req.method === "POST" && url.pathname.startsWith("/invoke/")) {
      const name = url.pathname.slice("/invoke/".length);
      const t0 = Date.now();
      try {
        const args = JSON.parse((await body(req)) || "{}");
        const out = await invoke(name, args);
        log(`${name} ok ${Date.now() - t0}ms`);
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify(out));
      } catch (e) {
        log(`${name} ERR ${Date.now() - t0}ms ${e.message}`);
        res.writeHead(400, { "content-type": "text/plain" });
        res.end(e.message);
      }
      return;
    }
    let rel = decodeURIComponent(url.pathname);
    if (rel === "/") rel = "/kit.html";
    const file = path.normalize(path.join(UI, rel));
    if (!file.startsWith(UI) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.writeHead(404);
      res.end("not found");
      return;
    }
    let data = fs.readFileSync(file);
    if (rel === "/kit.html") data = Buffer.from(data.toString("utf8").replace("<head>", `<head>${SHIM}`));
    res.writeHead(200, { "content-type": TYPES[path.extname(file)] || "application/octet-stream", "cache-control": "no-store" });
    res.end(data);
  })
  .listen(PORT, "127.0.0.1", () => console.log(`kit harness on http://127.0.0.1:${PORT}/kit.html (documents under ${WORK})`));
