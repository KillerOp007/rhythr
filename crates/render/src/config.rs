//! The player's visual settings, read from the game's `config.json` or an
//! exported `.rhs` skin (a zip whose "config" entry is that same JSON).
//!
//! Only the fields that affect a rendered replay are kept — everything about
//! gameplay, audio devices, online play etc. is ignored. This is the "adopt
//! the player's own skin" feature: the look should match what they see.

use std::io::Read;
use std::path::Path;

use crate::Error;

/// Note shape, derived from the `NoteSkin` texture name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteShape {
    /// Thin, sharp-cornered square outline (`thin`).
    Thin,
    /// Filled/outlined square, slightly rounded (`square`, `square 2/3`).
    Square,
    /// Rounded square (`rounded`, `rounded_fixed`).
    Rounded,
    /// Circle / other radial skins.
    Circle,
}

impl NoteShape {
    fn from_skin(name: &str) -> NoteShape {
        let n = name.to_ascii_lowercase();
        if n.contains("thin") {
            NoteShape::Thin
        } else if n.contains("circle") || n.contains("dot") {
            NoteShape::Circle
        } else if n.contains("round") {
            NoteShape::Rounded
        } else {
            NoteShape::Square
        }
    }

    /// (corner_radius, outline_fraction) for the rounded-rect note SDF, in
    /// mesh-local units (the note quad spans ±1).
    pub fn sdf_params(self) -> (f32, f32) {
        match self {
            NoteShape::Thin => (0.03, 0.16),
            NoteShape::Square => (0.08, 0.20),
            NoteShape::Rounded => (0.40, 0.24),
            NoteShape::Circle => (1.00, 0.26),
        }
    }
}

/// Converts an 8-bit sRGB colour plus a straight alpha into the linear RGBA
/// the shaders blend in (the render target re-encodes to sRGB on write).
///
/// The game blends its HUD in sRGB space (no sRGB framebuffer), so a config
/// opacity like 0.05 means `0.05 × 255` on screen. Our pipeline blends in
/// linear space; decoding the alpha through the sRGB curve reproduces the
/// game's result (verified: combo text at opacity 0.05 measures ~17/255 in
/// footage — linear alpha would give 62).
pub fn srgb8_to_linear(rgb: [u8; 3], a: f32) -> [f32; 4] {
    fn ch(s: f32) -> f32 {
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    [
        ch(rgb[0] as f32 / 255.0),
        ch(rgb[1] as f32 / 255.0),
        ch(rgb[2] as f32 / 255.0),
        ch(a.clamp(0.0, 1.0)),
    ]
}

/// The HUD/overlay settings, straight from the player's config. Each `*_*`
/// bool mirrors a `LeftPanel*Enabled` / `RightPanel*Enabled` / `*Enabled`
/// toggle, so an element turned off in-game is skipped in the render too.
#[derive(Debug, Clone)]
pub struct HudConfig {
    pub accuracy: bool,
    pub combo_ring: bool,
    pub grade: bool,
    pub pauses: bool,
    pub score: bool,
    pub points: bool,
    pub misses: bool,
    pub notes: bool,
    pub health_bar: bool,
    pub health_bar_color: [u8; 3],
    pub health_bar_alpha: f32,
    pub song_progress_bar: bool,
    pub song_progress_color: [u8; 3],
    pub song_progress_alpha: f32,
    pub combo_ring_color: [u8; 3],
    pub combo_ring_opacity: f32,
    pub playfield_combo_text: bool,
    pub combo_text_color: [u8; 3],
    pub combo_text_font_size: f32,
    pub combo_text_opacity: f32,
    pub combo_text_vpos_pct: f32,
    pub song_info: bool,
    /// Opacity of the red X flashed on a missed note (`MissEffectOpacity`).
    pub miss_effect_opacity: f32,
    /// Speed notation under the health bar. The game always shows it; the
    /// field exists so the desktop app can hide it per user override.
    pub speed_label: bool,
    /// Optional renderer extra (not a game element): timing error bar
    /// showing how early/late each hit was, danser-style.
    pub error_meter: ErrorMeter,
    /// Optional renderer extra: aim scatter showing where the cursor sat
    /// relative to each hit note's centre.
    pub aim_meter: ErrorMeter,
}

/// Placement/looks of an optional overlay meter. Positions are normalised
/// (0..1 of the frame); scale 1.0 is the design size.
#[derive(Debug, Clone, Copy)]
pub struct ErrorMeter {
    pub enabled: bool,
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub alpha: f32,
}

impl ErrorMeter {
    fn at(x: f32, y: f32) -> ErrorMeter {
        ErrorMeter {
            enabled: false,
            x,
            y,
            scale: 1.0,
            alpha: 0.9,
        }
    }
}

impl Default for HudConfig {
    fn default() -> Self {
        HudConfig {
            accuracy: true,
            combo_ring: true,
            grade: true,
            pauses: true,
            score: true,
            points: true,
            misses: true,
            notes: true,
            health_bar: true,
            health_bar_color: [0, 220, 80],
            health_bar_alpha: 0.86,
            song_progress_bar: true,
            song_progress_color: [220, 220, 225],
            song_progress_alpha: 0.86,
            combo_ring_color: [0, 200, 200],
            combo_ring_opacity: 1.0,
            playfield_combo_text: true,
            combo_text_color: [255, 255, 255],
            combo_text_font_size: 190.0,
            combo_text_opacity: 0.05,
            combo_text_vpos_pct: 25.0,
            song_info: true,
            miss_effect_opacity: 1.0,
            speed_label: true,
            // Defaults per user: the timing bar right under the speed/mods
            // notation, the aim scatter left of the combo ring; both off.
            error_meter: ErrorMeter::at(0.5, 0.88),
            aim_meter: ErrorMeter::at(0.15, 0.32),
        }
    }
}

/// The animated ambient background layers (tunnel, chevrons, rays, moving
/// grid). Parsed for every config so nothing is lost on import; their
/// rendering is still TODO (both reference configs have them disabled).
#[derive(Debug, Clone, Default)]
pub struct AmbientConfig {
    pub color: [f32; 3],
    pub accent: [f32; 3],
    pub accent_from_hit_note: bool,
    pub tunnel_enabled: bool,
    pub tunnel_opacity: f32,
    pub chevron_enabled: bool,
    pub chevron_opacity: f32,
    /// Pixel units at 1080p, as the sliders show them.
    pub chevron_gap: f32,
    pub chevron_large: f32,
    pub chevron_small: f32,
    pub chevron_width: f32,
    pub chevron_speed: f32,
    pub rays_enabled: bool,
    pub rays_opacity: f32,
    pub rays_intensity: f32,
    pub rays_width: f32,
    pub grid_enabled: bool,
    pub grid_opacity: f32,
    pub grid_cell_size: f32,
    pub grid_center_gap: f32,
    pub grid_fade_falloff: f32,
    pub grid_speed: f32,
    pub grid_parallax: f32,
}

/// One `BackgroundImages[]` layer from an exported skin. `placement` 0 is
/// screen space (centre/scale relative to the frame), 1 is world space (the
/// `space` rect in grid units around the playfield). Rotation is rare and
/// currently ignored.
#[derive(Debug, Clone)]
pub struct BackgroundLayer {
    pub bytes: Vec<u8>,
    pub fit: i64,
    pub placement: i64,
    pub center_x: f32,
    pub center_y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub flip_horizontal: bool,
    pub space_x: f32,
    pub space_y: f32,
    pub space_w: f32,
    pub space_h: f32,
    pub tint: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct SkinConfig {
    pub camera_fov: f32,
    pub approach_rate: f32,
    pub spawn_distance: f32,
    pub note_scale: f32,
    pub note_opacity: f32,
    pub fade_length: f32,
    pub parallax: f32,
    pub half_ghost: bool,
    pub note_shape: NoteShape,
    pub note_skin_name: String,
    pub border_skin_name: String,
    pub cursor_skin_name: String,
    pub colorset_name: String,
    pub border_color: [f32; 3],
    pub border_opacity: f32,
    pub cursor_color: [f32; 3],
    pub cursor_scale: f32,
    pub cursor_trail_enabled: bool,
    pub cursor_trail_opacity: f32,
    /// Lifetime of a trail dot in seconds (`CursorTrailFadeTimeSeconds`).
    pub cursor_trail_fade_secs: f32,
    /// Dot spacing scale (`CursorTrailSpacingMultiplier`); smaller = denser.
    pub cursor_trail_spacing: f32,
    /// Base trail colour, used when not inheriting from the cursor.
    pub cursor_trail_color: [f32; 3],
    /// Trail takes the cursor's colour (`CursorTrailInheritFromCursor`).
    pub cursor_trail_inherit: bool,
    /// Dots shrink as they age (`CursorTrailShrinkOverTime`).
    pub cursor_trail_shrink: bool,
    /// Colour stops (position 0..1 along the trail, sRGB rgb) when the
    /// gradient is enabled and non-empty; overrides the base colour.
    pub cursor_trail_gradient: Vec<(f32, [f32; 3])>,
    /// PNG bytes of the bundled note skin texture (from an imported `.rhs`),
    /// if the pack ships one. When present the renderer samples it instead
    /// of the procedural shape.
    pub note_texture: Option<Vec<u8>>,
    /// PNG bytes of the bundled playfield-border texture.
    pub border_texture: Option<Vec<u8>>,
    /// PNG bytes of the bundled cursor/trail texture.
    pub cursor_texture: Option<Vec<u8>>,
    /// Custom background layers from the skin's `BackgroundImages[]`,
    /// composited bottom-up behind the scene.
    pub background_images: Vec<BackgroundLayer>,
    /// Real note colours from the bundled `colorSet/*.txt`, in order. Empty
    /// when the pack references a built-in colorset (use the picker / a
    /// named-palette approximation instead).
    pub colorset: Vec<[f32; 3]>,
    /// HUD/overlay toggles and colours.
    pub hud: HudConfig,
    /// Mod icons (name without extension → PNG bytes) from the user's
    /// own installation, shown on the results screen.
    pub mod_icons: Vec<(String, Vec<u8>)>,
    /// The game's HUD font (Nunito ExtraBold) from the user's own
    /// installation; the bundled DejaVu is the fallback.
    pub hud_font: Option<Vec<u8>>,
    /// Playfield background colour (`BackgroundRed/Green/Blue`).
    pub background_color: [f32; 3],
    pub cursor_opacity: f32,
    /// Cursor rotation in degrees (`CursorRotation`).
    pub cursor_rotation_deg: f32,
    /// 3×3 cell separator lines on the playfield (`PlayfieldGrid*`).
    pub playfield_grid: bool,
    pub playfield_grid_color: [f32; 3],
    pub playfield_grid_opacity: f32,
    /// Line thickness in pixels at 1440p.
    pub playfield_grid_thickness: f32,
    /// Red edge vignette scaling with lost health (`FailVignetteOpacity`).
    pub fail_vignette_opacity: f32,
    /// HUD value text colour (`InterfaceTextColor*`).
    pub interface_text_color: [u8; 3],
    /// HUD value font scale (`InterfaceValuesFontSize`).
    pub interface_values_font_size: f32,
    /// Stat panel card colour/opacities and extra spacing (`Panel*`).
    pub panel_color: [u8; 3],
    pub panel_opacity: f32,
    pub panel_background_opacity: f32,
    pub panel_gap: f32,
    /// Missed notes fly past the hit plane instead of vanishing (`PushBack`).
    pub push_back: bool,
    /// Hides the whole HUD (`GameSceneDisableGui`).
    pub disable_gui: bool,
    /// VR-style camera: the cursor stays screen-centred and the world pans
    /// around it (`SpinCamera`).
    pub spin_camera: bool,
    /// Ambient background layers (parsed; rendering TODO).
    pub ambient: AmbientConfig,
}

impl Default for SkinConfig {
    /// The game's own defaults (used when a field is absent).
    fn default() -> Self {
        SkinConfig {
            camera_fov: 70.0,
            approach_rate: 24.5,
            spawn_distance: 12.0,
            note_scale: 0.9,
            note_opacity: 1.0,
            fade_length: 0.5,
            parallax: 0.0,
            half_ghost: false,
            note_shape: NoteShape::Square,
            note_skin_name: String::new(),
            border_skin_name: String::new(),
            cursor_skin_name: String::new(),
            colorset_name: String::new(),
            border_color: [1.0, 1.0, 1.0],
            border_opacity: 1.0,
            cursor_color: [1.0, 1.0, 1.0],
            cursor_scale: 1.0,
            cursor_trail_enabled: true,
            cursor_trail_opacity: 0.5,
            cursor_trail_fade_secs: 0.1,
            cursor_trail_spacing: 0.4,
            cursor_trail_color: [1.0, 1.0, 1.0],
            cursor_trail_inherit: true,
            cursor_trail_shrink: true,
            cursor_trail_gradient: Vec::new(),
            note_texture: None,
            background_images: Vec::new(),
            border_texture: None,
            cursor_texture: None,
            colorset: Vec::new(),
            hud: HudConfig::default(),
            mod_icons: Vec::new(),
            hud_font: None,
            background_color: [0.0, 0.0, 0.0],
            cursor_opacity: 1.0,
            cursor_rotation_deg: 0.0,
            playfield_grid: false,
            playfield_grid_color: [1.0, 1.0, 1.0],
            playfield_grid_opacity: 1.0,
            playfield_grid_thickness: 2.0,
            fail_vignette_opacity: 0.0,
            interface_text_color: [255, 255, 255],
            interface_values_font_size: 1.0,
            panel_color: [70, 1, 1],
            panel_opacity: 1.0,
            panel_background_opacity: 0.0,
            panel_gap: 0.0,
            push_back: false,
            disable_gui: false,
            spin_camera: false,
            ambient: AmbientConfig::default(),
        }
    }
}

type ZipCursor<'a> = zip::ZipArchive<std::io::Cursor<&'a [u8]>>;

/// Cap on any single decompressed `.rhs` entry (config text / PNG texture /
/// colorset). A skin pack is untrusted input; without this a zip bomb could
/// exhaust memory.
const MAX_RHS_ENTRY_BYTES: u64 = 64 << 20;

/// Reads a named zip entry's bytes, or None if it isn't present.
fn read_zip_entry(zip: &mut ZipCursor, name: &str) -> Result<Option<Vec<u8>>, Error> {
    match zip.by_name(name) {
        Ok(e) => {
            if e.size() > MAX_RHS_ENTRY_BYTES {
                return Err(Error::Config(format!(
                    "skin entry {name} too large ({} bytes)",
                    e.size()
                )));
            }
            let mut buf = Vec::with_capacity(e.size() as usize);
            // The header's `size` is untrusted — bound the actual read too.
            e.take(MAX_RHS_ENTRY_BYTES + 1)
                .read_to_end(&mut buf)
                .map_err(|err| Error::Config(format!("reading {name}: {err}")))?;
            if buf.len() as u64 > MAX_RHS_ENTRY_BYTES {
                return Err(Error::Config(format!("skin entry {name} too large")));
            }
            Ok(Some(buf))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(err) => Err(Error::Config(format!("reading {name}: {err}"))),
    }
}

/// Reads the first (usually only) entry whose name starts with `prefix`.
fn first_zip_entry_under(zip: &mut ZipCursor, prefix: &str) -> Result<Option<Vec<u8>>, Error> {
    let name = (0..zip.len()).find_map(|i| {
        let e = zip.by_index(i).ok()?;
        let n = e.name();
        // Skip the folder entry itself; take the first real file under it.
        (n.starts_with(prefix) && n.len() > prefix.len() && !n.ends_with('/'))
            .then(|| n.to_string())
    });
    match name {
        Some(n) => read_zip_entry(zip, &n),
        None => Ok(None),
    }
}

/// Parses a colorset `.txt` (one `#rrggbb` per line) to linear-ish RGB
/// triples in [0,1].
fn parse_colorset(text: &str) -> Vec<[f32; 3]> {
    text.lines()
        .filter_map(|line| {
            let hex = line.trim().trim_start_matches('#');
            if hex.len() != 6 {
                return None;
            }
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some([
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
            ])
        })
        .collect()
}

impl SkinConfig {
    /// Loads from a `config.json` or an exported/downloaded `.rhs` skin, by
    /// extension. A `.rhs` may bundle textures and a colorset (see
    /// `docs/reference/rhs-skin-format.md`); those are extracted and rendered
    /// directly — the only sanctioned way to get an exact skin (the user
    /// imports their own pack; no game assets are ripped).
    pub fn from_path(path: impl AsRef<Path>) -> Result<SkinConfig, Error> {
        let path = path.as_ref();
        let data = std::fs::read(path)?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ext == "rhs" {
            Self::from_rhs(&data)
        } else {
            Self::from_json(&String::from_utf8_lossy(&data))
        }
    }

    /// Parses a `.rhs` zip: the `config` settings plus any bundled skin
    /// assets (note/border/cursor textures, colorset).
    pub fn from_rhs(data: &[u8]) -> Result<SkinConfig, Error> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data))
            .map_err(|e| Error::Config(format!("bad .rhs zip: {e}")))?;

        // The config settings entry is mandatory.
        let config_json = read_zip_entry(&mut zip, "config")?
            .ok_or_else(|| Error::Config(".rhs has no config entry".into()))?;
        let mut cfg = Self::from_json(&String::from_utf8_lossy(&config_json))?;

        // Bundled assets: at most one file per skin folder. Grab the first
        // entry under each known prefix.
        cfg.note_texture = first_zip_entry_under(&mut zip, "noteSkin/")?;
        cfg.border_texture = first_zip_entry_under(&mut zip, "borderSkin/")?;
        cfg.cursor_texture = first_zip_entry_under(&mut zip, "cursorTrailSkin/")?;
        if let Some(txt) = first_zip_entry_under(&mut zip, "colorSet/")? {
            cfg.colorset = parse_colorset(&String::from_utf8_lossy(&txt));
        }
        // Background layers ship as numbered folders matching the
        // BackgroundImages[] order; a layer without its file is dropped.
        for (i, layer) in cfg.background_images.iter_mut().enumerate() {
            if let Some(bytes) = first_zip_entry_under(&mut zip, &format!("backgrounds/{i}/"))? {
                layer.bytes = bytes;
            }
        }
        cfg.background_images.retain(|l| !l.bytes.is_empty());
        Ok(cfg)
    }

    pub fn from_json(json: &str) -> Result<SkinConfig, Error> {
        let doc: serde_json::Value =
            serde_json::from_str(json).map_err(|e| Error::Config(format!("config JSON: {e}")))?;

        // Each setting is stored as { "Field": { "Value": ... } }.
        let f = |key: &str| -> Option<&serde_json::Value> { doc.get(key)?.get("Value") };
        let num = |key: &str, dflt: f32| f(key).and_then(|v| v.as_f64()).map_or(dflt, |v| v as f32);
        let boolean = |key: &str, dflt: bool| f(key).and_then(|v| v.as_bool()).unwrap_or(dflt);
        let text = |key: &str| {
            f(key)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let rgb = |r: &str, g: &str, b: &str| {
            [
                num(r, 255.0) / 255.0,
                num(g, 255.0) / 255.0,
                num(b, 255.0) / 255.0,
            ]
        };
        let rgb8 = |r: &str, g: &str, b: &str, dflt: [u8; 3]| {
            [
                num(r, dflt[0] as f32).round().clamp(0.0, 255.0) as u8,
                num(g, dflt[1] as f32).round().clamp(0.0, 255.0) as u8,
                num(b, dflt[2] as f32).round().clamp(0.0, 255.0) as u8,
            ]
        };

        let d = SkinConfig::default();
        let hd = HudConfig::default();
        let hud = HudConfig {
            accuracy: boolean("LeftPanelAccuracyEnabled", hd.accuracy),
            combo_ring: boolean("LeftPanelComboRingEnabled", hd.combo_ring),
            grade: boolean("LeftPanelGradeEnabled", hd.grade),
            pauses: boolean("LeftPanelPausesEnabled", hd.pauses),
            score: boolean("RightPanelScoreEnabled", hd.score),
            points: boolean("RightPanelPointsEnabled", hd.points),
            misses: boolean("RightPanelMissesEnabled", hd.misses),
            notes: boolean("RightPanelNotesEnabled", hd.notes),
            health_bar: boolean("HealthBarEnabled", hd.health_bar),
            health_bar_color: rgb8(
                "HealthBarColorRed",
                "HealthBarColorGreen",
                "HealthBarColorBlue",
                hd.health_bar_color,
            ),
            health_bar_alpha: num("HealthBarAlpha", hd.health_bar_alpha),
            song_progress_bar: boolean("SongProgressBarEnabled", hd.song_progress_bar),
            song_progress_color: rgb8(
                "SongProgressBarColorRed",
                "SongProgressBarColorGreen",
                "SongProgressBarColorBlue",
                hd.song_progress_color,
            ),
            song_progress_alpha: num("SongProgressBarAlpha", hd.song_progress_alpha),
            combo_ring_color: rgb8(
                "ComboRingColorRed",
                "ComboRingColorGreen",
                "ComboRingColorBlue",
                hd.combo_ring_color,
            ),
            combo_ring_opacity: num("ComboRingOpacity", hd.combo_ring_opacity),
            combo_text_color: rgb8(
                "PlayfieldComboTextColorRed",
                "PlayfieldComboTextColorGreen",
                "PlayfieldComboTextColorBlue",
                hd.combo_text_color,
            ),
            combo_text_font_size: num("PlayfieldComboTextFontSize", hd.combo_text_font_size),
            combo_text_opacity: num("PlayfieldComboTextOpacity", hd.combo_text_opacity),
            combo_text_vpos_pct: num(
                "PlayfieldComboTextVerticalPositionPercent",
                hd.combo_text_vpos_pct,
            ),
            // No explicit toggle: the game hides it by setting opacity to 0.
            playfield_combo_text: num("PlayfieldComboTextOpacity", hd.combo_text_opacity) > 0.0,
            song_info: boolean("SongInfoEnabled", hd.song_info),
            miss_effect_opacity: num("MissEffectOpacity", hd.miss_effect_opacity),
            // Not game settings — the desktop app's HUD overrides use these.
            speed_label: hd.speed_label,
            error_meter: hd.error_meter,
            aim_meter: hd.aim_meter,
        };
        // BackgroundImages[]: rich per-layer placement; bytes come from the
        // .rhs archive afterwards (a bare config.json can't carry them).
        let background_images: Vec<BackgroundLayer> = doc
            .get("BackgroundImages")
            .map(|v| v.get("Value").unwrap_or(v))
            .and_then(|v| v.as_array())
            .map(|layers| {
                layers
                    .iter()
                    .map(|l| {
                        let f =
                            |k: &str, d: f64| l.get(k).and_then(|x| x.as_f64()).unwrap_or(d) as f32;
                        let i = |k: &str| l.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                        BackgroundLayer {
                            bytes: Vec::new(),
                            fit: i("Fit"),
                            placement: i("Placement"),
                            center_x: f("CenterX", 0.5),
                            center_y: f("CenterY", 0.5),
                            scale_x: f("ScaleX", 1.0),
                            scale_y: f("ScaleY", 1.0),
                            flip_horizontal: l
                                .get("FlipHorizontal")
                                .and_then(|x| x.as_bool())
                                .unwrap_or(false),
                            space_x: f("SpaceX", 0.0),
                            space_y: f("SpaceY", 0.0),
                            space_w: f("SpaceWidth", 8.0),
                            space_h: f("SpaceHeight", 8.0),
                            tint: [
                                f("TintRed", 255.0) / 255.0,
                                f("TintGreen", 255.0) / 255.0,
                                f("TintBlue", 255.0) / 255.0,
                                f("TintOpacity", 1.0),
                            ],
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let note_skin_name = text("NoteSkin");
        Ok(SkinConfig {
            mod_icons: Vec::new(),
            hud_font: None,
            camera_fov: num("CameraFov", d.camera_fov),
            approach_rate: num("ApproachRate", d.approach_rate),
            spawn_distance: num("SpawnDistance", d.spawn_distance),
            note_scale: num("NoteScale", d.note_scale),
            note_opacity: num("NoteOpacity", d.note_opacity),
            fade_length: num("FadeLength", d.fade_length).clamp(0.01, 1.0),
            parallax: num("Parallax", 0.0),
            half_ghost: boolean("HalfGhost", d.half_ghost),
            note_shape: NoteShape::from_skin(&note_skin_name),
            note_skin_name,
            border_skin_name: text("BorderSkin"),
            cursor_skin_name: text("CursorSkin"),
            colorset_name: text("ColorSet"),
            border_color: rgb("BorderColorRed", "BorderColorGreen", "BorderColorBlue"),
            border_opacity: num("BorderOpacity", d.border_opacity),
            cursor_color: rgb("CursorColorRed", "CursorColorGreen", "CursorColorBlue"),
            cursor_scale: num("CursorScale", d.cursor_scale),
            cursor_trail_enabled: boolean("CursorTrailEnabled", d.cursor_trail_enabled),
            cursor_trail_opacity: num("CursorTrailOpacity", d.cursor_trail_opacity),
            cursor_trail_fade_secs: num("CursorTrailFadeTimeSeconds", d.cursor_trail_fade_secs)
                .max(0.01),
            cursor_trail_spacing: num("CursorTrailSpacingMultiplier", d.cursor_trail_spacing)
                .max(0.01),
            cursor_trail_color: rgb(
                "CursorTrailColorRed",
                "CursorTrailColorGreen",
                "CursorTrailColorBlue",
            ),
            cursor_trail_inherit: boolean("CursorTrailInheritFromCursor", d.cursor_trail_inherit),
            cursor_trail_shrink: boolean("CursorTrailShrinkOverTime", d.cursor_trail_shrink),
            cursor_trail_gradient: if boolean("CursorTrailGradientEnabled", true) {
                f("CursorTrailGradient")
                    .and_then(|v| v.as_array())
                    .map(|stops| {
                        let mut g: Vec<(f32, [f32; 3])> = stops
                            .iter()
                            .filter_map(|s| {
                                let p = s.get("Position")?.as_f64()? as f32;
                                let ch = |k: &str| Some(s.get(k)?.as_f64()? as f32 / 255.0);
                                Some((p, [ch("Red")?, ch("Green")?, ch("Blue")?]))
                            })
                            .collect();
                        g.sort_by(|a, b| a.0.total_cmp(&b.0));
                        g
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
            // Assets come from a .rhs bundle, not the JSON; from_rhs fills them.
            note_texture: None,
            background_images,
            border_texture: None,
            cursor_texture: None,
            colorset: Vec::new(),
            hud,
            background_color: [
                num("BackgroundRed", 0.0) / 255.0,
                num("BackgroundGreen", 0.0) / 255.0,
                num("BackgroundBlue", 0.0) / 255.0,
            ],
            cursor_opacity: num("CursorOpacity", d.cursor_opacity),
            cursor_rotation_deg: num("CursorRotation", 0.0),
            playfield_grid: boolean("PlayfieldGridEnabled", d.playfield_grid),
            playfield_grid_color: rgb(
                "PlayfieldGridColorRed",
                "PlayfieldGridColorGreen",
                "PlayfieldGridColorBlue",
            ),
            playfield_grid_opacity: num("PlayfieldGridOpacity", d.playfield_grid_opacity),
            playfield_grid_thickness: num("PlayfieldGridThickness", d.playfield_grid_thickness),
            fail_vignette_opacity: num("FailVignetteOpacity", d.fail_vignette_opacity),
            interface_text_color: rgb8(
                "InterfaceTextColorRed",
                "InterfaceTextColorGreen",
                "InterfaceTextColorBlue",
                d.interface_text_color,
            ),
            interface_values_font_size: num(
                "InterfaceValuesFontSize",
                d.interface_values_font_size,
            )
            .max(0.1),
            panel_color: rgb8(
                "PanelColorRed",
                "PanelColorGreen",
                "PanelColorBlue",
                d.panel_color,
            ),
            panel_opacity: num("PanelOpacity", d.panel_opacity),
            panel_background_opacity: num("PanelBackgroundOpacity", d.panel_background_opacity),
            panel_gap: num("PanelGap", d.panel_gap),
            push_back: boolean("PushBack", d.push_back),
            disable_gui: boolean("GameSceneDisableGui", d.disable_gui),
            spin_camera: boolean("SpinCamera", d.spin_camera),
            ambient: AmbientConfig {
                color: [
                    num("BackgroundRed", 0.0) / 255.0,
                    num("BackgroundGreen", 0.0) / 255.0,
                    num("BackgroundBlue", 0.0) / 255.0,
                ],
                accent: rgb(
                    "BackgroundAccentRed",
                    "BackgroundAccentGreen",
                    "BackgroundAccentBlue",
                ),
                accent_from_hit_note: boolean("BackgroundAccentFromHitNote", false),
                tunnel_enabled: boolean("BackgroundTunnelEnabled", false),
                tunnel_opacity: num("BackgroundTunnelOpacity", 0.2),
                chevron_enabled: boolean("BackgroundChevronEnabled", false),
                chevron_opacity: num("BackgroundChevronOpacity", 0.25),
                chevron_gap: num("BackgroundChevronGap", 1250.0),
                chevron_large: num("BackgroundChevronLargeSize", 435.0),
                chevron_small: num("BackgroundChevronSmallSize", 430.0),
                chevron_width: num("BackgroundChevronWidth", 12.0),
                chevron_speed: num("BackgroundChevronSpeedMultiplier", 0.7),
                rays_enabled: boolean("BackgroundRaysEnabled", false),
                rays_opacity: num("BackgroundRaysOpacity", 0.2),
                rays_intensity: num("BackgroundRaysIntensity", 0.4),
                rays_width: num("BackgroundRaysWidth", 12.0),
                grid_enabled: boolean("BackgroundGridEnabled", false),
                grid_opacity: num("BackgroundGridOpacity", 0.22),
                grid_cell_size: num("BackgroundGridCellSize", 320.0),
                grid_center_gap: num("BackgroundGridCenterGap", 1100.0),
                grid_fade_falloff: num("BackgroundGridFadeFalloff", 1.35),
                grid_speed: num("BackgroundGridSpeedMultiplier", 0.75),
                grid_parallax: num("GridParallax", 0.0),
            },
        })
    }

    /// Whether the border skin is the "corners" style (brackets, not a full
    /// frame).
    pub fn border_is_corners(&self) -> bool {
        let n = self.border_skin_name.to_ascii_lowercase();
        n.contains("corner")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_value_wrapped_fields() {
        let json = r#"{
            "CameraFov": {"Value": 75},
            "ApproachRate": {"Value": 28},
            "NoteScale": {"Value": 0.93},
            "FadeLength": {"Value": 0.1},
            "Parallax": {"Value": 0},
            "HalfGhost": {"Value": true},
            "NoteSkin": {"Value": "Textures/Game/notes/thin.png"},
            "BorderSkin": {"Value": "Textures/Game/borders/small-corners.png"},
            "CursorTrailEnabled": {"Value": false}
        }"#;
        let c = SkinConfig::from_json(json).unwrap();
        assert_eq!(c.camera_fov, 75.0);
        assert_eq!(c.approach_rate, 28.0);
        assert!((c.note_scale - 0.93).abs() < 1e-6);
        assert!((c.fade_length - 0.1).abs() < 1e-6);
        assert_eq!(c.parallax, 0.0);
        assert!(c.half_ghost);
        assert_eq!(c.note_shape, NoteShape::Thin);
        assert!(c.border_is_corners());
        assert!(!c.cursor_trail_enabled);
    }

    #[test]
    fn missing_fields_fall_back_to_game_defaults() {
        let c = SkinConfig::from_json("{}").unwrap();
        assert_eq!(c.camera_fov, 70.0);
        assert!(c.cursor_trail_enabled);
    }

    #[test]
    fn note_shape_classification() {
        assert_eq!(NoteShape::from_skin("notes/thin.png"), NoteShape::Thin);
        assert_eq!(NoteShape::from_skin("notes/circle.png"), NoteShape::Circle);
        assert_eq!(
            NoteShape::from_skin("notes/rounded_fixed.png"),
            NoteShape::Rounded
        );
        assert_eq!(
            NoteShape::from_skin("notes/square 3.png"),
            NoteShape::Square
        );
    }

    #[test]
    fn parses_colorset_hex_lines() {
        let cols = parse_colorset("#00BFFF\r\n#FFFF00\r\ngarbage\r\n#ff0000");
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[0], [0.0, 191.0 / 255.0, 1.0]);
        assert_eq!(cols[2], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn imports_bundled_assets_from_rhs() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        // A synthetic .rhs with settings + a note texture + a colorset.
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default();
        zw.start_file("config", opts).unwrap();
        zw.write_all(br#"{"NoteScale":{"Value":0.8}}"#).unwrap();
        zw.start_file("noteSkin/my note.png", opts).unwrap();
        zw.write_all(b"\x89PNG-fake-bytes").unwrap();
        zw.start_file("colorSet/set.txt", opts).unwrap();
        zw.write_all(b"#8b87ff\r\n#ffe1c9").unwrap();
        let data = zw.finish().unwrap().into_inner();

        let c = SkinConfig::from_rhs(&data).unwrap();
        assert!((c.note_scale - 0.8).abs() < 1e-6); // config parsed
        assert_eq!(c.note_texture.as_deref(), Some(&b"\x89PNG-fake-bytes"[..]));
        assert!(c.border_texture.is_none());
        assert_eq!(
            c.colorset,
            vec![
                [139.0 / 255.0, 135.0 / 255.0, 1.0],
                [1.0, 225.0 / 255.0, 201.0 / 255.0]
            ]
        );
    }

    #[test]
    fn rhs_without_config_errors() {
        use std::io::Write;
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zw.start_file("noteSkin/x.png", zip::write::SimpleFileOptions::default())
            .unwrap();
        zw.write_all(b"x").unwrap();
        let data = zw.finish().unwrap().into_inner();
        assert!(SkinConfig::from_rhs(&data).is_err());
    }
}
