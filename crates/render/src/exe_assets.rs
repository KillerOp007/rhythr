//! Extracting the game's built-in skin assets directly from the player's
//! own `rhythia.exe` (NativeAOT bundle) into the directory layout that
//! [`crate::BuiltinAssets::load`] reads.
//!
//! Legal note: nothing here ships game content. The extraction runs locally
//! against the user's own installation, at the user's explicit request —
//! the renderer never redistributes the assets.
//!
//! Format: the exe embeds .NET managed resources. Their index stores each
//! entry as a NativeFormat vertex containing the resource name (varint
//! length-prefixed UTF-8) followed by unsigned varints for offset and size
//! into a contiguous resource-data blob. Rather than hardcoding the blob's
//! position (it moves between game versions), the parser self-calibrates:
//! the PNG entries must all start with a PNG signature, which pins the one
//! base address that satisfies every entry simultaneously.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// All game resources live under this prefix; `route_name` decides which
/// of them the renderer actually wants.
const NAME_PREFIX: &[u8] = b"Rhythia.Resources.";
/// Sanity bound: no single embedded asset is anywhere near this large.
const MAX_ASSET_BYTES: u64 = 64 << 20;
/// The real exe has ~600 game entries; a crafted file must not mint more.
const MAX_ENTRIES: usize = 10_000;
/// Total extraction budget — the real asset set is ~2 MB.
const MAX_TOTAL_BYTES: u64 = 256 << 20;
/// Colorset lines become heap strings (~7x amplification) — cap the total.
const MAX_COLORSET_LINES: usize = 100_000;
/// Skin-texture categories under Textures.Game; anything else there is a
/// scan false positive.
const GAME_CATEGORIES: [&str; 6] = ["notes", "borders", "cursors", "ratings", "colorsets", ""];

/// One resource entry recovered from the index: name, blob-relative offset,
/// size.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

/// Decodes a .NET NativeFormat unsigned integer at `pos`. Returns the value
/// and the number of bytes consumed.
fn decode_unsigned(data: &[u8], pos: usize) -> Option<(u64, usize)> {
    let b0 = *data.get(pos)? as u64;
    if b0 & 0x01 == 0 {
        Some((b0 >> 1, 1))
    } else if b0 & 0x02 == 0 {
        let b1 = *data.get(pos + 1)? as u64;
        Some(((b0 >> 2) | (b1 << 6), 2))
    } else if b0 & 0x04 == 0 {
        let b1 = *data.get(pos + 1)? as u64;
        let b2 = *data.get(pos + 2)? as u64;
        Some(((b0 >> 3) | (b1 << 5) | (b2 << 13), 3))
    } else if b0 & 0x08 == 0 {
        let b1 = *data.get(pos + 1)? as u64;
        let b2 = *data.get(pos + 2)? as u64;
        let b3 = *data.get(pos + 3)? as u64;
        Some(((b0 >> 4) | (b1 << 4) | (b2 << 12) | (b3 << 20), 4))
    } else {
        let b = data.get(pos + 1..pos + 5)?;
        Some((u32::from_le_bytes(b.try_into().ok()?) as u64, 5))
    }
}

/// Scans the exe for resource-index entries: a varint string length, the
/// UTF-8 name starting with `Rhythia.Resources.Textures.Game.`, then two
/// varints (offset, size). The surrounding vertex structure is skipped —
/// the (name, offset, size) triple is all the extraction needs.
pub fn scan_entries(exe: &[u8]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut at = 0usize;
    while let Some(hit) = find(exe, NAME_PREFIX, at) {
        at = hit + 1;
        // The name string is length-prefixed; the varint sits 1-2 bytes
        // before the text for realistic name lengths (< 16384).
        let entry = [1usize, 2].iter().find_map(|&back| {
            let vpos = hit.checked_sub(back)?;
            let (len, consumed) = decode_unsigned(exe, vpos)?;
            if consumed != back || len < NAME_PREFIX.len() as u64 || len > 512 {
                return None;
            }
            let name_bytes = exe.get(hit..hit + len as usize)?;
            let name = std::str::from_utf8(name_bytes).ok()?;
            let mut pos = hit + len as usize;
            let (offset, used) = decode_unsigned(exe, pos)?;
            pos += used;
            let (size, _) = decode_unsigned(exe, pos)?;
            (size > 0 && size < MAX_ASSET_BYTES).then(|| Entry {
                name: name.to_string(),
                offset,
                size,
            })
        });
        if let Some(e) = entry {
            entries.push(e);
            if entries.len() >= MAX_ENTRIES {
                break;
            }
        }
    }
    entries
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Finds the one base address such that every PNG-named entry's bytes start
/// with the PNG signature. Self-calibrating: game updates move the blob,
/// but the constraint set stays unambiguous (hundreds of PNGs).
pub fn locate_blob_base(exe: &[u8], entries: &[Entry]) -> Option<u64> {
    let pngs: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.name.ends_with(".png"))
        .collect();
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    let sig_at = |base: u64, off: u64| -> bool {
        base.checked_add(off)
            .and_then(|s| usize::try_from(s).ok())
            .and_then(|s| exe.get(s..s.checked_add(SIG.len())?))
            == Some(SIG)
    };
    // A garbage first entry must not poison calibration: anchor on several
    // different probes and demand that (almost) every PNG entry agrees —
    // scan false positives may contribute a few bad offsets.
    for probe in pngs.iter().take(8) {
        let mut at = 0usize;
        while let Some(hit) = find(exe, SIG, at) {
            at = hit + 1;
            let Some(base) = (hit as u64).checked_sub(probe.offset) else {
                continue;
            };
            let hits = pngs.iter().filter(|e| sig_at(base, e.offset)).count();
            if hits * 10 >= pngs.len() * 9 {
                return Some(base);
            }
        }
    }
    None
}

/// Maps a resource name to its cache location `(category, file)` under
/// `builtin_assets/`, or None for resources the renderer has no use for
/// (menu art, story music, database schema, …).
///
/// Skin textures keep their original layout; the rest lands in dedicated
/// folders: game shaders in `shaders/` (vignette, fade, instanced_tint …),
/// the hit/miss sounds in `sounds/`, mod icons in `mods/`, and the two UI
/// fonts in `fonts/`. Colorset names may themselves contain dots; a
/// Textures.Game rest with a single dot ("kfc.png") is a top-level file.
fn route_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix(std::str::from_utf8(NAME_PREFIX).ok()?)?;
    if let Some(game) = rest.strip_prefix("Textures.Game.") {
        let (category, file) = game.split_once('.')?;
        return if file.contains('.') {
            (!file.is_empty()).then_some((category, file))
        } else {
            // "kfc" + "png": no category segment — the whole rest is the name.
            Some(("", game))
        };
    }
    if let Some(f) = rest.strip_prefix("Shaders.") {
        return Some(("shaders", f));
    }
    if let Some(f) = rest.strip_prefix("Sounds.default_hits.") {
        return Some(("sounds", f));
    }
    if let Some(f) = rest.strip_prefix("Sounds.default_misses.") {
        return Some(("sounds", f));
    }
    if rest == "Sounds.hit.wav" {
        return Some(("sounds", "hit.wav"));
    }
    if let Some(f) = rest.strip_prefix("Textures.Menu.mods.") {
        return Some(("mods", f));
    }
    if rest == "Fonts.default.ttf" || rest == "Fonts.default2.ttf" {
        return Some(("fonts", rest.strip_prefix("Fonts.").unwrap()));
    }
    None
}

fn sane_component(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(['/', '\\'])
        && !s.contains("..")
        && s.chars().all(|c| !c.is_control())
}

/// Extracts all `Textures/Game` assets from `exe_path` into `out_dir` in
/// the layout [`crate::BuiltinAssets::load`] expects: texture files under
/// `builtin_assets/<category>/`, colorsets merged into
/// `builtin_colorsets.json`. Returns the number of assets written.
pub fn extract_to_dir(exe_path: &Path, out_dir: &Path) -> Result<usize, String> {
    let exe = std::fs::read(exe_path).map_err(|e| format!("reading {}: {e}", exe_path.display()))?;
    let entries = scan_entries(&exe);
    if entries.is_empty() {
        return Err("no game resources found in this file — is it rhythia.exe?".into());
    }
    let base = locate_blob_base(&exe, &entries)
        .ok_or("could not locate the resource data in this exe (unsupported game version?)")?;

    let mut colorsets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut written = 0usize;
    let mut seen: std::collections::BTreeSet<(String, String)> = Default::default();
    let mut budget: u64 = 0;
    for e in &entries {
        let Some((category, file)) = route_name(&e.name) else {
            continue;
        };
        // Empty category = top-level file directly under builtin_assets.
        if (!category.is_empty() && !sane_component(category)) || !sane_component(file) {
            continue;
        }
        // Textures.Game entries outside the real categories are artefacts.
        if e.name.contains(".Textures.Game.") && !GAME_CATEGORIES.contains(&category) {
            continue;
        }
        // Duplicate names never occur in a genuine resource index; a later
        // (garbage) duplicate must not overwrite a good file.
        if !seen.insert((category.to_string(), file.to_string())) {
            continue;
        }
        budget += e.size;
        if budget > MAX_TOTAL_BYTES {
            return Err("resource index claims implausibly large assets — refusing".into());
        }
        let bytes = base
            .checked_add(e.offset)
            .and_then(|s| usize::try_from(s).ok())
            .and_then(|s| exe.get(s..s.checked_add(usize::try_from(e.size).ok()?)?));
        let Some(bytes) = bytes else {
            continue;
        };
        // A PNG-named entry whose bytes are not a PNG is a false positive.
        if file.ends_with(".png") && !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            continue;
        }
        if category == "colorsets" {
            let Some(set_name) = file.strip_suffix(".txt") else {
                continue;
            };
            let text = String::from_utf8_lossy(bytes);
            let colors: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|l| l.starts_with('#') && (l.len() == 7 || l.len() == 9))
                .map(str::to_string)
                .collect();
            if !colors.is_empty() {
                let total: usize = colorsets.values().map(Vec::len).sum();
                if total + colors.len() > MAX_COLORSET_LINES {
                    return Err("implausibly many colorset entries — refusing".into());
                }
                colorsets.insert(set_name.to_string(), colors);
                written += 1;
            }
        } else {
            let dir = out_dir.join("builtin_assets").join(category);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let mut f = std::fs::File::create(dir.join(file)).map_err(|e| e.to_string())?;
            f.write_all(bytes).map_err(|e| e.to_string())?;
            written += 1;
        }
    }
    if !colorsets.is_empty() {
        let json = serde_json::to_string_pretty(&colorsets).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
        std::fs::write(out_dir.join("builtin_colorsets.json"), json).map_err(|e| e.to_string())?;
    }
    if written == 0 {
        return Err("resource index found, but no usable skin assets in it".into());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_unsigned(v: u64) -> Vec<u8> {
        // Enough for tests: 1- and 2-byte encodings.
        if v < 0x80 {
            vec![(v << 1) as u8]
        } else {
            assert!(v < 0x4000);
            vec![((v << 2) | 0x01) as u8, (v >> 6) as u8]
        }
    }

    #[test]
    fn varint_roundtrip() {
        for v in [0u64, 1, 63, 64, 127, 128, 300, 16000] {
            let enc = encode_unsigned(v);
            let (dec, used) = decode_unsigned(&enc, 0).unwrap();
            assert_eq!((dec, used), (v, enc.len()), "value {v}");
        }
        // 3/4/5-byte forms decode too.
        assert_eq!(decode_unsigned(&[0x03, 0x01, 0x01], 0).unwrap().0, (0x03u64 >> 3) | (1 << 5) | (1 << 13));
        assert_eq!(
            decode_unsigned(&[0x1F, 0xEF, 0xBE, 0xAD, 0xDE], 0).unwrap(),
            (0xDEADBEEF, 5)
        );
    }

    /// End-to-end against a synthetic exe image: index entries + PNG blob.
    #[test]
    fn synthetic_exe_extracts() {
        let png: Vec<u8> = {
            let mut b = Vec::new();
            let mut enc = png::Encoder::new(std::io::Cursor::new(&mut b), 1, 1);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&[9, 9, 9, 255]).unwrap();
            b
        };
        let colorset = b"#ffffff\n#a2e0ff\n#a2e0ff\n".to_vec();

        // Blob: [png][colorset], preceded by index entries and junk.
        let mut blob = Vec::new();
        let png_off = blob.len() as u64;
        blob.extend_from_slice(&png);
        let cs_off = blob.len() as u64;
        blob.extend_from_slice(&colorset);

        let mut exe = vec![0xAAu8; 64]; // leading junk
        for (name, off, size) in [
            ("Rhythia.Resources.Textures.Game.notes.square 3.png", png_off, png.len() as u64),
            ("Rhythia.Resources.Textures.Game.colorsets.Arctic.txt", cs_off, colorset.len() as u64),
        ] {
            exe.extend_from_slice(&encode_unsigned(name.len() as u64));
            exe.extend_from_slice(name.as_bytes());
            exe.extend_from_slice(&encode_unsigned(off));
            exe.extend_from_slice(&encode_unsigned(size));
            exe.push(0xBB); // vertex separator junk
        }
        exe.extend_from_slice(&[0xCC; 32]);
        exe.extend_from_slice(&blob);

        let entries = scan_entries(&exe);
        assert_eq!(entries.len(), 2, "{entries:?}");
        let base = locate_blob_base(&exe, &entries).expect("base found");

        let tmp = tempfile::tempdir().unwrap();
        let n = extract_to_dir_from_bytes_for_test(&exe, base, &entries, tmp.path());
        assert_eq!(n, 2);
        let tex = std::fs::read(tmp.path().join("builtin_assets/notes/square 3.png")).unwrap();
        assert_eq!(tex, png);
        let json = std::fs::read_to_string(tmp.path().join("builtin_colorsets.json")).unwrap();
        assert!(json.contains("Arctic") && json.contains("#a2e0ff"));
    }

    /// Test-only variant of extract_to_dir that skips the file read.
    fn extract_to_dir_from_bytes_for_test(
        exe: &[u8],
        base: u64,
        entries: &[Entry],
        out_dir: &Path,
    ) -> usize {
        let mut colorsets: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut written = 0usize;
        for e in entries {
            let Some((category, file)) = route_name(&e.name) else {
                continue;
            };
            let start = (base + e.offset) as usize;
            let bytes = &exe[start..start + e.size as usize];
            if category == "colorsets" {
                let set_name = file.strip_suffix(".txt").unwrap();
                colorsets.insert(
                    set_name.into(),
                    String::from_utf8_lossy(bytes).lines().map(|l| l.trim().to_string()).collect(),
                );
                written += 1;
            } else {
                let dir = out_dir.join("builtin_assets").join(category);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(file), bytes).unwrap();
                written += 1;
            }
        }
        std::fs::write(
            out_dir.join("builtin_colorsets.json"),
            serde_json::to_string(&colorsets).unwrap(),
        )
        .unwrap();
        written
    }
}
