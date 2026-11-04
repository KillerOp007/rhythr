//! A real map (the long test map, 5004 notes): every note registers, no flag
//! is orphaned, and the time base normaliser leaves the synthetic run alone.
use rhythia_render::autoplay::{generate, AutoplaySettings};

fn real_map() -> rhythia_formats::map::Map {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/pass_long_map_map_json.json");
    rhythia_formats::map::Map::from_path(p).expect("test map")
}

#[test]
fn a_real_map_is_a_full_combo_at_both_speeds() {
    let map = real_map();
    let dice = rhythia_render::mark::pattern(&rhythia_render::mark::seed_from_hex(rhythia_render::mark::DEV_KEY_HEX).unwrap());
    for speed in [1.0f32, 1.5] {
        let s = AutoplaySettings { speed_mod: speed, ..Default::default() };
        let mut r = generate(&map, &s, 60, &dice).unwrap();
        let out = rhythia_sim::hitreg::match_hits(&map.notes, &r.frames, rhythia_sim::hitreg::hit_window_ms(&r));
        assert_eq!(out.derived_hits() as usize, map.notes.len(), "speed {speed}");
        assert_eq!(out.orphan_flags, 0, "speed {speed}");
        let n = r.frames.len();
        rhythia_sim::timebase::normalize(&mut r, &map);
        assert_eq!(r.frames.len(), n);
    }
}
