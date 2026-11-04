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
