//! Renders just the results screen to a PNG, so cover handling can be
//! eyeballed without sitting through a whole run.
//! Usage: results_still <replay> <map> <out.png> [width] [height]
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let replay = rhythia_formats::rhr::Replay::from_path(&a[0]).unwrap();
    let map0 = rhythia_formats::map::Map::from_path(&a[1]).unwrap();
    let (map, mods) = rhythia_render::mods::map_for_replay(&map0, &replay);
    let w: u32 = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(1920);
    let h: u32 = a.get(4).and_then(|v| v.parse().ok()).unwrap_or(1080);
    let cfg = rhythia_render::SkinConfig::default();
    let mut params = rhythia_render::scene::SceneParams::from(&cfg);
    params.apply_mods(&mods);
    params.apply_speed(replay.speed);
    let r = rhythia_render::Renderer::new(w, h, cfg.hud_font.as_deref()).unwrap();
    let hud = rhythia_render::hud::HudState::new(&map, &replay);
    let px = r.render_results(&replay, &map, &hud, &cfg, None).unwrap();
    rhythia_render::write_png(std::path::Path::new(&a[2]), &px, w, h).unwrap();
    println!("ok");
}
