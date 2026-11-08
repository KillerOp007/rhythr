//! A real map (the long test map, 5004 notes): every note registers, no flag
//! is orphaned, and the time base normaliser leaves the synthetic run alone.
use rhythia_render::autoplay::{generate, Aim, AutoplaySettings, Movement};

fn real_map(name: &str) -> rhythia_formats::map::Map {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata").join(name);
    rhythia_formats::map::Map::from_path(p).expect("test map")
}

/// The long map (3402 notes, 2002 of them in streams) and the hardrock
/// map (notes beyond the grid): every note registers at every speed mod,
/// in both movements, and the time base normaliser leaves the run alone.
#[test]
fn a_real_map_is_a_full_combo_at_every_speed() {
    let dice = rhythia_render::mark::pattern(&rhythia_render::mark::seed_from_hex(rhythia_render::mark::DEV_KEY_HEX).unwrap());
    for name in ["pass_long_map_map_json.json", "hardrock_map_map_json.json"] {
        let map = real_map(name);
        for (speed, movement, aim) in [(0.8f32, Movement::Dance, Aim::Flow), (1.0, Movement::Dance, Aim::Centre), (1.0, Movement::Normal, Aim::Flow), (1.5, Movement::Normal, Aim::Flow), (2.0, Movement::Dance, Aim::Flow), (2.0, Movement::Dance, Aim::Centre)] {
            let s = AutoplaySettings { speed_mod: speed, movement, aim, ..Default::default() };
            let mut r = generate(&map, &s, 60, &dice).unwrap();
            let out = rhythia_sim::hitreg::match_hits(&map.notes, &r.frames, rhythia_sim::hitreg::hit_window_ms(&r));
            assert_eq!(out.derived_hits() as usize, map.notes.len(), "{name} at {speed} {aim:?}");
            assert_eq!(out.orphan_flags, 0, "{name} at {speed} {aim:?}");
            // the matcher's first pass is timing only: check the geometry
            // the game would check, note by note, at the flag frame
            let flags: Vec<_> = r.frames.iter().filter(|f| f.hit).collect();
            assert_eq!(flags.len(), map.notes.len());
            let mut last = f64::MIN;
            for (i, (n, f)) in map.notes.iter().zip(&flags).enumerate() {
                let (wx, wy) = rhythia_render::scene::grid_to_world(n.x, n.y);
                let (cx, cy) = (f.x, f.y);
                // a chord's notes are served from one spot between them (no
                // margin to spare by the chord rule); a lone note keeps the
                // planner's margin
                let chord = (i > 0 && map.notes[i - 1].time_ms == n.time_ms) || (i + 1 < map.notes.len() && map.notes[i + 1].time_ms == n.time_ms);
                let limit = if chord { rhythia_sim::hitreg::HITBOX_HALF - 0.002 } else { rhythia_sim::hitreg::HITBOX_HALF - rhythia_render::autoplay::path::HIT_MARGIN + 1e-4 };
                assert!((cx - wx).abs() <= limit && (cy - wy).abs() <= limit, "{name} at {speed}: note {i} ({} ms) flagged with the cursor {:.3} off per axis", n.time_ms, (cx - wx).abs().max((cy - wy).abs()));
                assert!(f.ms >= n.time_ms as f64 - 1e-6 && f.ms >= last, "{name} at {speed}: note {i} flagged early or out of order");
                last = f.ms;
                assert_eq!(out.results[i].hit_ms, Some(f.ms), "{name} at {speed}: note {i} attributed to another flag");
            }
            let n = r.frames.len();
            rhythia_sim::timebase::normalize(&mut r, &map);
            assert_eq!(r.frames.len(), n);
        }
    }
}
