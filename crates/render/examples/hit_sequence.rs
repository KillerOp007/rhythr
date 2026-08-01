//! Renders a frame sequence around a note's hit to prove despawn timing:
//! the note must stay until the recorded hit frame, when the cursor is on
//! it. Usage: hit_sequence <replay> <map> <from_ms> <to_ms> <step_ms> <outdir>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let replay = rhythia_formats::rhr::Replay::from_path(&a[0]).unwrap();
    let map0 = rhythia_formats::map::Map::from_path(&a[1]).unwrap();
    let (map, mods) = rhythia_render::mods::map_for_replay(&map0, &replay);
    let from: f64 = a[2].parse().unwrap();
    let to: f64 = a[3].parse().unwrap();
    let step: f64 = a[4].parse().unwrap();
    let outdir = std::path::PathBuf::from(&a[5]);
    std::fs::create_dir_all(&outdir).unwrap();

    let cfg = rhythia_render::SkinConfig::default();
    let mut params = rhythia_render::scene::SceneParams::from(&cfg);
    params.grid_scale = mods.grid_scale;
    params.apply_speed(replay.speed);
    let r = rhythia_render::Renderer::new(1280, 720, cfg.hud_font.as_deref()).unwrap();
    let skin = r.prepare_skin(&cfg);
    let hud = rhythia_render::hud::HudState::new(&map, &replay);

    let mut t = from;
    let mut i = 0;
    while t <= to {
        let px = r
            .render_still(&params, &cfg, &skin, &replay, &map, t, Some(&hud))
            .unwrap();
        let name = outdir.join(format!("seq_{i:02}_{}ms.png", t.round() as i64));
        rhythia_render::write_png(&name, &px, 1280, 720).unwrap();
        i += 1;
        t += step;
    }
    println!("rendered {i} frames");
}
