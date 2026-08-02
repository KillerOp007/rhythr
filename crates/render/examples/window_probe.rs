//! Checks a candidate hit window against a replay: every recorded hit flag
//! must find a note (no orphans) and the derived totals must equal the header.
//! The game misses a note once `ms > note_t + hit_window`, with
//! `hit_window = hitwindow_ms * speed` (speed_hitwindow, on by default) and a
//! further ×0.8 under hardrock — so the window is a function of the replay,
//! not a constant. Usage: window_probe <replay.rhr> <map>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let replay = match rhythia_formats::rhr::Replay::from_path(&a[0]) {
        Ok(r) => r,
        Err(e) => {
            println!("PARSE-FAIL {e}");
            return;
        }
    };
    let map = match rhythia_formats::map::Map::from_path(&a[1]) {
        Ok(m) => m,
        Err(e) => {
            println!("MAP-FAIL {e}");
            return;
        }
    };
    let (map, _mods) = rhythia_render::mods::map_for_replay(&map, &replay);
    let hardrock = replay.mods.contains("hardrock");
    let speed = f64::from(replay.speed);
    let predicted = 55.0 * speed * if hardrock { 0.8 } else { 1.0 };
    let header_hits = replay.frames.iter().filter(|f| f.hit).count() as u32;

    let probe = |w: f64| -> (u32, u32, f64) {
        let out = rhythia_sim::hitreg::match_hits(&map.notes, &replay.frames, w);
        let derived = out.derived_hits();
        let worst = out
            .results
            .iter()
            .filter(|r| r.hit)
            .filter_map(|r| r.hit_ms.map(|hm| hm - map.notes[r.note_index].time_ms as f64))
            .fold(f64::MIN, f64::max);
        (derived, out.orphan_flags, worst)
    };

    let (d55, o55, w55) = probe(55.0);
    let (dp, op, wp) = probe(predicted);
    let (d80, o80, w80) = probe(80.0);

    let verdict = if op == 0 && dp == header_hits {
        "PRED-CLEAN"
    } else {
        "PRED-DIRTY"
    };
    println!(
        "{verdict} speed={speed:.2} hr={hardrock} pred={predicted:.1} header={header_hits} \
         | w55: hits={d55} orph={o55} worst={w55:.1} \
         | wpred: hits={dp} orph={op} worst={wp:.1} \
         | w80: hits={d80} orph={o80} worst={w80:.1}"
    );
}
