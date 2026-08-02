//! One-line validation summary for a replay+map pair: header totals vs
//! derived totals, attribution quality, integrity verdict and signals.
//! Usage: validate_replay <replay.rhr> <map>

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let replay = match rhythia_formats::rhr::Replay::from_path(&a[0]) {
        Ok(r) => r,
        Err(e) => {
            println!("PARSE-FAIL {} {e}", a[0]);
            return;
        }
    };
    let map = match rhythia_formats::map::Map::from_path(&a[1]) {
        Ok(m) => m,
        Err(e) => {
            println!("MAP-FAIL {} {e}", a[1]);
            return;
        }
    };
    let (map, mods) = rhythia_render::mods::map_for_replay(&map, &replay);
    let bound = mods.grid_scale + (0.5 - rhythia_sim::hitreg::CURSOR_EDGE_INSET);
    let half = rhythia_sim::hitreg::HITBOX_HALF;

    let out = rhythia_sim::hitreg::match_hits(
        &map.notes,
        &replay.frames,
        rhythia_sim::hitreg::hit_window_ms(&replay),
    );
    let derived = out.derived_hits();
    let header_hits = replay.frames.iter().filter(|f| f.hit).count() as u32;

    // Attribution quality: hits whose flag cursor (clamped) is outside.
    let flags: Vec<(f64, f32, f32)> = replay
        .frames
        .iter()
        .filter(|f| f.hit)
        .map(|f| (f.ms, f.x, f.y))
        .collect();
    let mut outside = 0u32;
    for r in &out.results {
        if !r.hit {
            continue;
        }
        let fm = r.hit_ms.unwrap();
        let Some(&(_, fx, fy)) = flags.iter().find(|(t, _, _)| (t - fm).abs() < 0.01) else {
            continue;
        };
        let (cx, cy) = (fx.clamp(-bound, bound), fy.clamp(-bound, bound));
        let n = &map.notes[r.note_index];
        let (wx, wy) = (n.x - 1.0, 1.0 - n.y);
        if (cx - wx).abs() > half || (cy - wy).abs() > half {
            outside += 1;
        }
    }

    if std::env::var("VALIDATE_VERBOSE").is_ok() {
        let rep = rhythia_sim::integrity::verify_replay(&replay, &map);
        for c in rep.failed_checks() {
            println!("  failed-check: {} expected={} actual={}", c.name, c.expected, c.actual);
        }
        // err_ms range
        let a0 = rhythia_render::analysis::analyze(&map, &replay);
        let min_err = a0
            .notes
            .iter()
            .filter_map(|n| n.err_ms)
            .fold(f64::MAX, f64::min);
        println!("  min_err_ms={min_err:.1}");
    }
    let analysis = rhythia_render::analysis::analyze(&map, &replay);
    let sig: Vec<String> = analysis
        .signals
        .iter()
        .map(|s| format!("{}:{}", s.severity, s.title.replace(' ', "_")))
        .collect();
    let neg_err = analysis
        .notes
        .iter()
        .filter(|n| n.err_ms.map(|e| e < -0.5).unwrap_or(false))
        .count();

    println!(
        "OK notes={} derived_hits={derived} flag_frames={header_hits} misses={} orphans={} outside={outside} neg_err={neg_err} grid={} verdict={} signals=[{}]",
        map.notes.len(),
        out.results.len() as u32 - derived,
        out.orphan_flags,
        mods.grid_scale,
        analysis.verdict,
        sig.join(",")
    );
}
