//! Matches the replay's per-frame hit flags to individual map notes.
//!
//! Each `.rhr` frame carries a flag set exactly on the frame where a note
//! was hit; the count of flagged frames equals the header hit count. Notes
//! are hit in chronological order, so flagged frames form an
//! order-preserving subsequence of the notes: the correct alignment is a
//! monotonic two-pointer walk. Validated against all four reference replays,
//! this reproduces the header hit/miss counts exactly; observed
//! |flag − note| deltas reach exactly 80 ms (the game's ~55 ms hit window
//! plus ~17 ms frame quantization), which pins the default window.
//!
//! Unlike the naive walk in rhr2mp4, a flag that can no longer match any
//! future note (its time is more than the window before the next
//! candidate note) is counted as an orphan and skipped instead of stalling
//! the pointer and cascading misses.
//!
//! Known limit: when a flag falls inside the window of several unassigned
//! notes, the earliest note wins. Timing alone cannot disambiguate that;
//! a later phase can refine per-note attribution using the cursor position
//! of the flagged frame vs. the note's grid position. Totals are unaffected.

use rhythia_formats::map::Note;
use rhythia_formats::rhr::Frame;

/// Tolerance between a flagged frame and its note. Empirical maximum on
/// real replays is exactly 80 ms; see module docs.
pub const DEFAULT_WINDOW_MS: f64 = 80.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteResult {
    /// Index into the notes slice this result belongs to.
    pub note_index: usize,
    pub hit: bool,
    /// Song time of the flagged frame that hit this note.
    pub hit_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct MatchOutcome {
    /// One entry per input note, in note order.
    pub results: Vec<NoteResult>,
    /// Flagged frames that matched no note within the window. Always 0 for
    /// consistent replays; nonzero feeds the integrity check.
    pub orphan_flags: u32,
}

impl MatchOutcome {
    pub fn derived_hits(&self) -> u32 {
        self.results.iter().filter(|r| r.hit).count() as u32
    }
}

/// Aligns flagged frames to notes with a monotonic two-pointer walk.
/// Notes must be sorted by time (Map guarantees this); flags are visited
/// in frame order.
pub fn match_hits(notes: &[Note], frames: &[Frame], window_ms: f64) -> MatchOutcome {
    let flags: Vec<f64> = frames.iter().filter(|f| f.hit).map(|f| f.ms).collect();

    let mut results: Vec<NoteResult> = (0..notes.len())
        .map(|i| NoteResult {
            note_index: i,
            hit: false,
            hit_ms: None,
        })
        .collect();

    let mut orphan_flags = 0u32;
    let mut fi = 0usize;

    for (ni, note) in notes.iter().enumerate() {
        let note_ms = note.time_ms as f64;
        // A flag more than `window_ms` before this note can never match it
        // or any later note — orphan it instead of stalling (rhr2mp4 bug).
        while fi < flags.len() && flags[fi] < note_ms - window_ms {
            orphan_flags += 1;
            fi += 1;
        }
        if fi < flags.len() && (flags[fi] - note_ms).abs() <= window_ms {
            results[ni].hit = true;
            results[ni].hit_ms = Some(flags[fi]);
            fi += 1;
        }
    }
    // Flags left after the last note matched nothing.
    orphan_flags += (flags.len() - fi) as u32;

    MatchOutcome {
        results,
        orphan_flags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(time_ms: i64) -> Note {
        Note {
            time_ms,
            x: 0.0,
            y: 0.0,
        }
    }

    fn flag(ms: f64) -> Frame {
        Frame {
            ms,
            x: 0.0,
            y: 0.0,
            health: 1.0,
            hit: true,
        }
    }

    #[test]
    fn simple_hits_and_miss() {
        let notes = [note(1000), note(2000), note(3000)];
        let frames = [flag(1010.0), flag(3020.0)];
        let out = match_hits(&notes, &frames, DEFAULT_WINDOW_MS);
        assert!(out.results[0].hit);
        assert!(!out.results[1].hit);
        assert!(out.results[2].hit);
        assert_eq!(out.orphan_flags, 0);
        assert_eq!(out.derived_hits(), 2);
    }

    #[test]
    fn dense_section_matches_in_order() {
        // Notes 50 ms apart with one flag each: order-preserving walk
        // pairs them one-to-one.
        let notes = [note(1000), note(1050)];
        let frames = [flag(1010.0), flag(1055.0)];
        let out = match_hits(&notes, &frames, DEFAULT_WINDOW_MS);
        assert!(out.results[0].hit && out.results[1].hit);
        assert_eq!(out.results[0].hit_ms, Some(1010.0));
        assert_eq!(out.results[1].hit_ms, Some(1055.0));
        assert_eq!(out.orphan_flags, 0);
    }

    #[test]
    fn single_flag_between_notes_takes_earliest() {
        // A lone flag inside two notes' windows is ambiguous on timing
        // alone; the monotonic walk assigns the earliest note. (Per-note
        // spatial disambiguation is a documented later refinement.)
        let notes = [note(1000), note(1050)];
        let frames = [flag(1040.0)];
        let out = match_hits(&notes, &frames, DEFAULT_WINDOW_MS);
        assert!(out.results[0].hit);
        assert!(!out.results[1].hit);
        assert_eq!(out.derived_hits(), 1);
    }

    #[test]
    fn out_of_window_flag_does_not_stall_matching() {
        // A stray flag far from any note must not block later matches
        // (the rhr2mp4 stuck-pointer bug).
        let notes = [note(5000), note(6000)];
        let frames = [flag(1000.0), flag(5005.0), flag(6010.0)];
        let out = match_hits(&notes, &frames, DEFAULT_WINDOW_MS);
        assert_eq!(out.orphan_flags, 1);
        assert!(out.results[0].hit);
        assert!(out.results[1].hit);
    }

    #[test]
    fn double_flag_for_one_note_leaves_an_orphan() {
        let notes = [note(1000)];
        let frames = [flag(995.0), flag(1005.0)];
        let out = match_hits(&notes, &frames, DEFAULT_WINDOW_MS);
        assert_eq!(out.derived_hits(), 1);
        assert_eq!(out.orphan_flags, 1);
    }

    #[test]
    fn trailing_orphan_after_last_note() {
        let notes = [note(1000)];
        let frames = [flag(1005.0), flag(9999.0)];
        let out = match_hits(&notes, &frames, DEFAULT_WINDOW_MS);
        assert_eq!(out.derived_hits(), 1);
        assert_eq!(out.orphan_flags, 1);
    }
}
