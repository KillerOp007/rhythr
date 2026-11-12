//! Speed-mod replays exist in the wild with TWO time bases: most store
//! their frame times in SONG time (a 1.45x run spans the full song
//! length; the video pipeline compresses by the speed), but some store
//! them in WALL-CLOCK time (already "shortened"). Feeding the latter
//! through the song-time pipeline double-applies the speed and the video
//! comes out ~2x fast; treating the former as wall-clock loses the speed
//! entirely. The base is detected per replay instead of assumed: only in
//! the correct base do the recorded hit flags line up with the map's note
//! times (measured: 471/471 derived hits in the right base versus 263
//! coincidental ones in the wrong base on a dense map).

use rhythia_formats::map::Map;
use rhythia_formats::rhr::{Origin, Replay};

use crate::hitreg::{hit_window_ms, match_hits};

/// The factor frame times must be multiplied by to become song time:
/// 1.0 when they already are (or the replay has no speed mod), or the
/// replay's speed when they are wall-clock.
pub fn time_scale(map: &Map, replay: &Replay) -> f64 {
    // A converted legacy replay had its song time rebuilt from the game's
    // own clock signals; there is nothing to detect.
    if matches!(replay.origin, Origin::Sspre { .. }) {
        return 1.0;
    }
    let speed = replay.speed as f64;
    if (speed - 1.0).abs() < 0.005 || replay.frames.is_empty() || map.notes.is_empty() {
        return 1.0;
    }
    // Both candidates are compared in song time, so both use the run's own
    // window.
    let window = hit_window_ms(replay);
    let as_is = match_hits(&map.notes, &replay.frames, window).derived_hits();
    let mut scaled = replay.clone();
    for f in &mut scaled.frames {
        f.ms *= speed;
    }
    let rescaled = match_hits(&map.notes, &scaled.frames, window).derived_hits();
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

/// Lead before the next note a skip lands on, at 1.00x: the game sets the
/// song clock to `next note - 1000 - 1000 * speed` (`NoteManager.gd`).
const SKIP_LEAD_MS: f64 = 1000.0;

/// True while a converted legacy replay has a skip no map has sized yet
/// (see [`Origin::Sspre`]); its later frame times are provisional until
/// [`normalize`] has run.
pub fn has_pending_skips(replay: &Replay) -> bool {
    matches!(&replay.origin, Origin::Sspre { skips } if skips.iter().any(|s| s.jump_ms == 0.0))
}

/// Sizes the skips of a converted legacy replay against this map: each
/// one shifts every later frame forward so that the skip moment lands the
/// game's fixed lead before the next note. Jumps sized against an earlier
/// map are taken back first, so pairing the replay with the wrong map and
/// then the right one ends in the right place. Returns true when a frame
/// moved.
fn resolve_skips(replay: &mut Replay, map: &Map) -> bool {
    let Origin::Sspre { skips } = &mut replay.origin else {
        return false;
    };
    let lead = SKIP_LEAD_MS + SKIP_LEAD_MS * f64::from(replay.speed);
    let before: Vec<f64> = skips.iter().map(|s| s.jump_ms).collect();

    // Where a skip sits among the frames right now: its own clock time
    // plus every jump before it.
    let position = |skips: &[rhythia_formats::rhr::Skip], i: usize| {
        skips[i].at_ms + skips[..i].iter().map(|s| s.jump_ms).sum::<f64>()
    };
    for i in (0..skips.len()).rev() {
        let (at, jump) = (position(skips, i), skips[i].jump_ms);
        if jump != 0.0 {
            for f in replay.frames.iter_mut().filter(|f| f.ms > at) {
                f.ms -= jump;
            }
            skips[i].jump_ms = 0.0;
        }
    }
    for i in 0..skips.len() {
        let at = position(skips, i);
        let next_note = map
            .notes
            .iter()
            .map(|n| n.time_ms as f64)
            .find(|&t| t >= at);
        // The game offers a skip only with the next note seconds ahead, so
        // a jump that would not go forward is not something it recorded.
        let Some(jump) = next_note.map(|t| t - lead - at).filter(|&j| j > 0.0) else {
            continue;
        };
        for f in replay.frames.iter_mut().filter(|f| f.ms > at) {
            f.ms += jump;
        }
        skips[i].jump_ms = jump;
    }
    skips.iter().map(|s| s.jump_ms).ne(before)
}

/// Brings a replay's frame times into song time in place: sizes the skips
/// of a converted legacy replay, and rescales a wall-clock `.rhr` (frames,
/// fail time, start offset). Idempotent: once in song time this is a
/// no-op. Returns true when frame times changed.
pub fn normalize(replay: &mut Replay, map: &Map) -> bool {
    if resolve_skips(replay, map) {
        return true;
    }
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
    use rhythia_formats::rhr::{Frame, Origin, Skip};

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
            pitch_lock: false,
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
            origin: Origin::Rhr,
        }
    }

    fn converted(speed: f32, frame_times: &[f64], skips_at: &[f64]) -> Replay {
        let mut r = replay_with(speed, frame_times);
        r.origin = Origin::Sspre {
            skips: skips_at
                .iter()
                .map(|&at_ms| Skip {
                    at_ms,
                    jump_ms: 0.0,
                })
                .collect(),
        };
        r
    }

    fn jumps(r: &Replay) -> Vec<f64> {
        match &r.origin {
            Origin::Sspre { skips } => skips.iter().map(|s| s.jump_ms).collect(),
            Origin::Rhr => Vec::new(),
        }
    }

    fn times(r: &Replay) -> Vec<f64> {
        r.frames.iter().map(|f| f.ms).collect()
    }

    #[test]
    fn a_pending_skip_lands_a_fixed_lead_before_the_next_note() {
        // The game jumps to `next note - 1000 - 1000 * speed`.
        let map = map_with(&[1000, 60000]);
        let mut r = converted(1.0, &[1500.0, 2000.0, 2500.0, 3000.0], &[2000.0]);
        assert!(normalize(&mut r, &map));
        assert_eq!(times(&r), vec![1500.0, 2000.0, 58500.0, 59000.0]);
        assert_eq!(jumps(&r), vec![56000.0]);
        assert!(!has_pending_skips(&r));
        assert!(!normalize(&mut r, &map), "a second pass changes nothing");
        assert_eq!(times(&r), vec![1500.0, 2000.0, 58500.0, 59000.0]);
    }

    #[test]
    fn pairing_with_another_map_sizes_the_skips_again() {
        // The wrong map first, then the right one: the result must be what
        // the right map alone would have given.
        let wrong = map_with(&[1000, 30000]);
        let right = map_with(&[1000, 60000, 61000, 200000]);
        let fresh = || converted(1.0, &[2000.0, 3000.0, 6000.0, 7000.0], &[2000.0, 6000.0]);
        let mut direct = fresh();
        normalize(&mut direct, &right);
        let mut detour = fresh();
        assert!(has_pending_skips(&detour));
        normalize(&mut detour, &wrong);
        assert_ne!(times(&detour), times(&direct));
        assert!(normalize(&mut detour, &right));
        assert_eq!(times(&detour), times(&direct));
        assert_eq!(jumps(&detour), jumps(&direct));
    }

    #[test]
    fn later_skips_build_on_earlier_ones_and_the_lead_scales_with_speed() {
        let map = map_with(&[1000, 60000, 61000, 200000]);
        // At 1.25x the lead is 1000 + 1250. The first skip moves everything
        // after 2000 by 60000 - 2250 - 2000 = 55750, the second skip
        // included: it now sits at 61750 and jumps on to 200000 - 2250.
        let mut r = converted(1.25, &[2000.0, 3000.0, 6000.0, 7000.0], &[2000.0, 6000.0]);
        assert!(normalize(&mut r, &map));
        assert_eq!(times(&r), vec![2000.0, 58750.0, 61750.0, 198750.0]);

        // ...unless the jump would go backwards: the game only offers a
        // skip with the next note far ahead, so such a file is not from
        // the game and its frames stay where they are.
        let mut odd = converted(1.0, &[2000.0, 3000.0], &[2000.0]);
        assert!(!normalize(&mut odd, &map_with(&[1000, 2500])));
        assert_eq!(times(&odd), vec![2000.0, 3000.0]);
        assert_eq!(jumps(&odd), vec![0.0]);
    }

    #[test]
    fn a_converted_replay_is_already_song_time_and_is_never_rescaled() {
        // Hits line up only when multiplied by the speed: a .rhr like this
        // is wall-clock and gets rescaled, a converted replay must not be.
        let map = map_with(&[1450, 2900, 4350, 5800]);
        let wall = [1000.0, 2000.0, 3000.0, 4000.0];
        assert_eq!(time_scale(&map, &replay_with(1.45, &wall)), 1.45f32 as f64);
        let mut r = converted(1.45, &wall, &[]);
        assert_eq!(time_scale(&map, &r), 1.0);
        assert!(!normalize(&mut r, &map));
        assert_eq!(times(&r), wall.to_vec());
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
