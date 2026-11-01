//! Where the game keeps its data (`CapoRhythia`): the roaming application
//! data folder on Windows, the Proton prefix of the Steam app under Linux,
//! or whatever `RHYTHR_GAME_DATA_DIR` names. Only the search lives here;
//! what gets written there is the kernel's `kit::game`.

use std::path::{Path, PathBuf};

/// The game's Steam app id; its Proton prefix sits under
/// `steamapps/compatdata/<id>` of whichever library holds it.
pub const STEAM_APP_ID: &str = "2250500";
/// Names the data folder outright (tests, unusual setups); it wins when it
/// is a game folder.
pub const ENV_OVERRIDE: &str = "RHYTHR_GAME_DATA_DIR";
const GAME_DIR: &str = "CapoRhythia";

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

/// Every Steam library root: the registry or default install, the known
/// Linux locations, plus each `path` in the roots' `libraryfolders.vdf`,
/// in that order and without repeats.
pub fn steam_library_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        // The registry knows custom install locations Steam was moved to.
        if let Some(p) = windows_steam_path() {
            roots.push(p);
        }
        for base in ["C:\\Program Files (x86)\\Steam", "C:\\Program Files\\Steam"] {
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
    for lib in libs {
        if !roots.contains(&lib) {
            roots.push(lib);
        }
    }
    roots
}

/// The folders to try, in order: the override, the roaming folder, the
/// Proton prefix in every Steam library, the XDG folders.
pub fn candidates(
    env_override: Option<PathBuf>,
    roaming: Option<PathBuf>,
    steam_roots: &[PathBuf],
    xdg: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    out.extend(env_override);
    out.extend(roaming.map(|r| r.join(GAME_DIR)));
    for root in steam_roots {
        out.push(
            root.join("steamapps")
                .join("compatdata")
                .join(STEAM_APP_ID)
                .join("pfx")
                .join("drive_c")
                .join("users")
                .join("steamuser")
                .join("AppData")
                .join("Roaming")
                .join(GAME_DIR),
        );
    }
    out.extend(xdg.iter().map(|x| x.join(GAME_DIR)));
    out
}

/// A folder is the game's when it holds `framework.json` or a `skins`
/// folder; an empty folder with the right name is not.
pub fn is_game_data_dir(p: &Path) -> bool {
    p.join("framework.json").is_file() || p.join("skins").is_dir()
}

/// The first candidate that is a game folder, or every candidate tried,
/// for the message that says where the kit looked.
pub fn pick(tried: Vec<PathBuf>) -> Result<PathBuf, Vec<PathBuf>> {
    match tried.iter().find(|p| is_game_data_dir(p)) {
        Some(found) => Ok(found.clone()),
        None => Err(tried),
    }
}

/// The status sentence when no candidate is the game's folder: every
/// folder tried, in the order tried.
pub fn not_found_text(tried: &[PathBuf]) -> String {
    let list: Vec<String> = tried.iter().map(|p| p.display().to_string()).collect();
    format!("Game folder not found. Looked in: {}.", list.join("; "))
}

/// The game folder on this machine: the candidates in their order, the
/// first that counts.
pub fn game_data_dir() -> Result<PathBuf, Vec<PathBuf>> {
    let env_override = std::env::var_os(ENV_OVERRIDE)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let (roaming, steam, xdg) = if cfg!(windows) {
        // Windows: the roaming folder; no Proton prefix to look for.
        (dirs::config_dir(), Vec::new(), Vec::new())
    } else {
        let xdg: Vec<PathBuf> = [dirs::config_dir(), dirs::data_dir()]
            .into_iter()
            .flatten()
            .collect();
        (None, steam_library_roots(), xdg)
    };
    pick(candidates(env_override, roaming, &steam, &xdg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn candidates_cover_every_steam_library_in_order() {
        let c = candidates(
            None,
            Some(PathBuf::from("/roam")),
            &[PathBuf::from("/lib1"), PathBuf::from("/lib2")],
            &[PathBuf::from("/xdg")],
        );
        assert_eq!(c[0], PathBuf::from("/roam/CapoRhythia"));
        assert!(
            c[1].ends_with(
                "lib1/steamapps/compatdata/2250500/pfx/drive_c/users/steamuser/AppData/Roaming/CapoRhythia"
            ),
            "{:?}",
            c[1]
        );
        assert!(c[2].starts_with("/lib2"));
        assert_eq!(c[3], PathBuf::from("/xdg/CapoRhythia"));
        assert_eq!(c.len(), 4);
        assert_eq!(
            candidates(Some(PathBuf::from("/o")), None, &[], &[])[0],
            PathBuf::from("/o")
        );
    }

    #[test]
    fn a_folder_counts_with_framework_json_or_a_skins_folder() {
        let t = tempfile::tempdir().unwrap();
        assert!(!is_game_data_dir(t.path()));
        std::fs::write(t.path().join("framework.json"), "{}").unwrap();
        assert!(is_game_data_dir(t.path()));
        let t2 = tempfile::tempdir().unwrap();
        std::fs::create_dir(t2.path().join("skins")).unwrap();
        assert!(is_game_data_dir(t2.path()));
    }

    #[test]
    fn the_first_existing_game_folder_wins() {
        // A missing folder and an empty one are passed over; the first
        // folder that is the game's wins, whatever comes after it.
        let empty = tempfile::tempdir().unwrap();
        let game = tempfile::tempdir().unwrap();
        std::fs::create_dir(game.path().join("skins")).unwrap();
        let later = tempfile::tempdir().unwrap();
        std::fs::write(later.path().join("framework.json"), "{}").unwrap();
        let picked = pick(vec![
            empty.path().join("nowhere"),
            empty.path().to_path_buf(),
            game.path().to_path_buf(),
            later.path().to_path_buf(),
        ])
        .unwrap();
        assert_eq!(picked, game.path());
    }

    #[test]
    fn the_not_found_sentence_lists_every_candidate() {
        let t = tempfile::tempdir().unwrap();
        let tried = vec![t.path().join("x"), t.path().join("y")];
        let err = pick(tried.clone()).unwrap_err();
        assert_eq!(err, tried);
        let text = not_found_text(&err);
        assert!(
            text.starts_with("Game folder not found. Looked in: "),
            "{text}"
        );
        assert!(
            text.contains(&format!("{}; {}", tried[0].display(), tried[1].display())),
            "{text}"
        );
        assert!(text.ends_with('.'), "{text}");
    }
}
