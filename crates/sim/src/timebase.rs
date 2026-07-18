//! Speed-mod replays exist in the wild with TWO time bases: most store
//! their frame times in SONG time (a 1.45x run spans the full song
//! length; the video pipeline compresses by the speed), but some store
//! them in WALL-CLOCK time — already "shortened". Feeding the latter
//! through the song-time pipeline double-applies the speed and the video
//! comes out ~2x fast; treating the former as wall-clock loses the speed
//! entirely. The base is detected per replay instead of assumed: only in
//! the correct base do the recorded hit flags line up with the map's note
//! times (measured: 471/471 derived hits in the right base versus 263
//! coincidental ones in the wrong base on a dense map).

use rhythia_formats::map::Map;
use rhythia_formats::rhr::Replay;

use crate::hitreg::{match_hits, DEFAULT_WINDOW_MS};

/// The factor frame times must be multiplied by to become song time:
/// 1.0 when they already are (or the replay has no speed mod), or the
/// replay's speed when they are wall-clock.
pub fn time_scale(map: &Map, replay: &Replay) -> f64 {
    let speed = replay.speed as f64;
    if (speed - 1.0).abs() < 0.005 || replay.frames.is_empty() || map.notes.is_empty() {
        return 1.0;
    }
    let as_is = match_hits(&map.notes, &replay.frames, DEFAULT_WINDOW_MS).derived_hits();
    let mut scaled = replay.clone();
    for f in &mut scaled.frames {
        f.ms *= speed;
    }
    let rescaled = match_hits(&map.notes, &scaled.frames, DEFAULT_WINDOW_MS).derived_hits();
    // Only switch on a clear win backed by real evidence. The ratio guards
    // against coincidental matches on a dense map; the absolute floor
    // guards against a WRONG map (hash-mismatch download, user-picked
    // file), where both counts are pure noise and e.g. 1-vs-0 must not
    // rescale the replay. A genuine wall-clock replay recovers essentially
    // every recorded hit in the right base, so demand at least half.
    let floor = (replay.hits.max(0) as u32 / 2).max(4);
    if rescaled >= floor && rescaled as f64 > as_is as f64 * 1.05 {
        speed
    } else {
        1.0
    }
}

/// Rescales a wall-clock replay into song time in place (frames, fail
/// time, start offset). Idempotent: once in song time, [`time_scale`]
/// returns 1.0 and this is a no-op. Returns true when a rescale happened.
pub fn normalize(replay: &mut Replay, map: &Map) -> bool {
    let scale = time_scale(map, replay);
    if scale == 1.0 {
        return false;
    }
    for f in &mut replay.frames {
        f.ms *= scale;
    }
    if replay.fail_time_ms > 0 {
        replay.fail_time_ms = (replay.fail_time_ms as f64 * scale).round() as i32;
    }
    if replay.start_from_ms > 0 {
        replay.start_from_ms = (replay.start_from_ms as f64 * scale).round() as i32;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhythia_formats::map::Note;
    use rhythia_formats::rhr::Frame;

    fn map_with(times: &[i64]) -> Map {
        Map {
            notes: times
                .iter()
                .map(|&t| Note {
                    time_ms: t,
                    x: 1.0,
                    y: 1.0,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn replay_with(speed: f32, hit_times: &[f64]) -> Replay {
        Replay {
            version: 5,
            timestamp_ticks: 0,
            player_name: "t".into(),
            legacy_map_id: String::new(),
            map_id: 0,
            start_from_ms: 0,
            mode: String::new(),
            passed: true,
            mods: "[]".into(),
            spin: false,
            speed,
            total_score: 0,
            accuracy_pct: 100.0,
            hits: hit_times.len() as i32,
            misses: 0,
            points: 0.0,
            fail_time_ms: -1,
            beatmap_hash: String::new(),
            frames: hit_times
                .iter()
                .map(|&t| Frame {
                    ms: t,
                    x: 0.0,
                    y: 0.0,
                    health: 1.0,
                    hit: true,
                })
                .collect(),
            trailing_bytes: 0,
        }
    }

    #[test]
    fn song_time_replays_stay_untouched() {
        let map = map_with(&[1000, 2000, 3000]);
        let mut r = replay_with(1.45, &[1005.0, 2010.0, 2995.0]);
        assert_eq!(time_scale(&map, &r), 1.0);
        assert!(!normalize(&mut r, &map));
        assert_eq!(r.frames[0].ms, 1005.0);
    }

    #[test]
    fn wall_clock_replays_rescale_once() {
        let map = map_with(&[1450, 2900, 4350, 5800]);
        // Wall-clock: hits at note_time / 1.45.
        let mut r = replay_with(1.45, &[1000.0, 2000.0, 3000.0, 4000.0]);
        r.fail_time_ms = 4100;
        assert!((time_scale(&map, &r) - 1.45).abs() < 1e-4);
        assert!(normalize(&mut r, &map));
        assert!((r.frames[0].ms - 1450.0).abs() < 0.01);
        assert!((r.fail_time_ms - 5945).abs() <= 1);
        // Idempotent: a second pass changes nothing.
        assert!(!normalize(&mut r, &map));
        assert!((r.frames[3].ms - 5800.0).abs() < 0.01);
    }

    #[test]
    fn coincidental_matches_on_a_wrong_map_do_not_rescale() {
        // A non-matching map (wrong version / user-picked file): neither
        // base really lines up, but one rescaled hit lands inside the
        // window by luck. 1-vs-0 must not mutate the replay.
        let map = map_with(&[1450, 8000, 16000]);
        let mut r = replay_with(1.45, &[1010.0, 2000.0, 3000.0, 4000.0]);
        assert_eq!(time_scale(&map, &r), 1.0);
        assert!(!normalize(&mut r, &map));
        assert_eq!(r.frames[0].ms, 1010.0);
    }
}
