//! Ghost-race analytics: the running score gap between the two runs,
//! sampled across the map for the racing-delta results graph; the live
//! widget itself reads `stats_at` directly so its numbers always equal
//! the per-side HUDs.
//!
//! All sampling goes through [`crate::hud::HudState::stats_at`] — the same
//! walk that drives the on-screen score/accuracy — so the series can never
//! disagree with what the viewer sees. A failed run is frozen at its fail
//! time (fail + hit window, the results screen's convention): the game
//! never resolved the later notes, so neither do we.

use rhythia_formats::map::Map;
use rhythia_formats::rhr::Replay;
use rhythia_sim::hitreg::DEFAULT_WINDOW_MS;

use crate::hud::HudState;

/// One run's inputs, as they exist wherever both sides are in scope.
pub struct RaceSide<'a> {
    pub map: &'a Map,
    pub replay: &'a Replay,
    pub state: &'a HudState,
}

/// The race at one sampled song time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RaceSample {
    pub t_ms: f64,
    /// Main minus ghost, in score points.
    pub score_delta: i64,
    /// Accuracy percent per side: `[main, ghost]`.
    pub acc: [f32; 2],
}

/// Whole-map race series, built once per render.
#[derive(Debug, Clone, Default)]
pub struct RaceSeries {
    pub samples: Vec<RaceSample>,
    /// Sample times where the score lead flipped to the other side. Zero is
    /// neutral: `+,0,+` is no change, `+,0,-` is one.
    pub lead_changes: Vec<f64>,
}

/// The race as the score-lead widget reads it: note-synchronized, so a
/// lead only exists once a note is answered by BOTH sides and came out
/// differently. Per-side scores bank a hit the instant it lands, which
/// made the raw gap flicker side-to-side on every note while the race
/// was even (one player is always a few ms earlier).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncedRace {
    /// Main minus ghost over the settled prefix.
    pub delta: i64,
    /// Cumulative score per side over the settled prefix.
    pub scores: [i64; 2],
    /// Notes settled by both sides.
    pub settled: u32,
}

/// Walks both runs note by note and stops at the last note whose outcome
/// is known on BOTH sides at `t_ms` (a hit is known when it lands, a miss
/// at the end of the hit window, and a side past its fail freeze answers
/// instantly with nothing).
pub fn synced_race(main: &RaceSide, ghost: &RaceSide, t_ms: f64) -> SyncedRace {
    let sides = [main, ghost];
    let ends = [side_end(main.replay), side_end(ghost.replay)];
    let results = [main.state.results(), ghost.state.results()];
    let n = results[0]
        .len()
        .min(results[1].len())
        .min(main.map.notes.len())
        .min(ghost.map.notes.len());

    let mut out = SyncedRace {
        delta: 0,
        scores: [0, 0],
        settled: 0,
    };
    let mut combo = [0i64; 2];
    for i in 0..n {
        // When is this note's outcome known on each side?
        let mut known = 0.0f64;
        for s in 0..2 {
            let reg = sides[s].map.notes[i].time_ms as f64 + DEFAULT_WINDOW_MS;
            let k = if reg >= ends[s] {
                // Frozen side: the game never resolved this note — the
                // answer ("nothing") is known from the freeze on.
                ends[s]
            } else {
                results[s][i].hit_ms.unwrap_or(reg)
            };
            known = known.max(k);
        }
        // Hit times and windows are monotone over the note order, so the
        // first pending note ends the settled prefix.
        if known > t_ms {
            break;
        }
        out.settled += 1;
        for s in 0..2 {
            let reg = sides[s].map.notes[i].time_ms as f64 + DEFAULT_WINDOW_MS;
            if reg >= ends[s] {
                continue;
            }
            if results[s][i].hit {
                combo[s] += 1;
                out.scores[s] += combo[s] * 100;
            } else {
                combo[s] = 0;
            }
        }
    }
    out.delta = out.scores[0] - out.scores[1];
    out
}

/// Tournament-style lead bar: signed fill in -1..1 (positive = main/left
/// leads), from the score gap RELATIVE to the leader's score so half a
/// bar means the same thing early and late in a song (raw score grows
/// quadratically). Full at a 50% lead; empty during the first notes,
/// where tiny scores whipsaw the ratio.
pub fn race_bar_value(delta: i64, leader_score: i64, resolved: u32) -> f32 {
    if resolved < 20 || leader_score <= 0 {
        return 0.0;
    }
    (delta as f32 / leader_score as f32 / 0.5).clamp(-1.0, 1.0)
}

/// Vertical share of the results delta graph that sits above the zero
/// line, from the two lead extremes. Clamped so the smaller side always
/// keeps a visible strip even in a one-sided race.
pub fn graph_zero_share(pos_max: i64, neg_max: i64) -> f32 {
    let total = pos_max + neg_max;
    if total <= 0 {
        return 0.5;
    }
    (pos_max as f32 / total as f32).clamp(0.12, 0.88)
}

/// Song time past which a run's stats stop moving. Matches the results
/// screen: a failed run is read at fail time + hit window.
pub fn side_end(replay: &Replay) -> f64 {
    if replay.failed() {
        replay.fail_time_ms as f64 + DEFAULT_WINDOW_MS + 1.0
    } else {
        f64::INFINITY
    }
}

impl RaceSeries {
    /// The standard whole-map series: 240 samples from song start to just
    /// past the last note (scores cannot move later).
    pub fn for_race(main: &RaceSide, ghost: &RaceSide) -> RaceSeries {
        let end = main
            .map
            .notes
            .last()
            .map(|n| n.time_ms as f64 + 1000.0)
            .unwrap_or(1000.0);
        RaceSeries::build(main, ghost, 0.0, end, 240)
    }

    /// Samples both runs at `samples` evenly spaced times over
    /// `start_ms..=end_ms` (at least 2).
    pub fn build(
        main: &RaceSide,
        ghost: &RaceSide,
        start_ms: f64,
        end_ms: f64,
        samples: usize,
    ) -> RaceSeries {
        let n = samples.max(2);
        let ends = [side_end(main.replay), side_end(ghost.replay)];
        let mut out = RaceSeries::default();

        for i in 0..n {
            let t = start_ms + (end_ms - start_ms) * i as f64 / (n - 1) as f64;
            let m = main.state.stats_at(main.map, main.replay, t.min(ends[0]));
            let g = ghost.state.stats_at(ghost.map, ghost.replay, t.min(ends[1]));
            out.samples.push(RaceSample {
                t_ms: t,
                score_delta: m.score - g.score,
                acc: [m.accuracy_pct, g.accuracy_pct],
            });
        }

        let mut last = 0i8;
        for p in &out.samples {
            let s = p.score_delta.signum() as i8;
            if s != 0 {
                if last != 0 && s != last {
                    out.lead_changes.push(p.t_ms);
                }
                last = s;
            }
        }
        out
    }
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

    fn replay_with(hit_times: &[f64]) -> Replay {
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
            speed: 1.0,
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

    /// Four notes at 1s..4s; five samples at 0/1250/2500/3750/5000 ms.
    const NOTES: [i64; 4] = [1000, 2000, 3000, 4000];

    fn series_for(main_hits: &[f64], ghost_hits: &[f64]) -> (RaceSeries, Replay, Replay) {
        let map = map_with(&NOTES);
        let (mr, gr) = (replay_with(main_hits), replay_with(ghost_hits));
        let (ms, gs) = (HudState::new(&map, &mr), HudState::new(&map, &gr));
        let s = RaceSeries::build(
            &RaceSide { map: &map, replay: &mr, state: &ms },
            &RaceSide { map: &map, replay: &gr, state: &gs },
            0.0,
            5000.0,
            5,
        );
        (s, mr, gr)
    }

    #[test]
    fn score_delta_matches_both_huds_at_every_sample() {
        // Main hits all four (100/300/600/1000); ghost misses note 2, so its
        // combo restarts: 100 / 100 / 200 / 400.
        let (s, _, _) = series_for(
            &[1005.0, 2005.0, 3005.0, 4005.0],
            &[1005.0, 3005.0, 4005.0],
        );
        let deltas: Vec<i64> = s.samples.iter().map(|p| p.score_delta).collect();
        assert_eq!(deltas, vec![0, 0, 200, 400, 600]);
        let last = s.samples.last().unwrap();
        assert_eq!(last.t_ms, 5000.0);
        assert!((last.acc[0] - 100.0).abs() < 1e-4);
        assert!((last.acc[1] - 75.0).abs() < 1e-4);
    }

    #[test]
    fn lead_changes_flag_each_sign_flip_and_zero_is_neutral() {
        // Main misses note 1 (0/100/300/600); ghost hits note 1 and misses
        // 2+3 (100/100/100/200). Deltas: 0, -100, 0, +200, +400 — exactly
        // one lead change, at the sample where the sign turns positive.
        let (s, _, _) = series_for(
            &[2005.0, 3005.0, 4005.0],
            &[1005.0, 4005.0],
        );
        let deltas: Vec<i64> = s.samples.iter().map(|p| p.score_delta).collect();
        assert_eq!(deltas, vec![0, -100, 0, 200, 400]);
        assert_eq!(s.lead_changes, vec![3750.0]);
    }

    #[test]
    fn failed_side_freezes_at_its_fail_time() {
        // Ghost hits notes 1+2 and fails at 2500: notes 3+4 were never
        // played, so its stats freeze (300 points, 100% of what it saw).
        let map = map_with(&NOTES);
        let mr = replay_with(&[1005.0, 2005.0, 3005.0, 4005.0]);
        let mut gr = replay_with(&[1005.0, 2005.0]);
        gr.fail_time_ms = 2500;
        gr.passed = false;
        let (ms, gs) = (HudState::new(&map, &mr), HudState::new(&map, &gr));
        let s = RaceSeries::build(
            &RaceSide { map: &map, replay: &mr, state: &ms },
            &RaceSide { map: &map, replay: &gr, state: &gs },
            0.0,
            5000.0,
            5,
        );
        let last = s.samples.last().unwrap();
        assert_eq!(last.score_delta, 1000 - 300);
        assert!((last.acc[1] - 100.0).abs() < 1e-4);
    }

    #[test]
    fn synced_delta_stays_zero_while_both_answer_notes_identically() {
        // A hits note 1 at 1005, B the same note at 1030: the raw scores
        // differ between those instants, but the synced walk waits for
        // both answers — an even race never shows a lead.
        let map = map_with(&[1000, 2000]);
        let (mr, gr) = (replay_with(&[1005.0, 2005.0]), replay_with(&[1030.0, 2030.0]));
        let (ms, gs) = (HudState::new(&map, &mr), HudState::new(&map, &gr));
        let m = RaceSide { map: &map, replay: &mr, state: &ms };
        let g = RaceSide { map: &map, replay: &gr, state: &gs };
        assert_eq!(synced_race(&m, &g, 1010.0).delta, 0);
        assert_eq!(synced_race(&m, &g, 1010.0).settled, 0);
        let at_1035 = synced_race(&m, &g, 1035.0);
        assert_eq!((at_1035.delta, at_1035.settled), (0, 1));
        assert_eq!(at_1035.scores, [100, 100]);
        // A already banked note 2, B's answer is still pending: no lead.
        assert_eq!(synced_race(&m, &g, 2010.0).settled, 1);
        assert_eq!(synced_race(&m, &g, 2010.0).delta, 0);
    }

    #[test]
    fn synced_delta_moves_only_when_a_note_resolves_differently() {
        // B never answers note 2, so its outcome is known at the window
        // end (2080) — only then does the lead appear.
        let map = map_with(&[1000, 2000]);
        let (mr, gr) = (replay_with(&[1005.0, 2005.0]), replay_with(&[1030.0]));
        let (ms, gs) = (HudState::new(&map, &mr), HudState::new(&map, &gr));
        let m = RaceSide { map: &map, replay: &mr, state: &ms };
        let g = RaceSide { map: &map, replay: &gr, state: &gs };
        assert_eq!(synced_race(&m, &g, 2050.0).delta, 0);
        let done = synced_race(&m, &g, 2085.0);
        assert_eq!((done.delta, done.settled), (200, 2));
        assert_eq!(done.scores, [300, 100]);
    }

    #[test]
    fn synced_delta_keeps_growing_after_a_side_fails() {
        // B fails at 1500 after note 1: its later notes answer instantly
        // with nothing, so A's continuing run keeps widening the gap.
        let map = map_with(&[1000, 2000, 3000]);
        let mr = replay_with(&[1005.0, 2005.0, 3005.0]);
        let mut gr = replay_with(&[1030.0]);
        gr.fail_time_ms = 1500;
        gr.passed = false;
        let (ms, gs) = (HudState::new(&map, &mr), HudState::new(&map, &gr));
        let m = RaceSide { map: &map, replay: &mr, state: &ms };
        let g = RaceSide { map: &map, replay: &gr, state: &gs };
        let late = synced_race(&m, &g, 3500.0);
        assert_eq!((late.delta, late.settled), (500, 3));
        assert_eq!(late.scores, [600, 100]);
    }

    #[test]
    fn lead_bar_scales_with_the_relative_lead_and_warms_up() {
        // Empty until 20 notes are resolved; then the gap relative to the
        // leader's score, full bar at a 50% lead, sign = who leads.
        assert_eq!(race_bar_value(5_000, 10_000, 10), 0.0);
        assert_eq!(race_bar_value(0, 100_000, 200), 0.0);
        assert!((race_bar_value(12_000, 100_000, 200) - 0.24).abs() < 1e-6);
        assert_eq!(race_bar_value(-60_000, 100_000, 200), -1.0);
        assert_eq!(race_bar_value(100, 0, 200), 0.0);
    }

    #[test]
    fn graph_zero_line_follows_the_lead_extremes_within_bounds() {
        // Symmetric race: zero line centred. One-sided race: the unused
        // half shrinks but never below 12% of the band.
        assert_eq!(graph_zero_share(500, 500), 0.5);
        assert_eq!(graph_zero_share(300, 100), 0.75);
        assert_eq!(graph_zero_share(1000, 0), 0.88);
        assert_eq!(graph_zero_share(0, 1000), 0.12);
        assert_eq!(graph_zero_share(0, 0), 0.5);
    }

    #[test]
    fn whole_map_series_spans_start_to_just_past_the_last_note() {
        let map = map_with(&NOTES);
        let (mr, gr) = (
            replay_with(&[1005.0, 2005.0, 3005.0, 4005.0]),
            replay_with(&[1005.0]),
        );
        let (ms, gs) = (HudState::new(&map, &mr), HudState::new(&map, &gr));
        let s = RaceSeries::for_race(
            &RaceSide { map: &map, replay: &mr, state: &ms },
            &RaceSide { map: &map, replay: &gr, state: &gs },
        );
        assert_eq!(s.samples.len(), 240);
        assert_eq!(s.samples[0].t_ms, 0.0);
        assert_eq!(s.samples.last().unwrap().t_ms, 5000.0);
    }

}
