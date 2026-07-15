//! Manifest validation: parses every replay listed in a
//! `testdata_manifest.json` (ground-truth values exported from the game's
//! own database) and requires the parser output to match exactly.

use std::path::Path;

use anyhow::Context;
use rhythia_formats::{map::Map, rhr::Replay};
use rhythia_sim::integrity;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Entry {
    pub file: String,
    pub rhr_version: i32,
    pub player: String,
    pub map_online_id: i32,
    pub legacy_map_id: String,
    pub mode: String,
    pub accuracy: f64,
    pub hits: i32,
    pub misses: i32,
    pub points_sp: f64,
    pub total_score: i64,
    pub mods: String,
    pub passed: i32,
    pub speed: f64,
    pub fail_time: Option<i32>,
    pub beatmap_hash: String,
    pub blob_bytes: u64,
    pub map_local: MapLocal,
}

#[derive(Deserialize)]
pub struct MapLocal {
    pub map_json: String,
    #[serde(rename = "noteCount")]
    pub note_count: usize,
}

/// f32 fields from the game DB arrive as f64 in the manifest; compare with
/// the precision an f32 round-trip can guarantee.
fn f32_matches(actual: f32, expected: f64) -> bool {
    (f64::from(actual) - expected).abs() < 1e-4
}

pub fn check_folder(dir: &Path) -> anyhow::Result<bool> {
    let manifest_path = dir.join("testdata_manifest.json");
    let raw = std::fs::read(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let entries: Vec<Entry> = serde_json::from_slice(&raw).context("parsing manifest")?;

    let mut all_ok = true;
    for entry in &entries {
        let path = dir.join(&entry.file);
        let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let replay = Replay::parse(&data).with_context(|| format!("parsing {}", entry.file))?;

        let mut failures: Vec<String> = Vec::new();
        let mut expect = |name: &str, ok: bool| {
            if !ok {
                failures.push(name.to_string());
            }
        };

        expect("blob_bytes", data.len() as u64 == entry.blob_bytes);
        expect("rhr_version", replay.version == entry.rhr_version);
        expect("player", replay.player_name == entry.player);
        expect("map_online_id", replay.map_id == entry.map_online_id);
        expect("legacy_map_id", replay.legacy_map_id == entry.legacy_map_id);
        expect("mode", replay.mode == entry.mode);
        expect("accuracy", f32_matches(replay.accuracy_pct, entry.accuracy));
        expect("hits", replay.hits == entry.hits);
        expect("misses", replay.misses == entry.misses);
        expect("points_sp", f32_matches(replay.points, entry.points_sp));
        expect("total_score", replay.total_score == entry.total_score);
        expect("mods", replay.mods == entry.mods);
        expect("passed", replay.passed == (entry.passed != 0));
        expect("speed", f32_matches(replay.speed, entry.speed));
        expect(
            "fail_time",
            replay.fail_time_ms == entry.fail_time.unwrap_or(-1),
        );
        expect("beatmap_hash", replay.beatmap_hash == entry.beatmap_hash);
        expect("no_trailing_bytes", replay.trailing_bytes == 0);
        expect(
            "sum_hitflags==hits",
            i64::from(replay.flagged_frames()) == i64::from(entry.hits),
        );

        let map_path = dir.join(&entry.map_local.map_json);
        let map =
            Map::from_path(&map_path).with_context(|| format!("reading {}", map_path.display()))?;
        expect(
            "map_note_count",
            map.notes.len() == entry.map_local.note_count,
        );

        let report = integrity::verify_replay(&replay, &map);
        expect("integrity_consistent", report.consistent());

        if failures.is_empty() {
            println!(
                "OK    {:<26} v{} {:>6} frames  {:>4} hits/{:<3} misses  acc {:.4}%  integrity ok",
                entry.file,
                replay.version,
                replay.frames.len(),
                replay.hits,
                replay.misses,
                replay.accuracy_pct,
            );
        } else {
            all_ok = false;
            println!(
                "FAIL  {:<26} mismatches: {}",
                entry.file,
                failures.join(", ")
            );
            for check in report.failed_checks() {
                println!(
                    "      integrity: {} expected {}, got {}",
                    check.name, check.expected, check.actual
                );
            }
        }
    }

    if all_ok {
        println!("\nall {} manifest entries match exactly", entries.len());
    } else {
        println!("\nMANIFEST VALIDATION FAILED");
    }
    Ok(all_ok)
}
