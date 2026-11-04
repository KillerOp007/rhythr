//! `rhythr-cli autoplay` end to end: a tiny map without audio renders a
//! tagged, marked video with a note about the missing audio; `mark-check`
//! then finds the mark. Skipped when ffmpeg is not on PATH.
use std::io::Write;
use std::process::{Command, Stdio};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

const MAP_JSON: &str = r#"{"OnlineId":7,"LegacyId":"tiny","Title":"Tiny","SongName":"Tiny Song",
        "Duration":2500,"StarRating":1.0,"Notes":[{"Time":400,"X":0,"Y":0},{"Time":900,"X":2,"Y":1},{"Time":1500,"X":1,"Y":2}]}"#;

fn build_rhm(path: &std::path::Path) {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zw.start_file("map", opts).unwrap();
    zw.write_all(MAP_JSON.as_bytes()).unwrap();
    std::fs::write(path, zw.finish().unwrap().into_inner()).unwrap();
}

#[test]
fn a_map_renders_tagged_and_marked_with_a_note_about_the_missing_audio() {
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg not found");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let rhm = dir.path().join("t.rhm");
    let out = dir.path().join("out.mp4");
    build_rhm(&rhm);
    let bin = env!("CARGO_BIN_EXE_rhythr-cli");
    let run = Command::new(bin)
        .args(["autoplay", "--map"])
        .arg(&rhm)
        .arg("-o")
        .arg(&out)
        .args(["--width", "320", "--height", "240", "--fps", "30", "--encoder", "x264"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "autoplay failed: {stderr}");
    assert!(out.exists(), "no output file");
    assert!(stderr.contains("RH-AUT-904"), "no audio note in: {stderr}");

    let probe = Command::new("ffmpeg").args(["-hide_banner", "-i"]).arg(&out).output().unwrap();
    let banner = String::from_utf8_lossy(&probe.stderr);
    assert!(banner.contains("rhythr autoplay render: not a real play"), "no metadata tag in: {banner}");

    let check = Command::new(bin).args(["mark-check", "--json"]).arg(&out).output().unwrap();
    let text = String::from_utf8_lossy(&check.stdout);
    let v: serde_json::Value = serde_json::from_str(text.trim()).expect("json");
    assert_eq!(v["kind"], "Found", "{text}");
}

/// A longer synthetic map: `n` notes a quarter second apart spread over the
/// field off the grid (as real maps are), so the picture has something
/// moving in it that does not pile up on the same pixels.
fn build_long_rhm(path: &std::path::Path, n: usize) {
    let notes: Vec<String> = (0..n)
        .map(|i| format!(r#"{{"Time":{},"X":{:.2},"Y":{:.2}}}"#, 400 + i * 250, (i * 37 % 250) as f64 / 100.0, (i * 53 % 250) as f64 / 100.0))
        .collect();
    let json = format!(
        r#"{{"OnlineId":8,"LegacyId":"long","Title":"Long","SongName":"Long Song","Duration":{},"StarRating":1.0,"Notes":[{}]}}"#,
        400 + n * 250,
        notes.join(",")
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zw.start_file("map", opts).unwrap();
    zw.write_all(json.as_bytes()).unwrap();
    std::fs::write(path, zw.finish().unwrap().into_inner()).unwrap();
}

/// The spec's bar for the hidden mark: a 1080p render re-encoded at 720p
/// and 2.5 Mbit/s (a video platform's treatment) still checks with z above
/// 50. Skipped when ffmpeg is not on PATH.
#[test]
fn the_mark_survives_a_720p_re_encode_at_two_and_a_half_megabit() {
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg not found");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let rhm = dir.path().join("long.rhm");
    let out = dir.path().join("out.mp4");
    let small = dir.path().join("small.mp4");
    // 48 notes: 15 seconds, so the checker averages about 150 frames, which
    // is a short clip, not a corner case; 24 notes landed at z 53
    build_long_rhm(&rhm, 48);
    let bin = env!("CARGO_BIN_EXE_rhythr-cli");
    let run = Command::new(bin)
        .args(["autoplay", "--map"])
        .arg(&rhm)
        .arg("-o")
        .arg(&out)
        .args(["--width", "1920", "--height", "1080", "--fps", "30", "--encoder", "x264"])
        .output()
        .unwrap();
    assert!(run.status.success(), "autoplay failed: {}", String::from_utf8_lossy(&run.stderr));
    let enc = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i"])
        .arg(&out)
        .args(["-vf", "scale=-2:720", "-c:v", "libx264", "-b:v", "2.5M", "-an"])
        .arg(&small)
        .output()
        .unwrap();
    assert!(enc.status.success(), "re-encode failed: {}", String::from_utf8_lossy(&enc.stderr));

    let check = Command::new(bin).args(["mark-check", "--json"]).arg(&small).output().unwrap();
    let text = String::from_utf8_lossy(&check.stdout);
    let v: serde_json::Value = serde_json::from_str(text.trim()).expect("json");
    eprintln!("re-encoded 720p 2.5M: {text}");
    assert_eq!(v["kind"], "Found", "{text}");
    let z = v["z"].as_f64().unwrap_or(0.0);
    assert!(z > 50.0, "z {z} after the re-encode, the spec wants more than 50: {text}");
}
