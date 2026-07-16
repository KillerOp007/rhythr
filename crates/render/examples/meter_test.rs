//! Debug: render one frame with both error meters enabled.
//! Usage: meter_test <replay> <map> <config> <at_ms> <out.png>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let replay = rhythia_formats::rhr::Replay::from_path(&a[0]).unwrap();
    let map = rhythia_formats::map::Map::from_path(&a[1]).unwrap();
    let mut cfg = rhythia_render::SkinConfig::from_path(&a[2]).unwrap();
    cfg.hud.error_meter.enabled = true;
    cfg.hud.aim_meter.enabled = true;
    let at: f64 = a[3].parse().unwrap();
    let params = rhythia_render::scene::SceneParams::from(&cfg);
    let r = rhythia_render::Renderer::new(1280, 720, cfg.hud_font.as_deref()).unwrap();
    let skin = r.prepare_skin(&cfg);
    let hud = rhythia_render::hud::HudState::new(&map, &replay);
    let px = r
        .render_still(&params, &cfg, &skin, &replay, &map, at, Some(&hud))
        .unwrap();
    rhythia_render::write_png(std::path::Path::new(&a[4]), &px, 1280, 720).unwrap();
    println!("ok");
}
