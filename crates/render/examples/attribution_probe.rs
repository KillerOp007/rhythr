//! Measures hit-attribution quality: for every HIT, is the cursor of the
//! matched flag frame inside that note's hit area? For every MISS, did a
//! neighbouring flag's cursor sit inside the missed note's area instead
//! (= a likely swapped attribution)?
//! Usage: attribution_probe <replay.rhr> <map>

const HITBOX_HALF: f32 = rhythia_sim::hitreg::HITBOX_HALF;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let replay = rhythia_formats::rhr::Replay::from_path(&a[0]).unwrap();
    let map = rhythia_formats::map::Map::from_path(&a[1]).unwrap();
    let (map, _mods) = rhythia_render::mods::map_for_replay(&map, &replay);

    let window = rhythia_sim::hitreg::hit_window_ms(&replay);
    let out = rhythia_sim::hitreg::match_hits(&map.notes, &replay.frames, window);

    // Flag frames with their cursor positions.
    let flags: Vec<(f64, f32, f32)> = replay
        .frames
        .iter()
        .filter(|f| f.hit)
        .map(|f| (f.ms, f.x, f.y))
        .collect();
    let world = |n: &rhythia_formats::map::Note| ((n.x - 1.0), (1.0 - n.y));
    let in_area = |fx: f32, fy: f32, n: &rhythia_formats::map::Note| {
        let (wx, wy) = world(n);
        (fx - wx).abs() <= HITBOX_HALF && (fy - wy).abs() <= HITBOX_HALF
    };
    let cursor_at_flag = |ms: f64| -> Option<(f32, f32)> {
        flags
            .iter()
            .find(|(t, _, _)| (t - ms).abs() < 0.01)
            .map(|&(_, x, y)| (x, y))
    };

    let mut hits = 0u32;
    let mut hits_outside = 0u32;
    let mut misses = 0u32;
    let mut miss_swap_candidates = 0u32;
    for r in &out.results {
        let n = &map.notes[r.note_index];
        if r.hit {
            hits += 1;
            let (fx, fy) = cursor_at_flag(r.hit_ms.unwrap()).unwrap();
            if !in_area(fx, fy, n) {
                hits_outside += 1;
                // How far out, and was a NEIGHBOURING frame inside?
                let (wx, wy) = world(n);
                let d = ((fx - wx).abs().max((fy - wy).abs()) - HITBOX_HALF).max(0.0);
                let fm = r.hit_ms.unwrap();
                let idx = replay.frames.partition_point(|f| f.ms < fm - 0.01);
                let near_inside = [-2i64, -1, 1, 2].iter().any(|&off| {
                    let k = idx as i64 + off;
                    if k < 0 || k as usize >= replay.frames.len() {
                        return false;
                    }
                    let f = &replay.frames[k as usize];
                    in_area(f.x, f.y, n)
                });
                println!("  outside: note {} d={:.3} cells, neighbour_frame_inside={}", r.note_index, d, near_inside);
            }
        } else {
            misses += 1;
            // Any flag inside this note's window whose cursor covered
            // THIS note but not the note it was attributed to?
            let nt = n.time_ms as f64;
            for other in &out.results {
                if !other.hit {
                    continue;
                }
                let fm = other.hit_ms.unwrap();
                if (fm - nt).abs() > window {
                    continue;
                }
                let (fx, fy) = cursor_at_flag(fm).unwrap();
                let on = &map.notes[other.note_index];
                if in_area(fx, fy, n) && !in_area(fx, fy, on) {
                    miss_swap_candidates += 1;
                    println!(
                        "  swap candidate: miss note {} @{}ms ({},{}) <- flag @{:.0}ms attributed to note {} @{}ms ({},{})",
                        r.note_index, n.time_ms, n.x, n.y, fm,
                        other.note_index, on.time_ms, on.x, on.y
                    );
                    break;
                }
            }
        }
    }
    println!(
        "hits={hits} hits_with_cursor_OUTSIDE_area={hits_outside} misses={misses} swap_candidates={miss_swap_candidates} orphans={}",
        out.orphan_flags
    );

    if let Ok(range) = std::env::var("RHYTHR_PROBE_RANGE") {
        let (lo, hi) = range.split_once("..").unwrap();
        let (lo, hi): (usize, usize) = (lo.parse().unwrap(), hi.parse().unwrap());
        for i in lo..hi.min(out.results.len()) {
            let r = &out.results[i];
            let n = &map.notes[i];
            let (wx, wy) = world(n);
            let cur = r.hit_ms.and_then(cursor_at_flag);
            println!(
                "note {i} t={} cell=({},{}) world=({:.2},{:.2}) hit={} hit_ms={:?} cursor={:?} covers_self={:?}",
                n.time_ms, n.x, n.y, wx, wy, r.hit, r.hit_ms, cur,
                cur.map(|(fx, fy)| in_area(fx, fy, n)),
            );
        }
    }
}

// Debug-Anhang: mit RHYTHR_PROBE_RANGE="lo..hi" Details für Notenbereich.
