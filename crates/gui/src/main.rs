//! rhythr — desktop app (Tauri shell around the render crates).
//!
//! Read-only like the CLI: replays are parsed, verified and rendered, never
//! written. Maps auto-download from production.rhythia.com (cached locally,
//! hash-verified against the replay header).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

mod live;

use rhythia_formats::{map::Map, rhr::Replay};
use rhythia_render::{scene::SceneParams, SkinConfig};
use rhythia_sim::integrity;

const USER_AGENT: &str = concat!(
    "rhythr/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/KillerOp007/rhythr)"
);
// Usage of this endpoint was agreed with the Rhythia team (July 2026):
// one request per uncached map with limit:1 and an empty session, local
// caching, an identifying User-Agent, no bulk crawling or prefetching,
// and backing off on 429/5xx. The endpoint is best-effort and may change.
// If the network scope of this tool ever changes, ask the team again first.
const API_BEATMAP_PAGE: &str = "https://production.rhythia.com/api/getBeatmapPage";
/// Refuse to download maps larger than this (malformed/hostile responses).
const MAX_MAP_BYTES: u64 = 512 * 1024 * 1024;
/// Ghost overlay colour (sRGB 0..1) — a warm orange, clearly distinct.
pub(crate) const GHOST_COLOR: [f32; 3] = [1.0, 0.55, 0.24];
const PREVIEW_W: u32 = 1280;
const PREVIEW_H: u32 = 720;

// ---------------------------------------------------------------- settings

/// Persisted app settings (config dir). HUD overrides live here so they
/// survive restarts and apply to every render until reset.
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    /// Main window geometry from the last session: (x, y, width, height).
    /// Reopening at the default size every launch meant re-arranging the
    /// window on a large screen every single time.
    window_rect: Option<(i32, i32, u32, u32)>,
    last_replay: Option<String>,
    last_config: Option<String>,
    game_assets: Option<String>,
    output_dir: Option<String>,
    /// Empty = derive "Player - Song.mp4" from the loaded replay/map.
    file_name: String,
    ffmpeg: Option<String>,
    width: u32,
    height: u32,
    fps: u32,
    /// Render quality, 0..=100, HIGHER IS BETTER — see
    /// [`rhythia_render::quality`].
    quality: u32,
    /// Present only in settings written before that scale was inverted,
    /// where the stored number was the raw x264 CRF. Converted into
    /// `quality` at load and never written back out.
    #[serde(skip_serializing)]
    crf: Option<u32>,
    /// Hand frames to ffmpeg over a loopback socket instead of its stdin.
    /// Off until it has been measured somewhere other than the machine it
    /// was written on — it wins about 5% with a real encoder attached, and
    /// costs a way for a render to fail that a pipe does not have.
    tcp_feed: bool,
    encoder: String,
    preset: String,
    results_secs: f64,
    /// Motion blur strength 0-2 (tmix).
    motion_blur: u32,
    /// Render speed of the last completed render, for the time estimate.
    last_render_fps: f64,
    /// Song volume in percent (0-150).
    music_volume: u32,
    /// Hit/miss-sound volume in percent (0 = off).
    hitsound_volume: u32,
    /// HUD element key -> forced on/off. Absent key = follow the config.
    hud_overrides: BTreeMap<String, bool>,
    /// Drag-editor positions per HUD element (normalised frame centre).
    hud_positions: BTreeMap<String, [f32; 2]>,
    /// Drag-editor sizes per HUD element (scale factor, 0.4..2.5).
    hud_scales: BTreeMap<String, f32>,
    /// Optional overlay meters (renderer extras, not game elements).
    error_meter: MeterSettings,
    aim_meter: MeterSettings,
    /// Ghost-race extra: the score-lead widget (numbers, tournament bar,
    /// results graph). Full-frame, so ghost_x/ghost_y stay unused.
    race_delta: MeterSettings,
    /// Custom playfield background (image or video file); replaces the
    /// skin's background during gameplay, results screen untouched.
    background: Option<String>,
    /// How much the custom background is darkened, 0-100 percent.
    background_dim: u32,
    /// Zoom on the background, percent (100 = exactly covering).
    background_zoom: u32,
    /// Shift in percent of the frame size (positive = right/down),
    /// clamped to the available overflow at compose time.
    background_off_x: i32,
    background_off_y: i32,
    /// Video backgrounds: start (and loop) playback from this second.
    background_start: f64,
    /// Video backgrounds in clip renders: true = the video follows the
    /// song position (as if it played since 0:00), false = it restarts
    /// at the clip start.
    background_sync_song: bool,
    recent_replays: Vec<String>,
    /// Named layout/look snapshots ("Save preset" in the app). "Before
    /// reset" is written automatically before every layout reset.
    presets: BTreeMap<String, LayoutPreset>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            window_rect: None,
            last_replay: None,
            last_config: None,
            game_assets: None,
            output_dir: None,
            file_name: String::new(),
            ffmpeg: None,
            width: 1920,
            height: 1080,
            fps: 60,
            quality: rhythia_render::quality::DEFAULT,
            crf: None,
            tcp_feed: false,
            encoder: "auto".into(),
            preset: "veryfast".into(),
            results_secs: 4.0,
            motion_blur: 0,
            last_render_fps: 0.0,
            music_volume: 100,
            hitsound_volume: 50,
            hud_overrides: BTreeMap::new(),
            hud_positions: BTreeMap::new(),
            hud_scales: BTreeMap::new(),
            error_meter: MeterSettings::at(0.5, 0.88),
            aim_meter: MeterSettings::at(0.15, 0.32),
            // The race widget only ever shows in ghost races, which are
            // deliberate — unlike the meters it defaults to on.
            race_delta: MeterSettings { enabled: true, ..MeterSettings::at(0.5, 0.095) },
            background: None,
            background_dim: 60,
            background_zoom: 100,
            background_off_x: 0,
            background_off_y: 0,
            background_start: 0.0,
            background_sync_song: true,
            recent_replays: Vec::new(),
            presets: BTreeMap::new(),
        }
    }
}

/// A named layout/look snapshot: everything that defines how a render
/// LOOKS — HUD layout, sizes and visibility, the meters, the skin config,
/// output format and the custom background. "TikTok layout" and "YouTube
/// layout" become one click each.
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct LayoutPreset {
    hud_overrides: BTreeMap<String, bool>,
    hud_positions: BTreeMap<String, [f32; 2]>,
    hud_scales: BTreeMap<String, f32>,
    error_meter: MeterSettings,
    aim_meter: MeterSettings,
    race_delta: MeterSettings,
    config_path: Option<String>,
    width: u32,
    height: u32,
    background: Option<String>,
    background_dim: u32,
    background_zoom: u32,
    background_off_x: i32,
    background_off_y: i32,
    background_start: f64,
    #[serde(default = "default_true")]
    background_sync_song: bool,
}

fn default_true() -> bool {
    true
}

impl Default for LayoutPreset {
    fn default() -> Self {
        let s = Settings::default();
        LayoutPreset {
            hud_overrides: BTreeMap::new(),
            hud_positions: BTreeMap::new(),
            hud_scales: BTreeMap::new(),
            error_meter: s.error_meter,
            aim_meter: s.aim_meter,
            race_delta: s.race_delta,
            config_path: None,
            width: s.width,
            height: s.height,
            background: None,
            background_dim: s.background_dim,
            background_zoom: s.background_zoom,
            background_off_x: s.background_off_x,
            background_off_y: s.background_off_y,
            background_start: s.background_start,
            background_sync_song: s.background_sync_song,
        }
    }
}

/// The current look as a preset.
fn preset_snapshot(inner: &Inner) -> LayoutPreset {
    let s = &inner.settings;
    LayoutPreset {
        hud_overrides: s.hud_overrides.clone(),
        hud_positions: s.hud_positions.clone(),
        hud_scales: s.hud_scales.clone(),
        error_meter: s.error_meter,
        aim_meter: s.aim_meter,
        race_delta: s.race_delta,
        config_path: inner
            .config_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        width: s.width,
        height: s.height,
        background: s.background.clone(),
        background_dim: s.background_dim,
        background_zoom: s.background_zoom,
        background_off_x: s.background_off_x,
        background_off_y: s.background_off_y,
        background_start: s.background_start,
        background_sync_song: s.background_sync_song,
    }
}

/// Restores only the LAYOUT part (element positions/sizes/visibility and
/// meters) — what Undo covers. Resolution, skin config and background
/// stay put.
fn apply_layout_only(settings: &mut Settings, p: &LayoutPreset) {
    settings.hud_overrides = p.hud_overrides.clone();
    settings.hud_positions = p.hud_positions.clone();
    settings.hud_scales = p.hud_scales.clone();
    settings.error_meter = p.error_meter;
    settings.aim_meter = p.aim_meter;
    settings.race_delta = p.race_delta;
}

/// Remembers the current layout on the undo stack — called before every
/// editor action, and once per drag GESTURE (mark_undo), not per live
/// tick. A new action invalidates the redo branch.
fn remember_layout(inner: &mut Inner) {
    let snap = preset_snapshot(inner);
    if inner.undo_stack.len() >= 50 {
        inner.undo_stack.remove(0);
    }
    inner.undo_stack.push(snap);
    inner.redo_stack.clear();
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rhythr")
}

fn maps_cache_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rhythr")
        .join("maps")
}

impl Settings {
    /// Takes over a quality setting written before the scale was inverted.
    ///
    /// Those files stored the raw x264 CRF, where LOWER meant better. Reading
    /// one of those numbers as a 0..=100 quality would turn somebody's "best"
    /// into "draft" the first time they opened the new version — 14 was the
    /// finest the old slider went and is nearly the coarsest the new one
    /// does. The old field is converted once and never written back.
    fn adopt_legacy_quality(&mut self) {
        if let Some(crf) = self.crf.take() {
            self.quality = rhythia_render::quality::from_legacy_crf(crf);
        }
    }

    fn load() -> Settings {
        let path = config_dir().join("settings.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Settings::default();
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.adopt_legacy_quality();
                s
            }
            Err(e) => {
                // Losing every preference in silence is worse than the
                // parse failure itself: keep the file so it can be looked
                // at (or hand-fixed) instead of overwriting it on the next
                // save with defaults.
                let backup = path.with_extension("json.broken");
                let _ = std::fs::rename(&path, &backup);
                eprintln!(
                    "settings.json could not be read ({e}); kept a copy at {} and started from defaults",
                    backup.display()
                );
                Settings::default()
            }
        }
    }

    /// Writes via a temporary file and a rename, so an interrupted save (or
    /// two windows saving at once) can never leave a half-written settings
    /// file behind — the old one stays until the new one is complete.
    fn save(&self) {
        let dir = config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let Ok(json) = serde_json::to_string_pretty(self) else {
            return;
        };
        let final_path = dir.join("settings.json");
        let tmp = dir.join("settings.json.tmp");
        if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &final_path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

// ------------------------------------------------------------------- state

/// Cached preview pipeline: one low-res GPU renderer plus the prepared skin
/// and resolved hit/miss state; rebuilt when replay/map/config change.
struct PreviewCtx {
    renderer: rhythia_render::Renderer,
    skin: rhythia_render::renderer::SkinTextures,
    hud: rhythia_render::hud::HudState,
    ghost: Option<rhythia_render::hud::GhostInput>,
    cfg: SkinConfig,
    params: SceneParams,
    /// The map with the main replay's geometry mods applied.
    map: rhythia_formats::map::Map,
    /// Video background: path + probed duration for per-scrub frame
    /// extraction (None for image/no background).
    bg_video: Option<(PathBuf, Option<f64>)>,
}

#[derive(Default)]
struct Inner {
    replay: Option<(PathBuf, Replay)>,
    map: Option<(PathBuf, Map)>,
    map_source: String,
    /// Probed duration of the current VIDEO background (drives the
    /// start-time slider); None for images/no background/unprobed.
    bg_duration: Option<f64>,
    /// Session clip range (song ms), rendered instead of the full run.
    clip: Option<(f64, f64)>,
    /// Height the live preview renders at; the Analyze window raises it
    /// so a full-screen replay stays sharp.
    preview_height: u32,
    /// Bumped with every preview invalidation; cached frames from an
    /// older generation are dropped.
    frame_gen: u64,
    /// Analyze-window view options: hide the game's rendered cursor
    /// (the raw-cursor overlay replaces it) and/or the notes (to study
    /// hit areas alone). Applied to previews AND playback segments.
    analyze_hide_cursor: bool,
    analyze_hide_notes: bool,
    /// How long resolved analyze hit-area boxes linger (ms, 0 = instant).
    analyze_linger_ms: f64,
    /// Multi-step layout history (Ctrl+Z / Ctrl+Y): snapshots taken
    /// before each editor action/gesture.
    undo_stack: Vec<LayoutPreset>,
    redo_stack: Vec<LayoutPreset>,
    /// True when the cached map's hash does not match the replay header.
    map_hash_mismatch: bool,
    config_path: Option<PathBuf>,
    /// Optional second replay rendered as a ghost overlay.
    ghost: Option<(PathBuf, Replay)>,
    base_config: SkinConfig,
    settings: Settings,
    preview: Option<PreviewCtx>,
}

/// Rendered preview frames, keyed by whole song ms. The Analyze window
/// pulls frames through a custom URI scheme (no base64, no IPC) and asks
/// for them to be rendered AHEAD of the playhead, so playback displays
/// finished PNGs instead of waiting on the GPU per frame.
#[derive(Default)]
struct FrameCache {
    /// Bumped whenever the preview pipeline changes; stale entries die.
    gen: u64,
    frames: std::collections::BTreeMap<i64, Arc<Vec<u8>>>,
    bytes: usize,
}

impl FrameCache {
    /// Frame size scales with the window, so bound the cache by BYTES —
    /// 96 MiB is ~2 s of 60 fps at 1440p and ~12 s at 720p.
    const CAP_BYTES: usize = 96 * 1024 * 1024;
    /// …and never keep more than a few seconds of frames anyway.
    const CAP_FRAMES: usize = 400;

    fn insert(&mut self, t: i64, png: Arc<Vec<u8>>, around: i64) {
        if let Some(old) = self.frames.insert(t, png.clone()) {
            self.bytes -= old.len();
        }
        self.bytes += png.len();
        while self.bytes > Self::CAP_BYTES || self.frames.len() > Self::CAP_FRAMES {
            // Drop whatever sits furthest from the playhead.
            let lo = *self.frames.keys().next().unwrap();
            let hi = *self.frames.keys().next_back().unwrap();
            let drop_key = if (around - lo).abs() >= (hi - around).abs() { lo } else { hi };
            if self.frames.len() <= 1 {
                break;
            }
            if let Some(v) = self.frames.remove(&drop_key) {
                self.bytes -= v.len();
            }
        }
    }

    fn clear(&mut self) {
        self.frames.clear();
        self.bytes = 0;
    }
}

/// A rendered playback segment: real video the webview can play with
/// hardware decoding, instead of streaming stills it has to decode one by
/// one. `out_fps` frames per SONG second — raise it and the same span
/// plays back slower while staying perfectly smooth.
#[derive(Clone)]
#[allow(dead_code)] // segment fields feed the fallback engines' events
struct ReadySegment {
    token: u64,
    path: PathBuf,
    start_ms: f64,
    span_ms: f64,
    out_fps: u32,
}

#[derive(Default)]
struct SegmentState {
    dir: Option<PathBuf>,
    ready: Option<ReadySegment>,
}

struct Shared {
    inner: Mutex<Inner>,
    cancel: AtomicBool,
    rendering: AtomicBool,
    frames: Mutex<FrameCache>,
    /// Newest prefetch request; older workers see the bump and stop.
    prefetch_gen: std::sync::atomic::AtomicU64,
    segment: Mutex<SegmentState>,
    /// Newest segment request; an older render sees the bump and stops.
    segment_gen: std::sync::atomic::AtomicU64,
    /// Frame requests being served right now. A burst (fast scrubbing)
    /// must not spawn unbounded threads, and the prefetcher steps aside
    /// while a live request waits for the renderer.
    frame_jobs: std::sync::atomic::AtomicUsize,
    /// Join handle of the active render thread (used on app exit).
    render_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Shared {
    /// Locks the app state, recovering from poisoning — a panic in one
    /// command (e.g. a GPU error during preview) must not brick every
    /// other command for the rest of the session.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

type App = Arc<Shared>;

// -------------------------------------------------------------------- DTOs

#[derive(Serialize, Clone)]
struct PlannedOutputDto {
    path: String,
    exists: bool,
}

#[derive(Serialize, Clone)]
struct VerifyDto {
    consistent: bool,
    /// Failed error-level checks as "name: expected X, got Y".
    problems: Vec<String>,
    /// The checks failed because this map is not the one that was played.
    /// Without this the UI accused the player's replay of being edited
    /// when they had simply picked the wrong map file.
    wrong_map: bool,
}

#[derive(Serialize, Clone)]
struct ReplayDto {
    path: String,
    file_name: String,
    player: String,
    map_id: i32,
    legacy_map_id: String,
    speed: f32,
    mods: Vec<String>,
    passed: bool,
    failed: bool,
    fail_time_ms: i32,
    length_ms: f64,
    hits: i32,
    misses: i32,
    accuracy_pct: f32,
    total_score: i64,
    points: f32,
    unix_ms: Option<i64>,
    verify: Option<VerifyDto>,
}

#[derive(Serialize, Clone)]
struct MapDto {
    path: String,
    title: String,
    song_name: String,
    note_count: usize,
    duration_ms: i64,
    has_audio: bool,
    has_cover: bool,
    source: String,
    hash_mismatch: bool,
}

#[derive(Serialize, Clone)]
struct ConfigDto {
    path: Option<String>,
    /// HUD flags as the config file defines them (override baseline).
    base_hud: BTreeMap<String, bool>,
    /// Flags after applying the app's overrides (what actually renders).
    effective_hud: BTreeMap<String, bool>,
}

#[derive(Serialize, Clone)]
struct GhostDto {
    path: String,
    file_name: String,
    player: String,
    same_map: bool,
}

#[derive(Serialize, Clone)]
struct StatusDto {
    replay: Option<ReplayDto>,
    ghost: Option<GhostDto>,
    map: Option<MapDto>,
    config: ConfigDto,
    settings: Settings,
    rendering: bool,
    /// The configured game-assets folder exists and holds an extraction.
    game_ok: bool,
    /// Duration of the current video background, for the start slider.
    bg_video_duration: Option<f64>,
    /// Session clip range (start_ms, end_ms) if the user set one.
    clip: Option<(f64, f64)>,
    /// Height the live preview renders at (Analyze can raise it).
    preview_height: u32,
    /// The skin renders on a bright background: overlay strokes must go
    /// dark or they vanish. Lives on status (not the analysis payload)
    /// so a mid-session skin swap updates it via the normal refresh.
    light_background: bool,
    can_undo: bool,
    can_redo: bool,
    /// Build identity, so a bug report always names the exact build.
    build: String,
}

#[derive(Serialize)]
struct TimelineDto {
    length_ms: f64,
    fail_ms: Option<f64>,
    /// Health 0..1 downsampled over the run.
    health: Vec<f32>,
    /// Song times of missed notes.
    miss_times: Vec<f64>,
}

// ------------------------------------------------------------- HUD toggles

/// Stable keys the UI toggles by; each maps onto one HudConfig element.
const HUD_KEYS: [&str; 14] = [
    "song_info",
    "song_progress",
    "combo_ring",
    "pauses",
    "grade",
    "accuracy",
    "score",
    "points",
    "misses",
    "notes",
    "health_bar",
    "combo_text",
    "miss_marker",
    "speed_label",
];

fn hud_flags(cfg: &SkinConfig) -> BTreeMap<String, bool> {
    let h = &cfg.hud;
    let mut m = BTreeMap::new();
    m.insert("song_info".into(), h.song_info);
    m.insert("song_progress".into(), h.song_progress_bar);
    m.insert("combo_ring".into(), h.combo_ring);
    m.insert("pauses".into(), h.pauses);
    m.insert("grade".into(), h.grade);
    m.insert("accuracy".into(), h.accuracy);
    m.insert("score".into(), h.score);
    m.insert("points".into(), h.points);
    m.insert("misses".into(), h.misses);
    m.insert("notes".into(), h.notes);
    m.insert("health_bar".into(), h.health_bar);
    m.insert("combo_text".into(), h.playfield_combo_text);
    m.insert("miss_marker".into(), h.miss_effect_opacity > 0.0);
    m.insert("speed_label".into(), h.speed_label);
    m
}

fn apply_overrides(cfg: &mut SkinConfig, overrides: &BTreeMap<String, bool>) {
    for (key, &on) in overrides {
        let h = &mut cfg.hud;
        match key.as_str() {
            "song_info" => h.song_info = on,
            "song_progress" => h.song_progress_bar = on,
            "combo_ring" => h.combo_ring = on,
            "pauses" => h.pauses = on,
            "grade" => h.grade = on,
            "accuracy" => h.accuracy = on,
            "score" => h.score = on,
            "points" => h.points = on,
            "misses" => h.misses = on,
            "notes" => h.notes = on,
            "health_bar" => h.health_bar = on,
            "combo_text" => {
                h.playfield_combo_text = on;
                if on && h.combo_text_opacity <= 0.0 {
                    h.combo_text_opacity = 0.05;
                }
            }
            "miss_marker" => {
                if !on {
                    h.miss_effect_opacity = 0.0;
                } else if h.miss_effect_opacity <= 0.0 {
                    h.miss_effect_opacity = 1.0;
                }
            }
            "speed_label" => h.speed_label = on,
            _ => {}
        }
    }
}

// ----------------------------------------------------------------- helpers

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A GPU that cannot be brought up reaches the user as one line of text, so
/// that line has to carry the next step: which layer failed, why it usually
/// fails, and the two escapes that exist (a driver the backend can use, or a
/// different backend). Errors that are not GPU bring-up keep their own text.
fn gpu_err(e: &rhythia_render::Error) -> String {
    match e {
        rhythia_render::Error::NoAdapter => "no usable GPU: no graphics adapter accepted the \
             renderer. rhythr draws through Vulkan (Linux, Windows), DX12 (Windows) or Metal \
             (macOS) — on Linux a driver without Vulkan support is the usual cause. Update the \
             graphics driver, or start rhythr with WGPU_BACKEND=gl to force the OpenGL backend."
            .to_string(),
        rhythia_render::Error::Device(msg) => format!(
            "the GPU was found but would not open a device ({msg}). That is normally an \
             out-of-date or wedged graphics driver; updating it, or starting rhythr with \
             WGPU_BACKEND=gl, is the next thing to try."
        ),
        other => other.to_string(),
    }
}

/// Placement/looks of an optional overlay meter (normalised position).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(default)]
struct MeterSettings {
    enabled: bool,
    x: f32,
    y: f32,
    /// Position on the ghost side of a split frame (None = follow x/y).
    ghost_x: Option<f32>,
    ghost_y: Option<f32>,
    scale: f32,
    alpha: f32,
}

impl MeterSettings {
    fn at(x: f32, y: f32) -> MeterSettings {
        MeterSettings {
            enabled: false,
            x,
            y,
            ghost_x: None,
            ghost_y: None,
            scale: 1.0,
            alpha: 0.9,
        }
    }

    fn apply(self, target: &mut rhythia_render::config::ErrorMeter) {
        target.enabled = self.enabled;
        target.x = self.x.clamp(0.0, 1.0);
        target.y = self.y.clamp(0.0, 1.0);
        target.ghost_x = self.ghost_x.map(|v| v.clamp(0.0, 1.0));
        target.ghost_y = self.ghost_y.map(|v| v.clamp(0.0, 1.0));
        target.scale = self.scale.clamp(0.4, 2.5);
        target.alpha = self.alpha.clamp(0.05, 1.0);
    }
}

impl Default for MeterSettings {
    fn default() -> Self {
        MeterSettings::at(0.5, 0.88)
    }
}

/// Applies the LAYOUT part of the settings onto a config — cheap (bools
/// and small maps only), so the live editor can re-apply it per preview
/// frame without rebuilding the renderer or re-uploading the skin. The
/// hud section resets from the BASE config first: a removed override
/// must fall back to the skin's own value.
fn apply_hud_settings(cfg: &mut SkinConfig, base: &SkinConfig, s: &Settings) {
    cfg.hud = base.hud.clone();
    apply_overrides(cfg, &s.hud_overrides);
    cfg.hud.positions = s.hud_positions.clone();
    cfg.hud.scales = s.hud_scales.clone();
    s.error_meter.apply(&mut cfg.hud.error_meter);
    s.aim_meter.apply(&mut cfg.hud.aim_meter);
    s.race_delta.apply(&mut cfg.hud.race_delta);
    // The dim rides the background quad's instance colour — no
    // recompose needed, so it is live too.
    cfg.custom_bg_dim = s
        .background
        .as_ref()
        .map(|_| s.background_dim.min(100) as f32 / 100.0);
}

/// The config as it renders: file config + game assets + HUD overrides.
fn effective_config(inner: &Inner) -> SkinConfig {
    let mut cfg = inner.base_config.clone();
    apply_hud_settings(&mut cfg, &inner.base_config, &inner.settings);
    if inner.analyze_hide_cursor {
        cfg.cursor_opacity = 0.0;
        cfg.cursor_trail_enabled = false;
    }
    if inner.analyze_hide_notes {
        cfg.note_opacity = 0.0;
    }
    // Custom background: replaces the skin's background layers. Silently
    // skipped if the file vanished — set_background validated it once.
    if let Some(p) = &inner.settings.background {
        let _ = rhythia_render::background::apply_background(
            &mut cfg,
            Path::new(p),
            &bg_options(&inner.settings),
        );
    }
    cfg
}

/// The user's background placement, as the render crate consumes it.
fn bg_options(s: &Settings) -> rhythia_render::background::BackgroundOptions {
    rhythia_render::background::BackgroundOptions {
        dim: s.background_dim.min(100) as f32 / 100.0,
        zoom: s.background_zoom.clamp(100, 400) as f32 / 100.0,
        offset: [
            s.background_off_x.clamp(-100, 100) as f32 / 100.0,
            s.background_off_y.clamp(-100, 100) as f32 / 100.0,
        ],
        start_secs: s.background_start.max(0.0),
        sync_offset_secs: 0.0,
    }
}

fn load_base_config(
    path: &Option<PathBuf>,
    game_assets: &Option<String>,
) -> Result<SkinConfig, String> {
    let mut cfg = match path {
        Some(p) => SkinConfig::from_path(p).map_err(err_str)?,
        None => SkinConfig::default(),
    };
    if let Some(dir) = game_assets {
        cfg.resolve_builtins(&rhythia_render::BuiltinAssets::load(Path::new(dir)));
    }
    Ok(cfg)
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let mut trimmed = cleaned.trim().trim_matches('.').to_string();
    // Windows chokes on device names as file stems (CON, PRN, COM1, …).
    let stem = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
            | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
            | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    );
    if reserved {
        trimmed.insert(0, '_');
    }
    if trimmed.chars().count() > 150 {
        trimmed = trimmed.chars().take(150).collect();
    }
    if trimmed.is_empty() {
        "render".into()
    } else {
        trimmed
    }
}

/// "Player - Song.mp4" from the loaded replay/map.
fn suggested_name(inner: &Inner) -> String {
    let player = inner
        .replay
        .as_ref()
        .map(|(_, r)| r.player_name.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("replay");
    let song = inner
        .map
        .as_ref()
        .map(|(_, m)| {
            if m.meta.song_name.is_empty() {
                m.meta.title.as_str()
            } else {
                m.meta.song_name.as_str()
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or("render");
    sanitize_filename(&format!("{player} - {song}.mp4"))
}

/// The game's hit/miss sounds from the extracted assets, when present and
/// the volume is above zero.
fn load_hitsounds(s: &Settings) -> Option<rhythia_render::video::HitsoundOptions> {
    if s.hitsound_volume == 0 {
        return None;
    }
    let dir = PathBuf::from(s.game_assets.as_ref()?)
        .join("builtin_assets")
        .join("sounds");
    Some(rhythia_render::video::HitsoundOptions {
        hit_wav: std::fs::read(dir.join("hit.wav")).ok()?,
        miss_wav: std::fs::read(dir.join("miss.wav")).ok(),
        volume: s.hitsound_volume.min(150) as f32 / 100.0,
    })
}

/// Bundle resource directory, set once at startup — where the AppImage
/// carries its ffmpeg (on Windows the resources sit next to the exe, so
/// the sibling check below covers it).
static RESOURCE_DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

/// ffmpeg to run: explicit setting first. On Windows the bundled sibling
/// wins (the installer ships a full build). On Linux the DISTRO ffmpeg
/// wins when present — its VAAPI/NVENC are linked against the system's
/// own driver libraries, where a portable static build can misbehave —
/// and the copy bundled in the AppImage is the fallback so the app still
/// works on systems with no ffmpeg at all.
fn resolve_ffmpeg(settings: &Settings) -> String {
    if let Some(f) = &settings.ffmpeg {
        if !f.trim().is_empty() {
            return f.clone();
        }
    }
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let in_path = || {
        std::env::var_os("PATH").into_iter().any(|paths| {
            std::env::split_paths(&paths).any(|d| d.join(name).is_file())
        })
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(name));
        }
    }
    if let Some(Some(res)) = RESOURCE_DIR.get() {
        candidates.push(res.join(name));
    }
    if !cfg!(windows) && in_path() {
        return name.into();
    }
    for c in candidates {
        if c.exists() {
            return c.to_string_lossy().into_owned();
        }
    }
    name.into()
}

/// How this install updates. "self": Windows (NSIS) and the Linux
/// AppImage replace themselves through the updater. "aur": the binary is
/// owned by pacman (the AUR package) — the update comes through the AUR
/// helper, the banner should say so. "page": deb/rpm — point at the
/// releases page.
#[tauri::command]
fn update_channel() -> String {
    if cfg!(windows) || std::env::var_os("APPIMAGE").is_some() {
        return "self".into();
    }
    #[cfg(unix)]
    {
        let pacman_owned = std::env::current_exe().is_ok_and(|exe| {
            std::process::Command::new("pacman")
                .arg("-Qqo")
                .arg(exe)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        });
        if pacman_owned {
            return "aur".into();
        }
    }
    "page".into()
}

/// Opens the GitHub releases page (the update path for deb/rpm installs).
#[tauri::command]
fn open_releases_page(app: tauri::AppHandle) {
    use tauri_plugin_opener::OpenerExt;
    let _ = app
        .opener()
        .open_url("https://github.com/KillerOp007/rhythr/releases/latest", None::<&str>);
}

/// Whether a remembered window origin still lands on a connected monitor.
/// Screens get unplugged; a window restored onto one that is gone would be
/// invisible with no way to drag it back.
/// The main window's last known geometry, kept away from the global state
/// lock so the event-loop thread never waits on a render worker.
static LAST_WINDOW_RECT: std::sync::Mutex<Option<(i32, i32, u32, u32)>> =
    std::sync::Mutex::new(None);

fn window_pos_visible(window: &tauri::WebviewWindow, x: i32, y: i32) -> bool {
    let Ok(monitors) = window.available_monitors() else {
        return false;
    };
    monitors.iter().any(|m| {
        let p = m.position();
        let s = m.size();
        x >= p.x - 32
            && y >= p.y - 32
            && x < p.x + s.width as i32
            && y < p.y + s.height as i32
    })
}

/// A plain-text description of this build and what it is working with, for
/// attaching to a bug report. Nothing here leaves the machine on its own —
/// the user picks where it is written and reads it first.
///
/// Deliberately excluded: the replay's player name and any absolute path
/// outside what the user already sees in the window, so sharing it does not
/// leak more than the screenshot they would have sent anyway.
#[tauri::command]
fn write_diagnostics(state: tauri::State<'_, App>, path: String) -> Result<String, String> {
    use std::fmt::Write as _;
    let app = state.inner();
    let inner = app.lock();
    let mut s = String::new();
    let _ = writeln!(s, "rhythr {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(s, "os: {} {}", std::env::consts::OS, std::env::consts::ARCH);

    let ffmpeg = resolve_ffmpeg(&inner.settings);
    let _ = writeln!(s, "\nffmpeg");
    let _ = writeln!(s, "  resolved: {ffmpeg}");
    let _ = writeln!(
        s,
        "  runs: {}",
        rhythia_render::video::ffmpeg_runs(&ffmpeg)
    );
    for e in ["nvenc", "qsv", "vaapi"] {
        match rhythia_render::video::encoder_error(&ffmpeg, e) {
            None => {
                let _ = writeln!(s, "  {e}: available");
            }
            Some(why) => {
                let _ = writeln!(s, "  {e}: unavailable — {why}");
            }
        }
    }

    let _ = writeln!(s, "\nloaded");
    match (&inner.replay, &inner.map) {
        (Some((_, r)), m) => {
            let _ = writeln!(
                s,
                "  replay: v{} · speed {:.2} · mods {} · {} hits / {} misses · {:.2}%",
                r.version, r.speed, r.mods, r.hits, r.misses, r.accuracy_pct
            );
            let _ = writeln!(
                s,
                "  frames: {} · failed: {} · trailing bytes: {}",
                r.frames.len(),
                r.failed(),
                r.trailing_bytes
            );
            match m {
                Some((_, map)) => {
                    let _ = writeln!(
                        s,
                        "  map: {} notes · hash match: {}",
                        map.notes.len(),
                        !inner.map_hash_mismatch
                    );
                    let report = integrity::verify_replay(r, map);
                    let _ = writeln!(s, "  integrity: consistent = {}", report.consistent());
                    for c in report.failed_checks() {
                        let _ = writeln!(
                            s,
                            "    {} — expected {}, got {}",
                            c.name, c.expected, c.actual
                        );
                    }
                }
                None => {
                    let _ = writeln!(s, "  map: none loaded");
                }
            }
        }
        _ => {
            let _ = writeln!(s, "  nothing loaded");
        }
    }

    let cfg = &inner.settings;
    let _ = writeln!(s, "\noutput");
    let _ = writeln!(
        s,
        "  {}x{} @ {} fps · quality {} · encoder {} · preset {}",
        cfg.width, cfg.height, cfg.fps, cfg.quality, cfg.encoder, cfg.preset
    );
    let _ = writeln!(
        s,
        "  skin config: {}",
        if cfg.last_config.is_some() { "loaded" } else { "defaults" }
    );
    let _ = writeln!(
        s,
        "  game assets: {}",
        if cfg.game_assets.is_some() { "connected" } else { "none" }
    );

    std::fs::write(&path, s).map_err(err_str)?;
    Ok(path)
}

fn verify_dto(replay: &Replay, map: &Map, hash_mismatch: bool) -> VerifyDto {
    let report = integrity::verify_replay(replay, map);
    let problems = report
        .failed_checks()
        .filter(|c| c.severity == integrity::Severity::Error)
        .map(|c| format!("{}: expected {}, got {}", c.name, c.expected, c.actual))
        .collect();
    let consistent = report.consistent();
    VerifyDto {
        consistent,
        problems,
        wrong_map: !consistent && wrong_map(replay, &report, hash_mismatch),
    }
}

/// Whether a failed check is better explained by the loaded map than by the
/// replay. A replay's hit flags only line up with the chart they were played
/// on, so against a foreign map most of them match nothing at all — a state
/// no edit produces, since editing a replay keeps its own totals coherent.
fn wrong_map(replay: &Replay, report: &integrity::IntegrityReport, hash_mismatch: bool) -> bool {
    if hash_mismatch {
        return true;
    }
    let flags = report.flagged_frames;
    let header_hits = replay.hits.max(0) as u32;
    // A wrong map cannot change the FILE: the number of flagged frames still
    // matches the header it was written with. A header inflated past its own
    // frames is the opposite — evidence about the replay, not the chart — so
    // it must keep reading as inconsistent instead of being explained away.
    if flags != header_hits {
        return false;
    }
    // With an honest header, a third of the recorded hits finding no note at
    // all, or barely half of them landing, is what a foreign chart looks
    // like. It is also what injected hit flags would look like, which is why
    // the wording this feeds is "may not match" rather than a verdict — only
    // the map hash can tell those apart, and that is the branch above.
    let orphan_heavy = flags > 20 && u64::from(report.orphan_flags) * 3 > u64::from(flags);
    let lost_most = header_hits > 20 && report.derived_hits * 2 < header_hits;
    orphan_heavy || lost_most
}

/// Overlays and meter chrome flip to dark strokes on bright skin
/// backgrounds — one rule, used by the HUD and the analyze overlay.
fn is_light_background(cfg: &rhythia_render::config::SkinConfig) -> bool {
    let bg = cfg.background_color;
    0.299 * bg[0] + 0.587 * bg[1] + 0.114 * bg[2] > 0.55
}

fn assemble_status(inner: &Inner, rendering: bool) -> StatusDto {
    let replay = inner.replay.as_ref().map(|(path, r)| {
        let verify = inner
            .map
            .as_ref()
            .map(|(_, m)| verify_dto(r, m, inner.map_hash_mismatch));
        let mods: Vec<String> = serde_json::from_str(&r.mods).unwrap_or_default();
        ReplayDto {
            path: path.to_string_lossy().into_owned(),
            file_name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            player: r.player_name.clone(),
            map_id: r.map_id,
            legacy_map_id: r.legacy_map_id.clone(),
            speed: r.speed,
            mods,
            passed: r.passed,
            failed: r.failed(),
            fail_time_ms: r.fail_time_ms,
            length_ms: r.length_ms(),
            hits: r.hits,
            misses: r.misses,
            accuracy_pct: r.accuracy_pct,
            total_score: r.total_score,
            points: r.points,
            unix_ms: r.unix_ms(),
            verify,
        }
    });
    let map = inner.map.as_ref().map(|(path, m)| MapDto {
        path: path.to_string_lossy().into_owned(),
        title: m.meta.title.clone(),
        song_name: m.meta.song_name.clone(),
        note_count: m.notes.len(),
        duration_ms: m.meta.duration_ms,
        has_audio: m.audio.is_some(),
        has_cover: m.cover.is_some(),
        source: inner.map_source.clone(),
        hash_mismatch: inner.map_hash_mismatch,
    });
    let ghost = inner.ghost.as_ref().map(|(path, g)| GhostDto {
        path: path.to_string_lossy().into_owned(),
        file_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        player: g.player_name.clone(),
        same_map: inner
            .replay
            .as_ref()
            .map(|(_, r)| g.map_id == r.map_id)
            .unwrap_or(false),
    });
    let base_hud = hud_flags(&inner.base_config);
    let effective_hud = hud_flags(&effective_config(inner));
    let game_ok = inner
        .settings
        .game_assets
        .as_ref()
        .map(|p| {
            let d = Path::new(p);
            d.join("builtin_colorsets.json").is_file() || d.join("builtin_assets").is_dir()
        })
        .unwrap_or(false);
    StatusDto {
        replay,
        ghost,
        map,
        config: ConfigDto {
            path: inner
                .config_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            base_hud,
            effective_hud,
        },
        settings: inner.settings.clone(),
        rendering,
        game_ok,
        bg_video_duration: inner.bg_duration,
        clip: inner.clip,
        preview_height: if inner.preview_height >= 240 { inner.preview_height } else { PREVIEW_H },
        light_background: is_light_background(&inner.base_config),
        build: env!("RHYTHR_BUILD").to_string(),
        can_undo: !inner.undo_stack.is_empty(),
        can_redo: !inner.redo_stack.is_empty(),
    }
}

fn png_bytes(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(std::io::Cursor::new(&mut buf), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(err_str)?;
        writer.write_image_data(rgba).map_err(err_str)?;
    }
    Ok(buf)
}


/// Keeps the map cache below ~2 GiB by deleting the oldest downloads
/// (there is no other eviction; maps are ~10-50 MB each).
fn evict_map_cache(keep_id: i32) {
    const MAX_CACHE_BYTES: u64 = 2 << 30;
    let dir = maps_cache_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((
                meta.modified().ok()?,
                meta.len(),
                e.path(),
            ))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    files.sort_by_key(|(mtime, _, _)| *mtime);
    let keep = format!("{keep_id}.");
    for (_, len, path) in files {
        if total <= MAX_CACHE_BYTES {
            break;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with(&keep) {
            continue; // never evict the map just downloaded
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// The server-side hash recorded when a map was downloaded into the cache.
fn cached_map_hash(map_id: i32) -> Option<String> {
    let meta = maps_cache_dir().join(format!("{map_id}.meta.json"));
    let text = std::fs::read_to_string(meta).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v["mapHash"].as_str().map(str::to_owned)
}

/// Looks for a cached download of the replay's map; validates the cached
/// hash against the replay header (an updated map must not silently render
/// the wrong notes).
fn try_cached_map(replay: &Replay) -> Option<(PathBuf, Map)> {
    if replay.map_id <= 0 {
        return None;
    }
    let sspm = maps_cache_dir().join(format!("{}.sspm", replay.map_id));
    if !sspm.exists() {
        return None;
    }
    let cached_hash = cached_map_hash(replay.map_id).unwrap_or_default();
    let mismatch = !replay.beatmap_hash.is_empty()
        && !cached_hash.is_empty()
        && cached_hash != replay.beatmap_hash;
    if mismatch {
        // Stale cache — the caller should re-download.
        return None;
    }
    let map = Map::from_path(&sspm).ok()?;
    Some((sspm, map))
}

/// Invalidate the cached preview pipeline (config/replay/map changed).
/// Rescales wall-clock replays (speed already applied to their frame
/// times) into song time as soon as replay and map are paired, so every
/// consumer — preview, timeline, verify badge, render — sees one
/// consistent base. Idempotent; no-op for well-formed replays.
fn normalize_time_bases(inner: &mut Inner) {
    let Some((_, map)) = &inner.map else { return };
    let map = map.clone();
    let mut changed = false;
    if let Some((_, r)) = &mut inner.replay {
        changed |= rhythia_sim::timebase::normalize(r, &map);
    }
    if let Some((_, g)) = &mut inner.ghost {
        changed |= rhythia_sim::timebase::normalize(g, &map);
    }
    if changed {
        invalidate_preview(inner);
    }
}

fn invalidate_preview(inner: &mut Inner) {
    inner.preview = None;
    inner.frame_gen = inner.frame_gen.wrapping_add(1);
}

// ---------------------------------------------------------------- commands

#[tauri::command]
async fn get_status(state: tauri::State<'_, App>) -> Result<StatusDto, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let inner = app.lock();
        assemble_status(&inner, app.rendering.load(Ordering::SeqCst))
    })
    .await
    .map_err(err_str)
}

/// Nudges the Analyze window after a source change so it reloads without
/// waiting for its poll.
fn notify_sources_changed(app_handle: &tauri::AppHandle) {
    let _ = app_handle.emit("sources-changed", ());
}

#[tauri::command]
fn load_replay(state: tauri::State<'_, App>, path: String, app_handle: tauri::AppHandle) -> Result<StatusDto, String> {
    let app = state.inner();
    let replay = Replay::from_path(&path).map_err(err_str)?;
    let mut inner = app.lock();
    // Keep the map when it still belongs to this replay (same online id) or
    // when the user picked it manually — the verify badge flags a true
    // mismatch. Auto-resolved maps for another id are swapped out.
    let keep_map = inner.map.is_some()
        && (inner.map_source == "local"
            || matches!(&inner.replay, Some((_, old)) if old.map_id == replay.map_id));
    if !keep_map {
        inner.map = None;
        inner.map_source.clear();
        inner.map_hash_mismatch = false;
        if let Some((p, m)) = try_cached_map(&replay) {
            inner.map = Some((p, m));
            inner.map_source = "cache".into();
        }
    } else if inner.map_source != "local" {
        // Same map, different replay: the stored mismatch flag belongs to
        // the old replay's hash — recompute against the new one.
        inner.map_hash_mismatch = cached_map_hash(replay.map_id)
            .is_some_and(|h| !replay.beatmap_hash.is_empty() && h != replay.beatmap_hash);
    }
    // A loaded ghost belongs to the previous replay; drop it when it no
    // longer fits the new one (other map, or a speed it cannot race).
    if let Some((_, g)) = &inner.ghost {
        let other_map =
            g.map_id != replay.map_id && !g.beatmap_hash.is_empty() && g.beatmap_hash != replay.beatmap_hash;
        if other_map || (g.speed - replay.speed).abs() > 0.005 {
            inner.ghost = None;
        }
    }
    // A clip range belongs to the run it was set on — a different replay
    // must not inherit it (a shorter run would collapse it to nothing).
    inner.clip = None;
    inner.settings.last_replay = Some(path.clone());
    let recent = &mut inner.settings.recent_replays;
    recent.retain(|p| p != &path);
    recent.insert(0, path.clone());
    recent.truncate(8);
    inner.settings.save();
    inner.replay = Some((PathBuf::from(path), replay));
    normalize_time_bases(&mut inner);
    invalidate_preview(&mut inner);
    notify_sources_changed(&app_handle);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn load_map(state: tauri::State<'_, App>, path: String, app_handle: tauri::AppHandle) -> Result<StatusDto, String> {
    let app = state.inner();
    let map = Map::from_path(&path).map_err(err_str)?;
    let mut inner = app.lock();
    inner.map = Some((PathBuf::from(path), map));
    inner.map_source = "local".into();
    inner.map_hash_mismatch = false;
    normalize_time_bases(&mut inner);
    invalidate_preview(&mut inner);
    notify_sources_changed(&app_handle);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
async fn download_map(
    state: tauri::State<'_, App>,
    app_handle: tauri::AppHandle,
) -> Result<StatusDto, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (map_id, replay_hash) = {
            let inner = app.lock();
            let (_, r) = inner.replay.as_ref().ok_or("no replay loaded")?;
            (r.map_id, r.beatmap_hash.clone())
        };
        if map_id <= 0 {
            return Err("replay has no online map id".to_string());
        }
        // Resolve the map page -> .sspm URL + server-side hash.
        let resp: serde_json::Value = ureq::post(API_BEATMAP_PAGE)
            .set("User-Agent", USER_AGENT)
            .send_json(serde_json::json!({"session": "", "id": map_id, "limit": 1}))
            .map_err(|e| match e {
                ureq::Error::Status(429, _) => {
                    "rhythia.com is rate-limiting requests — please wait a moment and press Download".to_string()
                }
                ureq::Error::Status(code, _) if code >= 500 => {
                    format!("rhythia.com is unavailable right now (HTTP {code}) — try again later")
                }
                e => format!("map lookup failed: {e}"),
            })?
            .into_json()
            .map_err(|e| format!("map lookup: bad response: {e}"))?;
        let beatmap = &resp["beatmap"];
        let file_url = beatmap["beatmapFile"]
            .as_str()
            .ok_or("map lookup: no beatmapFile in response")?
            .to_string();
        let map_hash = beatmap["mapHash"].as_str().unwrap_or("").to_string();
        let title = beatmap["title"].as_str().unwrap_or("").to_string();
        let hash_mismatch =
            !replay_hash.is_empty() && !map_hash.is_empty() && replay_hash != map_hash;

        let mut bytes = Vec::new();
        ureq::get(&file_url)
            .set("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| format!("map download failed: {e}"))?
            .into_reader()
            .take(MAX_MAP_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("map download failed: {e}"))?;
        let map = rhythia_formats::sspm::parse(&bytes)
            .or_else(|_| Map::from_rhm(&bytes))
            .map_err(|e| format!("downloaded map does not parse: {e}"))?;

        let dir = maps_cache_dir();
        std::fs::create_dir_all(&dir).map_err(err_str)?;
        let sspm_path = dir.join(format!("{map_id}.sspm"));
        std::fs::write(&sspm_path, &bytes).map_err(err_str)?;
        let meta = serde_json::json!({"mapHash": map_hash, "title": title, "mapId": map_id});
        let _ = std::fs::write(
            dir.join(format!("{map_id}.meta.json")),
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        );
        evict_map_cache(map_id);

        let mut inner = app.lock();
        // The download is slow; the user may have loaded a different replay
        // meanwhile. The cache write above still counts — but don't pair
        // this map with a replay it doesn't belong to.
        let still_wanted = inner
            .replay
            .as_ref()
            .is_some_and(|(_, r)| r.map_id == map_id);
        if still_wanted {
            inner.map = Some((sspm_path, map));
            inner.map_source = "downloaded".into();
            inner.map_hash_mismatch = hash_mismatch;
            normalize_time_bases(&mut inner);
            invalidate_preview(&mut inner);
        }
        Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
    })
    .await
    .map_err(err_str)?
    .inspect(|_| notify_sources_changed(&app_handle))
}

#[tauri::command]
fn load_config(
    state: tauri::State<'_, App>,
    app_handle: tauri::AppHandle,
    path: String,
) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let p = Some(PathBuf::from(&path));
    let cfg = load_base_config(&p, &inner.settings.game_assets)?;
    inner.base_config = cfg;
    inner.config_path = p;
    inner.settings.last_config = Some(path);
    inner.settings.save();
    invalidate_preview(&mut inner);
    // The analyze window renders with this config baked in (live engine,
    // cached geometry, overlay palette) — it must rebuild.
    notify_sources_changed(&app_handle);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn clear_config(
    state: tauri::State<'_, App>,
    app_handle: tauri::AppHandle,
) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    inner.config_path = None;
    inner.base_config = load_base_config(&None, &inner.settings.game_assets)?;
    inner.settings.last_config = None;
    inner.settings.save();
    invalidate_preview(&mut inner);
    notify_sources_changed(&app_handle);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

/// Where exe-extracted assets live. One fixed location: re-extracting
/// (e.g. after a game update) simply overwrites it.
fn assets_cache_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rhythr")
        .join("game-assets")
}

#[tauri::command]
async fn set_game_assets(
    state: tauri::State<'_, App>,
    path: Option<String>,
) -> Result<StatusDto, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut resolved = path.filter(|p| !p.trim().is_empty());
        // A rhythia.exe gets its skin assets extracted locally; the config
        // then resolves against the extracted copy. Extraction runs into a
        // temp dir first and only replaces the live cache once validated —
        // a failed/partial run must not pollute a previously good cache.
        if let Some(p) = &resolved {
            // Any file is treated as the game binary (rhythia.exe under
            // Windows/Proton, an extensionless ELF for the native Linux
            // build — the bundle format is the same); a directory is an
            // already-extracted assets folder.
            if Path::new(p).is_file() {
                // One extraction at a time (second click while running).
                static EXTRACTING: AtomicBool = AtomicBool::new(false);
                if EXTRACTING.swap(true, Ordering::SeqCst) {
                    return Err("an extraction is already running".into());
                }
                let result = (|| {
                    let cache = assets_cache_dir();
                    let tmp = cache.with_extension("tmp");
                    let _ = std::fs::remove_dir_all(&tmp);
                    let n = rhythia_render::exe_assets::extract_to_dir(Path::new(p), &tmp)?;
                    if n < 50 {
                        let _ = std::fs::remove_dir_all(&tmp);
                        return Err(format!(
                            "only {n} assets found in this exe — unexpected; not using it"
                        ));
                    }
                    let _ = std::fs::remove_dir_all(&cache);
                    std::fs::rename(&tmp, &cache).map_err(|e| e.to_string())?;
                    Ok(cache.to_string_lossy().into_owned())
                })();
                EXTRACTING.store(false, Ordering::SeqCst);
                resolved = Some(result?);
            }
        }
        let mut inner = app.lock();
        inner.settings.game_assets = resolved;
        let cfg = load_base_config(&inner.config_path, &inner.settings.game_assets)?;
        inner.base_config = cfg;
        inner.settings.save();
        invalidate_preview(&mut inner);
        Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
    })
    .await
    .map_err(err_str)?
}

/// Steam's install path from the registry (custom install drives). Read
/// via the registry API, not `reg query`: reg.exe writes piped output in
/// the legacy OEM/ANSI codepage, which mangles non-ASCII install paths
/// (D:\Spiele\…, D:\Игры\…) into U+FFFD and breaks the whole scan.
#[cfg(windows)]
fn windows_steam_path() -> Option<PathBuf> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam")
        .ok()?;
    let path: String = key.get_value("SteamPath").ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| PathBuf::from(path.replace('/', "\\")))
}

#[cfg(not(windows))]
fn windows_steam_path() -> Option<PathBuf> {
    None
}

/// Finds the game binary across every Steam library (registry/default
/// roots plus libraryfolders.vdf). Works for Windows installs, Proton
/// installs and the native Linux build alike — the extraction is
/// file-based and the .NET bundle layout is the same everywhere.
#[tauri::command]
async fn detect_game() -> Option<String> {
    // The scan touches every Steam library on every drive; a spun-down
    // HDD or dead network mapping blocks for seconds, and sync commands
    // run on the UI thread — so it goes through the blocking pool like
    // every other filesystem-heavy command here.
    tauri::async_runtime::spawn_blocking(detect_game_scan)
        .await
        .ok()
        .flatten()
}

fn detect_game_scan() -> Option<String> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        // The registry knows custom install locations Steam was moved to.
        if let Some(p) = windows_steam_path() {
            roots.push(p);
        }
        for base in [
            "C:\\Program Files (x86)\\Steam",
            "C:\\Program Files\\Steam",
        ] {
            roots.push(PathBuf::from(base));
        }
    } else if let Some(home) = dirs::home_dir() {
        for rel in [
            ".local/share/Steam",
            ".steam/steam",
            ".steam/root",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
            "snap/steam/common/.local/share/Steam",
        ] {
            roots.push(home.join(rel));
        }
    }
    // Extra Steam libraries.
    let mut libs: Vec<PathBuf> = Vec::new();
    for root in &roots {
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(vdf) {
            for line in text.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("\"path\"") {
                    let p = rest.trim().trim_matches('"').replace("\\\\", "\\");
                    libs.push(PathBuf::from(p));
                }
            }
        }
    }
    roots.extend(libs);
    // Scan every library's steamapps/common for a folder that mentions
    // the game, then take the largest plausible game binary inside it:
    // rhythia.exe under Windows/Proton, an extensionless ELF for the
    // native Linux build. Names are matched case-insensitively — installs
    // exist as "Rhythia", "rhythia" and "SoundSpacePlus".
    let mut best: Option<(u64, PathBuf)> = None;
    for root in roots {
        let common = root.join("steamapps").join("common");
        let Ok(entries) = std::fs::read_dir(&common) else {
            continue;
        };
        for e in entries.flatten() {
            let dir_name = e.file_name().to_string_lossy().to_lowercase();
            if !(dir_name.contains("rhythia") || dir_name.contains("sound space") || dir_name.contains("soundspace")) {
                continue;
            }
            if let Some((size, p)) = game_binary_in(&e.path()) {
                if best.as_ref().is_none_or(|(sz, _)| size > *sz) {
                    best = Some((size, p));
                }
            }
        }
    }
    best.map(|(_, p)| p.to_string_lossy().into_owned())
}

/// The largest plausibly-the-game binary directly inside `dir`: name
/// starts with the game's, is an .exe / extensionless / .x86_64 file, and
/// is big enough to be the ~280 MB single-file bundle.
fn game_binary_in(dir: &Path) -> Option<(u64, PathBuf)> {
    let mut best: Option<(u64, PathBuf)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_lowercase();
        let looks_like = name.starts_with("rhythia") || name.starts_with("sound space") || name.starts_with("soundspace");
        let ext_ok = name.ends_with(".exe") || name.ends_with(".x86_64") || !name.contains('.');
        if !looks_like || !ext_ok {
            continue;
        }
        // fs::metadata (not DirEntry::metadata) so symlinked game files —
        // common in hand-built Steam libraries — report their real size.
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.len() < 20_000_000 {
            continue;
        }
        if best.as_ref().is_none_or(|(sz, _)| meta.len() > *sz) {
            best = Some((meta.len(), path));
        }
    }
    best
}

#[tauri::command]
fn set_hud_override(
    state: tauri::State<'_, App>,
    key: String,
    value: Option<bool>,
) -> Result<StatusDto, String> {
    if !HUD_KEYS.contains(&key.as_str()) {
        return Err(format!("unknown HUD element: {key}"));
    }
    let app = state.inner();
    let mut inner = app.lock();
    remember_layout(&mut inner);
    let _ = &inner; // (override lands via the per-frame hud sync)
    match value {
        Some(v) => {
            inner.settings.hud_overrides.insert(key, v);
        }
        None => {
            inner.settings.hud_overrides.remove(&key);
        }
    }
    inner.settings.save();
    touch_frames(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn load_ghost(state: tauri::State<'_, App>, path: String, app_handle: tauri::AppHandle) -> Result<StatusDto, String> {
    let app = state.inner();
    let ghost = Replay::from_path(&path).map_err(err_str)?;
    let mut inner = app.lock();
    if let Some((_, r)) = &inner.replay {
        if ghost.map_id != r.map_id && !ghost.beatmap_hash.is_empty() && ghost.beatmap_hash != r.beatmap_hash {
            return Err("that replay was played on a different map".into());
        }
        // Both runs share one timeline and one audio track, so the speed
        // must match; other mods (mirror, hardrock) may differ per side.
        if (ghost.speed - r.speed).abs() > 0.005 {
            return Err(format!(
                "speed mods must match: your replay is {:.2}x, the ghost {:.2}x",
                r.speed, ghost.speed
            ));
        }
    }
    inner.ghost = Some((PathBuf::from(path), ghost));
    normalize_time_bases(&mut inner);
    invalidate_preview(&mut inner);
    notify_sources_changed(&app_handle);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn clear_ghost(state: tauri::State<'_, App>, app_handle: tauri::AppHandle) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    inner.ghost = None;
    invalidate_preview(&mut inner);
    notify_sources_changed(&app_handle);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

/// Stores a dragged HUD element's new centre (normalised to the frame —
/// or to one half in a ghost split) and refreshes the preview. Saved
/// immediately: the render always matches the live preview.
#[tauri::command]
fn set_hud_position(
    state: tauri::State<'_, App>,
    key: String,
    x: f32,
    y: f32,
    commit: Option<bool>,
) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    inner
        .settings
        .hud_positions
        .insert(key, [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)]);
    if commit.unwrap_or(true) {
        inner.settings.save();
    }
    touch_frames(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

/// The drag editor's corner-handle resize. A scale close to 1 removes the
/// entry — back to the standard size, no leftover override.
#[tauri::command]
fn set_hud_scale(
    state: tauri::State<'_, App>,
    key: String,
    scale: f32,
    commit: Option<bool>,
) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let s = scale.clamp(0.4, 2.5);
    if (s - 1.0).abs() < 0.02 {
        inner.settings.hud_scales.remove(&key);
    } else {
        inner.settings.hud_scales.insert(key, s);
    }
    if commit.unwrap_or(true) {
        inner.settings.save();
    }
    touch_frames(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[derive(Serialize, Clone)]
struct HudBoxDto {
    key: String,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

/// The drag editor's hitboxes at the given song time, in preview-frame
/// pixels — computed by the renderer from the very vertices it draws, so
/// box and pixels cannot drift apart.
#[tauri::command]
async fn hud_layout(state: tauri::State<'_, App>, time_ms: f64) -> Result<Vec<HudBoxDto>, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut inner = app.lock();
        {
            let Inner { preview, settings, base_config, .. } = &mut *inner;
            if let Some(ctx) = preview.as_mut() {
                apply_hud_settings(&mut ctx.cfg, base_config, settings);
            }
        }
        let inner = &*inner;
        let ctx = inner.preview.as_ref().ok_or("no preview yet")?;
        let (_, r) = inner.replay.as_ref().ok_or("no replay loaded")?;
        let boxes = ctx.renderer.hud_boxes(
            &ctx.params,
            &ctx.cfg,
            r,
            &ctx.map,
            time_ms,
            &ctx.hud,
            ctx.ghost.is_some(),
        );
        Ok(boxes
            .into_iter()
            .map(|b| HudBoxDto {
                key: b.key.to_string(),
                x0: b.x0,
                y0: b.y0,
                x1: b.x1,
                y1: b.y1,
            })
            .collect())
    })
    .await
    .map_err(err_str)?
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MeterPatch {
    enabled: Option<bool>,
    x: Option<f32>,
    y: Option<f32>,
    ghost_x: Option<f32>,
    ghost_y: Option<f32>,
    scale: Option<f32>,
    alpha: Option<f32>,
}

#[tauri::command]
fn set_meter(
    state: tauri::State<'_, App>,
    key: String,
    patch: MeterPatch,
    commit: Option<bool>,
) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let m = match key.as_str() {
        "error" => &mut inner.settings.error_meter,
        "aim" => &mut inner.settings.aim_meter,
        "race_delta" => &mut inner.settings.race_delta,
        _ => return Err(format!("unknown meter: {key}")),
    };
    if let Some(v) = patch.enabled {
        m.enabled = v;
    }
    if let Some(v) = patch.x {
        m.x = v.clamp(0.0, 1.0);
    }
    if let Some(v) = patch.y {
        m.y = v.clamp(0.0, 1.0);
    }
    if let Some(v) = patch.ghost_x {
        m.ghost_x = Some(v.clamp(0.0, 1.0));
    }
    if let Some(v) = patch.ghost_y {
        m.ghost_y = Some(v.clamp(0.0, 1.0));
    }
    if let Some(v) = patch.scale {
        m.scale = v.clamp(0.4, 2.5);
    }
    if let Some(v) = patch.alpha {
        m.alpha = v.clamp(0.05, 1.0);
    }
    if commit.unwrap_or(true) {
        inner.settings.save();
    }
    touch_frames(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

/// Puts every draggable HUD element back where the game/config puts it:
/// the drag-editor overrides and the meters' positions — nothing else
/// (visibility, scale and opacity survive).
#[tauri::command]
fn reset_hud_layout(state: tauri::State<'_, App>) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let backup = preset_snapshot(&inner);
    inner.settings.presets.insert("Before reset".into(), backup);
    remember_layout(&mut inner);
    inner.settings.hud_positions.clear();
    inner.settings.hud_scales.clear();
    fn park(m: &mut MeterSettings, d: MeterSettings) {
        m.x = d.x;
        m.y = d.y;
        m.ghost_x = None;
        m.ghost_y = None;
    }
    park(&mut inner.settings.error_meter, MeterSettings::at(0.5, 0.88));
    park(&mut inner.settings.aim_meter, MeterSettings::at(0.15, 0.32));
    park(&mut inner.settings.race_delta, MeterSettings::at(0.5, 0.095));
    inner.settings.save();
    touch_frames(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

/// Sets (or clears, with None) the custom playfield background. Validates
/// up front: images must fully decode, videos must be readable by the
/// bundled ffmpeg — so a bad file errors here instead of rendering black.
#[tauri::command]
fn set_background(state: tauri::State<'_, App>, path: Option<String>) -> Result<StatusDto, String> {
    use rhythia_render::background as bg;
    let app = state.inner();
    let mut inner = app.lock();
    let mut duration = None;
    if let Some(p) = &path {
        let pb = PathBuf::from(p);
        let kind = bg::classify_file(&pb).map_err(|e| format!("could not read background: {e}"))?;
        match kind {
            bg::BackgroundKind::Image => {
                let bytes = std::fs::read(&pb).map_err(|e| format!("could not read background: {e}"))?;
                if !bg::image_decodes(&bytes) {
                    return Err("could not decode this image".into());
                }
            }
            bg::BackgroundKind::Video => {
                let ffmpeg = resolve_ffmpeg(&inner.settings);
                duration = bg::probe_duration(&ffmpeg, &pb);
                if duration.is_none() {
                    return Err(
                        "ffmpeg could not read this file — unsupported or corrupt".into()
                    );
                }
            }
        }
    }
    // A different file means a different intro — the start point resets.
    if inner.settings.background != path {
        inner.settings.background_start = 0.0;
    }
    inner.bg_duration = duration;
    inner.settings.background = path;
    inner.settings.save();
    invalidate_preview(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

/// The user's zoom/shift/start placement of the custom background. All
/// fields optional — a patch, like set_meter.
#[derive(Deserialize, Default)]
#[serde(default)]
struct BackgroundTransform {
    zoom: Option<u32>,
    off_x: Option<i32>,
    off_y: Option<i32>,
    start: Option<f64>,
    sync_song: Option<bool>,
}

#[tauri::command]
fn set_background_transform(
    state: tauri::State<'_, App>,
    patch: BackgroundTransform,
) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let s = &mut inner.settings;
    if let Some(v) = patch.zoom {
        s.background_zoom = v.clamp(100, 400);
    }
    if let Some(v) = patch.off_x {
        s.background_off_x = v.clamp(-100, 100);
    }
    if let Some(v) = patch.off_y {
        s.background_off_y = v.clamp(-100, 100);
    }
    if let Some(v) = patch.start {
        s.background_start = v.max(0.0);
    }
    if let Some(v) = patch.sync_song {
        s.background_sync_song = v;
    }
    inner.settings.save();
    // Video backgrounds read zoom/shift/start live at frame extraction;
    // only an image needs its composite rebuilt.
    if inner.bg_duration.is_none() {
        invalidate_preview(&mut inner);
    }
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn save_preset(state: tauri::State<'_, App>, name: String) -> Result<StatusDto, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("give the preset a name first".into());
    }
    if name.len() > 48 {
        return Err("preset name too long".into());
    }
    let app = state.inner();
    let mut inner = app.lock();
    let snap = preset_snapshot(&inner);
    inner.settings.presets.insert(name, snap);
    inner.settings.save();
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn apply_preset(state: tauri::State<'_, App>, name: String) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let Some(p) = inner.settings.presets.get(&name).cloned() else {
        return Err(format!("no preset named {name}"));
    };
    remember_layout(&mut inner);
    apply_layout_only(&mut inner.settings, &p);
    inner.settings.width = p.width;
    inner.settings.height = p.height;
    inner.settings.background = p.background.clone();
    inner.settings.background_dim = p.background_dim;
    inner.settings.background_zoom = p.background_zoom;
    inner.settings.background_off_x = p.background_off_x;
    inner.settings.background_off_y = p.background_off_y;
    inner.settings.background_start = p.background_start;
    inner.settings.background_sync_song = p.background_sync_song;
    // Re-probe the background video's duration for the start slider.
    inner.bg_duration = p.background.as_ref().and_then(|b| {
        let pb = PathBuf::from(b);
        (rhythia_render::background::classify_file(&pb).ok()
            == Some(rhythia_render::background::BackgroundKind::Video))
        .then(|| {
            rhythia_render::background::probe_duration(&resolve_ffmpeg(&inner.settings), &pb)
        })
        .flatten()
    });
    // The preset's skin config — falls back to defaults when it had none
    // or the file is gone (the rest of the preset still applies).
    let cfg_path = p.config_path.as_ref().map(PathBuf::from);
    match load_base_config(&cfg_path, &inner.settings.game_assets) {
        Ok(cfg) => {
            inner.base_config = cfg;
            inner.config_path = cfg_path;
            inner.settings.last_config = p.config_path.clone();
        }
        Err(_) => { /* keep the current config */ }
    }
    inner.settings.save();
    invalidate_preview(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn delete_preset(state: tauri::State<'_, App>, name: String) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    inner.settings.presets.remove(&name);
    inner.settings.save();
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn undo_layout(state: tauri::State<'_, App>) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let Some(p) = inner.undo_stack.pop() else {
        return Err("nothing to undo".into());
    };
    let now = preset_snapshot(&inner);
    inner.redo_stack.push(now);
    apply_layout_only(&mut inner.settings, &p);
    inner.settings.save();
    touch_frames(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn redo_layout(state: tauri::State<'_, App>) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let Some(p) = inner.redo_stack.pop() else {
        return Err("nothing to redo".into());
    };
    let now = preset_snapshot(&inner);
    if inner.undo_stack.len() >= 50 {
        inner.undo_stack.remove(0);
    }
    inner.undo_stack.push(now);
    apply_layout_only(&mut inner.settings, &p);
    inner.settings.save();
    touch_frames(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

/// One undo snapshot per drag gesture: the frontend calls this on
/// pointer-down, then streams live positions without touching history.
#[tauri::command]
fn mark_undo(state: tauri::State<'_, App>) -> Result<(), String> {
    let app = state.inner();
    let mut inner = app.lock();
    remember_layout(&mut inner);
    Ok(())
}

/// Sets the clip range (song ms) rendered instead of the full run.
#[tauri::command]
fn set_clip(state: tauri::State<'_, App>, start_ms: f64, end_ms: f64) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    if end_ms - start_ms < 500.0 {
        return Err("clip is too short — give it at least half a second".into());
    }
    inner.clip = Some((start_ms.max(0.0), end_ms));
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn clear_clip(state: tauri::State<'_, App>) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    inner.clip = None;
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn set_background_dim(state: tauri::State<'_, App>, pct: u32) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    inner.settings.background_dim = pct.min(100);
    inner.settings.save();
    touch_frames(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn reset_hud_overrides(state: tauri::State<'_, App>) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let backup = preset_snapshot(&inner);
    inner.settings.presets.insert("Before reset".into(), backup);
    remember_layout(&mut inner);
    inner.settings.hud_overrides.clear();
    inner.settings.hud_positions.clear();
    inner.settings.hud_scales.clear();
    inner.settings.save();
    touch_frames(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OutputUpdate {
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<u32>,
    quality: Option<u32>,
    tcp_feed: Option<bool>,
    encoder: Option<String>,
    preset: Option<String>,
    results_secs: Option<f64>,
    motion_blur: Option<u32>,
    music_volume: Option<u32>,
    hitsound_volume: Option<u32>,
    output_dir: Option<String>,
    file_name: Option<String>,
    ffmpeg: Option<String>,
}

#[tauri::command]
fn set_output(state: tauri::State<'_, App>, update: OutputUpdate) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let was_portrait = inner.settings.height > inner.settings.width;
    let s = &mut inner.settings;
    if let Some(v) = update.width {
        s.width = v.clamp(320, 7680);
    }
    if let Some(v) = update.height {
        s.height = v.clamp(240, 4320);
    }
    if let Some(v) = update.fps {
        s.fps = v.clamp(24, 240);
    }
    if let Some(v) = update.quality {
        s.quality = v.clamp(rhythia_render::quality::MIN, rhythia_render::quality::MAX);
    }
    if let Some(v) = update.tcp_feed {
        s.tcp_feed = v;
    }
    if let Some(v) = update.encoder {
        s.encoder = v;
    }
    if let Some(v) = update.preset {
        s.preset = v;
    }
    if let Some(v) = update.results_secs {
        s.results_secs = v.clamp(0.0, 30.0);
    }
    if let Some(v) = update.motion_blur {
        s.motion_blur = v.min(2);
    }
    if let Some(v) = update.music_volume {
        s.music_volume = v.min(150);
    }
    if let Some(v) = update.hitsound_volume {
        s.hitsound_volume = v.min(150);
    }
    if let Some(v) = update.output_dir {
        s.output_dir = if v.trim().is_empty() { None } else { Some(v) };
    }
    if let Some(v) = update.file_name {
        s.file_name = v.trim().to_string();
    }
    if let Some(v) = update.ffmpeg {
        s.ffmpeg = if v.trim().is_empty() { None } else { Some(v) };
    }
    s.save();
    // Orientation flips rebuild the preview at the new aspect.
    if (inner.settings.height > inner.settings.width) != was_portrait {
        invalidate_preview(&mut inner);
    }
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn suggest_file_name(state: tauri::State<'_, App>) -> String {
    let app = state.inner();
    let inner = app.lock();
    suggested_name(&inner)
}

#[tauri::command]
fn timeline(state: tauri::State<'_, App>, samples: usize) -> Result<TimelineDto, String> {
    let app = state.inner();
    let inner = app.lock();
    let (_, replay) = inner.replay.as_ref().ok_or("no replay loaded")?;
    let n = samples.clamp(16, 2000);
    let run_end = if replay.failed() {
        replay.fail_time_ms as f64
    } else {
        replay.length_ms()
    };
    let mut health = vec![1.0f32; n];
    let mut level = 1.0f32;
    let mut fi = 0usize;
    let frames = &replay.frames;
    for (i, slot) in health.iter_mut().enumerate() {
        let t = run_end * (i as f64 + 1.0) / n as f64;
        while fi < frames.len() && frames[fi].ms <= t {
            level = frames[fi].health;
            fi += 1;
        }
        *slot = level;
    }
    let miss_times = inner
        .map
        .as_ref()
        .map(|(_, m)| {
            let window = rhythia_sim::hitreg::hit_window_ms(replay);
            let outcome = rhythia_sim::hitreg::match_hits(&m.notes, frames, window);
            outcome
                .results
                .iter()
                .filter(|r| !r.hit)
                .map(|r| m.notes[r.note_index].time_ms as f64)
                .filter(|&t| t <= run_end + window)
                .collect()
        })
        .unwrap_or_default();
    Ok(TimelineDto {
        length_ms: run_end,
        fail_ms: replay.failed().then_some(replay.fail_time_ms as f64),
        health,
        miss_times,
    })
}

#[derive(Serialize, Clone)]
struct NoteQuadDto {
    /// Index into this side's note list.
    i: u32,
    /// Four screen-space corners in preview pixels (TL, TR, BR, BL).
    pts: [[f32; 2]; 4],
    /// Approach depth — 0 at the hit plane.
    depth: f32,
}

#[derive(Serialize, Clone)]
struct SideProjDto {
    /// Viewport x offset and width in preview pixels.
    x: u32,
    w: u32,
    /// Column-major 4×4 view-projection matrix.
    m: [[f32; 4]; 4],
    /// Notes on screen right now, exactly as the renderer draws them —
    /// the overlay traces these instead of guessing a grid cell.
    notes: Vec<NoteQuadDto>,
    /// Playfield border quad; overlays clip to it.
    field: [[f32; 2]; 4],
}

#[derive(Serialize, Clone)]
struct PreviewFrameDto {
    img: String,
    w: u32,
    h: u32,
    sides: Vec<SideProjDto>,
}

#[tauri::command]
async fn preview(state: tauri::State<'_, App>, time_ms: f64) -> Result<String, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // The pipeline can be invalidated between the two steps (a setting
        // changed on another thread) — one retry covers that race.
        ensure_preview_ctx(&app, time_ms)?;
        let png = match render_frame_png(&app, time_ms) {
            Ok(p) => p,
            Err(_) => {
                ensure_preview_ctx(&app, time_ms)?;
                render_frame_png(&app, time_ms)?
            }
        };
        use base64::Engine as _;
        Ok(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png.as_slice())
        ))
    })
    .await
    .map_err(err_str)?
}

/// Renders a stretch of the replay to a real video file the Analyze
/// window can play back smoothly. `span_ms` of song time at `out_fps`
/// frames per song second: at 60 that is real time, at 240 it plays four
/// times slower without losing a single frame.
#[tauri::command]
fn prepare_segment(
    state: tauri::State<'_, App>,
    app_handle: tauri::AppHandle,
    start_ms: f64,
    span_ms: f64,
    height: u32,
    out_fps: u32,
) -> Result<u64, String> {
    let app = state.inner().clone();
    let token = app.segment_gen.fetch_add(1, Ordering::SeqCst) + 1;
    // Everything the render needs, copied out so the renderer never holds
    // the lock the still frames and the UI need.
    let (replay, map, cfg, mut params, ghost, ffmpeg, settings_w, settings_h, encoder, clip) = {
        let inner = app.lock();
        let (_, r) = inner.replay.as_ref().ok_or("no replay loaded")?;
        let (_, m) = inner.map.as_ref().ok_or("no map loaded")?;
        let cfg = effective_config(&inner);
        let params = SceneParams::from(&cfg);
        let ghost = inner.ghost.as_ref().map(|(_, g)| g.clone());
        (
            r.clone(),
            m.clone(),
            cfg,
            params,
            ghost,
            resolve_ffmpeg(&inner.settings),
            inner.settings.width,
            inner.settings.height,
            inner.settings.encoder.clone(),
            inner.clip,
        )
    };
    let (main_map, main_mods) = rhythia_render::mods::map_for_replay(&map, &replay);
    params.grid_scale = main_mods.grid_scale;
    params.apply_speed(replay.speed);

    let h = (height.clamp(360, 2160) / 2) * 2;
    let w = if settings_h > settings_w {
        ((h * settings_w / settings_h) / 2) * 2
    } else {
        ((h * 16 / 9) / 2) * 2
    };
    let fps = out_fps.clamp(30, 480);
    let run_end = clip.map(|c| c.1).unwrap_or(f64::INFINITY);
    let start = start_ms.max(0.0);
    let end = (start + span_ms.max(200.0)).min(run_end.max(start + 200.0));

    let dir = {
        let mut seg = app.segment.lock().unwrap_or_else(|p| p.into_inner());
        if seg.dir.is_none() {
            let d = std::env::temp_dir().join(format!("rhythr-segments-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&d);
            seg.dir = Some(d);
        }
        seg.dir.clone().unwrap()
    };
    let out = dir.join(format!("seg{token}.mp4"));

    let panic_out = out.clone();
    let panic_handle = app_handle.clone();
    std::thread::spawn(move || {
        // A panic in the job (wgpu device loss, driver reset, …) must still
        // reach the Analyze window: it clears its "Rendering …" pill only on
        // segment-ready or segment-error, so a silent death hangs it forever.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let emit_err = |msg: String| {
                let _ = app_handle.emit("segment-error", serde_json::json!({"token": token, "message": msg}));
            };
            let renderer = match rhythia_render::Renderer::new(w, h, cfg.hud_font.as_deref()) {
                Ok(r) => r,
                Err(e) => return emit_err(gpu_err(&e)),
            };
            let ghost_opts = ghost.map(|g| rhythia_render::video::GhostOptions {
                replay: g,
                color: GHOST_COLOR,
            });
            let opts = rhythia_render::video::VideoOptions {
                fps,
                start_ms: start,
                end_ms: end,
                ffmpeg,
                audio: None,
                // The preview is not the deliverable; deliberately below the
                // render default so scrubbing stays responsive.
                quality: rhythia_render::quality::from_legacy_crf(20),
                tcp_feed: false,
                preset: "ultrafast".into(),
                encoder,
                results_secs: 0.0,
                motion_blur: 0,
                music_volume: 0.0,
                hitsounds: None,
                ghost: ghost_opts,
                // Starts instantly and seeks anywhere: the moov atom up front
                // and a keyframe twice a second.
                extra_output_args: vec![
                    "-movflags".into(),
                    "+faststart".into(),
                    "-g".into(),
                    (fps / 2).max(1).to_string(),
                ],
                background_video: None,
            };
            let app2 = app.clone();
            let handle2 = app_handle.clone();
            let mut last_pct = u64::MAX;
            let res = rhythia_render::video::render_video(
                &renderer,
                &params,
                &cfg,
                &replay,
                &main_map,
                &out,
                &opts,
                move |done, total| {
                    if app2.segment_gen.load(Ordering::SeqCst) != token {
                        return false; // superseded — stop rendering
                    }
                    let pct = if total > 0 { done * 100 / total } else { 0 };
                    if pct != last_pct {
                        last_pct = pct;
                        let _ = handle2.emit(
                            "segment-progress",
                            serde_json::json!({"token": token, "pct": pct}),
                        );
                    }
                    true
                },
            );
            if app.segment_gen.load(Ordering::SeqCst) != token {
                let _ = std::fs::remove_file(&out);
                return;
            }
            match res {
                Ok(()) => {
                    {
                        let mut seg = app.segment.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(old) = seg.ready.take() {
                            let _ = std::fs::remove_file(&old.path);
                        }
                        seg.ready = Some(ReadySegment {
                            token,
                            path: out.clone(),
                            start_ms: start,
                            span_ms: end - start,
                            out_fps: fps,
                        });
                    }
                    let _ = app_handle.emit(
                        "segment-ready",
                        serde_json::json!({
                            "token": token,
                            "startMs": start,
                            "spanMs": end - start,
                            "outFps": fps,
                        }),
                    );
                }
                Err(rhythia_render::Error::Cancelled) => {
                    let _ = std::fs::remove_file(&out);
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&out);
                    emit_err(gpu_err(&e));
                }
            }
        }));
        if outcome.is_err() {
            let _ = std::fs::remove_file(&panic_out);
            let _ = panic_handle.emit(
                "segment-error",
                serde_json::json!({"token": token, "message": "segment renderer crashed"}),
            );
        }
    });
    Ok(token)
}

/// Starts the live Analyze engine: creates a wgpu surface on the main
/// thread (a Metal requirement), snapshots everything the render thread
/// needs, and spawns it. Returns false when live mode is unavailable.
#[tauri::command]
async fn start_live_session(
    state: tauri::State<'_, App>,
    live: tauri::State<'_, live::LiveHandles>,
    app_handle: tauri::AppHandle,
) -> Result<bool, String> {
    // Native mode ships on Windows; other platforms keep the proven
    // fallback engines unless explicitly forced (testing).
    let forced = std::env::var("RHYTHR_NATIVE_ANALYZE").is_ok();
    if !(cfg!(target_os = "windows") || forced) {
        return Ok(false);
    }
    // Flag the start so the close handler holds the window open while we
    // are between "old session taken" and "new session stored". Cleared
    // on every exit path via the guard below.
    if live.starting.swap(true, Ordering::SeqCst) {
        return Err("a live session start is already in flight".into());
    }
    struct StartFlag<'a>(&'a std::sync::atomic::AtomicBool);
    impl Drop for StartFlag<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _start_flag = StartFlag(&live.starting);

    let stopping = {
        let mut guard = live.session.lock().unwrap_or_else(|p| p.into_inner());
        guard.take()
    };
    if let Some(old) = stopping {
        // The old thread owns a swapchain on this window. Two flip-model
        // swapchains on one HWND is a DXGI error — wait for the old one
        // to actually release before creating a new surface.
        let _ = old.tx.send(live::LiveCmd::Stop);
        let flag = old.running.clone();
        let stopped = tauri::async_runtime::spawn_blocking(move || {
            for _ in 0..300 {
                if !flag.load(Ordering::SeqCst) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            false
        })
        .await
        .map_err(err_str)?;
        if !stopped {
            return Err("previous live session did not stop".into());
        }
    }
    let window = app_handle
        .get_webview_window("analyze")
        .ok_or("analyze window not open")?;
    let size = window.inner_size().map_err(err_str)?;

    let app = state.inner().clone();
    let init = {
        let inner = app.lock();
        let (_, r) = inner.replay.as_ref().ok_or("no replay loaded")?;
        let (_, m) = inner.map.as_ref().ok_or("no map loaded")?;
        // The analyze view keeps the SKIN's background: no custom
        // image/video layers in live mode (explicit user decision).
        let mut cfg = inner.base_config.clone();
        apply_hud_settings(&mut cfg, &inner.base_config, &inner.settings);
        let run_end = if r.failed() {
            r.fail_time_ms as f64
        } else {
            r.length_ms()
        };
        live::LiveInit {
            replay: r.clone(),
            map: m.clone(),
            ghost: inner.ghost.as_ref().map(|(_, g)| g.clone()),
            cfg,
            run_end,
            hide_cursor: inner.analyze_hide_cursor,
            hide_notes: inner.analyze_hide_notes,
            linger_ms: if inner.analyze_linger_ms > 0.0 {
                inner.analyze_linger_ms
            } else {
                rhythia_render::renderer::HITBOX_LINGER_MS
            },
            win_w: size.width,
            win_h: size.height,
            settings_w: inner.settings.width,
            settings_h: inner.settings.height,
        }
    };

    // Surface creation must happen on the main thread (macOS/Metal).
    let (stx, srx) = std::sync::mpsc::channel();
    let win2 = window.clone();
    window
        .run_on_main_thread(move || {
            // Built from the environment, not `default()`: WGPU_BACKEND is
            // the only escape hatch a user has when the picked backend is
            // the broken one, and wgpu reads it only when asked to.
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
            let surface = instance.create_surface(win2.clone()).map_err(|e| e.to_string());
            let _ = stx.send(surface.map(|s| (instance, s)));
        })
        .map_err(err_str)?;
    let (instance, surface) = srx
        .recv()
        .map_err(|_| "surface channel closed".to_string())?
        .map_err(|e| format!("could not create surface: {e}"))?;

    let (tx, rx) = std::sync::mpsc::channel();
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    live::spawn(
        app_handle.clone(),
        "analyze".into(),
        instance,
        surface,
        init,
        rx,
        running.clone(),
    );
    let mut guard = live.session.lock().unwrap_or_else(|p| p.into_inner());
    *guard = Some(live::LiveSession { tx, running });
    // The close handler saw `starting` and held the window; if it is
    // waiting, it takes this session the moment the flag clears.
    Ok(true)
}

/// Transport for the live engine.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn live_cmd(
    live: tauri::State<'_, live::LiveHandles>,
    app_handle: tauri::AppHandle,
    cmd: String,
    value: Option<f64>,
    w: Option<u32>,
    h: Option<u32>,
    hide_cursor: Option<bool>,
    hide_notes: Option<bool>,
) -> Result<(), String> {
    let guard = live.session.lock().unwrap_or_else(|p| p.into_inner());
    let Some(s) = guard.as_ref() else {
        return Err("no live session".into());
    };
    let msg = match cmd.as_str() {
        "play" => live::LiveCmd::Play,
        "pause" => live::LiveCmd::Pause,
        "seek" => live::LiveCmd::Seek(value.unwrap_or(0.0)),
        "speed" => live::LiveCmd::Speed(value.unwrap_or(1.0)),
        "resize" => {
            // The window's own physical size, not the JS estimate —
            // innerWidth·dpr is off by one at fractional Windows DPI,
            // and a 1px swapchain mismatch stretches the whole frame.
            let (pw, ph) = match (w, h) {
                (Some(w), Some(h)) => (w, h),
                _ => {
                    let win = app_handle
                        .get_webview_window("analyze")
                        .ok_or("analyze window gone")?;
                    let s = win.inner_size().map_err(err_str)?;
                    (s.width, s.height)
                }
            };
            live::LiveCmd::Resize(pw, ph)
        }
        "view" => live::LiveCmd::View {
            hide_cursor: hide_cursor.unwrap_or(false),
            hide_notes: hide_notes.unwrap_or(false),
        },
        "linger" => live::LiveCmd::Linger(value.unwrap_or(350.0)),
        "stop" => live::LiveCmd::Stop,
        other => return Err(format!("unknown live cmd: {other}")),
    };
    s.tx.send(msg).map_err(|_| "live thread gone".to_string())
}

/// Analyze view options — they change what the renderer draws, so the
/// preview pipeline and all cached frames restart.
#[tauri::command]
async fn set_analyze_view(
    state: tauri::State<'_, App>,
    hide_cursor: bool,
    hide_notes: bool,
) -> Result<StatusDto, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut inner = app.lock();
        if inner.analyze_hide_cursor != hide_cursor || inner.analyze_hide_notes != hide_notes {
            inner.analyze_hide_cursor = hide_cursor;
            inner.analyze_hide_notes = hide_notes;
            invalidate_preview(&mut inner);
        }
        assemble_status(&inner, app.rendering.load(Ordering::SeqCst))
    })
    .await
    .map_err(err_str)
}

/// How long resolved analyze hit-area boxes stay (ms; 0 restores the
/// default). Live sessions get it via live_cmd; this persists it for
/// restarts and the fallback geometry path.
#[tauri::command]
fn set_analyze_linger(state: tauri::State<'_, App>, ms: f64) {
    let app = state.inner();
    let mut inner = app.lock();
    inner.analyze_linger_ms = ms.clamp(0.0, 2000.0);
    touch_frames(&mut inner);
}

/// Drops the prepared segment (playback stopped, sources changed).
#[tauri::command]
fn cancel_segment(state: tauri::State<'_, App>) {
    let app = state.inner();
    app.segment_gen.fetch_add(1, Ordering::SeqCst);
    let mut seg = app.segment.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(old) = seg.ready.take() {
        let _ = std::fs::remove_file(&old.path);
    }
}

/// Answers one `rhframe` request: a cached frame if the prefetcher got
/// there first, otherwise a fresh render.
fn serve_frame(app_handle: &tauri::AppHandle, query: &str) -> Result<Arc<Vec<u8>>, String> {
    let time_ms = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("t="))
        .and_then(|v| v.parse::<f64>().ok())
        .ok_or("missing t")?;
    let app: App = (*app_handle.try_state::<App>().ok_or("app not ready")?).clone();
    let key = time_ms.round() as i64;
    // Refuse a pile-up rather than queueing threads behind the renderer;
    // the window simply asks again for the next frame.
    if app.frame_jobs.load(Ordering::SeqCst) >= 6 {
        return Err("busy".into());
    }
    struct Job(App);
    impl Drop for Job {
        fn drop(&mut self) {
            self.0.frame_jobs.fetch_sub(1, Ordering::SeqCst);
        }
    }
    app.frame_jobs.fetch_add(1, Ordering::SeqCst);
    let _job = Job(app.clone());
    let cur_gen = { app.lock().frame_gen };
    {
        let mut cache = app.frames.lock().unwrap_or_else(|p| p.into_inner());
        if cache.gen != cur_gen {
            cache.clear();
            cache.gen = cur_gen;
        }
        if let Some(png) = cache.frames.get(&key) {
            return Ok(png.clone());
        }
    }
    ensure_preview_ctx(&app, time_ms)?;
    let png = match render_frame_png(&app, time_ms) {
        Ok(p) => p,
        Err(_) => {
            ensure_preview_ctx(&app, time_ms)?;
            render_frame_png(&app, time_ms)?
        }
    };
    {
        let mut cache = app.frames.lock().unwrap_or_else(|p| p.into_inner());
        if cache.gen == cur_gen {
            cache.insert(key, png.clone(), key);
        }
    }
    Ok(png)
}

/// Drops cached frames without tearing down the GPU pipeline — for the
/// live-edit paths (HUD toggles, layout drags, background dim) that change
/// what a frame LOOKS like while the pipeline itself stays valid.
fn touch_frames(inner: &mut Inner) {
    inner.frame_gen = inner.frame_gen.wrapping_add(1);
}

/// Stops the background prefetcher (pause, seek, window closing).
#[tauri::command]
fn cancel_prefetch(state: tauri::State<'_, App>) {
    state.inner().prefetch_gen.fetch_add(1, Ordering::SeqCst);
}

/// On-screen geometry for the Analyze overlay. The frame image itself
/// travels through the `rhframe` URI scheme, so this stays tiny and both
/// can be fetched at once.
#[tauri::command]
async fn frame_geometry(
    state: tauri::State<'_, App>,
    time_ms: f64,
) -> Result<PreviewFrameDto, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        ensure_preview_ctx(&app, time_ms)?;
        match frame_sides(&app, time_ms) {
            Ok(s) => Ok(s),
            Err(_) => {
                ensure_preview_ctx(&app, time_ms)?;
                frame_sides(&app, time_ms)
            }
        }
    })
    .await
    .map_err(err_str)?
}

/// Geometry for a whole run of upcoming frames in ONE round trip —
/// playback must not pay IPC latency per frame.
#[tauri::command]
async fn frame_geometry_batch(
    state: tauri::State<'_, App>,
    times: Vec<f64>,
) -> Result<Vec<PreviewFrameDto>, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(&first) = times.first() else {
            return Ok(Vec::new());
        };
        ensure_preview_ctx(&app, first)?;
        // One lock for the whole batch: 60 separate acquisitions would
        // fight the render threads for it.
        let inner = app.lock();
        times
            .iter()
            .map(|t| frame_sides_locked(&inner, *t))
            .collect()
    })
    .await
    .map_err(err_str)?
}

/// Renders frames AHEAD of the playhead into the cache so playback shows
/// finished images instead of waiting on the GPU. Cheap to call often;
/// a newer request cancels the older worker.
#[tauri::command]
fn prefetch_frames(
    state: tauri::State<'_, App>,
    from_ms: f64,
    step_ms: f64,
    count: u32,
) -> Result<(), String> {
    let app = state.inner().clone();
    let gen = app.prefetch_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let step = if step_ms.abs() < 0.5 { 16.0 } else { step_ms };
    let count = count.min(60);
    std::thread::spawn(move || {
        for k in 0..count {
            if app.prefetch_gen.load(Ordering::SeqCst) != gen
                || app.rendering.load(Ordering::SeqCst)
            {
                return;
            }
            // Same arithmetic as the frontend: round(from + step*k).
            // A breath between frames: the lock is held for the whole
            // render, and the UI thread must be able to get in.
            if k > 0 {
                std::thread::sleep(std::time::Duration::from_millis(3));
            }
            let t = from_ms + step * k as f64;
            let key = t.round() as i64;
            let want = {
                let cache = app.frames.lock().unwrap_or_else(|p| p.into_inner());
                !cache.frames.contains_key(&key)
            };
            if !want {
                continue;
            }
            // A frame the window is waiting for beats work done ahead of
            // time — let it have the renderer first.
            let mut waited = 0;
            while app.frame_jobs.load(Ordering::SeqCst) > 0 && waited < 200 {
                std::thread::sleep(std::time::Duration::from_millis(2));
                waited += 1;
                if app.prefetch_gen.load(Ordering::SeqCst) != gen {
                    return;
                }
            }
            if ensure_preview_ctx(&app, t).is_err() {
                return;
            }
            let cur_gen = app.lock().frame_gen;
            match render_frame_png(&app, t) {
                Ok(png) => {
                    let mut cache = app.frames.lock().unwrap_or_else(|p| p.into_inner());
                    if cache.gen != cur_gen {
                        cache.clear();
                        cache.gen = cur_gen;
                    }
                    cache.insert(key, png, from_ms.round() as i64);
                }
                Err(_) => return,
            }
        }
    });
    Ok(())
}

/// Builds the preview pipeline if needed. Callers hold no other lock.
fn ensure_preview_ctx(app: &App, _time_ms: f64) -> Result<(), String> {
    {
        if app.rendering.load(Ordering::SeqCst) {
            return Err("rendering in progress".to_string());
        }
        let mut inner = app.lock();
        if inner.replay.is_none() || inner.map.is_none() {
            return Err("load a replay and map first".to_string());
        }
        if inner.preview.is_none() {
            let cfg = effective_config(&inner);
            // The preview mirrors the OUTPUT's orientation: editing a
            // vertical (Shorts) render needs a vertical live preview.
            // (0 only if a future Default sneaks past the initializer)
            let base_h = if inner.preview_height >= 240 { inner.preview_height } else { PREVIEW_H };
            let (pw, ph) = if inner.settings.height > inner.settings.width {
                (base_h * inner.settings.width / inner.settings.height, base_h)
            } else {
                (base_h * PREVIEW_W / PREVIEW_H, base_h)
            };
            let renderer = rhythia_render::Renderer::new(pw.max(64), ph, cfg.hud_font.as_deref())
                .map_err(|e| gpu_err(&e))?;
            let mut params = SceneParams::from(&cfg);
            let skin = renderer.prepare_skin(&cfg);
            let (_, r) = inner.replay.as_ref().unwrap();
            let (_, m) = inner.map.as_ref().unwrap();
            // Each side plays on its own field: its replay's mirror/hardrock
            // applied to its own copy of the notes.
            let mut ghost = inner.ghost.as_ref().map(|(_, g)| {
                let (gmap, gmods) = rhythia_render::mods::map_for_replay(m, g);
                rhythia_render::hud::GhostInput {
                    state: rhythia_render::hud::HudState::new(&gmap, g),
                    replay: g.clone(),
                    color: GHOST_COLOR,
                    map: gmap,
                    grid_scale: gmods.grid_scale,
                    race: None,
                }
            });
            let (main_map, main_mods) = rhythia_render::mods::map_for_replay(m, r);
            params.grid_scale = main_mods.grid_scale;
            params.apply_speed(r.speed);
            let hud = rhythia_render::hud::HudState::new(&main_map, r);
            if let Some(g) = ghost.as_mut() {
                g.race = Some(rhythia_render::race::RaceSeries::for_race(
                    &rhythia_render::race::RaceSide { map: &main_map, replay: r, state: &hud },
                    &rhythia_render::race::RaceSide { map: &g.map, replay: &g.replay, state: &g.state },
                ));
            }
            // A video background needs live frame extraction per scrub;
            // classify + probe its duration once per ctx.
            let bg_video = inner.settings.background.as_ref().and_then(|p| {
                let pb = PathBuf::from(p);
                (rhythia_render::background::classify_file(&pb).ok()
                    == Some(rhythia_render::background::BackgroundKind::Video))
                .then(|| {
                    let d = rhythia_render::background::probe_duration(
                        &resolve_ffmpeg(&inner.settings),
                        &pb,
                    );
                    (pb, d)
                })
            });
            inner.preview = Some(PreviewCtx {
                renderer,
                skin,
                hud,
                ghost,
                cfg,
                params,
                map: main_map,
                bg_video,
            });
        }
        {
            // Live editing: the cached pipeline stays, only the cheap
            // layout part of the config is refreshed per frame.
            let Inner { preview, settings, base_config, .. } = &mut *inner;
            if let Some(ctx) = preview.as_mut() {
                apply_hud_settings(&mut ctx.cfg, base_config, settings);
            }
        }
    }
    Ok(())
}

/// Renders one preview frame to PNG bytes. The pipeline must exist
/// ([`ensure_preview_ctx`]).
fn render_frame_png(app: &App, time_ms: f64) -> Result<Arc<Vec<u8>>, String> {
    {
        let inner = app.lock();
        let ctx = inner.preview.as_ref().ok_or("no preview")?;
        let (_, r) = inner.replay.as_ref().ok_or("no replay")?;
        if let Some((p, dur)) = &ctx.bg_video {
            // Match the render: the background video runs at wall-clock
            // speed of the OUTPUT, looped over its own duration. In
            // "restart at clip" mode the video's zero is the clip start.
            let mut t_song = time_ms;
            if !inner.settings.background_sync_song {
                if let Some((cs, _)) = inner.clip {
                    t_song = (time_ms - cs).max(0.0);
                }
            }
            let t_out = (t_song / 1000.0) / (r.speed as f64).clamp(0.25, 3.0);
            let (bw, bh) = ctx.renderer.dimensions();
            if let Some(frame) = rhythia_render::background::extract_frame(
                &resolve_ffmpeg(&inner.settings),
                p,
                t_out,
                *dur,
                bw,
                bh,
                &bg_options(&inner.settings),
            ) {
                ctx.renderer.stream_background(&ctx.skin, &frame);
            }
        }
        let pixels = ctx
            .renderer
            .render_still_with_ghost(
                &ctx.params,
                &ctx.cfg,
                &ctx.skin,
                r,
                &ctx.map,
                time_ms,
                Some(&ctx.hud),
                ctx.ghost.as_ref(),
            )
            .map_err(err_str)?;
        let (pw, ph) = ctx.renderer.dimensions();
        return png_bytes(&pixels, pw, ph).map(Arc::new);
    }
}

/// The on-screen geometry for `time_ms` — pure CPU math, no GPU work, so
/// the overlay can be fetched in parallel with the frame image.
fn frame_sides(app: &App, time_ms: f64) -> Result<PreviewFrameDto, String> {
    let inner = app.lock();
    frame_sides_locked(&inner, time_ms)
}

fn frame_sides_locked(inner: &Inner, time_ms: f64) -> Result<PreviewFrameDto, String> {
    let ctx_linger = if inner.analyze_linger_ms > 0.0 {
        inner.analyze_linger_ms
    } else {
        rhythia_render::renderer::HITBOX_LINGER_MS
    };
    {
        let ctx = inner.preview.as_ref().ok_or("no preview")?;
        let (_, r) = inner.replay.as_ref().ok_or("no replay")?;
        let (pw, ph) = ctx.renderer.dimensions();
        let img = String::new();
        let sides = ctx
            .renderer
            .field_projections(
                &ctx.params,
                r,
                ctx.ghost.as_ref().map(|g| (&g.replay, g.grid_scale)),
                time_ms,
            )
            .into_iter()
            .enumerate()
            .map(|(i, ((x, w), m))| {
                // Each side draws its own map with its own params (a ghost
                // may play mirrored or on a wider hardrock grid).
                let (params, map, replay) = match (i, ctx.ghost.as_ref()) {
                    (1, Some(g)) => {
                        let mut p = ctx.params;
                        p.grid_scale = g.grid_scale;
                        (p, &g.map, &g.replay)
                    }
                    _ => (ctx.params, &ctx.map, r),
                };
                let hud = match (i, ctx.ghost.as_ref()) {
                    (1, Some(g)) => &g.state,
                    _ => &ctx.hud,
                };
                let notes = ctx
                    .renderer
                    .note_screen_quads(
                        &params,
                        map,
                        replay,
                        time_ms,
                        (x, w),
                        Some(hud),
                        ctx_linger,
                    )
                    .into_iter()
                    .map(|(i, pts, depth)| NoteQuadDto { i: i as u32, pts, depth })
                    .collect();
                let field = ctx.renderer.playfield_quad(&params, replay, time_ms, (x, w));
                SideProjDto { x, w, m, notes, field }
            })
            .collect();
        Ok(PreviewFrameDto { img, w: pw, h: ph, sides })
    }
}

#[derive(Serialize, Clone)]
struct AnalysisDto {
    main: rhythia_render::analysis::Analysis,
    ghost: Option<rhythia_render::analysis::Analysis>,
    /// Cursor distance between the two runs over time (cells).
    ghost_distance: Option<rhythia_render::analysis::Series>,
    player: String,
    ghost_player: Option<String>,
    map_title: String,
    /// The game's cursor barrier per side (world units): the visible
    /// cursor — and every hit test — is clamped here. Overlays must
    /// clamp too; raw recordings (tablets!) go beyond it.
    cursor_bound: f32,
    ghost_cursor_bound: Option<f32>,
}

/// Full replay analytics for the Analyze tab. Recomputed on demand — a
/// few hundred ms even for long maps, and only requested on tab entry.
#[tauri::command]
async fn analysis_data(state: tauri::State<'_, App>) -> Result<AnalysisDto, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let inner = app.lock();
        let (_, r) = inner.replay.as_ref().ok_or("load a replay first")?;
        let (_, m) = inner.map.as_ref().ok_or("load a map first")?;
        // Analyze against the map as this player saw it (mirror/hardrock).
        let (main_map, main_mods) = rhythia_render::mods::map_for_replay(m, r);
        let main = rhythia_render::analysis::analyze(&main_map, r);
        let bound_of = |grid_scale: f32| {
            grid_scale + (0.5 - rhythia_sim::hitreg::CURSOR_EDGE_INSET)
        };
        let (ghost, ghost_distance, ghost_player, ghost_cursor_bound) = match inner.ghost.as_ref()
        {
            Some((_, g)) => {
                let (gmap, gmods) = rhythia_render::mods::map_for_replay(m, g);
                (
                    Some(rhythia_render::analysis::analyze(&gmap, g)),
                    Some(rhythia_render::analysis::cursor_distance(r, g)),
                    Some(g.player_name.clone()),
                    Some(bound_of(gmods.grid_scale)),
                )
            }
            None => (None, None, None, None),
        };
        Ok(AnalysisDto {
            main,
            ghost,
            ghost_distance,
            player: r.player_name.clone(),
            ghost_player,
            map_title: m.meta.title.clone(),
            cursor_bound: bound_of(main_mods.grid_scale),
            ghost_cursor_bound,
        })
    })
    .await
    .map_err(err_str)?
}

/// Render size of the live preview. The Analyze window raises this so a
/// full-screen replay stays sharp; closing it drops back to the default.
#[tauri::command]
async fn set_preview_quality(
    state: tauri::State<'_, App>,
    height: u32,
) -> Result<StatusDto, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut inner = app.lock();
        // Quantized: a pixel of window resize must not rebuild the whole
        // GPU pipeline (new device + skin upload).
        let h = (height.clamp(480, 2160) / 40) * 40;
        if inner.preview_height != h {
            inner.preview_height = h;
            invalidate_preview(&mut inner);
        }
        assemble_status(&inner, app.rendering.load(Ordering::SeqCst))
    })
    .await
    .map_err(err_str)
}

/// Opens (or focuses) the Analyze window — a second webview showing the
/// replay full size with its own controls.
#[tauri::command]
async fn open_analyze_window(app_handle: tauri::AppHandle) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    if let Some(w) = app_handle.get_webview_window("analyze") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    // The main window owns the session: if it goes, this one goes too.
    if let Some(main) = app_handle.get_webview_window("main") {
        let handle = app_handle.clone();
        main.on_window_event(move |e| {
            if matches!(e, tauri::WindowEvent::Destroyed) {
                if let Some(w) = handle.get_webview_window("analyze") {
                    let _ = w.close();
                }
            }
        });
    }
    let native_capable =
        cfg!(target_os = "windows") || std::env::var("RHYTHR_NATIVE_ANALYZE").is_ok();
    let win = WebviewWindowBuilder::new(&app_handle, "analyze", WebviewUrl::App("analyze.html".into()))
        .title("rhythr — Analyze")
        .inner_size(1280.0, 800.0)
        .min_inner_size(760.0, 520.0)
        // Live mode paints BEHIND the webview; the page body goes
        // transparent so the wgpu frame shows through.
        .transparent(native_capable)
        .build()
        .map_err(err_str)?;
    // Closing it drops the preview back to its normal size.
    let handle = app_handle.clone();
    let win_for_close = win.clone();
    win.on_window_event(move |e| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = e {
            // The live thread owns a surface on this window: it MUST stop
            // and drop it before the X11/Win32 window dies, or the next
            // present hits a dead drawable and takes the process down.
            // A start may also be in flight (session not stored yet) —
            // hold the window open until that start lands, then stop
            // whatever session it stored.
            let (stopping, starting) = {
                if let Some(lh) = handle.try_state::<live::LiveHandles>() {
                    let mut guard = lh.session.lock().unwrap_or_else(|p| p.into_inner());
                    (guard.take(), lh.starting.load(Ordering::SeqCst))
                } else {
                    (None, false)
                }
            };
            if stopping.is_some() || starting {
                api.prevent_close();
                if let Some(s) = &stopping {
                    let _ = s.tx.send(live::LiveCmd::Stop);
                }
                let win2 = win_for_close.clone();
                let handle2 = handle.clone();
                std::thread::spawn(move || {
                    let wait = |s: &live::LiveSession| {
                        let _ = s.tx.send(live::LiveCmd::Stop);
                        for _ in 0..200 {
                            if !s.running.load(Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    };
                    if let Some(s) = stopping {
                        wait(&s);
                    }
                    if starting {
                        // Let the in-flight start land, then take and stop
                        // whatever it stored.
                        if let Some(lh) = handle2.try_state::<live::LiveHandles>() {
                            for _ in 0..200 {
                                if !lh.starting.load(Ordering::SeqCst) {
                                    break;
                                }
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            }
                            let late = {
                                let mut guard =
                                    lh.session.lock().unwrap_or_else(|p| p.into_inner());
                                guard.take()
                            };
                            if let Some(s) = late {
                                wait(&s);
                            }
                        }
                    }
                    let _ = win2.destroy();
                });
            }
        }
        if matches!(e, tauri::WindowEvent::Destroyed) {
            if let Some(lh) = handle.try_state::<live::LiveHandles>() {
                let mut guard = lh.session.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(s) = guard.take() {
                    let _ = s.tx.send(live::LiveCmd::Stop);
                }
            }
            if let Some(app) = handle.try_state::<App>() {
                let mut inner = app.lock();
                let changed = inner.preview_height != PREVIEW_H
                    || inner.analyze_hide_cursor
                    || inner.analyze_hide_notes;
                inner.preview_height = PREVIEW_H;
                inner.analyze_hide_cursor = false;
                inner.analyze_hide_notes = false;
                inner.analyze_linger_ms = 0.0;
                if changed {
                    invalidate_preview(&mut inner);
                }
            }
        }
    });
    Ok(())
}

/// Writes a text export (JSON/CSV) to a user-chosen path.
#[tauri::command]
fn save_text_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(err_str)
}

/// The live engine's own picture at its current clock — PNG bytes for
/// the overlay snapshot, so the composite shows EXACTLY what the screen
/// shows (skin background, live resolution), not the preview pipeline's
/// rendition with the custom background.
#[tauri::command]
async fn live_still(
    live: tauri::State<'_, live::LiveHandles>,
) -> Result<tauri::ipc::Response, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let guard = live.session.lock().unwrap_or_else(|p| p.into_inner());
        let s = guard.as_ref().ok_or("no live session")?;
        s.tx.send(live::LiveCmd::Still(tx))
            .map_err(|_| "live thread gone".to_string())?;
    }
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| "live still timed out".to_string())?
    })
    .await
    .map_err(err_str)??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Overlay snapshots: with RHYTHR_SNAP_DIR set, the analyze window saves
/// there without a dialog — the composited picture+overlay PNG is the
/// only reliable way to SEE the overlay in automated tests (X11 screen
/// grabs of a transparent window are compositor lottery).
static SNAP_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[tauri::command]
fn overlay_snap_target() -> Option<String> {
    std::env::var("RHYTHR_SNAP_DIR").ok().map(|d| {
        let n = SNAP_COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("{d}/overlay-{n:03}.png")
    })
}

/// Decodes a canvas data URL and writes the PNG bytes.
#[tauri::command]
fn save_data_url_png(path: String, data_url: String) -> Result<(), String> {
    use base64::Engine as _;
    let b64 = data_url
        .split_once("base64,")
        .ok_or("not a base64 data url")?
        .1;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(err_str)?;
    std::fs::write(&path, bytes).map_err(err_str)
}

/// Writes a canvas-rendered PNG (data URL) to a user-chosen path.
#[tauri::command]
fn save_data_url(path: String, data_url: String) -> Result<(), String> {
    let b64 = data_url
        .strip_prefix("data:image/png;base64,")
        .ok_or("expected a PNG data URL")?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(err_str)?;
    std::fs::write(&path, bytes).map_err(err_str)
}

#[tauri::command]
async fn export_frame(
    state: tauri::State<'_, App>,
    time_ms: f64,
    path: String,
) -> Result<(), String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if app.rendering.load(Ordering::SeqCst) {
            return Err("rendering in progress".to_string());
        }
        let inner = app.lock();
        let (_, r) = inner.replay.as_ref().ok_or("no replay loaded")?;
        let (_, m) = inner.map.as_ref().ok_or("no map loaded")?;
        let cfg = effective_config(&inner);
        let (w, h) = (inner.settings.width, inner.settings.height);
        let mut params = SceneParams::from(&cfg);
        let renderer =
            rhythia_render::Renderer::new(w, h, cfg.hud_font.as_deref()).map_err(|e| gpu_err(&e))?;
        let skin = renderer.prepare_skin(&cfg);
        let (m, mods) = rhythia_render::mods::map_for_replay(m, r);
        params.grid_scale = mods.grid_scale;
        params.apply_speed(r.speed);
        let hud = rhythia_render::hud::HudState::new(&m, r);
        let pixels = renderer
            .render_still(&params, &cfg, &skin, r, &m, time_ms, Some(&hud))
            .map_err(err_str)?;
        rhythia_render::write_png(Path::new(&path), &pixels, w, h).map_err(err_str)
    })
    .await
    .map_err(err_str)?
}

/// Renders the shareable score card as a PNG in the requested size —
/// platform presets range from Discord's 1200x630 to a 1080x1920 Short.
#[tauri::command]
async fn export_card(
    state: tauri::State<'_, App>,
    path: String,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if app.rendering.load(Ordering::SeqCst) {
            return Err("rendering in progress".to_string());
        }
        let (w, h) = (width.clamp(256, 4096), height.clamp(256, 4096));
        let inner = app.lock();
        let (_, r) = inner.replay.as_ref().ok_or("no replay loaded")?;
        let (_, m) = inner.map.as_ref().ok_or("no map loaded")?;
        let cfg = effective_config(&inner);
        let renderer =
            rhythia_render::Renderer::new(w, h, cfg.hud_font.as_deref()).map_err(|e| gpu_err(&e))?;
        let hud = rhythia_render::hud::HudState::new(m, r);
        let pixels = renderer.render_card(r, m, &hud, &cfg).map_err(err_str)?;
        rhythia_render::write_png(Path::new(&path), &pixels, w, h).map_err(err_str)
    })
    .await
    .map_err(err_str)?
}

#[derive(Serialize, Clone)]
struct RenderProgress {
    done: u64,
    total: u64,
    fps: f64,
    eta_secs: f64,
}

#[tauri::command]
/// Where a render started right now would write, from the output folder,
/// the file name and the clip range. Shared by the render itself and by the
/// UI's overwrite check, so the two can never disagree about the target.
fn planned_output(inner: &Inner) -> Result<PathBuf, String> {
    let s = &inner.settings;
    let dir = s
        .output_dir
        .clone()
        .or_else(|| {
            dirs::video_dir()
                .or_else(dirs::download_dir)
                .map(|p| p.to_string_lossy().into_owned())
        })
        .ok_or("no output folder set")?;
    let mut name = if s.file_name.is_empty() {
        suggested_name(inner)
    } else {
        sanitize_filename(&s.file_name)
    };
    if !name.to_lowercase().ends_with(".mp4") {
        name.push_str(".mp4");
    }
    // A clip gets its range in the name: "… (1.02-1.34).mp4" (dots,
    // not colons — Windows path rules).
    if let Some((cs, ce)) = inner.clip {
        let tag = |ms: f64| {
            let t = (ms / 1000.0).max(0.0) as u64;
            format!("{}.{:02}", t / 60, t % 60)
        };
        let base = name.trim_end_matches(".mp4").to_string();
        name = format!("{base} ({}-{}).mp4", tag(cs), tag(ce));
    }
    Ok(PathBuf::from(&dir).join(name))
}

/// The first free "name (2).mp4", "name (3).mp4"… next to a taken path.
fn next_free_path(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} ({n}).{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

/// The target path plus whether something is already there, so the UI can
/// ask before a render replaces an earlier video.
#[tauri::command]
fn planned_output_path(state: tauri::State<'_, App>) -> Result<PlannedOutputDto, String> {
    let app = state.inner();
    let inner = app.lock();
    let path = planned_output(&inner)?;
    Ok(PlannedOutputDto {
        exists: path.exists(),
        path: path.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
fn start_render(
    state: tauri::State<'_, App>,
    app_handle: tauri::AppHandle,
    // Keep an existing file and write alongside it instead of over it.
    keep_existing: Option<bool>,
) -> Result<String, String> {
    let app = state.inner().clone();
    if app.rendering.swap(true, Ordering::SeqCst) {
        return Err("a render is already running".into());
    }
    let result = (|| -> Result<(String, RenderJob), String> {
        let inner = app.lock();
        let (_, replay) = inner.replay.as_ref().ok_or("no replay loaded")?;
        let (_, map) = inner.map.as_ref().ok_or("no map loaded")?;
        let s = &inner.settings;
        let mut out = planned_output(&inner)?;
        if keep_existing.unwrap_or(false) && out.exists() {
            out = next_free_path(&out);
        }
        let dir = out
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        std::fs::create_dir_all(&dir).map_err(err_str)?;
        let job = RenderJob {
            replay: replay.clone(),
            map: map.clone(),
            cfg: effective_config(&inner),
            width: s.width,
            height: s.height,
            fps: s.fps,
            quality: s.quality,
            tcp_feed: s.tcp_feed,
            encoder: s.encoder.clone(),
            preset: s.preset.clone(),
            results_secs: s.results_secs,
            motion_blur: s.motion_blur,
            music_volume: s.music_volume.min(150) as f32 / 100.0,
            hitsounds: load_hitsounds(s),
            ghost: inner.ghost.as_ref().map(|(_, g)| {
                rhythia_render::video::GhostOptions {
                    replay: g.clone(),
                    color: GHOST_COLOR,
                }
            }),
            background_video: s.background.as_ref().and_then(|p| {
                let pb = PathBuf::from(p);
                (rhythia_render::background::classify_file(&pb).ok()
                    == Some(rhythia_render::background::BackgroundKind::Video))
                .then(|| {
                    let mut opts = bg_options(s);
                    if s.background_sync_song {
                        if let Some((cs, _)) = inner.clip {
                            let speed = (replay.speed as f64).clamp(0.25, 3.0);
                            opts.sync_offset_secs = rhythia_render::background::sync_offset(
                                cs / 1000.0 / speed,
                                opts.start_secs,
                                inner.bg_duration,
                            );
                        }
                    }
                    rhythia_render::video::BackgroundVideo { path: pb, opts }
                })
            }),
            clip: inner.clip,
            ffmpeg: resolve_ffmpeg(s),
            out: out.clone(),
        };
        Ok((out.to_string_lossy().into_owned(), job))
    })();
    let (out_path, job) = match result {
        Ok(v) => v,
        Err(e) => {
            app.rendering.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    app.cancel.store(false, Ordering::SeqCst);
    let thread_app = app.clone();
    let handle = std::thread::spawn(move || {
        // A panic anywhere in the job (wgpu device loss, driver reset, …)
        // must still clear the rendering flag and tell the UI — otherwise
        // every later render/preview is refused until an app restart.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_render_job(&thread_app, &app_handle, job)
        }));
        thread_app.rendering.store(false, Ordering::SeqCst);
        match outcome {
            Ok(Ok(path)) => {
                let _ = app_handle.emit("render-done", path.to_string_lossy().into_owned());
            }
            Ok(Err(rhythia_render::Error::Cancelled)) => {
                let _ = app_handle.emit("render-cancelled", ());
            }
            Ok(Err(e)) => {
                let _ = app_handle.emit("render-error", gpu_err(&e));
            }
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "render thread panicked".into());
                let _ = app_handle.emit("render-error", format!("renderer crashed: {msg}"));
            }
        }
    });
    *app.render_thread.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
    Ok(out_path)
}

struct RenderJob {
    replay: Replay,
    map: Map,
    cfg: SkinConfig,
    width: u32,
    height: u32,
    fps: u32,
    quality: u32,
    tcp_feed: bool,
    encoder: String,
    preset: String,
    results_secs: f64,
    motion_blur: u32,
    music_volume: f32,
    hitsounds: Option<rhythia_render::video::HitsoundOptions>,
    ghost: Option<rhythia_render::video::GhostOptions>,
    /// Set when the custom background is a video file (images are already
    /// baked into `cfg`).
    background_video: Option<rhythia_render::video::BackgroundVideo>,
    /// Session clip range (song ms); None renders the full run.
    clip: Option<(f64, f64)>,
    ffmpeg: String,
    out: PathBuf,
}

fn run_render_job(
    app: &App,
    handle: &tauri::AppHandle,
    job: RenderJob,
) -> Result<PathBuf, rhythia_render::Error> {
    let _ = handle.emit("render-stage", "starting GPU renderer");
    let renderer =
        rhythia_render::Renderer::new(job.width, job.height, job.cfg.hud_font.as_deref())?;
    let params = SceneParams::from(&job.cfg);

    // Probe hardware encoders unless one was forced.
    let encoder = match job.encoder.as_str() {
        "auto" => rhythia_render::video::hardware_encoders()
            .iter()
            .copied()
            .find(|e| rhythia_render::video::encoder_works(&job.ffmpeg, e))
            .unwrap_or("x264")
            .to_string(),
        other => other.to_string(),
    };
    let _ = handle.emit("render-stage", format!("encoder: {encoder}"));

    // Embedded map audio goes through a temp file for ffmpeg.
    let mut _audio_tmp: Option<tempfile::NamedTempFile> = None;
    let audio = if let Some(bytes) = &job.map.audio {
        let mut tmp = tempfile::Builder::new()
            .prefix("rhythia-audio-")
            .suffix(".mp3")
            .tempfile()?;
        std::io::Write::write_all(&mut tmp, bytes)?;
        let path = tmp.path().to_path_buf();
        _audio_tmp = Some(tmp);
        Some(path)
    } else {
        None
    };

    let run_end = if job.replay.failed() {
        f64::from(job.replay.fail_time_ms)
    } else {
        job.replay.length_ms()
    };
    // A clip range renders just that slice; the results screen only
    // appears when the clip reaches the end of the run (video.rs rule).
    let (start_ms, end_ms) = match job.clip {
        Some((s, e)) => (s.clamp(0.0, run_end), e.clamp(s, run_end)),
        None => (0.0, run_end),
    };
    let opts = rhythia_render::video::VideoOptions {
        extra_output_args: Vec::new(),
        fps: job.fps,
        start_ms,
        end_ms,
        ffmpeg: job.ffmpeg.clone(),
        audio,
        quality: job.quality,
        tcp_feed: job.tcp_feed,
        preset: job.preset.clone(),
        encoder,
        results_secs: job.results_secs,
        motion_blur: job.motion_blur,
        music_volume: job.music_volume,
        hitsounds: job.hitsounds,
        ghost: job.ghost,
        background_video: job.background_video,
    };

    let started = std::time::Instant::now();
    let mut last_emit = std::time::Instant::now();
    let mut final_fps = 0.0f64;
    rhythia_render::video::render_video(
        &renderer,
        &params,
        &job.cfg,
        &job.replay,
        &job.map,
        &job.out,
        &opts,
        |done, total| {
            if app.cancel.load(Ordering::SeqCst) {
                return false;
            }
            if last_emit.elapsed().as_millis() >= 200 || done == total {
                last_emit = std::time::Instant::now();
                let elapsed = started.elapsed().as_secs_f64();
                let fps = if elapsed > 0.0 { done as f64 / elapsed } else { 0.0 };
                final_fps = fps;
                let eta = if fps > 0.0 {
                    (total - done) as f64 / fps
                } else {
                    0.0
                };
                let _ = handle.emit(
                    "render-progress",
                    RenderProgress {
                        done,
                        total,
                        fps,
                        eta_secs: eta,
                    },
                );
            }
            true
        },
    )?;
    if final_fps > 1.0 {
        let mut inner = app.lock();
        inner.settings.last_render_fps = final_fps;
        inner.settings.save();
    }
    Ok(job.out)
}

#[tauri::command]
fn cancel_render(state: tauri::State<'_, App>) {
    state.inner().cancel.store(true, Ordering::SeqCst);
}

/// One stop on the quality slider, resolved by the backend so the mapping
/// lives in exactly one place. Recomputing it in JavaScript would be a second
/// copy of [`rhythia_render::quality`] to keep in step, and the first time
/// they drifted the number on screen would stop describing the encode.
#[derive(Serialize, Clone)]
struct QualityStep {
    q: u32,
    x264: u32,
    hardware: u32,
    hint: &'static str,
}

#[derive(Serialize)]
struct EncoderProbe {
    available: Vec<String>,
    /// Encoder -> why it is unavailable (ffmpeg's own words).
    unavailable: BTreeMap<String, String>,
    /// Set when ffmpeg itself cannot be run. Nothing will encode, and the
    /// UI has to say so BEFORE a render — it used to advertise x264 as
    /// available regardless and only fail minutes in.
    ffmpeg_missing: bool,
    /// The path or command that was tried, so the message can name it.
    ffmpeg: String,
    /// Every stop on the quality slider with the value each encoder family
    /// will actually be given.
    quality_steps: Vec<QualityStep>,
}

/// Results of the encoder probe for one ffmpeg, kept for the life of the
/// process. Each probe is a real encode and costs about a tenth of a second;
/// the UI asks again whenever the output panel is rebuilt.
///
/// Deliberately NOT cached to disk: a driver update or a swapped GPU changes
/// the answer, and a stale file would send someone to an encoder that no
/// longer works with no way to tell why.
fn probe_cache() -> &'static std::sync::Mutex<
    std::collections::HashMap<String, (Vec<String>, BTreeMap<String, String>)>,
> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, (Vec<String>, BTreeMap<String, String>)>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The slider's stops. Step 5 is not arbitrary: the mapping moves the
/// encoder's own number by exactly one per step, so every position on the
/// slider is a distinct encode and none of them are duplicates.
fn quality_steps() -> Vec<QualityStep> {
    use rhythia_render::quality;
    (quality::MIN..=quality::MAX)
        .step_by(5)
        .map(|q| QualityStep {
            q,
            x264: quality::x264_crf(q),
            hardware: quality::hardware_q(q),
            hint: quality::describe(q),
        })
        .collect()
}

#[tauri::command]
async fn probe_encoders(state: tauri::State<'_, App>) -> Result<EncoderProbe, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let ffmpeg = {
            let inner = app.lock();
            resolve_ffmpeg(&inner.settings)
        };
        // Can ffmpeg run at all? Everything else is moot if not.
        let missing = !rhythia_render::video::ffmpeg_runs(&ffmpeg);
        let mut available = if missing {
            Vec::new()
        } else {
            vec!["auto".to_string(), "x264".to_string()]
        };
        let mut unavailable = BTreeMap::new();
        if !missing {
            let cached = probe_cache().lock().ok().and_then(|c| c.get(&ffmpeg).cloned());
            match cached {
                Some((hw, why)) => {
                    available.extend(hw);
                    unavailable = why;
                }
                None => {
                    let mut hw = Vec::new();
                    for e in rhythia_render::video::hardware_encoders() {
                        match rhythia_render::video::encoder_error(&ffmpeg, e) {
                            None => hw.push((*e).to_string()),
                            Some(reason) => {
                                unavailable.insert((*e).to_string(), reason);
                            }
                        }
                    }
                    if let Ok(mut c) = probe_cache().lock() {
                        c.insert(ffmpeg.clone(), (hw.clone(), unavailable.clone()));
                    }
                    available.extend(hw);
                }
            }
        }
        Ok(EncoderProbe {
            available,
            unavailable,
            ffmpeg_missing: missing,
            ffmpeg,
            quality_steps: quality_steps(),
        })
    })
    .await
    .map_err(err_str)?
}

// -------------------------------------------------------------------- main

/// How long a segment directory must have sat untouched before a platform
/// without a PID probe may delete it. Anything younger could still belong to
/// a rhythr that is playing from it right now.
const SEGMENT_DIR_STALE_SECS: u64 = 6 * 60 * 60;

/// Whether a PID still belongs to a running process. `None` where the
/// platform has no cheap answer and the caller has to fall back on age.
#[cfg(target_os = "linux")]
fn pid_alive(pid: u32) -> Option<bool> {
    Some(Path::new("/proc").join(pid.to_string()).exists())
}

#[cfg(not(target_os = "linux"))]
fn pid_alive(_pid: u32) -> Option<bool> {
    None
}

/// Drops `rhythr-segments-<pid>` directories that no live rhythr can own.
/// A second launch runs this before the single-instance plugin turns it away
/// (that happens inside the Tauri builder), so at this point it is a real
/// process next to the first one — and the first one's segments are the
/// video its Analyze window is playing from.
fn sweep_stale_segments() {
    let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let own = std::process::id();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = name
            .strip_prefix("rhythr-segments-")
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        // Ours is created lazily on the first segment — while this sweep runs.
        if pid == own {
            continue;
        }
        let stale = match pid_alive(pid) {
            Some(alive) => !alive,
            None => e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > SEGMENT_DIR_STALE_SECS),
        };
        if stale {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn main() {
    // WebKitGTK's DMA-BUF renderer is broken on many Linux/Wayland driver
    // combinations (blank window or a Wayland protocol-error crash,
    // especially on NVIDIA). Default it off unless the user overrides —
    // the UI is light, the webview performance cost is negligible.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let shared: App = Arc::new(Shared {
        inner: Mutex::new(Inner {
            settings: Settings::load(),
            preview_height: PREVIEW_H,
            ..Inner::default()
        }),
        cancel: AtomicBool::new(false),
        rendering: AtomicBool::new(false),
        frames: Mutex::new(FrameCache::default()),
        prefetch_gen: std::sync::atomic::AtomicU64::new(0),
        frame_jobs: std::sync::atomic::AtomicUsize::new(0),
        segment: Mutex::new(SegmentState::default()),
        segment_gen: std::sync::atomic::AtomicU64::new(0),
        render_thread: Mutex::new(None),
    });

    // Restore the last config; load a replay passed as CLI arg (file
    // association) or fall back to the last one used.
    {
        let mut inner = shared.lock();
        let cfg_path = inner
            .settings
            .last_config
            .clone()
            .map(PathBuf::from)
            .filter(|p| p.exists());
        inner.config_path = cfg_path;
        match load_base_config(&inner.config_path, &inner.settings.game_assets) {
            Ok(cfg) => inner.base_config = cfg,
            Err(_) => {
                inner.config_path = None;
                inner.base_config = SkinConfig::default();
            }
        }
        let arg_replay = std::env::args()
            .nth(1)
            .filter(|a| a.to_lowercase().ends_with(".rhr"));
        let candidate = arg_replay.or_else(|| inner.settings.last_replay.clone());
        if let Some(path) = candidate.filter(|p| Path::new(p).exists()) {
            if let Ok(replay) = Replay::from_path(&path) {
                if let Some((p, m)) = try_cached_map(&replay) {
                    inner.map = Some((p, m));
                    inner.map_source = "cache".into();
                }
                inner.replay = Some((PathBuf::from(path), replay));
                normalize_time_bases(&mut inner);
            }
        }
    }

    // Segments from a previous run (a crash, a kill) are dead weight, but
    // scanning temp is filesystem I/O the window should not wait behind.
    std::thread::spawn(sweep_stale_segments);

    tauri::Builder::default()
        // Frame channel for the Analyze window: PNG bytes straight into
        // the webview's image decoder — no base64, no JSON, no IPC. Served
        // off-thread; the handler must always respond or the image hangs.
        .register_asynchronous_uri_scheme_protocol("rhframe", |ctx, req, responder| {
            let app_handle = ctx.app_handle().clone();
            std::thread::spawn(move || {
                // The responder MUST be used: dropping it (a panic in the
                // renderer, say) leaves the request pending forever and
                // the window stops showing frames.
                let responder = std::sync::Mutex::new(Some(responder));
                let answer = |status: tauri::http::StatusCode, ct: &str, body: Vec<u8>| {
                    if let Some(r) = responder.lock().ok().and_then(|mut g| g.take()) {
                        if let Ok(resp) = tauri::http::Response::builder()
                            .status(status)
                            .header(tauri::http::header::CONTENT_TYPE, ct)
                            .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                            .header(tauri::http::header::CACHE_CONTROL, "no-store")
                            .body(body)
                        {
                            r.respond(resp);
                        }
                    }
                };
                let work = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    serve_frame(&app_handle, req.uri().query().unwrap_or(""))
                }));
                match work {
                    Ok(Ok(png)) => answer(tauri::http::StatusCode::OK, "image/png", png.as_ref().clone()),
                    Ok(Err(e)) => answer(
                        tauri::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "text/plain",
                        e.into_bytes(),
                    ),
                    Err(_) => answer(
                        tauri::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "text/plain",
                        b"frame renderer panicked".to_vec(),
                    ),
                }
            });
        })
        // The map's own music for the analyzer: one whole-file response —
        // WebAudio decodes the complete buffer up front, no ranges needed.
        .register_asynchronous_uri_scheme_protocol("rhaudio", |ctx, req, responder| {
            let app_handle = ctx.app_handle().clone();
            let _ = req;
            std::thread::spawn(move || {
                let responder = std::sync::Mutex::new(Some(responder));
                let answer = |status: tauri::http::StatusCode, ct: &str, body: Vec<u8>| {
                    if let Some(r) = responder.lock().ok().and_then(|mut g| g.take()) {
                        if let Ok(resp) = tauri::http::Response::builder()
                            .status(status)
                            .header(tauri::http::header::CONTENT_TYPE, ct)
                            .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                            .header(tauri::http::header::CACHE_CONTROL, "no-store")
                            .body(body)
                        {
                            r.respond(resp);
                        }
                    }
                };
                let audio = app_handle.try_state::<App>().and_then(|app| {
                    let inner = app.lock();
                    inner.map.as_ref().and_then(|(_, m)| m.audio.clone())
                });
                match audio {
                    Some(bytes) => {
                        let ct = if bytes.starts_with(b"OggS") {
                            "audio/ogg"
                        } else if bytes.starts_with(b"RIFF") {
                            "audio/wav"
                        } else if bytes.starts_with(b"fLaC") {
                            "audio/flac"
                        } else {
                            "audio/mpeg"
                        };
                        answer(tauri::http::StatusCode::OK, ct, bytes);
                    }
                    None => answer(
                        tauri::http::StatusCode::NOT_FOUND,
                        "text/plain",
                        b"map has no audio".to_vec(),
                    ),
                }
            });
        })
        // Playback segments: a real video file, served with byte ranges
        // because that is what a <video> element needs to start and seek.
        .register_asynchronous_uri_scheme_protocol("rhvideo", |ctx, req, responder| {
            let app_handle = ctx.app_handle().clone();
            let range = req
                .headers()
                .get(tauri::http::header::RANGE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            std::thread::spawn(move || {
                let responder = std::sync::Mutex::new(Some(responder));
                let answer = |status: tauri::http::StatusCode,
                              body: Vec<u8>,
                              content_range: Option<String>| {
                    if let Some(r) = responder.lock().ok().and_then(|mut g| g.take()) {
                        let mut b = tauri::http::Response::builder()
                            .status(status)
                            .header(tauri::http::header::CONTENT_TYPE, "video/mp4")
                            .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                            .header(tauri::http::header::ACCEPT_RANGES, "bytes")
                            .header(tauri::http::header::CACHE_CONTROL, "no-store");
                        if let Some(cr) = content_range {
                            b = b.header(tauri::http::header::CONTENT_RANGE, cr);
                        }
                        if let Ok(resp) = b.body(body) {
                            r.respond(resp);
                        }
                    }
                };
                let path = app_handle.try_state::<App>().and_then(|app| {
                    let seg = app.segment.lock().unwrap_or_else(|p| p.into_inner());
                    seg.ready.as_ref().map(|s| s.path.clone())
                });
                let Some(path) = path else {
                    return answer(tauri::http::StatusCode::NOT_FOUND, Vec::new(), None);
                };
                let Ok(bytes) = std::fs::read(&path) else {
                    return answer(tauri::http::StatusCode::NOT_FOUND, Vec::new(), None);
                };
                let total = bytes.len() as u64;
                match range
                    .as_deref()
                    .and_then(|r| r.strip_prefix("bytes="))
                    .map(|r| {
                        let mut it = r.split('-');
                        let s = it.next().unwrap_or("").parse::<u64>().unwrap_or(0);
                        let e = it
                            .next()
                            .filter(|v| !v.is_empty())
                            .and_then(|v| v.parse::<u64>().ok())
                            .unwrap_or(total.saturating_sub(1));
                        (s.min(total.saturating_sub(1)), e.min(total.saturating_sub(1)))
                    }) {
                    Some((s, e)) if total > 0 => {
                        let slice = bytes[s as usize..=e as usize].to_vec();
                        answer(
                            tauri::http::StatusCode::PARTIAL_CONTENT,
                            slice,
                            Some(format!("bytes {s}-{e}/{total}")),
                        )
                    }
                    _ => answer(tauri::http::StatusCode::OK, bytes, None),
                }
            });
        })
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // A .rhr double-click while the app runs lands here as a second
            // instance's argv — forward it and pull the window up.
            if let Some(path) = argv.get(1).filter(|a| a.to_lowercase().ends_with(".rhr")) {
                let _ = app.emit("open-replay", path.clone());
            }
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(shared)
        .manage(live::LiveHandles::default())
        .setup(|app| {
            let _ = RESOURCE_DIR.set(app.path().resource_dir().ok());
            // A persisted VIDEO background needs its duration for the
            // start-time slider — probe once, now that the bundled-ffmpeg
            // resource dir is known.
            let shared = app.state::<App>().inner().clone();
            let mut inner = shared.lock();
            if let Some(p) = inner.settings.background.clone() {
                let pb = PathBuf::from(&p);
                if rhythia_render::background::classify_file(&pb).ok()
                    == Some(rhythia_render::background::BackgroundKind::Video)
                {
                    inner.bg_duration = rhythia_render::background::probe_duration(
                        &resolve_ffmpeg(&inner.settings),
                        &pb,
                    );
                }
            }
            let rect = inner.settings.window_rect;
            drop(inner);

            // Restore the last geometry, then keep it current. Clamped
            // against the monitor list so a window remembered on a screen
            // that is now gone cannot open off-screen.
            if let Some(main) = app.handle().get_webview_window("main") {
                if let Some((x, y, w, h)) = rect {
                    if w >= 800 && h >= 600 && window_pos_visible(&main, x, y) {
                        let _ = main.set_size(tauri::PhysicalSize::new(w, h));
                        let _ = main.set_position(tauri::PhysicalPosition::new(x, y));
                    }
                }
                let handle = app.handle().clone();
                main.clone().on_window_event(move |e| {
                    if !matches!(
                        e,
                        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
                    ) {
                        return;
                    }
                    let Some(w) = handle.get_webview_window("main") else {
                        return;
                    };
                    // Skip minimised/maximised states: restoring those
                    // coordinates would reopen the window somewhere useless.
                    if w.is_minimized().unwrap_or(false) || w.is_maximized().unwrap_or(false) {
                        return;
                    }
                    let (Ok(pos), Ok(size)) = (w.outer_position(), w.inner_size()) else {
                        return;
                    };
                    // Recorded in a lock of its own, and written ONCE on
                    // exit. Taking the global state lock here would put the
                    // event-loop thread behind whatever the render or
                    // prefetch worker is holding — the documented way to
                    // freeze this window — and a drag fires this per pixel,
                    // so saving here meant rewriting settings.json hundreds
                    // of times to move a window.
                    if let Ok(mut slot) = LAST_WINDOW_RECT.lock() {
                        *slot = Some((pos.x, pos.y, size.width, size.height));
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            update_channel,
            open_releases_page,
            load_replay,
            load_map,
            download_map,
            load_config,
            clear_config,
            set_game_assets,
            detect_game,
            set_hud_override,
            set_hud_position,
            set_hud_scale,
            hud_layout,
            set_meter,
            load_ghost,
            clear_ghost,
            reset_hud_overrides,
            set_background,
            set_background_dim,
            set_background_transform,
            save_preset,
            apply_preset,
            delete_preset,
            undo_layout,
            redo_layout,
            mark_undo,
            frame_geometry,
            frame_geometry_batch,
            prefetch_frames,
            cancel_prefetch,
            prepare_segment,
            cancel_segment,
            set_analyze_view,
            set_analyze_linger,
            start_live_session,
            live_cmd,
            analysis_data,
            save_text_file,
            overlay_snap_target,
            save_data_url_png,
            live_still,
            save_data_url,
            set_preview_quality,
            open_analyze_window,
            set_clip,
            clear_clip,
            reset_hud_layout,
            set_output,
            suggest_file_name,
            timeline,
            preview,
            export_frame,
            export_card,
            start_render,
            planned_output_path,
            write_diagnostics,
            cancel_render,
            probe_encoders,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Persist the window geometry once, on the way out.
                let rect = LAST_WINDOW_RECT
                    .lock()
                    .ok()
                    .and_then(|slot| *slot);
                if let Some(rect) = rect {
                    let shared = app_handle.state::<App>();
                    let mut inner = shared.lock();
                    if inner.settings.window_rect != Some(rect) {
                        inner.settings.window_rect = Some(rect);
                        inner.settings.save();
                    }
                    drop(inner);
                }
                // Closing mid-render: cancel (kills ffmpeg, removes the
                // partial file, drops the audio temp) and give the render
                // thread a moment to finish that cleanup.
                let shared = app_handle.state::<App>();
                shared.cancel.store(true, Ordering::SeqCst);
                let handle = shared
                    .render_thread
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
                if let Some(handle) = handle {
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = handle.join();
                        let _ = tx.send(());
                    });
                    // A stalled ffmpeg must not hang the exit forever.
                    let _ = rx.recv_timeout(std::time::Duration::from_secs(5));
                }
                // This session's segments die with it, so the next launch's
                // sweep normally has nothing left to decide about.
                let seg_dir = {
                    let mut seg = shared.segment.lock().unwrap_or_else(|p| p.into_inner());
                    seg.ready = None;
                    seg.dir.take()
                };
                if let Some(dir) = seg_dir {
                    let _ = std::fs::remove_dir_all(dir);
                }
            }
        });
}

#[cfg(test)]
mod update_channel_tests {
    use super::*;

    #[test]
    fn a_test_binary_is_never_aur_managed() {
        // Sanity for the channel probe: whatever host this builds on, a
        // cargo test binary is not owned by pacman, so the only valid
        // answers are self-updating (Windows/AppImage env) or the
        // download-page fallback.
        let c = update_channel();
        assert!(c == "self" || c == "page", "unexpected channel {c}");
    }
}
