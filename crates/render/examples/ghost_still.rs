//! Split-ghost HUD layout check: one 1920x1080 frame.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let r1 = rhythia_formats::rhr::Replay::from_path(&a[0]).unwrap();
    let r2 = rhythia_formats::rhr::Replay::from_path(&a[1]).unwrap();
    let map = rhythia_formats::map::Map::from_path(&a[2]).unwrap();
    let at: f64 = a[3].parse().unwrap();
    let cfg = rhythia_render::SkinConfig::default();
    let (m1, mo1) = rhythia_render::mods::map_for_replay(&map, &r1);
    let (m2, mo2) = rhythia_render::mods::map_for_replay(&map, &r2);
    let mut params = rhythia_render::scene::SceneParams::from(&cfg);
    params.apply_mods(&mo1);
    params.apply_speed(r1.speed);
    let r = rhythia_render::Renderer::new(1920, 1080, cfg.hud_font.as_deref()).unwrap();
    let skin = r.prepare_skin(&cfg);
    let hud = rhythia_render::hud::HudState::new(&m1, &r1);
    let ghost = rhythia_render::hud::GhostInput {
        state: rhythia_render::hud::HudState::new(&m2, &r2),
        replay: r2,
        color: [1.0, 0.61, 0.26],
        map: m2,
        mods: mo2,
        race: None,
    };
    let px = r
        .render_still_with_ghost(&params, &cfg, &skin, &r1, &m1, at, Some(&hud), Some(&ghost))
        .unwrap();
    rhythia_render::write_png(std::path::Path::new(&a[4]), &px, 1920, 1080).unwrap();
    println!("ok");
}
