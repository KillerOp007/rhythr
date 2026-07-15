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

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use rhythia_formats::{map::Map, rhr::Replay};
use rhythia_render::{scene::SceneParams, SkinConfig};
use rhythia_sim::integrity;

const USER_AGENT: &str = "rhythr/0.2 (desktop app)";
const API_BEATMAP_PAGE: &str = "https://production.rhythia.com/api/getBeatmapPage";
/// Refuse to download maps larger than this (malformed/hostile responses).
const MAX_MAP_BYTES: u64 = 512 * 1024 * 1024;
const PREVIEW_W: u32 = 1280;
const PREVIEW_H: u32 = 720;

// ---------------------------------------------------------------- settings

/// Persisted app settings (config dir). HUD overrides live here so they
/// survive restarts and apply to every render until reset.
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
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
    crf: u32,
    encoder: String,
    preset: String,
    results_secs: f64,
    /// HUD element key -> forced on/off. Absent key = follow the config.
    hud_overrides: BTreeMap<String, bool>,
    recent_replays: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            last_replay: None,
            last_config: None,
            game_assets: None,
            output_dir: None,
            file_name: String::new(),
            ffmpeg: None,
            width: 1920,
            height: 1080,
            fps: 60,
            crf: 18,
            encoder: "auto".into(),
            preset: "veryfast".into(),
            results_secs: 4.0,
            hud_overrides: BTreeMap::new(),
            recent_replays: Vec::new(),
        }
    }
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
    fn load() -> Settings {
        let path = config_dir().join("settings.json");
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let dir = config_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(dir.join("settings.json"), json);
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
    cfg: SkinConfig,
    params: SceneParams,
}

#[derive(Default)]
struct Inner {
    replay: Option<(PathBuf, Replay)>,
    map: Option<(PathBuf, Map)>,
    map_source: String,
    /// True when the cached map's hash does not match the replay header.
    map_hash_mismatch: bool,
    config_path: Option<PathBuf>,
    base_config: SkinConfig,
    settings: Settings,
    preview: Option<PreviewCtx>,
}

struct Shared {
    inner: Mutex<Inner>,
    cancel: AtomicBool,
    rendering: AtomicBool,
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
struct VerifyDto {
    consistent: bool,
    /// Failed error-level checks as "name: expected X, got Y".
    problems: Vec<String>,
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
struct StatusDto {
    replay: Option<ReplayDto>,
    map: Option<MapDto>,
    config: ConfigDto,
    settings: Settings,
    rendering: bool,
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

/// The config as it renders: file config + game assets + HUD overrides.
fn effective_config(inner: &Inner) -> SkinConfig {
    let mut cfg = inner.base_config.clone();
    apply_overrides(&mut cfg, &inner.settings.hud_overrides);
    cfg
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

/// ffmpeg to run: explicit setting, else a bundled sibling of the exe
/// (Windows installer ships one), else PATH.
fn resolve_ffmpeg(settings: &Settings) -> String {
    if let Some(f) = &settings.ffmpeg {
        if !f.trim().is_empty() {
            return f.clone();
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
            let sibling = dir.join(name);
            if sibling.exists() {
                return sibling.to_string_lossy().into_owned();
            }
        }
    }
    "ffmpeg".into()
}

fn verify_dto(replay: &Replay, map: &Map) -> VerifyDto {
    let report = integrity::verify_replay(replay, map);
    let problems = report
        .failed_checks()
        .filter(|c| c.severity == integrity::Severity::Error)
        .map(|c| format!("{}: expected {}, got {}", c.name, c.expected, c.actual))
        .collect();
    VerifyDto {
        consistent: report.consistent(),
        problems,
    }
}

fn assemble_status(inner: &Inner, rendering: bool) -> StatusDto {
    let replay = inner.replay.as_ref().map(|(path, r)| {
        let verify = inner.map.as_ref().map(|(_, m)| verify_dto(r, m));
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
    let base_hud = hud_flags(&inner.base_config);
    let effective_hud = hud_flags(&effective_config(inner));
    StatusDto {
        replay,
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
    }
}

fn png_data_url(rgba: &[u8], w: u32, h: u32) -> Result<String, String> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(std::io::Cursor::new(&mut buf), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(err_str)?;
        writer.write_image_data(rgba).map_err(err_str)?;
    }
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&buf)
    ))
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
fn invalidate_preview(inner: &mut Inner) {
    inner.preview = None;
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_status(state: tauri::State<'_, App>) -> StatusDto {
    let app = state.inner();
    let inner = app.lock();
    assemble_status(&inner, app.rendering.load(Ordering::SeqCst))
}

#[tauri::command]
fn load_replay(state: tauri::State<'_, App>, path: String) -> Result<StatusDto, String> {
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
    inner.settings.last_replay = Some(path.clone());
    let recent = &mut inner.settings.recent_replays;
    recent.retain(|p| p != &path);
    recent.insert(0, path.clone());
    recent.truncate(8);
    inner.settings.save();
    inner.replay = Some((PathBuf::from(path), replay));
    invalidate_preview(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn load_map(state: tauri::State<'_, App>, path: String) -> Result<StatusDto, String> {
    let app = state.inner();
    let map = Map::from_path(&path).map_err(err_str)?;
    let mut inner = app.lock();
    inner.map = Some((PathBuf::from(path), map));
    inner.map_source = "local".into();
    inner.map_hash_mismatch = false;
    invalidate_preview(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
async fn download_map(state: tauri::State<'_, App>) -> Result<StatusDto, String> {
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
            .map_err(|e| format!("map lookup failed: {e}"))?
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
            invalidate_preview(&mut inner);
        }
        Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
    })
    .await
    .map_err(err_str)?
}

#[tauri::command]
fn load_config(state: tauri::State<'_, App>, path: String) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let p = Some(PathBuf::from(&path));
    let cfg = load_base_config(&p, &inner.settings.game_assets)?;
    inner.base_config = cfg;
    inner.config_path = p;
    inner.settings.last_config = Some(path);
    inner.settings.save();
    invalidate_preview(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn clear_config(state: tauri::State<'_, App>) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    inner.config_path = None;
    inner.base_config = load_base_config(&None, &inner.settings.game_assets)?;
    inner.settings.last_config = None;
    inner.settings.save();
    invalidate_preview(&mut inner);
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
            if p.to_lowercase().ends_with(".exe") {
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

/// Looks for the game's exe in the usual Steam locations (incl. extra
/// library folders from libraryfolders.vdf). Returns the exe path.
#[tauri::command]
fn detect_game() -> Option<String> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for base in [
        "C:\\Program Files (x86)\\Steam",
        "C:\\Program Files\\Steam",
    ] {
        roots.push(PathBuf::from(base));
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
    for root in roots {
        for game_dir in ["Rhythia", "SoundSpacePlus", "Sound Space Plus"] {
            let exe = root
                .join("steamapps")
                .join("common")
                .join(game_dir)
                .join("rhythia.exe");
            if exe.exists() {
                return Some(exe.to_string_lossy().into_owned());
            }
        }
    }
    None
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
    match value {
        Some(v) => {
            inner.settings.hud_overrides.insert(key, v);
        }
        None => {
            inner.settings.hud_overrides.remove(&key);
        }
    }
    inner.settings.save();
    invalidate_preview(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[tauri::command]
fn reset_hud_overrides(state: tauri::State<'_, App>) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    inner.settings.hud_overrides.clear();
    inner.settings.save();
    invalidate_preview(&mut inner);
    Ok(assemble_status(&inner, app.rendering.load(Ordering::SeqCst)))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OutputUpdate {
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<u32>,
    crf: Option<u32>,
    encoder: Option<String>,
    preset: Option<String>,
    results_secs: Option<f64>,
    output_dir: Option<String>,
    file_name: Option<String>,
    ffmpeg: Option<String>,
}

#[tauri::command]
fn set_output(state: tauri::State<'_, App>, update: OutputUpdate) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
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
    if let Some(v) = update.crf {
        s.crf = v.clamp(0, 51);
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
            let outcome = rhythia_sim::hitreg::match_hits(
                &m.notes,
                frames,
                rhythia_sim::hitreg::DEFAULT_WINDOW_MS,
            );
            outcome
                .results
                .iter()
                .filter(|r| !r.hit)
                .map(|r| m.notes[r.note_index].time_ms as f64)
                .filter(|&t| t <= run_end + rhythia_sim::hitreg::DEFAULT_WINDOW_MS)
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

#[tauri::command]
async fn preview(state: tauri::State<'_, App>, time_ms: f64) -> Result<String, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if app.rendering.load(Ordering::SeqCst) {
            return Err("rendering in progress".to_string());
        }
        let mut inner = app.lock();
        if inner.replay.is_none() || inner.map.is_none() {
            return Err("load a replay and map first".to_string());
        }
        if inner.preview.is_none() {
            let renderer =
                rhythia_render::Renderer::new(PREVIEW_W, PREVIEW_H).map_err(err_str)?;
            let cfg = effective_config(&inner);
            let params = SceneParams::from(&cfg);
            let skin = renderer.prepare_skin(&cfg);
            let (_, r) = inner.replay.as_ref().unwrap();
            let (_, m) = inner.map.as_ref().unwrap();
            let hud = rhythia_render::hud::HudState::new(m, r);
            inner.preview = Some(PreviewCtx {
                renderer,
                skin,
                hud,
                cfg,
                params,
            });
        }
        let inner = &*inner;
        let ctx = inner.preview.as_ref().unwrap();
        let (_, r) = inner.replay.as_ref().unwrap();
        let (_, m) = inner.map.as_ref().unwrap();
        let pixels = ctx
            .renderer
            .render_still(
                &ctx.params,
                &ctx.cfg,
                &ctx.skin,
                r,
                m,
                time_ms,
                Some(&ctx.hud),
            )
            .map_err(err_str)?;
        png_data_url(&pixels, PREVIEW_W, PREVIEW_H)
    })
    .await
    .map_err(err_str)?
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
        let params = SceneParams::from(&cfg);
        let renderer = rhythia_render::Renderer::new(w, h).map_err(err_str)?;
        let skin = renderer.prepare_skin(&cfg);
        let hud = rhythia_render::hud::HudState::new(m, r);
        let pixels = renderer
            .render_still(&params, &cfg, &skin, r, m, time_ms, Some(&hud))
            .map_err(err_str)?;
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
fn start_render(
    state: tauri::State<'_, App>,
    app_handle: tauri::AppHandle,
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
            suggested_name(&inner)
        } else {
            sanitize_filename(&s.file_name)
        };
        if !name.to_lowercase().ends_with(".mp4") {
            name.push_str(".mp4");
        }
        let out = PathBuf::from(&dir).join(name);
        std::fs::create_dir_all(&dir).map_err(err_str)?;
        let job = RenderJob {
            replay: replay.clone(),
            map: map.clone(),
            cfg: effective_config(&inner),
            width: s.width,
            height: s.height,
            fps: s.fps,
            crf: s.crf,
            encoder: s.encoder.clone(),
            preset: s.preset.clone(),
            results_secs: s.results_secs,
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
                let _ = app_handle.emit("render-error", e.to_string());
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
    crf: u32,
    encoder: String,
    preset: String,
    results_secs: f64,
    ffmpeg: String,
    out: PathBuf,
}

fn run_render_job(
    app: &App,
    handle: &tauri::AppHandle,
    job: RenderJob,
) -> Result<PathBuf, rhythia_render::Error> {
    let _ = handle.emit("render-stage", "starting GPU renderer");
    let renderer = rhythia_render::Renderer::new(job.width, job.height)?;
    let params = SceneParams::from(&job.cfg);

    // Probe hardware encoders unless one was forced.
    let encoder = match job.encoder.as_str() {
        "auto" => ["nvenc", "qsv", "vaapi"]
            .into_iter()
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

    let end_ms = if job.replay.failed() {
        f64::from(job.replay.fail_time_ms)
    } else {
        job.replay.length_ms()
    };
    let opts = rhythia_render::video::VideoOptions {
        fps: job.fps,
        start_ms: 0.0,
        end_ms,
        ffmpeg: job.ffmpeg.clone(),
        audio,
        crf: job.crf,
        preset: job.preset.clone(),
        encoder,
        results_secs: job.results_secs,
    };

    let started = std::time::Instant::now();
    let mut last_emit = std::time::Instant::now();
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
    Ok(job.out)
}

#[tauri::command]
fn cancel_render(state: tauri::State<'_, App>) {
    state.inner().cancel.store(true, Ordering::SeqCst);
}

#[derive(Serialize)]
struct EncoderProbe {
    available: Vec<String>,
    /// Encoder -> why it is unavailable (ffmpeg's own words).
    unavailable: BTreeMap<String, String>,
}

#[tauri::command]
async fn probe_encoders(state: tauri::State<'_, App>) -> Result<EncoderProbe, String> {
    let app = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let ffmpeg = {
            let inner = app.lock();
            resolve_ffmpeg(&inner.settings)
        };
        let mut available = vec!["auto".to_string(), "x264".to_string()];
        let mut unavailable = BTreeMap::new();
        for e in ["nvenc", "qsv", "vaapi"] {
            match rhythia_render::video::encoder_error(&ffmpeg, e) {
                None => available.push(e.to_string()),
                Some(reason) => {
                    unavailable.insert(e.to_string(), reason);
                }
            }
        }
        Ok(EncoderProbe {
            available,
            unavailable,
        })
    })
    .await
    .map_err(err_str)?
}

// -------------------------------------------------------------------- main

fn main() {
    let shared: App = Arc::new(Shared {
        inner: Mutex::new(Inner {
            settings: Settings::load(),
            ..Inner::default()
        }),
        cancel: AtomicBool::new(false),
        rendering: AtomicBool::new(false),
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
            }
        }
    }

    tauri::Builder::default()
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(shared)
        .invoke_handler(tauri::generate_handler![
            get_status,
            load_replay,
            load_map,
            download_map,
            load_config,
            clear_config,
            set_game_assets,
            detect_game,
            set_hud_override,
            reset_hud_overrides,
            set_output,
            suggest_file_name,
            timeline,
            preview,
            export_frame,
            start_render,
            cancel_render,
            probe_encoders,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
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
            }
        });
}
