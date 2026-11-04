//! `rhythr-cli mark-check` end to end: a clip marked with the dev pattern is
//! found with the built-in dev key, a clean clip is not. Skipped when ffmpeg
//! is not on PATH.
use rhythia_render::mark::check::resample;
use rhythia_render::mark::{pattern, seed_from_hex, AMPLITUDE, DEV_KEY_HEX, PAT_H, PAT_W};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Encodes `frames` grey frames of `w` by `h` (black 16 plus the stretched
/// pattern when `marked`) with libx264 at CRF 17.
fn encode(out: &Path, marked: bool, frames: usize, w: usize, h: usize) {
    let stretched = resample(&pattern(&seed_from_hex(DEV_KEY_HEX).unwrap()), PAT_H, PAT_W, h, w);
    let mut child = Command::new("ffmpeg")
        .args([
            "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "gray", "-s", &format!("{w}x{h}"), "-r", "30", "-i",
            "pipe:0", "-c:v", "libx264", "-crf", "17", "-pix_fmt", "yuv420p",
        ])
        .arg(out)
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn ffmpeg");
    {
        let stdin = child.stdin.as_mut().unwrap();
        for _ in 0..frames {
            let frame: Vec<u8> = (0..w * h)
                .map(|i| if marked { (16.0 + AMPLITUDE * stretched[i]).round().clamp(0.0, 255.0) as u8 } else { 16 })
                .collect();
            stdin.write_all(&frame).unwrap();
        }
    }
    assert!(child.wait().unwrap().success(), "ffmpeg failed");
}

#[test]
fn a_marked_clip_is_found_with_the_dev_key_and_a_clean_one_is_not() {
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg not found");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let marked = dir.path().join("marked.mp4");
    let clean = dir.path().join("clean.mp4");
    encode(&marked, true, 60, 320, 180);
    encode(&clean, false, 60, 320, 180);
    let bin = env!("CARGO_BIN_EXE_rhythr-cli");

    let out = Command::new(bin).args(["mark-check", "--json"]).arg(&marked).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "marked: status {:?}, stdout {text}, stderr {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(text.trim()).expect("json");
    assert_eq!(v["kind"], "Found", "{text}");
    assert_eq!(v["key"], "built-in dev", "{text}");

    let out = Command::new(bin).args(["mark-check", "--json"]).arg(&clean).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(text.trim()).expect("json");
    assert_eq!(v["kind"], "None", "{text}");
    assert_eq!(out.status.code(), Some(1));
}
