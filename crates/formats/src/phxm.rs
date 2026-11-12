//! Maps of the free community client (the Godot 4 rewrite): a folder in
//! the client's `maps` directory, or the same files zipped as a `.phxm`
//! export.
//!
//! ```text
//! metadata.json   ID, Artist, Title, Mappers[], Difficulty,
//!                 DifficultyName, Length, Rating, AudioExt, ...
//! objects.phxmo   the notes
//! audio.<ext>     optional
//! cover.png       optional
//! ```
//!
//! `objects.phxmo`, little-endian: a u32 (the number of object kinds the
//! file has room for), a u32 note count, then per note a u32 time in ms
//! and a u8 that says how the position follows: 0 for two u8 (column and
//! row on the 3x3 grid), anything else for two f32 (an off-grid
//! position). The counts of the other object kinds follow; the client
//! writes none of them yet and nothing here reads them.
//!
//! The client's playfield has its origin in the centre with y pointing
//! up, and it stores grid rows that way: row 0 is the BOTTOM row. A
//! [`Note`] counts rows from the top like every other map format here,
//! so rows are turned over on the way in.
//!
//! A `.phxr` replay names its notes by their index in this file, in this
//! order. Reading the client's own copy of a map is therefore the exact
//! way to pair one with its replay: a `.sspm` of the same map can list
//! two notes that share a timestamp the other way round.
//!
//! The layout is taken from the client's public source (`Rhythia/Client`,
//! `scripts/map/MapParser.cs`), which is AGPL-3.0; none of its code is.

use std::io::{Cursor, Read};
use std::path::Path;

use crate::map::{Map, MapMeta, Note, MAX_MAP_JSON_BYTES, MAX_MEDIA_BYTES};
use crate::reader::Reader;
use crate::{Error, Result};

const METADATA: &str = "metadata.json";
const OBJECTS: &str = "objects.phxmo";
const COVER: &str = "cover.png";

/// The names the client gives its numbered difficulties. A map whose
/// difficulty name is one of these has no custom name.
const STOCK_DIFFICULTY_NAMES: [&str; 6] = ["N/A", "Easy", "Medium", "Hard", "Insane", "Illogical"];

/// Smallest note on the wire: time, the position kind and two u8.
const MIN_NOTE_BYTES: usize = 4 + 1 + 2;

fn bad(msg: impl Into<String>) -> Error {
    Error::BadPhxm(msg.into())
}

/// Parses `objects.phxmo` into notes, sorted by time. Notes that share a
/// timestamp keep the order the file lists them in.
pub fn parse_objects(data: &[u8]) -> Result<Vec<Note>> {
    let mut r = Reader::new(data);
    r.u32()?; // object kinds
    let declared = r.u32()? as usize;
    // A forged count must not drive the allocation below.
    if declared > r.remaining() / MIN_NOTE_BYTES {
        return Err(bad(format!(
            "note count {declared} is more than the file holds"
        )));
    }
    let mut notes = Vec::with_capacity(declared);
    for _ in 0..declared {
        let time_ms = i64::from(r.u32()?);
        // Client space: centre at the origin, y up.
        let (cx, cy) = if r.u8()? == 0 {
            (f32::from(r.u8()?) - 1.0, f32::from(r.u8()?) - 1.0)
        } else {
            (r.f32()?, r.f32()?)
        };
        if !cx.is_finite() || !cy.is_finite() {
            return Err(bad("a note position is not a number"));
        }
        notes.push(Note {
            time_ms,
            x: cx + 1.0,
            y: 1.0 - cy,
        });
    }
    notes.sort_by_key(|n| n.time_ms);
    Ok(notes)
}

/// What `metadata.json` says, as far as a render uses it. Every field is
/// optional: the notes are what makes a map, and a missing or odd label
/// is no reason to refuse them.
fn meta_from(json: Option<&[u8]>, fallback_id: &str, notes: &[Note]) -> (MapMeta, Option<String>) {
    let doc: serde_json::Value = json
        .and_then(|bytes| serde_json::from_slice(bytes).ok())
        .unwrap_or(serde_json::Value::Null);
    let text = |name: &str| -> String {
        doc.get(name)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let (artist, title) = (text("Artist"), text("Title"));
    let name = match (artist.is_empty(), title.is_empty()) {
        (false, false) => format!("{artist} - {title}"),
        (true, false) => title,
        (_, true) => artist,
    };
    let id = text("ID");
    let difficulty_name = text("DifficultyName");
    let mut meta = MapMeta {
        legacy_id: if id.is_empty() {
            fallback_id.to_string()
        } else {
            id
        },
        song_name: name.clone(),
        title: name,
        mappers: doc
            .get("Mappers")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        duration_ms: doc
            .get("Length")
            .and_then(|v| v.as_f64())
            .filter(|l| l.is_finite() && *l > 0.0)
            .map_or_else(|| notes.last().map_or(0, |n| n.time_ms), |l| l as i64),
        difficulty: doc.get("Difficulty").and_then(|v| v.as_i64()).unwrap_or(0),
        custom_difficulty_name: if STOCK_DIFFICULTY_NAMES.contains(&difficulty_name.as_str()) {
            String::new()
        } else {
            difficulty_name
        },
        star_rating: doc.get("Rating").and_then(|v| v.as_f64()).unwrap_or(0.0),
        ..MapMeta::default()
    };
    meta.normalize();
    let audio_ext = Some(text("AudioExt")).filter(|e| !e.is_empty());
    (meta, audio_ext)
}

/// The audio file names worth trying, the one the metadata names first.
fn audio_names(audio_ext: Option<&str>) -> Vec<String> {
    let mut exts: Vec<&str> = audio_ext.into_iter().collect();
    for ext in ["mp3", "ogg"] {
        if !exts.contains(&ext) {
            exts.push(ext);
        }
    }
    exts.iter()
        // An extension is a plain word; anything else is not a file name
        // this module will go looking for.
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| format!("audio.{e}"))
        .collect()
}

/// Loads a map from the client's folder layout. `dir` is the map's own
/// folder (the one holding `objects.phxmo`).
pub fn from_folder(dir: &Path) -> Result<Map> {
    let read = |name: &str, limit: u64| -> Result<Option<Vec<u8>>> {
        let path = dir.join(name);
        let size = match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => m.len(),
            _ => return Ok(None),
        };
        if size > limit {
            return Err(bad(format!(
                "{name} is {size} bytes, over the {limit}-byte limit"
            )));
        }
        Ok(Some(std::fs::read(path)?))
    };
    let objects = read(OBJECTS, MAX_MAP_JSON_BYTES)?
        .ok_or_else(|| bad(format!("the folder holds no {OBJECTS}")))?;
    let notes = parse_objects(&objects)?;
    let folder_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let metadata = read(METADATA, MAX_MAP_JSON_BYTES)?;
    let (meta, audio_ext) = meta_from(metadata.as_deref(), folder_name, &notes);
    let mut audio = None;
    for name in audio_names(audio_ext.as_deref()) {
        audio = read(&name, MAX_MEDIA_BYTES)?;
        if audio.is_some() {
            break;
        }
    }
    Ok(Map {
        meta,
        notes,
        audio,
        cover: read(COVER, MAX_MEDIA_BYTES)?,
    })
}

/// Loads a `.phxm` export: the folder's files in a zip.
pub fn from_zip(data: &[u8]) -> Result<Map> {
    let mut zip = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| bad(format!("not a readable archive: {e}")))?;
    let mut read = |name: &str, limit: u64| -> Result<Option<Vec<u8>>> {
        let entry = match zip.by_name(name) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(e) => return Err(bad(format!("{name}: {e}"))),
        };
        // The declared size is the archive's own claim; the read limit is
        // what actually holds (see `Map::from_rhm`).
        let too_large = || bad(format!("{name} is over the {limit}-byte limit"));
        if entry.size() > limit {
            return Err(too_large());
        }
        let mut buf = Vec::new();
        entry.take(limit + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 > limit {
            return Err(too_large());
        }
        Ok(Some(buf))
    };
    let objects = read(OBJECTS, MAX_MAP_JSON_BYTES)?
        .ok_or_else(|| bad(format!("the archive holds no {OBJECTS}")))?;
    let notes = parse_objects(&objects)?;
    let metadata = read(METADATA, MAX_MAP_JSON_BYTES)?;
    let (meta, audio_ext) = meta_from(metadata.as_deref(), "", &notes);
    let mut audio = None;
    for name in audio_names(audio_ext.as_deref()) {
        audio = read(&name, MAX_MEDIA_BYTES)?;
        if audio.is_some() {
            break;
        }
    }
    let cover = read(COVER, MAX_MEDIA_BYTES)?;
    Ok(Map {
        meta,
        notes,
        audio,
        cover,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// `objects.phxmo` bytes for notes given in CLIENT space (centre
    /// origin, y up), laid out the way the client's exporter does it.
    fn objects(notes: &[(u32, f32, f32)]) -> Vec<u8> {
        let mut b = 12u32.to_le_bytes().to_vec();
        b.extend((notes.len() as u32).to_le_bytes());
        for &(ms, x, y) in notes {
            b.extend(ms.to_le_bytes());
            let on_grid = x.fract() == 0.0 && y.fract() == 0.0 && x.abs() <= 1.0 && y.abs() <= 1.0;
            if on_grid {
                b.extend([0, (x + 1.0) as u8, (y + 1.0) as u8]);
            } else {
                b.push(1);
                b.extend(x.to_le_bytes());
                b.extend(y.to_le_bytes());
            }
        }
        b.extend([0u8; 11 * 4]);
        b
    }

    const METADATA_JSON: &str = r#"{
        "ID": "some_mapper_-_some_song",
        "Artist": "Some Artist",
        "Title": "Some Song",
        "Rating": 0,
        "Mappers": ["one", "two"],
        "Difficulty": 3,
        "DifficultyName": "Hard",
        "Length": 93000,
        "HasAudio": true,
        "HasCover": true,
        "HasVideo": false,
        "AudioExt": "ogg"
    }"#;

    fn zipped(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in files {
            w.start_file(*name, opts).unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn grid_notes_come_out_in_map_space_with_rows_counted_from_the_top() {
        // Client space: (-1, 1) is top left, (1, -1) bottom right.
        let notes = parse_objects(&objects(&[
            (1000, -1.0, 1.0),
            (2000, 0.0, 0.0),
            (3000, 1.0, -1.0),
        ]))
        .unwrap();
        let got: Vec<(i64, f32, f32)> = notes.iter().map(|n| (n.time_ms, n.x, n.y)).collect();
        assert_eq!(got, [(1000, 0.0, 0.0), (2000, 1.0, 1.0), (3000, 2.0, 2.0)]);
    }

    #[test]
    fn off_grid_notes_keep_their_exact_position() {
        let notes = parse_objects(&objects(&[(500, 0.25, -0.5), (600, 1.5, 0.0)])).unwrap();
        assert_eq!((notes[0].x, notes[0].y), (1.25, 1.5));
        assert_eq!((notes[1].x, notes[1].y), (2.5, 1.0));
    }

    #[test]
    fn notes_sharing_a_timestamp_keep_the_order_of_the_file() {
        // Replay results go by this order.
        let notes = parse_objects(&objects(&[
            (1000, 1.0, 0.0),
            (1000, -1.0, 0.0),
            (900, 0.0, 0.0),
        ]))
        .unwrap();
        let got: Vec<(i64, f32)> = notes.iter().map(|n| (n.time_ms, n.x)).collect();
        assert_eq!(got, [(900, 1.0), (1000, 2.0), (1000, 0.0)]);
    }

    #[test]
    fn a_forged_note_count_and_a_cut_file_are_refused() {
        let mut data = objects(&[(1000, 0.0, 0.0)]);
        data[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = parse_objects(&data).unwrap_err().to_string();
        assert!(err.contains("note count"), "{err}");
        assert!(parse_objects(&objects(&[(1000, 0.5, 0.5)])[..12]).is_err());
        let mut nan = objects(&[(1000, 0.5, 0.5)]);
        nan[13..17].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(parse_objects(&nan).is_err());
    }

    #[test]
    fn an_export_loads_with_its_labels_audio_and_cover() {
        let data = zipped(&[
            (METADATA, METADATA_JSON.as_bytes()),
            (OBJECTS, &objects(&[(1000, 0.0, 0.0), (92000, 1.0, 1.0)])),
            ("audio.ogg", b"OggS-audio"),
            (COVER, b"png-cover"),
        ]);
        let map = from_zip(&data).unwrap();
        assert_eq!(map.notes.len(), 2);
        assert_eq!(map.meta.legacy_id, "some_mapper_-_some_song");
        assert_eq!(map.meta.song_name, "Some Artist - Some Song");
        assert_eq!(map.meta.mappers, ["one", "two"]);
        assert_eq!((map.meta.difficulty, map.meta.duration_ms), (3, 93000));
        assert_eq!(map.meta.custom_difficulty_name, "", "Hard is a stock name");
        assert_eq!(map.audio.as_deref(), Some(&b"OggS-audio"[..]));
        assert_eq!(map.cover.as_deref(), Some(&b"png-cover"[..]));
    }

    #[test]
    fn notes_alone_are_enough_for_a_map() {
        let data = zipped(&[(OBJECTS, &objects(&[(1000, 0.0, 0.0), (4000, 0.0, 0.0)]))]);
        let map = from_zip(&data).unwrap();
        assert_eq!(map.notes.len(), 2);
        assert_eq!(map.meta.duration_ms, 4000, "falls back to the last note");
        assert!(map.audio.is_none() && map.cover.is_none());

        let odd = zipped(&[
            (METADATA, b"not json at all"),
            (OBJECTS, &objects(&[(1000, 0.0, 0.0)])),
        ]);
        assert_eq!(from_zip(&odd).unwrap().notes.len(), 1);
    }

    #[test]
    fn an_archive_without_notes_says_what_is_missing() {
        let err = from_zip(&zipped(&[(METADATA, METADATA_JSON.as_bytes())]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("objects.phxmo"), "{err}");
        assert!(from_zip(b"not a zip").is_err());
    }

    #[test]
    fn a_custom_difficulty_name_is_kept() {
        let json = METADATA_JSON.replace("\"Hard\"", "\"Finger Breaker\"");
        let data = zipped(&[
            (METADATA, json.as_bytes()),
            (OBJECTS, &objects(&[(1000, 0.0, 0.0)])),
        ]);
        assert_eq!(
            from_zip(&data).unwrap().meta.custom_difficulty_name,
            "Finger Breaker"
        );
    }

    #[test]
    fn a_map_folder_loads_by_folder_by_objects_file_and_through_map_from_path() {
        let dir = std::env::temp_dir().join(format!("rhythr-phxm-test-{}", std::process::id()));
        let folder = dir.join("some_mapper_-_some_song");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join(OBJECTS),
            objects(&[(1000, 0.0, 0.0), (2000, 1.0, 1.0)]),
        )
        .unwrap();
        std::fs::write(folder.join("audio.mp3"), b"ID3-audio").unwrap();

        // No metadata.json: the folder name stands in for the id, and the
        // audio is found without being told its extension.
        let map = from_folder(&folder).unwrap();
        assert_eq!(map.notes.len(), 2);
        assert_eq!(map.meta.legacy_id, "some_mapper_-_some_song");
        assert_eq!(map.audio.as_deref(), Some(&b"ID3-audio"[..]));

        assert_eq!(Map::from_path(&folder).unwrap().notes.len(), 2);
        assert_eq!(Map::from_path(folder.join(OBJECTS)).unwrap().notes.len(), 2);
        assert_eq!(
            Map::from_path(folder.join(OBJECTS)).unwrap().meta.legacy_id,
            "some_mapper_-_some_song"
        );

        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let err = Map::from_path(&empty).unwrap_err().to_string();
        assert!(err.contains("objects.phxmo"), "{err}");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
