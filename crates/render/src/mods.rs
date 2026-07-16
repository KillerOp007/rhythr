//! Geometry mods recorded in a replay, applied to the note grid so the
//! render shows the field the player actually saw. Speed mods are baked
//! into the replay's frame times already; nofail/sudden-death and friends
//! change no geometry. Chaos randomises positions with a seed the replay
//! does not store, so it cannot be reconstructed and is left as-is.

use rhythia_formats::map::Map;
use rhythia_formats::rhr::Replay;
use rhythia_sim::hitreg::{match_hits, DEFAULT_WINDOW_MS};

/// Hardrock scales the note grid outward around its centre. Empirical, from
/// the hardrock testdata replay: the game's cursor clamp grows from
/// ±1.36875 to ±1.51875 — +0.15000 exactly, i.e. grid half-extent 1.0 →
/// 1.15 with the cursor margin unchanged — and cursor samples on edge
/// notes sit ~1.18× out versus 0.83–1.01× on unmodded baselines.
pub const HARDROCK_GRID_SCALE: f32 = 1.15;

/// What [`map_for_replay`] applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedMods {
    /// Grid half-extent this run played on (1.0, or 1.15 under hardrock).
    /// The playfield border widens with it.
    pub grid_scale: f32,
    /// Mirror flips applied to the notes (x, y).
    pub flip: (bool, bool),
}

impl ResolvedMods {
    pub fn none() -> ResolvedMods {
        ResolvedMods {
            grid_scale: 1.0,
            flip: (false, false),
        }
    }
}

fn mod_list(replay: &Replay) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(&replay.mods).unwrap_or_default()
}

/// Cursor position at `ms`, linearly interpolated (duplicate of the
/// renderer's private helper; both are trivial).
fn cursor_at(replay: &Replay, ms: f64) -> (f32, f32) {
    let f = &replay.frames;
    if f.is_empty() {
        return (0.0, 0.0);
    }
    match f.binary_search_by(|x| x.ms.total_cmp(&ms)) {
        Ok(i) => (f[i].x, f[i].y),
        Err(0) => (f[0].x, f[0].y),
        Err(i) if i >= f.len() => (f[f.len() - 1].x, f[f.len() - 1].y),
        Err(i) => {
            let (a, b) = (f[i - 1], f[i]);
            let t = ((ms - a.ms) / (b.ms - a.ms).max(1e-9)) as f32;
            (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
        }
    }
}

/// The mirror axis is a game-side setting not stored in the replay, so it
/// is recovered from the run itself: the cursor followed the *mirrored*
/// notes, so the flip whose note positions best match the cursor at the
/// hit moments is the one that was active (clear margin in practice: the
/// right flip scores a few tenths of a cell, wrong ones 1+).
fn detect_flip(map: &Map, replay: &Replay, fallback: (bool, bool)) -> (bool, bool) {
    let outcome = match_hits(&map.notes, &replay.frames, DEFAULT_WINDOW_MS);
    let mut best = fallback;
    let mut best_dist = f64::MAX;
    for flip_x in [false, true] {
        for flip_y in [false, true] {
            let (mut total, mut n) = (0f64, 0u32);
            for (note, res) in map.notes.iter().zip(&outcome.results) {
                let Some(hm) = res.hit_ms else { continue };
                let gx = if flip_x { 2.0 - note.x } else { note.x };
                let gy = if flip_y { 2.0 - note.y } else { note.y };
                let (cx, cy) = cursor_at(replay, hm);
                let (dx, dy) = (cx - (gx - 1.0), cy - (1.0 - gy));
                total += ((dx * dx + dy * dy) as f64).sqrt();
                n += 1;
            }
            if n > 0 && total / n as f64 <= best_dist {
                best_dist = total / n as f64;
                best = (flip_x, flip_y);
            }
        }
    }
    best
}

/// Returns the map as this replay's player saw it — mirror flips and the
/// hardrock spread applied to every note — plus what was applied. Sides of
/// a ghost race resolve this independently, so a hardrock run races a
/// normal one with each side's own field.
pub fn map_for_replay(map: &Map, replay: &Replay) -> (Map, ResolvedMods) {
    let mods = mod_list(replay);
    let has = |m: &str| mods.iter().any(|x| x == m);

    let mut resolved = ResolvedMods::none();
    if has("mod_mirror") || has("mod_double_mirror") {
        let fallback = if has("mod_double_mirror") {
            (true, true)
        } else {
            (true, false)
        };
        resolved.flip = detect_flip(map, replay, fallback);
    }
    if has("mod_hardrock") {
        resolved.grid_scale = HARDROCK_GRID_SCALE;
    }

    if resolved == ResolvedMods::none() {
        return (map.clone(), resolved);
    }
    let mut out = map.clone();
    for n in &mut out.notes {
        if resolved.flip.0 {
            n.x = 2.0 - n.x;
        }
        if resolved.flip.1 {
            n.y = 2.0 - n.y;
        }
        n.x = 1.0 + (n.x - 1.0) * resolved.grid_scale;
        n.y = 1.0 + (n.y - 1.0) * resolved.grid_scale;
    }
    (out, resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhythia_formats::map::Note;
    use rhythia_formats::rhr::Frame;

    fn note(t: i64, x: f32, y: f32) -> Note {
        Note {
            time_ms: t,
            x,
            y,
        }
    }

    fn replay_with(mods: &str, frames: Vec<Frame>) -> Replay {
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
            speed: 1.0,
            total_score: 0,
            accuracy_pct: 100.0,
            hits: frames.iter().filter(|f| f.hit).count() as i32,
            misses: 0,
            points: 0.0,
            fail_time_ms: -1,
            beatmap_hash: String::new(),
            frames,
            trailing_bytes: 0,
        }
    }

    #[test]
    fn no_mods_is_identity() {
        let map = Map {
            notes: vec![note(100, 0.0, 2.0)],
            ..Default::default()
        };
        let (m, r) = map_for_replay(&map, &replay_with("[]", vec![]));
        assert_eq!(r, ResolvedMods::none());
        assert_eq!(m.notes[0], note(100, 0.0, 2.0));
    }

    #[test]
    fn hardrock_scales_around_centre() {
        let map = Map {
            notes: vec![note(100, 0.0, 1.0), note(200, 1.0, 1.0)],
            ..Default::default()
        };
        let (m, r) = map_for_replay(&map, &replay_with("[\"mod_hardrock\"]", vec![]));
        assert_eq!(r.grid_scale, HARDROCK_GRID_SCALE);
        assert!((m.notes[0].x - (1.0 - HARDROCK_GRID_SCALE)).abs() < 1e-6);
        assert_eq!(m.notes[1], note(200, 1.0, 1.0)); // centre stays put
    }

    #[test]
    fn mirror_axis_recovered_from_cursor() {
        // Notes on the left column; the cursor sat on the RIGHT at the hit
        // moments, so an x flip must be detected.
        let map = Map {
            notes: vec![note(100, 0.0, 1.0), note(300, 0.0, 0.0)],
            ..Default::default()
        };
        let frames = vec![
            Frame { ms: 90.0, x: 1.0, y: 0.0, health: 1.0, hit: false },
            Frame { ms: 100.0, x: 1.0, y: 0.0, health: 1.0, hit: true },
            Frame { ms: 290.0, x: 1.0, y: 1.0, health: 1.0, hit: false },
            Frame { ms: 300.0, x: 1.0, y: 1.0, health: 1.0, hit: true },
        ];
        let (m, r) = map_for_replay(&map, &replay_with("[\"mod_mirror\"]", frames));
        assert_eq!(r.flip, (true, false));
        assert_eq!(m.notes[0].x, 2.0);
    }

    #[test]
    fn mirror_without_hits_falls_back() {
        let map = Map {
            notes: vec![note(100, 0.0, 1.0)],
            ..Default::default()
        };
        let (_, r) = map_for_replay(&map, &replay_with("[\"mod_double_mirror\"]", vec![]));
        assert_eq!(r.flip, (true, true));
    }
}
