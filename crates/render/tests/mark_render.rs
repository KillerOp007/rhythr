//! The hidden Autoplay mark goes through the real pipeline: a fraction of a
//! level on every pixel, both halves of the sign, black kept near black.
use rhythia_render::mark::{pattern, seed_from_hex, DEV_KEY_HEX};

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
fn the_mark_changes_every_frame_by_a_fraction_of_a_level_and_keeps_black_near_black() {
    let (renderer, cfg, params, map, replay) = scene();
    let skin = renderer.prepare_skin(&cfg);
    let off = renderer.render_still(&params, &cfg, &skin, &replay, &map, 300.0, None).unwrap();
    let seed = seed_from_hex(DEV_KEY_HEX).unwrap();
    renderer.set_mark(Some(&pattern(&seed)));
    assert!(renderer.mark_enabled());
    let on = renderer.render_still(&params, &cfg, &skin, &replay, &map, 300.0, None).unwrap();
    let n = off.len() / 4;
    let mut diff = 0.0f64;
    // On the black background the floor lifts every pixel to 1 and the
    // pattern then moves it to 0, 1 or 2: both tails must be populated.
    let (mut black, mut down, mut up) = (0usize, 0usize, 0usize);
    for i in 0..n {
        let d = on[i * 4] as f64 - off[i * 4] as f64;
        diff += d.abs();
        if off[i * 4] == 0 && off[i * 4 + 1] == 0 && off[i * 4 + 2] == 0 {
            black += 1;
            match on[i * 4] {
                0 => down += 1,
                2 => up += 1,
                _ => {}
            }
        }
    }
    let mean_abs = diff / n as f64;
    assert!(mean_abs > 0.3 && mean_abs < 1.4, "mean abs change {mean_abs}");
    assert!(black > n / 2, "the test scene should be mostly black: {black} of {n}");
    assert!(down > black / 10, "the negative half is missing: {down} of {black} black pixels went to 0");
    assert!(up > black / 10, "the positive half is missing: {up} of {black} black pixels went to 2");
    // a pixel in the empty top-left corner: black becomes 1 and the mark
    // moves it by at most 0.8 times the pattern's 4-unit tail, so 0..=4
    let corner = &on[0..4];
    assert!(corner[0] <= 4 && corner[1] <= 4 && corner[2] <= 4, "corner {:?}", corner);
    assert_eq!(corner[0], corner[1]);
    assert_eq!(corner[1], corner[2]);
    renderer.set_mark(None);
    assert!(!renderer.mark_enabled());
}

#[test]
fn without_the_mark_black_stays_zero() {
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
