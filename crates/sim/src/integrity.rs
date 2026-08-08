//! Replay integrity check (project hard rule #1).
//!
//! Derives hits/misses/accuracy from the frame stream + map and compares
//! them with the replay's header values. Any mismatch means the replay is
//! inconsistent — possibly edited — and every consumer (CLI, GUI, video
//! renderer) must surface a clear warning, including one burned into the
//! rendered video.
//!
//! Empirically pinned rules (validated against the four reference replays):
//!  * accuracy == hits / (hits + misses) × 100
//!  * attempted notes: all notes when passed; notes with
//!    time ≤ failTime when failed (lastFrame+window overshoots by one).

use rhythia_formats::map::Map;
use rhythia_formats::rhr::Replay;

use crate::hitreg::{self, MatchOutcome};

/// Header accuracy is an f32 computed by the game; allow for its rounding.
const ACCURACY_TOLERANCE_PCT: f64 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Mismatch in gameplay data — treat the replay as possibly manipulated.
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
    // the replay was recorded on — stronger than any heuristic below. Ignoring
    // it let a run whose id mismatch is printed two lines above still read as
    // "possibly manipulated" when the timing heuristics happened not to fire.
    let id_mismatch = report.checks.iter().any(|c| {
        !c.ok
            && (c.name == "map online id matches replay"
                || c.name == "map legacy id matches replay")
    });
    if id_mismatch {
        return true;
    }
    let flags = report.flagged_frames;
    let header_hits = replay_hits.max(0) as u32;
    // A wrong map cannot change the FILE: the number of flagged frames still
    // matches the header it was written with. A header inflated past its own
    // frames is the opposite — evidence about the replay, not the chart — so
    // it must keep reading as inconsistent instead of being explained away.
    if flags != header_hits {
        return false;
    }
    // With an honest header, a third of the recorded hits finding no note at
    // all, or barely half of them landing, is what a foreign chart looks
    // like. It is also what injected hit flags would look like, which is why
    // the wording this feeds is "may not match" rather than a verdict — only
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

    let attempted_notes = if replay.failed() {
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
    if replay.failed() {
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
    if let Some(online_id) = map.meta.online_id {
        push(
            "map online id matches replay",
            Severity::Warning,
            online_id == i64::from(replay.map_id),
            replay.map_id.to_string(),
            online_id.to_string(),
        );
    }
    if !map.meta.legacy_id.is_empty() && !replay.legacy_map_id.is_empty() {
        push(
            "map legacy id matches replay",
            Severity::Warning,
            map.meta.legacy_id == replay.legacy_map_id,
            replay.legacy_map_id.clone(),
            map.meta.legacy_id.clone(),
        );
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
    let window = hitreg::hit_window_ms(replay);
    let outcome = hitreg::match_hits(&map.notes, &replay.frames, window);
    verify(replay, map, &outcome)
}

#[cfg(test)]
mod wrong_map_tests {
    use super::*;

    fn report_with(checks: Vec<Check>, flagged: u32, derived_hits: u32, orphans: u32) -> IntegrityReport {
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

    /// A passing id check with quiet heuristics is NOT the wrong map — this
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
