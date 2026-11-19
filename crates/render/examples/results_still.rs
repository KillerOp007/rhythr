//! Renders just the results screen to a PNG, so cover handling and the Mods
//! box can be eyeballed without sitting through a whole run.
//! Usage: results_still <replay> <map> <out.png> [width] [height] [game assets dir] [mods json]
//! The game assets directory brings the game's own mod icons; the mods
//! list (`["mod_ghost","mod_nofail"]`) replaces the replay's, to see a row
//! the test replays do not have.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut replay = rhythia_formats::rhr::Replay::from_path(&a[0]).unwrap();
    if let Some(mods) = a.get(6) {
        replay.mods = mods.clone();
    }
    let map0 = rhythia_formats::map::Map::from_path(&a[1]).unwrap();
    let (map, mods) = rhythia_render::mods::map_for_replay(&map0, &replay);
    let w: u32 = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(1920);
    let h: u32 = a.get(4).and_then(|v| v.parse().ok()).unwrap_or(1080);
    let mut cfg = rhythia_render::SkinConfig::default();
    if let Some(dir) = a.get(5).filter(|d| !d.is_empty()) {
        cfg.resolve_builtins(&rhythia_render::BuiltinAssets::load(dir));
    }
    let mut params = rhythia_render::scene::SceneParams::from(&cfg);
    params.apply_mods(&mods);
    params.apply_speed(replay.speed);
    let r = rhythia_render::Renderer::new(w, h, cfg.hud_fonts().as_slice()).unwrap();
    let hud = rhythia_render::hud::HudState::new(&map, &replay);
    let px = r.render_results(&replay, &map, &hud, &cfg, None).unwrap();
    rhythia_render::write_png(std::path::Path::new(&a[2]), &px, w, h).unwrap();
    println!("ok");
}
