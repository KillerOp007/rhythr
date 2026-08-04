//! The playfield scene: a real 3D perspective camera (matching the game's
//! MVP + FOV model, not a flat approximation), grid→world placement, and
//! the note approach animation.
//!
//! Coordinate model (world units == cursor units, so the cursor lands on
//! the note it hits): the game places grid index X∈{0,1,2} at world (X−1),
//! one cell per world unit. Verified empirically against the test replays —
//! at hit-flag frames the recorded cursor sits at ~±0.85 for edge cells,
//! i.e. inside the ±1 note (the note's world half-width covers the gap).
//!
//!   * grid (gx,gy) ∈ {0,1,2}² → world (x,y) = ((gx−1)·S, (1−gy)·S),
//!     S = [`GRID_SPACING`]. Grid centre (1,1) is the origin; +y is up
//!     (grid y grows downward, hence the flip).
//!   * the cursor's recorded (x,y) are already world units.
//!   * the hit plane is z = 0; a note approaching with depth d sits at
//!     z = −d (farther from the camera, so smaller on screen).
//!
//! Constants are tunable and get calibrated against real in-game frames.

use glam::{Mat4, Vec3};

/// World spacing between adjacent grid cells. The game places grid index
/// X∈{0,1,2} at world (X−1), so one cell == one world unit; outer notes sit
/// at ±1. (The recorded cursor sits at ~±0.85 on a hit — inside the note,
/// not at its centre — because of the hitbox size and aim bias.)
pub const GRID_SPACING: f32 = 1.0;

/// The game clamps the VISIBLE cursor to the field edge minus a fixed
/// inset (Cursor.gd: `edgec = 0.13125` cells) — the recorded positions go
/// further out, but the player never sees that. ±1.36875 on a normal
/// grid; hardrock widens it with the grid (empirically +0.15, i.e. the
/// margin stays put while the outer cell centres move out). One source
/// of truth, shared with hit attribution.
pub const CURSOR_EDGE_INSET: f32 = rhythia_sim::hitreg::CURSOR_EDGE_INSET;

/// Half-width of the game's true hit area — one source of truth, shared
/// with hit attribution in the sim crate.
pub const HITBOX_HALF: f32 = rhythia_sim::hitreg::HITBOX_HALF;

/// Camera / approach parameters. Defaults are starting points pinned to the
/// game's config (FOV 70) and the reference footage; `frame` calibration
/// refines them.
#[derive(Debug, Clone, Copy)]
pub struct SceneParams {
    /// Vertical field of view in degrees (game config `CameraFov`).
    pub fov_y_deg: f32,
    /// Camera distance from the hit plane, in world units. The game keeps
    /// this as a fixed constant chosen so the ±1 grid fills the FOV; the
    /// exact value is calibrated against real frames (~1.4–2.0).
    pub eye_z: f32,
    /// Note world half-width (baseScale·NoteScale ≈ 0.5·0.9). Meshes are
    /// normalised to ±1, so this is the scale applied to them directly.
    pub note_radius: f32,
    /// World depth a note spawns at (`SpawnDistance`, config 12).
    pub spawn_depth: f32,
    /// Grid units the note travels per second of song time (`ApproachRate`,
    /// config 24.5). Visible window = spawn_depth / approach_rate ≈ 490 ms.
    pub approach_rate: f32,
    /// Fraction of the approach over which a note fades in (`FadeLength`,
    /// config 0.5 → full opacity once depth ≤ spawn_depth·(1−FadeLength)).
    pub fade_length: f32,
    /// Camera sway strength: the camera moves toward the cursor by
    /// cursor·parallax, the way the game does it.
    pub parallax: f32,
    /// VR-style camera (`SpinCamera`): the view rotates to keep the cursor
    /// screen-centred, so the world pans around a fixed centre dot.
    pub spin: bool,
    /// Overall note opacity (`NoteOpacity`).
    pub note_opacity: f32,
    /// HalfGhost mod: notes fade toward a fifth opacity near the hit plane.
    pub half_ghost: bool,
    /// Ghost mod: notes fade to NOTHING before the plane. Takes precedence
    /// over half_ghost, as it does in the game.
    pub ghost: bool,
    /// Nearsighted mod: notes stay invisible until late in the approach.
    pub nearsighted: bool,
    /// Grid half-extent of the playfield (1.0; hardrock widens it to
    /// [`crate::mods::HARDROCK_GRID_SCALE`]). Note positions are already
    /// transformed in the map; this only widens the border.
    pub grid_scale: f32,
    /// Replay speed already folded into `approach_rate` by [`apply_speed`]
    /// (1.0 until then). Depth thresholds that are real-world constants in
    /// the game — the HalfGhost fade zone — multiply it back out.
    ///
    /// [`apply_speed`]: SceneParams::apply_speed
    pub speed: f32,
    /// near/far clip planes (raylib defaults).
    pub near: f32,
    pub far: f32,
}

/// Mesh half-extent (±1) mapped to this many world units at NoteScale 1.0.
/// The game's own factor: `NoteManager.gd:345` scales the note mesh by
/// `0.45 * note_size * (note_hitbox_size / 1.14)`, and the hitbox term is
/// exactly 1 at its 1.140 default — so a note is 0.45 world units at
/// NoteScale 1.0, leaving the visible gap between adjacent cells.
const BASE_NOTE_SCALE: f32 = 0.45;

impl Default for SceneParams {
    fn default() -> Self {
        SceneParams {
            fov_y_deg: 70.0,
            // The gameplay camera's fixed distance from the hit plane,
            // MEASURED against the game rather than read out of it.
            //
            // The source says 3.75: song.tscn parks the Camera at 3.5 and
            // NoteManager.gd `do_half_lock` — the else-branch at :672, i.e.
            // everything that is not VR or free-cam — overwrites it every
            // frame. A screenshot of the real game says otherwise, and the
            // screenshot wins: with the same skin config at 2560x1440, its
            // border square measures 876 px. The border plane is 3.04 units
            // (song.tscn PlaneMesh id=30, no scale on the node) and the
            // border texture covers 99.6% of it, so at 3.75 the plane could
            // only project to 834 px — the visible border cannot be larger
            // than the plane it is painted on. Solving the same measurement
            // against our own render, which cancels the texture and the fov,
            // gives 3.53. 3.5 is the game's own scene value and sits 0.9%
            // from that, which is 8 px at 1440p.
            //
            // 3.25, what this was before, is ruled out just as clearly: it
            // would put the plane at 962 px, 9% above what was measured.
            eye_z: 3.5,
            note_radius: BASE_NOTE_SCALE,
            // Rhythia.gd:563/567 — the game's own note travel settings.
            spawn_depth: 40.0,
            approach_rate: 40.0,
            fade_length: 0.5,
            parallax: 0.0,
            spin: false,
            note_opacity: 1.0,
            half_ghost: false,
            ghost: false,
            nearsighted: false,
            grid_scale: 1.0,
            speed: 1.0,
            near: 0.01,
            far: 1000.0,
        }
    }
}

impl From<&crate::config::SkinConfig> for SceneParams {
    /// Builds camera/approach parameters from the player's own settings, so
    /// the render matches what they see in-game.
    fn from(c: &crate::config::SkinConfig) -> Self {
        let d = SceneParams::default();
        SceneParams {
            fov_y_deg: c.camera_fov,
            note_radius: BASE_NOTE_SCALE * c.note_scale,
            spawn_depth: c.spawn_distance,
            approach_rate: c.approach_rate,
            fade_length: c.fade_length,
            // The game's mapping, not a guess: NoteManager.gd:529 and :537
            // give `hlpower = 0.1 * parallax` and `hlm = 0.25`, and :541
            // moves the camera by `centeroff * hlpower * hlm`. `centeroff`
            // is `cursorpos - (1,-1,0)`, i.e. the cursor in exactly the
            // world units we use — so one config unit is 0.025.
            parallax: c.parallax * 0.025,
            spin: c.spin_camera,
            note_opacity: c.note_opacity,
            half_ghost: c.half_ghost,
            ..d
        }
    }
}

/// Maps a grid coordinate (as stored in the map, may be off-grid/quantum)
/// to its world position on the hit plane.
pub fn grid_to_world(gx: f32, gy: f32) -> (f32, f32) {
    ((gx - 1.0) * GRID_SPACING, (1.0 - gy) * GRID_SPACING)
}

impl SceneParams {
    /// View·projection matrix for a frame of the given pixel aspect ratio,
    /// with the camera swayed by the cursor position (parallax).
    ///
    /// `portrait` is decided by the caller from the OUTPUT frame, not this
    /// viewport: a ghost-split half of a 16:9 render is narrower than tall
    /// too, but must keep the landscape camera it always had.
    pub fn view_proj(&self, aspect: f32, portrait: bool, cursor: (f32, f32)) -> Mat4 {
        // Portrait frames keep the HORIZONTAL field of view of the usual
        // landscape render (fov_y is widened so fov_x stays put) — the
        // square playfield then fills the width instead of vanishing.
        let fov_y = if portrait && aspect < 1.0 {
            2.0 * ((self.fov_y_deg.to_radians() * 0.5).tan() / aspect).atan()
        } else {
            self.fov_y_deg.to_radians()
        };
        let proj = Mat4::perspective_rh(fov_y, aspect, self.near, self.far);
        let view = if self.spin {
            // SpinCamera: the camera rotates to keep the cursor dead centre —
            // the world pans around it like looking through a VR headset.
            let eye = Vec3::new(0.0, 0.0, self.eye_z);
            let target = Vec3::new(cursor.0, cursor.1, 0.0);
            Mat4::look_at_rh(eye, target, Vec3::Y)
        } else {
            // Camera sits in front of the hit plane looking toward −z and
            // slides TOWARD the cursor as the player aims — the game
            // translates the camera without re-aiming it
            // (NoteManager.gd:539-541 sets only `cam.transform.origin`),
            // which is what an unrotated look-at from the swayed position
            // reproduces.
            let sway = Vec3::new(cursor.0 * self.parallax, cursor.1 * self.parallax, 0.0);
            let eye = Vec3::new(0.0, 0.0, self.eye_z) + sway;
            let target = Vec3::new(sway.x, sway.y, 0.0);
            Mat4::look_at_rh(eye, target, Vec3::Y)
        };
        proj * view
    }

    /// Visible approach window in ms (spawn_depth / approach_rate · 1000).
    /// The game advances the note approach in REAL time while note times
    /// live in song time: under a speed mod one real approach-window's
    /// worth of notes covers speed× more song time. The renderer works in
    /// song time and then compresses by the speed, so the approach rate
    /// must shrink by the same factor for the on-screen approach DURATION
    /// to match the game's. (User-verified: identical play, same skin —
    /// without this the notes flew in 45% faster than in game at 1.45x.)
    pub fn apply_speed(&mut self, speed: f32) {
        let s = speed.clamp(0.25, 3.0);
        self.approach_rate /= s;
        self.speed = s;
    }

    pub fn approach_ms(&self) -> f32 {
        self.spawn_depth / self.approach_rate * 1000.0
    }

    /// Half-size of the playfield border, just outside the ±1 note grid.
    /// The factor is pixel-calibrated against the game's bracket box (the
    /// health bar spanning it measures 773px at 1440p ↔ 1.3395 world units).
    pub fn playfield_half(&self) -> f32 {
        // The game's border is a fixed 3.04-unit plane (song.tscn, Outer
        // PlaneMesh) around the 3.0 grid — half 1.52, NOT a function of
        // the note size. Edge notes (edge 1.45) stay inside with 0.07 of
        // air, exactly as on screen in the real client.
        self.grid_scale + 0.52
    }

    /// The game's hard bound for the visible cursor centre.
    pub fn cursor_bound(&self) -> f32 {
        self.grid_scale + (0.5 - CURSOR_EDGE_INSET)
    }

    /// Recorded cursor positions can leave the field; the game clamps the
    /// drawn cursor (and the camera that follows it) to the border.
    pub fn clamp_cursor(&self, c: (f32, f32)) -> (f32, f32) {
        let b = self.cursor_bound();
        (c.0.clamp(-b, b), c.1.clamp(-b, b))
    }

    /// Model matrix for a note's HIT AREA — the fixed square the game
    /// actually tests the cursor against, larger than the visual note.
    pub fn hitbox_model(&self, gx: f32, gy: f32, depth: f32) -> Mat4 {
        let (wx, wy) = grid_to_world(gx, gy);
        Mat4::from_translation(Vec3::new(wx, wy, -depth))
            * Mat4::from_scale(Vec3::splat(HITBOX_HALF))
    }

    /// Depth of a note at the given song time, or None if it is not on
    /// screen (already hit/passed, or not yet spawned). Matches the game:
    /// depth = (note_time − song_time)/1000 · ApproachRate.
    pub fn note_depth(&self, note_time_ms: f64, song_time_ms: f64) -> Option<f32> {
        let ahead_ms = (note_time_ms - song_time_ms) as f32;
        if ahead_ms < 0.0 {
            return None;
        }
        let depth = ahead_ms / 1000.0 * self.approach_rate;
        if depth > self.spawn_depth {
            None
        } else {
            Some(depth)
        }
    }

    /// Model matrix placing a normalised (±1) note mesh at its grid cell and
    /// approach depth, scaled to the note's world half-width.
    pub fn note_model(&self, gx: f32, gy: f32, depth: f32) -> Mat4 {
        let (wx, wy) = grid_to_world(gx, gy);
        Mat4::from_translation(Vec3::new(wx, wy, -depth))
            * Mat4::from_scale(Vec3::splat(self.note_radius))
    }

    /// Opacity of a note at the given approach depth (distance from the hit
    /// plane, in ApproachRate units), following the Sound Space Plus
    /// `NoteManager.gd` fade model (MIT) the Steam client inherited:
    ///
    /// * fade-in over the first `FadeLength` of the spawn distance, `^1.3`;
    /// * with HalfGhost, a fade-out over the same window SS+ uses — 12/50·AR
    ///   to 3/50·AR from the plane, a fixed 240 ms → 60 ms before the hit
    ///   because both distances scale with AR and the note travels AR
    ///   units/second;
    /// * `alpha = min(fade_in, fade_out) · NoteOpacity`.
    ///
    /// The fade-out floor and curvature are **calibrated to the player's own
    /// footage**, not SS+'s documented defaults: a near note measures ~6.5%
    /// opacity (not the 20% a base-0.8 fade gives), and the fade pulls in
    /// more sharply toward the plane (`^2.0`, gentle far → steep near). See
    /// [`HALFGHOST_FLOOR`]/[`FADE_CURVE`].
    pub fn note_opacity(&self, depth: f32) -> f32 {
        // Every threshold below is a DEPTH derived from the CONFIG approach
        // rate: the game's zones do not move under a speed mod, while
        // apply_speed divides ours, so undo that division here.
        let config_ar = self.approach_rate * self.speed;
        // linstep as the game defines it: 0 at `a`, 1 at `b`, whichever
        // side of the value they sit on.
        let linstep = |a: f32, b: f32, x: f32| ((x - a) / (b - a)).clamp(0.0, 1.0);

        // Fade in. Nearsighted replaces the normal spawn fade with a much
        // later one (NoteManager.gd:436-438).
        let (fi_start, fi_end) = if self.nearsighted {
            (30.0 / 50.0 * config_ar, 5.0 / 50.0 * config_ar)
        } else {
            (
                self.spawn_depth,
                self.spawn_depth * (1.0 - self.fade_length),
            )
        };
        let fade_in = if self.nearsighted || self.fade_length != 0.0 {
            linstep(fi_start, fi_end, depth).powf(FADE_CURVE)
        } else {
            1.0
        };

        // Fade out. Ghost wins over half-ghost, as in NoteManager.gd:425-434,
        // and its fade_out_base stays at the default 1, which is why a ghost
        // note reaches zero while a half-ghost one stops at a fifth.
        let fade_out = if self.ghost {
            let t = linstep(6.0 / 50.0 * config_ar, 18.0 / 50.0 * config_ar, depth);
            t.powf(FADE_CURVE)
        } else if self.half_ghost {
            let t = linstep(3.0 / 50.0 * config_ar, 12.0 / 50.0 * config_ar, depth);
            HALFGHOST_FLOOR + t.powf(FADE_CURVE) * (1.0 - HALFGHOST_FLOOR)
        } else {
            1.0
        };

        fade_in.min(fade_out) * self.note_opacity
    }
}

/// Residual opacity a HalfGhost note keeps at/after the fade-out end.
///
/// From the game: `NoteManager.gd:430-434` sets `fade_out_base = 0.8` for
/// half-ghost and `:144` computes
/// `(1 - fade_out_base) + linstep(..)^1.3 * fade_out_base`, so the floor is
/// exactly 0.20 and the shape is the one below.
///
/// This was 0.26, fitted to a footage reading of 72/255 (α≈0.28) at 70 ms
/// out. The source formula reaches 0.28 at ~90 ms instead, so the two
/// disagree by about 20 ms of timing rather than by shape — most likely the
/// timestamp or the approach rate assumed for that measurement. The source
/// wins here because it is unambiguous, and it is the same reasoning that
/// corrected the camera, the parallax and the colorset; if fresh half-ghost
/// footage ever contradicts it, measure again rather than nudging this.
pub const HALFGHOST_FLOOR: f32 = 0.20;

/// Curvature of the HalfGhost fade-out. `> 1` keeps the note bright through
/// the far part of the window and pulls opacity down as it nears the
/// plane; 1.3 fits the footage at −200/−120/−70 ms (α 0.80/0.44/0.28).
pub const FADE_CURVE: f32 = 1.3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_centre_is_origin_and_y_flips() {
        assert_eq!(grid_to_world(1.0, 1.0), (0.0, 0.0));
        let (x0, y0) = grid_to_world(0.0, 0.0);
        assert!(x0 < 0.0 && y0 > 0.0, "top-left grid → left and up");
        let (x2, y2) = grid_to_world(2.0, 2.0);
        assert!(x2 > 0.0 && y2 < 0.0, "bottom-right grid → right and down");
    }

    #[test]
    fn all_grid_notes_are_inside_the_frame_at_the_hit_plane() {
        // Every cell of the 3×3 grid must project inside the frustum on the
        // hit plane — the playfield fits the camera.
        let p = SceneParams::default();
        let vp = p.view_proj(16.0 / 9.0, false, (0.0, 0.0));
        for gy in 0..3 {
            for gx in 0..3 {
                let (x, y) = grid_to_world(gx as f32, gy as f32);
                let c = vp * glam::Vec4::new(x, y, 0.0, 1.0);
                assert!(c.w > 0.0, "grid ({gx},{gy}) behind camera");
                assert!(
                    c.x.abs() < c.w && c.y.abs() < c.w,
                    "grid ({gx},{gy}) outside frame"
                );
            }
        }
    }

    #[test]
    fn farther_notes_project_smaller() {
        // A fixed-size note should subtend less screen space as depth grows
        // — the essence of the perspective look.
        let p = SceneParams::default();
        let vp = p.view_proj(1.0, false, (0.0, 0.0));
        let screen_half = |depth: f32| {
            let c = vp * glam::Vec4::new(GRID_SPACING, 0.0, -depth, 1.0);
            (c.x / c.w).abs()
        };
        assert!(screen_half(0.0) > screen_half(6.0));
        assert!(screen_half(6.0) > screen_half(12.0));
    }

    #[test]
    fn approach_matches_game_model() {
        let p = SceneParams::default();
        // At its hit time a note is on the plane.
        assert_eq!(p.note_depth(1000.0, 1000.0), Some(0.0));
        // Depth = (ahead_ms/1000)·approach_rate.
        let d = p.note_depth(1000.0, 800.0).unwrap();
        assert!((d - 0.2 * 40.0).abs() < 1e-3);
        // The game's defaults show a note for a full second: spawn 40 units
        // travelled at 40 units/s.
        assert!((p.approach_ms() - 1000.0).abs() < 1e-3);
        // Just spawned at the visible-window edge.
        assert!(p
            .note_depth(1000.0, 1000.0 - p.approach_ms() as f64 + 1.0)
            .is_some());
        // Past its hit time, or not yet spawned.
        assert_eq!(p.note_depth(1000.0, 1001.0), None);
        assert_eq!(p.note_depth(1000.0, -100.0), None);
    }

    #[test]
    fn fade_is_full_after_first_half_then_ramps_to_zero() {
        let p = SceneParams::default(); // spawn 40, fade_length 0.5
        assert_eq!(p.note_opacity(0.0), 1.0);
        assert_eq!(p.note_opacity(20.0), 1.0); // full by half the approach
        assert!(p.note_opacity(30.0) < 1.0 && p.note_opacity(30.0) > 0.0);
        assert_eq!(p.note_opacity(40.0), 0.0); // gone at spawn distance
    }

    #[test]
    fn spin_camera_keeps_the_cursor_screen_centred() {
        let p = SceneParams {
            spin: true,
            ..SceneParams::default()
        };
        for cursor in [(0.0, 0.0), (-1.0, 0.55), (1.3, -0.9)] {
            let vp = p.view_proj(16.0 / 9.0, false, cursor);
            let c = vp * glam::Vec4::new(cursor.0, cursor.1, 0.0, 1.0);
            let ndc = (c.x / c.w, c.y / c.w);
            assert!(
                ndc.0.abs() < 1e-4 && ndc.1.abs() < 1e-4,
                "cursor {cursor:?} projected to {ndc:?}, expected centre"
            );
        }
    }

    #[test]
    fn halfghost_fades_out_to_calibrated_floor_near_the_plane() {
        // HalfGhost: fade-out from 12/50·AR (240 ms) to 3/50·AR (60 ms before
        // the hit), bottoming out at the footage-calibrated floor.
        let mut p = SceneParams {
            fade_length: 0.1, // quick fade-in so it doesn't mask the fade-out
            half_ghost: true,
            ..SceneParams::default()
        };
        p.approach_rate = 28.0; // → fade-out from depth 6.72 down to 1.68
        let far = 12.0 / 50.0 * p.approach_rate; // 6.72
        let near = 3.0 / 50.0 * p.approach_rate; // 1.68
                                                 // At/beyond the fade-out start the note is fully opaque.
        assert!((p.note_opacity(far) - 1.0).abs() < 1e-3);
        // At/inside the fade-out end it sits at the calibrated floor.
        assert!((p.note_opacity(near) - HALFGHOST_FLOOR).abs() < 1e-3);
        assert!((p.note_opacity(0.0) - HALFGHOST_FLOOR).abs() < 1e-3);
        // Monotonically dimmer as it approaches through the fade zone.
        let mid = (far + near) / 2.0;
        assert!(p.note_opacity(far) > p.note_opacity(mid));
        assert!(p.note_opacity(mid) > p.note_opacity(near));
    }

    #[test]
    fn speed_mods_keep_the_halfghost_fade_zone_in_place() {
        // The fade zone is a fixed pair of world depths in the game;
        // apply_speed slows the approach MOTION but must not pull the
        // fade thresholds toward the plane with it.
        let base = SceneParams {
            fade_length: 0.1,
            half_ghost: true,
            ..SceneParams::default()
        };
        let mut fast = base;
        fast.apply_speed(1.45);
        for depth in [0.5_f32, 1.47, 3.0, 5.88, 7.0] {
            assert!(
                (base.note_opacity(depth) - fast.note_opacity(depth)).abs() < 1e-5,
                "fade moved at depth {depth}"
            );
        }
    }
}
