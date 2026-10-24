# Note Kit Phase 1 (Kernel) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A pure-Rust kernel that turns a multi-part note design into a validated, watertight 2.5D mesh with solid and glass material ranges, exports it as the .rhs the game loads, and teaches rhythr's renderer to draw glass the way the game does.

**Architecture:** The design document (parts with 2D shapes, add/cut modes, depth bands, materials, effects) is resolved into oriented 2D regions per depth interval using i_overlay's fixed-scale integer kernel. The plane is subdivided into faces of constant membership across all intervals; caps are triangulated once per face and lifted to each level, walls are emitted on face edges where membership differs, so every vertex is shared by construction and no T-junction can exist. An in-house validator gates every export.

**Tech Stack:** Rust (workspace crate `rhythia-render`), i_overlay 8.1 (fixed-scale overlay and outline), i_triangle 0.47 (unchecked triangulation), kurbo 0.13 (spline flattening, already used), serde/serde_json, zip (already used), proptest 1 (new dev-dependency, pure Rust, MIT OR Apache-2.0), wgpu 30 + WGSL for the renderer change.

**Spec:** `docs/superpowers/specs/2026-10-24-note-kit-2-5d-design.md`

## Global Constraints

- Pure Rust only: no crate with a C or C++ build step (the Windows installer is cross-built with `cargo-xwin` for `x86_64-pc-windows-msvc`, the Linux packages inside an Ubuntu 22.04 container with glibc 2.35). `proptest` is the only new dependency and is pure Rust.
- Licenses: MIT or Apache-2.0 compatible only.
- Nothing platform-specific: no `std::os::unix`, no absolute paths, no `\r\n` assumptions when reading; exported text keeps the conventions the game's own files use (obj and mtl with `\n`, colorset lines with `\r\n`, as `designer::export_rhs` already does).
- Determinism: all boolean and offset operations run through `KIT_SCALE = 65536.0` with i_overlay's fixed-scale API; tests compare float results with tolerances (trigonometry may differ by an ulp between the MSVC CRT and glibc), and determinism is asserted within one run.
- House style: no em or en dashes anywhere (`scripts/preflight.sh` fails on them), comments state constraints not narration, commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Size rule from the spec: the union of all Add parts is normalised to the unit box (x and y half-extent 1.0); depth bands live in `[-2.0, 0.5]`; a new part defaults to `[-0.1, 0.1]`.
- Glass: any material after the first draws translucent in the game at a fixed alpha; rhythr uses `GLASS_ALPHA = 0.57` (measured from probe 7a).
- Budget: refuse exports above `MAX_TRIANGLES = 40_000`.
- The v1 designer (`crates/render/src/designer.rs`, the `designer_*` commands) keeps working untouched in this phase; the kit lives in `crates/render/src/kit/`.
- Run `cargo fmt --all`, `cargo test -p rhythia-render` and `scripts/preflight.sh` before every commit named below.

## Review Focus

Inputs the spec implies but which the happy path would not exercise. Each line names the task whose tests pin it.

1. Two parts whose boundaries cross at different depth bands (a cut bar through a ring at a shallower band): the mesh must stay watertight despite boundary crossings. Pinned in Task 8 (`crossing_bands_stay_watertight`).
2. A part scaled or placed so the silhouette leaves the unit box: the export must still be inside the box, never clipped, never larger. Pinned in Task 6 (`silhouette_is_normalised_to_the_box`) and Task 11 (property).
3. A cut that removes everything, or a design with only cut parts or only hidden parts: a clear `KitError::Empty`, never a panic or an empty .obj. Pinned in Task 6 (`all_cut_or_hidden_is_an_error`).
4. Depth bands that touch or overlap at the same level (front of one equals back of another): no zero-height interval, no duplicate level, no sliver wall. Pinned in Task 6 (`touching_bands_share_one_level`) and Task 8.
5. A v1 document in every mode (sliders, freeform with symmetry and mirror, import): migration must produce the same silhouette v1 produced. Pinned in Task 2.

---

### Task 1: Kit module and document model

**Files:**
- Create: `crates/render/src/kit/mod.rs`
- Create: `crates/render/src/kit/doc.rs`
- Modify: `crates/render/src/lib.rs` (add `pub mod kit;` after `pub mod designer;`)
- Modify: `crates/render/Cargo.toml` (dev-dependency `proptest = "1"`)
- Test: inline `#[cfg(test)] mod tests` in `doc.rs`

**Interfaces:**
- Produces: `kit::doc::{Design, Part, Group, Shape, Transform, Mode, Depth, Material, Effects, Waves, WaveStyle, Mirror}`, `Design::default_note()`, `kit::{KIT_SCALE, GLASS_ALPHA, MAX_TRIANGLES, Z_MIN, Z_MAX, KitError}`. `ControlPoint` is reused from `crate::designer`.

- [ ] **Step 1: Write the failing test**

In `crates/render/src/kit/doc.rs` (create the file with only this test module for now; the types come in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_note_roundtrips_through_json() {
        let d = Design::default_note();
        assert_eq!(d.version, 2);
        assert_eq!(d.parts.len(), 1);
        assert_eq!(d.parts[0].mode, Mode::Add);
        assert_eq!(d.parts[0].depth, Depth { front: 0.1, back: -0.1 });
        let json = serde_json::to_string(&d).unwrap();
        let back: Design = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn missing_optional_fields_take_defaults() {
        let json = r##"{"version":2,"name":"x","colors":["#ffffff"],
          "parts":[{"id":"a","name":"a","shape":{"kind":"circle"},
          "transform":{"x":0,"y":0,"w":1,"h":1,"rotation":0},
          "mode":"add","depth":{"front":0.1,"back":-0.1}}]}"##;
        let d: Design = serde_json::from_str(json).unwrap();
        let p = &d.parts[0];
        assert!(p.visible);
        assert!(!p.locked);
        assert_eq!(p.material, Material::Solid);
        assert_eq!(p.effects, Effects::default());
        assert!(d.groups.is_empty());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p rhythia-render kit::doc`
Expected: compile error, `Design` not found (the module does not exist yet).

- [ ] **Step 3: Write the module root and the types**

`crates/render/src/kit/mod.rs`:

```rust
//! The note kit: a multi-part 2.5D note is resolved into oriented 2D
//! regions per depth interval and emitted as a watertight mesh with
//! solid and glass ranges. Every boolean runs through one fixed integer
//! scale so identical inputs give bit-identical vertices, which is what
//! lets caps and walls share rings without ever drifting apart.

pub mod doc;

pub use doc::*;

/// Fixed float-to-integer scale for every overlay. A power of two keeps
/// the conversion exact in f32; 1/65536 of a note unit is far below
/// anything visible, and the largest coordinate (depth 2.0 of extent)
/// stays far inside the i32 engine's range.
pub const KIT_SCALE: f32 = 65536.0;
/// The game draws every material after the first at a fixed alpha;
/// measured from an in-game screenshot (probe 7a, 2026-10-24).
pub const GLASS_ALPHA: f32 = 0.57;
/// The exporter refuses larger meshes; 10,500 triangles played without
/// a stutter in the game, so the margin is wide.
pub const MAX_TRIANGLES: usize = 40_000;
pub const Z_MIN: f32 = -2.0;
pub const Z_MAX: f32 = 0.5;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum KitError {
    #[error("the design has no visible part that adds material")]
    Empty,
    #[error("bad document: {0}")]
    Doc(String),
    #[error("geometry scale rejected: {0}")]
    Scale(String),
    #[error("mesh invalid: {0}")]
    Invalid(String),
    #[error("too many triangles: {0} (limit {MAX_TRIANGLES})")]
    Budget(usize),
    #[error("part too complex: {0} points in {1} contours")]
    Complexity(usize, usize),
}
```

`crates/render/src/kit/doc.rs` (above the test module):

```rust
//! The design document: the one JSON shape that undo snapshots, autosave,
//! the .rhsd file and the IPC payload all share.

use crate::designer::ControlPoint;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Design {
    pub version: u32,
    pub name: String,
    /// The colorset in beat order.
    pub colors: Vec<String>,
    /// Bottom to top in the panel.
    pub parts: Vec<Part>,
    #[serde(default)]
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    /// Part ids; cuts inside a group apply to the group's adds only.
    pub parts: Vec<String>,
    pub mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub id: String,
    pub name: String,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    pub shape: Shape,
    pub transform: Transform,
    pub mode: Mode,
    pub depth: Depth,
    #[serde(default)]
    pub material: Material,
    #[serde(default)]
    pub effects: Effects,
}

fn yes() -> bool {
    true
}

/// A shape source at unit size (half-extent 1 before the transform).
/// Glyph and SVG imports arrive as `Contours`: the editor resolves fonts
/// and files once and stores the result, so the kernel never touches
/// fonts or the file system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    Circle,
    /// A square with a side flat at the bottom; rounding blends toward
    /// the circle exactly as Polygon does.
    Square { rounding: f32 },
    Polygon { sides: u32, rounding: f32 },
    Star { points: u32, inner: f32 },
    Ring { thickness: f32 },
    Pill { aspect: f32 },
    Contours {
        contours: Vec<Vec<[f32; 2]>>,
        #[serde(default)]
        label: String,
    },
    Points { control: Vec<ControlPoint> },
}

/// Position and size in note-box units (1.0 = the full half-extent),
/// rotation in degrees counterclockwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub rotation: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Transform { x: 0.0, y: 0.0, w: 1.0, h: 1.0, rotation: 0.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Add,
    Cut,
}

/// A depth band; front is toward the player (+z), front > back.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Depth {
    pub front: f32,
    pub back: f32,
}

impl Default for Depth {
    fn default() -> Self {
        Depth { front: 0.1, back: -0.1 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Material {
    #[default]
    Solid,
    Glass,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Effects {
    #[serde(default)]
    pub waves: Option<Waves>,
    /// Corner rounding radius in note units.
    #[serde(default)]
    pub round: Option<f32>,
    /// Keeps only the outline band of this thickness.
    #[serde(default)]
    pub rim: Option<f32>,
    #[serde(default)]
    pub mirror: Mirror,
    /// Copies around the centre (2 or more).
    #[serde(default)]
    pub repeat: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Waves {
    pub count: u32,
    /// Radial amplitude as a fraction of the radius.
    pub depth: f32,
    #[serde(default)]
    pub style: WaveStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WaveStyle {
    #[default]
    Sine,
    Spike,
    Pulse,
}

/// Reflection axes: `X` reflects across the x axis (y becomes -y), `Y`
/// across the y axis, `Both` adds the point reflection as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mirror {
    #[default]
    None,
    X,
    Y,
    Both,
}

impl Design {
    /// A plain disc in the game's Arctic colours: what a new design is.
    pub fn default_note() -> Design {
        Design {
            version: 2,
            name: "note".into(),
            colors: vec!["#ffffff".into(), "#a2e0ff".into(), "#a2e0ff".into()],
            parts: vec![Part {
                id: "p1".into(),
                name: "Disc".into(),
                visible: true,
                locked: false,
                shape: Shape::Circle,
                transform: Transform::default(),
                mode: Mode::Add,
                depth: Depth::default(),
                material: Material::Solid,
                effects: Effects::default(),
            }],
            groups: Vec::new(),
        }
    }
}
```

Add to `crates/render/src/lib.rs` after `pub mod designer;`: `pub mod kit;`

Add to `crates/render/Cargo.toml` under `[dev-dependencies]`: `proptest = "1"`

`ControlPoint` in `designer.rs` already derives `Serialize, Deserialize`; add `PartialEq` to its derive list (`#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]`) so `Shape` can derive `PartialEq`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-render kit::doc`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/kit crates/render/src/lib.rs crates/render/src/designer.rs crates/render/Cargo.toml Cargo.lock
git commit -m "Kit: the design document, one JSON shape for undo, autosave, file and IPC

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Migration of v1 designer documents

**Files:**
- Create: `crates/render/src/kit/migrate.rs`
- Modify: `crates/render/src/kit/mod.rs` (add `pub mod migrate;`)
- Test: inline tests in `migrate.rs`

**Interfaces:**
- Consumes: `Design`, `Part`, `Shape`, `Effects`, `Waves`, `WaveStyle`, `Mirror`, `Depth`, `Transform` from Task 1; `ControlPoint` from `crate::designer`.
- Produces: `pub fn from_v1(json: &str) -> Result<Design, KitError>`.

The v1 document (what `designer.js` autosaves and writes to .rhsd) is:

```
{ "P": { "shape": "circle"|"square"|"ngon", "n": 6, "round": 0.3, "hollow": true,
         "thick": 0.22, "depth": 0.05, "waves": 0, "waveDepth": 0.25,
         "waveform": "sine"|"spike"|"pulse", "twist": 0 },
  "free": null | [ {x, y, smooth} ... ],      // ONE wedge of control points
  "symmetry": 4, "mirror": false,
  "colors": ["#ffffff", ...],
  "imported": null | [ [[x,y]...] ... ],
  "refImg": {...} }
```

Precedence in v1: `imported` wins over `free`, which wins over the sliders.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::doc::*;

    const SLIDERS: &str = r##"{"P":{"shape":"ngon","n":6,"round":0.3,"hollow":true,
      "thick":0.22,"depth":0.05,"waves":5,"waveDepth":0.25,"waveform":"spike","twist":30},
      "free":null,"symmetry":4,"mirror":false,"colors":["#ff5d6c","#2fd6d0"],"imported":null}"##;

    #[test]
    fn sliders_become_one_polygon_part_with_effects() {
        let d = from_v1(SLIDERS).unwrap();
        assert_eq!(d.version, 2);
        assert_eq!(d.colors, vec!["#ff5d6c", "#2fd6d0"]);
        assert_eq!(d.parts.len(), 1);
        let p = &d.parts[0];
        assert_eq!(p.shape, Shape::Polygon { sides: 6, rounding: 0.3 });
        assert_eq!(p.effects.rim, Some(0.22));
        assert_eq!(p.effects.waves, Some(Waves { count: 5, depth: 0.25, style: WaveStyle::Spike }));
        // v1 sampled the radius at angle + twist, which turns the shape
        // clockwise; the kit rotates counterclockwise, hence the sign.
        assert_eq!(p.transform.rotation, -30.0);
        assert_eq!(p.depth, Depth { front: 0.05, back: -0.05 });
    }

    #[test]
    fn a_v1_square_keeps_its_rounding() {
        let json = SLIDERS.replace(r#""shape":"ngon""#, r#""shape":"square""#).replace(r#""twist":30"#, r#""twist":0"#);
        let d = from_v1(&json).unwrap();
        let p = &d.parts[0];
        assert_eq!(p.shape, Shape::Square { rounding: 0.3 });
        assert_eq!(p.transform.rotation, 0.0);
        assert!(!p.transform.rotation.is_sign_negative(), "a zero twist must not serialise as -0.0");
    }

    #[test]
    fn freeform_and_imports_ignore_stale_slider_rotation() {
        // v1 never turned freeform or imported outlines, even when the
        // slider fields still said "square" or carried a twist.
        let json = r##"{"P":{"shape":"square","n":6,"round":0.3,"hollow":false,"thick":0.22,
          "depth":0.05,"waves":0,"waveDepth":0.25,"waveform":"sine","twist":30},
          "free":[{"x":1,"y":0,"smooth":false},{"x":0.5,"y":0.5,"smooth":true}],
          "symmetry":4,"mirror":false,"colors":["#ffffff"],"imported":null}"##;
        assert_eq!(from_v1(json).unwrap().parts[0].transform.rotation, 0.0);
        let json = json.replace(r#""imported":null"#, r#""imported":[[[-1,-1],[1,-1],[1,1],[-1,1]]]"#);
        assert_eq!(from_v1(&json).unwrap().parts[0].transform.rotation, 0.0);
    }

    #[test]
    fn freeform_keeps_symmetry_and_mirror_as_effects() {
        let json = r##"{"P":{"shape":"circle","n":6,"round":0.3,"hollow":false,"thick":0.22,
          "depth":0.05,"waves":0,"waveDepth":0.25,"waveform":"sine","twist":0},
          "free":[{"x":1,"y":0,"smooth":false},{"x":0.5,"y":0.5,"smooth":true}],
          "symmetry":5,"mirror":true,"colors":["#ffffff"],"imported":null}"##;
        let d = from_v1(json).unwrap();
        let p = &d.parts[0];
        match &p.shape {
            Shape::Points { control } => assert_eq!(control.len(), 2),
            other => panic!("expected points, got {other:?}"),
        }
        assert_eq!(p.effects.repeat, Some(5));
        assert_eq!(p.effects.mirror, Mirror::X);
        assert_eq!(p.effects.rim, None);
    }

    #[test]
    fn imports_win_and_become_contours() {
        let json = r##"{"P":{"shape":"circle","n":6,"round":0.3,"hollow":true,"thick":0.2,
          "depth":0.05,"waves":0,"waveDepth":0.25,"waveform":"sine","twist":0},
          "free":[{"x":1,"y":0,"smooth":true}],"symmetry":4,"mirror":false,
          "colors":["#ffffff"],"imported":[[[-1,-1],[1,-1],[1,1],[-1,1]]]}"##;
        let d = from_v1(json).unwrap();
        match &d.parts[0].shape {
            Shape::Contours { contours, .. } => assert_eq!(contours.len(), 1),
            other => panic!("expected contours, got {other:?}"),
        }
        assert_eq!(d.parts[0].effects.rim, Some(0.2));
    }

    #[test]
    fn garbage_is_a_doc_error_not_a_panic() {
        assert!(matches!(from_v1("not json"), Err(KitError::Doc(_))));
        assert!(matches!(from_v1("{}"), Err(KitError::Doc(_))));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-render kit::migrate`
Expected: compile error, `from_v1` not found.

- [ ] **Step 3: Implement the migration**

`crates/render/src/kit/migrate.rs`:

```rust
//! Upgrades v1 designer documents (the single-outline editor) into kit
//! designs: one Add part carrying the same shape, effects and symmetry,
//! so nothing anyone saved is lost.

use crate::designer::ControlPoint;
use crate::kit::doc::*;
use crate::kit::KitError;

pub fn from_v1(json: &str) -> Result<Design, KitError> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| KitError::Doc(format!("v1 json: {e}")))?;
    let p = v.get("P").ok_or_else(|| KitError::Doc("v1 document has no P".into()))?;
    let f = |key: &str, default: f64| p.get(key).and_then(|x| x.as_f64()).unwrap_or(default) as f32;
    let s = |key: &str, default: &str| {
        p.get(key).and_then(|x| x.as_str()).unwrap_or(default).to_string()
    };
    let colors: Vec<String> = v
        .get("colors")
        .and_then(|c| serde_json::from_value(c.clone()).ok())
        .unwrap_or_else(|| vec!["#ffffff".into()]);
    let symmetry = v.get("symmetry").and_then(|x| x.as_u64()).unwrap_or(1) as u32;
    let mirror = v.get("mirror").and_then(|x| x.as_bool()).unwrap_or(false);
    let hollow = p.get("hollow").and_then(|x| x.as_bool()).unwrap_or(false);

    let mut effects = Effects::default();
    if hollow {
        effects.rim = Some(f("thick", 0.22));
    }
    let imported: Option<Vec<Vec<[f32; 2]>>> = v
        .get("imported")
        .filter(|x| x.is_array())
        .and_then(|x| serde_json::from_value(x.clone()).ok());
    let free: Option<Vec<ControlPoint>> = v
        .get("free")
        .filter(|x| x.is_array())
        .and_then(|x| serde_json::from_value(x.clone()).ok());

    // Only the slider outline turned with twist (and the square's 45
    // degree offset) in v1; freeform and imported outlines never did, so
    // their rotation is zero even when the slider fields still hold values.
    let (shape, name, rotation) = if let Some(contours) = imported {
        (Shape::Contours { contours, label: "import".into() }, "Import", 0.0)
    } else if let Some(control) = free.filter(|c| c.len() >= 2) {
        if symmetry > 1 {
            effects.repeat = Some(symmetry);
        }
        if mirror {
            effects.mirror = Mirror::X;
        }
        (Shape::Points { control }, "Freeform", 0.0)
    } else {
        let waves = f("waves", 0.0) as u32;
        if waves > 0 {
            effects.waves = Some(Waves {
                count: waves,
                depth: f("waveDepth", 0.25),
                style: match s("waveform", "sine").as_str() {
                    "spike" => WaveStyle::Spike,
                    "pulse" => WaveStyle::Pulse,
                    _ => WaveStyle::Sine,
                },
            });
        }
        // v1 applied the rounding slider to squares as well; the kit's
        // Square carries rounding and its own 45 degree turn, so waves keep
        // the phase v1 gave them (they were evaluated before the turn).
        let shape = match s("shape", "circle").as_str() {
            "square" => Shape::Square { rounding: f("round", 0.3) },
            "ngon" => Shape::Polygon { sides: f("n", 6.0) as u32, rounding: f("round", 0.3) },
            _ => Shape::Circle,
        };
        // v1 sampled its radius at (angle + twist), which turns the shape
        // by MINUS twist; the kit rotates counterclockwise. Written as a
        // subtraction so a zero twist serialises as 0.0, not -0.0.
        (shape, "Shape", 0.0 - f("twist", 0.0))
    };
    let depth = f("depth", 0.05).clamp(0.005, 0.5);
    Ok(Design {
        version: 2,
        name: "note".into(),
        colors,
        parts: vec![Part {
            id: "p1".into(),
            name: name.into(),
            visible: true,
            locked: false,
            shape,
            transform: Transform { rotation, ..Transform::default() },
            mode: Mode::Add,
            depth: Depth { front: depth, back: -depth },
            material: Material::Solid,
            effects,
        }],
        groups: Vec::new(),
    })
}
```

Add `pub mod migrate;` to `kit/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-render kit::migrate`
Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/kit
git commit -m "Kit: v1 designer documents migrate into one part, nothing saved is lost

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Fixed-scale geometry helpers

**Files:**
- Create: `crates/render/src/kit/geom.rs`
- Modify: `crates/render/src/kit/mod.rs` (add `pub mod geom;`)
- Test: inline tests in `geom.rs`

**Interfaces:**
- Produces:
  - `pub type Contour = Vec<[f32; 2]>; pub type Shapes = Vec<Vec<Contour>>;` (a shape is outer contour first, then holes)
  - `pub fn clean(contours: &[Contour], rule: FillRule) -> Result<Shapes, KitError>`
  - `pub fn boolean(a: &Shapes, b: &Shapes, rule: OverlayRule) -> Result<Shapes, KitError>`
  - `pub fn offset(shapes: &Shapes, delta: f32) -> Result<Shapes, KitError>`
  - `pub fn orient(shapes: &mut Shapes)` (outer counterclockwise, holes clockwise)
  - `pub fn signed_area(c: &[[f32; 2]]) -> f32`
  - `pub fn extent(shapes: &Shapes) -> f32` (max of |x| and |y| over outer contours)
  - `pub fn is_empty(shapes: &Shapes) -> bool`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn square(h: f32) -> Contour {
        vec![[-h, -h], [h, -h], [h, h], [-h, h]]
    }

    #[test]
    fn clean_orients_outer_ccw_and_holes_cw() {
        // Outer clockwise on purpose, hole counterclockwise on purpose.
        let outer: Contour = vec![[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]];
        let hole: Contour = vec![[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];
        let s = clean(&[outer, hole], FillRule::EvenOdd).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].len(), 2);
        assert!(signed_area(&s[0][0]) > 0.0);
        assert!(signed_area(&s[0][1]) < 0.0);
    }

    #[test]
    fn boolean_difference_cuts_a_hole() {
        let a = clean(&[square(1.0)], FillRule::NonZero).unwrap();
        let b = clean(&[square(0.4)], FillRule::NonZero).unwrap();
        let d = boolean(&a, &b, OverlayRule::Difference).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].len(), 2, "outer plus one hole");
        let u = boolean(&a, &b, OverlayRule::Union).unwrap();
        assert_eq!(u[0].len(), 1);
    }

    #[test]
    fn identical_inputs_give_bit_identical_vertices() {
        // The whole kit leans on this: a region's contour must come back
        // with the same bits whenever it is derived from the same inputs.
        let a = clean(&[square(1.0)], FillRule::NonZero).unwrap();
        let b = clean(&[square(0.4)], FillRule::NonZero).unwrap();
        let d1 = boolean(&a, &b, OverlayRule::Difference).unwrap();
        let d2 = boolean(&a, &b, OverlayRule::Difference).unwrap();
        assert_eq!(d1, d2);
        let i = boolean(&a, &b, OverlayRule::Intersect).unwrap();
        // The hole of the difference is the inner square, the same points
        // the intersection's outer contour has.
        let mut hole_pts: Vec<[u32; 2]> =
            d1[0][1].iter().map(|p| [p[0].to_bits(), p[1].to_bits()]).collect();
        let mut inner_pts: Vec<[u32; 2]> =
            i[0][0].iter().map(|p| [p[0].to_bits(), p[1].to_bits()]).collect();
        hole_pts.sort();
        inner_pts.sort();
        assert_eq!(hole_pts, inner_pts);
    }

    #[test]
    fn offset_shrinks_and_grows() {
        let a = clean(&[square(1.0)], FillRule::NonZero).unwrap();
        let smaller = offset(&a, -0.25).unwrap();
        assert!((extent(&smaller) - 0.75).abs() < 1e-3);
        let bigger = offset(&a, 0.25).unwrap();
        assert!((extent(&bigger) - 1.25).abs() < 2e-2, "round joins stay near 1.25");
        let gone = offset(&a, -1.5).unwrap();
        assert!(is_empty(&gone));
    }

    #[test]
    fn degenerate_input_is_empty_not_an_error() {
        let line: Contour = vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]];
        assert!(is_empty(&clean(&[line], FillRule::NonZero).unwrap()));
        assert!(is_empty(&clean(&[], FillRule::NonZero).unwrap()));
    }

    #[test]
    fn off_centre_inputs_share_ring_vertices_bit_for_bit() {
        // i_overlay's own fixed-scale calls centre their lattice on each
        // call's bounding box, so a ring and the cap cut around it came
        // back on different lattices. The shared adapter ends that: the
        // cap's hole must be the ring, bit for bit, for inputs that are
        // not centred and for inputs already on the lattice but with a
        // bounding-box centre half a cell off it.
        let rect = |x0: f32, y0: f32, x1: f32, y1: f32| -> Contour {
            vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
        };
        let bits = |c: &Contour| {
            let mut v: Vec<[u32; 2]> = c.iter().map(|p| [p[0].to_bits(), p[1].to_bits()]).collect();
            v.sort();
            v
        };
        for inner in [
            rect(0.1, -0.3, 0.7, 0.45),
            rect(0.25, -0.25, 0.5 + 1.0 / 65536.0, 0.5),
        ] {
            let r1 = clean(&[inner], FillRule::NonZero).unwrap();
            let r2 = clean(&[rect(-1.0, -1.0, 1.3, 1.0)], FillRule::NonZero).unwrap();
            let cap = boolean(&r2, &r1, OverlayRule::Difference).unwrap();
            assert_eq!(cap.len(), 1);
            assert_eq!(cap[0].len(), 2, "outer plus the hole the ring cut");
            assert_eq!(bits(&cap[0][1]), bits(&r1[0][0]));
        }
    }

    #[test]
    fn offset_grows_a_region_and_shrinks_its_hole() {
        let mut hole = square(0.5);
        hole.reverse();
        let ring = clean(&[square(1.0), hole], FillRule::NonZero).unwrap();
        assert_eq!(ring[0].len(), 2);
        let grown = offset(&ring, 0.1).unwrap();
        assert!((extent(&grown) - 1.1).abs() < 2e-2);
        let hole_extent = grown[0][1]
            .iter()
            .map(|p| p[0].abs().max(p[1].abs()))
            .fold(0.0f32, f32::max);
        assert!((hole_extent - 0.4).abs() < 1e-3, "hole extent {hole_extent}");
    }

    #[test]
    fn out_of_range_or_nan_input_is_a_scale_error() {
        let far: Contour = vec![[100.0, 0.0], [101.0, 0.0], [100.0, 1.0]];
        assert!(matches!(clean(&[far], FillRule::NonZero), Err(KitError::Scale(_))));
        let a = clean(&[square(1.0)], FillRule::NonZero).unwrap();
        assert!(matches!(offset(&a, f32::NAN), Err(KitError::Scale(_))));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-render kit::geom`
Expected: compile error, `clean` not found.

- [ ] **Step 3: Implement the helpers**

`crates/render/src/kit/geom.rs`:

```rust
//! Fixed-scale, fixed-origin wrappers around i_overlay. Every operation
//! snaps to ONE integer lattice (cell 1/65536, origin at the note's
//! centre, spanning plus and minus KIT_RANGE), so a region derived twice
//! from the same inputs is bit-identical, and contours shared between
//! neighbouring operations line up vertex for vertex. i_overlay's own
//! fixed-scale entry points fix the cell size but centre the lattice on
//! each call's bounding box, which is why the adapter is built here.

use crate::kit::{KitError, KIT_SCALE};
pub use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::ShapeType;
pub use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::overlay::FloatOverlay;
use i_overlay::i_float::adapter::FloatPointAdapter;
use i_overlay::i_float::float::rect::FloatRect;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};

pub type Contour = Vec<[f32; 2]>;
pub type Shapes = Vec<Vec<Contour>>;

/// Half-extent of the lattice in note units. Notes live in the unit box
/// and depth never exceeds 2, so 64 leaves room for any transform the
/// editor allows, while 64 * 65536 stays far inside the i32 engine.
pub const KIT_RANGE: f32 = 64.0;

/// Signed area, positive for counterclockwise. Summed in f64 relative to
/// the first vertex: a sliver one lattice cell wide would otherwise
/// cancel in f32 and come out with the wrong sign.
pub fn signed_area(c: &[[f32; 2]]) -> f32 {
    if c.len() < 3 {
        return 0.0;
    }
    let (ox, oy) = (c[0][0] as f64, c[0][1] as f64);
    let mut a = 0.0f64;
    for i in 0..c.len() {
        let p = c[i];
        let q = c[(i + 1) % c.len()];
        let (px, py) = (p[0] as f64 - ox, p[1] as f64 - oy);
        let (qx, qy) = (q[0] as f64 - ox, q[1] as f64 - oy);
        a += px * qy - qx * py;
    }
    (a * 0.5) as f32
}

/// Outer counterclockwise, holes clockwise: the wall builder encodes
/// facing in traversal direction, so this is enforced rather than
/// assumed from the kernel's defaults.
pub fn orient(shapes: &mut Shapes) {
    for s in shapes.iter_mut() {
        for (i, c) in s.iter_mut().enumerate() {
            let ccw = signed_area(c) > 0.0;
            if (i == 0) != ccw {
                c.reverse();
            }
        }
    }
}

fn tidy(mut shapes: Shapes) -> Shapes {
    // A shape whose outer boundary degenerates is dropped whole: keeping
    // a surviving hole would promote it to a filled region.
    shapes.retain(|s| s.first().is_some_and(|outer| outer.len() >= 3));
    for s in shapes.iter_mut() {
        s.retain(|c| c.len() >= 3);
    }
    orient(&mut shapes);
    shapes
}

fn check_range<'a>(pts: impl Iterator<Item = &'a [f32; 2]>) -> Result<(), KitError> {
    for p in pts {
        let ok = p[0].is_finite() && p[1].is_finite() && p[0].abs() <= KIT_RANGE && p[1].abs() <= KIT_RANGE;
        if !ok {
            return Err(KitError::Scale(format!(
                "point {p:?} is outside the lattice range of {KIT_RANGE}"
            )));
        }
    }
    Ok(())
}

/// The one lattice every operation shares.
fn adapter() -> FloatPointAdapter<[f32; 2], i32> {
    FloatPointAdapter::with_scale(
        FloatRect::new(-KIT_RANGE, KIT_RANGE, -KIT_RANGE, KIT_RANGE),
        KIT_SCALE,
    )
}

fn overlay(subject: &Shapes, clip: &Shapes, rule: OverlayRule, fill: FillRule) -> Result<Shapes, KitError> {
    check_range(subject.iter().flatten().flatten())?;
    check_range(clip.iter().flatten().flatten())?;
    let points = subject.iter().flatten().map(|c| c.len()).sum::<usize>()
        + clip.iter().flatten().map(|c| c.len()).sum::<usize>();
    let out: Shapes = FloatOverlay::with_adapter(adapter(), points.max(16))
        .unsafe_add_source(subject, ShapeType::Subject)
        .unsafe_add_source(clip, ShapeType::Clip)
        .overlay(rule, fill);
    Ok(tidy(out))
}

/// Resolves raw contours into clean regions under `rule`.
pub fn clean(contours: &[Contour], rule: FillRule) -> Result<Shapes, KitError> {
    let raw: Vec<Contour> = contours.iter().filter(|c| c.len() >= 3).cloned().collect();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    overlay(&vec![raw], &Vec::new(), OverlayRule::Subject, rule)
}

/// Boolean of two clean regions (nonzero: both are already consistent).
pub fn boolean(a: &Shapes, b: &Shapes, rule: OverlayRule) -> Result<Shapes, KitError> {
    if a.is_empty() && b.is_empty() {
        return Ok(Vec::new());
    }
    overlay(a, b, rule, FillRule::NonZero)
}

/// Grows (delta > 0) or shrinks (delta < 0) a region, round joins. A
/// region thinner than twice a negative delta vanishes, which is the
/// correct limit. The outline call centres its own lattice on the input,
/// so its result passes through the shared lattice once more; the input
/// is oriented first because the outline builder reads facing from the
/// traversal direction.
pub fn offset(shapes: &Shapes, delta: f32) -> Result<Shapes, KitError> {
    if !delta.is_finite() {
        return Err(KitError::Scale(format!("offset {delta} is not finite")));
    }
    if shapes.is_empty() || delta == 0.0 {
        return Ok(shapes.clone());
    }
    check_range(shapes.iter().flatten().flatten())?;
    let mut input = shapes.clone();
    orient(&mut input);
    let style = OutlineStyle::new(delta).line_join(LineJoin::Round(0.2));
    let grown: Shapes = input
        .outline_fixed_scale(&style, KIT_SCALE)
        .map_err(|e| KitError::Scale(format!("{e:?}")))?;
    let grown = tidy(grown);
    if grown.is_empty() {
        return Ok(Vec::new());
    }
    overlay(&grown, &Vec::new(), OverlayRule::Subject, FillRule::NonZero)
}

/// The largest |x| or |y| over every outer contour.
pub fn extent(shapes: &Shapes) -> f32 {
    shapes
        .iter()
        .filter_map(|s| s.first())
        .flatten()
        .map(|p| p[0].abs().max(p[1].abs()))
        .fold(0.0f32, f32::max)
}

/// True when no shape has a usable outer contour.
pub fn is_empty(shapes: &Shapes) -> bool {
    shapes.iter().all(|s| s.is_empty() || s[0].len() < 3)
}
```

Add `pub mod geom;` to `kit/mod.rs`.

The adapter-based overlay is the i_overlay 8.1 API verified against the crate source: `FloatOverlay::with_adapter(adapter, capacity)` returns a builder, `unsafe_add_source(resource, ShapeType)` chains, `overlay(rule, fill)` returns `Shapes`; `OverlayRule::Subject` with an empty clip returns the subject cleaned under `fill`. If a name differs in your i_overlay version, look at `~/.cargo/registry/src/*/i_overlay-8.1.0/src/float/overlay.rs` and report what you find; do not fall back to `overlay_with_fixed_scale`, whose lattice origin moves with the input.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-render kit::geom`
Expected: 8 passed. If `offset_shrinks_and_grows` fails on the exact extent of the grown square, loosen only that tolerance (round joins approximate the corner arc); never loosen either bit-identity test.

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/kit
git commit -m "Kit: fixed-scale geometry helpers, identical inputs give identical vertices

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Shape generators

**Files:**
- Create: `crates/render/src/kit/shapes.rs`
- Modify: `crates/render/src/kit/mod.rs` (add `pub mod shapes;`)
- Test: inline tests in `shapes.rs`

**Interfaces:**
- Consumes: `Shape`, `ControlPoint`, `crate::designer::flatten_controls`, `geom::{Contour, signed_area}`.
- Produces: `pub fn base_contours(shape: &Shape) -> Vec<Contour>` (unit size, dense, counterclockwise where generated; imports as stored).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::doc::Shape;
    use crate::kit::geom::signed_area;

    fn extent(c: &[[f32; 2]]) -> f32 {
        c.iter().map(|p| p[0].abs().max(p[1].abs())).fold(0.0, f32::max)
    }

    #[test]
    fn generated_shapes_are_dense_unit_and_ccw() {
        for shape in [
            Shape::Circle,
            Shape::Square { rounding: 0.0 },
            Shape::Polygon { sides: 6, rounding: 0.3 },
            Shape::Star { points: 5, inner: 0.45 },
            Shape::Pill { aspect: 0.5 },
        ] {
            let cs = base_contours(&shape);
            assert_eq!(cs.len(), 1, "{shape:?}");
            assert!(cs[0].len() >= 64, "{shape:?} has {} points", cs[0].len());
            assert!((extent(&cs[0]) - 1.0).abs() < 1e-3, "{shape:?}");
            assert!(signed_area(&cs[0]) > 0.0, "{shape:?}");
        }
    }

    #[test]
    fn ring_is_outer_plus_cw_hole() {
        let cs = base_contours(&Shape::Ring { thickness: 0.3 });
        assert_eq!(cs.len(), 2);
        assert!(signed_area(&cs[0]) > 0.0);
        assert!(signed_area(&cs[1]) < 0.0);
        assert!((extent(&cs[1]) - 0.7).abs() < 1e-3);
    }

    #[test]
    fn polygon_rounding_moves_between_ngon_and_circle() {
        let sharp = base_contours(&Shape::Polygon { sides: 4, rounding: 0.0 });
        let soft = base_contours(&Shape::Polygon { sides: 4, rounding: 1.0 });
        // At 45 degrees a diamond's radius is 1/sqrt(2); a circle's is 1.
        let r = |c: &[[f32; 2]]| {
            c.iter()
                .map(|p| (p[0].hypot(p[1]), (p[1].atan2(p[0]) - std::f32::consts::FRAC_PI_4).abs()))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap()
                .0
        };
        assert!((r(&sharp[0]) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.03);
        assert!((r(&soft[0]) - 1.0).abs() < 0.03);
    }

    #[test]
    fn points_flatten_through_the_spline_and_contours_pass_through() {
        let ctrl = |x: f32, y: f32| crate::designer::ControlPoint { x, y, smooth: true };
        let pts = base_contours(&Shape::Points {
            control: vec![ctrl(-1.0, -1.0), ctrl(1.0, -1.0), ctrl(1.0, 1.0), ctrl(-1.0, 1.0)],
        });
        assert!(pts[0].len() > 4);
        let raw = vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]];
        let cs = base_contours(&Shape::Contours { contours: raw.clone(), label: String::new() });
        assert_eq!(cs, raw);
    }

    #[test]
    fn star_edges_are_straight() {
        // Every sample between the tip at (1, 0) and the first valley
        // lies on their chord: a radius linear in the angle would bow it.
        let inner = 0.45f32;
        let cs = base_contours(&Shape::Star { points: 5, inner });
        let tip = [1.0f32, 0.0];
        let valley_angle = std::f32::consts::TAU / 10.0;
        let valley = [inner * valley_angle.cos(), inner * valley_angle.sin()];
        let mut between = 0;
        for p in &cs[0] {
            let ang = p[1].atan2(p[0]);
            if ang > 1e-4 && ang < valley_angle - 1e-4 {
                between += 1;
                let cross = (valley[0] - tip[0]) * (p[1] - tip[1]) - (valley[1] - tip[1]) * (p[0] - tip[0]);
                assert!(cross.abs() < 1e-5, "point {p:?} is off the chord by {cross}");
            }
        }
        assert!(between >= 10, "only {between} samples on the first edge");
    }

    #[test]
    fn pill_has_points_on_its_straight_sides() {
        let aspect = 0.3f32;
        let cs = base_contours(&Shape::Pill { aspect });
        let on_side = cs[0]
            .iter()
            .filter(|p| (p[1].abs() - aspect).abs() < 1e-4 && p[0].abs() < 1.0 - aspect - 1e-3)
            .count();
        assert!(on_side >= 40, "only {on_side} points on the flat sides");
        assert!((extent(&cs[0]) - 1.0).abs() < 1e-3);
    }
}
```

(`extent` here is the test module's own contour helper defined at its top.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-render kit::shapes`
Expected: compile error, `base_contours` not found.

- [ ] **Step 3: Implement the generators**

`crates/render/src/kit/shapes.rs`:

```rust
//! Shape sources at unit size. Generated outlines are dense (so waves
//! and rounding have points to work with) and counterclockwise; imports
//! pass through as stored.

use crate::designer::flatten_controls;
use crate::kit::doc::Shape;
use crate::kit::geom::Contour;
use std::f32::consts::{PI, TAU};

const SAMPLES: usize = 256;

fn polar(samples: usize, radius: impl Fn(f32) -> f32) -> Contour {
    (0..samples)
        .map(|i| {
            let th = i as f32 / samples as f32 * TAU;
            let r = radius(th);
            [r * th.cos(), r * th.sin()]
        })
        .collect()
}

/// Regular n-gon radius at angle `th`, blended toward the circle by
/// `rounding` in 0..=1. The square is the 4-gon turned so a side sits
/// flat at the bottom.
fn ngon_radius(sides: u32, rounding: f32, th: f32, offset: f32) -> f32 {
    let n = sides.max(3) as f32;
    let sector = TAU / n;
    let a = ((th + offset).rem_euclid(sector) - sector * 0.5).abs();
    let ngon = (PI / n).cos() / a.cos();
    ngon + (1.0 - ngon) * rounding.clamp(0.0, 1.0)
}

/// Scale a contour about the origin to half-extent 1. The turned 4-gon
/// has its corners on the unit circle, which leaves its flat sides at
/// 0.707; every other source reaches 1 on its own. A uniform scale keeps
/// the rounding blend and the later wave phase untouched.
fn fit_half_extent(mut c: Contour) -> Contour {
    let m = c.iter().flatten().fold(0.0f32, |m, v| m.max(v.abs()));
    if !(m.is_finite() && m > 0.0) {
        return c;
    }
    for p in &mut c {
        p[0] /= m;
        p[1] /= m;
    }
    c
}

/// Walks a closed polygon and resamples every edge in proportion to its
/// length, about `total` points in all, so long straight runs carry
/// interior points too: waves and rounding have nothing to work with on
/// a bare segment.
fn sample_edges(corners: &[[f32; 2]], total: usize) -> Contour {
    let n = corners.len();
    let len = |i: usize| {
        let (a, b) = (corners[i], corners[(i + 1) % n]);
        ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
    };
    let perimeter: f32 = (0..n).map(len).sum();
    if n < 3 || !(perimeter.is_finite() && perimeter > 0.0) {
        return corners.to_vec();
    }
    let mut c = Vec::with_capacity(total + n);
    for i in 0..n {
        let (a, b) = (corners[i], corners[(i + 1) % n]);
        let per = ((total as f32 * len(i) / perimeter).round() as usize).max(1);
        for s in 0..per {
            let t = s as f32 / per as f32;
            c.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    c
}

pub fn base_contours(shape: &Shape) -> Vec<Contour> {
    match shape {
        Shape::Circle => vec![polar(SAMPLES, |_| 1.0)],
        Shape::Square { rounding } => {
            vec![fit_half_extent(polar(SAMPLES, |th| {
                ngon_radius(4, *rounding, th, PI / 4.0)
            }))]
        }
        Shape::Polygon { sides, rounding } => {
            // Vertex at angle zero, as the v1 editor drew its polygons;
            // only the square is turned so a side sits flat.
            vec![polar(SAMPLES, |th| ngon_radius(*sides, *rounding, th, 0.0))]
        }
        Shape::Star { points, inner } => {
            // Tips at radius 1, valleys at `inner`, every edge sampled as
            // a straight segment in Cartesian space: a radius linear in
            // the angle would bow each edge into a spiral arc.
            let n = (*points).max(2) as usize;
            let inner = inner.clamp(0.05, 0.95);
            let corners: Vec<[f32; 2]> = (0..2 * n)
                .map(|k| {
                    let r = if k % 2 == 0 { 1.0 } else { inner };
                    let th = k as f32 / (2 * n) as f32 * TAU;
                    [r * th.cos(), r * th.sin()]
                })
                .collect();
            vec![sample_edges(&corners, SAMPLES)]
        }
        Shape::Ring { thickness } => {
            let t = thickness.clamp(0.02, 0.98);
            let mut hole = polar(SAMPLES, move |_| 1.0 - t);
            hole.reverse();
            vec![polar(SAMPLES, |_| 1.0), hole]
        }
        Shape::Pill { aspect } => {
            // A stadium: half-extent 1 along x, `aspect` along y. The two
            // arcs are sampled directly; the straight sides between them
            // get their points from the length-proportional resample.
            let r = aspect.clamp(0.1, 1.0);
            let cx = 1.0 - r;
            let half = SAMPLES / 2;
            let mut arcs = Vec::with_capacity(SAMPLES);
            for i in 0..half {
                let th = -PI / 2.0 + PI * i as f32 / half as f32;
                arcs.push([cx + r * th.cos(), r * th.sin()]);
            }
            for i in 0..half {
                let th = PI / 2.0 + PI * i as f32 / half as f32;
                arcs.push([-cx + r * th.cos(), r * th.sin()]);
            }
            vec![sample_edges(&arcs, SAMPLES)]
        }
        Shape::Contours { contours, .. } => contours.clone(),
        Shape::Points { control } => {
            if control.len() < 3 {
                return Vec::new();
            }
            vec![flatten_controls(control)]
        }
    }
}
```

(`ControlPoint` is the designer's type; the import above only serves the `Points` arm. If the import is unused after your edit, drop it.)

Add `pub mod shapes;` to `kit/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-render kit::shapes`
Expected: every test in the module passes (the six above plus any the implementer added).

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/kit
git commit -m "Kit: shape sources at unit size, dense and counterclockwise

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Part resolution (effects, transform, symmetry)

**Files:**
- Create: `crates/render/src/kit/resolve.rs`
- Modify: `crates/render/src/kit/mod.rs` (add `pub mod resolve;`)
- Test: inline tests in `resolve.rs`

**Interfaces:**
- Consumes: `base_contours` (Task 4), `geom::{clean, boolean, offset, Shapes, Contour, FillRule, OverlayRule, extent}` (Task 3), doc types (Task 1).
- Produces:
  - `pub struct ResolvedPart { pub id: String, pub mode: Mode, pub material: Material, pub depth: Depth, pub shapes: Shapes }`
  - `pub fn resolve_part(part: &Part) -> Result<ResolvedPart, KitError>`

Effect order is fixed: waves, round, rim, then transform, then symmetry, then clean.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::doc::*;
    use crate::kit::geom::{extent, signed_area};

    fn part(shape: Shape) -> Part {
        Part {
            id: "p".into(),
            name: "p".into(),
            visible: true,
            locked: false,
            shape,
            transform: Transform::default(),
            mode: Mode::Add,
            depth: Depth::default(),
            material: Material::Solid,
            effects: Effects::default(),
        }
    }

    #[test]
    fn plain_circle_resolves_to_one_shape() {
        let r = resolve_part(&part(Shape::Circle)).unwrap();
        assert_eq!(r.shapes.len(), 1);
        assert_eq!(r.shapes[0].len(), 1);
        assert!((extent(&r.shapes) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn transform_scales_rotates_and_moves() {
        let mut p = part(Shape::Square { rounding: 0.0 });
        p.transform = Transform { x: 0.5, y: 0.0, w: 0.5, h: 0.25, rotation: 0.0 };
        let r = resolve_part(&p).unwrap();
        let xs: Vec<f32> = r.shapes[0][0].iter().map(|q| q[0]).collect();
        let ys: Vec<f32> = r.shapes[0][0].iter().map(|q| q[1]).collect();
        let (min_x, max_x) = (xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max));
        let (min_y, max_y) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));
        assert!((min_x - 0.0).abs() < 1e-3 && (max_x - 1.0).abs() < 1e-3);
        assert!((min_y + 0.25).abs() < 1e-3 && (max_y - 0.25).abs() < 1e-3);
        p.transform = Transform { rotation: 90.0, w: 0.5, h: 0.25, ..Transform::default() };
        let r = resolve_part(&p).unwrap();
        let ys: Vec<f32> = r.shapes[0][0].iter().map(|q| q[1]).collect();
        assert!((ys.iter().cloned().fold(f32::MIN, f32::max) - 0.5).abs() < 1e-3);
    }

    fn hole_extent(shapes: &Shapes) -> f32 {
        shapes[0][1].iter().map(|p| p[0].abs().max(p[1].abs())).fold(0.0f32, f32::max)
    }

    #[test]
    fn finish_rims_and_rounds_in_note_units() {
        let r = resolve_part(&part(Shape::Square { rounding: 0.0 })).unwrap();
        assert_eq!(r.rim, None, "resolution carries the rim, finish applies it");
        let rimmed = finish_part(&r.shapes, None, Some(0.3)).unwrap();
        assert_eq!(rimmed[0].len(), 2, "outer plus hole");
        assert!(signed_area(&rimmed[0][1]) < 0.0);
        assert!((hole_extent(&rimmed) - 0.7).abs() < 1e-3);
        let rounded = finish_part(&r.shapes, Some(0.3), None).unwrap();
        // The corner at (1,1) is gone while the flat sides still touch 1:
        // the diagonal reach of a sharp corner is 2.0, of this rounding
        // about 1.78.
        let max_diag = rounded[0][0].iter().map(|v| v[0] + v[1]).fold(f32::MIN, f32::max);
        assert!(max_diag < 1.85 && max_diag > 1.7, "diagonal reach {max_diag}");
        assert!((extent(&rounded) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_finish_never_deletes_a_part() {
        // A ring whose wall is thinner than the rounding or the rim keeps
        // its region instead of vanishing.
        let r = resolve_part(&part(Shape::Ring { thickness: 0.4 })).unwrap();
        let rounded = finish_part(&r.shapes, Some(0.25), None).unwrap();
        assert_eq!(rounded.len(), 1);
        assert_eq!(rounded[0].len(), 2);
        let rimmed = finish_part(&r.shapes, None, Some(0.5)).unwrap();
        assert_eq!(rimmed[0].len(), 2);
    }

    #[test]
    fn rim_is_uniform_after_an_anisotropic_transform() {
        // A rim is a note-unit thickness, so a part scaled differently in
        // x and y still gets the same band on every side.
        let mut p = part(Shape::Square { rounding: 0.0 });
        p.transform = Transform { w: 0.5, h: 0.25, ..Transform::default() };
        let r = resolve_part(&p).unwrap();
        let rimmed = finish_part(&r.shapes, None, Some(0.05)).unwrap();
        let hole = &rimmed[0][1];
        let hx = hole.iter().map(|q| q[0].abs()).fold(0.0f32, f32::max);
        let hy = hole.iter().map(|q| q[1].abs()).fold(0.0f32, f32::max);
        assert!((hx - 0.45).abs() < 1e-3, "hole half-width {hx}");
        assert!((hy - 0.2).abs() < 1e-3, "hole half-height {hy}");
    }

    #[test]
    fn rotation_is_counterclockwise() {
        // A triangle with its apex up: turned 90 degrees counterclockwise
        // the apex points left, so x never exceeds zero.
        let ctrl = |x: f32, y: f32| crate::designer::ControlPoint { x, y, smooth: false };
        let mut p = part(Shape::Points { control: vec![ctrl(1.0, 0.0), ctrl(0.0, 1.0), ctrl(-1.0, 0.0)] });
        p.transform.rotation = 90.0;
        let r = resolve_part(&p).unwrap();
        let xs: Vec<f32> = r.shapes[0][0].iter().map(|q| q[0]).collect();
        assert!(xs.iter().cloned().fold(f32::MAX, f32::min) < -0.9);
        assert!(xs.iter().cloned().fold(f32::MIN, f32::max) < 0.01);
    }

    #[test]
    fn corrupt_parameters_are_capped_not_fatal() {
        // A three-point source keeps this cheap: the caps are what is
        // under test, not the geometry. Never run this against code
        // without the caps.
        let ctrl = |x: f32, y: f32| crate::designer::ControlPoint { x, y, smooth: false };
        let mut p = part(Shape::Points { control: vec![ctrl(0.6, 0.0), ctrl(0.9, 0.1), ctrl(0.6, 0.2)] });
        p.effects.repeat = Some(u32::MAX);
        p.effects.waves = Some(Waves { count: u32::MAX, depth: 40.0, style: WaveStyle::Sine });
        let r = resolve_part(&p).unwrap();
        assert!(!r.shapes.is_empty());
        assert!(r.shapes.len() <= 32, "repeat is capped at 32 copies");
        let finished = finish_part(&r.shapes, Some(5.0), Some(5.0)).unwrap();
        assert!(!finished.is_empty());
    }

    #[test]
    fn an_overly_complex_part_is_a_clear_error() {
        // The caps bound the loops, not the geometry; a source beyond the
        // point budget is refused before any offset can run away. The
        // budget measures the cleaned region, and the lattice drops
        // collinear points (a 20k-point circle cleans to about 15k), so
        // the source has to be well above the budget to be refused.
        let huge: Vec<[f32; 2]> = (0..40_000)
            .map(|i| {
                let t = i as f32 / 40_000.0 * std::f32::consts::TAU;
                [t.cos(), t.sin()]
            })
            .collect();
        let p = part(Shape::Contours { contours: vec![huge], label: String::new() });
        assert!(matches!(resolve_part(&p), Err(KitError::Complexity(..))));
    }

    #[test]
    fn waves_change_the_radius() {
        let mut p = part(Shape::Circle);
        p.effects.waves = Some(Waves { count: 6, depth: 0.2, style: WaveStyle::Sine });
        let r = resolve_part(&p).unwrap();
        let radii: Vec<f32> = r.shapes[0][0].iter().map(|v| v[0].hypot(v[1])).collect();
        let (min, max) = (radii.iter().cloned().fold(f32::MAX, f32::min), radii.iter().cloned().fold(f32::MIN, f32::max));
        assert!(min < 0.9 && max > 1.1, "radii {min}..{max}");
    }

    #[test]
    fn repeat_and_mirror_make_copies_that_merge() {
        let mut p = part(Shape::Circle);
        p.transform = Transform { x: 0.6, y: 0.0, w: 0.3, h: 0.3, rotation: 0.0 };
        p.effects.repeat = Some(4);
        let r = resolve_part(&p).unwrap();
        assert_eq!(r.shapes.len(), 4, "four separate discs");
        let mut q = part(Shape::Circle);
        q.transform = Transform { x: 0.0, y: 0.4, w: 0.5, h: 0.5, rotation: 0.0 };
        q.effects.mirror = Mirror::X;
        let r = resolve_part(&q).unwrap();
        assert_eq!(r.shapes.len(), 1, "overlapping mirror copies merge into one region");
        let min_y = r.shapes[0][0].iter().map(|v| v[1]).fold(f32::MAX, f32::min);
        assert!((min_y + 0.9).abs() < 1e-2);
    }

    #[test]
    fn degenerate_parts_resolve_to_nothing() {
        let mut p = part(Shape::Points { control: vec![] });
        let r = resolve_part(&p).unwrap();
        assert!(r.shapes.is_empty());
        p.shape = Shape::Circle;
        p.transform.w = 0.0;
        let r = resolve_part(&p).unwrap();
        assert!(r.shapes.is_empty());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-render kit::resolve`
Expected: compile error, `resolve_part` not found.

Never run `corrupt_parameters_are_capped_not_fatal` against code without the caps: with `repeat = u32::MAX` the uncapped copy loop tries to build four billion contours and took a shared machine to 9.9 GB of RAM before it was killed. Write that one test after the caps exist and say so in the report; the other tests may be run red.

- [ ] **Step 3: Implement resolution**

`crates/render/src/kit/resolve.rs`:

```rust
//! A part becomes clean regions: source outline, waves (keyed on each
//! point's angle and applied before the transform, so a turned shape
//! keeps its wave phase), the 2D transform, symmetry copies, then one
//! nonzero clean so overlapping copies merge. Rounding and the rim are
//! deferred to `finish_part`, which the evaluation runs AFTER the whole
//! note is normalised, so both work in note units on the merged region
//! the way the v1 editor applied them.

use crate::kit::doc::*;
use crate::kit::geom::{boolean, clean, offset, Contour, FillRule, OverlayRule, Shapes};
use crate::kit::shapes::base_contours;
use crate::kit::KitError;

/// Caps on stored parameters: a corrupt document degrades, it never
/// allocates or loops without bound.
const MAX_REPEAT: u32 = 32;
const MAX_WAVES: u32 = 32;
const MAX_WAVE_DEPTH: f32 = 0.95;
const MAX_FINISH: f32 = 0.9;
/// Complexity budget of one resolved part. The caps above bound the
/// loops, not the geometry: thirty-two wavy copies can merge into
/// thousands of contours, and an offset on that runs for minutes and
/// takes gigabytes. A part beyond this budget is a clear error instead.
pub const MAX_PART_POINTS: usize = 16_384;
pub const MAX_PART_CONTOURS: usize = 512;

fn complexity_check(shapes: &Shapes) -> Result<(), KitError> {
    let contours: usize = shapes.iter().map(|s| s.len()).sum();
    let points: usize = shapes.iter().flatten().map(|c| c.len()).sum();
    if contours > MAX_PART_CONTOURS || points > MAX_PART_POINTS {
        return Err(KitError::Complexity(points, contours));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPart {
    pub id: String,
    pub mode: Mode,
    pub material: Material,
    pub depth: Depth,
    /// Corner rounding radius and rim thickness in note units, applied
    /// by `finish_part` once the note is normalised.
    pub round: Option<f32>,
    pub rim: Option<f32>,
    pub shapes: Shapes,
}

fn wave_fn(style: WaveStyle, k: f32) -> f32 {
    let s = k.sin();
    match style {
        WaveStyle::Sine => s,
        WaveStyle::Spike => (2.0 / std::f32::consts::PI) * s.asin(),
        WaveStyle::Pulse => (3.0 * s).tanh() / 3.0f32.tanh(),
    }
}

/// Radial modulation around the part's own centre; a point's angle
/// picks the phase, so any dense outline (not only circles) waves.
fn apply_waves(contours: &mut [Contour], w: &Waves) {
    let count = w.count.min(MAX_WAVES);
    let depth = w.depth.clamp(-MAX_WAVE_DEPTH, MAX_WAVE_DEPTH);
    if count == 0 || depth == 0.0 {
        return;
    }
    for c in contours.iter_mut() {
        for p in c.iter_mut() {
            let th = p[1].atan2(p[0]);
            let f = 1.0 + depth * wave_fn(w.style, count as f32 * th);
            p[0] *= f;
            p[1] *= f;
        }
    }
}

fn apply_transform(contours: &mut [Contour], t: &Transform) {
    let (s, c) = t.rotation.to_radians().sin_cos();
    for contour in contours.iter_mut() {
        for p in contour.iter_mut() {
            let x = p[0] * t.w;
            let y = p[1] * t.h;
            p[0] = x * c - y * s + t.x;
            p[1] = x * s + y * c + t.y;
        }
    }
}

fn reflect(contours: &[Contour], fx: f32, fy: f32) -> Vec<Contour> {
    contours
        .iter()
        .map(|c| {
            let mut r: Contour = c.iter().map(|p| [p[0] * fx, p[1] * fy]).collect();
            // A single reflection flips orientation; reverse to keep the
            // nonzero winding consistent across copies.
            if fx * fy < 0.0 {
                r.reverse();
            }
            r
        })
        .collect()
}

fn symmetry_copies(contours: Vec<Contour>, e: &Effects) -> Vec<Contour> {
    let mut out: Vec<Contour> = contours.clone();
    match e.mirror {
        Mirror::None => {}
        Mirror::X => out.extend(reflect(&contours, 1.0, -1.0)),
        Mirror::Y => out.extend(reflect(&contours, -1.0, 1.0)),
        Mirror::Both => {
            out.extend(reflect(&contours, 1.0, -1.0));
            out.extend(reflect(&contours, -1.0, 1.0));
            out.extend(reflect(&contours, -1.0, -1.0));
        }
    }
    if let Some(n) = e.repeat.filter(|n| *n >= 2).map(|n| n.min(MAX_REPEAT)) {
        let base = out.clone();
        for k in 1..n {
            let a = std::f32::consts::TAU * k as f32 / n as f32;
            let (s, c) = a.sin_cos();
            out.extend(base.iter().map(|contour| {
                contour.iter().map(|p| [p[0] * c - p[1] * s, p[0] * s + p[1] * c]).collect::<Contour>()
            }));
        }
    }
    out
}

fn finish_value(v: Option<f32>) -> Option<f32> {
    v.filter(|x| x.is_finite() && *x > 1e-4).map(|x| x.min(MAX_FINISH))
}

pub fn resolve_part(part: &Part) -> Result<ResolvedPart, KitError> {
    let empty = || ResolvedPart {
        id: part.id.clone(),
        mode: part.mode,
        material: part.material,
        depth: part.depth,
        round: finish_value(part.effects.round),
        rim: finish_value(part.effects.rim),
        shapes: Vec::new(),
    };
    let mut contours = base_contours(&part.shape);
    if contours.is_empty() || part.transform.w.abs() < 1e-4 || part.transform.h.abs() < 1e-4 {
        return Ok(empty());
    }
    if let Some(w) = &part.effects.waves {
        apply_waves(&mut contours, w);
    }
    apply_transform(&mut contours, &part.transform);
    let copies = symmetry_copies(contours, &part.effects);
    let shapes = clean(&copies, FillRule::NonZero)?;
    complexity_check(&shapes)?;
    Ok(ResolvedPart { shapes, ..empty() })
}

/// Rounding (closing, then opening) and the rim (the region minus its
/// erosion) in note units, on a merged and normalised region. Where the
/// region is thinner than the rounding or the rim allows, it stays as it
/// is: a finish never deletes a part. Offsets are the expensive step,
/// so the complexity budget is checked before them.
pub fn finish_part(shapes: &Shapes, round: Option<f32>, rim: Option<f32>) -> Result<Shapes, KitError> {
    complexity_check(shapes)?;
    let mut region = shapes.clone();
    if let Some(r) = finish_value(round) {
        let closed = offset(&offset(&region, r)?, -r)?;
        let opened = offset(&closed, -r)?;
        if !opened.is_empty() {
            region = offset(&opened, r)?;
        }
    }
    if let Some(t) = finish_value(rim) {
        let eroded = offset(&region, -t)?;
        if !eroded.is_empty() {
            region = boolean(&region, &eroded, OverlayRule::Difference)?;
        }
    }
    Ok(region)
}
```

Add `pub mod resolve;` to `kit/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-render kit::resolve`
Expected: 11 passed, the whole module in well under a second.

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/kit
git commit -m "Kit: parts resolve through effects, transform and symmetry into clean regions

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Level evaluation with groups and normalisation

**Files:**
- Create: `crates/render/src/kit/levels.rs`
- Modify: `crates/render/src/kit/mod.rs` (add `pub mod levels;`)
- Test: inline tests in `levels.rs`

**Interfaces:**
- Consumes: `resolve_part`, `ResolvedPart` (Task 5), `geom::{boolean, extent, is_empty, Shapes, OverlayRule}` (Task 3), doc types.
- Produces:
  - `pub struct Interval { pub z0: f32, pub z1: f32, pub solid: Shapes, pub glass: Shapes }`
  - `pub struct Evaluation { pub scale: f32, pub levels: Vec<f32>, pub intervals: Vec<Interval>, pub parts: Vec<ResolvedPart> }`
  - `pub fn evaluate(design: &Design) -> Result<Evaluation, KitError>`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::doc::*;
    use crate::kit::geom::{extent, is_empty};

    fn part(id: &str, shape: Shape, mode: Mode, front: f32, back: f32) -> Part {
        Part {
            id: id.into(),
            name: id.into(),
            visible: true,
            locked: false,
            shape,
            transform: Transform::default(),
            mode,
            depth: Depth { front, back },
            material: Material::Solid,
            effects: Effects::default(),
        }
    }

    fn design(parts: Vec<Part>) -> Design {
        Design { version: 2, name: "t".into(), colors: vec!["#ffffff".into()], parts, groups: vec![] }
    }

    #[test]
    fn one_part_is_one_interval() {
        let ev = evaluate(&design(vec![part("a", Shape::Circle, Mode::Add, 0.1, -0.1)])).unwrap();
        assert_eq!(ev.levels, vec![-0.1, 0.1]);
        assert_eq!(ev.intervals.len(), 1);
        assert!(!is_empty(&ev.intervals[0].solid));
        assert!(is_empty(&ev.intervals[0].glass));
    }

    #[test]
    fn ring_with_recessed_plate_has_three_levels() {
        let ring = part("ring", Shape::Ring { thickness: 0.28 }, Mode::Add, 0.1, -0.1);
        let mut plate = part("plate", Shape::Circle, Mode::Add, -0.4, -0.6);
        plate.transform = Transform { w: 0.55, h: 0.55, ..Transform::default() };
        let ev = evaluate(&design(vec![ring, plate])).unwrap();
        assert_eq!(ev.levels, vec![-0.6, -0.4, -0.1, 0.1]);
        assert_eq!(ev.intervals.len(), 3);
        assert!(!is_empty(&ev.intervals[0].solid), "plate band");
        assert!(is_empty(&ev.intervals[1].solid), "the gap between plate and ring");
        assert_eq!(ev.intervals[2].solid[0].len(), 2, "ring band: outer plus hole");
    }

    #[test]
    fn cuts_remove_and_glass_yields_to_solid() {
        let disc = part("d", Shape::Circle, Mode::Add, 0.1, -0.1);
        let mut bar = part("b", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        bar.transform = Transform { w: 1.2, h: 0.2, ..Transform::default() };
        let ev = evaluate(&design(vec![disc.clone(), bar])).unwrap();
        assert_eq!(ev.intervals[0].solid.len(), 2, "the bar splits the disc in two");
        let mut glass = part("g", Shape::Square { rounding: 0.0 }, Mode::Add, 0.1, -0.1);
        glass.material = Material::Glass;
        glass.transform = Transform { w: 0.5, h: 0.5, ..Transform::default() };
        let ev = evaluate(&design(vec![disc, glass])).unwrap();
        assert!(is_empty(&ev.intervals[0].glass), "glass fully inside solid is swallowed");
    }

    #[test]
    fn groups_keep_their_cuts_to_themselves() {
        let base = part("base", Shape::Circle, Mode::Add, 0.1, -0.1);
        let mut small = part("small", Shape::Circle, Mode::Add, 0.1, -0.1);
        small.transform = Transform { x: 0.5, w: 0.4, h: 0.4, ..Transform::default() };
        let mut cut = part("cut", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        cut.transform = Transform { x: 0.5, w: 0.2, h: 0.2, ..Transform::default() };
        let mut d = design(vec![base, small, cut]);
        d.groups = vec![Group { id: "g".into(), name: "g".into(), parts: vec!["small".into(), "cut".into()], mode: Mode::Add }];
        let ev = evaluate(&d).unwrap();
        // The cut lives inside the group: the base disc stays whole, so
        // the solid region is one shape without a hole.
        assert_eq!(ev.intervals[0].solid.len(), 1);
        assert_eq!(ev.intervals[0].solid[0].len(), 1);
    }

    #[test]
    fn silhouette_is_normalised_to_the_box() {
        let mut big = part("a", Shape::Circle, Mode::Add, 0.1, -0.1);
        big.transform = Transform { x: 2.0, w: 3.0, h: 3.0, ..Transform::default() };
        let ev = evaluate(&design(vec![big])).unwrap();
        assert!((extent(&ev.intervals[0].solid) - 1.0).abs() < 1e-3);
        assert!(ev.scale < 1.0);
    }

    #[test]
    fn all_cut_or_hidden_is_an_error() {
        let mut hidden = part("a", Shape::Circle, Mode::Add, 0.1, -0.1);
        hidden.visible = false;
        assert_eq!(evaluate(&design(vec![hidden])).unwrap_err(), KitError::Empty);
        let only_cut = part("c", Shape::Circle, Mode::Cut, 0.1, -0.1);
        assert_eq!(evaluate(&design(vec![only_cut])).unwrap_err(), KitError::Empty);
        let disc = part("d", Shape::Circle, Mode::Add, 0.1, -0.1);
        let mut eraser = part("e", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        eraser.transform = Transform { w: 2.0, h: 2.0, ..Transform::default() };
        assert_eq!(evaluate(&design(vec![disc, eraser])).unwrap_err(), KitError::Empty);
    }

    #[test]
    fn touching_bands_share_one_level() {
        let a = part("a", Shape::Circle, Mode::Add, 0.1, -0.1);
        let mut b = part("b", Shape::Circle, Mode::Add, -0.1, -0.3);
        b.transform = Transform { w: 0.5, h: 0.5, ..Transform::default() };
        let ev = evaluate(&design(vec![a, b])).unwrap();
        assert_eq!(ev.levels, vec![-0.3, -0.1, 0.1]);
        assert_eq!(ev.intervals.len(), 2);
    }

    #[test]
    fn bad_depths_are_doc_errors() {
        let upside_down = part("a", Shape::Circle, Mode::Add, -0.1, 0.1);
        assert!(matches!(evaluate(&design(vec![upside_down])), Err(KitError::Doc(_))));
        let too_deep = part("a", Shape::Circle, Mode::Add, 0.1, -3.0);
        assert!(matches!(evaluate(&design(vec![too_deep])), Err(KitError::Doc(_))));
    }

    #[test]
    fn rim_is_applied_after_normalisation() {
        // A square twice the box with a rim of 0.3: normalised first, the
        // hole sits at 0.7; rimmed first and scaled after, it would sit
        // at 0.85. The v1 editor normalised first, and so does the kit.
        let mut big = part("a", Shape::Square { rounding: 0.0 }, Mode::Add, 0.1, -0.1);
        big.transform = Transform { w: 2.0, h: 2.0, ..Transform::default() };
        big.effects.rim = Some(0.3);
        let ev = evaluate(&design(vec![big])).unwrap();
        let solid = &ev.intervals[0].solid;
        assert_eq!(solid[0].len(), 2, "outer plus hole");
        let hole = solid[0][1].iter().map(|p| p[0].abs().max(p[1].abs())).fold(0.0f32, f32::max);
        assert!((hole - 0.7).abs() < 1e-3, "hole extent {hole}");
        assert!((extent(solid) - 1.0).abs() < 1e-3);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-render kit::levels`
Expected: compile error, `evaluate` not found.

- [ ] **Step 3: Implement evaluation**

`crates/render/src/kit/levels.rs`:

```rust
//! Depth levels and the solid and glass regions between them. Groups
//! fold first (a group's cuts only touch its own adds), then everything
//! combines per interval: union of adds minus union of cuts, with glass
//! as a second region that yields to solid wherever both overlap.

use crate::kit::doc::*;
use crate::kit::geom::{boolean, extent, is_empty, OverlayRule, Shapes};
use crate::kit::resolve::{finish_part, resolve_part, ResolvedPart};
use crate::kit::{KitError, Z_MAX, Z_MIN};

#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    pub z0: f32,
    pub z1: f32,
    pub solid: Shapes,
    pub glass: Shapes,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    /// The factor applied to every x and y to put the silhouette on the
    /// unit box (1.0 when it already was).
    pub scale: f32,
    pub levels: Vec<f32>,
    pub intervals: Vec<Interval>,
    pub parts: Vec<ResolvedPart>,
}

const LEVEL_EPS: f32 = 1e-5;

fn union_all(shapes: &[&Shapes]) -> Result<Shapes, KitError> {
    let mut acc: Shapes = Vec::new();
    for s in shapes {
        if is_empty(s) {
            continue;
        }
        acc = if acc.is_empty() { (*s).clone() } else { boolean(&acc, s, OverlayRule::Union)? };
    }
    Ok(acc)
}

fn subtract(a: &Shapes, b: &Shapes) -> Result<Shapes, KitError> {
    if is_empty(a) {
        return Ok(Vec::new());
    }
    if is_empty(b) {
        return Ok(a.clone());
    }
    boolean(a, b, OverlayRule::Difference)
}

/// One contribution to an interval after group folding.
struct Contribution<'a> {
    mode: Mode,
    material: Material,
    shapes: std::borrow::Cow<'a, Shapes>,
}

fn covers(p: &ResolvedPart, z0: f32, z1: f32) -> bool {
    let zm = (z0 + z1) * 0.5;
    p.depth.back < zm && zm < p.depth.front
}

pub fn evaluate(design: &Design) -> Result<Evaluation, KitError> {
    for p in design.parts.iter().filter(|p| p.visible) {
        if !(p.depth.front > p.depth.back) {
            return Err(KitError::Doc(format!("part {}: front must be above back", p.id)));
        }
        if p.depth.back < Z_MIN || p.depth.front > Z_MAX {
            return Err(KitError::Doc(format!(
                "part {}: depth must stay within {Z_MIN}..{Z_MAX}",
                p.id
            )));
        }
    }
    let mut parts: Vec<ResolvedPart> = design
        .parts
        .iter()
        .filter(|p| p.visible)
        .map(resolve_part)
        .collect::<Result<_, _>>()?;
    parts.retain(|p| !p.shapes.is_empty());

    // Normalise the silhouette (union of every add) to the unit box.
    let adds: Vec<&Shapes> = parts.iter().filter(|p| p.mode == Mode::Add).map(|p| &p.shapes).collect();
    let silhouette = union_all(&adds)?;
    let m = extent(&silhouette);
    if m < 1e-6 {
        return Err(KitError::Empty);
    }
    let scale = if (m - 1.0).abs() > 1e-4 { 1.0 / m } else { 1.0 };
    if scale != 1.0 {
        for p in &mut parts {
            for s in &mut p.shapes {
                for c in s.iter_mut() {
                    for q in c.iter_mut() {
                        q[0] *= scale;
                        q[1] *= scale;
                    }
                }
            }
        }
    }
    // Rounding and rims are note-unit finishes, so they run on the
    // normalised, merged regions (the v1 editor normalised first too); a
    // finish never changes the outer extent, so the scale stays exact.
    for p in &mut parts {
        if p.round.is_some() || p.rim.is_some() {
            p.shapes = finish_part(&p.shapes, p.round, p.rim)?;
        }
    }
    parts.retain(|p| !p.shapes.is_empty());

    // Level set.
    let mut levels: Vec<f32> = parts.iter().flat_map(|p| [p.depth.back, p.depth.front]).collect();
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap());
    levels.dedup_by(|a, b| (*a - *b).abs() < LEVEL_EPS);

    let in_group: std::collections::HashMap<&str, &Group> = design
        .groups
        .iter()
        .flat_map(|g| g.parts.iter().map(move |id| (id.as_str(), g)))
        .collect();

    let mut intervals = Vec::with_capacity(levels.len().saturating_sub(1));
    let mut any_solid = false;
    for w in levels.windows(2) {
        let (z0, z1) = (w[0], w[1]);
        let covering: Vec<&ResolvedPart> = parts.iter().filter(|p| covers(p, z0, z1)).collect();
        let mut contributions: Vec<Contribution> = Vec::new();
        // Groups fold first.
        for g in &design.groups {
            let members: Vec<&ResolvedPart> =
                covering.iter().copied().filter(|p| g.parts.iter().any(|id| id == &p.id)).collect();
            if members.is_empty() {
                continue;
            }
            let adds: Vec<&Shapes> = members.iter().filter(|p| p.mode == Mode::Add).map(|p| &p.shapes).collect();
            let cuts: Vec<&Shapes> = members.iter().filter(|p| p.mode == Mode::Cut).map(|p| &p.shapes).collect();
            let folded = subtract(&union_all(&adds)?, &union_all(&cuts)?)?;
            let material = if members.iter().all(|p| p.material == Material::Glass) {
                Material::Glass
            } else {
                Material::Solid
            };
            contributions.push(Contribution { mode: g.mode, material, shapes: std::borrow::Cow::Owned(folded) });
        }
        for p in covering.iter().filter(|p| !in_group.contains_key(p.id.as_str())) {
            contributions.push(Contribution { mode: p.mode, material: p.material, shapes: std::borrow::Cow::Borrowed(&p.shapes) });
        }
        let pick = |mode: Mode, material: Option<Material>| -> Vec<&Shapes> {
            contributions
                .iter()
                .filter(|c| c.mode == mode && material.map_or(true, |m| c.material == m))
                .map(|c| c.shapes.as_ref())
                .collect()
        };
        let cuts = union_all(&pick(Mode::Cut, None))?;
        let solid = subtract(&union_all(&pick(Mode::Add, Some(Material::Solid)))?, &cuts)?;
        let glass = subtract(&subtract(&union_all(&pick(Mode::Add, Some(Material::Glass)))?, &cuts)?, &solid)?;
        any_solid |= !is_empty(&solid) || !is_empty(&glass);
        intervals.push(Interval { z0, z1, solid, glass });
    }
    if !any_solid {
        return Err(KitError::Empty);
    }
    Ok(Evaluation { scale, levels, intervals, parts })
}
```

Add `pub mod levels;` to `kit/mod.rs`.

**Amendment after the review of 1bf98f1 (controller ruling; binding where it differs from the Step 3 code above):**

1. Group materials. An Add-mode group folds per material: with `cuts = union(the group's Cut members)`, it contributes `subtract(union(its solid Add members), cuts)` as Add/Solid and `subtract(union(its glass Add members), cuts)` as Add/Glass (an empty contribution is skipped). A Cut-mode group contributes `subtract(union(all its Add members), cuts)` as one Cut. The rule "glass only when every member is glass" is wrong: a cut defaults to Solid and would turn a glass lens opaque. Glass stays glass and loses to solid only where the two overlap (spec 4.2 step 2).
2. Groups are validated before anything is resolved: a part id listed in more than one group is `KitError::Doc(format!("part {id} is in more than one group"))`.
3. The silhouette is the largest `extent` over the parts that add material: loose Add parts and Add members of Add-mode groups. Cut parts and every member of a Cut-mode group only remove and never count. The union of the adds is not built: the extent of a union equals the largest extent of its inputs.
4. When `scale > 1.0`, every remover (a loose Cut part, a Cut member of an Add-mode group, every member of a Cut-mode group) is intersected with the square `let r = 3.0 * m; vec![vec![vec![[-r, -r], [r, -r], [r, r], [-r, r]]]]` via `boolean(.., OverlayRule::Intersect)` BEFORE scaling: outside the square it removes nothing, and without the clip a scale above 1 pushes a cut past `KIT_RANGE` and a valid design fails with `KitError::Scale`. The square is the silhouette plus a margin of two note units (three times `m`, which scales to half-extent 3.0, far inside `KIT_RANGE`): the finish runs on the clipped outline after scaling, and a rim or rounding reaches at most `MAX_FINISH = 0.9` from the outline, so with the margin the clip edge can never carve a band along the box.
5. After scaling (only when `scale != 1.0`) every part is re-snapped to the lattice with a new helper in `geom.rs`, so single-contributor intervals sit on the same grid as overlay results:

```rust
/// Passes a region through the shared lattice once more, so coordinates
/// produced outside i_overlay (a scale, a hand-written contour) sit on
/// the same grid as every overlay result.
pub fn snap(shapes: &Shapes) -> Result<Shapes, KitError> {
    if shapes.is_empty() {
        return Ok(Vec::new());
    }
    overlay(shapes, &Vec::new(), OverlayRule::Subject, FillRule::NonZero)
}
```

with one test in `geom.rs`:

```rust
    #[test]
    fn snap_puts_every_coordinate_on_the_lattice() {
        let tri: Shapes = vec![vec![vec![[0.1, 0.2], [0.3, 0.2], [0.3, 0.7]]]];
        let snapped = snap(&tri).unwrap();
        assert_eq!(snapped[0][0].len(), 3);
        for p in snapped.iter().flatten().flatten() {
            for v in p {
                let k = v * KIT_SCALE;
                assert!((k - k.round()).abs() < 1e-2, "{v} is off the lattice");
            }
        }
    }
```

6. The comment above the finish loop states the real invariant: a finish never makes a part larger (a rim keeps the outer loop, a rounding pulls convex corners inward), so the box is never exceeded; the finish runs per part on the normalised region.
7. Housekeeping: the group membership lookup is a `HashSet<&str>`, the flag that at least one interval holds material is named `any_region`, and the level sort uses `f32::total_cmp` (NaN is already rejected by the depth validation, so the sort needs no unwrap).
8. Six more tests in `levels.rs`; the nine above stay unchanged:

```rust
    #[test]
    fn glass_keeps_its_material_inside_a_group() {
        let mut lens = part("lens", Shape::Circle, Mode::Add, 0.1, -0.1);
        lens.material = Material::Glass;
        let mut hole = part("hole", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        hole.transform = Transform { w: 0.5, h: 0.5, ..Transform::default() };
        let mut d = design(vec![lens, hole]);
        d.groups = vec![Group { id: "g".into(), name: "g".into(), parts: vec!["lens".into(), "hole".into()], mode: Mode::Add }];
        let ev = evaluate(&d).unwrap();
        // The cut defaults to Solid, which must not turn the lens solid.
        assert!(is_empty(&ev.intervals[0].solid));
        assert_eq!(ev.intervals[0].glass.len(), 1);
        assert_eq!(ev.intervals[0].glass[0].len(), 2, "the lens keeps its hole");
    }

    #[test]
    fn a_cut_group_removes_the_union_of_its_adds() {
        let plate = part("plate", Shape::Square { rounding: 0.0 }, Mode::Add, 0.1, -0.1);
        let mut a = part("a", Shape::Circle, Mode::Add, 0.1, -0.1);
        a.transform = Transform { x: -0.5, w: 0.3, h: 0.3, ..Transform::default() };
        let mut b = part("b", Shape::Circle, Mode::Add, 0.1, -0.1);
        b.transform = Transform { x: 0.9, w: 0.3, h: 0.3, ..Transform::default() };
        let mut d = design(vec![plate, a, b]);
        d.groups = vec![Group { id: "g".into(), name: "g".into(), parts: vec!["a".into(), "b".into()], mode: Mode::Cut }];
        let ev = evaluate(&d).unwrap();
        // One hole inside, one bite at the edge; the discs add nothing.
        assert_eq!(ev.intervals[0].solid.len(), 1);
        assert_eq!(ev.intervals[0].solid[0].len(), 2);
        // The discs do not count toward the silhouette: the plate stays the box.
        assert!((ev.scale - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_part_in_two_groups_is_a_doc_error() {
        let base = part("base", Shape::Circle, Mode::Add, 0.1, -0.1);
        let mut d = design(vec![base]);
        d.groups = vec![
            Group { id: "g1".into(), name: "g1".into(), parts: vec!["base".into()], mode: Mode::Add },
            Group { id: "g2".into(), name: "g2".into(), parts: vec!["base".into()], mode: Mode::Cut },
        ];
        assert!(matches!(evaluate(&d), Err(KitError::Doc(_))));
    }

    #[test]
    fn a_far_cut_does_not_fail_a_small_note() {
        // The disc has half-extent 0.01, so the scale is 100; the cut's far
        // edge would land at 100, past KIT_RANGE, without the clip.
        let mut disc = part("disc", Shape::Circle, Mode::Add, 0.1, -0.1);
        disc.transform = Transform { w: 0.01, h: 0.01, ..Transform::default() };
        let mut cut = part("cut", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        cut.transform = Transform { x: 0.5, w: 0.5, h: 0.5, ..Transform::default() };
        let ev = evaluate(&design(vec![disc, cut])).unwrap();
        assert!((ev.scale - 100.0).abs() < 0.1);
        let solid = &ev.intervals[0].solid;
        assert_eq!(solid.len(), 1);
        // The cut covers x >= 0: the left half-disc remains and reaches -1.
        assert!(solid[0][0].iter().all(|p| p[0] <= 1e-3));
        assert!((extent(solid) - 1.0).abs() < 2e-3);
    }

    #[test]
    fn glass_survives_where_it_does_not_overlap_solid() {
        let mut solid = part("solid", Shape::Square { rounding: 0.0 }, Mode::Add, 0.1, -0.1);
        solid.transform = Transform { x: -0.5, w: 0.5, h: 0.5, ..Transform::default() };
        let mut glass = part("glass", Shape::Square { rounding: 0.0 }, Mode::Add, 0.1, -0.1);
        glass.material = Material::Glass;
        glass.transform = Transform { x: 0.5, w: 0.5, h: 0.5, ..Transform::default() };
        let ev = evaluate(&design(vec![solid, glass])).unwrap();
        let iv = &ev.intervals[0];
        // Side by side, both survive and together fill the box.
        assert_eq!(iv.solid.len(), 1);
        assert_eq!(iv.glass.len(), 1);
        assert!(iv.solid[0][0].iter().all(|p| p[0] <= 1e-4));
        assert!(iv.glass[0][0].iter().all(|p| p[0] >= -1e-4));
        assert!((extent(&iv.glass) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_cut_stays_inside_its_band() {
        let plate = part("plate", Shape::Square { rounding: 0.0 }, Mode::Add, 0.1, -0.3);
        let mut hole = part("hole", Shape::Circle, Mode::Cut, 0.1, -0.1);
        hole.transform = Transform { w: 0.4, h: 0.4, ..Transform::default() };
        let ev = evaluate(&design(vec![plate, hole])).unwrap();
        assert_eq!(ev.levels.len(), 3);
        // Lower band: the plate is whole. Upper band: the plate has the hole.
        assert_eq!(ev.intervals[0].solid[0].len(), 1);
        assert_eq!(ev.intervals[1].solid[0].len(), 2);
    }

    #[test]
    fn a_rimmed_cut_beyond_the_note_removes_nothing_when_scaled_up() {
        // The plate has half-extent 0.5, so the scale is 2. The cut is a
        // disc of radius 2 in note units with a rim of 0.2: a ring at
        // 1.8..2.0, outside the box. Clipping the cut to the box itself
        // would run the rim along the plate's edge and carve a band.
        let mut plate = part("plate", Shape::Square { rounding: 0.0 }, Mode::Add, 0.1, -0.1);
        plate.transform = Transform { w: 0.5, h: 0.5, ..Transform::default() };
        let mut cut = part("cut", Shape::Circle, Mode::Cut, 0.1, -0.1);
        cut.transform = Transform { w: 1.0, h: 1.0, ..Transform::default() };
        cut.effects.rim = Some(0.2);
        let ev = evaluate(&design(vec![plate, cut])).unwrap();
        assert!((ev.scale - 2.0).abs() < 1e-3);
        let solid = &ev.intervals[0].solid;
        assert_eq!(solid.len(), 1);
        assert_eq!(solid[0].len(), 1, "no band carved along the edge");
        assert!((extent(solid) - 1.0).abs() < 1e-4);
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-render kit::levels` and `cargo test -p rhythia-render kit::geom`
Expected: 16 passed in levels, 9 passed in geom.

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/kit
git commit -m "Kit: depth levels, groups fold first, glass yields to solid

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: The validator

**Files:**
- Create: `crates/render/src/kit/validate.rs`
- Modify: `crates/render/src/kit/mod.rs` (add `pub mod validate;`)
- Test: inline tests in `validate.rs`

**Interfaces:**
- Produces:
  - `pub struct KitMesh { pub positions: Vec<[f32; 3]>, pub indices: Vec<u32>, pub solid: std::ops::Range<usize>, pub glass: std::ops::Range<usize> }` (index ranges, each a multiple of 3; `solid` first, `glass` second, together covering `indices`)
  - `pub struct Report { pub triangles: usize, pub vertices: usize }`
  - `pub fn validate(mesh: &KitMesh) -> Result<Report, KitError>`

The validator checks, per material range: every edge appears exactly twice with opposite direction (closed, consistently oriented surface), no degenerate triangles, all coordinates finite and inside the box, triangle budget.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A closed unit cube, outward-facing, 12 triangles.
    fn cube() -> KitMesh {
        let p = |x: f32, y: f32, z: f32| [x, y, z];
        let positions = vec![
            p(-1.0, -1.0, -0.1), p(1.0, -1.0, -0.1), p(1.0, 1.0, -0.1), p(-1.0, 1.0, -0.1),
            p(-1.0, -1.0, 0.1), p(1.0, -1.0, 0.1), p(1.0, 1.0, 0.1), p(-1.0, 1.0, 0.1),
        ];
        let indices: Vec<u32> = vec![
            4, 5, 6, 4, 6, 7, // front (+z)
            0, 2, 1, 0, 3, 2, // back (-z)
            0, 1, 5, 0, 5, 4, // bottom (-y)
            1, 2, 6, 1, 6, 5, // right (+x)
            2, 3, 7, 2, 7, 6, // top (+y)
            3, 0, 4, 3, 4, 7, // left (-x)
        ];
        let n = indices.len();
        KitMesh { positions, indices, solid: 0..n, glass: n..n }
    }

    #[test]
    fn a_closed_cube_passes() {
        let r = validate(&cube()).unwrap();
        assert_eq!(r.triangles, 12);
        assert_eq!(r.vertices, 8);
    }

    #[test]
    fn a_missing_triangle_is_an_open_edge() {
        let mut m = cube();
        m.indices.truncate(m.indices.len() - 3);
        m.solid = 0..m.indices.len();
        m.glass = m.indices.len()..m.indices.len();
        let e = validate(&m).unwrap_err();
        assert!(matches!(e, KitError::Invalid(ref s) if s.contains("open")), "{e}");
    }

    #[test]
    fn a_flipped_triangle_is_inconsistent() {
        let mut m = cube();
        m.indices.swap(0, 1);
        let e = validate(&m).unwrap_err();
        assert!(matches!(e, KitError::Invalid(_)), "{e}");
    }

    #[test]
    fn degenerate_out_of_box_and_nan_fail() {
        let mut m = cube();
        m.indices[0] = m.indices[1];
        assert!(matches!(validate(&m).unwrap_err(), KitError::Invalid(ref s) if s.contains("degenerate")));
        let mut m = cube();
        m.positions[0][0] = -1.5;
        assert!(matches!(validate(&m).unwrap_err(), KitError::Invalid(ref s) if s.contains("box")));
        let mut m = cube();
        m.positions[0][2] = f32::NAN;
        assert!(matches!(validate(&m).unwrap_err(), KitError::Invalid(ref s) if s.contains("finite")));
    }

    #[test]
    fn ranges_must_tile_the_index_buffer_in_triangles() {
        let mut m = cube();
        m.solid = 0..4;
        assert!(matches!(validate(&m).unwrap_err(), KitError::Invalid(_)));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-render kit::validate`
Expected: compile error, `KitMesh` not found.

- [ ] **Step 3: Implement the validator**

`crates/render/src/kit/validate.rs`:

```rust
//! The export gate. The game crashes on broken skins, so a mesh that is
//! not a closed, consistently oriented surface inside the box never
//! leaves the kernel. Deterministic and dependency-free, which is what
//! lets it double as the oracle for the property tests.

use crate::kit::{KitError, MAX_TRIANGLES, Z_MAX, Z_MIN};
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct KitMesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Index ranges per material; solid first, glass second.
    pub solid: Range<usize>,
    pub glass: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub triangles: usize,
    pub vertices: usize,
}

const BOX_EPS: f32 = 1e-4;
/// Twice the smallest area a real triangle can have here: two lattice
/// steps (1/65536 each) span about 2.3e-10, a lattice step times the
/// smallest level gap (1e-5) about 1.5e-10. Anything below is a fold of
/// coincident positions, not a face.
const MIN_AREA2: f32 = 1e-11;

fn invalid(msg: impl Into<String>) -> KitError {
    KitError::Invalid(msg.into())
}

fn check_range(mesh: &KitMesh, range: &Range<usize>, label: &str) -> Result<(), KitError> {
    if range.start > range.end || range.end > mesh.indices.len() || (range.end - range.start) % 3 != 0 {
        return Err(invalid(format!("{label} range {range:?} does not tile triangles")));
    }
    if range.is_empty() {
        return Ok(());
    }
    // Ordered so the edge named in an error is the same on every run.
    let mut directed: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for t in mesh.indices[range.clone()].chunks_exact(3) {
        let (a, b, c) = (t[0], t[1], t[2]);
        if a == b || b == c || a == c {
            return Err(invalid(format!("{label}: degenerate triangle {t:?}")));
        }
        let (pa, pb, pc) = (mesh.positions[a as usize], mesh.positions[b as usize], mesh.positions[c as usize]);
        let u = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let v = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let cross = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let area2 = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        if area2 < MIN_AREA2 {
            return Err(invalid(format!("{label}: degenerate triangle {t:?} (zero area)")));
        }
        for (x, y) in [(a, b), (b, c), (c, a)] {
            *directed.entry((x, y)).or_insert(0) += 1;
        }
    }
    for (&(x, y), &n) in &directed {
        if n != 1 {
            return Err(invalid(format!("{label}: edge {x}-{y} used {n} times in one direction (inconsistent orientation)")));
        }
        if directed.get(&(y, x)).copied().unwrap_or(0) != 1 {
            return Err(invalid(format!("{label}: open edge {x}-{y} (surface not closed)")));
        }
    }
    Ok(())
}

pub fn validate(mesh: &KitMesh) -> Result<Report, KitError> {
    let triangles = mesh.indices.len() / 3;
    if mesh.indices.len() % 3 != 0 {
        return Err(invalid("index count is not a multiple of three"));
    }
    if triangles == 0 {
        return Err(invalid("no triangles"));
    }
    if triangles > MAX_TRIANGLES {
        return Err(KitError::Budget(triangles));
    }
    if mesh.solid.start != 0 || mesh.solid.end != mesh.glass.start || mesh.glass.end != mesh.indices.len() {
        return Err(invalid("material ranges must cover the index buffer in order"));
    }
    for (i, p) in mesh.positions.iter().enumerate() {
        if !p.iter().all(|v| v.is_finite()) {
            return Err(invalid(format!("vertex {i} is not finite")));
        }
        if p[0].abs() > 1.0 + BOX_EPS || p[1].abs() > 1.0 + BOX_EPS {
            return Err(invalid(format!("vertex {i} leaves the unit box: {p:?}")));
        }
        if p[2] < Z_MIN - BOX_EPS || p[2] > Z_MAX + BOX_EPS {
            return Err(invalid(format!("vertex {i} leaves the depth range: {p:?}")));
        }
    }
    if let Some(&i) = mesh.indices.iter().find(|&&i| i as usize >= mesh.positions.len()) {
        return Err(invalid(format!("index {i} out of range")));
    }
    check_range(mesh, &mesh.solid, "solid")?;
    check_range(mesh, &mesh.glass, "glass")?;
    Ok(Report { triangles, vertices: mesh.positions.len() })
}
```

Add `pub mod validate;` to `kit/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-render kit::validate`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/kit
git commit -m "Kit: the validator, a closed oriented surface in the box or no export

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Mesh emission by face subdivision

**Files:**
- Create: `crates/render/src/kit/mesh.rs`
- Modify: `crates/render/src/kit/mod.rs` (add `pub mod mesh;`)
- Test: inline tests in `mesh.rs`

**Interfaces:**
- Consumes: `Evaluation`, `Interval` (Task 6), `geom::{boolean, is_empty, Shapes, OverlayRule, signed_area}` (Task 3), `KitMesh`, `validate` (Task 7), `i_triangle::float::unchecked::UncheckedTriangulatable`.
- Produces: `pub fn build_mesh(ev: &Evaluation) -> Result<KitMesh, KitError>` (validated before return).

The construction, per material (solid first, then glass):

1. Collect the material's regions per interval `R[k]`.
2. Subdivide the plane into faces of constant membership: start with the union `U` of all `R[k]`; for each `k`, split every face `f` into `f ∩ R[k]` and `f − R[k]`, dropping empties, carrying a membership bitmask. Every face is an i_overlay output of the same inputs, so neighbouring faces share boundary vertices bit for bit.
3. Intern every vertex `(x, y, z)` by its float bits in one pool: the same coordinates always give the same index.
4. Caps: triangulate each face once (unchecked, no simplification, outer counterclockwise, holes clockwise as `orient` guarantees; triangle indices refer to the face's input points in order, which the test `triangulation_keeps_the_input_points` pins). At every level `z_k`, for each face: solid below and not above gives a front cap (triangles as is, lifted to `z_k`); solid above and not below gives a back cap (reversed).
5. Walls: for every face boundary edge `a -> b` (outer contours counterclockwise, holes clockwise, so the face is on the left of the edge) and every interval `k` in the face's membership, if the face across the edge (found through an undirected edge map) is not a member of `k`, emit the wall quad on `a -> b` between `z_k` and `z_{k+1}`: with `f` = front vertices (at `z_{k+1}`) and `b` = back vertices (at `z_k`), triangles `[fa, ba, bb]` and `[fa, bb, fb]`, the designer's pattern.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::doc::*;
    use crate::kit::levels::evaluate;
    use crate::kit::validate::validate;

    fn part(id: &str, shape: Shape, mode: Mode, front: f32, back: f32) -> Part {
        Part {
            id: id.into(),
            name: id.into(),
            visible: true,
            locked: false,
            shape,
            transform: Transform::default(),
            mode,
            depth: Depth { front, back },
            material: Material::Solid,
            effects: Effects::default(),
        }
    }

    fn design(parts: Vec<Part>) -> Design {
        Design { version: 2, name: "t".into(), colors: vec!["#ffffff".into()], parts, groups: vec![] }
    }

    fn mesh_of(parts: Vec<Part>) -> KitMesh {
        build_mesh(&evaluate(&design(parts)).unwrap()).unwrap()
    }

    #[test]
    fn triangulation_keeps_the_input_points() {
        // The whole cap scheme relies on triangle indices pointing at the
        // face's own points, in order, with nothing added or dropped.
        let face: Vec<Vec<[f32; 2]>> = vec![
            vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [0.0, 0.5], [-1.0, 1.0]],
            vec![[-0.5, -0.5], [-0.5, 0.0], [0.0, 0.0], [0.0, -0.5]],
        ];
        let (count, tris) = triangulate_face(&face).unwrap();
        assert_eq!(count, 9);
        assert!(tris.iter().flatten().all(|&i| (i as usize) < 9));
        assert_eq!(tris.len(), 9 + 2 * 1 - 2, "n + 2h - 2 triangles for one hole");
    }

    #[test]
    fn a_disc_is_a_closed_slab() {
        let m = mesh_of(vec![part("a", Shape::Circle, Mode::Add, 0.1, -0.1)]);
        let r = validate(&m).unwrap();
        assert!(r.triangles > 500);
        assert!(m.glass.is_empty());
        let zs: Vec<f32> = m.positions.iter().map(|p| p[2]).collect();
        assert!(zs.iter().all(|z| (*z - 0.1).abs() < 1e-6 || (*z + 0.1).abs() < 1e-6));
    }

    #[test]
    fn ring_with_recessed_plate_is_watertight() {
        let ring = part("ring", Shape::Ring { thickness: 0.28 }, Mode::Add, 0.1, -0.1);
        let mut plate = part("plate", Shape::Circle, Mode::Add, -0.4, -0.6);
        plate.transform = Transform { w: 0.55, h: 0.55, ..Transform::default() };
        let m = mesh_of(vec![ring, plate]);
        validate(&m).unwrap();
        let levels: std::collections::BTreeSet<i32> = m.positions.iter().map(|p| (p[2] * 1000.0).round() as i32).collect();
        assert_eq!(levels.into_iter().collect::<Vec<_>>(), vec![-600, -400, -100, 100]);
    }

    #[test]
    fn crossing_bands_stay_watertight() {
        // A cut bar through a disc at a shallower band than the disc: the
        // bar's boundary crosses the disc's region boundary at a level
        // that is not the disc's, which is where T-junctions would arise.
        let disc = part("d", Shape::Circle, Mode::Add, 0.1, -0.3);
        let mut bar = part("b", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        bar.transform = Transform { w: 1.5, h: 0.15, rotation: 20.0, ..Transform::default() };
        let m = mesh_of(vec![disc, bar]);
        validate(&m).unwrap();
    }

    #[test]
    fn stacked_same_footprint_parts_fuse_without_internal_faces() {
        let a = part("a", Shape::Circle, Mode::Add, 0.1, -0.1);
        let b = part("b", Shape::Circle, Mode::Add, -0.1, -0.3);
        let fused = mesh_of(vec![a.clone(), b]);
        validate(&fused).unwrap();
        let single = mesh_of(vec![part("c", Shape::Circle, Mode::Add, 0.1, -0.3)]);
        // A cap triangle has all three vertices at one z, a wall triangle
        // never does. The outer caps match the single slab's, and the
        // shared level has none.
        let caps_at = |m: &KitMesh, z: f32| {
            m.indices
                .chunks_exact(3)
                .filter(|t| t.iter().all(|&i| (m.positions[i as usize][2] - z).abs() < 1e-6))
                .count()
        };
        assert_eq!(caps_at(&fused, 0.1), caps_at(&single, 0.1));
        assert_eq!(caps_at(&fused, -0.3), caps_at(&single, -0.3));
        assert_eq!(caps_at(&fused, -0.1), 0);
    }

    #[test]
    fn glass_gets_its_own_closed_range() {
        let ring = part("ring", Shape::Ring { thickness: 0.28 }, Mode::Add, 0.1, -0.1);
        let mut inlay = part("g", Shape::Circle, Mode::Add, 0.1, -0.1);
        inlay.material = Material::Glass;
        inlay.transform = Transform { w: 0.5, h: 0.5, ..Transform::default() };
        let m = mesh_of(vec![ring, inlay]);
        validate(&m).unwrap();
        assert!(!m.glass.is_empty());
        assert!(!m.solid.is_empty());
    }

    #[test]
    fn same_design_gives_the_same_mesh_twice() {
        let ring = part("ring", Shape::Ring { thickness: 0.28 }, Mode::Add, 0.1, -0.1);
        let mut plate = part("plate", Shape::Star { points: 5, inner: 0.5 }, Mode::Add, -0.4, -0.6);
        plate.transform = Transform { w: 0.55, h: 0.55, rotation: 13.0, ..Transform::default() };
        let d = design(vec![ring, plate]);
        let m1 = build_mesh(&evaluate(&d).unwrap()).unwrap();
        let m2 = build_mesh(&evaluate(&d).unwrap()).unwrap();
        assert_eq!(m1, m2);
    }

    #[test]
    fn caps_face_outward() {
        // Front caps at +z wind counterclockwise seen from +z, back caps
        // clockwise; the walls then follow from the validator's edge pairing.
        let m = mesh_of(vec![part("a", Shape::Circle, Mode::Add, 0.1, -0.1)]);
        let area = |t: &[u32]| {
            let p = |i: u32| m.positions[i as usize];
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        };
        let mut front = 0;
        let mut back = 0;
        for t in m.indices.chunks_exact(3) {
            let zs: Vec<f32> = t.iter().map(|&i| m.positions[i as usize][2]).collect();
            if zs.iter().all(|z| (*z - 0.1).abs() < 1e-6) {
                assert!(area(t) > 0.0, "front cap triangle {t:?} winds clockwise");
                front += 1;
            } else if zs.iter().all(|z| (*z + 0.1).abs() < 1e-6) {
                assert!(area(t) < 0.0, "back cap triangle {t:?} winds counterclockwise");
                back += 1;
            }
        }
        assert!(front > 200 && back > 200);
    }

    #[test]
    fn more_than_64_intervals_is_a_clear_error() {
        let regions: Vec<Shapes> = vec![Vec::new(); 65];
        let levels = vec![0.0f32; 66];
        let mut pool = Pool { positions: Vec::new(), index: HashMap::new() };
        let mut out = Vec::new();
        let e = emit_material(&regions, &levels, &mut pool, &mut out).unwrap_err();
        assert!(matches!(e, KitError::Invalid(ref s) if s.contains("64")), "{e}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-render kit::mesh`
Expected: compile error, `build_mesh` and `triangulate_face` not found.

- [ ] **Step 3: Implement mesh emission**

`crates/render/src/kit/mesh.rs`:

```rust
//! From regions per depth interval to a watertight mesh. The plane is
//! subdivided into faces of constant membership across all intervals,
//! so every cap is a face triangulated once and lifted to a level, and
//! every wall sits on a face edge whose two sides disagree about an
//! interval. All faces come from the same fixed-scale kernel, so they
//! share boundary vertices bit for bit and no T-junction can exist.

use crate::kit::geom::{boolean, is_empty, OverlayRule, Shapes};
use crate::kit::levels::{Evaluation, Interval};
use crate::kit::validate::{validate, KitMesh};
use crate::kit::KitError;
use i_triangle::float::unchecked::UncheckedTriangulatable;
use std::collections::HashMap;

type Face = Vec<Vec<[f32; 2]>>; // outer contour first, then holes

struct FaceInfo {
    face: Face,
    /// Bit k set: this face is material in interval k.
    members: u64,
    /// Triangles as indices into the face's own points (outer then holes).
    tris: Vec<[u32; 3]>,
}

/// Triangulates one face. Returns the point count and triangles whose
/// indices address the face's points in contour order; no points are
/// added or dropped (unchecked triangulation does not simplify).
pub(crate) fn triangulate_face(face: &Face) -> (usize, Vec<[u32; 3]>) {
    let count: usize = face.iter().map(|c| c.len()).sum();
    let shape: Vec<Vec<[f32; 2]>> = face.clone();
    let raw = shape.unchecked_triangulate();
    let tri = raw.to_triangulation::<u32>();
    debug_assert_eq!(tri.points.len(), count, "unchecked triangulation must keep every point");
    let tris = tri.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    (count, tris)
}

fn subdivide(regions: &[Shapes]) -> Result<Vec<FaceInfo>, KitError> {
    let mut all: Shapes = Vec::new();
    for r in regions {
        if is_empty(r) {
            continue;
        }
        all = if all.is_empty() { r.clone() } else { boolean(&all, r, OverlayRule::Union)? };
    }
    let mut faces: Vec<(Face, u64)> = all.into_iter().map(|f| (f, 0u64)).collect();
    for (k, r) in regions.iter().enumerate() {
        if is_empty(r) {
            continue;
        }
        let mut next: Vec<(Face, u64)> = Vec::new();
        for (face, members) in faces {
            let one: Shapes = vec![face];
            for f in boolean(&one, r, OverlayRule::Intersect)? {
                next.push((f, members | (1u64 << k)));
            }
            for f in boolean(&one, r, OverlayRule::Difference)? {
                next.push((f, members));
            }
        }
        faces = next;
    }
    Ok(faces
        .into_iter()
        .filter(|(_, m)| *m != 0)
        .map(|(face, members)| {
            let (_, tris) = triangulate_face(&face);
            FaceInfo { face, members, tris }
        })
        .collect())
}

struct Pool {
    positions: Vec<[f32; 3]>,
    index: HashMap<[u32; 3], u32>,
}

impl Pool {
    fn get(&mut self, p: [f32; 3]) -> u32 {
        let key = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
        *self.index.entry(key).or_insert_with(|| {
            self.positions.push(p);
            (self.positions.len() - 1) as u32
        })
    }
}

fn emit_material(regions: &[Shapes], levels: &[f32], pool: &mut Pool, out: &mut Vec<u32>) -> Result<(), KitError> {
    let faces = subdivide(regions)?;
    let n = regions.len();
    // Caps: per level, per face, where membership changes across the level.
    for info in &faces {
        let pts: Vec<[f32; 2]> = info.face.iter().flatten().copied().collect();
        for (li, &z) in levels.iter().enumerate() {
            let below = li > 0 && info.members & (1u64 << (li - 1)) != 0;
            let above = li < n && info.members & (1u64 << li) != 0;
            if below == above {
                continue;
            }
            for t in &info.tris {
                let a = pool.get([pts[t[0] as usize][0], pts[t[0] as usize][1], z]);
                let b = pool.get([pts[t[1] as usize][0], pts[t[1] as usize][1], z]);
                let c = pool.get([pts[t[2] as usize][0], pts[t[2] as usize][1], z]);
                if below {
                    out.extend_from_slice(&[a, b, c]); // front cap faces +z
                } else {
                    out.extend_from_slice(&[a, c, b]); // back cap faces -z
                }
            }
        }
    }
    // Walls: directed edges with the face on the left; the neighbour is
    // whoever owns the reverse edge.
    let mut owner: HashMap<([u32; 2], [u32; 2]), usize> = HashMap::new();
    let key = |p: [f32; 2]| [p[0].to_bits(), p[1].to_bits()];
    for (fi, info) in faces.iter().enumerate() {
        for c in &info.face {
            for i in 0..c.len() {
                owner.insert((key(c[i]), key(c[(i + 1) % c.len()])), fi);
            }
        }
    }
    for info in &faces {
        for c in &info.face {
            for i in 0..c.len() {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                let across = owner.get(&(key(b), key(a))).map(|&j| faces[j].members).unwrap_or(0);
                for k in 0..n {
                    let mine = info.members & (1u64 << k) != 0;
                    let theirs = across & (1u64 << k) != 0;
                    if mine && !theirs {
                        let (z0, z1) = (levels[k], levels[k + 1]);
                        let fa = pool.get([a[0], a[1], z1]);
                        let fb = pool.get([b[0], b[1], z1]);
                        let ba = pool.get([a[0], a[1], z0]);
                        let bb = pool.get([b[0], b[1], z0]);
                        out.extend_from_slice(&[fa, ba, bb]);
                        out.extend_from_slice(&[fa, bb, fb]);
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn build_mesh(ev: &Evaluation) -> Result<KitMesh, KitError> {
    let mut pool = Pool { positions: Vec::new(), index: HashMap::new() };
    let mut indices: Vec<u32> = Vec::new();
    let solid_regions: Vec<Shapes> = ev.intervals.iter().map(|i: &Interval| i.solid.clone()).collect();
    emit_material(&solid_regions, &ev.levels, &mut pool, &mut indices)?;
    let solid_end = indices.len();
    let glass_regions: Vec<Shapes> = ev.intervals.iter().map(|i| i.glass.clone()).collect();
    emit_material(&glass_regions, &ev.levels, &mut pool, &mut indices)?;
    let mesh = KitMesh {
        positions: pool.positions,
        solid: 0..solid_end,
        glass: solid_end..indices.len(),
        indices,
    };
    validate(&mesh)?;
    Ok(mesh)
}
```

Add `pub mod mesh;` to `kit/mod.rs`.

If `triangulation_keeps_the_input_points` fails because the unchecked triangulation reorders or drops points, switch `triangulate_face` to `i_triangle::float::triangulator::Triangulator` with validation disabled per its docs (the `unchecked` path is documented as "without validation or simplification"; the test exists precisely to catch a change in that contract). If `crossing_bands_stay_watertight` fails with an open edge, print the offending edge's two positions: the cause will be two faces disagreeing on a shared vertex, which means an overlay did not get identical inputs; fix by feeding the SAME `Shapes` value to both the intersect and the difference call (the code above already does).

**Amendment (controller ruling before dispatch; binding where it differs from the Step 3 code above):**

1. `triangulate_face` returns `Result<(usize, Vec<[u32; 3]>), KitError>`. A point count that differs from the input, or an index at or beyond it, is `Err(KitError::Invalid("triangulation changed the point set".into()))`, not a debug assertion: a release build must never emit indices into a different point set. `subdivide` propagates the error with `?` (a loop instead of the `map` closure).
2. The membership mask is a `u64`, so `emit_material` starts with `if regions.len() > 64 { return Err(KitError::Invalid(format!("{} depth intervals, at most 64", regions.len()))); }`. A design with that many bands is a corrupt file, and a shift by 64 or more would panic.
3. Cap winding is pinned by `caps_face_outward`. If the triangulation turns out to emit clockwise triangles for a counterclockwise contour, swap the two `extend_from_slice` lines in the cap loop (front caps reversed, back caps as is) and say so in the report; the test holds either way.
4. Two outcomes must be reported as NEEDS_CONTEXT with the offending positions, never patched around: the validator rejecting cap triangles as degenerate (straight runs carry collinear points, which a triangulator may fan into zero-area triangles), and a watertight test failing with an open edge (a T-junction between faces split on only one side). The controller holds the fixes for both.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-render kit::mesh`
Expected: 9 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/kit
git commit -m "Kit: faces of constant membership give caps and walls that share every vertex

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Export (obj with material sections, mtl, .rhs)

**Files:**
- Create: `crates/render/src/kit/export.rs`
- Modify: `crates/render/src/kit/mod.rs` (add `pub mod export;`)
- Test: inline tests in `export.rs`

**Interfaces:**
- Consumes: `KitMesh` (Task 7), `build_mesh` (Task 8), `evaluate` (Task 6), `Design` (Task 1), `crate::config::SkinConfig::from_rhs` (existing), `crate::mesh::Mesh::from_obj_str` (existing).
- Produces:
  - `pub fn to_obj(mesh: &KitMesh, name: &str) -> String`
  - `pub fn mtl_text(with_glass: bool) -> String`
  - `pub fn export_rhs(design: &Design, name: &str) -> Result<Vec<u8>, KitError>`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::doc::*;

    fn glass_design() -> Design {
        let mut d = Design::default_note();
        d.parts[0].shape = Shape::Ring { thickness: 0.28 };
        d.parts.push(Part {
            id: "g".into(),
            name: "inlay".into(),
            visible: true,
            locked: false,
            shape: Shape::Circle,
            transform: Transform { w: 0.5, h: 0.5, ..Transform::default() },
            mode: Mode::Add,
            depth: Depth::default(),
            material: Material::Glass,
            effects: Effects::default(),
        });
        d
    }

    #[test]
    fn obj_has_one_object_and_a_section_per_material() {
        let ev = crate::kit::levels::evaluate(&glass_design()).unwrap();
        let m = crate::kit::mesh::build_mesh(&ev).unwrap();
        let obj = to_obj(&m, "note");
        assert_eq!(obj.matches("\no ").count(), 1);
        assert!(obj.contains("mtllib note.mtl\n"));
        assert_eq!(obj.matches("usemtl Material.001\n").count(), 1);
        assert_eq!(obj.matches("usemtl Transparent\n").count(), 1);
        assert_eq!(obj.matches("\nf ").count(), m.indices.len() / 3);
        // rhythr's own loader reads it back with every triangle.
        let parsed = crate::mesh::Mesh::from_obj_str(&obj).unwrap();
        assert_eq!(parsed.indices.len(), m.indices.len());
    }

    #[test]
    fn mtl_names_match_the_game_convention() {
        let solid_only = mtl_text(false);
        assert!(solid_only.contains("newmtl Material.001"));
        assert!(!solid_only.contains("Transparent"));
        assert!(mtl_text(true).contains("newmtl Transparent"));
    }

    #[test]
    fn rhs_pack_loads_back_as_a_skin() {
        let bytes = export_rhs(&glass_design(), "designed").unwrap();
        let cfg = crate::config::SkinConfig::from_rhs(&bytes).unwrap();
        assert!(cfg.note_mesh.is_some());
        assert_eq!(cfg.colorset.len(), 3);
        let obj = String::from_utf8(cfg.note_mesh.unwrap()).unwrap();
        assert!(obj.contains("usemtl Transparent"));
    }

    #[test]
    fn an_empty_design_does_not_export() {
        let mut d = Design::default_note();
        d.parts[0].visible = false;
        assert_eq!(export_rhs(&d, "x").unwrap_err(), KitError::Empty);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-render kit::export`
Expected: compile error, `to_obj` not found.

- [ ] **Step 3: Implement the exporter**

`crates/render/src/kit/export.rs`:

```rust
//! The baked output: one .obj with a `usemtl` section per material, the
//! .mtl it references (always bundled, the game crashes without it),
//! and the colorset, in the archive layout the game's importer accepts.

use crate::kit::doc::Design;
use crate::kit::levels::evaluate;
use crate::kit::mesh::build_mesh;
use crate::kit::validate::KitMesh;
use crate::kit::KitError;
use std::fmt::Write as _;

const MATERIAL: &str = "newmtl {name}\nNs 250.000000\nKa 1.000000 1.000000 1.000000\nKd 1.000000 1.000000 1.000000\nKs 0.000000 0.000000 0.000000\nKe 0.000000 0.000000 0.000000\nNi 1.450000\nd 1.000000\nillum 1\n";

pub fn mtl_text(with_glass: bool) -> String {
    let mut s = String::from("# rhythr note kit\n");
    s.push_str(&MATERIAL.replace("{name}", "Material.001"));
    if with_glass {
        s.push('\n');
        // The game ignores `d` and draws every material after the first
        // translucent; the name follows its built-in cube.
        s.push_str(&MATERIAL.replace("{name}", "Transparent"));
    }
    s
}

pub fn to_obj(mesh: &KitMesh, name: &str) -> String {
    let mut s = String::with_capacity(mesh.positions.len() * 36 + mesh.indices.len() * 8);
    s.push_str("# rhythr note kit\nmtllib note.mtl\n");
    let _ = writeln!(s, "o {name}");
    for p in &mesh.positions {
        let _ = writeln!(s, "v {:.6} {:.6} {:.6}", p[0], p[1], p[2]);
    }
    for (label, range) in [("Material.001", &mesh.solid), ("Transparent", &mesh.glass)] {
        if range.is_empty() {
            continue;
        }
        let _ = writeln!(s, "usemtl {label}");
        for t in mesh.indices[range.clone()].chunks_exact(3) {
            let _ = writeln!(s, "f {} {} {}", t[0] + 1, t[1] + 1, t[2] + 1);
        }
    }
    s
}

pub fn export_rhs(design: &Design, name: &str) -> Result<Vec<u8>, KitError> {
    use std::io::Write;
    let mesh = build_mesh(&evaluate(design)?)?;
    let obj = to_obj(&mesh, name);
    let colorset: String = design
        .colors
        .iter()
        .filter(|c| {
            let h = c.trim().trim_start_matches('#');
            h.len() == 6 && h.chars().all(|ch| ch.is_ascii_hexdigit())
        })
        .map(|c| format!("#{}\r\n", c.trim().trim_start_matches('#')))
        .collect();
    let config = format!(r#"{{"NoteSkin":{{"Value":"{name}.obj"}},"ColorSet":{{"Value":"{name}"}}}}"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let zip_err = |e: zip::result::ZipError| KitError::Invalid(format!("zip: {e}"));
    let io_err = |e: std::io::Error| KitError::Invalid(format!("zip: {e}"));
    zw.start_file("config", opts).map_err(zip_err)?;
    zw.write_all(config.as_bytes()).map_err(io_err)?;
    zw.start_file(format!("noteSkin/{name}.obj"), opts).map_err(zip_err)?;
    zw.write_all(obj.as_bytes()).map_err(io_err)?;
    zw.start_file("noteSkin/note.mtl", opts).map_err(zip_err)?;
    zw.write_all(mtl_text(!mesh.glass.is_empty()).as_bytes()).map_err(io_err)?;
    if !colorset.is_empty() {
        zw.start_file(format!("colorSet/{name}.txt"), opts).map_err(zip_err)?;
        zw.write_all(colorset.as_bytes()).map_err(io_err)?;
    }
    Ok(zw.finish().map_err(zip_err)?.into_inner())
}
```

Add `pub mod export;` to `kit/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rhythia-render kit::export`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/kit
git commit -m "Kit: export, one object with a material section per range, mtl always bundled

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: Renderer draws glass like the game

**Files:**
- Modify: `crates/render/src/mesh.rs` (`Vertex` gains `alpha`, loader tracks `usemtl`, `quad()` sets alpha 1)
- Modify: `crates/render/src/renderer.rs:471-475` (vertex layout gains attribute 7)
- Modify: `crates/render/src/note.wgsl:30-66` and `:80-84` (pass the vertex alpha through, multiply in the mesh branch)
- Modify: `crates/render/src/designer.rs` (every `Vertex { pos }` literal gains `alpha: 1.0`)
- Create: `crates/render/examples/kit_render_png.rs`
- Test: inline tests in `mesh.rs`, a render test in `crates/render/tests/kit_render.rs`

**Interfaces:**
- Consumes: `kit::GLASS_ALPHA`.
- Produces: `Vertex { pos: [f32; 3], alpha: f32 }`; `Mesh::from_obj_str` assigns `alpha = GLASS_ALPHA` to every vertex used by a face under any material other than the first `usemtl` of the file (vertices shared between materials are duplicated, keyed by position index and material).

- [ ] **Step 1: Write the failing tests**

In `crates/render/src/mesh.rs` tests:

```rust
    #[test]
    fn vertices_under_a_second_material_carry_the_glass_alpha() {
        let obj = "o x\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nusemtl Material.001\nf 1 2 3\nusemtl Transparent\nf 2 4 3\n";
        let m = Mesh::from_obj_str(obj).unwrap();
        // Vertices 2 and 3 are shared: they appear once opaque, once glass.
        assert_eq!(m.indices.len(), 6);
        let alphas: Vec<f32> = m.indices.iter().map(|&i| m.vertices[i as usize].alpha).collect();
        assert_eq!(&alphas[0..3], &[1.0, 1.0, 1.0]);
        assert!(alphas[3..6].iter().all(|a| (*a - crate::kit::GLASS_ALPHA).abs() < 1e-6));
        assert_eq!(m.vertices.len(), 6);
    }

    #[test]
    fn files_without_materials_are_fully_opaque() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let m = Mesh::from_obj_str(obj).unwrap();
        assert!(m.vertices.iter().all(|v| v.alpha == 1.0));
    }
```

`crates/render/tests/kit_render.rs`:

```rust
//! The renderer draws a kit mesh the way the game does: solid opaque,
//! glass at the game's fixed alpha. Rendered through the real pipeline.
use rhythia_render::kit::{doc::*, export::to_obj, levels::evaluate, mesh::build_mesh, GLASS_ALPHA};

fn ring_with_glass_inlay() -> Design {
    let mut d = Design::default_note();
    d.colors = vec!["#ff0000".into()];
    d.parts[0].shape = Shape::Ring { thickness: 0.28 };
    d.parts.push(Part {
        id: "g".into(),
        name: "inlay".into(),
        visible: true,
        locked: false,
        shape: Shape::Circle,
        transform: Transform { w: 0.5, h: 0.5, ..Transform::default() },
        mode: Mode::Add,
        depth: Depth::default(),
        material: Material::Glass,
        effects: Effects::default(),
    });
    d
}

#[test]
fn glass_inlay_renders_at_the_game_alpha() {
    let design = ring_with_glass_inlay();
    let mesh = build_mesh(&evaluate(&design).unwrap()).unwrap();
    let mut cfg = rhythia_render::config::SkinConfig::default();
    cfg.note_mesh = Some(to_obj(&mesh, "t").into_bytes());
    cfg.colorset = rhythia_render::config::parse_colorset("#ff0000");
    cfg.camera_fov = 34.0;
    cfg.cursor_opacity = 0.0;
    cfg.cursor_trail_enabled = false;
    let mut map = rhythia_formats::map::Map::default();
    map.notes.push(rhythia_formats::map::Note { time_ms: 300, x: 1.0, y: 1.0 });
    let replay = rhythia_render::testing::idle_replay();
    let renderer = rhythia_render::Renderer::new(320, 320, cfg.hud_fonts().as_slice()).unwrap();
    let skin = renderer.prepare_skin(&cfg);
    let params = rhythia_render::scene::SceneParams::from(&cfg);
    let px = renderer.render_still(&params, &cfg, &skin, &replay, &map, 300.0, None).unwrap();
    let at = |x: usize, y: usize| {
        let i = (y * 320 + x) * 4;
        [px[i] as f32, px[i + 1] as f32, px[i + 2] as f32]
    };
    let centre = at(160, 160);
    let ring = at(160, 160 - 60);
    assert!(ring[0] > 200.0, "ring is solid red: {ring:?}");
    // The glass inlay over the black background: red scaled by the alpha.
    let expected = 255.0 * GLASS_ALPHA;
    assert!((centre[0] - expected).abs() < 20.0, "centre {centre:?} expected about {expected}");
}
```

This test needs an idle replay. Add to `crates/render/src/lib.rs` a tiny public helper module (it already builds synthetic replays in examples; move that byte builder here so tests and examples share it):

```rust
/// Helpers for tests and debug examples.
pub mod testing {
    /// A one-frame idle replay in the current file format, for rendering
    /// synthetic scenes.
    pub fn idle_replay() -> rhythia_formats::rhr::Replay {
        let mut b: Vec<u8> = Vec::new();
        b.extend((20260803i32).to_le_bytes());
        b.extend(0i64.to_le_bytes());
        b.push(0);
        b.push(0);
        b.extend(0i32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.push(0);
        b.push(1);
        b.push(2);
        b.extend(b"[]");
        b.push(0);
        b.extend(1.0f32.to_le_bytes());
        b.extend(0i64.to_le_bytes());
        b.extend(100.0f32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.extend(0.0f32.to_le_bytes());
        b.extend((-1i32).to_le_bytes());
        b.push(0);
        b.push(0);
        b.extend(1i32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.extend(0.0f32.to_le_bytes());
        b.extend(0.0f32.to_le_bytes());
        b.extend(1.0f32.to_le_bytes());
        b.push(0);
        rhythia_formats::rhr::Replay::parse(&b).expect("synthetic replay parses")
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-render vertices_under_a_second_material && cargo test -p rhythia-render --test kit_render`
Expected: compile errors (`alpha` field missing, `testing` module missing).

- [ ] **Step 3: Implement**

`crates/render/src/mesh.rs`: change `Vertex` to

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    /// Per-vertex opacity factor: 1.0 for the first material, the game's
    /// fixed glass alpha for every material after it.
    pub alpha: f32,
}
```

In `from_obj_str`, track materials: keep `let mut material: usize = 0; let mut seen: Vec<String> = Vec::new();` and on a line starting with `usemtl ` set `material = index of the name in seen (push if new)`. Replace the vertex resolution so a face vertex is looked up by `(position index, material)` in a `HashMap<(u32, usize), u32>` that pushes `Vertex { pos, alpha: if material == 0 { 1.0 } else { crate::kit::GLASS_ALPHA } }` on first sight. `quad()` sets `alpha: 1.0` on its four vertices. In `designer.rs`, every `Vertex { pos: [...] }` literal becomes `Vertex { pos: [...], alpha: 1.0 }`.

`crates/render/src/renderer.rs:471-475`: `attributes: &wgpu::vertex_attr_array![0 => Float32x3, 7 => Float32],`

`crates/render/src/note.wgsl`: add `@location(7) alpha: f32,` to `VsIn`; add `@location(3) @interpolate(flat) valpha: f32,` to `VsOut`; set `out.valpha = in.alpha;` in both branches of `vs_main`; in `fs_main` replace the mesh branch body `return in.color;` with `return vec4<f32>(in.color.rgb, in.color.a * in.valpha);`.

Add the `testing` module to `lib.rs` as shown.

`crates/render/examples/kit_render_png.rs` (debug tool, renders a design JSON file through the pipeline to a PNG):

```rust
//! Debug tool: renders a kit design (JSON file) through rhythr's real
//! pipeline to a PNG. Usage: kit_render_png <design.json> <out.png>
fn main() {
    let mut args = std::env::args().skip(1);
    let design_path = args.next().expect("design json");
    let out = args.next().expect("out png");
    let json = std::fs::read_to_string(&design_path).unwrap();
    let design: rhythia_render::kit::doc::Design = serde_json::from_str(&json).unwrap();
    let ev = rhythia_render::kit::levels::evaluate(&design).unwrap();
    let mesh = rhythia_render::kit::mesh::build_mesh(&ev).unwrap();
    let obj = rhythia_render::kit::export::to_obj(&mesh, "preview");
    let mut cfg = rhythia_render::config::SkinConfig::default();
    cfg.note_mesh = Some(obj.into_bytes());
    cfg.colorset = rhythia_render::config::parse_colorset(&design.colors.join("\n"));
    cfg.camera_fov = 34.0;
    cfg.cursor_opacity = 0.0;
    cfg.cursor_trail_enabled = false;
    let mut map = rhythia_formats::map::Map::default();
    for (i, t) in [360i64, 480, 620].into_iter().enumerate() {
        map.notes.push(rhythia_formats::map::Note { time_ms: t, x: i as f32, y: 1.0 });
    }
    let replay = rhythia_render::testing::idle_replay();
    let renderer = rhythia_render::Renderer::new(960, 560, cfg.hud_fonts().as_slice()).unwrap();
    let skin = renderer.prepare_skin(&cfg);
    let params = rhythia_render::scene::SceneParams::from(&cfg);
    let px = renderer.render_still(&params, &cfg, &skin, &replay, &map, 300.0, None).unwrap();
    rhythia_render::write_png(std::path::Path::new(&out), &px, 960, 560).unwrap();
    println!("wrote {out} ({} triangles)", mesh.indices.len() / 3);
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo check --workspace --examples --tests && cargo test -p rhythia-render`
Expected: the whole workspace compiles (every `Vertex { pos: ... }` literal anywhere, in examples such as `crates/render/examples/game_probe_skins.rs` too, must gain `alpha: 1.0`; `cargo check --workspace` is what finds them all), every existing test still passes (the vertex layout change touches every mesh path; the designer tests and the obj round trip prove nothing regressed), the two new mesh tests pass, and `kit_render` passes. If the centre pixel is off by more than the tolerance, print `centre` and check that the note sits where the test samples (x 1.0, y 1.0 is the centre cell at 320 by 320); adjust the sample coordinates, not the alpha.

- [ ] **Step 5: Commit**

```bash
git add crates/render/src crates/render/tests crates/render/examples/kit_render_png.rs
git commit -m "Renderer: glass draws at the game's fixed alpha, per vertex, through the obj loader

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: Property tests

**Files:**
- Create: `crates/render/tests/kit_properties.rs`

**Interfaces:**
- Consumes: everything public from `kit::{doc, levels::evaluate, mesh::build_mesh, validate::validate, export::export_rhs}`.

- [ ] **Step 1: Write the property tests**

```rust
//! Random designs against the validator. Any failure here is a kernel
//! bug by definition: the kit promises watertight meshes for every
//! input the editor can produce.
use proptest::prelude::*;
use rhythia_render::kit::doc::*;
use rhythia_render::kit::export::export_rhs;
use rhythia_render::kit::levels::evaluate;
use rhythia_render::kit::mesh::build_mesh;
use rhythia_render::kit::validate::validate;
use rhythia_render::kit::KitError;

fn shape() -> impl Strategy<Value = Shape> {
    prop_oneof![
        Just(Shape::Circle),
        (0.0f32..1.0).prop_map(|rounding| Shape::Square { rounding }),
        (3u32..10, 0.0f32..1.0).prop_map(|(sides, rounding)| Shape::Polygon { sides, rounding }),
        (3u32..9, 0.2f32..0.9).prop_map(|(points, inner)| Shape::Star { points, inner }),
        (0.1f32..0.8).prop_map(|thickness| Shape::Ring { thickness }),
        (0.2f32..1.0).prop_map(|aspect| Shape::Pill { aspect }),
    ]
}

fn effects() -> impl Strategy<Value = Effects> {
    (
        proptest::option::of((1u32..12, 0.05f32..0.4).prop_map(|(count, depth)| Waves { count, depth, style: WaveStyle::Sine })),
        proptest::option::of(0.02f32..0.3),
        proptest::option::of(0.05f32..0.4),
        prop_oneof![Just(Mirror::None), Just(Mirror::X), Just(Mirror::Y), Just(Mirror::Both)],
        proptest::option::of(2u32..7),
    )
        .prop_map(|(waves, round, rim, mirror, repeat)| Effects { waves, round, rim, mirror, repeat })
}

fn part(i: usize) -> impl Strategy<Value = Part> {
    (
        shape(),
        -0.6f32..0.6,
        -0.6f32..0.6,
        0.2f32..1.0,
        0.2f32..1.0,
        0.0f32..360.0,
        prop_oneof![3 => Just(Mode::Add), 1 => Just(Mode::Cut)],
        -1.9f32..0.4,
        0.05f32..0.5,
        prop_oneof![4 => Just(Material::Solid), 1 => Just(Material::Glass)],
        effects(),
    )
        .prop_map(move |(shape, x, y, w, h, rotation, mode, back, span, material, effects)| Part {
            id: format!("p{i}"),
            name: format!("p{i}"),
            visible: true,
            locked: false,
            shape,
            transform: Transform { x, y, w, h, rotation },
            mode,
            depth: Depth { front: (back + span).min(0.5), back },
            material,
            effects,
        })
}

fn design() -> impl Strategy<Value = Design> {
    (1usize..=5).prop_flat_map(|n| {
        let parts: Vec<_> = (0..n).map(part).collect();
        parts.prop_map(|parts| Design { version: 2, name: "p".into(), colors: vec!["#ffffff".into()], parts, groups: vec![] })
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, max_shrink_iters: 2000, ..ProptestConfig::default() })]

    #[test]
    fn every_design_is_watertight_in_the_box_or_cleanly_empty(d in design()) {
        match evaluate(&d) {
            Err(KitError::Empty) => {}
            Err(e) => prop_assert!(false, "unexpected error {e}"),
            Ok(ev) => {
                let m = build_mesh(&ev).unwrap();
                let r = validate(&m).unwrap();
                prop_assert!(r.triangles <= 40_000);
                prop_assert!(m.positions.iter().all(|p| p[0].abs() <= 1.0001 && p[1].abs() <= 1.0001));
                let m2 = build_mesh(&evaluate(&d).unwrap()).unwrap();
                prop_assert_eq!(m, m2);
                let bytes = export_rhs(&d, "p").unwrap();
                prop_assert!(rhythia_render::config::SkinConfig::from_rhs(&bytes).is_ok());
            }
        }
    }
}
```

- [ ] **Step 2: Run the property tests**

Run: `cargo test -p rhythia-render --test kit_properties -- --nocapture`
Expected: passes. A failure prints a minimal shrunk design; copy it into a named regression test in `kit/mesh.rs` (same style as `crossing_bands_stay_watertight`), fix the kernel, and keep the regression test. Common first failures and their fixes: a part with `w` or `h` so small after rounding that the region vanishes (handled by `is_empty` checks; make sure `subdivide` skips empty regions), repeat copies landing exactly on each other (the nonzero clean merges them; if an overlay complains about a scale, the rotated copy exceeded the i32 range, which the box prevents).

- [ ] **Step 3: Commit**

```bash
git add crates/render/tests/kit_properties.rs
git commit -m "Kit: three hundred random designs a run, every mesh watertight or cleanly empty

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 12: Cross-platform build check and in-game probe

**Files:**
- Modify: `docs/USER-GUIDE.md` (one paragraph under the skins section: import skins from a local folder, the game rejects network paths)
- No code changes beyond fixes the checks demand.

- [ ] **Step 1: Run the full checks on Linux**

Run: `cargo fmt --all && cargo test && cargo clippy -p rhythia-render && scripts/preflight.sh`
Expected: all tests pass, no new clippy warnings in `crates/render/src/kit`, preflight passes (the AUR check passes because no version changed).

- [ ] **Step 2: Cross-compile the Windows target**

Run: `cargo check -p rhythia-gui --target x86_64-pc-windows-msvc` with `cargo-xwin` on the path (the same environment the testbuilds use: `PATH` includes `~/.cargo/bin` and the tauri CLI).
Expected: compiles. `proptest` is a dev-dependency and never enters the Windows binary; `i_overlay`, `i_triangle` and `kurbo` already cross-compiled for every testbuild since August.

- [ ] **Step 3: Build the Linux container check**

Run: `scripts/build-linux.sh` is the full package build and takes long; for this phase run `podman run --rm -v "$PWD":/work -w /work rhythr-linux-build cargo check -p rhythia-render` instead (the image exists from the last release).
Expected: compiles under glibc 2.35.

- [ ] **Step 4: Render the reference designs and send one to the game**

Write three design JSON files to the scratchpad (a ring with a recessed plate, the ring with a glass inlay, a disc with a rotated cut bar), render each with `cargo run --release -p rhythia-render --example kit_render_png -- <design.json> <out.png>`, and look at the PNGs. Export each with a tiny one-off `cargo run --example` call to `export_rhs` (or add a `--rhs <out.rhs>` third argument to `kit_render_png.rs` that writes `export_rhs(&design, "probe")`), copy the three .rhs files to `/home/killer/RhythiaRenderer/game-probes/kit/` and ask the owner to import them from a local folder and screenshot them in play. The acceptance criterion: all three draw, the glass inlay is translucent, the recessed plate shows parallax, no crash at map end.

- [ ] **Step 5: Commit**

```bash
git add docs/USER-GUIDE.md crates/render/examples/kit_render_png.rs
git commit -m "Kit phase 1 closes: cross-platform checks and the in-game probe set

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Self-review notes

- Spec coverage: document model (Task 1), v1 migration (Task 2), fixed-scale kernel (Task 3), shape sources (Task 4), effects and symmetry (Task 5), level evaluation with groups, glass and normalisation (Task 6), validator (Task 7), ring sharing by construction (Task 8), export with material sections (Task 9), renderer glass alpha and the approach fade already present (Task 10), property tests (Task 11), cross-platform and the in-game acceptance round (Task 12). The editor, IPC commands, turntable, presets and imports as parts belong to phases 2 to 4 and are not in this plan. Bevel and chamfer profiles are not in phase 1 (the spec lists profiles as a later phase feature).
- Deviation from the spec worth knowing: `Shape::Glyph` and `Shape::Svg` are one variant `Shape::Contours { contours, label }` so the kernel never touches fonts or files; the editor resolves them (phase 4) and stores contours.
- Type consistency: `KitMesh` is defined in `validate.rs` and used by `mesh.rs` and `export.rs`; `Shapes` is `geom::Shapes`; `Evaluation.parts` is `Vec<ResolvedPart>`; `GLASS_ALPHA` lives in `kit/mod.rs` and the obj loader reads it from there.
- Review Focus coverage: crossing bands (Task 8), normalisation (Task 6 and 11), empty designs (Task 6 and 9), touching bands (Task 6), v1 modes (Task 2).
