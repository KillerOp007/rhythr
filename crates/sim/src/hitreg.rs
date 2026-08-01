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

/// Half-width of the game's hit area (NoteManager.gd `note_hitbox_size`
/// 1.1375): the fixed square around a note's centre the cursor must
/// cover, in world units. Attribution and the analyzer's hit-area boxes
/// share this one constant.
pub const HITBOX_HALF: f32 = 0.56875;

/// Grid cell (0..2) to world units, the game's own mapping.
fn note_world(n: &Note) -> (f32, f32) {
    (n.x - 1.0, 1.0 - n.y)
}

fn covers(fx: f32, fy: f32, n: &Note) -> bool {
    let (wx, wy) = note_world(n);
    (fx - wx).abs() <= HITBOX_HALF && (fy - wy).abs() <= HITBOX_HALF
}

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

/// Aligns flagged frames to notes with a monotonic two-pointer walk,
/// then refines the attribution with the recorded cursor (see Phase 2
/// below). Notes must be sorted by time (Map guarantees this).
pub fn match_hits(notes: &[Note], frames: &[Frame], window_ms: f64) -> MatchOutcome {
    match_hits_inner(notes, frames, window_ms, true)
}

/// Timing-only matching, no cursor-guided reattribution. Use when the
/// note coordinates are NOT yet in the cursor's space — mirror-flip
/// detection runs on the unflipped map, and letting Phase 2 "correct"
/// attributions against geometry the player never saw can invert the
/// detected axis.
pub fn match_hits_timing_only(notes: &[Note], frames: &[Frame], window_ms: f64) -> MatchOutcome {
    match_hits_inner(notes, frames, window_ms, false)
}

fn match_hits_inner(
    notes: &[Note],
    frames: &[Frame],
    window_ms: f64,
    cursor_guided: bool,
) -> MatchOutcome {
    let flag_frames: Vec<(f64, f32, f32)> = frames
        .iter()
        .filter(|f| f.hit)
        .map(|f| (f.ms, f.x, f.y))
        .collect();
    let flags: Vec<f64> = flag_frames.iter().map(|f| f.0).collect();

    let mut results: Vec<NoteResult> = (0..notes.len())
        .map(|i| NoteResult {
            note_index: i,
            hit: false,
            hit_ms: None,
        })
        .collect();

    let mut orphan_flags = 0u32;
    let mut fi = 0usize;
    // Which flag frame each hit note owns — Phase 2 swaps move the INDEX,
    // so the cursor lookup can never alias two flags with equal stamps.
    let mut flag_of: Vec<Option<usize>> = vec![None; notes.len()];

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
            flag_of[ni] = Some(fi);
            fi += 1;
        }
    }
    // Flags left after the last note matched nothing.
    orphan_flags += (flags.len() - fi) as u32;

    // Phase 2 — cursor-guided reattribution. Timing alone cannot tell
    // near-simultaneous notes apart, and the earliest-note rule above
    // sometimes hands a flag to the WRONG one: the analyzer then paints
    // a hit box the cursor never touched and a miss box it sat inside.
    // The flag frame recorded the cursor, so use it: a flag moves from
    // its note to a missed neighbour when the cursor covered the missed
    // note's hit area and NOT the attributed one. Totals never change —
    // each swap trades one hit and one miss between two notes.
    //
    // A note arms at its chart time: a flag may precede its true note
    // only by the replay's ~17 ms frame-stamp quantization (module docs),
    // never by the full window — otherwise a cursor parked on a
    // soon-future note's cell steals flags and manufactures impossible
    // early hits (negative timing errors).
    const EARLY_SLACK_MS: f64 = 17.0;
    // A mis-shifted CHAIN (every flag one note early) unravels one link
    // per pass, from the tail backwards — so run to convergence. Each
    // swap strictly increases the number of cursor-consistent hits, so
    // this terminates within one pass per note; the cap is a backstop.
    let passes = if cursor_guided { notes.len().max(1) } else { 0 };
    for _pass in 0..passes {
        let mut changed = false;
        for mi in 0..results.len() {
            if results[mi].hit {
                continue;
            }
            let miss_note = &notes[mi];
            let miss_t = miss_note.time_ms as f64;
            for hi in 0..results.len() {
                if !results[hi].hit {
                    continue;
                }
                let Some(fidx) = flag_of[hi] else { continue };
                let (fm, fx, fy) = flag_frames[fidx];
                if fm - miss_t > window_ms || miss_t - fm > EARLY_SLACK_MS {
                    continue;
                }
                // Only when the cursor is unambiguous: inside the missed
                // note's area, outside the attributed one's.
                if covers(fx, fy, miss_note) && !covers(fx, fy, &notes[hi]) {
                    results[mi].hit = true;
                    results[mi].hit_ms = Some(fm);
                    flag_of[mi] = Some(fidx);
                    results[hi].hit = false;
                    results[hi].hit_ms = None;
                    flag_of[hi] = None;
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }

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

    /// The pass_long real-replay case: two near-simultaneous notes, the
    /// flag lands on the earlier one by time but the cursor sat on the
    /// later one — attribution must follow the cursor.
    #[test]
    fn cursor_reattributes_swapped_double_note() {
        let notes = [
            Note { time_ms: 1000, x: 0.0, y: 2.0 }, // world (-1, -1)
            Note { time_ms: 1005, x: 1.0, y: 2.0 }, // world (0, -1)
        ];
        let frames = [Frame { ms: 1004.0, x: 0.0, y: -1.0, health: 1.0, hit: true }];
        let out = match_hits(&notes, &frames, DEFAULT_WINDOW_MS);
        assert!(!out.results[0].hit, "cursor never covered note 0");
        assert!(out.results[1].hit, "cursor sat on note 1");
        assert_eq!(out.results[1].hit_ms, Some(1004.0));
        assert_eq!(out.derived_hits(), 1);
    }

    /// Overlapping areas (adjacent cells overlap by 0.1375): when the
    /// cursor covers BOTH notes the earliest-note rule must stand.
    #[test]
    fn ambiguous_cursor_keeps_earliest_attribution() {
        let notes = [
            Note { time_ms: 1000, x: 0.0, y: 2.0 }, // world (-1, -1)
            Note { time_ms: 1005, x: 1.0, y: 2.0 }, // world (0, -1)
        ];
        // Cursor midway: covers both areas.
        let frames = [Frame { ms: 1004.0, x: -0.5, y: -1.0, health: 1.0, hit: true }];
        let out = match_hits(&notes, &frames, DEFAULT_WINDOW_MS);
        assert!(out.results[0].hit);
        assert!(!out.results[1].hit);
    }

    /// A flag must never be stolen by a note far in the FUTURE — the
    /// hitbox arms at the note's chart time (early slack = one frame).
    #[test]
    fn future_note_cannot_steal_a_flag() {
        let notes = [
            Note { time_ms: 1000, x: 0.0, y: 0.0 }, // world (-1, 1)
            Note { time_ms: 1075, x: 2.0, y: 0.0 }, // world (1, 1)
        ];
        // Flag at 1000 for note 0, but the cursor already left toward
        // note 1's cell (fast jump + frame quantization).
        let frames = [Frame { ms: 1000.0, x: 1.0, y: 1.0, health: 1.0, hit: true }];
        let out = match_hits(&notes, &frames, DEFAULT_WINDOW_MS);
        assert!(out.results[0].hit, "note 0 keeps its flag");
        assert!(!out.results[1].hit, "a note 75 ms in the future must not steal it");
    }

    /// Totals are invariant under reattribution.
    #[test]
    fn reattribution_never_changes_totals() {
        let notes = [
            Note { time_ms: 1000, x: 0.0, y: 0.0 },
            Note { time_ms: 1010, x: 2.0, y: 0.0 },
            Note { time_ms: 1020, x: 1.0, y: 1.0 },
        ];
        let frames = [
            Frame { ms: 1008.0, x: 1.0, y: 1.0, health: 1.0, hit: true }, // on note 1
            Frame { ms: 1022.0, x: 0.0, y: 0.0, health: 1.0, hit: true }, // on note 2
        ];
        let out = match_hits(&notes, &frames, DEFAULT_WINDOW_MS);
        assert_eq!(out.derived_hits(), 2);
        assert_eq!(out.orphan_flags, 0);
    }
}
