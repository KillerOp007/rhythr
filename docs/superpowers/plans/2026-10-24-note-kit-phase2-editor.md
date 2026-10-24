# Note Kit Phase 2: Editor Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A new Note Kit window in the desktop app where a user builds a note from parts (add, cut, depth bands, solid or glass), sees the exact silhouette the kernel produces, can toggle the real-renderer previews on and off, undoes, autosaves, saves and loads kit documents, migrates old designer documents, and exports a .rhs; shipped as a testbuild next to the old designer.

**Architecture:** One ES-module front-end under `crates/gui/ui/kit/` with a single store (design plus editor state, `commit(label)` with a sliding undo window) and a render layer that rewrites the panels from state; a Canvas 2D front view that draws the kernel's normalised geometry and edits transforms with handles; coalesced IPC to five Rust commands in a new `crates/gui/src/kit_commands.rs` that wrap the Phase 1 kernel (`rhythia_render::kit`) and translate its errors into the editor's words; previews reuse the designer's cached renderer through a shared frame function. The old designer window stays untouched until Phase 4.

**Tech Stack:** Rust (Tauri 2, serde, the Phase 1 kit), plain JavaScript ES modules (no framework, no bundler), Canvas 2D, Node 22 `node --test` for the pure front-end logic.

**Spec:** `docs/superpowers/specs/2026-10-24-note-kit-2-5d-design.md` (sections 3, 5, 6, 8 phase 2; the owner's decisions of 24.10. evening: depth range stays [-2.0, 0.5]; the old slider mode survives only as quick-shape presets; a new part spans [-0.1, 0.1]; the previews start hidden and the last state is remembered).

## Global Constraints

- Pure Rust and plain JavaScript only: no new crates, no npm packages, no bundler; the front-end runs in Tauri's webview as `<script type="module">` and the pure modules run under `node --test` (Node 22 is installed; the test runner needs no package.json).
- Nothing platform-specific: paths come from the dialog plugin, JavaScript uses no Node-only APIs in browser modules, Rust uses no `std::os::unix`.
- House style: no em or en dashes anywhere (`scripts/preflight.sh` fails on them, also in JavaScript and HTML); comments state constraints, not narration; UI text is English; commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Vocabulary (spec 5.3): the window says Add, Cut, Combine, Depth, Glass, Mirror, Repeat, Rim, Waves, Round; never boolean, union, mesh, vertex, modifier, manifold, lattice. Kernel errors reach the user only through `kit_commands::user_message`.
- Document format: the kit document is the JSON of `rhythia_render::kit::doc::Design` exactly as serde writes it (`{"kind":"circle"}`, `{"kind":"square","rounding":0.0}`, `{"kind":"polygon","sides":6,"rounding":0.0}`, `{"kind":"star","points":5,"inner":0.5}`, `{"kind":"ring","thickness":0.28}`, `{"kind":"pill","aspect":0.5}`, `{"kind":"contours","contours":[...],"label":"..."}`, `{"kind":"points","control":[{"x":..,"y":..,"smooth":true}]}`; `mode` is `"add"` or `"cut"`; `material` is `"solid"` or `"glass"`; `effects` is `{"waves":null,"round":null,"rim":null,"mirror":"none","repeat":null}`; `version` is 2). A file without `"version": 2` is an old designer document and migrates through `rhythia_render::kit::migrate::from_v1`.
- Depth bands live in [-2.0, 0.5] (`kit::Z_MIN`, `kit::Z_MAX`); a new part gets `{"front": 0.1, "back": -0.1}`; the inspector clamps to the range and keeps front above back.
- Transforms the editor offers stay far inside the kernel's range: x and y in [-1.5, 1.5], w and h in [0.02, 1.5], rotation in [-180, 180] degrees. The kernel normalises the union of Add parts to the box, so the front view draws the kernel's normalised geometry and maps handles through the returned `scale`.
- Previews render through the real renderer at 960 by 560 as the designer does, with the kit mesh and the design's colours; hidden previews cost nothing (no IPC, no loop).
- The old designer (`designer.html`, `designer.js`, the `designer_*` commands) keeps working; this phase touches it only to share the preview frame function.
- Run `cargo fmt --all`, `cargo test -p rhythia-gui`, `node --test crates/gui/ui/kit/tests/`, `cargo check --workspace --examples --tests` and `scripts/preflight.sh` before every commit named below.

## Review Focus

Inputs the spec implies but which the happy path would not exercise. Each line names the task whose tests pin it.

1. A slider drag of two hundred events must be one undo step, and the next distinct edit a new one: the sliding coalesce window keyed by label. Pinned in Task 3 (`a_drag_is_one_undo_step`).
2. An evaluation error (every part cut away) must leave the last good geometry on screen and the handles usable; the status line carries the sentence. Pinned in Task 5 (`an_error_keeps_the_last_good_geometry`) and Task 1 (`user_message` mapping).
3. A rotated part must select and scale by its own corners, not by its axis-aligned box: hit test and handles in the part's frame. Pinned in Task 4 (`a_rotated_rect_is_hit_in_its_own_frame`, `corner_handles_follow_the_rotation`).
4. An old designer document (sliders, freeform with symmetry, import) must open as one part and evaluate; the editor must not write the old format back. Pinned in Task 1 (`an_old_designer_file_migrates`) and Task 8 (the migration import path).
5. Previews switched off must not call the backend; switched on they resume with the current design, and the state survives a restart. Pinned in Task 7 (`previews_off_makes_no_calls`).

---

## File structure

- Create `crates/gui/src/kit_commands.rs`: the Rust side of the window (evaluate, mesh, document load with migration, save, export, default document, preview). Pure helpers (`evaluate_design`, `build_design`, `load_document`, `skin_name_from`, `user_message`) are plain functions with unit tests; the `#[tauri::command]` wrappers are thin.
- Modify `crates/gui/src/main.rs`: `mod kit_commands;`, a `kit_skin` cache field on `App`, the preview frame function extracted from `designer_preview`, `open_kit_window`, command registration.
- Create `crates/gui/ui/kit.html`, `crates/gui/ui/kit.css`: the window's layout (toolbar, parts list, front view with the preview pane below it, inspector).
- Create `crates/gui/ui/kit/store.js`: state, commit and undo, part operations, serialisation (pure, Node-tested).
- Create `crates/gui/ui/kit/view-math.js`: the view transform, part rectangles, hit tests, handle maths (pure, Node-tested).
- Create `crates/gui/ui/kit/view.js`: the Canvas 2D front view (drawing, pointer handling) over view-math and the store.
- Create `crates/gui/ui/kit/ipc.js`: `invoke` plus coalesced calls.
- Create `crates/gui/ui/kit/render.js`: the toolbar, parts list and inspector rewritten from state.
- Create `crates/gui/ui/kit/preview.js`: the previews pane, its toggle, the still and the loop, the splitter.
- Create `crates/gui/ui/kit/files.js`: autosave, recent list, save, load, import of the old designer's autosave, export.
- Create `crates/gui/ui/kit/main.js`: bootstrap and wiring, keyboard shortcuts.
- Create `crates/gui/ui/kit/tests/store.test.mjs`, `crates/gui/ui/kit/tests/view-math.test.mjs`, `crates/gui/ui/kit/tests/preview.test.mjs`.
- Modify `crates/gui/ui/index.html` and `crates/gui/ui/app.js`: a "Note Kit (beta)" toolbar button.
- Modify `docs/USER-GUIDE.md`: a short beta paragraph.

---

### Task 1: Rust commands over the kernel

**Files:**
- Create: `crates/gui/src/kit_commands.rs`
- Modify: `crates/gui/src/main.rs` (add `mod kit_commands;` near the other module lines; register the commands in `generate_handler!` right after `designer_preview`)
- Test: inline tests in `kit_commands.rs`

**Interfaces:**
- Consumes: `rhythia_render::kit::{doc::Design, geom::Shapes, levels::evaluate, mesh::build_mesh, validate::validate, export::export_rhs, migrate::from_v1, KitError, MAX_TRIANGLES}`.
- Produces (JSON over IPC, field names as written): `Evaluated { scale, levels, parts: [{ id, shapes }], intervals: [{ z0, z1, solid, glass }], triangles, mesh_error }`, `Built { positions, indices, solid: [start, end], glass: [start, end], triangles, vertices }`, commands `kit_evaluate(design)`, `kit_mesh(design)`, `kit_default_doc()`, `kit_migrate_v1(json)`, `kit_doc_load(path)`, `kit_doc_save(path, json)`, `kit_export(design, path)`; `pub fn user_message(&KitError) -> String`.

- [ ] **Step 1: Write the failing tests**

Create `crates/gui/src/kit_commands.rs` with only the test module first:

```rust
//! The note kit window's commands. The kernel does the work; this file
//! turns its results into what the editor draws and its errors into the
//! editor's own words.

#[cfg(test)]
mod tests {
    use super::*;
    use rhythia_render::kit::doc::Design;

    #[test]
    fn the_default_note_evaluates_to_one_band() {
        let e = evaluate_design(&Design::default_note()).unwrap();
        assert_eq!(e.intervals.len(), 1);
        assert!((e.scale - 1.0).abs() < 1e-4);
        assert_eq!(e.parts.len(), 1);
        assert!(e.triangles.unwrap() > 500);
        assert!(e.mesh_error.is_none());
    }

    #[test]
    fn errors_are_sentences_without_kernel_words() {
        let mut d = Design::default_note();
        d.parts[0].visible = false;
        let m = evaluate_design(&d).unwrap_err();
        assert_eq!(m, "nothing to show: every part is hidden or cut away");
        let banned = ["boolean", "union", "mesh", "vertex", "lattice", "manifold"];
        for e in [
            KitError::Budget(50_000),
            KitError::Complexity(20_000, 3),
            KitError::Scale("point [100.0, 0.0] is outside the lattice range of 64".into()),
            KitError::Empty,
        ] {
            let m = user_message(&e).to_lowercase();
            for b in banned {
                assert!(!m.contains(b), "{m} contains {b}");
            }
        }
    }

    #[test]
    fn a_kit_file_loads_as_is_and_an_old_designer_file_migrates() {
        let kit = serde_json::to_string(&Design::default_note()).unwrap();
        let back: Design = serde_json::from_str(&load_document(&kit).unwrap()).unwrap();
        assert_eq!(back.parts[0].id, "p1");
        // The old designer's document: take the slider fixture verbatim
        // from crates/render/src/kit/migrate.rs's tests (the circle with
        // rounding) so the two test suites agree on the v1 format.
        let v1 = OLD_DESIGNER_FIXTURE;
        let d: Design = serde_json::from_str(&load_document(v1).unwrap()).unwrap();
        assert_eq!(d.version, 2);
        assert_eq!(d.parts.len(), 1);
        assert!(load_document("not json").unwrap_err().starts_with("not a design file"));
    }

    #[test]
    fn skin_names_come_from_the_file_stem_and_stay_safe() {
        assert_eq!(skin_name_from("/tmp/My Note.rhs"), "My Note");
        assert_eq!(skin_name_from("C:\\skins\\a!b.rhs"), "a-b");
        assert_eq!(skin_name_from("/tmp/.rhs"), "note");
        assert_eq!(skin_name_from("/tmp/   .rhs"), "note");
        let long = format!("/tmp/{}.rhs", "x".repeat(90));
        assert_eq!(skin_name_from(&long).len(), 64);
    }

    #[test]
    fn the_built_mesh_carries_its_ranges() {
        let b = build_design(&Design::default_note()).unwrap();
        assert_eq!(b.solid, [0, b.indices.len()]);
        assert_eq!(b.glass, [b.indices.len(), b.indices.len()]);
        assert_eq!(b.triangles, b.indices.len() / 3);
        assert_eq!(b.vertices, b.positions.len());
    }
}
```

`OLD_DESIGNER_FIXTURE` is a `const OLD_DESIGNER_FIXTURE: &str = r##"..."##;` inside the test module holding the v1 JSON copied from the first slider test in `crates/render/src/kit/migrate.rs` (read that file; the fixture's keys are `P`, `free`, `symmetry`, `mirror`, `colors`, `imported`, matching the old `docState()` in `crates/gui/ui/designer.js:93-95`).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-gui kit_commands`
Expected: compile error, `evaluate_design` not found (after adding `mod kit_commands;` to `main.rs`).

- [ ] **Step 3: Implement the module**

Replace the file's head (keep the test module) with:

```rust
//! The note kit window's commands. The kernel does the work; this file
//! turns its results into what the editor draws and its errors into the
//! editor's own words.

use rhythia_render::kit::doc::Design;
use rhythia_render::kit::geom::Shapes;
use rhythia_render::kit::{export, levels, mesh, migrate, validate, KitError, MAX_TRIANGLES};
use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct PartGeometry {
    pub id: String,
    pub shapes: Shapes,
}

#[derive(Serialize, Debug)]
pub struct LevelRegion {
    pub z0: f32,
    pub z1: f32,
    pub solid: Shapes,
    pub glass: Shapes,
}

/// What the front view draws: everything in note units after the
/// kernel's normalisation, plus the factor that maps the document's
/// transforms onto it.
#[derive(Serialize, Debug)]
pub struct Evaluated {
    pub scale: f32,
    pub levels: Vec<f32>,
    pub parts: Vec<PartGeometry>,
    pub intervals: Vec<LevelRegion>,
    /// Triangle count of the baked note; None with `mesh_error` set when
    /// the mesh step refused the design (too detailed, say).
    pub triangles: Option<usize>,
    pub mesh_error: Option<String>,
}

/// The window never shows kernel vocabulary (spec 5.3).
pub fn user_message(e: &KitError) -> String {
    match e {
        KitError::Empty => "nothing to show: every part is hidden or cut away".into(),
        KitError::Budget(n) => {
            format!("too detailed: {n} triangles, the limit is {MAX_TRIANGLES}")
        }
        KitError::Complexity(points, outlines) => {
            format!("too complex: {points} points in {outlines} outlines; lower Repeat or Waves")
        }
        KitError::Scale(_) => "a part reaches too far from the centre".into(),
        KitError::Doc(m) | KitError::Invalid(m) => m.clone(),
    }
}

pub fn evaluate_design(design: &Design) -> Result<Evaluated, String> {
    let ev = levels::evaluate(design).map_err(|e| user_message(&e))?;
    let (triangles, mesh_error) = match mesh::build_mesh(&ev) {
        Ok(m) => (Some(m.indices.len() / 3), None),
        Err(e) => (None, Some(user_message(&e))),
    };
    Ok(Evaluated {
        scale: ev.scale,
        levels: ev.levels.clone(),
        parts: ev
            .parts
            .iter()
            .map(|p| PartGeometry {
                id: p.id.clone(),
                shapes: p.shapes.clone(),
            })
            .collect(),
        intervals: ev
            .intervals
            .iter()
            .map(|i| LevelRegion {
                z0: i.z0,
                z1: i.z1,
                solid: i.solid.clone(),
                glass: i.glass.clone(),
            })
            .collect(),
        triangles,
        mesh_error,
    })
}

/// The baked note for the turntable (Phase 3) and for diagnostics.
#[derive(Serialize, Debug)]
pub struct Built {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub solid: [usize; 2],
    pub glass: [usize; 2],
    pub triangles: usize,
    pub vertices: usize,
}

pub fn build_design(design: &Design) -> Result<Built, String> {
    let ev = levels::evaluate(design).map_err(|e| user_message(&e))?;
    let m = mesh::build_mesh(&ev).map_err(|e| user_message(&e))?;
    let r = validate::validate(&m).map_err(|e| user_message(&e))?;
    Ok(Built {
        solid: [m.solid.start, m.solid.end],
        glass: [m.glass.start, m.glass.end],
        triangles: r.triangles,
        vertices: r.vertices,
        positions: m.positions,
        indices: m.indices,
    })
}

/// A saved document: version 2 as is (re-serialised so the editor only
/// ever sees serde's spelling), anything else is the old designer's
/// format and migrates. Returns the design's JSON.
pub fn load_document(text: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not a design file: {e}"))?;
    if value.get("version").and_then(|v| v.as_u64()) == Some(2) {
        let d: Design =
            serde_json::from_value(value).map_err(|e| format!("design file damaged: {e}"))?;
        return serde_json::to_string(&d).map_err(|e| e.to_string());
    }
    let d = migrate::from_v1(text).map_err(|e| user_message(&e))?;
    serde_json::to_string(&d).map_err(|e| e.to_string())
}

/// The skin name the export accepts (letters, digits, spaces, `_`, `-`,
/// 1 to 64 characters, no leading or trailing space), from a file stem.
pub fn skin_name_from(path: &str) -> String {
    let stem = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".rhs");
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        "note".into()
    } else {
        cleaned
    }
}

fn join<T>(r: Result<Result<T, String>, tauri::Error>) -> Result<T, String> {
    r.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn kit_evaluate(design: Design) -> Result<Evaluated, String> {
    join(tauri::async_runtime::spawn_blocking(move || evaluate_design(&design)).await)
}

#[tauri::command]
pub async fn kit_mesh(design: Design) -> Result<Built, String> {
    join(tauri::async_runtime::spawn_blocking(move || build_design(&design)).await)
}

#[tauri::command]
pub fn kit_default_doc() -> Result<String, String> {
    serde_json::to_string(&Design::default_note()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn kit_migrate_v1(json: String) -> Result<String, String> {
    load_document(&json)
}

#[tauri::command]
pub async fn kit_doc_load(path: String) -> Result<String, String> {
    join(
        tauri::async_runtime::spawn_blocking(move || {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            load_document(&text)
        })
        .await,
    )
}

#[tauri::command]
pub async fn kit_doc_save(path: String, json: String) -> Result<String, String> {
    join(
        tauri::async_runtime::spawn_blocking(move || {
            // Only a design that parses is written: the file is the user's work.
            serde_json::from_str::<Design>(&json).map_err(|e| format!("design file damaged: {e}"))?;
            std::fs::write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
            Ok(path)
        })
        .await,
    )
}

#[tauri::command]
pub async fn kit_export(design: Design, path: String) -> Result<String, String> {
    join(
        tauri::async_runtime::spawn_blocking(move || {
            let name = skin_name_from(&path);
            let bytes = export::export_rhs(&design, &name).map_err(|e| user_message(&e))?;
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            Ok(path)
        })
        .await,
    )
}
```

The length cap: `.take(64)` runs before `trim`, so a 90-character stem gives 64 characters, which the test asserts; a stem that ends in spaces after the cut is trimmed below 64, which the export accepts too.

In `main.rs`: add `mod kit_commands;` next to the other `mod` lines, and in `generate_handler![...]` after `designer_preview,` add `kit_commands::kit_evaluate, kit_commands::kit_mesh, kit_commands::kit_default_doc, kit_commands::kit_migrate_v1, kit_commands::kit_doc_load, kit_commands::kit_doc_save, kit_commands::kit_export,`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-gui kit_commands`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/gui/src/kit_commands.rs crates/gui/src/main.rs
git commit -m "Kit window: commands over the kernel, errors in the editor's words

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Previews through the shared renderer, and the window

**Files:**
- Modify: `crates/gui/src/main.rs` (`App` gains `kit_skin`; `designer_preview`'s scene code moves into `preview_frame`; new `kit_preview` and `open_kit_window`; registration)
- Modify: `crates/gui/ui/index.html` and `crates/gui/ui/app.js` (the toolbar button)
- Test: inline test in `main.rs`'s existing test module (a GPU test like `crates/render/tests/kit_render.rs`; this machine has an adapter)

**Interfaces:**
- Produces: `kit_preview(design: Design, time_ms: f64, still: Option<bool>) -> Result<String, String>` (a `data:image/png;base64,...` URL exactly like `designer_preview`), `open_kit_window()`.

- [ ] **Step 1: Write the failing test**

In `main.rs`'s test module (there are four `#[cfg(test)]` modules; use the one nearest the designer code, or add one right after `designer_preview`):

```rust
    #[test]
    fn a_kit_frame_renders_through_the_shared_path() {
        let design = rhythia_render::kit::doc::Design::default_note();
        let cfg = rhythia_render::config::SkinConfig::default();
        let renderer = rhythia_render::Renderer::new(
            DESIGNER_PREVIEW_W,
            DESIGNER_PREVIEW_H,
            cfg.hud_fonts().as_slice(),
        )
        .unwrap();
        let (cfg, skin) = kit_skin_for(&renderer, &design).unwrap();
        let url = preview_frame(&renderer, &cfg, &skin, 300.0, true).unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.len() > 1000);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p rhythia-gui a_kit_frame_renders`
Expected: compile error, `kit_skin_for` and `preview_frame` not found.

- [ ] **Step 3: Implement**

1. In `App` (main.rs around line 576) add, after `designer_skin`:

```rust
    /// The kit window's prepared skin keyed by the design's JSON hash; it
    /// shares `designer_renderer` so one wgpu device serves both windows.
    kit_skin: Mutex<
        Option<(
            u64,
            rhythia_render::config::SkinConfig,
            rhythia_render::renderer::SkinTextures,
        )>,
    >,
```

and initialise it with `Mutex::new(None)` where `designer_skin` is initialised.

2. Extract the scene part of `designer_preview` (everything after `let (_, cfg, skin) = cache.as_ref().expect("just filled");` down to the data URL return) into a free function next to it:

```rust
/// One preview frame: the gameplay loop at `time_ms`, or the still scene
/// (three notes side by side at staggered depths) when `still`. Shared by
/// the old designer and the kit window so the two can never drift.
fn preview_frame(
    renderer: &rhythia_render::Renderer,
    cfg: &rhythia_render::config::SkinConfig,
    skin: &rhythia_render::renderer::SkinTextures,
    time_ms: f64,
    still: bool,
) -> Result<String, String> {
    // body moved verbatim from designer_preview; `still.unwrap_or(false)`
    // becomes the `still` parameter
}
```

`designer_preview` then ends with `preview_frame(renderer, cfg, skin, time_ms, still.unwrap_or(false))`. Its tests and behaviour do not change.

3. The kit skin builder:

```rust
/// The kit note as a prepared skin: the baked mesh as an obj, the
/// design's colours, the preview camera (tele FOV, no cursor).
fn kit_skin_for(
    renderer: &rhythia_render::Renderer,
    design: &rhythia_render::kit::doc::Design,
) -> Result<(rhythia_render::config::SkinConfig, rhythia_render::renderer::SkinTextures), String> {
    let ev = rhythia_render::kit::levels::evaluate(design)
        .map_err(|e| kit_commands::user_message(&e))?;
    let mesh = rhythia_render::kit::mesh::build_mesh(&ev)
        .map_err(|e| kit_commands::user_message(&e))?;
    let obj = rhythia_render::kit::export::to_obj(&mesh, "preview");
    let mut cfg = rhythia_render::config::SkinConfig::default();
    cfg.note_mesh = Some(obj.into_bytes());
    let cs = rhythia_render::config::parse_colorset(&design.colors.join("\n"));
    if !cs.is_empty() {
        cfg.colorset = cs;
    }
    cfg.camera_fov = 34.0;
    cfg.cursor_opacity = 0.0;
    cfg.cursor_trail_enabled = false;
    let skin = renderer.prepare_skin(&cfg);
    Ok((cfg, skin))
}

#[tauri::command]
async fn kit_preview(
    state: tauri::State<'_, App>,
    design: rhythia_render::kit::doc::Design,
    time_ms: f64,
    still: Option<bool>,
) -> Result<String, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_string(&design).map_err(|e| e.to_string())?.hash(&mut h);
            h.finish()
        };
        let mut slot = app.designer_renderer.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_none() {
            let cfg = rhythia_render::config::SkinConfig::default();
            *slot = Some(
                rhythia_render::Renderer::new(
                    DESIGNER_PREVIEW_W,
                    DESIGNER_PREVIEW_H,
                    cfg.hud_fonts().as_slice(),
                )
                .map_err(|e| e.to_string())?,
            );
        }
        let renderer = slot.as_ref().expect("just created");
        let mut cache = app.kit_skin.lock().unwrap_or_else(|p| p.into_inner());
        let stale = cache.as_ref().map(|(h, _, _)| *h != hash).unwrap_or(true);
        if stale {
            let (cfg, skin) = kit_skin_for(renderer, &design)?;
            *cache = Some((hash, cfg, skin));
        }
        let (_, cfg, skin) = cache.as_ref().expect("just filled");
        preview_frame(renderer, cfg, skin, time_ms, still.unwrap_or(false))
    })
    .await
    .map_err(err_str)?
}
```

4. `open_kit_window`: a copy of `open_designer_window` (main.rs:4296-4350) with the label `"kit"`, the URL `kit.html`, the title `rhythr · Note Kit`, the same sizes, and the destroy handler clearing `kit_skin` (leave `designer_renderer` alone when the designer window is still open: clear it only if `app_handle.get_webview_window("designer")` is `None`). Register `kit_preview` and `open_kit_window` in `generate_handler!`.

5. `index.html`: next to `btn-designer` add `<button class="btn small" id="btn-kit" title="Build a note from parts: add, cut, depth, glass (beta)">Note Kit (beta)</button>`; `app.js` next to the designer handler: `$("btn-kit").addEventListener("click", () => { call(() => invoke("open_kit_window")); });`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-gui` (the whole crate: the designer tests prove the extraction changed nothing)
Expected: all pass, including `a_kit_frame_renders_through_the_shared_path`.

- [ ] **Step 5: Commit**

```bash
git add crates/gui/src/main.rs crates/gui/ui/index.html crates/gui/ui/app.js
git commit -m "Kit window: previews through the shared frame function, the window and its button

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: The store

**Files:**
- Create: `crates/gui/ui/kit/store.js`
- Test: `crates/gui/ui/kit/tests/store.test.mjs`

**Interfaces:**
- Produces: `createStore(design, { now })` returning `{ state, subscribe, commit(label, mutate), undo, redo, select, part, nextId, addPart(kind), removePart, duplicatePart, updatePart(id, label, patch), movePart(id, to), load(design, path), setEvaluated, setEvalError, setStatus, setUi, serialize }`; constants `DEPTH_MIN`, `DEPTH_MAX`, `NEW_DEPTH`, `UNDO_LIMIT`, `COALESCE_MS`, `SHAPES`, `newPart(id, kind)`.

- [ ] **Step 1: Write the failing tests**

```js
// crates/gui/ui/kit/tests/store.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { createStore, newPart, COALESCE_MS, UNDO_LIMIT } from "../store.js";

function design() {
  return { version: 2, name: "t", colors: ["#ffffff"], parts: [newPart("p1", "circle")], groups: [] };
}

test("a_drag_is_one_undo_step", () => {
  let t = 1000;
  const s = createStore(design(), { now: () => t });
  for (let i = 0; i < 200; i++) {
    s.updatePart("p1", "move", (p) => (p.transform.x += 0.001));
    t += 10;
  }
  assert.equal(s.state.undo.length, 1);
  t += COALESCE_MS + 1;
  s.updatePart("p1", "move", (p) => (p.transform.x += 0.1));
  assert.equal(s.state.undo.length, 2);
  s.updatePart("p1", "rotate", (p) => (p.transform.rotation = 10));
  assert.equal(s.state.undo.length, 3, "a different label never merges");
  assert.ok(s.undo());
  assert.equal(s.part("p1").transform.rotation, 0);
  assert.ok(s.redo());
  assert.equal(s.part("p1").transform.rotation, 10);
});

test("add_duplicate_remove_keep_ids_unique_and_selection_valid", () => {
  const s = createStore(design());
  const id = s.addPart("star");
  assert.equal(id, "p2");
  assert.equal(s.state.selected, "p2");
  const copy = s.duplicatePart("p2");
  assert.equal(copy, "p3");
  assert.equal(s.part("p3").name, "Star copy");
  assert.equal(s.state.design.parts.map((p) => p.id).join(","), "p1,p2,p3");
  s.removePart("p3");
  assert.equal(s.state.selected, "p2", "selection falls back to the top part");
  s.state.design.groups.push({ id: "g", name: "g", parts: ["p1", "p2"], mode: "add" });
  s.removePart("p2");
  assert.deepEqual(s.state.design.groups[0].parts, ["p1"]);
  assert.ok(s.undo());
  assert.equal(s.state.design.parts.length, 2);
});

test("move_reorders_within_bounds", () => {
  const s = createStore(design());
  s.addPart("ring");
  s.addPart("pill");
  s.movePart("p3", 0);
  assert.equal(s.state.design.parts[0].id, "p3");
  s.movePart("p3", 99);
  assert.equal(s.state.design.parts[2].id, "p3");
});

test("undo_is_bounded_and_load_resets_history", () => {
  let t = 0;
  const s = createStore(design(), { now: () => (t += COALESCE_MS + 1) });
  for (let i = 0; i < UNDO_LIMIT + 20; i++) s.updatePart("p1", "x", (p) => (p.transform.x = i));
  assert.equal(s.state.undo.length, UNDO_LIMIT);
  s.load(design(), "/tmp/a.rhsd");
  assert.equal(s.state.undo.length, 0);
  assert.equal(s.state.path, "/tmp/a.rhsd");
  assert.equal(s.state.dirty, false);
});

test("serialize_writes_the_kit_format", () => {
  const s = createStore(design());
  const d = JSON.parse(s.serialize());
  assert.equal(d.version, 2);
  assert.deepEqual(d.parts[0].shape, { kind: "circle" });
  assert.deepEqual(d.parts[0].effects, { waves: null, round: null, rim: null, mirror: "none", repeat: null });
  assert.deepEqual(d.parts[0].depth, { front: 0.1, back: -0.1 });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `node --test crates/gui/ui/kit/tests/`
Expected: failure, cannot find module `../store.js`.

- [ ] **Step 3: Implement the store**

```js
// crates/gui/ui/kit/store.js
// One store: the design, the editor state and undo. Every change goes
// through commit(label, mutate); the render layer redraws from state, so
// panels and state can never disagree.

export const DEPTH_MIN = -2.0;
export const DEPTH_MAX = 0.5;
export const NEW_DEPTH = { front: 0.1, back: -0.1 };
export const UNDO_LIMIT = 100;
// A continuous gesture (a slider drag, a key held down) is one undo step:
// commits with the same label inside this window merge into the first.
export const COALESCE_MS = 350;

export const SHAPES = {
  circle: () => ({ kind: "circle" }),
  square: () => ({ kind: "square", rounding: 0 }),
  polygon: () => ({ kind: "polygon", sides: 6, rounding: 0 }),
  star: () => ({ kind: "star", points: 5, inner: 0.5 }),
  ring: () => ({ kind: "ring", thickness: 0.28 }),
  pill: () => ({ kind: "pill", aspect: 0.5 }),
};
const NAMES = { circle: "Disc", square: "Square", polygon: "Polygon", star: "Star", ring: "Ring", pill: "Pill" };

export function newPart(id, kind) {
  return {
    id,
    name: NAMES[kind] || "Part",
    visible: true,
    locked: false,
    shape: SHAPES[kind](),
    transform: { x: 0, y: 0, w: 1, h: 1, rotation: 0 },
    mode: "add",
    depth: { ...NEW_DEPTH },
    material: "solid",
    effects: { waves: null, round: null, rim: null, mirror: "none", repeat: null },
  };
}

function topId(design) {
  return design.parts.length ? design.parts[design.parts.length - 1].id : null;
}

export function createStore(design, opts = {}) {
  const now = opts.now || (() => Date.now());
  const listeners = new Set();
  const s = {
    design,
    selected: topId(design),
    path: null,
    dirty: false,
    undo: [],
    redo: [],
    last: { label: null, at: -Infinity },
    ui: { previews: false },
    // The last good evaluation stays on screen through an error, so the
    // handles never vanish under the cursor.
    evaluated: null,
    evalError: null,
    status: { text: "", error: false },
  };
  const emit = () => {
    for (const l of listeners) l(s);
  };
  const snap = () => JSON.stringify(s.design);
  const noLast = () => ({ label: null, at: -Infinity });
  function fixSelection() {
    if (s.selected && !s.design.parts.some((p) => p.id === s.selected)) s.selected = topId(s.design);
  }
  const api = {
    state: s,
    subscribe(fn) {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    commit(label, mutate) {
      const before = snap();
      mutate(s.design);
      const t = now();
      const merge = label === s.last.label && t - s.last.at < COALESCE_MS;
      if (!merge) {
        s.undo.push(before);
        if (s.undo.length > UNDO_LIMIT) s.undo.shift();
        s.redo.length = 0;
      }
      s.last = { label, at: t };
      s.dirty = true;
      fixSelection();
      emit();
    },
    undo() {
      const b = s.undo.pop();
      if (b === undefined) return false;
      s.redo.push(snap());
      s.design = JSON.parse(b);
      s.last = noLast();
      s.dirty = true;
      fixSelection();
      emit();
      return true;
    },
    redo() {
      const n = s.redo.pop();
      if (n === undefined) return false;
      s.undo.push(snap());
      s.design = JSON.parse(n);
      s.last = noLast();
      s.dirty = true;
      fixSelection();
      emit();
      return true;
    },
    select(id) {
      s.selected = id;
      emit();
    },
    part(id = s.selected) {
      return s.design.parts.find((p) => p.id === id) || null;
    },
    nextId() {
      const ids = new Set(s.design.parts.map((p) => p.id));
      let n = 1;
      while (ids.has(`p${n}`)) n++;
      return `p${n}`;
    },
    addPart(kind) {
      const id = api.nextId();
      api.commit("add", (d) => d.parts.push(newPart(id, kind)));
      s.selected = id;
      emit();
      return id;
    },
    removePart(id) {
      api.commit("remove", (d) => {
        d.parts = d.parts.filter((p) => p.id !== id);
        for (const g of d.groups) g.parts = g.parts.filter((x) => x !== id);
      });
    },
    duplicatePart(id) {
      const src = api.part(id);
      if (!src) return null;
      const nid = api.nextId();
      api.commit("duplicate", (d) => {
        const copy = JSON.parse(JSON.stringify(src));
        copy.id = nid;
        copy.name = `${src.name} copy`;
        const i = d.parts.findIndex((p) => p.id === id);
        d.parts.splice(i + 1, 0, copy);
      });
      s.selected = nid;
      emit();
      return nid;
    },
    updatePart(id, label, patch) {
      api.commit(label, (d) => {
        const p = d.parts.find((x) => x.id === id);
        if (p) patch(p);
      });
    },
    movePart(id, to) {
      api.commit("reorder", (d) => {
        const from = d.parts.findIndex((p) => p.id === id);
        if (from < 0) return;
        const [p] = d.parts.splice(from, 1);
        d.parts.splice(Math.max(0, Math.min(d.parts.length, to)), 0, p);
      });
    },
    load(next, path = null) {
      s.design = next;
      s.path = path;
      s.dirty = false;
      s.undo.length = 0;
      s.redo.length = 0;
      s.last = noLast();
      s.selected = topId(next);
      emit();
    },
    setEvaluated(ev) {
      s.evaluated = ev;
      s.evalError = null;
      emit();
    },
    setEvalError(msg) {
      s.evalError = msg;
      emit();
    },
    setStatus(text, error = false) {
      s.status = { text, error };
      emit();
    },
    setUi(patch) {
      Object.assign(s.ui, patch);
      emit();
    },
    serialize() {
      return JSON.stringify(s.design);
    },
  };
  return api;
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `node --test crates/gui/ui/kit/tests/`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/gui/ui/kit/store.js crates/gui/ui/kit/tests/store.test.mjs
git commit -m "Kit window: the store, one commit path with a sliding undo window

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: View maths

**Files:**
- Create: `crates/gui/ui/kit/view-math.js`
- Test: `crates/gui/ui/kit/tests/view-math.test.mjs`

**Interfaces:**
- Produces: `makeView()`, `unit(view, size)`, `toPx(view, size, p)`, `toWorld(view, size, x, y)`, `zoomAt(view, size, x, y, factor)`, `partRect(transform, scale)`, `rectCorners(rect)`, `toLocal(rect, p)`, `rectContains(rect, p)`, `HANDLES`, `handleAt(rect, p, tolerance)`, `scaleFrom(rect, handle, p, keepAspect)`, `angleFrom(rect, p)`, `HITBOX`.

- [ ] **Step 1: Write the failing tests**

```js
// crates/gui/ui/kit/tests/view-math.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import {
  makeView, toPx, toWorld, zoomAt, partRect, rectCorners, rectContains, handleAt, scaleFrom, angleFrom,
} from "../view-math.js";

const size = { w: 800, h: 600 };
const near = (a, b, eps = 1e-6) => assert.ok(Math.abs(a - b) < eps, `${a} vs ${b}`);

test("px_and_world_round_trip_and_zoom_keeps_the_cursor_point", () => {
  const v = makeView();
  const [x, y] = toPx(v, size, [0.3, -0.2]);
  const [wx, wy] = toWorld(v, size, x, y);
  near(wx, 0.3);
  near(wy, -0.2);
  zoomAt(v, size, 500, 100, 2);
  const [ax, ay] = toWorld(v, size, 500, 100);
  const u = makeView();
  const [bx, by] = toWorld(u, size, 500, 100);
  near(ax, bx);
  near(ay, by);
});

test("a_rotated_rect_is_hit_in_its_own_frame", () => {
  // Half extents 1 by 0.2, turned 45 degrees: the point (0.6, 0.6) lies
  // on the long axis and is inside; (0.6, -0.6) is across it and outside.
  const r = partRect({ x: 0, y: 0, w: 1, h: 0.2, rotation: 45 }, 1);
  assert.ok(rectContains(r, [0.6, 0.6]));
  assert.ok(!rectContains(r, [0.6, -0.6]));
  // The scale maps document transforms onto the normalised drawing.
  const r2 = partRect({ x: 1, y: 0, w: 0.5, h: 0.5, rotation: 0 }, 0.5);
  assert.ok(rectContains(r2, [0.5, 0]));
  assert.ok(!rectContains(r2, [1, 0]));
});

test("corner_handles_follow_the_rotation", () => {
  const r = partRect({ x: 0, y: 0, w: 1, h: 0.5, rotation: 90 }, 1);
  const { corners } = rectCorners(r);
  // Local (1, 0.5) turned by 90 degrees is (-0.5, 1).
  near(corners[2][0], -0.5);
  near(corners[2][1], 1);
  assert.equal(handleAt(r, [-0.5, 1.0], 0.05), "ne");
  assert.equal(handleAt(r, [0, 0], 0.05), null);
});

test("scaling_is_about_the_centre_and_shift_keeps_aspect", () => {
  const r = partRect({ x: 0, y: 0, w: 1, h: 0.5, rotation: 0 }, 1);
  assert.deepEqual(scaleFrom(r, "e", [1.5, 0.1], false), { hw: 1.5, hh: 0.5 });
  const k = scaleFrom(r, "ne", [2, 0.5], true);
  near(k.hw, 2);
  near(k.hh, 1);
  assert.equal(scaleFrom(r, "w", [0, 0], false).hw, 0.02, "never collapses");
});

test("the_rotation_handle_reads_degrees_from_the_top", () => {
  const r = partRect({ x: 0, y: 0, w: 1, h: 1, rotation: 0 }, 1);
  near(angleFrom(r, [0, 2]), 0);
  near(angleFrom(r, [-2, 0]), 90);
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `node --test crates/gui/ui/kit/tests/`
Expected: failure, cannot find module `../view-math.js`.

- [ ] **Step 3: Implement**

```js
// crates/gui/ui/kit/view-math.js
// Pure maths for the front view: a screen-space zoom and pan over note
// units (half-extent 1 is the box), part rectangles in the part's own
// frame, hit tests and handle geometry. No DOM here, so it is testable.

// The game's true hit area over the note's visual half-size.
export const HITBOX = 0.56875 / 0.45;
export const MIN_HALF = 0.02;

export function makeView() {
  return { z: 1, cx: 0, cy: 0 };
}

export function unit(view, size) {
  return (Math.min(size.w, size.h) / 2.9) * view.z;
}

export function toPx(view, size, p) {
  const u = unit(view, size);
  return [size.w / 2 + (p[0] - view.cx) * u, size.h / 2 - (p[1] - view.cy) * u];
}

export function toWorld(view, size, x, y) {
  const u = unit(view, size);
  return [(x - size.w / 2) / u + view.cx, -(y - size.h / 2) / u + view.cy];
}

// The point under the cursor stays under the cursor.
export function zoomAt(view, size, x, y, factor) {
  const [wx, wy] = toWorld(view, size, x, y);
  view.z = Math.max(0.4, Math.min(12, view.z * factor));
  const u = unit(view, size);
  view.cx = wx - (x - size.w / 2) / u;
  view.cy = wy + (y - size.h / 2) / u;
  return view;
}

// A part's rectangle in drawing units: the document transform scaled by
// the kernel's normalisation factor, so handles sit on the drawn shape.
export function partRect(t, scale) {
  return { cx: t.x * scale, cy: t.y * scale, hw: t.w * scale, hh: t.h * scale, rot: (t.rotation * Math.PI) / 180 };
}

export function rectCorners(r) {
  const c = Math.cos(r.rot);
  const s = Math.sin(r.rot);
  const at = (dx, dy) => [r.cx + dx * c - dy * s, r.cy + dx * s + dy * c];
  return {
    corners: [at(-r.hw, -r.hh), at(r.hw, -r.hh), at(r.hw, r.hh), at(-r.hw, r.hh)],
    edges: [at(0, -r.hh), at(r.hw, 0), at(0, r.hh), at(-r.hw, 0)],
    top: at(0, r.hh),
  };
}

export function toLocal(r, p) {
  const c = Math.cos(-r.rot);
  const s = Math.sin(-r.rot);
  const dx = p[0] - r.cx;
  const dy = p[1] - r.cy;
  return [dx * c - dy * s, dx * s + dy * c];
}

export function rectContains(r, p) {
  const [lx, ly] = toLocal(r, p);
  return Math.abs(lx) <= r.hw && Math.abs(ly) <= r.hh;
}

// Corners then edges, in the order rectCorners returns them.
export const HANDLES = ["sw", "se", "ne", "nw", "s", "e", "n", "w"];

export function handleAt(r, p, tolerance) {
  const g = rectCorners(r);
  const all = [...g.corners, ...g.edges];
  for (let i = 0; i < all.length; i++) {
    if (Math.hypot(all[i][0] - p[0], all[i][1] - p[1]) <= tolerance) return HANDLES[i];
  }
  return null;
}

// Scaling about the centre: the pointer's local coordinate sets the half
// extent on the dragged axis; a corner drags both.
export function scaleFrom(r, handle, p, keepAspect) {
  const [lx, ly] = toLocal(r, p);
  let hw = r.hw;
  let hh = r.hh;
  if (handle.includes("e") || handle.includes("w")) hw = Math.max(MIN_HALF, Math.abs(lx));
  if (handle.includes("n") || handle.includes("s")) hh = Math.max(MIN_HALF, Math.abs(ly));
  if (keepAspect && r.hw > 0 && r.hh > 0) {
    const k = Math.max(hw / r.hw, hh / r.hh);
    hw = r.hw * k;
    hh = r.hh * k;
  }
  return { hw, hh };
}

// The rotation handle sits above the top edge; straight up is zero.
export function angleFrom(r, p) {
  return (Math.atan2(p[1] - r.cy, p[0] - r.cx) * 180) / Math.PI - 90;
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `node --test crates/gui/ui/kit/tests/`
Expected: 10 passed (5 store, 5 view-math).

- [ ] **Step 5: Commit**

```bash
git add crates/gui/ui/kit/view-math.js crates/gui/ui/kit/tests/view-math.test.mjs
git commit -m "Kit window: view maths, rectangles in the part's own frame

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: The window, the IPC layer and the front view

**Files:**
- Create: `crates/gui/ui/kit.html`, `crates/gui/ui/kit.css`, `crates/gui/ui/kit/ipc.js`, `crates/gui/ui/kit/view.js`, `crates/gui/ui/kit/main.js` (bootstrap only; the panels come in Task 6)
- Test: `crates/gui/ui/kit/tests/ipc.test.mjs` (the coalescing with a fake invoke), manual check in the window

**Interfaces:**
- Consumes: the store (Task 3), view-math (Task 4), `kit_evaluate` and `kit_default_doc` (Task 1).
- Produces: `coalesced(name, onResult, onError, invokeFn)` in `ipc.js`; `createView(canvas, store, { onEdit })` in `view.js` with `draw()`, `fit()`, `resetView()`; `main.js` exposes nothing.

- [ ] **Step 1: Write the failing test for the IPC layer**

```js
// crates/gui/ui/kit/tests/ipc.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { coalesced } from "../ipc.js";

test("an_error_keeps_the_last_good_geometry", async () => {
  // Three requests while one is in flight: only the newest runs after,
  // and an error reaches onError without touching onResult.
  const calls = [];
  let resolveFirst;
  const fake = (name, args) =>
    new Promise((resolve, reject) => {
      calls.push(args.design);
      if (args.design === "bad") reject(new Error("nothing to show"));
      else if (calls.length === 1) resolveFirst = resolve;
      else resolve(`ok:${args.design}`);
    });
  const results = [];
  const errors = [];
  const run = coalesced("kit_evaluate", (r) => results.push(r), (e) => errors.push(e), fake);
  run({ design: "a" });
  run({ design: "b" });
  run({ design: "c" });
  resolveFirst("ok:a");
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(calls, ["a", "c"]);
  assert.deepEqual(results, ["ok:a", "ok:c"]);
  run({ design: "bad" });
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(results, ["ok:a", "ok:c"], "a failed call leaves the last result alone");
  assert.equal(errors.length, 1);
  assert.match(errors[0], /nothing to show/);
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test crates/gui/ui/kit/tests/`
Expected: failure, cannot find module `../ipc.js`.

- [ ] **Step 3: Implement**

`crates/gui/ui/kit/ipc.js`:

```js
// IPC with one rule: at most one call of a kind in flight, the latest
// arguments win, errors are reported but never clear the last result.
export const invoke = (name, args) => window.__TAURI__.core.invoke(name, args);

export function coalesced(name, onResult, onError, invokeFn = invoke) {
  let busy = false;
  let again = null;
  async function run(args) {
    if (busy) {
      again = args;
      return;
    }
    busy = true;
    try {
      onResult(await invokeFn(name, args));
    } catch (e) {
      onError(String(e && e.message ? e.message : e));
    } finally {
      busy = false;
      if (again) {
        const a = again;
        again = null;
        run(a);
      }
    }
  }
  return run;
}
```

`crates/gui/ui/kit.html`:

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>rhythr · Note Kit</title>
  <link rel="stylesheet" href="style.css" />
  <link rel="stylesheet" href="kit.css" />
</head>
<body id="kit-body">
  <header id="kt-toolbar">
    <div class="group" id="kt-add">
      <span class="label">Add</span>
      <button class="btn small" data-add="circle">Disc</button>
      <button class="btn small" data-add="square">Square</button>
      <button class="btn small" data-add="polygon">Polygon</button>
      <button class="btn small" data-add="star">Star</button>
      <button class="btn small" data-add="ring">Ring</button>
      <button class="btn small" data-add="pill">Pill</button>
    </div>
    <div class="group">
      <button class="btn small ghost" id="kt-undo" title="Undo (Ctrl+Z)">Undo</button>
      <button class="btn small ghost" id="kt-redo" title="Redo (Ctrl+Y)">Redo</button>
    </div>
    <div class="group">
      <button class="btn small" id="kt-previews" title="Show or hide the previews (P)">Previews</button>
    </div>
    <div class="group grow"></div>
    <div class="group">
      <button class="btn small ghost" id="kt-new" title="New design">New</button>
      <button class="btn small ghost" id="kt-load" title="Open a design (Ctrl+O)">Open</button>
      <button class="btn small ghost" id="kt-save" title="Save the design (Ctrl+S)">Save</button>
      <button class="btn small ghost" id="kt-import-old" title="Bring in the note from the old Note Designer">From old designer</button>
      <button class="btn small" id="kt-export" title="Export as .rhs for rhythr and the game">Export .rhs</button>
    </div>
  </header>
  <main id="kt-root">
    <aside id="kt-parts-pane">
      <h2>Parts</h2>
      <div id="kt-parts"></div>
      <div class="hint">Top of the list draws on top. Drag to reorder.</div>
    </aside>
    <section id="kt-centre">
      <div id="kt-canvas-box">
        <canvas id="kt-canvas"></canvas>
        <button id="kt-view-reset" class="btn small ghost" title="Reset zoom and pan">100%</button>
      </div>
      <div id="kt-canvas-hint" class="hint">
        Click selects a part, drag moves it, corner handles scale (Shift keeps the proportions),
        the handle above rotates, arrows nudge. Wheel zooms, middle drag or Space+drag pans.
        The solid square is the hard size limit; the dashed one is the game's true hit area.
      </div>
      <div id="kt-splitter" title="Drag to resize editor vs. previews" hidden></div>
      <div id="kt-preview-box" hidden>
        <div id="kt-preview-row">
          <figure>
            <img id="kt-still" alt="Note close-up" />
            <figcaption class="hint">Still: first three colors on approach</figcaption>
          </figure>
          <figure>
            <img id="kt-preview" alt="Gameplay preview" />
            <figcaption class="hint">Gameplay loop, real renderer</figcaption>
          </figure>
        </div>
      </div>
    </section>
    <aside id="kt-inspector">
      <div id="kt-inspector-body"></div>
      <h2>Colors</h2>
      <div id="kt-colors"></div>
      <button class="btn small ghost" id="kt-color-add">Add color</button>
    </aside>
  </main>
  <footer id="kt-status-bar">
    <span id="kt-status"></span>
    <span id="kt-triangles" class="hint"></span>
  </footer>
  <script type="module" src="kit/main.js"></script>
</body>
</html>
```

`crates/gui/ui/kit.css`:

```css
/* Note kit window. Inherits the app tokens from style.css. Three columns
   fill the window: parts, the front view (previews below it when shown),
   the inspector. */
#kit-body { overflow: hidden; display: flex; flex-direction: column; height: 100vh; margin: 0; }
#kt-toolbar { display: flex; gap: 0.75rem; align-items: center; padding: 0.5rem 1rem; border-bottom: 1px solid var(--border); flex: 0 0 auto; }
#kt-toolbar .group { display: flex; gap: 0.3rem; align-items: center; }
#kt-toolbar .group.grow { flex: 1 1 auto; }
#kt-toolbar .label { color: var(--muted); font-size: var(--fs-sm); margin-right: 0.2rem; }
#kt-previews.on { outline: 2px solid var(--accent); }
#kt-root { display: flex; gap: 1rem; padding: 0.75rem 1rem; flex: 1 1 auto; min-height: 0; box-sizing: border-box; }
#kt-parts-pane { flex: 0 0 240px; overflow-y: auto; display: flex; flex-direction: column; gap: 0.4rem; }
#kt-parts-pane h2, #kt-inspector h2 { font-size: var(--fs-md); margin: 0.2rem 0; color: var(--muted); }
#kt-parts { display: flex; flex-direction: column-reverse; gap: 0.2rem; }
.kt-part { display: flex; align-items: center; gap: 0.35rem; padding: 0.3rem 0.4rem; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--panel); cursor: grab; }
.kt-part.selected { outline: 2px solid var(--accent); }
.kt-part.hidden-part { opacity: 0.5; }
.kt-part.dragging { opacity: 0.4; }
.kt-part .name { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.kt-part .badge { font-size: var(--fs-sm); padding: 0 0.3rem; border-radius: var(--radius-sm); border: 1px solid var(--border); color: var(--muted); }
.kt-part .badge.cut { color: var(--danger); border-color: var(--danger); }
.kt-part .badge.glass { color: var(--accent); border-color: var(--accent); }
.kt-part button { opacity: 0.6; }
.kt-part button:hover { opacity: 1; }
#kt-centre { flex: 1 1 auto; min-width: 0; display: flex; flex-direction: column; gap: 0.5rem; }
#kt-canvas-box { flex: 1 1 60%; min-height: 0; position: relative; }
#kt-canvas { position: absolute; inset: 0; width: 100%; height: 100%; background: var(--panel); border: 1px solid var(--border); border-radius: var(--radius); cursor: default; touch-action: none; }
#kt-view-reset { position: absolute; top: 8px; right: 8px; z-index: 2; opacity: 0.75; }
#kt-canvas-hint { flex: 0 0 auto; }
#kt-splitter { flex: 0 0 8px; margin: -2px 0; cursor: ns-resize; border-radius: 4px; background: transparent; touch-action: none; }
#kt-splitter:hover { background: var(--accent-soft); }
#kt-preview-box { flex: 0 0 clamp(140px, 26vh, 340px); min-height: 120px; }
#kt-preview-row { display: flex; gap: 0.5rem; height: 100%; }
#kt-preview-row figure { flex: 1 1 50%; min-width: 0; margin: 0; display: flex; flex-direction: column; gap: 0.15rem; }
#kt-preview-row img { flex: 1 1 auto; min-height: 0; width: 100%; object-fit: contain; object-position: center; border: 1px solid var(--border); border-radius: var(--radius); background: #000; display: block; }
#kt-inspector { flex: 0 0 320px; overflow-y: auto; padding-right: 0.25rem; }
.kt-field { display: flex; align-items: center; gap: 0.4rem; margin: 0.25rem 0; }
.kt-field label { flex: 0 0 5.5rem; color: var(--muted); font-size: var(--fs-sm); }
.kt-field input[type="range"] { flex: 1 1 auto; }
.kt-field input[type="number"], .kt-field input[type="text"], .kt-field select { width: 5.5rem; }
.kt-field input[type="text"] { width: 100%; }
.kt-field .val { width: 3.2rem; text-align: right; font-variant-numeric: tabular-nums; color: var(--muted); font-size: var(--fs-sm); }
.kt-seg { display: flex; gap: 0; }
.kt-seg button { border-radius: 0; }
.kt-seg button:first-child { border-radius: var(--radius-sm) 0 0 var(--radius-sm); }
.kt-seg button:last-child { border-radius: 0 var(--radius-sm) var(--radius-sm) 0; }
.kt-seg button.on { background: var(--accent-soft); }
#kt-depth-bar { position: relative; height: 10px; background: var(--border); border-radius: 5px; margin: 0.3rem 0 0.5rem; }
#kt-depth-bar .band { position: absolute; top: 0; bottom: 0; background: var(--accent); border-radius: 5px; }
#kt-depth-bar .zero { position: absolute; top: -3px; bottom: -3px; width: 2px; background: var(--muted); }
#kt-colors { display: flex; flex-direction: column; gap: 0.25rem; margin-bottom: 0.4rem; }
.kt-color-row { display: flex; align-items: center; gap: 0.4rem; cursor: grab; }
.kt-color-row input[type="color"] { width: 2.2rem; height: 1.6rem; padding: 0; border: 1px solid var(--border); border-radius: var(--radius-sm); background: none; cursor: pointer; }
.kt-color-row .hex { width: 6rem; }
.kt-color-row.dragging { opacity: 0.4; }
#kt-status-bar { display: flex; justify-content: space-between; padding: 0.3rem 1rem; border-top: 1px solid var(--border); min-height: 1.4em; flex: 0 0 auto; }
#kt-status.err { color: var(--danger); }
```

`crates/gui/ui/kit/view.js`:

```js
// The front view: an orthographic Canvas 2D drawing of what the kernel
// produced (normalised, note units), each part's own outline, and the
// selected part's handles. The only place geometry is edited.
import {
  makeView, unit, toPx, toWorld, zoomAt, partRect, rectCorners, rectContains, handleAt, scaleFrom, angleFrom, HITBOX,
} from "./view-math.js";

const GLASS_ALPHA = 0.57;
const HANDLE_PX = 7;
const ROTATE_PX = 24;

export function createView(canvas, store, hooks) {
  const ctx = canvas.getContext("2d");
  const view = makeView();
  const size = { w: 520, h: 520 };
  let hatch = null;
  let spaceDown = false;
  let gesture = null;

  function fit() {
    const box = canvas.parentElement;
    const dpr = window.devicePixelRatio || 1;
    size.w = Math.max(200, box.clientWidth);
    size.h = Math.max(200, box.clientHeight);
    canvas.width = Math.round(size.w * dpr);
    canvas.height = Math.round(size.h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    draw();
  }

  function path(shapes) {
    const p = new Path2D();
    for (const shape of shapes) {
      for (const contour of shape) {
        contour.forEach((pt, i) => {
          const [x, y] = toPx(view, size, pt);
          if (i === 0) p.moveTo(x, y);
          else p.lineTo(x, y);
        });
        p.closePath();
      }
    }
    return p;
  }

  function hatchPattern() {
    if (hatch) return hatch;
    const c = document.createElement("canvas");
    c.width = 8;
    c.height = 8;
    const g = c.getContext("2d");
    g.strokeStyle = "rgba(255, 120, 90, 0.5)";
    g.lineWidth = 1.5;
    g.beginPath();
    g.moveTo(0, 8);
    g.lineTo(8, 0);
    g.stroke();
    hatch = ctx.createPattern(c, "repeat");
    return hatch;
  }

  function box() {
    const a = toPx(view, size, [-1, 1]);
    const b = toPx(view, size, [1, -1]);
    ctx.strokeStyle = "rgba(255,255,255,0.35)";
    ctx.lineWidth = 1;
    ctx.setLineDash([]);
    ctx.strokeRect(a[0], a[1], b[0] - a[0], b[1] - a[1]);
    const ha = toPx(view, size, [-HITBOX, HITBOX]);
    const hb = toPx(view, size, [HITBOX, -HITBOX]);
    ctx.setLineDash([4, 4]);
    ctx.strokeStyle = "rgba(255,255,255,0.18)";
    ctx.strokeRect(ha[0], ha[1], hb[0] - ha[0], hb[1] - ha[1]);
    ctx.setLineDash([]);
  }

  function draw() {
    const s = store.state;
    ctx.clearRect(0, 0, size.w, size.h);
    box();
    const ev = s.evaluated;
    const colour = (s.design.colors && s.design.colors[0]) || "#ffffff";
    if (ev) {
      // Outer contours run counterclockwise and holes clockwise, so the
      // nonzero rule cuts the holes without any extra bookkeeping.
      ctx.fillStyle = colour;
      for (const iv of ev.intervals) ctx.fill(path(iv.solid), "nonzero");
      ctx.globalAlpha = GLASS_ALPHA;
      for (const iv of ev.intervals) ctx.fill(path(iv.glass), "nonzero");
      ctx.globalAlpha = 1;
      for (const part of s.design.parts) {
        const geom = ev.parts.find((g) => g.id === part.id);
        if (!geom || !part.visible) continue;
        const p = path(geom.shapes);
        const sel = part.id === s.selected;
        if (part.mode === "cut") {
          ctx.fillStyle = hatchPattern();
          ctx.fill(p, "nonzero");
          ctx.setLineDash([5, 3]);
          ctx.strokeStyle = sel ? "rgba(255,140,110,0.95)" : "rgba(255,140,110,0.6)";
        } else {
          ctx.setLineDash([]);
          ctx.strokeStyle = sel ? "rgba(120,200,255,0.95)" : "rgba(255,255,255,0.25)";
        }
        ctx.lineWidth = sel ? 1.5 : 1;
        ctx.stroke(p);
        ctx.setLineDash([]);
      }
    }
    const part = store.part();
    if (part && ev) handles(part, ev.scale);
  }

  function handles(part, scale) {
    const r = partRect(part.transform, scale);
    const g = rectCorners(r);
    ctx.strokeStyle = "rgba(120,200,255,0.8)";
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 3]);
    ctx.beginPath();
    g.corners.forEach((c, i) => {
      const [x, y] = toPx(view, size, c);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.closePath();
    ctx.stroke();
    ctx.setLineDash([]);
    if (part.locked) return;
    ctx.fillStyle = "#fff";
    for (const c of [...g.corners, ...g.edges]) {
      const [x, y] = toPx(view, size, c);
      ctx.fillRect(x - 3.5, y - 3.5, 7, 7);
    }
    const [tx, ty] = toPx(view, size, g.top);
    const [cx, cy] = toPx(view, size, [r.cx, r.cy]);
    const len = Math.hypot(tx - cx, ty - cy) || 1;
    const rx = tx + ((tx - cx) / len) * ROTATE_PX;
    const ry = ty + ((ty - cy) / len) * ROTATE_PX;
    ctx.beginPath();
    ctx.moveTo(tx, ty);
    ctx.lineTo(rx, ry);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(rx, ry, 5, 0, Math.PI * 2);
    ctx.fill();
  }

  function rotateHandleHit(part, scale, mx, my) {
    const r = partRect(part.transform, scale);
    const g = rectCorners(r);
    const [tx, ty] = toPx(view, size, g.top);
    const [cx, cy] = toPx(view, size, [r.cx, r.cy]);
    const len = Math.hypot(tx - cx, ty - cy) || 1;
    const rx = tx + ((tx - cx) / len) * ROTATE_PX;
    const ry = ty + ((ty - cy) / len) * ROTATE_PX;
    return Math.hypot(mx - rx, my - ry) <= HANDLE_PX + 2;
  }

  function pick(wp) {
    // Top of the list draws on top, so it is picked first.
    const parts = store.state.design.parts;
    const scale = store.state.evaluated ? store.state.evaluated.scale : 1;
    for (let i = parts.length - 1; i >= 0; i--) {
      const p = parts[i];
      if (!p.visible) continue;
      if (rectContains(partRect(p.transform, scale), wp)) return p;
    }
    return null;
  }

  canvas.addEventListener("pointerdown", (e) => {
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    canvas.setPointerCapture(e.pointerId);
    if (e.button === 1 || spaceDown) {
      gesture = { kind: "pan", mx, my, cx: view.cx, cy: view.cy };
      return;
    }
    if (e.button !== 0) return;
    const wp = toWorld(view, size, mx, my);
    const s = store.state;
    const scale = s.evaluated ? s.evaluated.scale : 1;
    const sel = store.part();
    if (sel && !sel.locked && s.evaluated) {
      const r = partRect(sel.transform, scale);
      if (rotateHandleHit(sel, scale, mx, my)) {
        gesture = { kind: "rotate", id: sel.id };
        return;
      }
      const h = handleAt(r, wp, HANDLE_PX / unit(view, size));
      if (h) {
        gesture = { kind: "scale", id: sel.id, handle: h, r };
        return;
      }
    }
    const hit = pick(wp);
    if (!hit) {
      store.select(null);
      return;
    }
    if (hit.id !== s.selected) store.select(hit.id);
    if (!hit.locked) {
      gesture = { kind: "move", id: hit.id, wp, x: hit.transform.x, y: hit.transform.y, scale };
    }
  });

  canvas.addEventListener("pointermove", (e) => {
    if (!gesture || !canvas.hasPointerCapture(e.pointerId)) return;
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    if (gesture.kind === "pan") {
      const u = unit(view, size);
      view.cx = gesture.cx - (mx - gesture.mx) / u;
      view.cy = gesture.cy + (my - gesture.my) / u;
      draw();
      return;
    }
    const wp = toWorld(view, size, mx, my);
    const scale = store.state.evaluated ? store.state.evaluated.scale : 1;
    if (gesture.kind === "move") {
      const dx = (wp[0] - gesture.wp[0]) / gesture.scale;
      const dy = (wp[1] - gesture.wp[1]) / gesture.scale;
      store.updatePart(gesture.id, "move", (p) => {
        p.transform.x = clamp(gesture.x + dx, -1.5, 1.5);
        p.transform.y = clamp(gesture.y + dy, -1.5, 1.5);
      });
    } else if (gesture.kind === "scale") {
      const { hw, hh } = scaleFrom(gesture.r, gesture.handle, wp, e.shiftKey);
      store.updatePart(gesture.id, "scale", (p) => {
        p.transform.w = clamp(hw / scale, 0.02, 1.5);
        p.transform.h = clamp(hh / scale, 0.02, 1.5);
      });
    } else if (gesture.kind === "rotate") {
      const part = store.part(gesture.id);
      const r = partRect(part.transform, scale);
      let a = angleFrom(r, wp);
      if (e.shiftKey) a = Math.round(a / 15) * 15;
      a = ((((a + 180) % 360) + 360) % 360) - 180;
      store.updatePart(gesture.id, "rotate", (p) => (p.transform.rotation = Math.round(a * 10) / 10));
    }
    hooks.onEdit();
  });

  const end = (e) => {
    if (gesture && canvas.hasPointerCapture(e.pointerId)) canvas.releasePointerCapture(e.pointerId);
    gesture = null;
  };
  canvas.addEventListener("pointerup", end);
  canvas.addEventListener("pointercancel", end);

  canvas.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      zoomAt(view, size, e.clientX - rect.left, e.clientY - rect.top, e.deltaY < 0 ? 1.15 : 1 / 1.15);
      hooks.onZoom(Math.round(view.z * 100));
      draw();
    },
    { passive: false },
  );

  window.addEventListener("keydown", (e) => {
    if (e.code === "Space" && !isTyping()) {
      spaceDown = true;
      e.preventDefault();
    }
  });
  window.addEventListener("keyup", (e) => {
    if (e.code === "Space") spaceDown = false;
  });

  function resetView() {
    view.z = 1;
    view.cx = 0;
    view.cy = 0;
    hooks.onZoom(100);
    draw();
  }

  new ResizeObserver(fit).observe(canvas.parentElement);
  return { draw, fit, resetView };
}

export function isTyping() {
  const t = document.activeElement && document.activeElement.tagName;
  return t === "INPUT" || t === "SELECT" || t === "TEXTAREA";
}

const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
```

`crates/gui/ui/kit/main.js` (bootstrap for this task; Tasks 6 to 8 extend it):

```js
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
```

- [ ] **Step 4: Run the tests and look at the window**

Run: `node --test crates/gui/ui/kit/tests/` (expected: 11 passed), then `cargo tauri dev` (or the repo's usual dev command; see `docs/WINDOWS-BUILD.md` and the gui README for the dev invocation), open "Note Kit (beta)", add a star, move it, scale it by a corner, rotate it, zoom and pan. The silhouette follows the kernel; cut parts show hatched after Task 6 switches a mode. Close the dev app afterwards (one GPU process at a time on this machine).

- [ ] **Step 5: Commit**

```bash
git add crates/gui/ui/kit.html crates/gui/ui/kit.css crates/gui/ui/kit/ipc.js crates/gui/ui/kit/view.js crates/gui/ui/kit/main.js crates/gui/ui/kit/tests/ipc.test.mjs
git commit -m "Kit window: the front view draws the kernel's geometry and edits transforms with handles

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Parts list and inspector

**Files:**
- Create: `crates/gui/ui/kit/render.js`
- Modify: `crates/gui/ui/kit/main.js` (call `render(store)` on every state change; wire the colour editor)
- Test: `crates/gui/ui/kit/tests/render.test.mjs` for the pure helpers (`fmt`, `clampDepth`, `shapeFields`)

**Interfaces:**
- Produces: `render(store)` (rewrites `#kt-parts`, `#kt-inspector-body`, `#kt-colors`, toolbar states), `fmt(value, step)`, `clampDepth(front, back, which, value)`, `shapeFields(kind)`.

- [ ] **Step 1: Write the failing tests**

```js
// crates/gui/ui/kit/tests/render.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { fmt, clampDepth, shapeFields } from "../render.js";

test("numbers_show_as_many_decimals_as_their_step", () => {
  assert.equal(fmt(0.123456, 0.01), "0.12");
  assert.equal(fmt(6, 1), "6");
  assert.equal(fmt(-0.5, 0.05), "-0.50");
});

test("depth_keeps_front_above_back_inside_the_range", () => {
  // Pushing back up past front pushes front along, up to the range.
  assert.deepEqual(clampDepth(0.1, -0.1, "back", 0.3), { front: 0.35, back: 0.3 });
  assert.deepEqual(clampDepth(0.1, -0.1, "back", 0.49), { front: 0.5, back: 0.45 });
  assert.deepEqual(clampDepth(0.1, -0.1, "front", -0.5), { front: -0.5, back: -0.55 });
  assert.deepEqual(clampDepth(0.1, -0.1, "front", 9), { front: 0.5, back: -0.1 });
  assert.deepEqual(clampDepth(0.1, -0.1, "back", -9), { front: 0.1, back: -2 });
});

test("every_shape_kind_lists_its_own_fields", () => {
  assert.deepEqual(shapeFields("circle"), []);
  assert.equal(shapeFields("polygon").map((f) => f[0]).join(","), "sides,rounding");
  assert.equal(shapeFields("star")[1][0], "inner");
  assert.deepEqual(shapeFields("contours"), []);
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `node --test crates/gui/ui/kit/tests/`
Expected: failure, cannot find module `../render.js`.

- [ ] **Step 3: Implement the render layer**

```js
// crates/gui/ui/kit/render.js
// Panels are rewritten from state on every change. Nothing in the DOM
// is a source of truth; a focused input keeps its own text until it
// commits, everything else is replaced.
import { DEPTH_MIN, DEPTH_MAX, SHAPES } from "./store.js";

export const MIN_BAND = 0.05;

export function fmt(v, step) {
  const d = step >= 1 ? 0 : Math.max(0, Math.ceil(-Math.log10(step)));
  return Number(v).toFixed(d);
}

export function clampDepth(front, back, which, value) {
  const lo = DEPTH_MIN;
  const hi = DEPTH_MAX;
  let f = front;
  let b = back;
  if (which === "front") {
    f = Math.max(lo + MIN_BAND, Math.min(hi, value));
    if (b > f - MIN_BAND) b = f - MIN_BAND;
  } else {
    b = Math.max(lo, Math.min(hi - MIN_BAND, value));
    if (f < b + MIN_BAND) f = b + MIN_BAND;
  }
  const r = (x) => Math.round(x * 100) / 100;
  return { front: r(f), back: r(b) };
}

// [key, label, min, max, step]
const FIELDS = {
  circle: [],
  square: [["rounding", "Round corners", 0, 1, 0.01]],
  polygon: [["sides", "Sides", 3, 12, 1], ["rounding", "Round corners", 0, 1, 0.01]],
  star: [["points", "Points", 3, 12, 1], ["inner", "Inner size", 0.1, 0.9, 0.01]],
  ring: [["thickness", "Thickness", 0.05, 0.95, 0.01]],
  pill: [["aspect", "Roundness", 0.1, 1, 0.01]],
  contours: [],
  points: [],
};
export function shapeFields(kind) {
  return FIELDS[kind] || [];
}

const KIND_LABELS = { circle: "Disc", square: "Square", polygon: "Polygon", star: "Star", ring: "Ring", pill: "Pill", contours: "Outline (imported)", points: "Outline (freehand)" };

const $ = (id) => document.getElementById(id);

function el(tag, attrs = {}, children = []) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v;
    else if (k === "text") e.textContent = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (v !== null && v !== undefined) e.setAttribute(k, v);
  }
  for (const c of children) e.append(c);
  return e;
}

function slider(label, value, min, max, step, onInput, labelCommit) {
  const val = el("span", { class: "val", text: fmt(value, step) });
  const range = el("input", { type: "range", min, max, step, value });
  range.addEventListener("input", () => {
    val.textContent = fmt(range.value, step);
    onInput(Number(range.value), labelCommit);
  });
  return el("div", { class: "kt-field" }, [el("label", { text: label }), range, val]);
}

function number(label, value, min, max, step, onCommit) {
  const input = el("input", { type: "number", min, max, step, value: fmt(value, step) });
  const commit = () => {
    const v = Number(input.value);
    if (Number.isFinite(v)) onCommit(Math.max(min, Math.min(max, v)));
  };
  input.addEventListener("change", commit);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") commit();
  });
  return el("div", { class: "kt-field" }, [el("label", { text: label }), input]);
}

function segmented(label, options, current, onPick) {
  const seg = el("div", { class: "kt-seg" }, options.map(([v, text]) =>
    el("button", { class: `btn small${v === current ? " on" : ""}`, text, onclick: () => onPick(v) })));
  return el("div", { class: "kt-field" }, [el("label", { text: label }), seg]);
}

function renderParts(store) {
  const s = store.state;
  const root = $("kt-parts");
  root.replaceChildren();
  let dragId = null;
  for (const p of s.design.parts) {
    const row = el("div", { class: `kt-part${p.id === s.selected ? " selected" : ""}${p.visible ? "" : " hidden-part"}`, draggable: "true" });
    row.append(
      el("button", { class: "btn small ghost", title: p.visible ? "Hide" : "Show", text: p.visible ? "👁" : "◌",
        onclick: (e) => { e.stopPropagation(); store.updatePart(p.id, "visible", (x) => (x.visible = !x.visible)); } }),
      el("button", { class: "btn small ghost", title: p.locked ? "Unlock" : "Lock", text: p.locked ? "🔒" : "🔓",
        onclick: (e) => { e.stopPropagation(); store.updatePart(p.id, "lock", (x) => (x.locked = !x.locked)); } }),
      el("span", { class: "name", text: p.name }),
      el("span", { class: `badge${p.mode === "cut" ? " cut" : ""}`, text: p.mode === "cut" ? "Cut" : "Add" }),
      el("span", { class: `badge${p.material === "glass" ? " glass" : ""}`, text: p.material === "glass" ? "Glass" : "Solid" }),
    );
    row.addEventListener("click", () => store.select(p.id));
    row.addEventListener("dragstart", (e) => { dragId = p.id; row.classList.add("dragging"); e.dataTransfer.effectAllowed = "move"; });
    row.addEventListener("dragend", () => row.classList.remove("dragging"));
    row.addEventListener("dragover", (e) => e.preventDefault());
    row.addEventListener("drop", (e) => {
      e.preventDefault();
      if (!dragId || dragId === p.id) return;
      const to = s.design.parts.findIndex((x) => x.id === p.id);
      store.movePart(dragId, to);
      dragId = null;
    });
    root.append(row);
  }
}

function renderInspector(store) {
  const body = $("kt-inspector-body");
  const p = store.part();
  const focused = document.activeElement;
  if (focused && body.contains(focused) && focused.tagName === "INPUT" && focused.type !== "range") return;
  body.replaceChildren();
  if (!p) {
    body.append(el("div", { class: "hint", text: "Select a part, or add one from the toolbar." }));
    return;
  }
  const up = (label, patch) => store.updatePart(p.id, label, patch);
  body.append(el("h2", { text: "Part" }));
  const name = el("input", { type: "text", value: p.name });
  name.addEventListener("change", () => up("name", (x) => (x.name = name.value.trim() || x.name)));
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Name" }), name]));
  const kind = p.shape.kind;
  const kindSelect = el("select", {});
  for (const k of Object.keys(SHAPES)) kindSelect.append(el("option", { value: k, text: KIND_LABELS[k], selected: k === kind ? "" : null }));
  if (!(kind in SHAPES)) kindSelect.append(el("option", { value: kind, text: KIND_LABELS[kind] || kind, selected: "" }));
  kindSelect.addEventListener("change", () => up("shape", (x) => (x.shape = SHAPES[kindSelect.value]())));
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Shape" }), kindSelect]));
  for (const [key, label, min, max, step] of shapeFields(kind)) {
    body.append(slider(label, p.shape[key], min, max, step, (v) => up(`shape.${key}`, (x) => (x.shape[key] = v))));
  }
  body.append(segmented("Mode", [["add", "Add"], ["cut", "Cut"]], p.mode, (v) => up("mode", (x) => (x.mode = v))));
  body.append(segmented("Material", [["solid", "Solid"], ["glass", "Glass"]], p.material, (v) => up("material", (x) => (x.material = v))));
  body.append(el("h2", { text: "Position and size" }));
  const t = p.transform;
  body.append(number("X", t.x, -1.5, 1.5, 0.01, (v) => up("x", (x) => (x.transform.x = v))));
  body.append(number("Y", t.y, -1.5, 1.5, 0.01, (v) => up("y", (x) => (x.transform.y = v))));
  body.append(number("Width", t.w, 0.02, 1.5, 0.01, (v) => up("w", (x) => (x.transform.w = v))));
  body.append(number("Height", t.h, 0.02, 1.5, 0.01, (v) => up("h", (x) => (x.transform.h = v))));
  body.append(slider("Rotation", t.rotation, -180, 180, 1, (v) => up("rotation", (x) => (x.transform.rotation = v))));
  body.append(el("h2", { text: "Depth" }));
  const bar = el("div", { id: "kt-depth-bar" });
  const span = DEPTH_MAX - DEPTH_MIN;
  const pct = (z) => `${((z - DEPTH_MIN) / span) * 100}%`;
  bar.append(el("div", { class: "band", style: `left:${pct(p.depth.back)};right:${100 - ((p.depth.front - DEPTH_MIN) / span) * 100}%` }));
  bar.append(el("div", { class: "zero", style: `left:${pct(0)}` }));
  body.append(bar);
  body.append(number("Front", p.depth.front, DEPTH_MIN, DEPTH_MAX, 0.05, (v) => up("depth", (x) => (x.depth = clampDepth(x.depth.front, x.depth.back, "front", v)))));
  body.append(number("Back", p.depth.back, DEPTH_MIN, DEPTH_MAX, 0.05, (v) => up("depth", (x) => (x.depth = clampDepth(x.depth.front, x.depth.back, "back", v)))));
  body.append(el("div", { class: "hint", text: "Front faces the player. The game's own disc spans 0.1 to -0.1." }));
  body.append(el("div", { class: "kt-field" }, [
    el("button", { class: "btn small ghost", text: "Duplicate", onclick: () => store.duplicatePart(p.id) }),
    el("button", { class: "btn small ghost", text: "Remove", onclick: () => store.removePart(p.id) }),
  ]));
}

function renderColors(store) {
  const s = store.state;
  const root = $("kt-colors");
  root.replaceChildren();
  let dragIdx = -1;
  s.design.colors.forEach((hex, i) => {
    const row = el("div", { class: "kt-color-row", draggable: "true" });
    const pick = el("input", { type: "color", value: hex });
    const text = el("input", { type: "text", class: "hex", value: hex });
    const set = (v) => {
      if (!/^#[0-9a-fA-F]{6}$/.test(v)) return;
      store.commit("color", (d) => (d.colors[i] = v.toLowerCase()));
    };
    pick.addEventListener("input", () => set(pick.value));
    text.addEventListener("change", () => set(text.value.trim()));
    row.append(el("span", { class: "grip", text: "⋮⋮" }), pick, text,
      el("button", { class: "btn small ghost", text: "×", title: "Remove color",
        onclick: () => { if (s.design.colors.length > 1) store.commit("color", (d) => d.colors.splice(i, 1)); } }));
    row.addEventListener("dragstart", () => { dragIdx = i; row.classList.add("dragging"); });
    row.addEventListener("dragend", () => row.classList.remove("dragging"));
    row.addEventListener("dragover", (e) => e.preventDefault());
    row.addEventListener("drop", (e) => {
      e.preventDefault();
      if (dragIdx < 0 || dragIdx === i) return;
      store.commit("color", (d) => { const [c] = d.colors.splice(dragIdx, 1); d.colors.splice(i, 0, c); });
    });
    root.append(row);
  });
}

export function render(store) {
  const s = store.state;
  renderParts(store);
  renderInspector(store);
  renderColors(store);
  $("kt-undo").disabled = s.undo.length === 0;
  $("kt-redo").disabled = s.redo.length === 0;
  $("kt-previews").classList.toggle("on", s.ui.previews);
  document.title = `rhythr · Note Kit${s.dirty ? " *" : ""}${s.path ? ` · ${s.path.split(/[\\/]/).pop()}` : ""}`;
}
```

In `main.js`: `import { render } from "./render.js";` and in the subscriber call `render(store)` before `view.draw()`; wire `$("kt-color-add")` to `store.commit("color", (d) => d.colors.push("#a2e0ff"))`.

The colour editor and the drag reorder follow the old designer's `renderColors` (designer.js:1127-1200) in behaviour; the vocabulary of every label is the spec's.

- [ ] **Step 4: Run the tests and check the window**

Run: `node --test crates/gui/ui/kit/tests/` (expected: 14 passed), then the dev app: add two parts, switch one to Cut (hatched in the view, badge in the list), to Glass (translucent fill), reorder by drag, hide and lock, change depth (bar follows, front stays above back), rename, duplicate, remove, undo through all of it.

- [ ] **Step 5: Commit**

```bash
git add crates/gui/ui/kit/render.js crates/gui/ui/kit/main.js crates/gui/ui/kit/tests/render.test.mjs
git commit -m "Kit window: parts list and inspector rewritten from state

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: Previews with a toggle

**Files:**
- Create: `crates/gui/ui/kit/preview.js`
- Modify: `crates/gui/ui/kit/main.js` (wire the toggle button, the P key, the preview updates)
- Test: `crates/gui/ui/kit/tests/preview.test.mjs` (the scheduler with a fake clock and a fake invoke; no DOM)

**Interfaces:**
- Consumes: `kit_preview(design, timeMs, still)` (Task 2).
- Produces: `createPreviewScheduler({ invoke, setTimer, clearTimer, now, onStill, onFrame, onError })` with `setEnabled(bool)`, `setVisible(bool)`, `designChanged(design)`, `tick()`; `mountPreviews(store, els)` wiring the DOM (toggle, splitter, images).

- [ ] **Step 1: Write the failing test**

```js
// crates/gui/ui/kit/tests/preview.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { createPreviewScheduler, STILL_DELAY_MS } from "../preview.js";

function harness() {
  const calls = [];
  const timers = [];
  const h = {
    calls,
    timers,
    sched: null,
    fire() {
      const t = timers.splice(0);
      for (const fn of t) fn();
    },
  };
  h.sched = createPreviewScheduler({
    invoke: async (name, args) => {
      calls.push([name, args.still ? "still" : "loop", args.design.name]);
      return "data:image/png;base64,AAAA";
    },
    setTimer: (fn) => {
      timers.push(fn);
      return timers.length;
    },
    clearTimer: () => {},
    now: () => 0,
    onStill: () => {},
    onFrame: () => {},
    onError: () => {},
  });
  return h;
}

test("previews_off_makes_no_calls", async () => {
  const h = harness();
  h.sched.setEnabled(false);
  h.sched.designChanged({ name: "a" });
  h.fire();
  await h.sched.tick();
  assert.deepEqual(h.calls, []);
});

test("previews_on_render_the_still_after_the_delay_and_frames_on_tick", async () => {
  const h = harness();
  h.sched.setEnabled(true);
  h.sched.setVisible(true);
  h.sched.designChanged({ name: "a" });
  assert.deepEqual(h.calls, []);
  h.fire();
  await Promise.resolve();
  assert.deepEqual(h.calls, [["kit_preview", "still", "a"]]);
  await h.sched.tick();
  assert.deepEqual(h.calls[1], ["kit_preview", "loop", "a"]);
  h.sched.setEnabled(false);
  await h.sched.tick();
  assert.equal(h.calls.length, 2, "off stops the loop at once");
  h.sched.setEnabled(true);
  await h.sched.tick();
  assert.equal(h.calls.length, 3, "on resumes with the current design");
  assert.ok(STILL_DELAY_MS > 0);
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test crates/gui/ui/kit/tests/`
Expected: failure, cannot find module `../preview.js`.

- [ ] **Step 3: Implement**

```js
// crates/gui/ui/kit/preview.js
// The previews are optional: hidden, they cost nothing (no calls, no
// loop); shown, the still re-renders shortly after the last change and
// the gameplay loop runs through the real renderer at about 30 fps.
export const STILL_DELAY_MS = 250;
export const LOOP_MS = 2700;
export const FRAME_MS = 33;
export const PREVIEWS_KEY = "rhythr.kit.previews";
export const PREVIEW_H_KEY = "rhythr.kit.previewH";

export function createPreviewScheduler(io) {
  let enabled = false;
  let visible = true;
  let design = null;
  let stillTimer = null;
  let stillBusy = false;
  let stillAgain = false;
  let frameBusy = false;
  const t0 = io.now();

  async function still() {
    if (!enabled || !design) return;
    if (stillBusy) {
      stillAgain = true;
      return;
    }
    stillBusy = true;
    try {
      io.onStill(await io.invoke("kit_preview", { design, timeMs: 0, still: true }));
    } catch (e) {
      io.onError(String(e));
    } finally {
      stillBusy = false;
      if (stillAgain) {
        stillAgain = false;
        still();
      }
    }
  }

  return {
    setEnabled(on) {
      enabled = on;
      if (on && design) {
        io.clearTimer(stillTimer);
        stillTimer = io.setTimer(still, STILL_DELAY_MS);
      }
    },
    setVisible(v) {
      visible = v;
    },
    designChanged(d) {
      design = d;
      if (!enabled) return;
      io.clearTimer(stillTimer);
      stillTimer = io.setTimer(still, STILL_DELAY_MS);
    },
    // One gameplay frame; the caller paces the calls.
    async tick() {
      if (!enabled || !visible || !design || frameBusy) return;
      frameBusy = true;
      try {
        const timeMs = (io.now() - t0) % LOOP_MS;
        io.onFrame(await io.invoke("kit_preview", { design, timeMs, still: false }));
      } catch (e) {
        io.onError(String(e));
      } finally {
        frameBusy = false;
      }
    },
  };
}

export function mountPreviews(store, els, invoke) {
  const sched = createPreviewScheduler({
    invoke,
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (t) => clearTimeout(t),
    now: () => performance.now(),
    onStill: (url) => (els.still.src = url),
    onFrame: (url) => (els.loop.src = url),
    onError: (msg) => store.setStatus(msg, true),
  });
  let saved = false;
  try {
    saved = localStorage.getItem(PREVIEWS_KEY) === "1";
  } catch {
    saved = false;
  }
  function apply(on) {
    store.setUi({ previews: on });
    els.box.hidden = !on;
    els.splitter.hidden = !on;
    sched.setEnabled(on);
    try {
      localStorage.setItem(PREVIEWS_KEY, on ? "1" : "0");
    } catch {
      // Best effort: the toggle still works for this session.
    }
  }
  els.toggle.addEventListener("click", () => apply(!store.state.ui.previews));
  document.addEventListener("visibilitychange", () => sched.setVisible(!document.hidden));
  (async function loop() {
    while (true) {
      const t = performance.now();
      await sched.tick();
      const took = performance.now() - t;
      await new Promise((r) => setTimeout(r, Math.max(FRAME_MS - took, store.state.ui.previews ? 1 : 150)));
    }
  })();
  // The splitter from the old designer, same stored height.
  const bar = els.splitter;
  const box = els.box;
  try {
    const h = Number(localStorage.getItem(PREVIEW_H_KEY));
    if (h >= 120 && h <= window.innerHeight * 0.7) box.style.flexBasis = `${h}px`;
  } catch {
    // Default balance.
  }
  let startY = 0;
  let startH = 0;
  bar.addEventListener("pointerdown", (e) => {
    startY = e.clientY;
    startH = box.getBoundingClientRect().height;
    bar.setPointerCapture(e.pointerId);
  });
  bar.addEventListener("pointermove", (e) => {
    if (!bar.hasPointerCapture(e.pointerId)) return;
    box.style.flexBasis = `${Math.max(120, Math.min(window.innerHeight * 0.7, startH - (e.clientY - startY)))}px`;
  });
  bar.addEventListener("pointerup", (e) => {
    if (!bar.hasPointerCapture(e.pointerId)) return;
    bar.releasePointerCapture(e.pointerId);
    try {
      localStorage.setItem(PREVIEW_H_KEY, String(Math.round(box.getBoundingClientRect().height)));
    } catch {
      // Best effort.
    }
  });
  apply(saved);
  return { apply, designChanged: (d) => sched.designChanged(d), toggle: () => apply(!store.state.ui.previews) };
}
```

In `main.js`: `import { mountPreviews } from "./preview.js";`, after the view is created: `const previews = mountPreviews(store, { toggle: $("kt-previews"), box: $("kt-preview-box"), splitter: $("kt-splitter"), still: $("kt-still"), loop: $("kt-preview") }, invoke);` and in the subscriber, where `serial !== lastSerial`, also `previews.designChanged(s.design)`. The P key (Task 8's keyboard map) calls `previews.toggle()`.

- [ ] **Step 4: Run the tests and check the window**

Run: `node --test crates/gui/ui/kit/tests/` (expected: 16 passed). Dev app: the window opens without previews; press the button: the pane appears, the still arrives, the loop runs; drag the splitter; close and reopen the window: the state is remembered; switch off: the frames stop (watch `top`, the process idles).

- [ ] **Step 5: Commit**

```bash
git add crates/gui/ui/kit/preview.js crates/gui/ui/kit/main.js crates/gui/ui/kit/tests/preview.test.mjs
git commit -m "Kit window: previews behind a toggle, off by default and remembered

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Files, autosave, migration, export, keyboard

**Files:**
- Create: `crates/gui/ui/kit/files.js`
- Modify: `crates/gui/ui/kit/main.js` (wire the file buttons, autosave on change, keyboard shortcuts, the startup document)
- Test: `crates/gui/ui/kit/tests/files.test.mjs` (recent list and autosave helpers with a fake storage; the startup choice)

**Interfaces:**
- Consumes: `kit_doc_load`, `kit_doc_save`, `kit_export`, `kit_migrate_v1`, `kit_default_doc` (Task 1); the dialog plugin exactly as `designer.js` uses it (read its save and load handlers in `wireControls`, designer.js:905-1125, and copy the calls: the `save` dialog with a `.rhsd` filter for documents and a `.rhs` filter for exports, the `open` dialog for documents).
- Produces: `makeRecent(storage)` with `list()`, `add(path)`; `autosave(storage, serialize)` debounced; `startupDesign(storage, invoke)`; `importOldDesigner(storage, invoke)`; `bindKeys(store, actions)`.

- [ ] **Step 1: Write the failing tests**

```js
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `node --test crates/gui/ui/kit/tests/`
Expected: failure, cannot find module `../files.js`.

- [ ] **Step 3: Implement**

```js
// crates/gui/ui/kit/files.js
// Documents: autosave in the webview's storage so closing never loses
// work, a recent list, save and open through the dialog plugin, the old
// designer's autosave as an import, and the .rhs export.
export const DOC_KEY = "rhythr.kit.doc";
export const RECENT_KEY = "rhythr.kit.recent";
export const OLD_DOC_KEY = "rhythr.designer.doc";
export const RECENT_LIMIT = 6;
export const AUTOSAVE_MS = 600;

export function makeRecent(storage) {
  const read = () => {
    try {
      const v = JSON.parse(storage.getItem(RECENT_KEY) || "[]");
      return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
    } catch {
      return [];
    }
  };
  return {
    list: read,
    add(path) {
      const l = [path, ...read().filter((p) => p !== path)].slice(0, RECENT_LIMIT);
      try {
        storage.setItem(RECENT_KEY, JSON.stringify(l));
      } catch {
        // Best effort.
      }
      return l;
    },
  };
}

export function autosave(storage, serialize) {
  let timer = null;
  return () => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      try {
        storage.setItem(DOC_KEY, serialize());
      } catch {
        // Storage full or unavailable: autosave is best effort.
      }
    }, AUTOSAVE_MS);
  };
}

function parseDesign(text) {
  const d = JSON.parse(text);
  if (!d || d.version !== 2 || !Array.isArray(d.parts)) throw new Error("not a kit design");
  if (!Array.isArray(d.groups)) d.groups = [];
  if (!Array.isArray(d.colors) || !d.colors.length) d.colors = ["#ffffff"];
  return d;
}

export async function startupDesign(storage, invoke) {
  try {
    const saved = storage.getItem(DOC_KEY);
    if (saved) return parseDesign(saved);
  } catch {
    // A damaged autosave is not worth a dialog; start fresh.
  }
  return parseDesign(await invoke("kit_default_doc"));
}

export async function importOldDesigner(storage, invoke) {
  const old = storage.getItem(OLD_DOC_KEY);
  if (!old) throw new Error("nothing saved in the old Note Designer on this machine");
  return parseDesign(await invoke("kit_migrate_v1", { json: old }));
}

export function bindKeys(store, actions, isTyping) {
  window.addEventListener("keydown", (e) => {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "z" && !e.shiftKey) { e.preventDefault(); store.undo(); return; }
    if (mod && (e.key.toLowerCase() === "y" || (e.key.toLowerCase() === "z" && e.shiftKey))) { e.preventDefault(); store.redo(); return; }
    if (mod && e.key.toLowerCase() === "s") { e.preventDefault(); actions.save(); return; }
    if (mod && e.key.toLowerCase() === "o") { e.preventDefault(); actions.open(); return; }
    if (isTyping()) return;
    const p = store.part();
    if (e.key === "p" || e.key === "P") { actions.togglePreviews(); return; }
    if (e.key === "Escape") { store.select(null); return; }
    if (!p) return;
    if (e.key === "Delete" || e.key === "Backspace") { store.removePart(p.id); return; }
    if (mod && e.key.toLowerCase() === "d") { e.preventDefault(); store.duplicatePart(p.id); return; }
    const step = e.shiftKey ? 0.1 : 0.01;
    const nudge = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] }[e.key];
    if (nudge && !p.locked) {
      e.preventDefault();
      store.updatePart(p.id, "nudge", (x) => {
        x.transform.x = Math.max(-1.5, Math.min(1.5, Math.round((x.transform.x + nudge[0]) * 1000) / 1000));
        x.transform.y = Math.max(-1.5, Math.min(1.5, Math.round((x.transform.y + nudge[1]) * 1000) / 1000));
      });
    }
  });
}
```

In `main.js`:
- startup: `const design = await startupDesign(localStorage, invoke);` instead of the default call;
- autosave: `const save = autosave(localStorage, () => store.serialize());` called in the subscriber when the serial changed;
- buttons: New (confirm when dirty via `window.confirm` is a blocking dialog: use the dialog plugin's `ask` as designer.js does, or skip the confirmation and rely on undo; choose the plugin's `ask`), Open (dialog `open` with the `.rhsd` filter, then `kit_doc_load`, `store.load(parseDesign(json), path)`, `recent.add(path)`), Save (if `store.state.path` write there, else dialog `save` with the `.rhsd` filter; `kit_doc_save(path, store.serialize())`, then `store.state.dirty = false`, `recent.add`), From old designer (`importOldDesigner` then `store.load(design)` with `path` null), Export (dialog `save` with the `.rhs` filter and the default name `${store.state.design.name}.rhs`; `kit_export`; status "Exported to <path>");
- a recent-list `<select id="kt-recent">` under the toolbar's file group (add it to `kit.html`) filled from `recent.list()` on every render, choosing an entry opens it;
- `bindKeys(store, { save, open, togglePreviews: previews.toggle }, isTyping)`.

- [ ] **Step 4: Run the tests and check the window**

Run: `node --test crates/gui/ui/kit/tests/` (expected: 19 passed). Dev app: edit, close the window, reopen: the design is back; Save to a file, New, Open it; Export a .rhs and load it in rhythr's Config slot (the main window) to see the note; "From old designer" after saving something in the old Note Designer window: the migrated part appears and evaluates (a freeform design with symmetry 4 comes in as one outline, not four pieces); every shortcut.

- [ ] **Step 5: Commit**

```bash
git add crates/gui/ui/kit/files.js crates/gui/ui/kit/main.js crates/gui/ui/kit.html crates/gui/ui/kit/tests/files.test.mjs
git commit -m "Kit window: autosave, files, the old designer's note as an import, export, shortcuts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Checks, guide, testbuild

**Files:**
- Modify: `docs/USER-GUIDE.md` (one paragraph after the Note Designer bullet: the Note Kit is in beta, what it does today, that the old designer stays until the kit is complete, the previews toggle, import from a local folder)
- No code changes beyond fixes the checks demand.

- [ ] **Step 1: Full checks on Linux**

Run: `cargo fmt --all && cargo test && node --test crates/gui/ui/kit/tests/ && cargo clippy -p rhythia-gui && scripts/preflight.sh`
Expected: all green, no new clippy findings in `kit_commands.rs` or the touched parts of `main.rs`, no dashes in the new JavaScript, HTML and CSS (preflight scans them).

- [ ] **Step 2: Windows cross-check**

Run: `cargo xwin check -p rhythia-gui --target x86_64-pc-windows-msvc` (the worktree needs `crates/gui/ffmpeg.exe` and `ffmpeg-LICENSE.txt`; symlink them from the main checkout if missing, they are gitignored).
Expected: compiles.

- [ ] **Step 3: The testbuild**

Run: `tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc` as `docs/WINDOWS-BUILD.md` describes (the version stays 0.6.2; this is a testbuild, not a release) and copy the installer to `/home/killer/RhythiaRenderer/rhythr_0.6.2_x64-setup_TESTBUILD-kit1.exe`. Then `scripts/build-linux.sh` only if the owner asks for a Linux package.
Expected: the installer exists; the owner tests the kit window on Windows (add parts, cut, glass, depth, previews toggle, save and open, export and import into the game) and reports.

- [ ] **Step 4: Commit**

```bash
git add docs/USER-GUIDE.md
git commit -m "Kit phase 2 closes: the editor core, guide note, testbuild kit1

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Self-review notes

- Spec coverage for phase 2 (spec section 8): store and render layer (Tasks 3, 6), parts list (6), inspector (6), front view with transforms and selection (4, 5), previews (7, with the owner's toggle), undo (3), autosave and files (8), v1 migration (1, 8), testbuild (9). Deferred to phase 3 by the spec: groups, effect chips, symmetry controls, point edit, snapping, turntable; phase 4: presets beyond the quick shapes, imports as parts, removal of the old designer.
- The owner's decisions are in: depth range [-2.0, 0.5] (`DEPTH_MIN`, `DEPTH_MAX`), the slider mode as quick-shape buttons only (the migration turns an old design into one part with the same shape), new parts at [-0.1, 0.1] (`NEW_DEPTH`), previews off by default and remembered (`PREVIEWS_KEY`).
- Type consistency: `Evaluated` field names (`scale`, `levels`, `parts[].id`, `parts[].shapes`, `intervals[].solid`, `intervals[].glass`, `triangles`, `mesh_error`) are used by `view.js` and `main.js` with exactly those spellings; the document spelling in `store.js` matches serde's (`kind`, `mode`, `material`, `effects` keys, `mirror: "none"`).
- The spec's "headless UI checks (the v2 scenario harness)" does not exist in the repo; this plan tests the pure modules under Node (store, view maths, IPC coalescing, preview scheduling, files) and leaves DOM behaviour to the dev-app checks and the testbuild. Phase 3 can add a DOM harness if the owner wants one.
- Review Focus coverage: the coalesced undo (Task 3), the error keeping the last geometry (Tasks 1 and 5), rotated hit tests and handles (Task 4), migration end to end (Tasks 1 and 8), previews off making no calls (Task 7).
- Known phase 3 input from phase 1's ledger: rhythr's obj loader keys the glass alpha on the first `usemtl`, so a glass-only design previews opaque in rhythr while the game draws it translucent (probe kit4); the loader should read the material order from the bundled .mtl. Not in this plan; the kit preview shows the kernel's obj, which puts `Material.001` first whenever any solid exists.

## Execution amendments (24.10., evening; rulings recorded in the ledger `docs/superpowers/plans/2026-10-24-note-kit-phase2-ledger.md`)

The code as committed differs from the task text above in these points; the ledger carries the reason for each.

- Node test command: `node --test crates/gui/ui/kit/tests/*.test.mjs` (Node 22 treats a directory argument as a module path).
- Task 1: `user_message` replaces any `Invalid` or `Doc` text that contains a banned word (boolean, union, mesh, vertex, modifier, manifold, lattice, triangulation, degenerate, edge, orientation, index, triangle, surface, zip) with "this design could not be built; try fewer parts, less rounding or fewer waves" and logs the raw text to stderr; the user-actionable kernel texts pass through unchanged. `load_document` refuses JSON with neither `"version": 2` nor a `"P"` key as "not a design file".
- Task 2: the designer window's destroy handler locks the renderer first and clears both skins while holding it. `crates/gui/build.rs` walks `ui/` recursively through `build_support.rs` (`copy_tree`, skips any `tests` directory), included under `cfg(test)` by `main.rs` so its test runs with the crate. `"kit"` is in `crates/gui/capabilities/default.json`. `open_kit_window` sets `.disable_drag_drop_handler()` (HTML5 drag reorder on Windows).
- Task 3: a module-level `DISCRETE` set of labels never coalesces: `add`, `remove`, `duplicate`, `reorder`, `visible`, `lock`, `mode`, `material`, `shape`, `name`, `color-add`, `color-remove`, `color-reorder`. Gesture and field labels carry the part id (`move:p1`, `scale:p1`, `rotate:p1`, `nudge:p1`, `x:p1`, `shape.rounding:p1`, `depth:p1`); the colour picker uses `color:<index>`.
- Task 5: Shift keeps aspect on corner handles only; `fit` and the wheel skip a zero-size box; pointer capture is released on every pointerup; the scale and rotate gestures use the normalisation factor captured at pointerdown; glass from all intervals fills once; the context menu is suppressed; the bitmap is sized from the canvas's client box and pointer maths subtract `clientLeft` and `clientTop`; the wheel ignores `deltaY` 0 and zooms by `exp(-deltaY * 0.0015)` clamped to [0.5, 2]; `spaceDown` resets on blur; the evaluate result overwrites the status only when it carries `mesh_error` or when the status was written by an evaluate; `lastSerial` starts as the startup serial.
- Task 6: panels rebuild only on a structural change (inspector: selected id, shape kind, part presence; parts list: the id order; colours: the count) and otherwise patch every widget in place, including a focused field whose text equals what it last showed (`dataset.shown`); number commits round to the field's step (`roundToStep`) and skip when unchanged (depth after `clampDepth`); the × on the last colour is disabled; a kind-select rebuild restores focus.
- Task 7: `previews.designChanged` also runs once after the startup evaluate; `mountPreviews` de-duplicates a repeated preview error and clears it on the next good still or frame.
- Task 8: `files.js` exports `isTextEntry` and `ownsArrows`; `bindKeys(store, actions, activeElement)` lets Ctrl+S and Ctrl+O work while typing (they blur the field first, which commits it), guards undo, redo, P, Escape, Delete, Backspace and Ctrl+D with `isTextEntry`, and the arrows with `ownsArrows`; Delete and Backspace ignore the modifier; a nudge at the clamp does not commit; `autosave()` returns a function with `.flush()` called on pagehide and when hidden; Open, recent and the old-designer import also ask before replacing a dirty design; a restored autosave counts as dirty; file actions run one at a time; the recent select is `kt-recent` and blurs after a choice; the toolbar wraps.
- Task 9: the testbuild is built after the final whole-branch review and its fix wave, not before.

## Owner test round 1 (24.10., evening, after testbuild kit1)

The owner found that moving a part changed its size, that moving changed proportions, and that hiding one part "focused" the other. Cause: the view drew the kernel's normalised geometry live (the union of Add parts scaled to the unit box), so moving the largest part shrank everything and hiding a part rescaled the rest. Changes (controller, hands-on in a browser harness against the real kernel):

- `kit/units.js` (new): `toDesignUnits(ev)` divides every point of `parts[].shapes` and `intervals[].solid/glass` by `ev.scale` (a factor of 1, 0, NaN or negative returns the payload as is); `boxExtent(ev)` is `1 / scale`. `main.js` converts every evaluate result before `store.setEvaluated`.
- `view.js` edits in design units: `partRect(t, 1)` everywhere, gestures use world deltas directly (no factor at pointerdown), the solid square is the note box at ±`boxExtent` and the dashed hit area at ±`boxExtent · HITBOX`, so the box follows the largest part while parts keep their size. The hint under the canvas says so.
- A hidden part stays selected but shows no handles and takes no handle gesture. On an evaluate error the stale fills and hatches are not drawn (outlines only), so hiding every part empties the canvas; `main.js` also clears the triangle count on an error.
- `view-math.js`: `MAX_HALF = 1.5`; `scaleFrom(r, handle, p, keepAspect, maxHalf = MAX_HALF)` caps the common factor with Shift held, so a corner drag at the size limit keeps the proportions instead of one side stopping while the other grows; both results are capped.
- The zoom button now fits the design: `resetView` sets `z = clamp(1 / boxExtent, 0.4, 12)` (the default design still reads 100%) and the title reads "Fit the design: reset zoom and pan".
- Handles have a dark rim so they stay visible over a white fill.
- Harness: `crates/render/examples/kit_eval_json.rs` prints the `kit_evaluate` payload for a design on stdin (`--default` prints the default design); `crates/gui/ui/kit/tests/harness.mjs` serves `crates/gui/ui/` with a `window.__TAURI__` shim (`/invoke/<name>` runs the example for `kit_evaluate`, real files for the documents, canned preview and export, `window.__harness.dialog` for the dialogs). Scenarios played through with Playwright (one page, closed afterwards): move, corner and edge scale, rotated scale, Shift scale at the cap, rotate (centred and off-centre), three undos and a redo, deselect by empty click, lock, nudge with Shift, numeric fields with clamp, Escape, Ctrl+D, Delete, cut hatch, glass, depth fields pushing each other, shape change, list reorder by drag, colour add, remove and hex edit, wheel zoom, middle-button and Space pan, fit, a half-size design (factor 2, box shrinks), everything hidden (canvas empty, error on the status line), click-drag on an unselected part, previews on and off with P, Save, New with and without discard, Open, recent, Export, autosave restore.
