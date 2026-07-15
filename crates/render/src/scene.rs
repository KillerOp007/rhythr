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
    /// Camera sway strength: the world shifts by −cursor·parallax.
    pub parallax: f32,
    /// VR-style camera (`SpinCamera`): the view rotates to keep the cursor
    /// screen-centred, so the world pans around a fixed centre dot.
    pub spin: bool,
    /// Overall note opacity (`NoteOpacity`).
    pub note_opacity: f32,
    /// HalfGhost mod: notes fade toward half opacity near the hit plane.
    pub half_ghost: bool,
    /// near/far clip planes (raylib defaults).
    pub near: f32,
    pub far: f32,
}

/// Mesh half-extent (±1) mapped to this many world units at NoteScale 1.0;
/// with NoteScale ~0.9 this leaves the game's ~10% gap between cells.
const BASE_NOTE_SCALE: f32 = 0.5;

impl Default for SceneParams {
    fn default() -> Self {
        SceneParams {
            fov_y_deg: 70.0,
            // Fixed camera distance (a game constant); calibrated against the
            // real replay footage so the corner-bracket playfield fills ~51%
            // of the frame height at fov 75.
            eye_z: 3.25,
            note_radius: BASE_NOTE_SCALE * 0.9,
            spawn_depth: 12.0,
            approach_rate: 24.5,
            fade_length: 0.5,
            parallax: 0.0,
            spin: false,
            note_opacity: 1.0,
            half_ghost: false,
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
            // The config slider (0..~10) scales a small sway factor.
            parallax: c.parallax * 0.003,
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
    pub fn view_proj(&self, aspect: f32, cursor: (f32, f32)) -> Mat4 {
        let proj = Mat4::perspective_rh(self.fov_y_deg.to_radians(), aspect, self.near, self.far);
        let view = if self.spin {
            // SpinCamera: the camera rotates to keep the cursor dead centre —
            // the world pans around it like looking through a VR headset.
            let eye = Vec3::new(0.0, 0.0, self.eye_z);
            let target = Vec3::new(cursor.0, cursor.1, 0.0);
            Mat4::look_at_rh(eye, target, Vec3::Y)
        } else {
            // Camera sits in front of the hit plane looking toward −z, swayed
            // opposite to the cursor so the field parallaxes as the player
            // aims.
            let sway = Vec3::new(-cursor.0 * self.parallax, -cursor.1 * self.parallax, 0.0);
            let eye = Vec3::new(0.0, 0.0, self.eye_z) + sway;
            let target = Vec3::new(sway.x, sway.y, 0.0);
            Mat4::look_at_rh(eye, target, Vec3::Y)
        };
        proj * view
    }

    /// Visible approach window in ms (spawn_depth / approach_rate · 1000).
    pub fn approach_ms(&self) -> f32 {
        self.spawn_depth / self.approach_rate * 1000.0
    }

    /// Half-size of the playfield border, just outside the ±1 note grid.
    /// The factor is pixel-calibrated against the game's bracket box (the
    /// health bar spanning it measures 773px at 1440p ↔ 1.3395 world units).
    pub fn playfield_half(&self) -> f32 {
        1.0 + self.note_radius * 0.73
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
    /// [`HALFGHOST_FLOOR`]/[`HALFGHOST_CURVE`].
    pub fn note_opacity(&self, depth: f32) -> f32 {
        let fade_in_len = (self.spawn_depth * self.fade_length).max(1e-3);
        let fade_in = ((self.spawn_depth - depth) / fade_in_len)
            .clamp(0.0, 1.0)
            .powf(1.3);
        let mut alpha = fade_in;
        if self.half_ghost {
            let start = 12.0 / 50.0 * self.approach_rate;
            let end = 3.0 / 50.0 * self.approach_rate;
            let t = ((depth - end) / (start - end))
                .clamp(0.0, 1.0)
                .powf(HALFGHOST_CURVE);
            let fade_out = HALFGHOST_FLOOR + t * (1.0 - HALFGHOST_FLOOR);
            alpha = alpha.min(fade_out);
        }
        alpha * self.note_opacity
    }
}

/// Residual opacity a HalfGhost note keeps at/after the fade-out end. SS+'s
/// documented default is 0.20, but the player's own footage measures ~0.065
/// on a near note (a clean, scale-independent colour read), so we match that.
pub const HALFGHOST_FLOOR: f32 = 0.065;

/// Curvature of the HalfGhost fade-out. `> 1` keeps the note bright through
/// the far part of the window and pulls opacity down sharply as it nears the
/// plane; 2.0 fits the footage's mid-window point (~48% at ~180 ms).
pub const HALFGHOST_CURVE: f32 = 2.0;

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
        let vp = p.view_proj(16.0 / 9.0, (0.0, 0.0));
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
        let vp = p.view_proj(1.0, (0.0, 0.0));
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
        assert!((d - 0.2 * 24.5).abs() < 1e-3);
        // Just spawned at the visible-window edge (~490 ms ahead).
        assert!(p
            .note_depth(1000.0, 1000.0 - p.approach_ms() as f64 + 1.0)
            .is_some());
        // Past its hit time, or not yet spawned.
        assert_eq!(p.note_depth(1000.0, 1001.0), None);
        assert_eq!(p.note_depth(1000.0, 400.0), None);
    }

    #[test]
    fn fade_is_full_after_first_half_then_ramps_to_zero() {
        let p = SceneParams::default(); // spawn 12, fade_length 0.5
        assert_eq!(p.note_opacity(0.0), 1.0);
        assert_eq!(p.note_opacity(6.0), 1.0); // full by depth 6
        assert!(p.note_opacity(9.0) < 1.0 && p.note_opacity(9.0) > 0.0);
        assert_eq!(p.note_opacity(12.0), 0.0); // gone at spawn distance
    }

    #[test]
    fn spin_camera_keeps_the_cursor_screen_centred() {
        let p = SceneParams {
            spin: true,
            ..SceneParams::default()
        };
        for cursor in [(0.0, 0.0), (-1.0, 0.55), (1.3, -0.9)] {
            let vp = p.view_proj(16.0 / 9.0, cursor);
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
}
