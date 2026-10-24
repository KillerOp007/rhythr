//! Debug tool for the editor harness: reads a kit design (JSON) on stdin
//! and prints the same payload the desktop app's `kit_evaluate` command
//! returns, so the window's front-end can be driven in a plain browser
//! against the real kernel. `--default` prints the default design instead;
//! `--load <file>` prints what `kit_doc_load` would ({json, migrated}) for
//! a .rhs or .rhsd; `--save <file>` bakes the design on stdin into a .rhs
//! like `kit_export` and prints {path}. Errors print as {error}.
//! Usage: kit_eval_json [--default | --load <file> | --save <file>] < design.json

use rhythia_render::kit::doc::Design;
use rhythia_render::kit::{export, levels, mesh, migrate, KitError, MAX_TRIANGLES};
use serde::Serialize;
use std::io::Read;

#[derive(Serialize)]
struct PartGeometry {
    id: String,
    shapes: rhythia_render::kit::geom::Shapes,
}

#[derive(Serialize)]
struct LevelRegion {
    z0: f32,
    z1: f32,
    solid: rhythia_render::kit::geom::Shapes,
    glass: rhythia_render::kit::geom::Shapes,
}

#[derive(Serialize)]
struct Evaluated {
    levels: Vec<f32>,
    parts: Vec<PartGeometry>,
    intervals: Vec<LevelRegion>,
    triangles: Option<usize>,
    mesh_error: Option<String>,
}

fn message(e: &KitError) -> String {
    match e {
        KitError::Empty => "nothing to show: every part is hidden or cut away".into(),
        KitError::Budget(n) => format!("too detailed: {n} triangles, the limit is {MAX_TRIANGLES}"),
        KitError::Complexity(p, c) => {
            format!("too complex: {p} points in {c} outlines; lower Repeat or Waves")
        }
        KitError::Scale(_) => "a part reaches too far from the centre".into(),
        KitError::Doc(m) | KitError::Invalid(m) => m.clone(),
    }
}

fn print_error(text: String) {
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({ "error": text })).unwrap()
    );
}

/// The desktop app's `load_bytes` rule, repeated here because that function
/// lives in the gui crate: an archive gives up its embedded design, text is
/// taken as is; version 2 opens as is, the old designer's format converts.
fn load(path: &str) -> Result<serde_json::Value, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = if bytes.starts_with(b"PK") {
        export::embedded_design(&bytes)
            .map_err(|e| message(&e))?
            .ok_or_else(|| "this skin carries no editable design".to_string())?
    } else {
        String::from_utf8(bytes).map_err(|_| "not a design file".to_string())?
    };
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("not a design file: {e}"))?;
    if value.get("version").and_then(|v| v.as_u64()) == Some(2) {
        return Ok(serde_json::json!({ "json": text, "migrated": false }));
    }
    if value.get("P").is_none() {
        return Err("not a design file".into());
    }
    let d = migrate::from_v1(&text).map_err(|e| message(&e))?;
    Ok(serde_json::json!({ "json": serde_json::to_string(&d).unwrap(), "migrated": true }))
}

fn save(path: &str, design: &Design) -> Result<serde_json::Value, String> {
    let stem = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".rhs");
    let name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    let name = name.trim();
    let name = if name.is_empty() { "note" } else { name };
    let bytes = export::export_rhs(design, name).map_err(|e| message(&e))?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "path": path }))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--default") {
        println!(
            "{}",
            serde_json::to_string(&Design::default_note()).unwrap()
        );
        return;
    }
    if args.first().map(String::as_str) == Some("--load") {
        match load(args.get(1).expect("--load <file>")) {
            Ok(v) => println!("{}", serde_json::to_string(&v).unwrap()),
            Err(e) => print_error(e),
        }
        return;
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .expect("read stdin");
    let design: Design = match serde_json::from_str(&text) {
        Ok(d) => d,
        Err(e) => {
            print_error(format!("design: {e}"));
            return;
        }
    };
    if args.first().map(String::as_str) == Some("--save") {
        match save(args.get(1).expect("--save <file>"), &design) {
            Ok(v) => println!("{}", serde_json::to_string(&v).unwrap()),
            Err(e) => print_error(e),
        }
        return;
    }
    let ev = match levels::evaluate(&design) {
        Ok(ev) => ev,
        Err(e) => {
            print_error(message(&e));
            return;
        }
    };
    let (triangles, mesh_error) = match mesh::build_mesh(&ev) {
        Ok(m) => (Some(m.indices.len() / 3), None),
        Err(e) => (None, Some(message(&e))),
    };
    let out = Evaluated {
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
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}
