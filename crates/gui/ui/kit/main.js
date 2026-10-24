// Bootstrap: the store, the front view, evaluation on every change.
import { createStore } from "./store.js";
import { createView } from "./view.js";
import { invoke, coalesced } from "./ipc.js";

const $ = (id) => document.getElementById(id);

async function boot() {
  const design = JSON.parse(await invoke("kit_default_doc"));
  const store = createStore(design);
  // The startup evaluate below is explicit, so the subscriber must see the
  // starting design as already sent.
  let lastSerial = store.serialize();
  // The status line is shared with the file actions. An evaluate clears only
  // what an evaluate wrote, so a good result never wipes a "Saved" message.
  let statusFromEval = false;
  let evalText = "";
  const evalStatus = (text) => {
    statusFromEval = true;
    evalText = text;
    store.setStatus(text, true);
  };
  const evaluate = coalesced(
    "kit_evaluate",
    (ev) => {
      store.setEvaluated(ev);
      $("kt-triangles").textContent = ev.triangles == null ? "" : `${ev.triangles} triangles`;
      if (ev.mesh_error) {
        evalStatus(ev.mesh_error);
      } else if (statusFromEval) {
        statusFromEval = false;
        store.setStatus("", false);
      }
    },
    (msg) => {
      store.setEvalError(msg);
      evalStatus(msg);
    },
  );
  const view = createView($("kt-canvas"), store, {
    onEdit: () => {},
    onZoom: (pct) => ($("kt-view-reset").textContent = `${pct}%`),
  });
  $("kt-view-reset").addEventListener("click", view.resetView);
  store.subscribe((s) => {
    view.draw();
    const serial = store.serialize();
    if (serial !== lastSerial) {
      lastSerial = serial;
      evaluate({ design: s.design });
    }
    // Someone else wrote the status line: it is no longer ours to clear.
    if (statusFromEval && s.status.text !== evalText) statusFromEval = false;
    const st = $("kt-status");
    st.textContent = s.status.text;
    st.classList.toggle("err", s.status.error);
  });
  for (const b of document.querySelectorAll("[data-add]")) {
    b.addEventListener("click", () => store.addPart(b.dataset.add));
  }
  $("kt-undo").addEventListener("click", () => store.undo());
  $("kt-redo").addEventListener("click", () => store.redo());
  view.fit();
  evaluate({ design: store.state.design });
  window.kitStore = store;
}

boot().catch((e) => {
  const st = document.getElementById("kt-status");
  st.textContent = String(e);
  st.classList.add("err");
});
