//! What an Autoplay render adds to the picture: the ticker when it is on,
//! and nothing else. Black stays black (0.8.0 lifted it by a level for a
//! hidden mark, which later versions do not have).

fn scene() -> (
    rhythia_render::Renderer,
    rhythia_render::config::SkinConfig,
    rhythia_render::scene::SceneParams,
    rhythia_formats::map::Map,
    rhythia_formats::rhr::Replay,
) {
    let cfg = rhythia_render::config::SkinConfig {
        cursor_opacity: 0.0,
        cursor_trail_enabled: false,
        ..Default::default()
    };
    let mut map = rhythia_formats::map::Map::default();
    map.notes.push(rhythia_formats::map::Note { time_ms: 300, x: 1.0, y: 1.0 });
    let replay = rhythia_render::testing::idle_replay();
    let renderer = rhythia_render::Renderer::new(640, 360, cfg.hud_fonts().as_slice()).unwrap();
    let params = rhythia_render::scene::SceneParams::from(&cfg);
    (renderer, cfg, params, map, replay)
}

#[test]
fn black_stays_zero() {
    let (renderer, cfg, params, map, replay) = scene();
    let skin = renderer.prepare_skin(&cfg);
    let off = renderer.render_still(&params, &cfg, &skin, &replay, &map, 300.0, None).unwrap();
    assert_eq!(&off[0..3], &[0, 0, 0]);
}

#[test]
fn the_ticker_is_drawn_when_set() {
    let (renderer, cfg, params, map, replay) = scene();
    let skin = renderer.prepare_skin(&cfg);
    renderer.set_ticker(Some("AUTOPLAY   rendered with rhythr".into()));
    let on = renderer.render_still(&params, &cfg, &skin, &replay, &map, 300.0, None).unwrap();
    renderer.set_ticker(None);
    let off = renderer.render_still(&params, &cfg, &skin, &replay, &map, 300.0, None).unwrap();
    let band_rows = 0..(360.0 * 0.08) as usize;
    let changed = band_rows
        .flat_map(|y| (0..640).map(move |x| (y, x)))
        .filter(|&(y, x)| on[(y * 640 + x) * 4] != off[(y * 640 + x) * 4])
        .count();
    assert!(changed > 200, "the ticker changed {changed} pixels in the top band");
}
