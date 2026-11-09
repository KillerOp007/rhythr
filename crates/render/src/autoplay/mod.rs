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

/// Where the cursor goes inside a stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Aim {
    /// Through the centre of every note, as a perfect player would hit it;
    /// a corner is turned on the note itself.
    Centre,
    /// Like a player's flow aim: a little inside the corners, always within
    /// the hit area, so the turn spreads along the path.
    Flow,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoplaySettings {
    pub movement: Movement,
    pub dance: DanceAmount,
    pub aim: Aim,
    /// Real seconds without a note before the cursor rests in the centre.
    pub rest_secs: f64,
    /// The game's speed mod, 0.80 to 2.00 in steps of 0.05.
    pub speed_mod: f32,
    /// Whether the AUTOPLAY ticker runs across the top of the video. The
    /// hidden mark and the metadata tag do not depend on it.
    pub banner: bool,
}

impl Default for AutoplaySettings {
    fn default() -> Self {
        AutoplaySettings { movement: Movement::Dance, dance: DanceAmount::Medium, aim: Aim::Centre, rest_secs: 4.5, speed_mod: 1.0, banner: true }
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

/// The ticker for these settings, or none when the banner is switched off.
pub fn ticker_for(settings: &AutoplaySettings) -> Option<String> {
    settings.banner.then(|| ticker_text(settings))
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
            visits.push(Waypoint { t_ms: t, x: notes[i].1, y: notes[i].2, exact: false });
        } else if count == 2
            && (notes[i].1 - notes[i + 1].1).abs() <= 2.0 * HITBOX_HALF
            && (notes[i].2 - notes[i + 1].2).abs() <= 2.0 * HITBOX_HALF
        {
            // one spot in both hit areas: it has no room to spare, so the
            // planner keeps the cursor exactly there at exactly these times
            let mx = (notes[i].1 + notes[i + 1].1) / 2.0;
            let my = (notes[i].2 + notes[i + 1].2) / 2.0;
            visits.push(Waypoint { t_ms: t, x: mx, y: my, exact: true });
            visits.push(Waypoint { t_ms: t + 0.5, x: mx, y: my, exact: true });
        } else {
            let dt = frame_ms.min(window_ms / count as f64);
            for (k, n) in notes[i..j].iter().enumerate() {
                visits.push(Waypoint { t_ms: t + k as f64 * dt, x: n.1, y: n.2, exact: false });
            }
        }
        i = j;
    }

    let plan = plan(&visits, &settings, &field, dice);
    let last_note = notes.last().map(|n| n.0 as f64).unwrap_or(0.0);
    let end_ms = audio_end_ms(map).filter(|&a| a > last_note).unwrap_or(last_note + END_PAD_MS);

    // the regular frames, then one flagged frame per note where the plan
    // passes it (its chart time, or the closest approach inside a sweep)
    let count = (end_ms / frame_ms).ceil() as usize + 1;
    let mut frames: Vec<Frame> = Vec::with_capacity(count + visits.len());
    for k in 0..count {
        let ms = k as f64 * frame_ms;
        let (x, y) = sample(&plan, ms, &field);
        frames.push(Frame { ms, x, y, health: 1.0, hit: false });
    }
    for &t in &plan.visits {
        let (x, y) = sample(&plan, t, &field);
        frames.push(Frame { ms: t, x, y, health: 1.0, hit: true });
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

    /// The game's own test, with the planner's margin: every note's flag
    /// frame has the cursor inside the note's hit area, flags come in note
    /// order and never before the note, and the matcher attributes each
    /// flag to its own note. Counting hits alone cannot see any of this:
    /// the matcher's first pass is timing only.
    fn assert_every_note_covered(map: &Map, r: &Replay, what: &str) {
        let flags: Vec<&Frame> = r.frames.iter().filter(|f| f.hit).collect();
        assert_eq!(flags.len(), map.notes.len(), "{what}: one flag per note");
        let out = rhythia_sim::hitreg::match_hits(&map.notes, &r.frames, rhythia_sim::hitreg::hit_window_ms(r));
        assert_eq!(out.derived_hits() as usize, map.notes.len(), "{what}: full combo");
        assert_eq!(out.orphan_flags, 0, "{what}: orphan flags");
        let mut last = f64::MIN;
        for (i, (n, f)) in map.notes.iter().zip(&flags).enumerate() {
            let (wx, wy) = world(n);
            // a note sharing its time with another is served from one spot
            // between them, which the chord rule puts inside both hit areas
            // but with no margin to spare; a note on its own keeps the
            // planner's margin
            let chord = (i > 0 && map.notes[i - 1].time_ms == n.time_ms) || (i + 1 < map.notes.len() && map.notes[i + 1].time_ms == n.time_ms);
            let limit = if chord { HITBOX_HALF - 0.002 } else { HITBOX_HALF - path::HIT_MARGIN + 1e-4 };
            assert!((f.x - wx).abs() <= limit && (f.y - wy).abs() <= limit, "{what}: note {i} at {} ms ({},{}) flagged at {} ms with the cursor at ({:.3},{:.3}), {:.3} off per axis", n.time_ms, n.x, n.y, f.ms, f.x, f.y, (f.x - wx).abs().max((f.y - wy).abs()));
            assert!(f.ms >= n.time_ms as f64 - 1e-6, "{what}: note {i} flagged {:.1} ms early", n.time_ms as f64 - f.ms);
            assert!(f.ms >= last, "{what}: flags out of order at note {i}");
            last = f.ms;
            assert_eq!(out.results[i].hit_ms, Some(f.ms), "{what}: note {i} attributed to another flag");
        }
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

    /// The largest distance of any frame in `(from_ms, to_ms)` from the
    /// straight line between the two notes (the chord).
    fn max_off_chord(r: &Replay, from_ms: f64, to_ms: f64, a: (f32, f32), b: (f32, f32)) -> f32 {
        let d = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        r.frames
            .iter()
            .filter(|f| f.ms > from_ms && f.ms < to_ms)
            .map(|f| ((b.0 - a.0) * (a.1 - f.y) - (a.0 - f.x) * (b.1 - a.1)).abs() / d)
            .fold(0.0f32, f32::max)
    }

    /// Real maps leave 150 ms between notes: the dance must arc on the
    /// distance alone, time to spare only widens it. Close notes stay almost
    /// straight, far notes get a visible arc, the amount scales it.
    #[test]
    fn close_notes_go_almost_straight_and_far_notes_arc_even_in_a_hurry() {
        let near = map_with(&[(1000, 1.0, 1.0), (1150, 1.5, 1.0)]);
        let far = map_with(&[(1000, 0.0, 1.0), (1150, 2.0, 1.0)]);
        let off = |map: &Map, dance: DanceAmount| {
            let s = AutoplaySettings { movement: Movement::Dance, dance, ..Default::default() };
            let r = generate(map, &s, 120, &dice()).unwrap();
            let a = world(&map.notes[0]);
            let b = world(&map.notes[1]);
            max_off_chord(&r, 1000.0, 1150.0, a, b)
        };
        assert!(off(&near, DanceAmount::Medium) < 0.15, "near: {}", off(&near, DanceAmount::Medium));
        let (subtle, medium, wild) = (off(&far, DanceAmount::Subtle), off(&far, DanceAmount::Medium), off(&far, DanceAmount::Wild));
        assert!(medium > 0.3, "far at medium: {medium}");
        assert!(subtle < medium && medium < wild, "subtle {subtle}, medium {medium}, wild {wild}");
        let s = AutoplaySettings { movement: Movement::Normal, ..Default::default() };
        let r = generate(&far, &s, 120, &dice()).unwrap();
        assert!(max_off_chord(&r, 1000.0, 1150.0, world(&far.notes[0]), world(&far.notes[1])) < 0.02, "normal movement stays straight");
    }

    /// A stream (notes faster than eight per second) is taken in one
    /// sweep: no dance belly however wild, so a line of notes is a line.
    /// (The line runs along the top row: a first note on the centre row
    /// would send the cursor out from the centre and straight back, a
    /// reversal, which is a loop by design.)
    #[test]
    fn a_stream_in_a_line_goes_straight_even_in_wild_dance() {
        let map = map_with(&[(1000, 0.0, 0.0), (1048, 1.0, 0.0), (1096, 2.0, 0.0), (1144, 2.0, 1.0), (1192, 1.0, 1.0)]);
        let s = AutoplaySettings { movement: Movement::Dance, dance: DanceAmount::Wild, aim: Aim::Flow, ..Default::default() };
        let r = generate(&map, &s, 240, &dice()).unwrap();
        // the note in the middle of the line is passed on the line, through
        // its centre, heading straight on (the corners at either end are
        // cut, that is the flow aim's job)
        let mid = passes_within(&r, 1048.0, world(&map.notes[1]));
        assert!(mid < 0.02, "a straight pass is cut by {mid}");
        let turn = turn_at(&r, 1048.0);
        assert!(turn < 10.0, "a stream line bends {turn} degrees at its middle note");
        let out = rhythia_sim::hitreg::match_hits(&map.notes, &r.frames, rhythia_sim::hitreg::hit_window_ms(&r));
        assert_eq!(out.derived_hits(), 5);
    }

    /// How close the sampled path comes to `c` within 24 ms of `ms`: the
    /// distance to the nearest segment between two consecutive frames (the
    /// frames themselves may straddle the point).
    fn passes_within(r: &Replay, ms: f64, c: (f32, f32)) -> f32 {
        let fr: Vec<&Frame> = r.frames.iter().filter(|f| !f.hit && (f.ms - ms).abs() <= 24.0).collect();
        fr.windows(2)
            .map(|w| {
                let (a, b) = ((w[0].x, w[0].y), (w[1].x, w[1].y));
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len2 = dx * dx + dy * dy;
                let t = if len2 > 1e-12 { (((c.0 - a.0) * dx + (c.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                ((a.0 + dx * t - c.0).powi(2) + (a.1 + dy * t - c.1).powi(2)).sqrt()
            })
            .fold(f32::INFINITY, f32::min)
    }

    /// Heading of the run 20 ms before and after `ms`, as the angle between
    /// the two chords, in degrees.
    fn turn_at(r: &Replay, ms: f64) -> f32 {
        let at = |t: f64| {
            let f = r.frames.iter().filter(|f| !f.hit).min_by(|a, b| (a.ms - t).abs().partial_cmp(&(b.ms - t).abs()).unwrap()).unwrap();
            (f.x, f.y)
        };
        let (a, p, b) = (at(ms - 20.0), at(ms), at(ms + 20.0));
        let u = (p.0 - a.0, p.1 - a.1);
        let v = (b.0 - p.0, b.1 - p.1);
        let len = (u.0 * u.0 + u.1 * u.1).sqrt() * (v.0 * v.0 + v.1 * v.1).sqrt();
        ((u.0 * v.0 + u.1 * v.1) / len.max(1e-9)).clamp(-1.0, 1.0).acos().to_degrees()
    }

    /// Inside a stream the cursor flows like a player's: it cuts the corner
    /// a little instead of turning on the note centre (players pass about a
    /// quarter of a unit beside it), the hit fires as it passes, and every
    /// note still registers. Measured on a ring of one-unit hops at 48 ms,
    /// in both movements.
    #[test]
    fn a_stream_cuts_its_corners_like_a_player_and_still_hits_every_note() {
        let map = map_with(&[(1000, 0.0, 0.0), (1048, 0.0, 1.0), (1096, 0.0, 2.0), (1144, 1.0, 2.0), (1192, 2.0, 2.0), (1240, 2.0, 1.0), (1288, 2.0, 0.0), (1336, 1.0, 0.0), (1384, 0.0, 0.0)]);
        for movement in [Movement::Normal, Movement::Dance] {
            let s = AutoplaySettings { movement, dance: DanceAmount::Wild, aim: Aim::Flow, ..Default::default() };
            let r = generate(&map, &s, 240, &dice()).unwrap();
            // the corner notes (0,2), (2,2), (2,0): the turn is spread along
            // the arc instead of taken on the note (a ring of unit hops turns
            // 45 degrees per hop whatever the path; the question is where)
            for t in [1096.0, 1192.0, 1288.0] {
                let on = turn_at(&r, t);
                assert!(on < 45.0, "{movement:?}: the corner at {t} turns {on} degrees on the note (a hard corner is 65)");
            }
            // the corner is cut, but never by more than the cap, and the hit
            // fires where the cursor passes the note, inside its hit area
            for n in &map.notes[1..map.notes.len() - 1] {
                let (wx, wy) = world(n);
                let flag = r.frames.iter().find(|f| f.hit && (f.ms - n.time_ms as f64).abs() <= 30.0).expect("a flag near the note");
                let d = ((flag.x - wx).powi(2) + (flag.y - wy).powi(2)).sqrt();
                assert!(d <= 0.31, "{movement:?}: note at {} flagged {d} from its centre", n.time_ms);
            }
            let corner = world(&map.notes[2]);
            let closest = r.frames.iter().filter(|f| f.ms > 1070.0 && f.ms < 1122.0).map(|f| ((f.x - corner.0).powi(2) + (f.y - corner.1).powi(2)).sqrt()).fold(f32::INFINITY, f32::min);
            assert!(closest > 0.1, "{movement:?}: the corner is not cut (closest {closest})");
            assert_every_note_covered(&map, &r, &format!("ring {movement:?}"));
        }
    }

    /// One steady speed where the hops are equal, and a long hop among
    /// them does not double the speed for exactly that hop: the change is
    /// spread, as a player's would be.
    #[test]
    fn a_stream_keeps_one_speed_and_spreads_the_change_for_a_long_hop() {
        // four one-unit hops, a two-unit hop, four one-unit hops, 48 ms each
        let map = map_with(&[
            (1000, 0.0, 0.0), (1048, 0.0, 1.0), (1096, 0.0, 2.0), (1144, 1.0, 2.0), (1192, 2.0, 2.0),
            (1240, 2.0, 0.0), (1288, 1.0, 0.0), (1336, 1.0, 1.0), (1384, 1.0, 2.0), (1432, 0.0, 2.0),
        ]);
        for aim in [Aim::Centre, Aim::Flow] {
            let s = AutoplaySettings { movement: Movement::Normal, aim, ..Default::default() };
            let r = generate(&map, &s, 240, &dice()).unwrap();
            let steps: Vec<(f64, f32)> = r
                .frames
                .iter()
                .filter(|f| !f.hit && f.ms >= 1000.0 && f.ms <= 1432.0)
                .collect::<Vec<_>>()
                .windows(2)
                .map(|w| (w[1].ms, ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt()))
                .collect();
            let ratio_max = steps.windows(2).map(|w| w[1].1 / w[0].1.max(1e-6)).fold(0.0f32, f32::max);
            assert!(ratio_max < 1.6, "{aim:?}: the speed jumps by {ratio_max}x between two frames (a hop planned on its own doubles it)");
            // the equal-hop stretches away from the long hop run at one speed
            // (after the ramp in from the centre, and clear of the long hop's)
            let early: Vec<f32> = steps.iter().filter(|(t, _)| *t > 1090.0 && *t < 1140.0).map(|s| s.1).collect();
            let late: Vec<f32> = steps.iter().filter(|(t, _)| *t > 1300.0 && *t < 1425.0).map(|s| s.1).collect();
            for (name, v) in [("early", early), ("late", late)] {
                let (lo, hi) = v.iter().fold((f32::INFINITY, 0.0f32), |(lo, hi), &x| (lo.min(x), hi.max(x)));
                assert!(hi / lo < 1.08, "{aim:?} {name} stretch: speed varies from {lo} to {hi} per frame");
            }
            assert_every_note_covered(&map, &r, &format!("long hop {aim:?}"));
        }
    }

    /// Back and forth on a line (a jack at 1/32) is a narrow loop: the
    /// cursor turns around each note on a small arc, never stops dead and
    /// snaps back, and still passes within a tenth of a unit of the note.
    #[test]
    fn a_jack_is_a_loop_not_a_dead_stop() {
        let map = map_with(&[(1000, 0.0, 1.0), (1048, 1.0, 1.0), (1096, 0.0, 1.0), (1144, 1.0, 1.0), (1192, 0.0, 1.0), (1240, 1.0, 1.0), (1288, 0.0, 1.0), (1336, 1.0, 1.0)]);
        for aim in [Aim::Centre, Aim::Flow] {
            let s = AutoplaySettings { movement: Movement::Normal, aim, ..Default::default() };
            let r = generate(&map, &s, 240, &dice()).unwrap();
            for t in [1096.0, 1144.0, 1192.0, 1240.0] {
                let turn = turn_at(&r, t);
                assert!(turn < 120.0, "{aim:?}: the cursor turns {turn} degrees on the note at {t}: a dead stop");
            }
            // it loops beside the line, by a visible but small amount, on both sides
            let (above, below) = r.frames.iter().filter(|f| !f.hit && f.ms > 1090.0 && f.ms < 1250.0).fold((0.0f32, 0.0f32), |(a, b), f| (a.max(f.y), b.min(f.y)));
            assert!(above > 0.08 && below < -0.08 && above < 0.4 && below > -0.4, "{aim:?}: the loop spans {below}..{above} beside the line");
            // it comes close to every note: through the centre with the
            // centre aim, within a tenth of a unit with the flow aim
            let near = if aim == Aim::Flow { path::TIP_CUT + 0.02 } else { 0.02 };
            for n in &map.notes[1..map.notes.len() - 1] {
                let closest = passes_within(&r, n.time_ms as f64, world(n));
                assert!(closest <= near, "{aim:?}: note at {} passed {closest} from its centre", n.time_ms);
            }
            assert_every_note_covered(&map, &r, &format!("jack {aim:?}"));
        }
    }

    /// With the centre aim the stream is still one flow (steady speed,
    /// rounded through the neighbours' direction, the jump in ramps), but
    /// the cursor goes through the centre of every note.
    #[test]
    fn the_centre_aim_passes_through_every_stream_note() {
        let map = map_with(&[(1000, 0.0, 0.0), (1048, 0.0, 1.0), (1096, 0.0, 2.0), (1144, 1.0, 2.0), (1192, 2.0, 2.0), (1240, 2.0, 1.0), (1288, 2.0, 0.0), (1336, 1.0, 0.0), (1384, 0.0, 0.0)]);
        for movement in [Movement::Normal, Movement::Dance] {
            let s = AutoplaySettings { movement, aim: Aim::Centre, ..Default::default() };
            let r = generate(&map, &s, 240, &dice()).unwrap();
            for n in &map.notes[1..map.notes.len() - 1] {
                let closest = passes_within(&r, n.time_ms as f64, world(n));
                assert!(closest <= 0.02, "{movement:?}: note at {} passed {closest} from its centre", n.time_ms);
            }
            for t in [1096.0, 1192.0, 1288.0] {
                let on = turn_at(&r, t);
                assert!(on < 80.0, "{movement:?}: the corner at {t} turns {on} degrees: a kink, not a curve");
            }
            assert_every_note_covered(&map, &r, &format!("centre ring {movement:?}"));
        }
    }

    /// A chord inside a stream: the two notes share one spot that is in
    /// both hit areas with almost no room to spare, so the sweep passes it
    /// exactly, on time, and both notes are covered at their flags. (The
    /// matcher cannot tell: its first pass is timing only.)
    #[test]
    fn a_chord_inside_a_stream_is_passed_exactly_and_covers_both_notes() {
        let map = map_with(&[(1000, 0.0, 0.0), (1048, 1.0, 0.0), (1096, 2.0, 0.0), (1144, 2.0, 1.0), (1144, 2.0, 2.0), (1192, 1.0, 2.0), (1240, 0.0, 2.0), (1288, 0.0, 1.0)]);
        for (movement, aim) in [(Movement::Normal, Aim::Flow), (Movement::Dance, Aim::Flow), (Movement::Dance, Aim::Centre)] {
            let s = AutoplaySettings { movement, aim, ..Default::default() };
            let r = generate(&map, &s, 240, &dice()).unwrap();
            assert_every_note_covered(&map, &r, &format!("chord in stream {movement:?} {aim:?}"));
            let mid = ((world(&map.notes[3]).0 + world(&map.notes[4]).0) / 2.0, (world(&map.notes[3]).1 + world(&map.notes[4]).1) / 2.0);
            let at = r.frames.iter().find(|f| f.hit && (f.ms - 1144.0).abs() < 1e-6).expect("the chord's flag at its time");
            assert!((at.x - mid.0).abs() < 0.01 && (at.y - mid.1).abs() < 0.01, "{movement:?}: the chord's spot is passed at ({},{}) instead of exactly", at.x, at.y);
        }
    }

    /// Two streams with one jump between them are one sweep, and a map of
    /// many streams sweeps every one of them (the runs never overlap, so
    /// none is skipped).
    #[test]
    fn every_stream_in_a_map_is_swept() {
        let mut notes: Vec<(i64, f32, f32)> = Vec::new();
        let mut t = 1000;
        for block in 0..6 {
            for k in 0..5 {
                notes.push((t, (k % 3) as f32, (block % 3) as f32));
                t += 48;
            }
            t += if block % 2 == 0 { 190 - 48 } else { 400 - 48 };
        }
        let map = map_with(&notes);
        let points: Vec<Waypoint> = map
            .notes
            .iter()
            .map(|n| {
                let (x, y) = world(n);
                Waypoint { t_ms: n.time_ms as f64, x, y, exact: false }
            })
            .collect();
        let p = plan(&points, &AutoplaySettings::default(), &field_for(&map), &dice());
        let kinds: Vec<path::SegmentKind> = p.segments.iter().map(|s| s.kind).collect();
        let sweep = kinds.iter().filter(|k| **k == path::SegmentKind::Sweep).count();
        // 30 notes: everything but the first jump from the centre and the
        // final ease is swept, including the three 190 ms jumps between
        // blocks and the jumps into and out of the 400 ms gaps
        assert!(sweep >= 29, "{sweep} sweep pieces of {} segments: {kinds:?}", kinds.len());
        let last_note_piece = &p.segments[p.segments.len() - 2];
        assert_eq!(last_note_piece.kind, path::SegmentKind::Sweep, "the last stream is not swept");
    }

    /// The jump into a stream accelerates into it instead of switching
    /// speed on the first note: no frame-to-frame speed jump around it.
    #[test]
    fn the_jump_into_a_stream_accelerates_instead_of_switching_speed() {
        let map = map_with(&[(1000, 1.0, 2.0), (1190, 0.0, 0.0), (1238, 0.0, 1.0), (1286, 0.0, 2.0), (1334, 1.0, 2.0), (1382, 2.0, 2.0), (1572, 2.0, 0.0)]);
        for (movement, aim) in [(Movement::Normal, Aim::Centre), (Movement::Dance, Aim::Centre), (Movement::Normal, Aim::Flow)] {
            let s = AutoplaySettings { movement, aim, ..Default::default() };
            let r = generate(&map, &s, 240, &dice()).unwrap();
            let steps: Vec<f32> = r
                .frames
                .iter()
                .filter(|f| !f.hit && f.ms >= 1100.0 && f.ms <= 1300.0)
                .collect::<Vec<_>>()
                .windows(2)
                .map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt())
                .collect();
            let worst = steps.windows(2).map(|w| w[1] / w[0].max(1e-6)).fold(0.0f32, f32::max);
            assert!(worst < 1.25, "{movement:?}: the speed jumps by {worst}x between two frames around the stream's first note");
            let (first, last) = (steps[0], steps[steps.len() - 1]);
            assert!(last > first * 1.15, "{movement:?}: no acceleration into the stream ({first} to {last} per frame)");
        }
    }

    /// Streams of every shape register every note: pseudo-random maps with
    /// gaps from a chord's spacing to a jump, hops from half a unit to the
    /// field's diagonal, a little off the grid, at every speed mod and in
    /// both movements; and no frame leaves the field.
    #[test]
    fn random_streams_register_every_note_and_stay_in_the_field() {
        let mut seed: u64 = 0x5eed_1234_abcd_0001;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        let gaps = [20i64, 42, 48, 84, 95, 125, 190];
        for case in 0..48 {
            let mut notes: Vec<(i64, f32, f32)> = Vec::new();
            let mut t = 600i64;
            let (mut gx, mut gy) = (1.0f32, 1.0f32);
            for _ in 0..70 {
                t += gaps[(next() % gaps.len() as u32) as usize];
                let step = |v: f32, r: u32| -> f32 {
                    let d = match r % 5 {
                        0 => -1.0,
                        1 => 1.0,
                        2 => -2.0,
                        3 => 2.0,
                        _ => 0.0,
                    };
                    (v + d).clamp(0.0, 2.0)
                };
                gx = step(gx, next());
                gy = step(gy, next());
                let jitter = |r: u32| (r % 25) as f32 / 100.0 - 0.12;
                let (jx, jy) = if next() % 3 == 0 { (jitter(next()), jitter(next())) } else { (0.0, 0.0) };
                notes.push((t, gx + jx, gy + jy));
                // now and then a chord: a twin at the same time one unit
                // over (merged to one spot), or two units over (swept)
                if next() % 9 == 0 {
                    let far = if next() % 2 == 0 { 1.0 } else { 2.0 };
                    let tx = if gx + far <= 2.0 { gx + far } else { gx - far };
                    if (0.0..=2.0).contains(&tx) {
                        notes.push((t, tx, gy));
                    }
                }
            }
            let map = map_with(&notes);
            let speed = [0.8f32, 1.0, 1.5, 2.0][case % 4];
            let movement = if case % 2 == 0 { Movement::Normal } else { Movement::Dance };
            let aim = if (case / 2) % 2 == 0 { Aim::Flow } else { Aim::Centre };
            let s = AutoplaySettings { movement, aim, speed_mod: speed, ..Default::default() };
            let r = generate(&map, &s, 120, &dice()).unwrap();
            assert_every_note_covered(&map, &r, &format!("case {case} ({movement:?} {aim:?} at {speed}), notes {notes:?}"));
            let bound = 1.12 + (0.5 - rhythia_sim::hitreg::CURSOR_EDGE_INSET);
            for f in &r.frames {
                assert!(f.x.abs() <= bound + 1e-4 && f.y.abs() <= bound + 1e-4, "case {case}: {f:?}");
            }
        }
    }

    /// A map that opens with a stream: the cursor waits in the centre and
    /// winds up into the stream, instead of crawling for a second and
    /// switching pace on the first note.
    #[test]
    fn a_map_that_opens_with_a_stream_winds_up_from_the_centre() {
        let map = map_with(&[(1000, 0.0, 1.0), (1048, 1.0, 1.0), (1096, 2.0, 1.0), (1144, 2.0, 2.0), (1192, 1.0, 2.0), (1240, 0.0, 2.0)]);
        for aim in [Aim::Centre, Aim::Flow] {
            let s = AutoplaySettings { movement: Movement::Normal, aim, ..Default::default() };
            let r = generate(&map, &s, 240, &dice()).unwrap();
            let at = |t: f64| r.frames.iter().filter(|f| !f.hit).min_by(|a, b| (a.ms - t).abs().partial_cmp(&(b.ms - t).abs()).unwrap()).unwrap();
            let early = at(500.0);
            assert!(early.x.abs() < 0.1 && early.y.abs() < 0.1, "{aim:?}: left the centre long before the stream: {early:?}");
            let steps: Vec<f32> = r
                .frames
                .iter()
                .filter(|f| !f.hit && f.ms >= 800.0 && f.ms <= 1140.0)
                .collect::<Vec<_>>()
                .windows(2)
                .map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt())
                .collect();
            // from rest the pace can only grow by a fraction of itself per
            // frame, so the measure is the change of the step itself: a
            // smooth wind-up over a tenth of a second changes it by a
            // thirtieth of the stream's step per frame, a switch on the
            // first note by the whole step at once
            let in_stream: f32 = steps[steps.len() - 6..].iter().sum::<f32>() / 6.0;
            let worst = steps.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
            assert!(worst < 0.15 * in_stream, "{aim:?}: the step changes by {worst} between two frames on the way into the stream (the stream's step is {in_stream})");
            let at_note: f32 = steps[steps.len() - 36..steps.len() - 30].iter().sum::<f32>() / 6.0;
            assert!(at_note > 0.75 * in_stream, "{aim:?}: not up to pace at the first note ({at_note} vs {in_stream} per frame)");
            assert_every_note_covered(&map, &r, &format!("opening stream {aim:?}"));
        }
    }

    /// The jumps into and out of a stream belong to the stream's sweep: the
    /// cursor arrives at the first note already heading the way it leaves,
    /// and leaves the last note the way it arrived, instead of hooking in
    /// from a dance arc and turning on the spot. Measured on the plan
    /// itself, a millisecond either side of the note.
    #[test]
    fn the_jump_into_a_stream_arrives_heading_the_streams_way() {
        let map = map_with(&[(1000, 1.0, 2.0), (1190, 0.0, 0.0), (1238, 0.0, 1.0), (1286, 0.0, 2.0), (1334, 1.0, 2.0), (1524, 2.0, 0.0)]);
        let points: Vec<Waypoint> = map
            .notes
            .iter()
            .map(|n| {
                let (x, y) = world(n);
                Waypoint { t_ms: n.time_ms as f64, x, y, exact: false }
            })
            .collect();
        let field = field_for(&map);
        let angle = |u: (f32, f32), v: (f32, f32)| {
            let dot = u.0 * v.0 + u.1 * v.1;
            let len = (u.0 * u.0 + u.1 * u.1).sqrt() * (v.0 * v.0 + v.1 * v.1).sqrt();
            (dot / len.max(1e-12)).clamp(-1.0, 1.0).acos().to_degrees()
        };
        for movement in [Movement::Dance, Movement::Normal] {
            let s = AutoplaySettings { movement, ..Default::default() };
            let p = plan(&points, &s, &field, &dice());
            let step = |from: f64, to: f64| {
                let (a, b) = (sample(&p, from, &field), sample(&p, to, &field));
                (b.0 - a.0, b.1 - a.1)
            };
            let turn = angle(step(1189.0, 1190.0), step(1190.0, 1191.0));
            assert!(turn < 30.0, "{movement:?}: the cursor turns {turn:.0} degrees at the stream's first note");
            let turn_out = angle(step(1333.0, 1334.0), step(1334.0, 1335.0));
            assert!(turn_out < 30.0, "{movement:?}: the cursor turns {turn_out:.0} degrees at the stream's last note");
        }
    }

    /// A chord is two waypoints half a millisecond apart on the same spot;
    /// that is not a stream, and the jump into it keeps its dance arc.
    #[test]
    fn a_chord_does_not_pull_the_jump_before_it_into_a_stream_sweep() {
        let map = map_with(&[(1000, 0.0, 1.0), (1190, 2.0, 1.0), (1190, 2.0, 2.0), (1380, 0.0, 1.0)]);
        let s = AutoplaySettings { movement: Movement::Dance, dance: DanceAmount::Medium, ..Default::default() };
        let r = generate(&map, &s, 120, &dice()).unwrap();
        let a = world(&map.notes[0]);
        let b = ((world(&map.notes[1]).0 + world(&map.notes[2]).0) / 2.0, (world(&map.notes[1]).1 + world(&map.notes[2]).1) / 2.0);
        let off = max_off_chord(&r, 1000.0, 1190.0, a, b);
        assert!(off > 0.3, "the jump into the chord arcs by only {off}");
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
    fn the_banner_is_on_unless_switched_off_and_old_settings_keep_it() {
        assert!(AutoplaySettings::default().banner);
        let old: AutoplaySettings = serde_json::from_str("{}").unwrap();
        assert!(old.banner, "a settings file from before the switch keeps the ticker");
        let off: AutoplaySettings = serde_json::from_str(r#"{"banner":false}"#).unwrap();
        assert!(!off.banner);
    }

    #[test]
    fn the_ticker_goes_with_the_banner() {
        let on = AutoplaySettings::default();
        assert_eq!(ticker_for(&on), Some(ticker_text(&on)));
        let off = AutoplaySettings { banner: false, ..Default::default() };
        assert_eq!(ticker_for(&off), None);
    }

    #[test]
    fn the_aim_defaults_to_the_centre_and_reads_both_words() {
        assert_eq!(AutoplaySettings::default().aim, Aim::Centre);
        let s: AutoplaySettings = serde_json::from_str(r#"{"movement":"dance","aim":"flow"}"#).unwrap();
        assert_eq!(s.aim, Aim::Flow);
        let s: AutoplaySettings = serde_json::from_str(r#"{"movement":"normal"}"#).unwrap();
        assert_eq!(s.aim, Aim::Centre, "a settings file from before the aim existed reads as the centre");
    }

    #[test]
    fn settings_normalise_to_the_grids() {
        let s = AutoplaySettings { speed_mod: 1.337, rest_secs: 9.9, ..Default::default() }.normalized();
        assert!((s.speed_mod - 1.35).abs() < 1e-6 && (s.rest_secs - 8.0).abs() < 1e-9);
        let s = AutoplaySettings { speed_mod: 0.1, rest_secs: 0.0, ..Default::default() }.normalized();
        assert!((s.speed_mod - 0.8).abs() < 1e-6 && (s.rest_secs - 2.0).abs() < 1e-9);
    }
}
