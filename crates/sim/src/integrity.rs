//! Replay integrity check (project hard rule #1).
//!
//! Derives hits/misses/accuracy from the frame stream + map and compares
//! them with the replay's header values. Any mismatch means the replay is
//! inconsistent (possibly edited) and every consumer (CLI, GUI, video
//! renderer) must surface a clear warning, including one burned into the
//! rendered video.
//!
//! Empirically pinned rules (validated against the four reference replays):
//!  * accuracy == hits / (hits + misses) × 100
//!  * attempted notes: all notes when passed; notes with
//!    time ≤ failTime when failed (lastFrame+window overshoots by one).

use rhythia_formats::map::Map;
use rhythia_formats::rhr::{Origin, Replay};

use crate::hitreg::{self, MatchOutcome};

/// Header accuracy is an f32 computed by the game; allow for its rounding.
const ACCURACY_TOLERANCE_PCT: f64 = 0.01;

/// Name of the check that compares the note count a converted replay
/// carries with the loaded map's.
const NOTE_COUNT_CHECK: &str = "map has the notes the replay judged";

/// Name of the check that holds a `.phxr` replay's hits against its
/// recorded cursor.
const CURSOR_CHECK: &str = "cursor on the note at every hit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Mismatch in gameplay data: treat the replay as possibly manipulated.
    Error,
    /// Suspicious but not by itself proof of tampering.
    Warning,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub name: &'static str,
    pub severity: Severity,
    pub ok: bool,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Clone)]
pub struct IntegrityReport {
    pub flagged_frames: u32,
    pub derived_hits: u32,
    pub derived_misses: u32,
    pub attempted_notes: u32,
    pub derived_accuracy_pct: f64,
    pub orphan_flags: u32,
    pub checks: Vec<Check>,
}

/// Whether an inconsistent verdict is better explained by the wrong CHART
/// than by a tampered replay.
///
/// This lived in the desktop app, so the Analyze window and the CLI kept
/// telling people their file "may be corrupted or edited" when all they had
/// done was load somebody else's map. It needs nothing but the replay and the
/// report, so it belongs here where all three can reach it.
pub fn looks_like_the_wrong_map(
    replay_hits: i32,
    report: &IntegrityReport,
    hash_mismatch: bool,
) -> bool {
    if hash_mismatch {
        return true;
    }
    // A failed map-id check is direct evidence the loaded chart is not the one
    // the replay was recorded on (stronger than any heuristic below). Ignoring
    // it let a run whose id mismatch is printed two lines above still read as
    // "possibly manipulated" when the timing heuristics happened not to fire.
    let id_mismatch = report.checks.iter().any(|c| {
        !c.ok
            && (c.name == "map online id matches replay"
                || c.name == "map legacy id matches replay"
                || c.name == NOTE_COUNT_CHECK
                // A cursor that was nowhere near a large share of the
                // notes it hit was playing other notes.
                || (c.name == CURSOR_CHECK && c.severity == Severity::Error))
    });
    if id_mismatch {
        return true;
    }
    let flags = report.flagged_frames;
    let header_hits = replay_hits.max(0) as u32;
    // A wrong map cannot change the FILE: the number of flagged frames still
    // matches the header it was written with. A header inflated past its own
    // frames is the opposite (evidence about the replay, not the chart), so
    // it must keep reading as inconsistent instead of being explained away.
    if flags != header_hits {
        return false;
    }
    // With an honest header, a third of the recorded hits finding no note at
    // all, or barely half of them landing, is what a foreign chart looks
    // like. It is also what injected hit flags would look like, which is why
    // the wording this feeds is "may not match" rather than a verdict: only
    // the map hash can tell those apart, and that is the branch above.
    let orphan_heavy = flags > 20 && u64::from(report.orphan_flags) * 3 > u64::from(flags);
    let lost_most = header_hits > 20 && report.derived_hits * 2 < header_hits;
    orphan_heavy || lost_most
}

impl IntegrityReport {
    /// True when every Error-level check passed. Warnings never make a
    /// replay "inconsistent" on their own.
    pub fn consistent(&self) -> bool {
        self.checks
            .iter()
            .all(|c| c.ok || c.severity == Severity::Warning)
    }

    pub fn failed_checks(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| !c.ok)
    }
}

/// Runs the integrity check. `outcome` must come from
/// [`hitreg::match_hits`] over the same replay and map.
pub fn verify(replay: &Replay, map: &Map, outcome: &MatchOutcome) -> IntegrityReport {
    let flagged_frames = replay.flagged_frames();
    let derived_hits = outcome.derived_hits();

    let converted = replay.origin.converted();
    let attempted_notes = if converted {
        // The other clients' formats name every note they judged, so the
        // count is the file's own. Deriving it from the map would be less
        // exact: a run that stops mid-map leaves the notes of its last
        // few milliseconds unjudged, and one started mid-map never saw
        // the notes before its start.
        (replay.hits.max(0) as u32).saturating_add(replay.misses.max(0) as u32)
    } else if replay.failed() {
        map.notes
            .iter()
            .filter(|n| n.time_ms <= i64::from(replay.fail_time_ms))
            .count() as u32
    } else {
        map.notes.len() as u32
    };
    let derived_misses = attempted_notes.saturating_sub(derived_hits);

    let derived_accuracy_pct = if attempted_notes > 0 {
        f64::from(derived_hits) / f64::from(attempted_notes) * 100.0
    } else {
        100.0
    };

    let mut checks = Vec::new();
    let mut push = |name, severity, ok, expected: String, actual: String| {
        checks.push(Check {
            name,
            severity,
            ok,
            expected,
            actual,
        });
    };

    push(
        "flagged frames == header hits",
        Severity::Error,
        i64::from(flagged_frames) == i64::from(replay.hits),
        replay.hits.to_string(),
        flagged_frames.to_string(),
    );
    push(
        "matched hits == header hits",
        Severity::Error,
        i64::from(derived_hits) == i64::from(replay.hits),
        replay.hits.to_string(),
        derived_hits.to_string(),
    );
    push(
        "derived misses == header misses",
        Severity::Error,
        i64::from(derived_misses) == i64::from(replay.misses),
        replay.misses.to_string(),
        derived_misses.to_string(),
    );
    push(
        "derived accuracy == header accuracy",
        Severity::Error,
        (derived_accuracy_pct - f64::from(replay.accuracy_pct)).abs() <= ACCURACY_TOLERANCE_PCT,
        format!("{:.4}", replay.accuracy_pct),
        format!("{derived_accuracy_pct:.4}"),
    );
    push(
        "no orphan hit flags",
        Severity::Error,
        outcome.orphan_flags == 0,
        "0".into(),
        outcome.orphan_flags.to_string(),
    );
    // Only a `.rhr` records health. A converted run that ends early may
    // just as well have been abandoned at full health.
    if replay.failed() && !converted {
        let min_health = replay
            .frames
            .iter()
            .map(|f| f.health)
            .fold(f32::INFINITY, f32::min);
        push(
            "health reaches 0 on fail",
            Severity::Error,
            min_health <= 0.0,
            "<= 0".into(),
            format!("{min_health}"),
        );
    }
    push(
        "no trailing bytes after frames",
        Severity::Warning,
        replay.trailing_bytes == 0,
        "0".into(),
        replay.trailing_bytes.to_string(),
    );
    // The recorder writes frames in time order, so a backwards step means
    // the stream was spliced or reordered. Hit counts can survive such a
    // splice (reordering non-flag frames leaves them intact), so this is a
    // tamper signal the count checks above can miss. Warning-level: the
    // invariant rests on the four reference replays, not a formal guarantee.
    let first_backstep = replay.frames.windows(2).position(|w| w[1].ms < w[0].ms);
    push(
        "frame times non-decreasing",
        Severity::Warning,
        first_backstep.is_none(),
        "monotonic".into(),
        first_backstep.map_or_else(
            || "monotonic".into(),
            |i| format!("frame {} steps back", i + 1),
        ),
    );
    // A converted replay names its map by legacy id only; it has no
    // online id to compare.
    if let Some(online_id) = map.meta.online_id.filter(|_| !converted) {
        push(
            "map online id matches replay",
            Severity::Warning,
            online_id == i64::from(replay.map_id),
            replay.map_id.to_string(),
            online_id.to_string(),
        );
    }
    // A `.phxr` names its map by the folder it sat in on the player's
    // disk, which is a file name, not an id: it proves nothing either way.
    let names_its_map = !matches!(replay.origin, Origin::Phxr(_));
    if names_its_map && !map.meta.legacy_id.is_empty() && !replay.legacy_map_id.is_empty() {
        push(
            "map legacy id matches replay",
            Severity::Warning,
            map.meta.legacy_id == replay.legacy_map_id,
            replay.legacy_map_id.clone(),
            map.meta.legacy_id.clone(),
        );
    }
    match &replay.origin {
        Origin::Rhr => {}
        Origin::Sspre { judged_end, .. } => {
            // The file names its notes by index; an index past the map's
            // last note belongs to another map.
            if let Some(end) = *judged_end {
                push(
                    NOTE_COUNT_CHECK,
                    Severity::Error,
                    end as usize <= map.notes.len(),
                    format!("at least {end}"),
                    map.notes.len().to_string(),
                );
            }
        }
        Origin::Phxr(judgments) => {
            // The file states the note count of its map. Any other count
            // is another chart, and every result would land on the wrong
            // note.
            push(
                NOTE_COUNT_CHECK,
                Severity::Error,
                judgments.map_notes == map.notes.len() as u64,
                judgments.map_notes.to_string(),
                map.notes.len().to_string(),
            );
            // The client itself refuses a file whose checksum is off, so
            // one that fails here was changed after it was written (or
            // cut short). Anyone can recompute the checksum; passing it
            // proves nothing, failing it does.
            push(
                "replay checksum intact",
                Severity::Error,
                judgments.checksum_ok,
                "matches".into(),
                if judgments.checksum_ok {
                    "matches".into()
                } else {
                    "does not match".into()
                },
            );
            // The flagged frames of a `.phxr` are built from its own note
            // results, so the count checks above cannot tell whether the
            // results fit the recording. Whether the recorded cursor was
            // on the note at each hit can. A few stray hits stay a
            // warning: the path between two samples is an estimate (see
            // `pairing`). A large share of them is not an estimate any
            // more: the cursor was playing other notes.
            if let Some(off_target) = judgments.off_target_hits {
                let hits = replay.hits.max(0) as u32;
                let widespread = off_target > 20 && off_target.saturating_mul(10) > hits;
                push(
                    CURSOR_CHECK,
                    if widespread {
                        Severity::Error
                    } else {
                        Severity::Warning
                    },
                    off_target == 0,
                    "0 hits off target".into(),
                    format!("{off_target} hits off target"),
                );
            }
        }
    }

    IntegrityReport {
        flagged_frames,
        derived_hits,
        derived_misses,
        attempted_notes,
        derived_accuracy_pct,
        orphan_flags: outcome.orphan_flags,
        checks,
    }
}

/// Convenience: hitreg + verify in one call with the run's own hit window.
pub fn verify_replay(replay: &Replay, map: &Map) -> IntegrityReport {
    let outcome = hitreg::judge(replay, &map.notes);
    verify(replay, map, &outcome)
}

#[cfg(test)]
mod wrong_map_tests {
    use super::*;

    fn report_with(
        checks: Vec<Check>,
        flagged: u32,
        derived_hits: u32,
        orphans: u32,
    ) -> IntegrityReport {
        IntegrityReport {
            flagged_frames: flagged,
            derived_hits,
            derived_misses: 0,
            attempted_notes: flagged,
            derived_accuracy_pct: 0.0,
            orphan_flags: orphans,
            checks,
        }
    }

    fn id_check(ok: bool) -> Check {
        Check {
            name: "map legacy id matches replay",
            severity: Severity::Warning,
            ok,
            expected: String::new(),
            actual: String::new(),
        }
    }

    /// The case reproduced from testdata: a failed id check is definitive
    /// evidence of the wrong chart, even when the timing heuristics do not
    /// fire (enough hits line up by coincidence). It used to read as
    /// "possibly manipulated" with the id mismatch printed two lines above.
    #[test]
    fn a_failed_id_check_alone_means_wrong_map() {
        // Heuristics deliberately quiet: flags==header, few orphans, most hits land.
        let report = report_with(vec![id_check(false)], 471, 366, 105);
        assert!(looks_like_the_wrong_map(471, &report, false));
    }

    /// A legacy Sound Space Plus replay knows its map by name only. Holding
    /// its missing online id against a map that has one would call every
    /// such run "the wrong map".
    #[test]
    fn a_converted_replay_is_not_checked_against_an_online_id() {
        use rhythia_formats::map::{Map, MapMeta, Note};
        use rhythia_formats::rhr::{Frame, Origin, Replay};
        let map = Map {
            meta: MapMeta {
                online_id: Some(13076),
                legacy_id: "some_map".into(),
                ..Default::default()
            },
            notes: vec![Note {
                time_ms: 1000,
                x: 1.0,
                y: 1.0,
            }],
            ..Default::default()
        };
        let frame = Frame {
            ms: 1010.0,
            x: 0.0,
            y: 0.0,
            health: 1.0,
            hit: true,
        };
        let mut replay = Replay {
            version: 5,
            timestamp_ticks: 0,
            player_name: String::new(),
            legacy_map_id: "some_map".into(),
            map_id: 0,
            start_from_ms: 0,
            mode: String::new(),
            passed: true,
            mods: "[]".into(),
            spin: false,
            speed: 1.0,
            total_score: 0,
            accuracy_pct: 100.0,
            hits: 1,
            misses: 0,
            points: 0.0,
            fail_time_ms: -1,
            beatmap_hash: String::new(),
            pitch_lock: false,
            frames: vec![frame],
            trailing_bytes: 0,
            origin: Origin::Sspre {
                skips: vec![],
                judged_end: None,
                hits: vec![],
            },
        };
        let names = |r: &Replay| -> Vec<&'static str> {
            verify_replay(r, &map)
                .checks
                .iter()
                .map(|c| c.name)
                .collect()
        };
        assert!(!names(&replay).contains(&"map online id matches replay"));
        assert!(names(&replay).contains(&"map legacy id matches replay"));
        replay.origin = Origin::Rhr;
        assert!(names(&replay).contains(&"map online id matches replay"));
    }

    /// A passing id check with quiet heuristics is NOT the wrong map: this
    /// must not fire on an honest run just because an id check exists.
    #[test]
    fn a_passing_id_check_is_not_a_wrong_map() {
        let report = report_with(vec![id_check(true)], 471, 366, 105);
        assert!(!looks_like_the_wrong_map(471, &report, false));
    }

    /// The heuristic path still stands on its own when no id check is present
    /// (a cache-JSON map carries no legacy id): most hits finding no note.
    #[test]
    fn heuristic_still_catches_a_wrong_map_without_an_id_check() {
        let report = report_with(vec![], 100, 30, 5); // derived_hits*2 < header
        assert!(looks_like_the_wrong_map(100, &report, false));
    }
}

#[cfg(test)]
mod converted_tests {
    use super::*;
    use crate::timebase::normalize;
    use rhythia_formats::map::{MapMeta, Note};
    use rhythia_formats::rhr::{Frame, Judgments, LegacyHit};

    const MISS: u8 = Judgments::MISS;

    /// Four centre notes, a second apart.
    fn map() -> Map {
        Map {
            meta: MapMeta {
                legacy_id: "the_sspm_id".into(),
                ..Default::default()
            },
            notes: (1..=4)
                .map(|i| Note {
                    time_ms: i * 1000,
                    x: 1.0,
                    y: 1.0,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn replay_of(origin: Origin, last_sample_ms: f64, hits: i32, misses: i32) -> Replay {
        let sample = |ms| Frame {
            ms,
            x: 0.0,
            y: 0.0,
            health: 1.0,
            hit: false,
        };
        let judged = hits + misses;
        Replay {
            version: 5,
            timestamp_ticks: 0,
            player_name: String::new(),
            legacy_map_id: "folder name on disk".into(),
            map_id: 0,
            start_from_ms: 0,
            mode: String::new(),
            passed: true,
            mods: "[]".into(),
            spin: false,
            speed: 1.0,
            total_score: 0,
            accuracy_pct: if judged > 0 {
                hits as f32 / judged as f32 * 100.0
            } else {
                100.0
            },
            hits,
            misses,
            points: 0.0,
            fail_time_ms: -1,
            beatmap_hash: String::new(),
            pitch_lock: false,
            frames: vec![sample(0.0), sample(last_sample_ms)],
            trailing_bytes: 0,
            origin,
        }
    }

    /// A `.phxr` run over [`map`], paired with it.
    fn phxr(results: &[u8], last_sample_ms: f64) -> Replay {
        let hits = results.iter().filter(|&&r| r != MISS).count() as i32;
        let mut r = replay_of(
            Origin::Phxr(Judgments {
                first_note: 0,
                results: results.to_vec(),
                map_notes: 4,
                flip: (false, false),
                checksum_ok: true,
                off_target_hits: None,
            }),
            last_sample_ms,
            hits,
            results.len() as i32 - hits,
        );
        normalize(&mut r, &map());
        r
    }

    fn check<'a>(report: &'a IntegrityReport, name: &str) -> Option<&'a Check> {
        report.checks.iter().find(|c| c.name == name)
    }

    #[test]
    fn a_paired_phxr_run_is_consistent() {
        let r = phxr(&[0, 12, MISS, 200], 5000.0);
        let report = verify_replay(&r, &map());
        assert!(report.consistent(), "{:?}", report.checks);
        assert_eq!(
            (
                report.derived_hits,
                report.derived_misses,
                report.attempted_notes
            ),
            (3, 1, 4)
        );
        assert!(check(&report, "replay checksum intact").unwrap().ok);
        assert!(check(&report, NOTE_COUNT_CHECK).unwrap().ok);
        assert!(check(&report, CURSOR_CHECK).unwrap().ok);
        assert!(
            check(&report, "map legacy id matches replay").is_none(),
            "a folder name is not an id"
        );
        assert!(!looks_like_the_wrong_map(r.hits, &report, false));
    }

    #[test]
    fn a_phxr_run_abandoned_at_full_health_is_still_consistent() {
        // Two of four notes judged, both hit; the recording stops there.
        let r = phxr(&[0, 0], 2300.0);
        assert!(r.failed());
        let report = verify_replay(&r, &map());
        assert!(report.consistent(), "{:?}", report.checks);
        assert_eq!((report.attempted_notes, report.derived_misses), (2, 0));
        assert!(check(&report, "health reaches 0 on fail").is_none());
    }

    #[test]
    fn a_broken_checksum_makes_a_phxr_run_inconsistent() {
        let mut r = phxr(&[0, 0, 0, 0], 5000.0);
        if let Origin::Phxr(j) = &mut r.origin {
            j.checksum_ok = false;
        }
        let report = verify_replay(&r, &map());
        assert!(!report.consistent());
        let failed: Vec<&str> = report.failed_checks().map(|c| c.name).collect();
        assert_eq!(failed, ["replay checksum intact"]);
    }

    #[test]
    fn a_lateness_beyond_the_hit_window_does_not_pass_as_a_hit() {
        // 254 quarter ms is 63.5 ms, later than the game ever allows.
        let r = phxr(&[0, 254, 0, 0], 5000.0);
        let report = verify_replay(&r, &map());
        assert!(!report.consistent());
        assert_eq!(report.orphan_flags, 1);
    }

    #[test]
    fn another_note_count_points_at_the_wrong_map() {
        let mut r = phxr(&[0, 0, 0, 0], 5000.0);
        if let Origin::Phxr(j) = &mut r.origin {
            j.map_notes = 900;
        }
        let report = verify_replay(&r, &map());
        assert!(!check(&report, NOTE_COUNT_CHECK).unwrap().ok);
        assert!(
            !report.consistent(),
            "every result would sit on the wrong note"
        );
        assert!(looks_like_the_wrong_map(r.hits, &report, false));
    }

    #[test]
    fn a_cursor_that_missed_most_of_its_hits_points_at_the_wrong_map() {
        // Forty centre notes, all hit according to the file, with the
        // cursor parked in a corner throughout: same note count, other
        // chart.
        let map = Map {
            notes: (1..=40)
                .map(|i| Note {
                    time_ms: i * 500,
                    x: 1.0,
                    y: 1.0,
                })
                .collect(),
            ..Default::default()
        };
        let mut r = replay_of(
            Origin::Phxr(Judgments {
                first_note: 0,
                results: vec![0; 40],
                map_notes: 40,
                flip: (false, false),
                checksum_ok: true,
                off_target_hits: None,
            }),
            25_000.0,
            40,
            0,
        );
        for f in &mut r.frames {
            (f.x, f.y) = (1.3, 1.3);
        }
        normalize(&mut r, &map);
        let report = verify_replay(&r, &map);
        let off = check(&report, CURSOR_CHECK).unwrap();
        assert_eq!((off.ok, off.severity), (false, Severity::Error));
        assert!(!report.consistent());
        assert!(looks_like_the_wrong_map(r.hits, &report, false));
    }

    #[test]
    fn hits_with_the_cursor_elsewhere_raise_a_warning_only() {
        let mut r = phxr(&[0, 0, 0, 0], 5000.0);
        // The same run with the cursor parked in a corner.
        for f in &mut r.frames {
            (f.x, f.y) = (1.3, 1.3);
        }
        normalize(&mut r, &map());
        let report = verify_replay(&r, &map());
        let off = check(&report, CURSOR_CHECK).unwrap();
        assert!(!off.ok);
        assert_eq!(off.actual, "4 hits off target");
        assert_eq!(off.severity, Severity::Warning);
        assert!(report.consistent());
    }

    #[test]
    fn a_legacy_run_that_stops_mid_map_is_judged_by_its_own_count() {
        // Notes 0 and 1 judged (one hit, one miss), then the run ends. The
        // file names the note of its hit; the flagged frame comes from
        // pairing it with the map.
        let hit_on_the_first_note = vec![LegacyHit {
            note: 0,
            after_ms: 0.0,
            by_ms: 1004.0,
        }];
        let mut r = replay_of(
            Origin::Sspre {
                skips: vec![],
                judged_end: Some(2),
                hits: hit_on_the_first_note.clone(),
            },
            2300.0,
            1,
            1,
        );
        r.legacy_map_id = "the_sspm_id".into();
        normalize(&mut r, &map());
        assert_eq!(r.flagged_frames(), 1);
        assert_eq!(r.fail_time_ms, 2300);
        let report = verify_replay(&r, &map());
        assert!(report.consistent(), "{:?}", report.checks);
        assert_eq!((report.attempted_notes, report.derived_misses), (2, 1));
        assert!(check(&report, NOTE_COUNT_CHECK).unwrap().ok);

        // An index past the map's last note belongs to another map.
        r.origin = Origin::Sspre {
            skips: vec![],
            judged_end: Some(40),
            hits: hit_on_the_first_note,
        };
        let report = verify_replay(&r, &map());
        assert!(looks_like_the_wrong_map(r.hits, &report, false));
    }
}
