//! Map loading: the game's cache JSON (`cache/maps/<sha256>.json`) and the
//! `.rhm` archive (a plain zip with entries "map" = the same JSON, plus
//! optional "audio" and "cover").
//!
//! Both share one JSON schema:
//! `{ Title, SongName, Mappers[], Duration(ms), Difficulty,
//!    CustomDifficultyName, StarRating, LegacyId, OnlineId,
//!    Notes: [{Time(ms), X, Y}], AudioFileName, ImagePath }`
//!
//! Notes live on the 3×3 grid, X/Y ∈ {0,1,2} — but off-grid "quantum"
//! floats exist in the Sound Space universe, so X/Y parse as f32.

use std::io::{Cursor, Read};
use std::path::Path;

use crate::{Error, Result};

/// Read limit for the "map" JSON inside a `.rhm`. The largest kit map
/// (3402 notes) is 95 KiB, so 64 MiB leaves room for absurdly dense maps
/// while keeping a forged zip header from driving a huge allocation.
const MAX_MAP_JSON_BYTES: u64 = 64 << 20;

/// Read limit for the embedded audio/cover. The kit's longest song is a
/// 14 MiB mp3; 512 MiB is far beyond any real map asset.
const MAX_MEDIA_BYTES: u64 = 512 << 20;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Note {
    pub time_ms: i64,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Default)]
pub struct MapMeta {
    pub online_id: Option<i64>,
    pub online_status: String,
    pub legacy_id: String,
    pub song_name: String,
    pub mappers: Vec<String>,
    pub title: String,
    pub duration_ms: i64,
    pub difficulty: i64,
    pub custom_difficulty_name: String,
    pub star_rating: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Map {
    pub meta: MapMeta,
    /// Sorted ascending by time.
    pub notes: Vec<Note>,
    pub audio: Option<Vec<u8>>,
    pub cover: Option<Vec<u8>>,
}

impl Map {
    /// Loads a map by file extension: `.rhm` (zip) or `.json` (cache format).
    pub fn from_path(path: impl AsRef<Path>) -> Result<Map> {
        let path = path.as_ref();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let data = std::fs::read(path)?;
        match ext.as_str() {
            "rhm" => Map::from_rhm(&data),
            "json" => Map::from_cache_json(&data),
            "sspm" => crate::sspm::parse(&data),
            other => Err(Error::UnsupportedExtension(other.to_string())),
        }
    }

    /// Parses the game's cache JSON (also the "map" entry inside `.rhm`).
    pub fn from_cache_json(data: &[u8]) -> Result<Map> {
        let doc: serde_json::Value = serde_json::from_slice(data)?;

        let str_field = |name: &'static str| -> String {
            doc.get(name)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };

        let meta = MapMeta {
            online_id: doc.get("OnlineId").and_then(|v| v.as_i64()),
            online_status: str_field("OnlineStatus"),
            legacy_id: str_field("LegacyId"),
            song_name: str_field("SongName"),
            mappers: doc
                .get("Mappers")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            title: str_field("Title"),
            duration_ms: doc.get("Duration").and_then(|v| v.as_i64()).unwrap_or(0),
            difficulty: doc.get("Difficulty").and_then(|v| v.as_i64()).unwrap_or(0),
            custom_difficulty_name: str_field("CustomDifficultyName"),
            star_rating: doc
                .get("StarRating")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
        };

        let raw_notes = doc
            .get("Notes")
            .and_then(|v| v.as_array())
            .ok_or(Error::BadMapField("Notes"))?;

        let mut notes = Vec::with_capacity(raw_notes.len());
        for n in raw_notes {
            // Time is an integer in every observed map; tolerate floats.
            let time = n
                .get("Time")
                .and_then(|v| v.as_f64())
                .ok_or(Error::BadMapField("Notes[].Time"))?;
            // X/Y are usually 0/1/2 but may be off-grid quantum floats.
            let x = n
                .get("X")
                .and_then(|v| v.as_f64())
                .ok_or(Error::BadMapField("Notes[].X"))?;
            let y = n
                .get("Y")
                .and_then(|v| v.as_f64())
                .ok_or(Error::BadMapField("Notes[].Y"))?;
            notes.push(Note {
                time_ms: time.round() as i64,
                x: x as f32,
                y: y as f32,
            });
        }
        notes.sort_by_key(|n| n.time_ms);

        Ok(Map {
            meta,
            notes,
            audio: None,
            cover: None,
        })
    }

    /// Parses a `.rhm` zip. Only the "map" entry is mandatory; the game
    /// ships maps without audio and/or cover.
    pub fn from_rhm(data: &[u8]) -> Result<Map> {
        let mut zip = zip::ZipArchive::new(Cursor::new(data))?;

        let read_entry = |zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
                          name: &str,
                          limit: u64|
         -> Result<Option<Vec<u8>>> {
            match zip.by_name(name) {
                Ok(entry) => {
                    // The declared uncompressed size comes straight from the
                    // zip header and is attacker-controlled: a tiny archive
                    // can claim terabytes. Never pre-reserve from it — take
                    // the entry through a hard read limit instead, so a zip
                    // bomb hits ArchiveEntryTooLarge rather than the OOM
                    // killer.
                    if entry.size() > limit {
                        return Err(Error::ArchiveEntryTooLarge {
                            entry: name.to_string(),
                            declared: entry.size(),
                            limit,
                        });
                    }
                    let mut buf = Vec::new();
                    // limit + 1 so a lying header that actually expands past
                    // the cap is caught by the length check below.
                    entry.take(limit + 1).read_to_end(&mut buf)?;
                    if buf.len() as u64 > limit {
                        return Err(Error::ArchiveEntryTooLarge {
                            entry: name.to_string(),
                            declared: buf.len() as u64,
                            limit,
                        });
                    }
                    Ok(Some(buf))
                }
                Err(zip::result::ZipError::FileNotFound) => Ok(None),
                Err(e) => Err(e.into()),
            }
        };

        let map_json =
            read_entry(&mut zip, "map", MAX_MAP_JSON_BYTES)?.ok_or(Error::MissingMapEntry)?;
        let audio = read_entry(&mut zip, "audio", MAX_MEDIA_BYTES)?;
        let cover = read_entry(&mut zip, "cover", MAX_MEDIA_BYTES)?;

        let mut map = Map::from_cache_json(&map_json)?;
        map.audio = audio;
        map.cover = cover;
        Ok(map)
    }
}
