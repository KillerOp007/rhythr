//! Full forensics for one moment of a replay: every note in a time
//! window with its verdict, the deciding flag frame, the RAW and the
//! game-clamped cursor, and the covers test — for checking exactly what
//! the analyzer shows against the recorded ground truth.
//! Usage: moment_probe <replay.rhr> <map> <from_ms> <to_ms>

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let replay = rhythia_formats::rhr::Replay::from_path(&a[0]).unwrap();
    let map = rhythia_formats::map::Map::from_path(&a[1]).unwrap();
    let from: f64 = a[2].parse().unwrap();
    let to: f64 = a[3].parse().unwrap();
    let (map, mods) = rhythia_render::mods::map_for_replay(&map, &replay);
    let bound = mods.grid_scale + (0.5 - rhythia_sim::hitreg::CURSOR_EDGE_INSET);
    let half = rhythia_sim::hitreg::HITBOX_HALF;

    let out = rhythia_sim::hitreg::match_hits(
        &map.notes,
        &replay.frames,
        rhythia_sim::hitreg::hit_window_ms(&replay),
    );
    let cursor_at = |ms: f64| -> (f32, f32) {
        let i = replay.frames.partition_point(|f| f.ms <= ms);
        let i = i.saturating_sub(1);
        let f = &replay.frames[i];
        if i + 1 < replay.frames.len() {
            let g = &replay.frames[i + 1];
            let k = ((ms - f.ms) / (g.ms - f.ms).max(1e-9)).clamp(0.0, 1.0) as f32;
            (f.x + (g.x - f.x) * k, f.y + (g.y - f.y) * k)
        } else {
            (f.x, f.y)
        }
    };

    println!(
        "mods: grid_scale={} bound=±{:.5} hitbox_half={half}",
        mods.grid_scale, bound
    );
    for r in &out.results {
        let n = &map.notes[r.note_index];
        let nt = n.time_ms as f64;
        if nt < from || nt > to {
            continue;
        }
        let (wx, wy) = (n.x - 1.0, 1.0 - n.y);
        let judge_ms = if r.hit { r.hit_ms.unwrap_or(nt) } else { nt };
        let (rx, ry) = cursor_at(judge_ms);
        let (cx, cy) = (rx.clamp(-bound, bound), ry.clamp(-bound, bound));
        let inside = (cx - wx).abs() <= half && (cy - wy).abs() <= half;
        println!(
            "note {:4} t={:7.0} cell=({},{}) world=({:+.2},{:+.2}) | {} judge@{:7.0} err={:+5.0}ms | raw=({:+.3},{:+.3}) clamped=({:+.3},{:+.3}) inside={} dx={:+.3} dy={:+.3}",
            r.note_index, nt, n.x, n.y, wx, wy,
            if r.hit { "HIT " } else { "MISS" },
            judge_ms,
            judge_ms - nt,
            rx, ry, cx, cy, inside,
            (cx - wx).abs() - half,
            (cy - wy).abs() - half,
        );
    }
}
