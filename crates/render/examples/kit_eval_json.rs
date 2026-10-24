//! Debug tool for the editor harness: reads a kit design (JSON) on stdin
//! and prints the same payload the desktop app's `kit_evaluate` command
//! returns, so the window's front-end can be driven in a plain browser
//! against the real kernel. `--default` prints the default design instead.
//! Usage: kit_eval_json [--default] < design.json

use rhythia_render::kit::doc::Design;
use rhythia_render::kit::{levels, mesh, KitError, MAX_TRIANGLES};
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
    scale: f32,
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

fn main() {
    if std::env::args().any(|a| a == "--default") {
        println!("{}", serde_json::to_string(&Design::default_note()).unwrap());
        return;
    }
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).expect("read stdin");
    let design: Design = match serde_json::from_str(&text) {
        Ok(d) => d,
        Err(e) => {
            println!("{}", serde_json::to_string(&serde_json::json!({ "error": format!("design: {e}") })).unwrap());
            return;
        }
    };
    let ev = match levels::evaluate(&design) {
        Ok(ev) => ev,
        Err(e) => {
            println!("{}", serde_json::to_string(&serde_json::json!({ "error": message(&e) })).unwrap());
            return;
        }
    };
    let (triangles, mesh_error) = match mesh::build_mesh(&ev) {
        Ok(m) => (Some(m.indices.len() / 3), None),
        Err(e) => (None, Some(message(&e))),
    };
    let out = Evaluated {
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
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}
