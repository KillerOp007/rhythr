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

/// Words the window never shows (spec 5.3, the vocabulary rule). The
/// kernel's own texts for internal failures can contain them.
const BANNED_WORDS: [&str; 15] = [
    "boolean",
    "union",
    "mesh",
    "vertex",
    "modifier",
    "manifold",
    "lattice",
    "triangulation",
    "degenerate",
    "edge",
    "orientation",
    "index",
    "triangle",
    "surface",
    "zip",
];

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
        KitError::Doc(m) | KitError::Invalid(m) => {
            let lower = m.to_lowercase();
            if BANNED_WORDS.iter().any(|w| lower.contains(w)) {
                eprintln!("kit: {m}");
                "this design could not be built; try fewer parts, less rounding or fewer waves"
                    .into()
            } else {
                m.clone()
            }
        }
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

/// A loaded document: the design's JSON, and whether it came from the old
/// designer's format (the editor then keeps no path, so a Save never
/// overwrites the old file with the new format).
#[derive(Serialize, Debug)]
pub struct Loaded {
    pub json: String,
    pub migrated: bool,
}

/// A saved document: version 2 as is (re-serialised so the editor only
/// ever sees serde's spelling), the old designer's format (it has a `P`
/// key) migrates, anything else is not a design file.
pub fn load_document(text: &str) -> Result<Loaded, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not a design file: {e}"))?;
    if value.get("version").and_then(|v| v.as_u64()) == Some(2) {
        let d: Design =
            serde_json::from_value(value).map_err(|e| format!("design file damaged: {e}"))?;
        let json = serde_json::to_string(&d).map_err(|e| e.to_string())?;
        return Ok(Loaded {
            json,
            migrated: false,
        });
    }
    if value.get("P").is_none() {
        return Err("not a design file: neither a kit design nor an old designer document".into());
    }
    let d = migrate::from_v1(text).map_err(|e| user_message(&e))?;
    let json = serde_json::to_string(&d).map_err(|e| e.to_string())?;
    Ok(Loaded {
        json,
        migrated: true,
    })
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
    load_document(&json).map(|l| l.json)
}

#[tauri::command]
pub async fn kit_doc_load(path: String) -> Result<Loaded, String> {
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
            serde_json::from_str::<Design>(&json)
                .map_err(|e| format!("design file damaged: {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use rhythia_render::kit::doc::Design;

    /// The old designer's document, verbatim from the slider fixture in
    /// crates/render/src/kit/migrate.rs's tests, so the two suites agree
    /// on the v1 format.
    const OLD_DESIGNER_FIXTURE: &str = r##"{"P":{"shape":"ngon","n":6,"round":0.3,"hollow":true,
      "thick":0.22,"depth":0.05,"waves":5,"waveDepth":0.25,"waveform":"spike","twist":30},
      "free":null,"symmetry":4,"mirror":false,"colors":["#ff5d6c","#2fd6d0"],"imported":null}"##;

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
            KitError::Invalid("vertex 3 leaves the unit box: [1.2, 0.0, 0.0]".into()),
            KitError::Invalid("solid: degenerate triangle [1, 2, 2]".into()),
            KitError::Invalid("solid: open edge 3-7 (surface not closed)".into()),
        ] {
            let m = user_message(&e).to_lowercase();
            for b in banned {
                assert!(!m.contains(b), "{m} contains {b}");
            }
        }
        // Every internal failure text becomes the one fixed sentence (the
        // loop above only knows the first six words, so say it outright).
        for internal in [
            "solid: open edge 3-7 (surface not closed)",
            "glass: edge 1-2 used 3 times but 2-1 1 times (inconsistent orientation)",
            "index 9 out of range",
            "zip: invalid archive",
        ] {
            assert_eq!(
                user_message(&KitError::Invalid(internal.into())),
                "this design could not be built; try fewer parts, less rounding or fewer waves",
                "{internal}"
            );
        }
        // The kernel's precise, plain-worded texts stay as they are.
        let grew = "part p1 grew beyond the box after rounding: 1.2";
        assert_eq!(user_message(&KitError::Invalid(grew.into())), grew);
        let order = "part p1: front must be above back";
        assert_eq!(user_message(&KitError::Doc(order.into())), order);
        let depth = "part p1: depth must stay within -2..0.5";
        assert_eq!(user_message(&KitError::Doc(depth.into())), depth);
        let intervals = "65 depth intervals, at most 64";
        assert_eq!(
            user_message(&KitError::Invalid(intervals.into())),
            intervals
        );
    }

    #[test]
    fn a_kit_file_loads_as_is_and_an_old_designer_file_migrates() {
        let kit = serde_json::to_string(&Design::default_note()).unwrap();
        let back: Design = serde_json::from_str(&load_document(&kit).unwrap().json).unwrap();
        assert_eq!(back.parts[0].id, "p1");
        assert!(
            !load_document(&kit).unwrap().migrated,
            "a v2 document is not a migration"
        );
        // The old designer's document (the six-sided slider fixture).
        let v1 = OLD_DESIGNER_FIXTURE;
        let d: Design = serde_json::from_str(&load_document(v1).unwrap().json).unwrap();
        assert_eq!(d.version, 2);
        assert_eq!(d.parts.len(), 1);
        assert!(
            load_document(v1).unwrap().migrated,
            "the v1 fixture is a migration"
        );
        assert!(load_document("not json")
            .unwrap_err()
            .starts_with("not a design file"));
        assert!(load_document("[]")
            .unwrap_err()
            .starts_with("not a design file"));
        assert!(load_document("{}")
            .unwrap_err()
            .starts_with("not a design file"));
        assert!(load_document("{\"version\":3}")
            .unwrap_err()
            .starts_with("not a design file"));
    }

    /// Every shape the editor's Add buttons write, inside a part exactly as
    /// store.js's newPart builds it, must parse and evaluate: the kernel and
    /// the editor share one document format.
    #[test]
    fn a_part_built_by_the_editor_parses() {
        let shapes = [
            r#"{"kind":"circle"}"#,
            r#"{"kind":"square","rounding":0}"#,
            r#"{"kind":"polygon","sides":6,"rounding":0}"#,
            r#"{"kind":"star","points":5,"inner":0.5}"#,
            r#"{"kind":"ring","thickness":0.28}"#,
            r#"{"kind":"pill","aspect":0.5}"#,
        ];
        for shape in shapes {
            let json = format!(
                r##"{{"version":2,"name":"t","colors":["#ffffff"],"parts":[{{"id":"p1","name":"Part","visible":true,"locked":false,"shape":{shape},"transform":{{"x":0,"y":0,"w":1,"h":1,"rotation":0}},"mode":"add","depth":{{"front":0.1,"back":-0.1}},"material":"solid","effects":{{"waves":null,"round":null,"rim":null,"mirror":"none","repeat":null}}}}],"groups":[]}}"##
            );
            let d: Design = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{shape}: {e}"));
            let e = evaluate_design(&d).unwrap_or_else(|e| panic!("{shape}: {e}"));
            assert_eq!(e.parts.len(), 1, "{shape}");
            assert!(e.mesh_error.is_none(), "{shape}: {:?}", e.mesh_error);
        }
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
