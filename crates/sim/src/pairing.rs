//! What a converted replay can only settle once its map is at hand.
//!
//! A `.rhr` flags the frame on which a note was hit. A `.phxr` stores a
//! result per note instead (missed, or hit this late), which says nothing
//! without the note times. [`place_judgments`] turns those results into
//! the flagged frames the rest of the pipeline reads, so hit matching,
//! the HUD and the analyzer need no second code path.
//!
//! A legacy `.sspre` names the note of every hit but not its moment, only
//! the cursor samples around it. [`place_legacy_hits`] settles the moment
//! against the note's own time.
//!
//! Both converted formats also end a failed or abandoned run by simply
//! stopping, with no fail time in the file. Where that was, in terms of
//! notes, follows from how far the judged notes reach into the map
//! ([`early_end_ms`]).
//!
//! Everything here is derived, never stored: pairing the same replay with
//! another map takes the earlier result back first.

use rhythia_formats::map::{Map, Note};
use rhythia_formats::rhr::{Frame, Judgments, Origin, Replay};

use crate::hitreg::{hit_window_ms, BASE_HIT_WINDOW_MS, HITBOX_HALF};

/// How far outside a note's hit area the recorded cursor path may pass at
/// the moment of a hit before the hit counts as off target, in cells.
///
/// The cursor is sampled 60 times a second and the hit happened on some
/// game frame in between, so all that is known is the straight line
/// between the two samples around it. The real path can bow away from
/// that line in a sharp turn. This margin is a first guess for that bow
/// and has not been calibrated against real recordings.
const OFF_TARGET_SLACK: f32 = 0.25;

/// How much later than its computed time a hit may really have happened,
/// in song-time ms at 1.00x: the client cuts the song clock to whole
/// milliseconds before it takes the lateness, and stores the lateness in
/// quarter milliseconds, both rounding down.
const HIT_TIME_SLACK_MS: f64 = 2.0;

/// How far a legacy time stamp can be off, in real-time ms: the game cuts
/// its clock to whole milliseconds, and the conversion reads every stamp
/// one millisecond late.
const LEGACY_STAMP_SLACK_MS: f64 = 1.0;

/// The free client's health rule. Health runs from 0 to 100. A miss costs
/// the current step and makes the next one a fifth more expensive; a hit
/// eases the step again and gives part of it back. The step never drops
/// below its starting value.
struct Health {
    value: f64,
    step: f64,
}

impl Health {
    const FULL: f64 = 100.0;
    const BASE_STEP: f64 = 15.0;

    fn new() -> Health {
        Health {
            value: Self::FULL,
            step: Self::BASE_STEP,
        }
    }

    fn judge(&mut self, hit: bool) {
        if hit {
            self.step = (self.step / 1.45).max(Self::BASE_STEP);
            self.value = (self.value + self.step / 1.75).min(Self::FULL);
        } else {
            self.value = (self.value - self.step).max(0.0);
            self.step = (self.step * 1.2).min(Self::FULL);
        }
    }

    /// As the 0..1 a frame carries.
    fn fraction(&self) -> f32 {
        (self.value / Self::FULL) as f32
    }
}

/// The cursor position at a song time, between the two samples around it.
/// The client's own playback draws the cursor the same way.
fn cursor_at(samples: &[Frame], ms: f64) -> (f32, f32) {
    let after = samples.partition_point(|f| f.ms <= ms);
    let before = after.checked_sub(1).map(|i| samples[i]);
    match (before, samples.get(after)) {
        (Some(a), Some(b)) if b.ms > a.ms => {
            let t = ((ms - a.ms) / (b.ms - a.ms)) as f32;
            (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
        }
        (Some(a), _) => (a.x, a.y),
        (None, Some(b)) => (b.x, b.y),
        (None, None) => (0.0, 0.0),
    }
}

/// A note's centre in the cursor's space, mirrored the way the run was
/// played.
fn note_centre(note: &Note, (flip_x, flip_y): (bool, bool)) -> (f32, f32) {
    let (x, y) = (note.x - 1.0, 1.0 - note.y);
    (if flip_x { -x } else { x }, if flip_y { -y } else { y })
}

/// True when the straight path from `a` to `b` touches the square of
/// half-width `reach` around `centre`.
fn path_touches(a: (f32, f32), b: (f32, f32), centre: (f32, f32), reach: f32) -> bool {
    // Clip the path against the square, one axis at a time; what is left
    // of the parameter range is the part inside.
    let (mut enter, mut leave) = (0.0f32, 1.0f32);
    for (from, to, mid) in [(a.0, b.0, centre.0), (a.1, b.1, centre.1)] {
        let (lo, hi) = (mid - reach, mid + reach);
        let run = to - from;
        if run == 0.0 {
            if from < lo || from > hi {
                return false;
            }
            continue;
        }
        let (t1, t2) = ((lo - from) / run, (hi - from) / run);
        enter = enter.max(t1.min(t2));
        leave = leave.min(t1.max(t2));
        if enter > leave {
            return false;
        }
    }
    true
}

/// True when the recorded cursor was on the note around `ms`: the path
/// through the samples that bracket the moment of the hit touches the
/// note's hit area, widened by [`OFF_TARGET_SLACK`].
fn cursor_was_on(samples: &[Frame], ms: f64, slack_ms: f64, centre: (f32, f32)) -> bool {
    let reach = HITBOX_HALF + OFF_TARGET_SLACK;
    let Some(last) = samples.len().checked_sub(1) else {
        return false;
    };
    let from = samples.partition_point(|f| f.ms <= ms).saturating_sub(1);
    let to = samples.partition_point(|f| f.ms < ms + slack_ms).min(last);
    let at = |i: usize| (samples[i].x, samples[i].y);
    if from >= to {
        return path_touches(at(from), at(from), centre, reach);
    }
    (from..to).any(|i| path_touches(at(i), at(i + 1), centre, reach))
}

/// Where a run that stopped short of the map's last note ended, as the
/// fail time the pipeline works with: every note up to and including that
/// time was judged, none after it. `judged_end` is one past the last
/// judged note. Returns -1 when the run judged the map to its end.
pub(crate) fn early_end_ms(map: &Map, judged_end: usize, last_frame_ms: f64) -> i32 {
    let Some(unjudged) = map.notes.get(judged_end) else {
        return -1;
    };
    // The recording's own end, held between the last judged note and the
    // first one the run never got to.
    let mut end = (last_frame_ms.ceil() as i64).min(unjudged.time_ms - 1);
    if let Some(judged) = judged_end.checked_sub(1).and_then(|i| map.notes.get(i)) {
        end = end.max(judged.time_ms);
    }
    end.clamp(0, i64::from(i32::MAX)) as i32
}

/// Marks where an early-ended legacy run stopped. Returns true when the
/// fail time changed.
pub(crate) fn mark_early_end(replay: &mut Replay, map: &Map) -> bool {
    let Origin::Sspre {
        judged_end: Some(judged_end),
        ..
    } = replay.origin
    else {
        return false;
    };
    let last_frame_ms = replay.frames.last().map_or(f64::NEG_INFINITY, |f| f.ms);
    let end = early_end_ms(map, judged_end as usize, last_frame_ms);
    let changed = end != replay.fail_time_ms;
    replay.fail_time_ms = end;
    changed
}

/// The recorded samples in their own order with the hits among them: each
/// hit goes in right after the last sample that is not later than it.
/// `hits` must be sorted by time.
fn weave(samples: Vec<Frame>, hits: Vec<Frame>) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(samples.len() + hits.len());
    let mut pending = hits.into_iter().peekable();
    for sample in samples {
        while let Some(hit) = pending.next_if(|h| h.ms < sample.ms) {
            frames.push(hit);
        }
        frames.push(sample);
    }
    frames.extend(pending);
    frames
}

/// Where a hit of a converted legacy replay belongs once the notes are
/// known.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LegacyPlacement {
    /// The note the file names for the hit.
    pub note: usize,
    /// Song time of the hit among the frames as they are now.
    pub ms: f64,
    /// True when the note exists and the samples around the hit leave
    /// room for it inside the note's window.
    pub fits: bool,
}

/// Settles when each hit of a converted legacy replay happened, in file
/// order. None for any other replay.
///
/// The file names each hit's note but not its moment: only that it came
/// after one cursor sample and no later than the next. The game's own rule
/// says more: a note can be hit from its time on, and it is missed in the
/// first game frame after its window closed in which the cursor is not on
/// it. The hit test comes first (`NoteManager.gd`, `reposition_notes`), so
/// a hit can land one game frame behind the window, and the sample behind
/// it later still. Real recordings have a few such hits per run, up to
/// 11 ms behind a 55 ms window.
///
/// So a hit whose samples leave room for it is put inside its note's
/// window: at the window's end when its sample lies behind that, at the
/// note's time when the cut-off stamp lies before it, on its sample
/// otherwise. A hit whose samples leave no room (the note had not come yet
/// at the sample behind the hit, or its window was shut already at the
/// sample before) stays on its sample and does not fit: a hit these notes
/// have no place for.
pub(crate) fn legacy_placements(replay: &Replay, notes: &[Note]) -> Option<Vec<LegacyPlacement>> {
    let Origin::Sspre { skips, hits, .. } = &replay.origin else {
        return None;
    };
    let window = hit_window_ms(replay);
    let slack = LEGACY_STAMP_SLACK_MS * f64::from(replay.speed).max(1.0);
    // From the run's own clock to where the frames are now: every skip
    // before a moment has moved it forward by its jump.
    let now = |own_ms: f64| {
        own_ms
            + skips
                .iter()
                .filter(|s| s.at_ms < own_ms)
                .map(|s| s.jump_ms)
                .sum::<f64>()
    };
    Some(
        hits.iter()
            .map(|hit| {
                let (after, by) = (now(hit.after_ms), now(hit.by_ms));
                let note = hit.note as usize;
                // An index past the last note belongs to another map.
                let room = notes.get(note).and_then(|n| {
                    let open = n.time_ms as f64;
                    let close = open + window;
                    (by + slack >= open && after <= close + slack).then_some((open, close))
                });
                LegacyPlacement {
                    note,
                    ms: room.map_or(by, |(open, close)| by.clamp(open, close)),
                    fits: room.is_some(),
                }
            })
            .collect(),
    )
}

/// Puts the flagged frames of a converted legacy replay where
/// [`legacy_placements`] settles its hits against this map, with the
/// cursor taken from the recorded path at that moment.
///
/// Derived from the hits kept in the replay's origin, so pairing with
/// another map starts over. Returns true when a frame changed.
pub(crate) fn place_legacy_hits(replay: &mut Replay, map: &Map) -> bool {
    let Some(placements) = legacy_placements(replay, &map.notes) else {
        return false;
    };
    if placements.is_empty() {
        return false;
    }
    // Only the recorded samples; every flagged frame is one of ours.
    let samples: Vec<Frame> = replay.frames.iter().filter(|f| !f.hit).copied().collect();
    let mut placed: Vec<Frame> = placements
        .iter()
        .map(|p| {
            let (x, y) = cursor_at(&samples, p.ms);
            Frame {
                ms: p.ms,
                x,
                y,
                health: 1.0,
                hit: true,
            }
        })
        .collect();
    // Notes are hit in map order almost always, but a late hit on one note
    // can come after an early hit on the next.
    placed.sort_by(|a, b| a.ms.total_cmp(&b.ms));

    let frames = weave(samples, placed);
    let changed = frames != replay.frames;
    replay.frames = frames;
    changed
}

/// Turns a `.phxr` replay's note results into flagged frames against this
/// map: one frame per hit, at the note's time plus the recorded lateness,
/// with the cursor where the samples around it put it. Also fills in what
/// follows from the results: health on every frame, where an early-ended
/// run stopped, and how many hits the cursor was not on the note for.
///
/// Frames placed against an earlier map are taken out first, so pairing
/// with the wrong map and then the right one ends in the right place.
/// Returns true when the replay changed.
pub(crate) fn place_judgments(replay: &mut Replay, map: &Map) -> bool {
    let Origin::Phxr(judgments) = &replay.origin else {
        return false;
    };
    let (first_note, flip) = (judgments.first_note, judgments.flip);
    let results = judgments.results.clone();
    let was_off_target = judgments.off_target_hits;
    let speed = f64::from(replay.speed);

    // Only the recorded samples; every flagged frame is one of ours.
    let samples: Vec<Frame> = replay.frames.iter().filter(|f| !f.hit).copied().collect();

    let mut hits: Vec<Frame> = Vec::new();
    // (song time, hit) of every judged note, for the health walk.
    let mut judged: Vec<(f64, bool)> = Vec::with_capacity(results.len());
    let mut off_target = 0u32;
    for (i, &result) in results.iter().enumerate() {
        // Results beyond the map's last note belong to another map.
        let Some(note) = first_note.checked_add(i).and_then(|n| map.notes.get(n)) else {
            break;
        };
        let note_ms = note.time_ms as f64;
        let Some(late) = Judgments::lateness_ms(result) else {
            judged.push((note_ms + BASE_HIT_WINDOW_MS * speed, false));
            continue;
        };
        // Lateness is real time; the song ran `speed` times as fast.
        let ms = note_ms + late * speed;
        let (x, y) = cursor_at(&samples, ms);
        // Notes that share a timestamp may be listed the other way round
        // in the file this map was loaded from (see `phxm`), so a hit on
        // any of them is a hit on target.
        let slack_ms = HIT_TIME_SLACK_MS * speed.max(1.0);
        let on_target = map
            .notes
            .iter()
            .filter(|n| n.time_ms == note.time_ms)
            .any(|n| cursor_was_on(&samples, ms, slack_ms, note_centre(n, flip)));
        if !on_target {
            off_target += 1;
        }
        hits.push(Frame {
            ms,
            x,
            y,
            health: 1.0,
            hit: true,
        });
        judged.push((ms, true));
    }
    // Notes are judged in map order almost always, but a late hit on one
    // note can come after an early hit on the next.
    hits.sort_by(|a, b| a.ms.total_cmp(&b.ms));
    judged.sort_by(|a, b| a.0.total_cmp(&b.0));

    let last_sample_ms = samples
        .iter()
        .map(|f| f.ms)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut frames = weave(samples, hits);

    let mut health = Health::new();
    let mut next = 0;
    for frame in &mut frames {
        while next < judged.len() && judged[next].0 <= frame.ms {
            health.judge(judged[next].1);
            next += 1;
        }
        frame.health = health.fraction();
    }

    let judged_end = first_note.saturating_add(results.len());
    let fail_time_ms = early_end_ms(map, judged_end, last_sample_ms);

    let changed = frames != replay.frames
        || fail_time_ms != replay.fail_time_ms
        || was_off_target != Some(off_target);
    replay.frames = frames;
    replay.fail_time_ms = fail_time_ms;
    if let Origin::Phxr(judgments) = &mut replay.origin {
        judgments.off_target_hits = Some(off_target);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hitreg::{hit_window_ms, judge, match_hits};
    use crate::timebase::normalize;
    use rhythia_formats::rhr::{LegacyHit, Skip};

    /// Notes as (time, column, row) on the 3x3 grid, row 0 on top.
    fn map_with(notes: &[(i64, f32, f32)]) -> Map {
        Map {
            notes: notes
                .iter()
                .map(|&(time_ms, x, y)| Note { time_ms, x, y })
                .collect(),
            ..Default::default()
        }
    }

    fn replay_of(origin: Origin, samples: &[(f64, f32, f32)], hits: i32, misses: i32) -> Replay {
        Replay {
            version: 5,
            timestamp_ticks: 0,
            player_name: String::new(),
            legacy_map_id: String::new(),
            map_id: 0,
            start_from_ms: 0,
            mode: String::new(),
            passed: true,
            mods: "[]".into(),
            spin: false,
            speed: 1.0,
            total_score: 0,
            accuracy_pct: 100.0,
            hits,
            misses,
            points: 0.0,
            fail_time_ms: -1,
            beatmap_hash: String::new(),
            pitch_lock: false,
            frames: samples
                .iter()
                .map(|&(ms, x, y)| Frame {
                    ms,
                    x,
                    y,
                    health: 1.0,
                    hit: false,
                })
                .collect(),
            trailing_bytes: 0,
            origin,
        }
    }

    /// A `.phxr` replay as the parser hands it over: samples only, the
    /// results waiting in the origin.
    fn phxr(samples: &[(f64, f32, f32)], first_note: usize, results: &[u8]) -> Replay {
        let hits = results.iter().filter(|&&r| r != Judgments::MISS).count() as i32;
        replay_of(
            Origin::Phxr(Judgments {
                first_note,
                results: results.to_vec(),
                map_notes: 0,
                flip: (false, false),
                checksum_ok: true,
                off_target_hits: None,
            }),
            samples,
            hits,
            results.len() as i32 - hits,
        )
    }

    fn flagged(r: &Replay) -> Vec<(f64, f32, f32)> {
        r.frames
            .iter()
            .filter(|f| f.hit)
            .map(|f| (f.ms, f.x, f.y))
            .collect()
    }

    fn off_target(r: &Replay) -> Option<u32> {
        match &r.origin {
            Origin::Phxr(j) => j.off_target_hits,
            _ => None,
        }
    }

    const MISS: u8 = Judgments::MISS;

    #[test]
    fn a_hit_becomes_a_flagged_frame_at_the_note_time_plus_its_lateness() {
        // Centre note at 1000, hit 10 ms late (40 quarter ms); top right
        // note at 2000 missed; centre note at 3000 hit on time.
        let map = map_with(&[(1000, 1.0, 1.0), (2000, 2.0, 0.0), (3000, 1.0, 1.0)]);
        let mut r = phxr(
            &[
                (0.0, 0.0, 0.0),
                (1000.0, 0.2, 0.0),
                (1020.0, 0.4, 0.2),
                (4000.0, 0.0, 0.0),
            ],
            0,
            &[40, MISS, 0],
        );
        assert!(normalize(&mut r, &map));
        // 1010 lies halfway between the samples at 1000 and 1020.
        let got = flagged(&r);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, 1010.0);
        assert!((got[0].1 - 0.3).abs() < 1e-6 && (got[0].2 - 0.1).abs() < 1e-6);
        assert_eq!(got[1].0, 3000.0);
        assert_eq!(r.flagged_frames() as i32, r.hits);
        // The frames stay in time order with the samples untouched.
        let times: Vec<f64> = r.frames.iter().map(|f| f.ms).collect();
        assert_eq!(times, [0.0, 1000.0, 1010.0, 1020.0, 3000.0, 4000.0]);

        // The ordinary hit matching now reads the run like any other.
        let outcome = match_hits(&map.notes, &r.frames, hit_window_ms(&r));
        let verdicts: Vec<bool> = outcome.results.iter().map(|n| n.hit).collect();
        assert_eq!(verdicts, [true, false, true]);
        assert_eq!(outcome.orphan_flags, 0);
        assert_eq!(off_target(&r), Some(0));
    }

    #[test]
    fn lateness_is_real_time_and_stretches_with_the_speed() {
        let map = map_with(&[(1000, 1.0, 1.0)]);
        let mut r = phxr(&[(0.0, 0.0, 0.0), (2000.0, 0.0, 0.0)], 0, &[40]);
        r.speed = 1.5;
        normalize(&mut r, &map);
        assert_eq!(flagged(&r)[0].0, 1015.0);
    }

    #[test]
    fn results_start_at_the_first_judged_note() {
        // A run started mid-map: the two results belong to notes 2 and 3.
        let map = map_with(&[
            (1000, 1.0, 1.0),
            (2000, 1.0, 1.0),
            (3000, 1.0, 1.0),
            (4000, 1.0, 1.0),
        ]);
        let mut r = phxr(&[(2500.0, 0.0, 0.0), (5000.0, 0.0, 0.0)], 2, &[0, 20]);
        normalize(&mut r, &map);
        let times: Vec<f64> = flagged(&r).iter().map(|f| f.0).collect();
        assert_eq!(times, [3000.0, 4005.0]);
        assert_eq!(r.fail_time_ms, -1, "the run reached the last note");
    }

    #[test]
    fn pairing_again_replaces_the_frames_instead_of_adding_to_them() {
        let right = map_with(&[(1000, 1.0, 1.0), (2000, 1.0, 1.0)]);
        let wrong = map_with(&[(700, 1.0, 1.0), (900, 1.0, 1.0), (5000, 1.0, 1.0)]);
        let fresh = || phxr(&[(0.0, 0.0, 0.0), (3000.0, 0.0, 0.0)], 0, &[0, 0]);

        let mut direct = fresh();
        assert!(normalize(&mut direct, &right));
        assert!(
            !normalize(&mut direct, &right),
            "a second pass changes nothing"
        );

        let mut detour = fresh();
        normalize(&mut detour, &wrong);
        assert_ne!(detour.frames, direct.frames);
        assert!(normalize(&mut detour, &right));
        assert_eq!(detour.frames, direct.frames);
        assert_eq!(detour.fail_time_ms, direct.fail_time_ms);
    }

    #[test]
    fn health_follows_the_results_frame_by_frame() {
        let map = map_with(&[(1000, 1.0, 1.0), (2000, 1.0, 1.0), (3000, 1.0, 1.0)]);
        // Miss, miss, hit; a sample between each pair of verdicts.
        let mut r = phxr(
            &[
                (0.0, 0.0, 0.0),
                (1500.0, 0.0, 0.0),
                (2500.0, 0.0, 0.0),
                (3500.0, 0.0, 0.0),
            ],
            0,
            &[MISS, MISS, 0],
        );
        normalize(&mut r, &map);
        let at = |ms: f64| r.frames.iter().find(|f| f.ms == ms).unwrap().health;
        assert_eq!(at(0.0), 1.0);
        // First miss costs 15, the second 15 * 1.2 = 18.
        assert!((at(1500.0) - 0.85).abs() < 1e-6);
        assert!((at(2500.0) - 0.67).abs() < 1e-6);
        // The hit eases the step from 21.6 to 15 (its floor) and gives
        // back 15 / 1.75.
        let after_hit = 0.67 + 15.0 / 1.75 / 100.0;
        assert!((at(3500.0) - after_hit as f32).abs() < 1e-6);
        // A miss registers when its window closes, not at the note.
        let mut early = phxr(
            &[(0.0, 0.0, 0.0), (1040.0, 0.0, 0.0), (1060.0, 0.0, 0.0)],
            0,
            &[MISS],
        );
        normalize(&mut early, &map_with(&[(1000, 1.0, 1.0)]));
        assert_eq!(early.frames[1].health, 1.0);
        assert!(early.frames[2].health < 1.0);
    }

    #[test]
    fn health_never_leaves_its_range() {
        let mut h = Health::new();
        for _ in 0..40 {
            h.judge(false);
        }
        assert_eq!(h.fraction(), 0.0);
        for _ in 0..400 {
            h.judge(true);
        }
        assert_eq!(h.fraction(), 1.0);
    }

    #[test]
    fn a_run_that_stops_short_of_the_last_note_ends_there() {
        let map = map_with(&[
            (1000, 1.0, 1.0),
            (2000, 1.0, 1.0),
            (3000, 1.0, 1.0),
            (4000, 1.0, 1.0),
        ]);
        // Two notes judged, the recording stops at 2400.
        let mut r = phxr(&[(0.0, 0.0, 0.0), (2400.0, 0.0, 0.0)], 0, &[0, MISS]);
        assert!(normalize(&mut r, &map));
        assert_eq!(r.fail_time_ms, 2400);
        assert!(r.failed());

        // The end never reaches the first unjudged note, even when the
        // recording ran on a little, and never falls before the last
        // judged one when the last sample came early.
        let mut long = phxr(&[(0.0, 0.0, 0.0), (3020.0, 0.0, 0.0)], 0, &[0, MISS]);
        normalize(&mut long, &map);
        assert_eq!(long.fail_time_ms, 2999);
        let mut short = phxr(&[(0.0, 0.0, 0.0), (1990.0, 0.0, 0.0)], 0, &[0, 0]);
        normalize(&mut short, &map);
        assert_eq!(short.fail_time_ms, 2000);

        // Nothing judged at all: the run ended before the first note.
        let mut none = phxr(&[(-500.0, 0.0, 0.0), (300.0, 0.0, 0.0)], 0, &[]);
        normalize(&mut none, &map);
        assert_eq!(none.fail_time_ms, 300);
    }

    #[test]
    fn a_hit_with_the_cursor_elsewhere_is_counted_as_off_target() {
        // Top left note; the cursor sits bottom right the whole time.
        let map = map_with(&[(1000, 0.0, 0.0), (2000, 2.0, 2.0)]);
        let mut r = phxr(&[(0.0, 1.0, -1.0), (3000.0, 1.0, -1.0)], 0, &[0, 0]);
        normalize(&mut r, &map);
        assert_eq!(
            off_target(&r),
            Some(1),
            "only the second note is under the cursor"
        );

        // Just outside the hit area still counts as on target: the
        // position at a hit is an estimate.
        let mut near = phxr(&[(0.0, -1.7, 1.0), (3000.0, -1.7, 1.0)], 0, &[0]);
        normalize(&mut near, &map);
        assert_eq!(off_target(&near), Some(0));
    }

    #[test]
    fn a_cursor_that_leaves_right_after_the_hit_was_still_on_the_note() {
        // Centre note hit at 1012; the cursor sat on it at the sample
        // before (1000) and is two cells away at the next one (1016).
        // Halfway along that line it is off the note, the path is not.
        let map = map_with(&[(1000, 1.0, 1.0)]);
        let mut r = phxr(
            &[
                (0.0, 0.0, 0.0),
                (1000.0, 0.0, 0.0),
                (1016.0, 2.0, 0.0),
                (3000.0, 2.0, 0.0),
            ],
            0,
            &[48],
        );
        normalize(&mut r, &map);
        assert_eq!(flagged(&r)[0].0, 1012.0);
        assert_eq!(off_target(&r), Some(0));
    }

    #[test]
    fn a_path_touches_a_square_it_crosses_starts_in_or_grazes() {
        let centre = (0.0, 0.0);
        assert!(
            path_touches((-2.0, 0.0), (2.0, 0.0), centre, 0.5),
            "straight through"
        );
        assert!(
            path_touches((0.1, 0.1), (5.0, 5.0), centre, 0.5),
            "starts inside"
        );
        assert!(
            path_touches((-1.0, 0.0), (0.0, 1.0), centre, 0.5),
            "clips the corner"
        );
        assert!(
            !path_touches((-2.0, 1.0), (2.0, 1.0), centre, 0.5),
            "passes above"
        );
        assert!(
            !path_touches((-2.0, 0.0), (-1.0, 0.0), centre, 0.5),
            "stops short"
        );
        assert!(
            !path_touches((1.0, 1.0), (1.0, 1.0), centre, 0.5),
            "a point outside"
        );
        assert!(
            path_touches((0.2, -0.2), (0.2, -0.2), centre, 0.5),
            "a point inside"
        );
    }

    #[test]
    fn a_double_listed_the_other_way_round_is_not_off_target() {
        // Two notes at the same time, left and right. The replay's first
        // result is a hit, its second a miss; the cursor was on the right
        // one, which this copy of the map happens to list second.
        let map = map_with(&[(1000, 0.0, 1.0), (1000, 2.0, 1.0)]);
        let mut r = phxr(&[(0.0, 1.0, 0.0), (3000.0, 1.0, 0.0)], 0, &[0, MISS]);
        normalize(&mut r, &map);
        assert_eq!(off_target(&r), Some(0));
        // The ordinary matching hands the hit to the note under the cursor.
        let outcome = match_hits(&map.notes, &r.frames, hit_window_ms(&r));
        let verdicts: Vec<bool> = outcome.results.iter().map(|n| n.hit).collect();
        assert_eq!(verdicts, [false, true]);
    }

    #[test]
    fn a_mirrored_run_is_measured_against_the_mirrored_notes() {
        // Top left note, played flipped left to right: it showed top right.
        let map = map_with(&[(1000, 0.0, 0.0)]);
        let mut r = phxr(&[(0.0, 1.0, 1.0), (3000.0, 1.0, 1.0)], 0, &[0]);
        normalize(&mut r, &map);
        assert_eq!(off_target(&r), Some(1));
        if let Origin::Phxr(j) = &mut r.origin {
            j.flip = (true, false);
        }
        normalize(&mut r, &map);
        assert_eq!(off_target(&r), Some(0));
    }

    #[test]
    fn results_beyond_the_map_are_left_out() {
        // Paired with a map that is too short: no panic, no phantom hits.
        let map = map_with(&[(1000, 1.0, 1.0)]);
        let mut r = phxr(&[(0.0, 0.0, 0.0), (3000.0, 0.0, 0.0)], 0, &[0, 0, 0]);
        normalize(&mut r, &map);
        assert_eq!(flagged(&r).len(), 1);
        let mut far = phxr(&[(0.0, 0.0, 0.0)], usize::MAX, &[0, 0]);
        normalize(&mut far, &map);
        assert!(flagged(&far).is_empty());
    }

    #[test]
    fn a_legacy_run_that_stops_short_gets_a_fail_time_too() {
        let map = map_with(&[(1000, 1.0, 1.0), (2000, 1.0, 1.0), (3000, 1.0, 1.0)]);
        let legacy = |judged_end| {
            replay_of(
                Origin::Sspre {
                    skips: vec![],
                    judged_end,
                    hits: vec![],
                },
                &[(0.0, 0.0, 0.0), (2100.0, 0.0, 0.0)],
                2,
                0,
            )
        };
        let mut stopped = legacy(Some(2));
        assert!(normalize(&mut stopped, &map));
        assert_eq!(stopped.fail_time_ms, 2100);
        assert!(!normalize(&mut stopped, &map));

        let mut complete = legacy(Some(3));
        assert!(!normalize(&mut complete, &map));
        assert_eq!(complete.fail_time_ms, -1);

        // A run that judged nothing says nothing about where it ended.
        let mut silent = legacy(None);
        assert!(!normalize(&mut silent, &map));
        assert_eq!(silent.fail_time_ms, -1);
    }

    /// A legacy replay as the parser hands it over: the samples, and every
    /// hit as (note, sample before it, sample behind it) on the run's own
    /// clock. The flagged frames come from pairing it with a map.
    fn legacy(samples: &[(f64, f32, f32)], hits: &[(u32, f64, f64)], skips: &[f64]) -> Replay {
        replay_of(
            Origin::Sspre {
                skips: skips
                    .iter()
                    .map(|&at_ms| Skip {
                        at_ms,
                        jump_ms: 0.0,
                    })
                    .collect(),
                judged_end: None,
                hits: hits
                    .iter()
                    .map(|&(note, after_ms, by_ms)| LegacyHit {
                        note,
                        after_ms,
                        by_ms,
                    })
                    .collect(),
            },
            samples,
            hits.len() as i32,
            0,
        )
    }

    fn judged(r: &Replay, map: &Map) -> (Vec<Option<f64>>, u32) {
        let outcome = judge(r, &map.notes);
        (
            outcome.results.iter().map(|n| n.hit_ms).collect(),
            outcome.orphan_flags,
        )
    }

    #[test]
    fn a_legacy_hit_taken_a_frame_behind_the_window_lands_at_its_end() {
        // The window of the note at 1000 shuts at 1055. The game still took
        // the hit in the frame after that, and the next cursor sample came
        // at 1066: one sample at 1054, inside the window, then that one.
        let map = map_with(&[(1000, 1.0, 1.0)]);
        let mut r = legacy(
            &[(990.0, 0.0, 0.0), (1054.0, 0.0, 0.0), (1066.0, 0.6, 0.0)],
            &[(0, 1054.0, 1066.0)],
            &[],
        );
        assert!(normalize(&mut r, &map));
        // At the window's end, on the recorded path between the samples.
        let at = flagged(&r);
        assert_eq!(at.len(), 1);
        assert_eq!(at[0].0, 1055.0);
        assert!((at[0].1 - 0.05).abs() < 1e-5, "{:?}", at[0]);
        // The samples are untouched and the frames still run forward.
        let samples: Vec<f64> = r.frames.iter().filter(|f| !f.hit).map(|f| f.ms).collect();
        assert_eq!(samples, vec![990.0, 1054.0, 1066.0]);
        assert!(r.frames.windows(2).all(|w| w[0].ms <= w[1].ms));
        assert_eq!(judged(&r, &map), (vec![Some(1055.0)], 0));
        // Found by the time alone as well, which the flip detection uses.
        let by_time = match_hits(&map.notes, &r.frames, hit_window_ms(&r));
        assert_eq!((by_time.derived_hits(), by_time.orphan_flags), (1, 0));
        assert!(!normalize(&mut r, &map), "a second pass changes nothing");
    }

    #[test]
    fn a_legacy_hit_stamped_just_before_its_note_lands_on_the_note() {
        // The game cuts its clock to whole milliseconds, which at a speed
        // mod can leave the sample behind a hit a moment before the note.
        let map = map_with(&[(1000, 1.0, 1.0)]);
        let mut r = legacy(
            &[(988.0, 0.0, 0.0), (999.4, 0.0, 0.0), (1011.0, 0.0, 0.0)],
            &[(0, 988.0, 999.4)],
            &[],
        );
        normalize(&mut r, &map);
        assert_eq!(flagged(&r)[0].0, 1000.0);
        assert_eq!(judged(&r, &map), (vec![Some(1000.0)], 0));
    }

    #[test]
    fn a_legacy_hit_inside_its_window_stays_on_its_sample() {
        let map = map_with(&[(1000, 1.0, 1.0)]);
        let mut r = legacy(
            &[(990.0, 0.0, 0.0), (1012.0, 0.25, 0.5)],
            &[(0, 990.0, 1012.0)],
            &[],
        );
        normalize(&mut r, &map);
        assert_eq!(flagged(&r), vec![(1012.0, 0.25, 0.5)]);
        // The sample itself stays a plain frame, with the hit behind it.
        let frames: Vec<(f64, bool)> = r.frames.iter().map(|f| (f.ms, f.hit)).collect();
        assert_eq!(
            frames,
            vec![(990.0, false), (1012.0, false), (1012.0, true)]
        );
    }

    #[test]
    fn a_legacy_run_is_judged_by_the_notes_its_file_names() {
        // Two notes 10 ms apart. The file says the first was missed and the
        // second hit. Going by the time alone the one flag goes to the
        // first note, which gets the totals right and the notes wrong.
        let map = map_with(&[(1000, 1.0, 1.0), (1010, 1.0, 1.0)]);
        let mut r = legacy(
            &[(990.0, 0.0, 0.0), (1012.0, 0.0, 0.0)],
            &[(1, 990.0, 1012.0)],
            &[],
        );
        r.misses = 1;
        normalize(&mut r, &map);
        let by_time = match_hits(&map.notes, &r.frames, hit_window_ms(&r));
        assert!(by_time.results[0].hit && !by_time.results[1].hit);
        assert_eq!(judged(&r, &map), (vec![None, Some(1012.0)], 0));
        // The same result against the notes as played.
        let mut mirrored = map.clone();
        for n in &mut mirrored.notes {
            n.x = 2.0 - n.x;
        }
        assert_eq!(judged(&r, &mirrored), (vec![None, Some(1012.0)], 0));
    }

    #[test]
    fn a_legacy_hit_the_notes_have_no_place_for_is_an_orphan() {
        let map = map_with(&[(1000, 1.0, 1.0), (5000, 1.0, 1.0)]);
        let mut r = legacy(
            &[
                (990.0, 0.0, 0.0),
                (1012.0, 0.0, 0.0),
                (1100.0, 0.0, 0.0),
                (1112.0, 0.0, 0.0),
                (4000.0, 0.0, 0.0),
                (4012.0, 0.0, 0.0),
            ],
            &[
                (0, 990.0, 1012.0),  // fine
                (0, 1012.0, 1100.0), // the same note again
                (0, 1100.0, 1112.0), // its window was shut at the sample before
                (1, 4000.0, 4012.0), // the note had not come yet
                (7, 4000.0, 4012.0), // no such note
            ],
            &[],
        );
        normalize(&mut r, &map);
        // Every hit keeps a flagged frame; the ones without room stay on
        // their samples.
        let times: Vec<f64> = flagged(&r).iter().map(|f| f.0).collect();
        assert_eq!(times, vec![1012.0, 1055.0, 1112.0, 4012.0, 4012.0]);
        assert_eq!(judged(&r, &map), (vec![Some(1012.0), None], 4));
        let report = crate::integrity::verify_replay(&r, &map);
        assert!(!report.consistent());
    }

    #[test]
    fn legacy_hits_behind_a_skip_follow_the_jump_and_another_map_starts_over() {
        // At 1.00x a skip lands 2000 ms before the next note: from 500 to
        // 58000 against the note at 60000, a jump of 57500.
        let map = map_with(&[(60000, 1.0, 1.0)]);
        let mut r = legacy(
            &[
                (400.0, 0.0, 0.0),
                (600.0, 0.0, 0.0),
                (2500.0, 0.0, 0.0),
                (2560.0, 0.0, 0.0),
            ],
            &[(0, 2500.0, 2560.0)],
            &[500.0],
        );
        assert!(normalize(&mut r, &map));
        // 2560 on the run's clock is 60060 now, 5 ms behind the window.
        assert_eq!(
            flagged(&r).iter().map(|f| f.0).collect::<Vec<_>>(),
            vec![60055.0]
        );
        assert_eq!(judged(&r, &map), (vec![Some(60055.0)], 0));
        assert!(!normalize(&mut r, &map));

        let other = map_with(&[(30010, 1.0, 1.0)]);
        assert!(normalize(&mut r, &other));
        // Jump 27510: the samples sit at 30010 and 30070 now.
        assert_eq!(
            flagged(&r).iter().map(|f| f.0).collect::<Vec<_>>(),
            vec![30065.0]
        );
        assert_eq!(judged(&r, &other), (vec![Some(30065.0)], 0));
        assert!(normalize(&mut r, &map));
        assert_eq!(
            flagged(&r).iter().map(|f| f.0).collect::<Vec<_>>(),
            vec![60055.0]
        );
    }
}
