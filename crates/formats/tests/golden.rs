//! Golden tests against the real replays in testdata/ and their manifest
//! (ground-truth values exported from the game's own database).

use rhythia_formats::{map::Map, rhr::Replay};

fn testdata(path: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(path)
}

fn manifest() -> Vec<serde_json::Value> {
    let raw = std::fs::read(testdata("testdata_manifest.json")).expect("manifest present");
    serde_json::from_slice(&raw).expect("manifest parses")
}

#[test]
fn all_manifest_replays_parse_exactly() {
    for entry in manifest() {
        let file = entry["file"].as_str().unwrap();
        let data = std::fs::read(testdata(file)).expect("replay file present");
        let r = Replay::parse(&data).unwrap_or_else(|e| panic!("{file}: {e}"));

        assert_eq!(
            data.len() as u64,
            entry["blob_bytes"].as_u64().unwrap(),
            "{file} size"
        );
        assert_eq!(r.trailing_bytes, 0, "{file} trailing bytes");
        assert_eq!(
            i64::from(r.version),
            entry["rhr_version"].as_i64().unwrap(),
            "{file} version"
        );
        assert_eq!(
            r.player_name,
            entry["player"].as_str().unwrap(),
            "{file} player"
        );
        assert_eq!(
            i64::from(r.map_id),
            entry["map_online_id"].as_i64().unwrap(),
            "{file} map id"
        );
        assert_eq!(
            r.legacy_map_id,
            entry["legacy_map_id"].as_str().unwrap(),
            "{file} legacy id"
        );
        assert_eq!(r.mode, entry["mode"].as_str().unwrap(), "{file} mode");
        assert!(
            (f64::from(r.accuracy_pct) - entry["accuracy"].as_f64().unwrap()).abs() < 1e-4,
            "{file} accuracy"
        );
        assert_eq!(
            i64::from(r.hits),
            entry["hits"].as_i64().unwrap(),
            "{file} hits"
        );
        assert_eq!(
            i64::from(r.misses),
            entry["misses"].as_i64().unwrap(),
            "{file} misses"
        );
        assert!(
            (f64::from(r.points) - entry["points_sp"].as_f64().unwrap()).abs() < 1e-4,
            "{file} points"
        );
        assert_eq!(
            r.total_score,
            entry["total_score"].as_i64().unwrap(),
            "{file} score"
        );
        assert_eq!(r.mods, entry["mods"].as_str().unwrap(), "{file} mods");
        assert_eq!(
            r.passed,
            entry["passed"].as_i64().unwrap() != 0,
            "{file} passed"
        );
        assert!(
            (f64::from(r.speed) - entry["speed"].as_f64().unwrap()).abs() < 1e-6,
            "{file} speed"
        );
        let expected_fail = entry["fail_time"].as_i64().unwrap_or(-1);
        assert_eq!(i64::from(r.fail_time_ms), expected_fail, "{file} fail time");
        assert_eq!(
            r.beatmap_hash,
            entry["beatmap_hash"].as_str().unwrap(),
            "{file} hash"
        );

        // The core frame invariant: flagged frames equal the header hits.
        assert_eq!(
            i64::from(r.flagged_frames()),
            entry["hits"].as_i64().unwrap(),
            "{file} flag sum"
        );
        // Frames are chronological.
        assert!(
            r.frames.windows(2).all(|w| w[0].ms <= w[1].ms),
            "{file} frame times monotonic"
        );
    }
}

#[test]
fn cache_json_maps_load_with_expected_note_counts() {
    for entry in manifest() {
        let local = &entry["map_local"];
        let map = Map::from_path(testdata(local["map_json"].as_str().unwrap())).unwrap();
        assert_eq!(
            map.notes.len() as u64,
            local["noteCount"].as_u64().unwrap(),
            "{} note count",
            local["map_json"]
        );
        assert!(
            map.notes.windows(2).all(|w| w[0].time_ms <= w[1].time_ms),
            "notes sorted"
        );
        assert_eq!(map.meta.legacy_id, local["legacyId"].as_str().unwrap());
        assert_eq!(map.meta.title, local["title"].as_str().unwrap());
        // Cache JSONs have no embedded media.
        assert!(map.audio.is_none() && map.cover.is_none());
    }
}
