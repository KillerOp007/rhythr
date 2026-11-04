//! Autoplay: a synthetic run through a map. The result is a `Replay` value
//! in memory (never a file) that the ordinary pipeline renders: frames at
//! the output frame rate, one hit flag per note, a cursor that moves the way
//! the reference videos do.

pub mod path;
pub mod ticker;

use path::{plan, sample, Field, Waypoint};
use rhythia_formats::map::Map;
use rhythia_formats::rhr::{Frame, Replay};
use rhythia_sim::hitreg::{BASE_HIT_WINDOW_MS, CURSOR_EDGE_INSET, HITBOX_HALF};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Movement {
    Normal,
    Dance,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DanceAmount {
    Subtle,
    Medium,
    Wild,
}

impl DanceAmount {
    pub fn factor(self) -> f32 {
        match self {
            DanceAmount::Subtle => 0.5,
            DanceAmount::Medium => 1.0,
            DanceAmount::Wild => 1.5,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoplaySettings {
    pub movement: Movement,
    pub dance: DanceAmount,
    /// Real seconds without a note before the cursor rests in the centre.
    pub rest_secs: f64,
    /// The game's speed mod, 0.80 to 2.00 in steps of 0.05.
    pub speed_mod: f32,
}

impl Default for AutoplaySettings {
    fn default() -> Self {
        AutoplaySettings { movement: Movement::Dance, dance: DanceAmount::Medium, rest_secs: 4.5, speed_mod: 1.0 }
    }
}

impl AutoplaySettings {
    /// Snaps the speed mod to its 0.05 grid in 0.80..=2.00 and the rest
    /// threshold to its 0.5 grid in 2.0..=8.0.
    pub fn normalized(self) -> Self {
        let speed = (self.speed_mod.clamp(0.8, 2.0) / 0.05).round() * 0.05;
        let rest = (self.rest_secs.clamp(2.0, 8.0) / 0.5).round() * 0.5;
        AutoplaySettings { speed_mod: (speed * 100.0).round() / 100.0, rest_secs: rest, ..self }
    }
}

pub const PLAYER_NAME: &str = "Autoplay";
/// Cursor units per real second: the pace the paths are planned for.
pub const CRUISE: f32 = 3.0;
/// The run keeps going this long after the last note (unless the audio
/// ends first).
pub const END_PAD_MS: f64 = 3000.0;
/// Notes closer than this in time form a chord.
const CHORD_MS: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoplayError {
    NoNotes,
}

impl std::fmt::Display for AutoplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutoplayError::NoNotes => write!(f, "the map has no notes"),
        }
    }
}

impl std::error::Error for AutoplayError {}

/// The audio's end from the map's metadata, when it says anything.
pub fn audio_end_ms(map: &Map) -> Option<f64> {
    (map.meta.duration_ms > 0).then_some(map.meta.duration_ms as f64)
}

/// The ticker text for these settings.
pub fn ticker_text(settings: &AutoplaySettings) -> String {
    let movement = match settings.movement {
        Movement::Dance => "cursor dance",
        Movement::Normal => "normal movement",
    };
    format!("AUTOPLAY   rendered with rhythr   not a real play   {movement}   {:.2}x", settings.speed_mod)
}

/// The field barrier for these notes: the normal grid or wider when a note
/// sits beyond it, exactly as hit attribution widens its own bound.
fn field_for(map: &Map) -> Field {
    let mut grid = 1.0f32;
    for n in &map.notes {
        let (wx, wy) = crate::scene::grid_to_world(n.x, n.y);
        grid = grid.max(wx.abs()).max(wy.abs());
    }
    Field { bound: grid + (0.5 - CURSOR_EDGE_INSET) }
}

/// Builds the synthetic run: frames at `fps` (at least 60) with the cursor on the plan,
/// one hit-flagged frame per note at its visit time. `dice` (the mark
/// pattern) decides the sides of bellies and boomerangs.
pub fn generate(map: &Map, settings: &AutoplaySettings, fps: u32, dice: &[f32]) -> Result<Replay, AutoplayError> {
    if map.notes.is_empty() {
        return Err(AutoplayError::NoNotes);
    }
    let settings = settings.normalized();
    // at least 60 per second of map time whatever the output rate: the
    // renderer interpolates between frames and a 24 fps run would turn
    // every arc into a polygon
    let fps = fps.clamp(24, 240).max(60);
    let frame_ms = 1000.0 / fps as f64;
    let window_ms = BASE_HIT_WINDOW_MS * f64::from(settings.speed_mod);
    let field = field_for(map);

    // waypoints: one per note, chords swept in chart order inside the
    // window, an adjacent pair covered from one point in both hit areas
    let mut notes: Vec<(i64, f32, f32)> = map
        .notes
        .iter()
        .map(|n| {
            let (wx, wy) = crate::scene::grid_to_world(n.x, n.y);
            (n.time_ms.max(0), wx, wy)
        })
        .collect();
    notes.sort_by_key(|n| n.0);
    let mut visits: Vec<Waypoint> = Vec::with_capacity(notes.len()); // one per note, note order
    let mut i = 0;
    while i < notes.len() {
        let mut j = i + 1;
        while j < notes.len() && notes[j].0 - notes[i].0 <= CHORD_MS {
            j += 1;
        }
        let count = j - i;
        let t = notes[i].0 as f64;
        if count == 1 {
            visits.push(Waypoint { t_ms: t, x: notes[i].1, y: notes[i].2 });
        } else if count == 2
            && (notes[i].1 - notes[i + 1].1).abs() <= 2.0 * HITBOX_HALF
            && (notes[i].2 - notes[i + 1].2).abs() <= 2.0 * HITBOX_HALF
        {
            let mx = (notes[i].1 + notes[i + 1].1) / 2.0;
            let my = (notes[i].2 + notes[i + 1].2) / 2.0;
            visits.push(Waypoint { t_ms: t, x: mx, y: my });
            visits.push(Waypoint { t_ms: t + 0.5, x: mx, y: my });
        } else {
            let dt = frame_ms.min(window_ms / count as f64);
            for (k, n) in notes[i..j].iter().enumerate() {
                visits.push(Waypoint { t_ms: t + k as f64 * dt, x: n.1, y: n.2 });
            }
        }
        i = j;
    }

    let plan = plan(&visits, &settings, &field, dice);
    let last_note = notes.last().map(|n| n.0 as f64).unwrap_or(0.0);
    let end_ms = audio_end_ms(map).filter(|&a| a > last_note).unwrap_or(last_note + END_PAD_MS);

    // the regular frames, then one flagged frame per note at its visit time
    let count = (end_ms / frame_ms).ceil() as usize + 1;
    let mut frames: Vec<Frame> = Vec::with_capacity(count + visits.len());
    for k in 0..count {
        let ms = k as f64 * frame_ms;
        let (x, y) = sample(&plan, ms, &field);
        frames.push(Frame { ms, x, y, health: 1.0, hit: false });
    }
    for v in &visits {
        let (x, y) = sample(&plan, v.t_ms, &field);
        frames.push(Frame { ms: v.t_ms, x, y, health: 1.0, hit: true });
    }
    frames.sort_by(|a, b| a.ms.partial_cmp(&b.ms).unwrap().then(b.hit.cmp(&a.hit)));

    let ticks = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64 / 100 + 621_355_968_000_000_000)
        .unwrap_or(0);
    Ok(Replay {
        version: rhythia_formats::rhr::V_PITCH_LOCK,
        timestamp_ticks: ticks,
        player_name: PLAYER_NAME.to_string(),
        legacy_map_id: map.meta.legacy_id.clone(),
        map_id: map.meta.online_id.and_then(|id| i32::try_from(id).ok()).unwrap_or(0),
        start_from_ms: 0,
        mode: "autoplay".to_string(),
        passed: true,
        mods: "[]".to_string(),
        spin: false,
        speed: settings.speed_mod,
        total_score: 0,
        accuracy_pct: 100.0,
        hits: map.notes.len() as i32,
        misses: 0,
        points: 0.0,
        fail_time_ms: -1,
        beatmap_hash: String::new(),
        pitch_lock: false,
        frames,
        trailing_bytes: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhythia_formats::map::{Map, Note};

    fn map_with(notes: &[(i64, f32, f32)]) -> Map {
        let mut map = Map::default();
        for &(t, x, y) in notes {
            map.notes.push(Note { time_ms: t, x, y });
        }
        map.notes.sort_by_key(|n| n.time_ms);
        map
    }

    fn dice() -> Vec<f32> {
        crate::mark::pattern(&crate::mark::seed_from_hex(crate::mark::DEV_KEY_HEX).unwrap())
    }

    fn world(n: &Note) -> (f32, f32) {
        crate::scene::grid_to_world(n.x, n.y)
    }

    #[test]
    fn the_cursor_is_on_every_isolated_note_at_its_time() {
        let map = map_with(&[(500, 0.0, 0.0), (1200, 2.0, 1.0), (2500, 1.0, 2.0), (4000, 0.0, 2.0)]);
        let r = generate(&map, &AutoplaySettings::default(), 60, &dice()).unwrap();
        for n in &map.notes {
            let (wx, wy) = world(n);
            let f = r
                .frames
                .iter()
                .min_by(|a, b| (a.ms - n.time_ms as f64).abs().partial_cmp(&(b.ms - n.time_ms as f64).abs()).unwrap())
                .unwrap();
            assert!((f.x - wx).abs() < 0.02 && (f.y - wy).abs() < 0.02, "note at {} ms: frame {:?}", n.time_ms, f);
        }
    }

    #[test]
    fn frames_come_at_least_sixty_per_second_whatever_the_output_rate() {
        let map = map_with(&[(500, 0.0, 0.0), (2500, 2.0, 2.0)]);
        let slow = generate(&map, &AutoplaySettings::default(), 24, &dice()).unwrap();
        let sixty = generate(&map, &AutoplaySettings::default(), 60, &dice()).unwrap();
        let fast = generate(&map, &AutoplaySettings::default(), 120, &dice()).unwrap();
        assert_eq!(slow.frames.len(), sixty.frames.len(), "a 24 fps output must not thin the run below 60 frames per second");
        assert!(fast.frames.len() > sixty.frames.len() * 3 / 2, "a faster output keeps its own rate: {} vs {}", fast.frames.len(), sixty.frames.len());
    }

    #[test]
    fn every_note_registers_as_a_hit() {
        let map = map_with(&[(500, 0.0, 0.0), (1200, 2.0, 1.0), (1200, 1.0, 1.0), (2500, 1.0, 2.0), (2516, 1.0, 2.0), (4000, 0.0, 2.0)]);
        let r = generate(&map, &AutoplaySettings::default(), 60, &dice()).unwrap();
        let out = rhythia_sim::hitreg::match_hits(&map.notes, &r.frames, rhythia_sim::hitreg::hit_window_ms(&r));
        assert_eq!(out.derived_hits() as usize, map.notes.len());
        assert_eq!(out.orphan_flags, 0);
        assert_eq!(r.frames.iter().filter(|f| f.hit).count(), map.notes.len());
    }

    #[test]
    fn a_chord_of_eight_registers_every_note() {
        let notes: Vec<(i64, f32, f32)> = (0..8).map(|i| (1000, (i % 3) as f32, (i / 3) as f32)).collect();
        let map = map_with(&notes);
        let r = generate(&map, &AutoplaySettings::default(), 60, &dice()).unwrap();
        let out = rhythia_sim::hitreg::match_hits(&map.notes, &r.frames, rhythia_sim::hitreg::hit_window_ms(&r));
        assert_eq!(out.derived_hits(), 8);
        assert_eq!(out.orphan_flags, 0);
    }

    #[test]
    fn no_frame_leaves_the_barrier_and_off_grid_notes_are_reached_and_clamped() {
        let map = map_with(&[(500, -0.4, 0.0), (1500, 2.4, 2.4), (2600, 1.0, 1.0)]);
        let s = AutoplaySettings { dance: DanceAmount::Wild, ..Default::default() };
        let r = generate(&map, &s, 60, &dice()).unwrap();
        let bound = 1.4 + (0.5 - rhythia_sim::hitreg::CURSOR_EDGE_INSET);
        for f in &r.frames {
            assert!(f.x.abs() <= bound + 1e-4 && f.y.abs() <= bound + 1e-4, "{f:?}");
        }
        let out = rhythia_sim::hitreg::match_hits(&map.notes, &r.frames, rhythia_sim::hitreg::hit_window_ms(&r));
        assert_eq!(out.derived_hits(), 3);
    }

    #[test]
    fn close_notes_go_almost_straight() {
        let map = map_with(&[(1000, 0.0, 1.0), (1150, 2.0, 1.0)]);
        let r = generate(&map, &AutoplaySettings::default(), 120, &dice()).unwrap();
        let path_len: f32 = r
            .frames
            .windows(2)
            .filter(|w| w[0].ms >= 1000.0 && w[1].ms <= 1150.0)
            .map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt())
            .sum();
        assert!(path_len < 2.0 * 1.02, "path {path_len} for a chord of 2.0");
    }

    #[test]
    fn same_spot_with_time_makes_a_boomerang_and_short_gaps_hold() {
        let map = map_with(&[(1000, 2.0, 1.0), (2500, 2.0, 1.0), (2700, 2.0, 1.0)]);
        let r = generate(&map, &AutoplaySettings::default(), 60, &dice()).unwrap();
        let (wx, wy) = crate::scene::grid_to_world(2.0, 1.0);
        let far = r
            .frames
            .iter()
            .filter(|f| f.ms > 1100.0 && f.ms < 2400.0)
            .map(|f| ((f.x - wx).powi(2) + (f.y - wy).powi(2)).sqrt())
            .fold(0.0f32, f32::max);
        assert!(far > 0.3, "boomerang reach {far}");
        let held = r.frames.iter().filter(|f| f.ms > 2520.0 && f.ms < 2680.0).all(|f| (f.x - wx).abs() < 1e-3 && (f.y - wy).abs() < 1e-3);
        assert!(held, "short gap should hold");
    }

    #[test]
    fn rest_threshold_is_real_time() {
        let at = |gap_ms: i64, speed: f32| {
            let map = map_with(&[(1000, 0.0, 0.0), (1000 + gap_ms, 0.0, 0.0)]);
            let s = AutoplaySettings { speed_mod: speed, ..Default::default() };
            let r = generate(&map, &s, 60, &dice()).unwrap();
            r.frames
                .iter()
                .filter(|f| f.ms > 1500.0 && f.ms < 1000.0 + gap_ms as f64 - 500.0)
                .any(|f| f.x.abs() < 1e-3 && f.y.abs() < 1e-3)
        };
        assert!(at(9000, 2.0), "9 s at 2.00x is 4.5 s real: rests");
        assert!(at(9000, 1.0));
        assert!(!at(6000, 2.0), "6 s at 2.00x is 3 s real: no rest");
    }

    #[test]
    fn the_run_is_deterministic_and_normal_movement_has_no_detours() {
        let map = map_with(&[(500, 0.0, 0.0), (2000, 2.0, 2.0), (3500, 2.0, 2.0)]);
        let a = generate(&map, &AutoplaySettings::default(), 60, &dice()).unwrap();
        let b = generate(&map, &AutoplaySettings::default(), 60, &dice()).unwrap();
        assert_eq!(a.frames, b.frames);
        let n = generate(&map, &AutoplaySettings { movement: Movement::Normal, ..Default::default() }, 60, &dice()).unwrap();
        let (wx, wy) = crate::scene::grid_to_world(2.0, 2.0);
        assert!(n.frames.iter().filter(|f| f.ms > 2100.0 && f.ms < 3400.0).all(|f| (f.x - wx).abs() < 1e-3 && (f.y - wy).abs() < 1e-3));
    }

    #[test]
    fn the_replay_header_and_the_end_follow_the_spec() {
        let mut map = map_with(&[(500, 1.0, 1.0)]);
        map.meta.duration_ms = 2000;
        let r = generate(&map, &AutoplaySettings::default(), 60, &dice()).unwrap();
        assert_eq!(r.player_name, "Autoplay");
        assert_eq!(r.mods, "[]");
        assert!(r.passed && r.misses == 0 && r.hits == 1 && (r.accuracy_pct - 100.0).abs() < 1e-6);
        assert!((r.length_ms() - 2000.0).abs() < 20.0, "ends at the audio end, got {}", r.length_ms());
        map.meta.duration_ms = 0;
        let r = generate(&map, &AutoplaySettings::default(), 60, &dice()).unwrap();
        assert!((r.length_ms() - 3500.0).abs() < 20.0);
        assert!(matches!(generate(&Map::default(), &AutoplaySettings::default(), 60, &dice()), Err(AutoplayError::NoNotes)));
    }

    #[test]
    fn the_ticker_text_names_movement_and_speed() {
        let s = AutoplaySettings { speed_mod: 1.25, ..Default::default() };
        assert_eq!(ticker_text(&s), "AUTOPLAY   rendered with rhythr   not a real play   cursor dance   1.25x");
        let s = AutoplaySettings { movement: Movement::Normal, ..Default::default() };
        assert!(ticker_text(&s).ends_with("normal movement   1.00x"));
    }

    #[test]
    fn settings_normalise_to_the_grids() {
        let s = AutoplaySettings { speed_mod: 1.337, rest_secs: 9.9, ..Default::default() }.normalized();
        assert!((s.speed_mod - 1.35).abs() < 1e-6 && (s.rest_secs - 8.0).abs() < 1e-9);
        let s = AutoplaySettings { speed_mod: 0.1, rest_secs: 0.0, ..Default::default() }.normalized();
        assert!((s.speed_mod - 0.8).abs() < 1e-6 && (s.rest_secs - 2.0).abs() < 1e-9);
    }
}
