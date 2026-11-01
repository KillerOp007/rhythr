// crates/gui/ui/kit/presets.js
// The Presets popover: five finished notes from the kernel, each a tile
// with a thumbnail drawn from its own evaluation. A pick hands the preset
// to the caller, which replaces the document (asking about unsaved work
// first, as New does).

import { drawThumbnail } from "./thumb.js";

// What the panel shows per preset; pure, for the test.
export function tileModel(presets) {
  return presets.map((p) => ({ id: p.id, name: p.name, parts: p.design.parts.length }));
}

export function mountPresets({ button, panel }, { invoke, onPick }) {
  let loaded = null;
  let open = false;
  const close = () => {
    if (!open) return;
    open = false;
    panel.hidden = true;
    button.classList.remove("on");
  };
  // The tiles are built once; the kernel evaluates each preset once for its
  // thumbnail. A failed evaluation leaves the tile with an empty picture
  // rather than a broken panel.
  async function build() {
    const presets = await invoke("kit_presets");
    panel.replaceChildren();
    for (const preset of presets) {
      const tile = document.createElement("button");
      tile.type = "button";
      tile.className = "kt-preset";
      tile.dataset.id = preset.id;
      tile.title = `${preset.name}: ${preset.design.parts.length} parts, all editable`;
      const canvas = document.createElement("canvas");
      const name = document.createElement("span");
      name.textContent = preset.name;
      tile.append(canvas, name);
      tile.addEventListener("click", () => {
        close();
        onPick(preset);
      });
      panel.append(tile);
      invoke("kit_evaluate", { design: preset.design })
        .then((ev) => drawThumbnail(canvas, ev, (preset.design.colors && preset.design.colors[0]) || "#ffffff"))
        .catch(() => {});
    }
    return presets;
  }
  const show = async () => {
    if (!loaded) loaded = build();
    await loaded;
    const r = button.getBoundingClientRect();
    panel.style.top = `${Math.round(r.bottom + 6)}px`;
    panel.style.left = `${Math.round(r.left)}px`;
    open = true;
    panel.hidden = false;
    button.classList.add("on");
  };
  button.addEventListener("click", () => (open ? close() : show()));
  document.addEventListener("pointerdown", (e) => {
    if (open && !panel.contains(e.target) && !button.contains(e.target)) close();
  });
  // Esc closes the panel and nothing else: the window's own Esc (clear the
  // selection, finish point editing) must not fire for the same press.
  window.addEventListener(
    "keydown",
    (e) => {
      if (open && e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    },
    true,
  );
  return { open: show, close, isOpen: () => open };
}
