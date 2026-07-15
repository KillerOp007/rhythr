use std::fs;
use std::path::Path;

/// Strips developer comments out of the frontend before tauri-build embeds
/// it: the HTML/JS/CSS ship as readable text inside the exe, and comments
/// are for the repo, not for whoever runs `strings` on a release binary.
/// Conservative rules only — nothing that could touch string literals:
/// whole `//`-lines in JS, `/* … */` blocks in CSS, `<!-- … -->` in HTML,
/// and blank lines everywhere.
fn strip_comments(name: &str, text: &str) -> String {
    let mut out = text.to_string();
    let strip_blocks = |out: &mut String, open: &str, close: &str| loop {
        let Some(a) = out.find(open) else { break };
        let Some(rel) = out[a..].find(close) else { break };
        out.replace_range(a..a + rel + close.len(), "");
    };
    if name.ends_with(".css") {
        strip_blocks(&mut out, "/*", "*/");
    }
    if name.ends_with(".html") {
        strip_blocks(&mut out, "<!--", "-->");
    }
    out.lines()
        .filter(|line| {
            let t = line.trim_start();
            if t.is_empty() {
                return false;
            }
            // JS whole-line comments; never touches `//` inside strings
            // because those lines don't *start* with it.
            !(name.ends_with(".js") && t.starts_with("//"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn main() {
    println!("cargo:rerun-if-changed=ui");
    let src = Path::new("ui");
    let dist = Path::new("ui-dist");
    fs::create_dir_all(dist).expect("create ui-dist");
    for entry in fs::read_dir(src).expect("read ui/") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let data = fs::read(entry.path()).expect("read ui file");
        let out = if name.ends_with(".js") || name.ends_with(".css") || name.ends_with(".html") {
            strip_comments(&name, &String::from_utf8_lossy(&data)).into_bytes()
        } else {
            data
        };
        fs::write(dist.join(&name), out).expect("write ui-dist file");
    }
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
    tauri_build::build();
}
