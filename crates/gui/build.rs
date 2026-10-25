use std::fs;
use std::path::Path;

#[path = "build_support.rs"]
mod build_support;

fn main() {
    println!("cargo:rerun-if-changed=ui");
    let src = Path::new("ui");
    let dist = Path::new("ui-dist");
    // Start clean: a file deleted from ui/ must not keep shipping inside
    // the exe from an earlier build.
    let _ = fs::remove_dir_all(dist);
    build_support::copy_tree(src, dist);
    // The embedded HUD font's license requires shipping with distributions.
    println!("cargo:rerun-if-changed=../render/assets/hud-font-LICENSE");
    if let Ok(l) = fs::read_to_string("../render/assets/hud-font-LICENSE") {
        fs::write("FONT-LICENSE.txt", l.replace('\n', "\r\n")).expect("write FONT-LICENSE.txt");
    }
    // The user guide ships next to the exe as README.txt.
    println!("cargo:rerun-if-changed=../../docs/USER-GUIDE.md");
    if let Ok(guide) = fs::read_to_string("../../docs/USER-GUIDE.md") {
        fs::write("README.txt", guide.replace('\n', "\r\n")).expect("write README.txt");
    }
    // A visible build identity: "which build am I running" must never be
    // guesswork again.
    let stamp = std::process::Command::new("git")
        .args(["log", "-1", "--format=%cd-%h", "--date=format:%d.%m.%H%M"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "dev".into());
    println!("cargo:rustc-env=RHYTHR_BUILD={stamp}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");

    tauri_build::build();
}
