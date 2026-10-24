//! Debug tool: renders a kit design (JSON file) through rhythr's real
//! pipeline to a PNG. Usage:
//! kit_render_png <design.json> <out.png> [--rhs <out.rhs>]
//! With `--rhs` the design is also baked into a skin archive the game can
//! import; the skin is named after the design.
fn main() {
    let mut args = std::env::args().skip(1);
    let design_path = args.next().expect("design json");
    let out = args.next().expect("out png");
    let rhs = match args.next().as_deref() {
        None => None,
        Some("--rhs") => Some(args.next().expect("out rhs after --rhs")),
        Some(other) => panic!("unknown argument {other}, expected --rhs <out.rhs>"),
    };
    let json = std::fs::read_to_string(&design_path).unwrap();
    let design: rhythia_render::kit::doc::Design = serde_json::from_str(&json).unwrap();
    let ev = rhythia_render::kit::levels::evaluate(&design).unwrap();
    let mesh = rhythia_render::kit::mesh::build_mesh(&ev).unwrap();
    let obj = rhythia_render::kit::export::to_obj(&mesh, "preview");
    let cfg = rhythia_render::config::SkinConfig {
        note_mesh: Some(obj.into_bytes()),
        colorset: rhythia_render::config::parse_colorset(&design.colors.join("\n")),
        camera_fov: 34.0,
        cursor_opacity: 0.0,
        cursor_trail_enabled: false,
        ..Default::default()
    };
    let mut map = rhythia_formats::map::Map::default();
    for (i, t) in [360i64, 480, 620].into_iter().enumerate() {
        map.notes.push(rhythia_formats::map::Note {
            time_ms: t,
            x: i as f32,
            y: 1.0,
        });
    }
    let replay = rhythia_render::testing::idle_replay();
    let renderer = rhythia_render::Renderer::new(960, 560, cfg.hud_fonts().as_slice()).unwrap();
    let skin = renderer.prepare_skin(&cfg);
    let params = rhythia_render::scene::SceneParams::from(&cfg);
    let px = renderer
        .render_still(&params, &cfg, &skin, &replay, &map, 300.0, None)
        .unwrap();
    rhythia_render::write_png(std::path::Path::new(&out), &px, 960, 560).unwrap();
    println!("wrote {out} ({} triangles)", mesh.indices.len() / 3);
    if let Some(rhs_path) = rhs {
        let bytes = rhythia_render::kit::export::export_rhs(&design, &design.name).unwrap();
        std::fs::write(&rhs_path, &bytes).unwrap();
        println!("wrote {rhs_path} ({} bytes)", bytes.len());
    }
}
