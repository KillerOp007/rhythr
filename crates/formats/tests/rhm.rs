//! Synthetic in-memory `.rhm` archives exercising the zip map path:
//! the normal case, missing/optional entries, and the attacker-controlled
//! uncompressed-size guard.

use std::io::{Cursor, Write};

use rhythia_formats::map::Map;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const MAP_JSON: &str = r#"{"OnlineId":42,"LegacyId":"demo","Title":"Demo","SongName":"Demo Song",
        "Duration":1000,"StarRating":2.5,"Notes":[{"Time":100,"X":1,"Y":1},
        {"Time":200,"X":0,"Y":2}]}"#;

fn build_rhm(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        zw.start_file(*name, opts).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

#[test]
fn full_rhm_loads_map_audio_and_cover() {
    let data = build_rhm(&[
        ("map", MAP_JSON.as_bytes()),
        ("audio", b"fake-mp3-bytes"),
        ("cover", b"fake-png-bytes"),
    ]);
    let map = Map::from_rhm(&data).unwrap();
    assert_eq!(map.notes.len(), 2);
    assert_eq!(map.meta.title, "Demo");
    assert_eq!(map.meta.online_id, Some(42));
    assert_eq!(map.audio.as_deref(), Some(&b"fake-mp3-bytes"[..]));
    assert_eq!(map.cover.as_deref(), Some(&b"fake-png-bytes"[..]));
}

#[test]
fn map_only_rhm_is_the_documented_normal_case() {
    // The game ships maps without audio/cover.
    let data = build_rhm(&[("map", MAP_JSON.as_bytes())]);
    let map = Map::from_rhm(&data).unwrap();
    assert_eq!(map.notes.len(), 2);
    assert!(map.audio.is_none());
    assert!(map.cover.is_none());
}

#[test]
fn rhm_without_map_entry_errors() {
    let data = build_rhm(&[("audio", b"only-audio")]);
    assert!(matches!(
        Map::from_rhm(&data),
        Err(rhythia_formats::Error::MissingMapEntry)
    ));
}

#[test]
fn corrupt_zip_errors_without_panic() {
    assert!(Map::from_rhm(b"not a zip at all").is_err());
}

#[test]
fn oversized_map_entry_is_rejected() {
    // A map JSON past the 64 MiB cap must error, not allocate unbounded.
    // (Real zip data, honestly large — the header-size fast path and the
    // read-limit path both lead to ArchiveEntryTooLarge.)
    let big = vec![b' '; (64 << 20) + 1];
    let mut json = Vec::from(&b"{\"Notes\":[]"[..]);
    json.extend_from_slice(&big); // trailing whitespace keeps it valid-ish and huge
    json.push(b'}');
    let data = build_rhm(&[("map", &json)]);
    assert!(matches!(
        Map::from_rhm(&data),
        Err(rhythia_formats::Error::ArchiveEntryTooLarge { .. })
    ));
}
