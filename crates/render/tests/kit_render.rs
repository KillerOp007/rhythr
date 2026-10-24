//! The renderer draws a kit mesh the way the game does: solid opaque,
//! glass at the game's fixed alpha. Rendered through the real pipeline.
use rhythia_render::kit::{
    doc::*, export::to_obj, levels::evaluate, mesh::build_mesh, GLASS_ALPHA,
};

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
        transform: Transform {
            w: 0.5,
            h: 0.5,
            ..Transform::default()
        },
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
    let cfg = rhythia_render::config::SkinConfig {
        note_mesh: Some(to_obj(&mesh, "t").into_bytes()),
        colorset: rhythia_render::config::parse_colorset("#ff0000"),
        camera_fov: 34.0,
        cursor_opacity: 0.0,
        cursor_trail_enabled: false,
        ..Default::default()
    };
    let mut map = rhythia_formats::map::Map::default();
    map.notes.push(rhythia_formats::map::Note {
        time_ms: 300,
        x: 1.0,
        y: 1.0,
    });
    let replay = rhythia_render::testing::idle_replay();
    let renderer = rhythia_render::Renderer::new(320, 320, cfg.hud_fonts().as_slice()).unwrap();
    let skin = renderer.prepare_skin(&cfg);
    let params = rhythia_render::scene::SceneParams::from(&cfg);
    let px = renderer
        .render_still(&params, &cfg, &skin, &replay, &map, 300.0, None)
        .unwrap();
    let at = |x: usize, y: usize| {
        let i = (y * 320 + x) * 4;
        [px[i] as f32, px[i + 1] as f32, px[i + 2] as f32]
    };
    let centre = at(160, 160);
    let ring = at(160, 160 - 60);
    assert!(ring[0] > 200.0, "ring is solid red: {ring:?}");
    // The glass inlay over the black background: red scaled by the alpha.
    let expected = 255.0 * GLASS_ALPHA;
    assert!(
        (centre[0] - expected).abs() < 20.0,
        "centre {centre:?} expected about {expected}"
    );
}
