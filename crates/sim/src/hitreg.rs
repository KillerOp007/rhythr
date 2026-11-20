//! Matches the replay's per-frame hit flags to individual map notes.
//!
//! Each `.rhr` frame carries a flag set exactly on the frame where a note
//! was hit; the count of flagged frames equals the header hit count. Notes
//! are hit in chronological order, so flagged frames form an
//! order-preserving subsequence of the notes: the correct alignment is a
//! monotonic two-pointer walk. Validated against all four reference replays,
//! this reproduces the header hit/miss counts exactly. The tolerance is the
//! game's own hit window, which scales with the run's speed (see
//! [`hit_window_ms`]).
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
//!
//! Older recordings (before file version 20260517) flag the moment, not
//! the hit: one frame, however many notes the cursor took in it. There the
//! header names more hits than there are flags, and [`judge`] hands the
//! difference to the notes a flagged moment's cursor also covered (see
//! [`marks_moments`]).

use rhythia_formats::map::Note;
use rhythia_formats::rhr::{Frame, Origin, Replay, V_BEATMAP_HASH, V_INT32_TIME};

/// The game's base hit window: `Rhythia.gd` `hitwindow_ms`, default 55 ms.
pub const BASE_HIT_WINDOW_MS: f64 = 55.0;

/// The hit window the given run was played with, in song-time milliseconds.
///
/// The window is one-sided: a note is missed the moment
/// `ms > note_t + hit_window` (`NoteManager.gd:262`), and a flag never
/// precedes its note. It is **not** a constant: the game scales it with the
/// speed multiplier (`speed_hitwindow`, on by default), so it belongs to the
/// replay.
///
/// Measured across 37 leaderboard replays covering every mod combination:
/// with this window every recorded flag finds a note (zero orphans) and the
/// latest attributed hit lands just under it at each speed (55.0 at 1.00×,
/// 63.0 at 1.15×, 68.0 at 1.25×, 74.0 at 1.35×, 79.0 at 1.45×) and never
/// above. A flat 80 ms, which this replaced, was the 1.45× case generalised
/// to every run; it stretched attribution ~25 ms past the game's own miss
/// point on 1.00× replays.
///
/// Deliberately NOT modelled: `NoteManager.gd:398-399` narrows the window to
/// 0.8× under hardrock. The one hardrock replay we hold (testdata
/// `hardrock_score192.rhr`, 1.45×) contradicts it: its latest honest flag
/// sits at 78 ms, which needs the full 79.75, while 0.8× (63.8) orphans 5
/// flags and breaks the header totals. Either the factor postdates that
/// recording or the online mod differs from the local one; until a hardrock
/// replay actually demands it, applying it would corrupt real runs.
pub fn hit_window_ms(replay: &Replay) -> f64 {
    // Clamped at BOTH ends, because the speed is a header field a file can
    // claim anything for: the parser only rejects values that are not > 0,
    // so +inf and absurd finite values reach here. Consumers size buffers
    // from the window (the error histogram in analysis.rs), and an infinite
    // or gigantic one turns a forged header into a capacity-overflow panic
    // or an out-of-memory abort that no Result can carry. 4x covers every
    // speed the game offers with room to spare.
    let speed = f64::from(replay.speed).clamp(0.01, 4.0);
    BASE_HIT_WINDOW_MS * speed
}

/// Half-width of the game's hit area, in world units: the fixed square
/// around a note's centre the cursor must cover. `note_hitbox_size` is
/// 1.140 by default, but the game special-cases exactly that value:
/// `NoteManager.gd:193-194` reads `hbs = note_hitbox_size/2` and then
/// `if hbs == 0.57: hbs = 0.56875`. Attribution and the analyzer's
/// hit-area boxes share this one constant.
pub const HITBOX_HALF: f32 = 0.56875;

/// The game's cursor barrier: the cursor the game TESTS hits with is
/// clamped to the field edge minus the fixed inset (Cursor.gd `edgec`
/// 0.13125 cells). Recordings from absolute devices (tablets) contain
/// raw positions beyond it, but no hit can ever happen out there.
pub const CURSOR_EDGE_INSET: f32 = 0.13125;

/// Grid cell (0..2) to world units, the game's own mapping.
fn note_world(n: &Note) -> (f32, f32) {
    (n.x - 1.0, 1.0 - n.y)
}

/// The cursor bound for a note set: normal grids clamp at ±1.36875,
/// hardrock grids wider (derived from the notes themselves so this
/// crate needs no mods knowledge).
fn cursor_bound(notes: &[Note]) -> f32 {
    let mut grid = 1.0f32;
    for n in notes {
        let (wx, wy) = note_world(n);
        grid = grid.max(wx.abs()).max(wy.abs());
    }
    grid + (0.5 - CURSOR_EDGE_INSET)
}

fn covers(fx: f32, fy: f32, n: &Note, bound: f32) -> bool {
    covers_within(fx, fy, n, bound, 0.0)
}

/// [`covers`] with the hit area grown by `tolerance` on every side.
fn covers_within(fx: f32, fy: f32, n: &Note, bound: f32, tolerance: f32) -> bool {
    let (cx, cy) = (fx.clamp(-bound, bound), fy.clamp(-bound, bound));
    let (wx, wy) = note_world(n);
    let reach = HITBOX_HALF + tolerance;
    (cx - wx).abs() <= reach && (cy - wy).abs() <= reach
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

/// The result of every note for this replay.
///
/// A `.rhr` flags the frames on which a note was hit and leaves the notes
/// to be worked out, which is [`match_hits`]. A converted legacy replay
/// names the note of every hit itself, and here it is taken at its word:
/// working the notes out again from the times gets the totals right but
/// hands about a third of the misses of a real run to a neighbouring note.
/// A hit the notes have no place for (the wrong map, a note named twice)
/// counts as an orphan, exactly like a flag that matches nothing.
///
/// `notes` may be the map as played (mirrored, spread); only the times
/// and the order matter.
pub fn judge(replay: &Replay, notes: &[Note]) -> MatchOutcome {
    let Some(placements) = crate::pairing::legacy_placements(replay, notes) else {
        return judge_flags(replay, notes);
    };
    let mut results: Vec<NoteResult> = (0..notes.len())
        .map(|i| NoteResult {
            note_index: i,
            hit: false,
            hit_ms: None,
        })
        .collect();
    let mut orphan_flags = 0u32;
    for p in placements {
        match results.get_mut(p.note) {
            Some(r) if p.fits && !r.hit => {
                r.hit = true;
                r.hit_ms = Some(p.ms);
            }
            _ => orphan_flags += 1,
        }
    }
    MatchOutcome {
        results,
        orphan_flags,
    }
}

/// Whether this recording flags moments, not hits.
///
/// From file version 20260517 on the game writes one flagged frame for
/// every hit, two when two notes fall together. Before that it flagged the
/// moment: one frame, however many notes the cursor took in it. Measured
/// on 89 leaderboard runs: in the 55 newer ones the flagged frames equal
/// the header's hits every time, and flags less than a millisecond apart
/// are common (up to 248 pairs in a run). In the 36 older ones such pairs
/// are all but absent, and 13 name more hits in the header than they have
/// flags, never by more than the notes a flagged moment's cursor also
/// covered. The versions between the two are not in that set; they are
/// read like the older one, which only ever adds hits the header names.
pub fn marks_moments(replay: &Replay) -> bool {
    matches!(replay.origin, Origin::Rhr) && replay.version < V_BEATMAP_HASH
}

/// How far past the hit window a flag of this recording may be stamped.
///
/// Files before 20260510 store frame times with their fraction, and a
/// frame is stamped a moment after its hit was judged: one real run holds
/// a flag 0.08 ms past its window, on a note the header counts as hit. The
/// newer format stores whole milliseconds, where such a fraction never
/// shows, so it gets no slack.
fn stamp_slack_ms(replay: &Replay) -> f64 {
    if matches!(replay.origin, Origin::Rhr) && replay.version < V_INT32_TIME {
        1.0
    } else {
        0.0
    }
}

/// A `.rhr`: the flags against the notes, and for an older recording the
/// hits its flags stand for beyond the first (see [`marks_moments`]).
fn judge_flags(replay: &Replay, notes: &[Note]) -> MatchOutcome {
    let window_ms = hit_window_ms(replay);
    let slack_ms = stamp_slack_ms(replay);
    let mut outcome = match_hits_inner(notes, &replay.frames, window_ms, slack_ms, true);
    if marks_moments(replay) {
        let named = u32::try_from(replay.hits).unwrap_or(0);
        let missing = named.saturating_sub(outcome.derived_hits());
        if missing > 0 {
            share_flags(&mut outcome, notes, &replay.frames, window_ms, missing);
        }
    }
    outcome
}

/// How far outside a note's hit area a flagged moment's cursor may sit and
/// still have taken the note along. A cursor pressed against the field's
/// edge stands exactly on the border of the boxes that end there (a note at
/// 0.2 or 1.8 of the grid), and the recorded, clamped position comes out a
/// thousandth of a cell past it: twelve hits of one real run sit 0.0007
/// outside.
const SHARED_FLAG_TOLERANCE: f32 = 0.002;

/// Older recordings only: gives the hits the header names beyond the flags
/// to the notes a flagged moment took along.
///
/// A note qualifies when a flagged frame lies inside its window, from the
/// note's own time on, with the cursor on the note: that is the moment the
/// game hit it, together with the note the flag was matched to. No more
/// than `missing` are ever added, so the header's count is the ceiling;
/// fewer are added when the flags cannot account for it, and the totals
/// then stay short of the header for the integrity check to see.
///
/// More notes can qualify than hits are missing: a frame is stamped a
/// moment after its hit was judged, so a flag right at a note's own time
/// may have been judged before the note could be hit. The notes covered
/// latest after their own time are therefore taken first. Which note of
/// such a group was the miss stays an estimate: 35 newer runs rewritten
/// the older way (flags of one moment merged into one) had 1658 hits to
/// hand out, and 122 of them went to a miss next to the note that was hit.
/// The totals came out right in every one. Most files leave no choice at
/// all (as many notes qualify as hits are missing).
fn share_flags(
    outcome: &mut MatchOutcome,
    notes: &[Note],
    frames: &[Frame],
    window_ms: f64,
    missing: u32,
) {
    let bound = cursor_bound(notes);
    let flags: Vec<&Frame> = frames.iter().filter(|f| f.hit).collect();
    // (how long after the note's time, note, the flag's stamp)
    let mut taken_along: Vec<(f64, usize, f64)> = Vec::new();
    for (ni, note) in notes.iter().enumerate() {
        if outcome.results[ni].hit {
            continue;
        }
        let t = note.time_ms as f64;
        let soonest = flags
            .iter()
            .filter(|f| f.ms >= t && f.ms - t <= window_ms)
            .filter(|f| covers_within(f.x, f.y, note, bound, SHARED_FLAG_TOLERANCE))
            .map(|f| f.ms)
            .min_by(f64::total_cmp);
        if let Some(ms) = soonest {
            taken_along.push((ms - t, ni, ms));
        }
    }
    taken_along.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    for &(_, ni, ms) in taken_along.iter().take(missing as usize) {
        outcome.results[ni].hit = true;
        outcome.results[ni].hit_ms = Some(ms);
    }
}

/// Aligns flagged frames to notes with a monotonic two-pointer walk,
/// then refines the attribution with the recorded cursor (see Phase 2
/// below). Notes must be sorted by time (Map guarantees this).
pub fn match_hits(notes: &[Note], frames: &[Frame], window_ms: f64) -> MatchOutcome {
    match_hits_inner(notes, frames, window_ms, 0.0, true)
}

/// Timing-only matching, no cursor-guided reattribution. Use when the
/// note coordinates are NOT yet in the cursor's space: mirror-flip
/// detection runs on the unflipped map, and letting Phase 2 "correct"
/// attributions against geometry the player never saw can invert the
/// detected axis.
pub fn match_hits_timing_only(notes: &[Note], frames: &[Frame], window_ms: f64) -> MatchOutcome {
    match_hits_inner(notes, frames, window_ms, 0.0, false)
}

/// `late_slack_ms` widens the late edge of the window only (see
/// [`stamp_slack_ms`]); a flag never gets to be earlier for it.
fn match_hits_inner(
    notes: &[Note],
    frames: &[Frame],
    window_ms: f64,
    late_slack_ms: f64,
    cursor_guided: bool,
) -> MatchOutcome {
    let late_ms = window_ms + late_slack_ms;
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
    // Which flag frame each hit note owns. Phase 2 swaps move the INDEX,
    // so the cursor lookup can never alias two flags with equal stamps.
    let mut flag_of: Vec<Option<usize>> = vec![None; notes.len()];

    for (ni, note) in notes.iter().enumerate() {
        let note_ms = note.time_ms as f64;
        // A flag more than `window_ms` before this note can never match it
        // or any later note, so orphan it instead of stalling (rhr2mp4 bug).
        while fi < flags.len() && flags[fi] < note_ms - window_ms {
            orphan_flags += 1;
            fi += 1;
        }
        // The loop above left no flag more than the window early.
        if fi < flags.len() && flags[fi] - note_ms <= late_ms {
            results[ni].hit = true;
            results[ni].hit_ms = Some(flags[fi]);
            flag_of[ni] = Some(fi);
            fi += 1;
        }
    }
    // Flags left after the last note matched nothing.
    orphan_flags += (flags.len() - fi) as u32;

    // Phase 2: cursor-guided reattribution. Timing alone cannot tell
    // near-simultaneous notes apart, and the earliest-note rule above
    // sometimes hands a flag to the WRONG one: the analyzer then paints
    // a hit box the cursor never touched and a miss box it sat inside.
    // The flag frame recorded the cursor, so use it: a flag moves from
    // its note to a missed neighbour when the cursor covered the missed
    // note's hit area and NOT the attributed one. Totals never change:
    // each swap trades one hit and one miss between two notes.
    //
    // A note arms at its chart time: a flag may precede its true note
    // only by the replay's ~17 ms frame-stamp quantization (module docs),
    // never by the full window. Otherwise a cursor parked on a
    // soon-future note's cell steals flags and manufactures impossible
    // early hits (negative timing errors).
    const EARLY_SLACK_MS: f64 = 17.0;
    let bound = cursor_bound(notes);
    // A mis-shifted CHAIN (every flag one note early) unravels one link
    // per pass, from the tail backwards, so run to convergence. Each
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
                if fm - miss_t > late_ms || miss_t - fm > EARLY_SLACK_MS {
                    continue;
                }
                // Only when the cursor is unambiguous: inside the missed
                // note's area, outside the attributed one's.
                if covers(fx, fy, miss_note, bound) && !covers(fx, fy, &notes[hi], bound) {
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

    // Phase 3: hit<->hit untangling. Two hit notes can have swapped
    // flags when the player touched them in the OPPOSITE of chart order
    // (routine on near-simultaneous doubles): the order-preserving walk
    // then pins each flag to the wrong note and every verdict dot lands
    // outside its box. Swap a pair when the crossed assignment matches
    // the cursor strictly better; totals cannot change (both stay hits).
    if cursor_guided {
        for _pass in 0..4 {
            let mut changed = false;
            for i in 0..results.len() {
                if !results[i].hit {
                    continue;
                }
                for j in (i + 1)..results.len() {
                    // Re-read every iteration: a swap below moves note i
                    // onto a different flag (a stale binding would hand
                    // the SAME flag to two notes on 3-note clusters).
                    let Some(fi) = flag_of[i] else { break };
                    let dt = notes[j].time_ms - notes[i].time_ms;
                    if dt as f64 > window_ms {
                        break;
                    }
                    if !results[j].hit {
                        continue;
                    }
                    let Some(fj) = flag_of[j] else { continue };
                    let (fmi, xi, yi) = flag_frames[fi];
                    let (fmj, xj, yj) = flag_frames[fj];
                    // Both flags must be plausible for the OTHER note.
                    let ti = notes[i].time_ms as f64;
                    let tj = notes[j].time_ms as f64;
                    if fmj - ti > late_ms
                        || ti - fmj > EARLY_SLACK_MS
                        || fmi - tj > late_ms
                        || tj - fmi > EARLY_SLACK_MS
                    {
                        continue;
                    }
                    let straight = usize::from(covers(xi, yi, &notes[i], bound))
                        + usize::from(covers(xj, yj, &notes[j], bound));
                    let crossed = usize::from(covers(xj, yj, &notes[i], bound))
                        + usize::from(covers(xi, yi, &notes[j], bound));
                    if crossed > straight {
                        results[i].hit_ms = Some(fmj);
                        results[j].hit_ms = Some(fmi);
                        flag_of[i] = Some(fj);
                        flag_of[j] = Some(fi);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
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

    fn replay_with(mods: &str, speed: f32) -> Replay {
        Replay {
            version: 5,
            timestamp_ticks: 0,
            player_name: "t".into(),
            legacy_map_id: String::new(),
            map_id: 0,
            start_from_ms: 0,
            mode: String::new(),
            passed: true,
            mods: mods.to_string(),
            spin: false,
            speed,
            total_score: 0,
            accuracy_pct: 100.0,
            hits: 0,
            misses: 0,
            points: 0.0,
            fail_time_ms: -1,
            beatmap_hash: String::new(),
            pitch_lock: false,
            frames: Vec::new(),
            trailing_bytes: 0,
            origin: rhythia_formats::rhr::Origin::Rhr,
        }
    }

    #[test]
    fn hit_window_follows_speed_and_hardrock() {
        let w = |mods: &str, speed: f32| hit_window_ms(&replay_with(mods, speed));
        // The game's own numbers: 55 ms, scaled by the speed multiplier.
        assert!((w("[]", 1.0) - 55.0).abs() < 1e-9);
        assert!((w("[]", 1.45) - 79.75).abs() < 1e-4); // 1.45f32 is not exact
        assert!((w("[]", 2.0) - 110.0).abs() < 1e-9);
        // Mods do not touch it, including hardrock, whose 0.8 factor the
        // one real hardrock replay contradicts (see hit_window_ms docs).
        assert!((w("[\"mod_hardrock\"]", 1.0) - 55.0).abs() < 1e-9);
        assert!((w("[\"mod_hardrock\"]", 1.45) - 79.75).abs() < 1e-4);
        assert!((w("[\"mod_mirror\",\"mod_nofail\"]", 1.0) - 55.0).abs() < 1e-9);
        // A malformed mods field must not widen or zero the window.
        assert!((w("not json", 1.0) - 55.0).abs() < 1e-9);
        // A header can claim any speed; the window must stay finite and
        // positive so callers can size buffers from it.
        assert!(w("[]", 0.0) > 0.0);
        assert!((w("[]", f32::INFINITY) - 220.0).abs() < 1e-9);
        assert!((w("[]", 1.0e9) - 220.0).abs() < 1e-9);
        assert!(w("[]", -5.0) > 0.0 && w("[]", -5.0).is_finite());
    }

    /// These cases exercise attribution mechanics, not the window value:
    /// 80 ms is the window of a 1.45x run, which is what they were built
    /// and validated against.
    const TEST_WINDOW_MS: f64 = 80.0;

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
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
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
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
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
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
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
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
        assert_eq!(out.orphan_flags, 1);
        assert!(out.results[0].hit);
        assert!(out.results[1].hit);
    }

    #[test]
    fn double_flag_for_one_note_leaves_an_orphan() {
        let notes = [note(1000)];
        let frames = [flag(995.0), flag(1005.0)];
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
        assert_eq!(out.derived_hits(), 1);
        assert_eq!(out.orphan_flags, 1);
    }

    #[test]
    fn trailing_orphan_after_last_note() {
        let notes = [note(1000)];
        let frames = [flag(1005.0), flag(9999.0)];
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
        assert_eq!(out.derived_hits(), 1);
        assert_eq!(out.orphan_flags, 1);
    }

    /// The pass_long real-replay case: two near-simultaneous notes, the
    /// flag lands on the earlier one by time but the cursor sat on the
    /// later one: attribution must follow the cursor.
    #[test]
    fn cursor_reattributes_swapped_double_note() {
        let notes = [
            Note {
                time_ms: 1000,
                x: 0.0,
                y: 2.0,
            }, // world (-1, -1)
            Note {
                time_ms: 1005,
                x: 1.0,
                y: 2.0,
            }, // world (0, -1)
        ];
        let frames = [Frame {
            ms: 1004.0,
            x: 0.0,
            y: -1.0,
            health: 1.0,
            hit: true,
        }];
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
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
            Note {
                time_ms: 1000,
                x: 0.0,
                y: 2.0,
            }, // world (-1, -1)
            Note {
                time_ms: 1005,
                x: 1.0,
                y: 2.0,
            }, // world (0, -1)
        ];
        // Cursor midway: covers both areas.
        let frames = [Frame {
            ms: 1004.0,
            x: -0.5,
            y: -1.0,
            health: 1.0,
            hit: true,
        }];
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
        assert!(out.results[0].hit);
        assert!(!out.results[1].hit);
    }

    /// A flag must never be stolen by a note far in the FUTURE: the
    /// hitbox arms at the note's chart time (early slack = one frame).
    #[test]
    fn future_note_cannot_steal_a_flag() {
        let notes = [
            Note {
                time_ms: 1000,
                x: 0.0,
                y: 0.0,
            }, // world (-1, 1)
            Note {
                time_ms: 1075,
                x: 2.0,
                y: 0.0,
            }, // world (1, 1)
        ];
        // Flag at 1000 for note 0, but the cursor already left toward
        // note 1's cell (fast jump + frame quantization).
        let frames = [Frame {
            ms: 1000.0,
            x: 1.0,
            y: 1.0,
            health: 1.0,
            hit: true,
        }];
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
        assert!(out.results[0].hit, "note 0 keeps its flag");
        assert!(
            !out.results[1].hit,
            "a note 75 ms in the future must not steal it"
        );
    }

    /// Three simultaneous notes taken in the order B, C, A: the crossed
    /// flags must resolve to a full rotation without any duplicate flag
    /// attribution (regression: a stale flag binding handed one flag to
    /// two notes).
    #[test]
    fn three_note_cluster_rotates_without_duplicates() {
        let notes = [
            Note {
                time_ms: 1000,
                x: 0.0,
                y: 1.0,
            }, // world (-1, 0)
            Note {
                time_ms: 1000,
                x: 1.0,
                y: 1.0,
            }, // world (0, 0)
            Note {
                time_ms: 1000,
                x: 2.0,
                y: 1.0,
            }, // world (1, 0)
        ];
        let frames = [
            Frame {
                ms: 1000.0,
                x: 0.0,
                y: 0.0,
                health: 1.0,
                hit: true,
            }, // on note 1
            Frame {
                ms: 1005.0,
                x: 1.0,
                y: 0.0,
                health: 1.0,
                hit: true,
            }, // on note 2
            Frame {
                ms: 1010.0,
                x: -1.0,
                y: 0.0,
                health: 1.0,
                hit: true,
            }, // on note 0
        ];
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
        let ms: Vec<f64> = out.results.iter().map(|r| r.hit_ms.unwrap()).collect();
        assert_eq!(ms, vec![1010.0, 1000.0, 1005.0]);
        // No two notes may share a flag.
        let mut sorted = ms.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        sorted.dedup();
        assert_eq!(sorted.len(), 3, "duplicate flag attribution");
    }

    /// Totals are invariant under reattribution.
    #[test]
    fn reattribution_never_changes_totals() {
        let notes = [
            Note {
                time_ms: 1000,
                x: 0.0,
                y: 0.0,
            },
            Note {
                time_ms: 1010,
                x: 2.0,
                y: 0.0,
            },
            Note {
                time_ms: 1020,
                x: 1.0,
                y: 1.0,
            },
        ];
        let frames = [
            Frame {
                ms: 1008.0,
                x: 1.0,
                y: 1.0,
                health: 1.0,
                hit: true,
            }, // on note 1
            Frame {
                ms: 1022.0,
                x: 0.0,
                y: 0.0,
                health: 1.0,
                hit: true,
            }, // on note 2
        ];
        let out = match_hits(&notes, &frames, TEST_WINDOW_MS);
        assert_eq!(out.derived_hits(), 2);
        assert_eq!(out.orphan_flags, 0);
    }

    // ---- older recordings: a flag stands for the moment

    fn note_at(time_ms: i64, x: f32, y: f32) -> Note {
        Note { time_ms, x, y }
    }

    /// A frame with the cursor at a world position (0, 0 is the centre).
    fn frame_at(ms: f64, x: f32, y: f32, hit: bool) -> Frame {
        Frame {
            ms,
            x,
            y,
            health: 1.0,
            hit,
        }
    }

    /// A run of the given file version whose header names `hits` hits.
    fn run(version: i32, hits: i32, frames: Vec<Frame>) -> Replay {
        let mut r = replay_with("[]", 1.0);
        r.version = version;
        r.hits = hits;
        r.frames = frames;
        r
    }

    const OLDER: i32 = rhythia_formats::rhr::V_FAIL_TIME;
    const NEWER: i32 = V_BEATMAP_HASH;

    fn hits_of(outcome: &MatchOutcome) -> Vec<bool> {
        outcome.results.iter().map(|r| r.hit).collect()
    }

    #[test]
    fn an_older_recording_flags_the_moment_and_every_note_under_the_cursor_is_hit() {
        // Two notes 5 ms apart in neighbouring cells, one flagged frame
        // with the cursor between them.
        let notes = [note_at(1000, 1.0, 1.0), note_at(1005, 2.0, 1.0)];
        let frames = vec![frame_at(1010.0, 0.5, 0.0, true)];
        let older = judge(&run(OLDER, 2, frames.clone()), &notes);
        assert_eq!(hits_of(&older), [true, true]);
        assert_eq!(older.results[1].hit_ms, Some(1010.0));
        assert_eq!(older.orphan_flags, 0);
        // The newer recorder writes a frame per hit: one flag is one hit.
        let newer = judge(&run(NEWER, 2, frames), &notes);
        assert_eq!(hits_of(&newer), [true, false]);
    }

    #[test]
    fn a_note_the_cursor_was_not_on_is_not_taken_along() {
        let notes = [note_at(1000, 1.0, 1.0), note_at(1005, 2.0, 1.0)];
        // The cursor sits on the first note's centre, a whole cell from
        // the second.
        let outcome = judge(
            &run(OLDER, 2, vec![frame_at(1010.0, 0.0, 0.0, true)]),
            &notes,
        );
        assert_eq!(hits_of(&outcome), [true, false]);
        assert_eq!(
            outcome.derived_hits(),
            1,
            "the header's second hit stays unaccounted for"
        );
    }

    #[test]
    fn a_flag_before_a_notes_time_takes_nothing_along() {
        // Covered, but ten milliseconds before the note can be hit.
        let notes = [note_at(1000, 1.0, 1.0), note_at(1020, 2.0, 1.0)];
        let outcome = judge(
            &run(OLDER, 2, vec![frame_at(1010.0, 0.5, 0.0, true)]),
            &notes,
        );
        assert_eq!(hits_of(&outcome), [true, false]);
    }

    #[test]
    fn no_more_notes_are_taken_along_than_the_header_names() {
        // Three notes in one cell under one flag, and a header of two hits:
        // the flag's own note, and of the other two the one covered latest
        // after its own time (5 ms against 2).
        let notes = [
            note_at(1000, 1.0, 1.0),
            note_at(1005, 1.0, 1.0),
            note_at(1008, 1.0, 1.0),
        ];
        let frames = vec![frame_at(1010.0, 0.0, 0.0, true)];
        let two = judge(&run(OLDER, 2, frames.clone()), &notes);
        assert_eq!(hits_of(&two), [true, true, false]);
        let three = judge(&run(OLDER, 3, frames.clone()), &notes);
        assert_eq!(hits_of(&three), [true, true, true]);
        // A header that names no more hits than there are flags changes
        // nothing.
        let one = judge(&run(OLDER, 1, frames), &notes);
        assert_eq!(hits_of(&one), [true, false, false]);
    }

    #[test]
    fn a_cursor_pressed_to_the_fields_edge_takes_along_the_note_whose_box_ends_there() {
        // The far note sits a hair outside the grid, which moves the edge
        // the cursor is clamped to by as much; the recorded cursor lies
        // beyond it (a tablet), and the second note's box ends exactly at
        // the edge.
        let notes = [
            note_at(1000, 2.0, 1.0),
            note_at(1004, 1.8, 1.0),
            note_at(9000, 2.0007, 1.0),
        ];
        let outcome = judge(
            &run(OLDER, 2, vec![frame_at(1010.0, 1.5, 0.0, true)]),
            &notes,
        );
        assert_eq!(hits_of(&outcome), [true, true, false]);
    }

    #[test]
    fn a_flag_a_fraction_past_the_window_counts_where_times_carry_fractions() {
        // The window of a 1.00x run is 55 ms.
        let notes = [note_at(1000, 1.0, 1.0)];
        let late =
            |version, ms| judge(&run(version, 1, vec![frame_at(ms, 0.0, 0.0, true)]), &notes);
        let fractional = late(OLDER, 1055.08);
        assert_eq!((fractional.derived_hits(), fractional.orphan_flags), (1, 0));
        // More than the fraction of a millisecond is past the window.
        let too_late = late(OLDER, 1056.5);
        assert_eq!((too_late.derived_hits(), too_late.orphan_flags), (0, 1));
        // Whole-millisecond files get no such slack.
        let whole = late(V_INT32_TIME, 1055.08);
        assert_eq!((whole.derived_hits(), whole.orphan_flags), (0, 1));
    }

    #[test]
    fn only_a_rhr_before_the_per_hit_recorder_flags_moments() {
        assert!(marks_moments(&run(OLDER, 0, vec![])));
        assert!(marks_moments(&run(V_INT32_TIME, 0, vec![])));
        assert!(!marks_moments(&run(NEWER, 0, vec![])));
        assert!(!marks_moments(&run(
            rhythia_formats::rhr::V_PITCH_LOCK,
            0,
            vec![]
        )));
        let mut converted = run(OLDER, 0, vec![]);
        converted.origin = Origin::Sspre {
            skips: vec![],
            judged_end: None,
            hits: vec![],
        };
        assert!(!marks_moments(&converted));
    }
}
