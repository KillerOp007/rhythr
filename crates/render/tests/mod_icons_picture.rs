//! The game's mod icons reach the picture: under the health bar while the
//! run plays, and in the Mods box of the results screen. The icons here are
//! stand-ins (plain magenta blocks, a colour nothing else in a frame has),
//! because the real ones belong to the game and are not in this repository.

use rhythia_render::config::SkinConfig;

const W: u32 = 640;
const H: u32 = 360;

fn block() -> Vec<u8> {
    let mut img = image::RgbaImage::new(135, 135);
    for p in img.pixels_mut() {
        *p = image::Rgba([255, 0, 255, 255]);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).expect("png");
    out.into_inner()
}

fn scene(icons: bool) -> (rhythia_render::Renderer, SkinConfig, rhythia_formats::map::Map, rhythia_formats::rhr::Replay) {
    let mut cfg = SkinConfig { cursor_opacity: 0.0, cursor_trail_enabled: false, ..Default::default() };
    if icons {
        cfg.mod_icons = vec![("mod_45".into(), block()), ("mod_hardrock".into(), block())];
    }
    let mut map = rhythia_formats::map::Map::default();
    map.notes.push(rhythia_formats::map::Note { time_ms: 300, x: 1.0, y: 1.0 });
    let mut replay = rhythia_render::testing::idle_replay();
    replay.speed = 1.45;
    replay.mods = "[\"mod_hardrock\"]".into();
    let renderer = rhythia_render::Renderer::new(W, H, cfg.hud_fonts().as_slice()).unwrap();
    (renderer, cfg, map, replay)
}

/// Magenta pixels in the rows `rows` of an RGBA frame.
fn magenta(px: &[u8], rows: std::ops::Range<u32>) -> usize {
    rows.flat_map(|y| (0..W).map(move |x| ((y * W + x) * 4) as usize))
        .filter(|&i| px[i] > 200 && px[i + 1] < 60 && px[i + 2] > 200)
        .count()
}

#[test]
fn the_icons_are_drawn_under_the_field_while_the_run_plays() {
    for icons in [true, false] {
        let (renderer, cfg, map, replay) = scene(icons);
        let params = rhythia_render::scene::SceneParams::from(&cfg);
        let skin = renderer.prepare_skin(&cfg);
        let hud = rhythia_render::hud::HudState::new(&map, &replay);
        let px = renderer.render_still(&params, &cfg, &skin, &replay, &map, 300.0, Some(&hud)).unwrap();
        let below = magenta(&px, H / 2..H);
        if icons {
            // Two blocks of 0.048 of the short edge each: some 17 px square.
            assert!(below > 400, "{below} magenta pixels in the lower half");
            assert_eq!(magenta(&px, 0..H / 2), 0, "and none above the middle");
        } else {
            assert_eq!(magenta(&px, 0..H), 0, "no icons, nothing of their colour");
        }
    }
}

#[test]
fn the_icons_are_drawn_in_the_mods_box_of_the_results_screen() {
    for icons in [true, false] {
        let (renderer, cfg, map, replay) = scene(icons);
        let hud = rhythia_render::hud::HudState::new(&map, &replay);
        let px = renderer.render_results(&replay, &map, &hud, &cfg, None).unwrap();
        let in_box = magenta(&px, (H as f32 * 0.70) as u32..(H as f32 * 0.85) as u32);
        if icons {
            // Two blocks of 0.085 of the short edge: some 30 px square.
            assert!(in_box > 1200, "{in_box} magenta pixels in the Mods box");
            assert_eq!(magenta(&px, 0..(H as f32 * 0.68) as u32), 0, "and none above it");
        } else {
            assert_eq!(magenta(&px, 0..H), 0, "no icons, nothing of their colour");
        }
    }
}

/// One renderer, two configs in a row: the atlas follows the config it is
/// handed, so a render after the installation was connected shows the icons
/// and one for a config without them shows none.
#[test]
fn the_atlas_follows_the_config() {
    let (renderer, with, map, replay) = scene(true);
    let without = SkinConfig { mod_icons: Vec::new(), ..with.clone() };
    let hud = rhythia_render::hud::HudState::new(&map, &replay);
    let count = |cfg: &SkinConfig| magenta(&renderer.render_results(&replay, &map, &hud, cfg, None).unwrap(), 0..H);
    assert_eq!(count(&without), 0);
    assert!(count(&with) > 1200);
    assert_eq!(count(&without), 0);
    assert!(count(&with) > 1200);
}
