//! Autoplay in the main window: the export folder listing, picking a map,
//! the settings, and leaving the mode again. The synthetic run is a
//! `Replay` value in memory and nothing here writes it anywhere.

use crate::{App, Inner, Settings, StatusDto};
use rhythia_errcode::Area;
use rhythia_formats::map::Map;
use rhythia_render::autoplay::{self, AutoplaySettings};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::OnceLock;

/// The active Autoplay session: which export and which settings.
pub struct AutoplaySession {
    pub rhm: PathBuf,
    pub settings: AutoplaySettings,
}

#[derive(Serialize, Clone, Debug)]
pub struct ExportEntryDto {
    pub path: String,
    pub file_name: String,
    pub title: String,
    pub song_name: String,
    pub mappers: Vec<String>,
    pub duration_ms: i64,
    pub note_count: usize,
    pub has_audio: bool,
    pub exported_unix_ms: i64,
    pub unreadable: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ExportsDto {
    pub folder: String,
    pub source: String,
    pub maps: Vec<ExportEntryDto>,
    /// Changes whenever the folder's listing changes; the window polls and
    /// re-renders only on a new token.
    pub token: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct AutoplayDto {
    pub rhm: String,
    pub file_name: String,
    pub title: String,
    pub song_name: String,
    pub mappers: Vec<String>,
    pub duration_ms: i64,
    pub note_count: usize,
    pub has_audio: bool,
    pub settings: AutoplaySettings,
    /// "owner" or "dev": which mark key this build carries.
    pub key_kind: String,
    /// A note from the pick (the map has no audio), already stamped.
    pub note: Option<String>,
}

/// A failure of an Autoplay action with its code.
pub fn autoplay_err(msg: impl AsRef<str>) -> String {
    rhythia_errcode::stamp_in(Area::Autoplay, msg.as_ref())
}

/// The mark pattern of this build, generated once.
pub fn mark_pattern() -> &'static [f32] {
    static PATTERN: OnceLock<Vec<f32>> = OnceLock::new();
    PATTERN.get_or_init(|| rhythia_render::mark::pattern(&rhythia_render::mark::built_in().0))
}

/// The export folder and which rule found it: the manual setting, the
/// environment override, or the game's data folder.
pub fn exports_dir(settings: &Settings) -> Result<(PathBuf, &'static str), String> {
    let env = std::env::var_os(crate::game_data::ENV_OVERRIDE).filter(|s| !s.is_empty()).map(PathBuf::from);
    exports_dir_with(settings, env, crate::game_data::game_data_dir())
}

pub fn exports_dir_with(
    settings: &Settings,
    env: Option<PathBuf>,
    game: Result<PathBuf, Vec<PathBuf>>,
) -> Result<(PathBuf, &'static str), String> {
    if let Some(d) = settings.autoplay_exports_dir.as_deref().filter(|d| !d.trim().is_empty()) {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Ok((p, "setting"));
        }
    }
    if let Some(e) = env {
        if e.is_dir() {
            return Ok((e.join("exports"), "env"));
        }
    }
    match game {
        Ok(g) => Ok((g.join("exports"), "game")),
        Err(tried) => {
            let list: Vec<String> = tried.iter().map(|p| p.display().to_string()).collect();
            Err(format!("export folder not found. Looked in: {}", list.join("; ")))
        }
    }
}

const MAX_MAP_JSON: u64 = 64 << 20;

/// The map entry of an export, without its audio or cover.
pub fn read_header(path: &Path) -> Result<ExportEntryDto, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a map archive: {e}"))?;
    let mut bytes = Vec::new();
    {
        let entry = zip.by_name("map").map_err(|_| "not a map archive: no map entry".to_string())?;
        entry.take(MAX_MAP_JSON).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    }
    let has_audio = zip.by_name("audio").is_ok();
    let map = Map::from_cache_json(&bytes).map_err(|e| format!("map archive unreadable: {e}"))?;
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let exported_unix_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok(ExportEntryDto {
        path: path.to_string_lossy().into_owned(),
        file_name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        title: if map.meta.title.is_empty() { map.meta.song_name.clone() } else { map.meta.title.clone() },
        song_name: map.meta.song_name.clone(),
        mappers: map.meta.mappers.clone(),
        duration_ms: map.meta.duration_ms,
        note_count: map.notes.len(),
        has_audio,
        exported_unix_ms,
        unreadable: false,
    })
}

/// Every `.rhm` in the folder, newest export first, with a token over the
/// listing. A missing folder lists as empty (the game has not exported
/// anything yet); a damaged archive lists as unreadable instead of failing
/// the whole listing.
pub fn list_exports(dir: &Path) -> Result<ExportsDto, String> {
    use std::hash::{Hash, Hasher};
    let mut files: Vec<(PathBuf, i64, u64)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            let is_rhm = p.extension().map(|e| e.eq_ignore_ascii_case("rhm")).unwrap_or(false);
            if !is_rhm || !p.is_file() {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            files.push((p, mtime, meta.len()));
        }
    }
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (p, mtime, len) in &files {
        p.hash(&mut h);
        mtime.hash(&mut h);
        len.hash(&mut h);
    }
    let maps = files
        .iter()
        .map(|(p, mtime, _)| {
            read_header(p).unwrap_or_else(|_| ExportEntryDto {
                path: p.to_string_lossy().into_owned(),
                file_name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                title: format!(
                    "{} (unreadable)",
                    p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                ),
                song_name: String::new(),
                mappers: Vec::new(),
                duration_ms: 0,
                note_count: 0,
                has_audio: false,
                exported_unix_ms: *mtime,
                unreadable: true,
            })
        })
        .collect();
    Ok(ExportsDto { folder: dir.to_string_lossy().into_owned(), source: String::new(), maps, token: h.finish() })
}

/// Loads the export, builds the run and makes it the session. Returns the
/// stamped note when the map has no audio.
pub fn start_session(inner: &mut Inner, rhm: PathBuf) -> Result<Option<String>, String> {
    let map = Map::from_path(&rhm).map_err(|e| autoplay_err(format!("map archive unreadable: {e}")))?;
    if map.notes.is_empty() {
        return Err(autoplay_err("the map has no notes"));
    }
    let settings = inner.settings.autoplay.normalized();
    let fps = inner.settings.fps.clamp(24, 240);
    let replay = autoplay::generate(&map, &settings, fps, mark_pattern()).map_err(|e| autoplay_err(e.to_string()))?;
    let note = map
        .audio
        .is_none()
        .then(|| format!("note: {}", autoplay_err("the map has no audio, rendering silent")));
    inner.ghost = None;
    inner.clip = None;
    inner.replay_restored = false;
    inner.map_hash_mismatch = false;
    inner.replay = Some((rhm.clone(), replay));
    inner.map = Some((rhm.clone(), map));
    inner.map_source = "autoplay".into();
    inner.autoplay = Some(AutoplaySession { rhm: rhm.clone(), settings });
    inner.autoplay_note = note.clone();
    inner.settings.autoplay = settings;
    inner.settings.last_autoplay = Some(rhm.to_string_lossy().into_owned());
    inner.settings.save();
    crate::invalidate_preview(inner);
    Ok(note)
}

/// Rebuilds the run after a settings change.
pub fn regenerate(inner: &mut Inner) -> Result<(), String> {
    let Some(sess) = inner.autoplay.as_ref() else { return Ok(()) };
    let rhm = sess.rhm.clone();
    let settings = sess.settings;
    let Some((_, map)) = inner.map.as_ref() else { return Ok(()) };
    let fps = inner.settings.fps.clamp(24, 240);
    let replay = autoplay::generate(map, &settings, fps, mark_pattern()).map_err(|e| autoplay_err(e.to_string()))?;
    inner.replay = Some((rhm, replay));
    inner.clip = None;
    crate::invalidate_preview(inner);
    Ok(())
}

/// Drops the session and everything it loaded.
pub fn clear_session(inner: &mut Inner) {
    if inner.autoplay.take().is_some() {
        inner.autoplay_note = None;
        inner.replay = None;
        inner.map = None;
        inner.map_source.clear();
        inner.clip = None;
        inner.settings.last_autoplay = None;
        inner.settings.save();
        crate::invalidate_preview(inner);
    }
}

/// The status block for the window.
pub fn status_dto(inner: &Inner) -> Option<AutoplayDto> {
    let sess = inner.autoplay.as_ref()?;
    let map = inner.map.as_ref().map(|(_, m)| m);
    Some(AutoplayDto {
        rhm: sess.rhm.to_string_lossy().into_owned(),
        file_name: sess.rhm.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        title: map.map(|m| if m.meta.title.is_empty() { m.meta.song_name.clone() } else { m.meta.title.clone() }).unwrap_or_default(),
        song_name: map.map(|m| m.meta.song_name.clone()).unwrap_or_default(),
        mappers: map.map(|m| m.meta.mappers.clone()).unwrap_or_default(),
        duration_ms: map.map(|m| m.meta.duration_ms).unwrap_or(0),
        note_count: map.map(|m| m.notes.len()).unwrap_or(0),
        has_audio: map.map(|m| m.audio.is_some()).unwrap_or(false),
        settings: sess.settings,
        key_kind: rhythia_render::mark::built_in().1.label().to_string(),
        note: inner.autoplay_note.clone(),
    })
}

/// The `autoplay` section of the diagnostics file.
pub fn diagnostics_section(inner: &Inner) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    match exports_dir(&inner.settings) {
        Ok((dir, source)) => {
            let _ = writeln!(s, "  export folder: {} (from {source}, {})", crate::redact_path(&dir.display().to_string()), if dir.is_dir() { "exists" } else { "not there yet" });
        }
        Err(e) => {
            let _ = writeln!(s, "  export folder: {}", crate::redact_path(&e));
        }
    }
    match &inner.autoplay_exports_seen {
        Some((_, n)) => {
            let _ = writeln!(s, "  maps seen at the last listing: {n}");
        }
        None => {
            let _ = writeln!(s, "  maps seen at the last listing: no listing this session");
        }
    }
    match &inner.autoplay {
        Some(sess) => {
            let _ = writeln!(s, "  session: {} ({:?})", sess.rhm.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), sess.settings);
        }
        None => {
            let _ = writeln!(s, "  session: none");
        }
    }
    if let Some(n) = &inner.autoplay_note {
        let _ = writeln!(s, "  note: {n}");
    }
    let _ = writeln!(s, "  settings: {:?}", inner.settings.autoplay);
    let _ = writeln!(s, "  key: {}", rhythia_render::mark::built_in().1.label());
    s
}

fn status(app: &App, inner: &Inner) -> StatusDto {
    crate::assemble_status(inner, app.rendering.load(Ordering::SeqCst))
}

#[tauri::command]
pub fn autoplay_exports(state: tauri::State<'_, App>) -> Result<ExportsDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let (dir, source) = exports_dir(&inner.settings).map_err(autoplay_err)?;
    let mut listing = list_exports(&dir)?;
    listing.source = source.to_string();
    inner.autoplay_exports_seen = Some((listing.folder.clone(), listing.maps.len()));
    Ok(listing)
}

#[tauri::command]
pub fn autoplay_pick(state: tauri::State<'_, App>, app_handle: tauri::AppHandle, path: String) -> Result<StatusDto, String> {
    let app = state.inner();
    if app.rendering.load(Ordering::SeqCst) {
        return Err("rendering in progress".into());
    }
    let mut inner = app.lock();
    start_session(&mut inner, PathBuf::from(path))?;
    crate::notify_sources_changed(&app_handle);
    Ok(status(app, &inner))
}

#[tauri::command]
pub fn autoplay_settings(
    state: tauri::State<'_, App>,
    app_handle: tauri::AppHandle,
    settings: AutoplaySettings,
) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    let settings = settings.normalized();
    inner.settings.autoplay = settings;
    if let Some(sess) = inner.autoplay.as_mut() {
        sess.settings = settings;
    }
    inner.settings.save();
    regenerate(&mut inner)?;
    crate::notify_sources_changed(&app_handle);
    Ok(status(app, &inner))
}

#[tauri::command]
pub fn autoplay_clear(state: tauri::State<'_, App>, app_handle: tauri::AppHandle) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    clear_session(&mut inner);
    crate::notify_sources_changed(&app_handle);
    Ok(status(app, &inner))
}

#[tauri::command]
pub fn autoplay_set_folder(state: tauri::State<'_, App>, path: Option<String>) -> Result<StatusDto, String> {
    let app = state.inner();
    let mut inner = app.lock();
    inner.settings.autoplay_exports_dir = path.filter(|p| !p.trim().is_empty());
    inner.settings.save();
    Ok(status(app, &inner))
}

#[tauri::command]
pub fn autoplay_open_folder(state: tauri::State<'_, App>) -> Result<(), String> {
    let app = state.inner();
    let dir = {
        let inner = app.lock();
        exports_dir(&inner.settings).map_err(autoplay_err)?.0
    };
    if !dir.is_dir() {
        return Err(autoplay_err(format!("export folder not found. Looked in: {}", dir.display())));
    }
    tauri_plugin_opener::reveal_item_in_dir(&dir).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A `.rhm` with a `map` entry (title, `notes` notes) and optionally an
    /// `audio` entry.
    fn write_rhm(path: &std::path::Path, title: &str, notes: usize, audio: bool) {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let notes_json: Vec<String> = (0..notes).map(|i| format!(r#"{{"Time":{},"X":1,"Y":1}}"#, 300 + i as i64 * 250)).collect();
        let json = format!(
            r#"{{"OnlineId":1,"LegacyId":"x","Title":"{title}","SongName":"{title} song","Duration":5000,"Notes":[{}]}}"#,
            notes_json.join(",")
        );
        zw.start_file("map", opts).unwrap();
        zw.write_all(json.as_bytes()).unwrap();
        if audio {
            zw.start_file("audio", opts).unwrap();
            zw.write_all(b"fake-mp3").unwrap();
        }
        std::fs::write(path, zw.finish().unwrap().into_inner()).unwrap();
    }

    #[test]
    fn the_listing_reads_headers_newest_first_and_hashes_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        write_rhm(&dir.path().join("a.rhm"), "A", 3, true);
        std::thread::sleep(std::time::Duration::from_millis(30));
        write_rhm(&dir.path().join("b.rhm"), "B", 5, false);
        let l1 = list_exports(dir.path()).unwrap();
        assert_eq!(l1.maps.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), vec!["B", "A"]);
        assert_eq!(l1.maps[0].note_count, 5);
        assert!(!l1.maps[0].has_audio && l1.maps[1].has_audio);
        let l2 = list_exports(dir.path()).unwrap();
        assert_eq!(l1.token, l2.token);
        std::thread::sleep(std::time::Duration::from_millis(30));
        write_rhm(&dir.path().join("c.rhm"), "C", 1, true);
        assert_ne!(list_exports(dir.path()).unwrap().token, l1.token);
    }

    #[test]
    fn a_damaged_archive_is_listed_as_unreadable_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.rhm"), b"not a zip").unwrap();
        let l = list_exports(dir.path()).unwrap();
        assert!(l.maps.is_empty() || l.maps[0].title.contains("unreadable"));
    }

    #[test]
    fn the_exports_folder_prefers_the_setting_then_the_env_then_the_game() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = crate::Settings::default();
        s.autoplay_exports_dir = Some(dir.path().to_string_lossy().into_owned());
        assert_eq!(exports_dir_with(&s, None, Err(vec![])).unwrap().1, "setting");
        s.autoplay_exports_dir = Some("/does/not/exist".into());
        let e = exports_dir_with(&s, None, Err(vec![std::path::PathBuf::from("/nope")])).unwrap_err();
        assert!(e.contains("export folder not found"), "{e}");
        let game = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(game.path().join("exports")).unwrap();
        let (p, src) = exports_dir_with(&s, None, Ok(game.path().to_path_buf())).unwrap();
        assert_eq!(src, "game");
        assert_eq!(p, game.path().join("exports"));
        let (p, src) = exports_dir_with(&s, Some(game.path().to_path_buf()), Err(vec![])).unwrap();
        assert_eq!(src, "env");
        assert_eq!(p, game.path().join("exports"));
    }

    #[test]
    fn a_map_without_audio_renders_silent_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let rhm = dir.path().join("silent.rhm");
        write_rhm(&rhm, "Silent", 4, false);
        let mut inner = crate::Inner::default();
        let note = start_session(&mut inner, rhm.clone()).unwrap();
        assert!(inner.autoplay.is_some() && inner.replay.is_some() && inner.map.is_some());
        let st = crate::assemble_status(&inner, false);
        let dto = st.autoplay.as_ref().unwrap();
        assert!(!dto.has_audio);
        assert!(note.as_deref().unwrap_or("").contains("RH-AUT-904"), "{note:?}");
        assert!(dto.note.as_deref().unwrap_or("").contains("RH-AUT-904"));
        assert_eq!(inner.settings.last_autoplay.as_deref(), Some(rhm.to_string_lossy().as_ref()));
    }
}
