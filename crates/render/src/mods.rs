//! Geometry mods recorded in a replay, applied to the note grid so the
//! render shows the field the player actually saw. Speed mods are baked
//! into the replay's frame times already; nofail/sudden-death and friends
//! change no geometry. Chaos randomises positions with a seed the replay
//! does not store, so it cannot be reconstructed and is left as-is.

use rhythia_formats::map::Map;
use rhythia_formats::rhr::Replay;
use rhythia_sim::hitreg::{hit_window_ms, match_hits_timing_only};

/// Hardrock scales the note grid outward around its centre. Empirical, from
/// the hardrock testdata replay: the game's cursor clamp grows from
/// ±1.36875 to ±1.51875 (+0.15000 exactly, i.e. grid half-extent 1.0 →
/// 1.15 with the cursor margin unchanged), and cursor samples on edge
/// notes sit ~1.18× out versus 0.83-1.01× on unmodded baselines.
pub const HARDROCK_GRID_SCALE: f32 = 1.15;

/// What [`map_for_replay`] applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedMods {
    /// Grid half-extent this run played on (1.0, or 1.15 under hardrock).
    /// The playfield border widens with it.
    pub grid_scale: f32,
    /// Mirror flips applied to the notes (x, y).
    pub flip: (bool, bool),
    /// Ghost: notes fade to nothing before reaching the plane
    /// (`NoteManager.gd:425-428`, fade_out_base stays 1).
    pub ghost: bool,
    /// Half Ghost: notes fade toward a fifth opacity near the plane rather
    /// than to nothing. The client also carries this as a `HalfGhost`
    /// setting in its config, which is why the render ORs the two rather
    /// than letting either win: a run recorded with the mod always shows
    /// it, and a viewer who has it switched on still gets what they asked
    /// for.
    pub half_ghost: bool,
    /// Nearsighted: notes stay invisible until late in the approach
    /// (`NoteManager.gd:436-438`).
    ///
    /// No shipped build is known to write this: the client's own mod icons
    /// cover ghost and half ghost and there is no nearsighted among them.
    /// The branch stays because it is correct and costs nothing, but do not
    /// read a render as having exercised it.
    pub nearsighted: bool,
}

/// Mods that change the run in a way rhythr cannot reproduce, so a video
/// of one is not what the player played. Anything not listed here is
/// either applied ([`map_for_replay`]) or genuinely cosmetic to the render
/// (nofail and sudden death change no geometry; the speed mods are already
/// baked into the replay's frame times).
///
/// Returned as the raw strings so an unknown future mod surfaces as itself
/// rather than being silently swallowed.
pub fn unsupported_mods(replay: &Replay) -> Vec<String> {
    const HANDLED: [&str; 9] = [
        "mod_mirror",
        "mod_mirror_y",
        "mod_double_mirror",
        "mod_hardrock",
        "mod_ghost",
        "mod_half_ghost",
        "mod_nearsighted",
        "mod_nofail",
        "mod_sudden_death",
    ];
    // The speed mods need no handling: the replay's frame times are song
    // time, so the speed is already in the data.
    named_mods(replay).into_iter().filter(|m| !HANDLED.contains(&m.as_str())).collect()
}

/// A speed mod's name: `mod_` and its percentage (mod_87, mod_125), or for
/// the faster ranked steps the part above a hundred (mod_15, mod_45). A
/// number is not enough to go by: `mod_360` is a mod of its own with its
/// own icon, and no speed reaches three times the song.
fn is_speed_name(name: &str) -> bool {
    name.strip_prefix("mod_")
        .filter(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
        .and_then(|r| r.parse::<u32>().ok())
        .is_some_and(|n| n < 300)
}

/// The mods of a run under the names of the game's icons, for whoever
/// draws them. They are [`named_mods`] but for the mirrors: the replay
/// lists each mirrored axis (`mod_mirror`, `mod_mirror_y`), and the game
/// has one icon for a single mirror and one for both.
pub fn icon_mods(replay: &Replay) -> Vec<String> {
    let mut mods = named_mods(replay);
    let x = mods.iter().position(|m| m == "mod_mirror");
    let y = mods.iter().position(|m| m == "mod_mirror_y");
    match (x, y) {
        (Some(x), Some(y)) => {
            mods[x.min(y)] = "mod_double_mirror".into();
            mods.remove(x.max(y));
        }
        (None, Some(y)) => mods[y] = "mod_mirror".into(),
        _ => {}
    }
    let mut out: Vec<String> = Vec::new();
    for m in mods {
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

/// The mods a run lists, in its order and each once, without the speed
/// mods: a run's speed is the replay's own number, and whoever shows the
/// mods shows it from there.
pub fn named_mods(replay: &Replay) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in mod_list(replay) {
        if !is_speed_name(&m) && !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

impl ResolvedMods {
    pub fn none() -> ResolvedMods {
        ResolvedMods {
            grid_scale: 1.0,
            flip: (false, false),
            ghost: false,
            half_ghost: false,
            nearsighted: false,
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

/// Which axes a mirrored run was flipped on, read from the run itself: the
/// cursor followed the *mirrored* notes, so the flip whose note positions
/// best match the cursor at the hit moments is the one that was active
/// (clear margin in practice: the right flip scores about half a cell,
/// wrong ones 1.5 and more). The replay names the axes too and that is the
/// `fallback`, but a converted replay of a free client carries one name
/// for either axis, and the run cannot be wrong about itself.
fn detect_flip(map: &Map, replay: &Replay, fallback: (bool, bool)) -> (bool, bool) {
    // Timing-only on purpose: this runs on the UNFLIPPED map, and the
    // cursor-guided phase would "correct" attributions against geometry
    // the player never saw, biasing the flip score toward identity.
    let outcome = match_hits_timing_only(&map.notes, &replay.frames, hit_window_ms(replay));
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

/// Returns the map as this replay's player saw it (mirror flips and the
/// hardrock spread applied to every note) plus what was applied. Sides of
/// a ghost race resolve this independently, so a hardrock run races a
/// normal one with each side's own field.
pub fn map_for_replay(map: &Map, replay: &Replay) -> (Map, ResolvedMods) {
    let mods = mod_list(replay);
    let has = |m: &str| mods.iter().any(|x| x == m);

    let mut resolved = ResolvedMods::none();
    // The game lists each mirrored axis on its own: `mod_mirror` is left
    // to right, `mod_mirror_y` is top to bottom, and a run with both
    // carries both names. `mod_double_mirror` is what the game calls the
    // icon for both, and what a converted free-client replay arrives with.
    let double = has("mod_double_mirror");
    let named = (has("mod_mirror") || double, has("mod_mirror_y") || double);
    if named != (false, false) {
        resolved.flip = detect_flip(map, replay, named);
    }
    if has("mod_hardrock") {
        resolved.grid_scale = HARDROCK_GRID_SCALE;
    }
    // Visibility mods change nothing about the notes themselves, only when
    // they can be seen, but a render that ignores them shows a comfortable
    // read of a run that was played half blind.
    resolved.ghost = has("mod_ghost");
    resolved.half_ghost = has("mod_half_ghost");
    resolved.nearsighted = has("mod_nearsighted");

    // Only the mods that MOVE notes force a rebuilt map: mirror flips and
    // the hardrock grid scale, both applied in the loop below. The
    // visibility mods ride along on the params and must not be part of this
    // test, or a ghost run would rebuild the map for nothing.
    if resolved.flip == (false, false) && resolved.grid_scale == 1.0 {
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
        Note { time_ms: t, x, y }
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
            pitch_lock: false,
            frames,
            trailing_bytes: 0,
            origin: rhythia_formats::rhr::Origin::Rhr,
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
            Frame {
                ms: 90.0,
                x: 1.0,
                y: 0.0,
                health: 1.0,
                hit: false,
            },
            Frame {
                ms: 100.0,
                x: 1.0,
                y: 0.0,
                health: 1.0,
                hit: true,
            },
            Frame {
                ms: 290.0,
                x: 1.0,
                y: 1.0,
                health: 1.0,
                hit: false,
            },
            Frame {
                ms: 300.0,
                x: 1.0,
                y: 1.0,
                health: 1.0,
                hit: true,
            },
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

    /// The game names the top-to-bottom mirror `mod_mirror_y`, on its own
    /// or next to `mod_mirror`. It used to be read by nothing: a run with
    /// only that mirror was rendered unmirrored, its cursor a field away
    /// from every note it hit.
    #[test]
    fn the_vertical_mirror_is_read_from_its_own_name() {
        // A note in the top row; the cursor sat in the BOTTOM row when it
        // was hit.
        let map = Map {
            notes: vec![note(100, 0.0, 0.0), note(300, 1.0, 0.0)],
            ..Default::default()
        };
        let frame = |ms: f64, x: f32, y: f32, hit: bool| Frame { ms, x, y, health: 1.0, hit };
        let frames = vec![
            frame(90.0, -1.0, -1.0, false),
            frame(100.0, -1.0, -1.0, true),
            frame(290.0, 0.0, -1.0, false),
            frame(300.0, 0.0, -1.0, true),
        ];
        let (m, r) = map_for_replay(&map, &replay_with("[\"mod_mirror_y\"]", frames));
        assert_eq!(r.flip, (false, true));
        assert_eq!((m.notes[0].x, m.notes[0].y), (0.0, 2.0));

        // Without a hit to go by, the names decide.
        let named = |mods: &str| map_for_replay(&map, &replay_with(mods, vec![])).1.flip;
        assert_eq!(named("[\"mod_mirror_y\"]"), (false, true));
        assert_eq!(named("[\"mod_mirror\"]"), (true, false));
        assert_eq!(named("[\"mod_mirror\",\"mod_mirror_y\"]"), (true, true));
        assert!(unsupported_mods(&replay_with("[\"mod_mirror\",\"mod_mirror_y\"]", vec![])).is_empty());
    }

    #[test]
    fn the_mirrors_are_shown_under_the_games_two_icons() {
        let shown = |m: &str| icon_mods(&replay_with(m, vec![]));
        assert_eq!(shown("[\"mod_mirror\"]"), vec!["mod_mirror"]);
        assert_eq!(shown("[\"mod_mirror_y\"]"), vec!["mod_mirror"]);
        assert_eq!(shown("[\"mod_hardrock\",\"mod_mirror\",\"mod_ghost\",\"mod_mirror_y\"]"), vec!["mod_hardrock", "mod_double_mirror", "mod_ghost"]);
        assert_eq!(shown("[\"mod_mirror_y\",\"mod_mirror\"]"), vec!["mod_double_mirror"]);
        assert_eq!(shown("[\"mod_double_mirror\"]"), vec!["mod_double_mirror"]);
        assert_eq!(shown("[\"mod_45\",\"mod_nofail\"]"), vec!["mod_nofail"]);
    }

    /// Half ghost is the visibility mod the shipped client actually offers
    /// (its own icon set has `mod_ghost` and `mod_half_ghost` and no
    /// nearsighted at all), and it used to be read by nothing.
    #[test]
    fn half_ghost_is_resolved_from_the_replay() {
        let map = Map {
            notes: vec![note(100, 0.0, 1.0)],
            ..Default::default()
        };
        let (_, r) = map_for_replay(&map, &replay_with("[\"mod_half_ghost\"]", vec![]));
        assert!(r.half_ghost);
        assert!(!r.ghost, "half ghost is not the full one");

        // And the full ghost must not be triggered by a prefix match.
        let (_, r) = map_for_replay(&map, &replay_with("[\"mod_ghost\"]", vec![]));
        assert!(r.ghost && !r.half_ghost);
    }

    /// A mod rhythr cannot reproduce has to be nameable, or a video of a
    /// chaos run looks like an ordinary one with an extra icon on the
    /// results screen.
    #[test]
    fn only_the_mods_that_change_nothing_are_treated_as_covered() {
        let unsupported = |m: &str| unsupported_mods(&replay_with(m, vec![]));

        // Applied, or genuinely nothing to do.
        for m in [
            "mod_mirror",
            "mod_double_mirror",
            "mod_hardrock",
            "mod_ghost",
            "mod_half_ghost",
            "mod_nofail",
            "mod_sudden_death",
        ] {
            assert!(
                unsupported(&format!("[\"{m}\"]")).is_empty(),
                "{m} is handled and should not be reported"
            );
        }

        // Speed is baked into the frame times, whatever the percentage.
        for m in ["mod_15", "mod_87", "mod_100", "mod_145"] {
            assert!(unsupported(&format!("[\"{m}\"]")).is_empty(), "{m}");
        }
        // 360 is a number too, and not a speed: it is a mod rhythr does
        // nothing for, and it has to be named like any other.
        assert_eq!(unsupported("[\"mod_360\"]"), vec!["mod_360"]);

        // Chaos randomises note positions with a seed the replay does not
        // store, so the render is provably not the run that was played.
        assert_eq!(unsupported("[\"mod_chaos\"]"), vec!["mod_chaos"]);
        assert_eq!(unsupported("[\"mod_practice\"]"), vec!["mod_practice"]);
        // An unknown future mod surfaces as itself rather than vanishing.
        assert_eq!(unsupported("[\"mod_whatever\"]"), vec!["mod_whatever"]);
        // "mod_" with nothing after it is not a speed mod.
        assert_eq!(unsupported("[\"mod_\"]"), vec!["mod_"]);

        assert!(unsupported("[]").is_empty());
        assert!(unsupported("not json at all").is_empty());
    }

    #[test]
    fn the_named_mods_come_in_the_replays_order_once_and_without_the_speed() {
        let named = |m: &str| named_mods(&replay_with(m, vec![]));
        assert_eq!(
            named("[\"mod_hardrock\",\"mod_45\",\"mod_ghost\",\"mod_hardrock\",\"mod_125\",\"mod_360\"]"),
            vec!["mod_hardrock", "mod_ghost", "mod_360"]
        );
        assert!(named("[]").is_empty());
        assert!(named("not json").is_empty());
    }
}
