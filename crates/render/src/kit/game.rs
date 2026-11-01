//! Send to game: the baked note written straight into the game's own data
//! folder, so a test needs no file dialog and no import. The kit owns one
//! profile there (`rhythr-kit.json`) and three files under `skins/`; it
//! never touches anything else, never deletes, and refuses when the game
//! relocates its data (`DataPath`) because it cannot know what moves.

use crate::kit::doc::Design;
use crate::kit::export::{colorset_text, mtl_text, to_obj};
use crate::kit::levels::evaluate;
use crate::kit::mesh::build_mesh;
use crate::kit::KitError;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The one name the kit uses in the game: the profile, the obj, the mtl
/// and the colorset all carry it, so a later send overwrites the same
/// files and the game folder never fills up.
pub const PROFILE_NAME: &str = "rhythr-kit";
pub const MSG_AGAIN: &str =
    "Sent to the game. Apply the profile rhythr-kit again (switch away and back) to load it.";
pub const MSG_NO_PROFILE: &str = "Sent the files, but the active profile could not be read: import a saved .rhs once, then Send to game keeps it current.";

/// The first send names the folder once, so a wrong target shows up
/// instead of staying silent.
pub fn msg_first(folder: &Path) -> String {
    format!(
        "Sent to the game: {}. Pick the profile rhythr-kit in the game's settings.",
        folder.display()
    )
}

#[derive(Serialize, Debug, PartialEq)]
pub struct SendReport {
    pub folder: PathBuf,
    pub profile_created: bool,
    pub message: String,
}

#[derive(Debug)]
pub enum SendError {
    /// The kernel refused the design; the caller words it for the user.
    Design(KitError),
    Io {
        path: PathBuf,
        error: String,
    },
    /// `framework.json` relocates the game's data; nothing was written.
    DataPath(String),
}

impl From<KitError> for SendError {
    fn from(e: KitError) -> Self {
        SendError::Design(e)
    }
}

fn io(path: &Path, e: std::io::Error) -> SendError {
    SendError::Io {
        path: path.to_path_buf(),
        error: e.to_string(),
    }
}

/// `framework.json` as a JSON object when it exists and parses.
fn framework(data_dir: &Path) -> Option<serde_json::Value> {
    let bytes = std::fs::read(data_dir.join("framework.json")).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.is_object().then_some(v)
}

/// The game's `"Key": {"Value": "..."}` shape.
fn value_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key)?.get("Value")?.as_str()
}

/// The active profile's JSON object, or None when `framework.json`, the
/// name it holds or the file it names cannot be read. A name with a path
/// separator or a parent step is not a profile name and is not followed;
/// neither are the game's own files (`userconfig.json` holds secrets,
/// `framework.json` is not a profile) or the kit's profile, so a damaged
/// `framework.json` can never make the kit copy them.
fn active_profile(data_dir: &Path, fw: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let name = value_str(fw?, "CurrentGameProfile")?;
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return None;
    }
    let own = format!("{PROFILE_NAME}.json");
    if ["userconfig.json", "framework.json", own.as_str()]
        .iter()
        .any(|r| name.eq_ignore_ascii_case(r))
    {
        return None;
    }
    let bytes = std::fs::read(data_dir.join(name)).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.is_object().then_some(v)
}

/// Writes the baked `design` into `data_dir` (the game's data folder):
/// `skins/notes/rhythr-kit.obj` and `.mtl`, `skins/colorsets/rhythr-kit.txt`
/// (only when the design has a valid colour), and once a profile
/// `rhythr-kit.json` copied from the active profile with the note and the
/// colorset pointed at those files. Nothing else in the folder is read or
/// written; a non-empty `DataPath` in `framework.json` stops the send
/// before any file is touched.
pub fn send_to_game(design: &Design, data_dir: &Path) -> Result<SendReport, SendError> {
    let fw = framework(data_dir);
    if let Some(p) = fw
        .as_ref()
        .and_then(|f| value_str(f, "DataPath"))
        .filter(|s| !s.trim().is_empty())
    {
        return Err(SendError::DataPath(p.to_string()));
    }
    let mesh = build_mesh(&evaluate(design)?)?;
    let obj_name = format!("{PROFILE_NAME}.obj");
    let mtl_name = format!("{PROFILE_NAME}.mtl");
    let cs_name = format!("{PROFILE_NAME}.txt");
    let obj = to_obj(&mesh, PROFILE_NAME, &mtl_name);
    let notes = data_dir.join("skins").join("notes");
    let colorsets = data_dir.join("skins").join("colorsets");
    for d in [&notes, &colorsets] {
        std::fs::create_dir_all(d).map_err(|e| io(d, e))?;
    }
    write_atomic(&notes.join(&obj_name), obj.as_bytes())?;
    write_atomic(&notes.join(&mtl_name), mtl_text().as_bytes())?;
    let colorset = colorset_text(&design.colors);
    let has_colorset = !colorset.is_empty();
    if has_colorset {
        write_atomic(&colorsets.join(&cs_name), colorset.as_bytes())?;
    }
    let folder = data_dir.to_path_buf();
    let profile_path = data_dir.join(format!("{PROFILE_NAME}.json"));
    if profile_path.exists() {
        return Ok(SendReport {
            folder,
            profile_created: false,
            message: MSG_AGAIN.into(),
        });
    }
    let no_profile = |folder: PathBuf| SendReport {
        folder,
        profile_created: false,
        message: MSG_NO_PROFILE.into(),
    };
    let Some(active) = active_profile(data_dir, fw.as_ref()) else {
        return Ok(no_profile(folder));
    };
    let obj_ref = game_path(data_dir, &["skins", "notes", obj_name.as_str()]);
    let cs_ref =
        has_colorset.then(|| game_path(data_dir, &["skins", "colorsets", cs_name.as_str()]));
    let Ok(profile) = profile_from(&active, &obj_ref, cs_ref.as_deref()) else {
        return Ok(no_profile(folder));
    };
    let bytes = serde_json::to_vec(&profile).map_err(|e| SendError::Io {
        path: profile_path.clone(),
        error: e.to_string(),
    })?;
    write_atomic(&profile_path, &bytes)?;
    Ok(SendReport {
        folder,
        profile_created: true,
        message: msg_first(data_dir),
    })
}

/// The path string the game stores for a file under its data folder.
/// Inside a Proton prefix (`.../pfx/drive_c/...`) the game sees a Windows
/// drive, so the string is rebuilt from `drive_c` as `C:\...`; anywhere
/// else it is the host path.
pub fn game_path(data_dir: &Path, rel: &[&str]) -> String {
    let comps: Vec<String> = data_dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if let Some(i) = comps
        .windows(2)
        .position(|w| w[0] == "pfx" && w[1] == "drive_c")
    {
        let mut parts: Vec<&str> = comps[i + 2..].iter().map(String::as_str).collect();
        parts.extend(rel.iter().copied());
        return format!("C:\\{}", parts.join("\\"));
    }
    let mut p = data_dir.to_path_buf();
    for r in rel {
        p.push(r);
    }
    p.display().to_string()
}

/// A copy of `active` with `NoteSkin` pointed at `obj` and, when given,
/// `ColorSet` at `colorset`; every other key stays as it is. Keys that are
/// missing or not in the game's `{"Value": ...}` shape are replaced whole.
pub fn profile_from(
    active: &serde_json::Value,
    obj: &str,
    colorset: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut profile = active.clone();
    let map = profile
        .as_object_mut()
        .ok_or_else(|| "the active profile is not a JSON object".to_string())?;
    map.insert("NoteSkin".into(), serde_json::json!({ "Value": obj }));
    if let Some(cs) = colorset {
        map.insert("ColorSet".into(), serde_json::json!({ "Value": cs }));
    }
    Ok(profile)
}

/// Writes next to the target and renames over it, so the game never reads
/// a half-written file; a failed rename leaves no temporary behind.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SendError> {
    let tmp = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => path.with_extension(format!("{ext}.tmp")),
        None => path.with_extension("tmp"),
    };
    std::fs::write(&tmp, bytes).map_err(|e| io(path, e))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(path, e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path, profile: serde_json::Value) {
        std::fs::write(
            dir.join("framework.json"),
            r#"{"CurrentGameProfile":{"Value":"p1.json"},"DataPath":{"Value":""}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("p1.json"), serde_json::to_vec(&profile).unwrap()).unwrap();
    }

    fn active() -> serde_json::Value {
        serde_json::json!({
            "ApproachRate": {"Value": 3.25},
            "NoteSkin": {"Value": "old.obj"},
            "ColorSet": {"Value": "old.txt"},
            "FpsCap": {"Value": 360}
        })
    }

    fn read_json(p: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
    }

    fn files_under(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    out.extend(files_under(&p));
                } else {
                    out.push(p);
                }
            }
        }
        out
    }

    fn value_of<'a>(v: &'a serde_json::Value, key: &str) -> &'a str {
        v[key]["Value"].as_str().unwrap()
    }

    #[test]
    fn first_send_writes_the_files_and_creates_the_profile_once() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        fixture(dir, active());
        std::fs::create_dir_all(dir.join("skins").join("notes")).unwrap();
        std::fs::create_dir_all(dir.join("skins").join("colorsets")).unwrap();

        let r = send_to_game(&Design::default_note(), dir).unwrap();
        assert!(r.profile_created);
        assert_eq!(r.message, msg_first(dir));
        assert_eq!(r.folder, dir);

        let obj = std::fs::read_to_string(dir.join("skins/notes/rhythr-kit.obj")).unwrap();
        assert!(obj.starts_with("# rhythr note kit\nmtllib rhythr-kit.mtl\no rhythr-kit\n"));
        let mtl = std::fs::read_to_string(dir.join("skins/notes/rhythr-kit.mtl")).unwrap();
        assert_eq!(mtl, crate::kit::export::mtl_text());
        let cs = std::fs::read_to_string(dir.join("skins/colorsets/rhythr-kit.txt")).unwrap();
        assert_eq!(cs, "#ffffff\r\n#a2e0ff\r\n#a2e0ff\r\n");

        let profile_path = dir.join("rhythr-kit.json");
        let profile = read_json(&profile_path);
        assert!(value_of(&profile, "NoteSkin").ends_with("rhythr-kit.obj"));
        assert!(value_of(&profile, "ColorSet").ends_with("rhythr-kit.txt"));
        assert_eq!(profile["ApproachRate"]["Value"], 3.25);
        assert_eq!(profile["FpsCap"]["Value"], 360);
        assert!(files_under(dir)
            .iter()
            .all(|p| p.extension().and_then(|e| e.to_str()) != Some("tmp")));

        let before = std::fs::read(&profile_path).unwrap();
        let again = send_to_game(&Design::default_note(), dir).unwrap();
        assert!(!again.profile_created);
        assert_eq!(again.message, MSG_AGAIN);
        assert_eq!(std::fs::read(&profile_path).unwrap(), before);
    }

    #[test]
    fn send_creates_the_skin_folders_it_needs() {
        let t = tempfile::tempdir().unwrap();
        fixture(t.path(), active());
        send_to_game(&Design::default_note(), t.path()).unwrap();
        assert!(t.path().join("skins/notes").is_dir());
        assert!(t.path().join("skins/colorsets").is_dir());
    }

    #[test]
    fn profile_rewrite_inserts_missing_keys_and_keeps_the_rest() {
        let active = serde_json::json!({"ApproachRate": {"Value": 40}, "NoteSkin": "old.obj"});
        let p = profile_from(
            &active,
            r"C:\x\rhythr-kit.obj",
            Some(r"C:\x\rhythr-kit.txt"),
        )
        .unwrap();
        assert_eq!(p["NoteSkin"]["Value"], r"C:\x\rhythr-kit.obj");
        assert_eq!(p["ColorSet"]["Value"], r"C:\x\rhythr-kit.txt");
        assert_eq!(p["ApproachRate"]["Value"], 40);
        assert!(profile_from(&serde_json::json!([1, 2]), "a", None).is_err());
    }

    #[test]
    fn no_colours_means_no_colorset_file_and_the_profile_keeps_its_own() {
        let t = tempfile::tempdir().unwrap();
        fixture(t.path(), active());
        let mut d = Design::default_note();
        d.colors = vec!["bad".into()];
        let r = send_to_game(&d, t.path()).unwrap();
        assert!(r.profile_created);
        assert!(!t.path().join("skins/colorsets/rhythr-kit.txt").exists());
        let profile = read_json(&t.path().join("rhythr-kit.json"));
        assert_eq!(value_of(&profile, "ColorSet"), "old.txt");
        assert!(value_of(&profile, "NoteSkin").ends_with("rhythr-kit.obj"));
    }

    #[test]
    fn a_missing_framework_or_profile_falls_back_to_files_only() {
        let t = tempfile::tempdir().unwrap();
        let r = send_to_game(&Design::default_note(), t.path()).unwrap();
        assert!(!r.profile_created);
        assert_eq!(r.message, MSG_NO_PROFILE);
        assert!(t.path().join("skins/notes/rhythr-kit.obj").exists());
        assert!(!t.path().join("rhythr-kit.json").exists());

        let t2 = tempfile::tempdir().unwrap();
        std::fs::write(
            t2.path().join("framework.json"),
            r#"{"CurrentGameProfile":{"Value":"gone.json"}}"#,
        )
        .unwrap();
        let r2 = send_to_game(&Design::default_note(), t2.path()).unwrap();
        assert!(!r2.profile_created);
        assert_eq!(r2.message, MSG_NO_PROFILE);
        assert!(!t2.path().join("rhythr-kit.json").exists());
    }

    #[test]
    fn a_framework_naming_a_reserved_file_is_not_followed() {
        // The kit never reads userconfig.json (secrets), framework.json or
        // its own profile as the active profile, whatever a damaged
        // framework.json names, in any letter case.
        for name in [
            "userconfig.json",
            "USERCONFIG.JSON",
            "framework.json",
            "rhythr-kit.json",
        ] {
            let t = tempfile::tempdir().unwrap();
            std::fs::write(
                t.path().join("framework.json"),
                format!(r#"{{"CurrentGameProfile":{{"Value":"{name}"}}}}"#),
            )
            .unwrap();
            std::fs::write(
                t.path().join("userconfig.json"),
                r#"{"OnlineSessionToken":{"Value":"secret"}}"#,
            )
            .unwrap();
            let r = send_to_game(&Design::default_note(), t.path()).unwrap();
            assert!(!r.profile_created, "{name}");
            assert_eq!(r.message, MSG_NO_PROFILE, "{name}");
            assert!(!t.path().join("rhythr-kit.json").exists(), "{name}");
        }
    }

    #[test]
    fn a_design_the_kernel_refuses_writes_nothing() {
        let t = tempfile::tempdir().unwrap();
        fixture(t.path(), active());
        let mut d = Design::default_note();
        d.parts[0].visible = false;
        let err = send_to_game(&d, t.path()).unwrap_err();
        assert!(matches!(err, SendError::Design(KitError::Empty)), "{err:?}");
        assert!(!t.path().join("skins").exists());
    }

    #[test]
    fn a_relocated_data_path_is_refused_before_anything_is_written() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(
            t.path().join("framework.json"),
            r#"{"CurrentGameProfile":{"Value":"p1.json"},"DataPath":{"Value":"D:\\games\\rhythia-data"}}"#,
        )
        .unwrap();
        let err = send_to_game(&Design::default_note(), t.path()).unwrap_err();
        match err {
            SendError::DataPath(p) => assert_eq!(p, r"D:\games\rhythia-data"),
            other => panic!("{other:?}"),
        }
        assert!(!t.path().join("skins").exists());
    }

    #[test]
    fn game_paths_follow_the_game_s_own_syntax() {
        let proton = Path::new(
            "/lib/steamapps/compatdata/2250500/pfx/drive_c/users/steamuser/AppData/Roaming/CapoRhythia",
        );
        assert_eq!(
            game_path(proton, &["skins", "notes", "rhythr-kit.obj"]),
            r"C:\users\steamuser\AppData\Roaming\CapoRhythia\skins\notes\rhythr-kit.obj"
        );
        let plain = Path::new("/home/x/CapoRhythia");
        assert_eq!(
            game_path(plain, &["skins", "notes", "rhythr-kit.obj"]),
            plain
                .join("skins")
                .join("notes")
                .join("rhythr-kit.obj")
                .display()
                .to_string()
        );
    }
}
