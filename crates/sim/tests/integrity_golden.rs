//! Integrity check against the four real kit replays: every derived value
//! must reproduce the game's own database numbers exactly — and a tampered
//! header must be detected.

use rhythia_formats::{map::Map, rhr::Replay};
use rhythia_sim::integrity;

fn testdata(path: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(path)
}

fn load_all() -> Vec<(String, Replay, Map)> {
    let raw = std::fs::read(testdata("testdata_manifest.json")).unwrap();
    let manifest: Vec<serde_json::Value> = serde_json::from_slice(&raw).unwrap();
    manifest
        .iter()
        .map(|e| {
            let file = e["file"].as_str().unwrap().to_string();
            let replay = Replay::from_path(testdata(&file)).unwrap();
            let map =
                Map::from_path(testdata(e["map_local"]["map_json"].as_str().unwrap())).unwrap();
            (file, replay, map)
        })
        .collect()
}

#[test]
fn real_replays_are_consistent() {
    for (file, replay, map) in load_all() {
        let report = integrity::verify_replay(&replay, &map);
        assert!(
            report.consistent(),
            "{file} flagged inconsistent: {:?}",
            report.failed_checks().collect::<Vec<_>>()
        );
        assert_eq!(report.derived_hits, replay.hits as u32, "{file} hits");
        assert_eq!(report.derived_misses, replay.misses as u32, "{file} misses");
        assert!(
            (report.derived_accuracy_pct - f64::from(replay.accuracy_pct)).abs() < 0.01,
            "{file} accuracy {} vs {}",
            report.derived_accuracy_pct,
            replay.accuracy_pct
        );
        assert_eq!(report.orphan_flags, 0, "{file} orphans");
    }
}

#[test]
fn tampered_header_is_detected() {
    for (file, mut replay, map) in load_all() {
        // Simulate replay editing: header claims better stats than the
        // frames support. (In-memory only — nothing is ever written.)
        replay.hits += 10;
        replay.misses = replay.misses.saturating_sub(10);
        replay.accuracy_pct = 100.0;
        let report = integrity::verify_replay(&replay, &map);
        assert!(
            !report.consistent(),
            "{file}: tampered header must be flagged"
        );
    }
}

#[test]
fn fail_replays_use_fail_time_cutoff() {
    for (file, replay, map) in load_all() {
        let report = integrity::verify_replay(&replay, &map);
        if replay.failed() {
            assert!(
                (report.attempted_notes as usize) < map.notes.len(),
                "{file}: fail run must not count post-fail notes"
            );
        } else {
            assert_eq!(report.attempted_notes as usize, map.notes.len(), "{file}");
        }
        assert_eq!(
            report.attempted_notes,
            report.derived_hits + report.derived_misses,
            "{file} attempted = hits + misses"
        );
    }
}
