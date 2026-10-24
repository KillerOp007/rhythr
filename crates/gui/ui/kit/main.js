// Bootstrap: the store, the front view, evaluation on every change.
import { createStore } from "./store.js";
import { createView } from "./view.js";
import { invoke, coalesced } from "./ipc.js";

const $ = (id) => document.getElementById(id);

async function boot() {
  const design = JSON.parse(await invoke("kit_default_doc"));
  const store = createStore(design);
  const evaluate = coalesced(
    "kit_evaluate",
    (ev) => {
      store.setEvaluated(ev);
      $("kt-triangles").textContent = ev.triangles == null ? "" : `${ev.triangles} triangles`;
      store.setStatus(ev.mesh_error || "", !!ev.mesh_error);
    },
    (msg) => {
      store.setEvalError(msg);
      store.setStatus(msg, true);
    },
  );
  const view = createView($("kt-canvas"), store, {
    onEdit: () => {},
    onZoom: (pct) => ($("kt-view-reset").textContent = `${pct}%`),
  });
  $("kt-view-reset").addEventListener("click", view.resetView);
  let lastSerial = "";
  store.subscribe((s) => {
    view.draw();
    const serial = store.serialize();
    if (serial !== lastSerial) {
      lastSerial = serial;
      evaluate({ design: s.design });
    }
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
