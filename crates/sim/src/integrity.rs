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

/// Convenience: hitreg + verify in one call with the default window.
pub fn verify_replay(replay: &Replay, map: &Map) -> IntegrityReport {
    let outcome = hitreg::match_hits(&map.notes, &replay.frames, hitreg::DEFAULT_WINDOW_MS);
    verify(replay, map, &outcome)
}
