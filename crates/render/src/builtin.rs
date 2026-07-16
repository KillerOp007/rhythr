//! Resolving the game's built-in assets (colorsets, note/cursor/border
//! textures) that a config references by name, e.g.
//! `Textures/Game/colorsets/Arctic.txt` or `Textures/Game/notes/thin.png`.
//!
//! These assets are Capo Games' property. They are **not** shipped with this
//! tool — [`BuiltinAssets::load`] reads them from a directory the user points
//! at (their own Rhythia installation, or a locally-extracted copy). The
//! published renderer extracts them at runtime from the user's install; it
//! never bundles them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::SkinConfig;

/// A directory of the game's built-in assets plus the parsed colorset table.
pub struct BuiltinAssets {
    dir: PathBuf,
    colorsets: HashMap<String, Vec<[f32; 3]>>,
}

impl BuiltinAssets {
    /// Loads from a directory containing `builtin_colorsets.json` and the
    /// texture folders (`notes/`, `borders/`, `cursors/`, either directly or
    /// under `builtin_assets/`). Missing colorset file is tolerated (empty).
    pub fn load(dir: impl AsRef<Path>) -> BuiltinAssets {
        let dir = dir.as_ref().to_path_buf();
        let colorsets = std::fs::read(dir.join("builtin_colorsets.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.as_object().cloned())
            .map(|obj| {
                obj.into_iter()
                    .filter_map(|(name, val)| {
                        let cols: Vec<[f32; 3]> = val
                            .as_array()?
                            .iter()
                            .filter_map(|c| parse_hex(c.as_str()?))
                            .collect();
                        (!cols.is_empty()).then_some((name.to_ascii_lowercase(), cols))
                    })
                    .collect()
            })
            .unwrap_or_default();
        BuiltinAssets { dir, colorsets }
    }

    /// All mod icons from the extraction (name without extension → PNG).
    pub fn mod_icons(&self) -> Vec<(String, Vec<u8>)> {
        let dir = self.dir.join("builtin_assets").join("mods");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut icons = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".png") {
                if let Ok(bytes) = std::fs::read(e.path()) {
                    icons.push((stem.to_string(), bytes));
                }
            }
        }
        icons.sort();
        icons
    }

    /// Colours of a named built-in colorset (case-insensitive).
    pub fn colorset(&self, name: &str) -> Option<&[[f32; 3]]> {
        self.colorsets
            .get(&name.to_ascii_lowercase())
            .map(Vec::as_slice)
    }

    /// PNG bytes of a built-in texture, looked up by category (`notes`,
    /// `borders`, `cursors`) and file name.
    pub fn texture(&self, category: &str, file: &str) -> Option<Vec<u8>> {
        let candidates = [
            self.dir.join("builtin_assets").join(category).join(file),
            self.dir.join(category).join(file),
        ];
        candidates.iter().find_map(|p| std::fs::read(p).ok())
    }
}

/// `#rrggbb` → linear-ish RGB in [0,1].
fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let hex = s.trim().trim_start_matches('#');
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
}

/// The `<name>` of a built-in reference `Textures/Game/<category>/<name>`,
/// or None if the value is a custom/absolute path.
fn builtin_ref<'a>(value: &'a str, category: &str) -> Option<&'a str> {
    let needle = format!("Textures/Game/{category}/");
    let rest = value.rsplit_once(&needle).map(|(_, r)| r)?;
    Some(rest)
}

impl SkinConfig {
    /// Fills in built-in colorset colours and note/border/cursor textures
    /// that the config references by name but that a `.rhs` didn't bundle,
    /// using the game's assets from `assets`. Custom (already-bundled)
    /// values are left untouched.
    pub fn resolve_builtins(&mut self, assets: &BuiltinAssets) {
        if self.colorset.is_empty() {
            if let Some(name) = builtin_ref(&self.colorset_name, "colorsets") {
                let name = name.trim_end_matches(".txt");
                if let Some(cols) = assets.colorset(name) {
                    self.colorset = cols.to_vec();
                }
            }
        }
        if self.hud_font.is_none() {
            self.hud_font = assets.texture("fonts", "default.ttf");
        }
        if self.mod_icons.is_empty() {
            self.mod_icons = assets.mod_icons();
        }
        if self.note_texture.is_none() {
            if let Some(file) = builtin_ref(&self.note_skin_name, "notes") {
                self.note_texture = assets.texture("notes", file);
            }
        }
        if self.border_texture.is_none() {
            if let Some(file) = builtin_ref(&self.border_skin_name, "borders") {
                self.border_texture = assets.texture("borders", file);
            }
        }
        if self.cursor_texture.is_none() {
            if let Some(file) = builtin_ref(&self.cursor_skin_name, "cursors") {
                self.cursor_texture = assets.texture("cursors", file);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_ref_extracts_name() {
        assert_eq!(
            builtin_ref("Textures/Game/colorsets/Arctic.txt", "colorsets"),
            Some("Arctic.txt")
        );
        assert_eq!(
            builtin_ref("Textures/Game/notes/thin.png", "notes"),
            Some("thin.png")
        );
        assert_eq!(
            builtin_ref(r"C:\Users\x\AppData\...\my note.png", "notes"),
            None
        );
    }

    #[test]
    fn parses_hex() {
        assert_eq!(
            parse_hex("#a2e0ff"),
            Some([162.0 / 255.0, 224.0 / 255.0, 1.0])
        );
        assert_eq!(parse_hex("bad"), None);
    }
}
