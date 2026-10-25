//! The frontend copy step of `build.rs`, in its own file so the crate's tests
//! can include it too (build scripts are not test targets, so a test written
//! inside `build.rs` would never run).

use std::fs;
use std::path::Path;

/// Strips developer comments out of the frontend before tauri-build embeds
/// it: the HTML/JS/CSS ship as readable text inside the exe, and comments
/// are for the repo, not for whoever runs `strings` on a release binary.
/// Conservative rules only (nothing that could touch string literals):
/// whole `//`-lines in JS, `/* … */` blocks in CSS, `<!-- … -->` in HTML,
/// and blank lines everywhere.
pub fn strip_comments(name: &str, text: &str) -> String {
    let mut out = text.to_string();
    let strip_blocks = |out: &mut String, open: &str, close: &str| {
        while let (Some(a), Some(rel)) = (
            out.find(open),
            out.find(open).and_then(|a| out[a..].find(close)),
        ) {
            out.replace_range(a..a + rel + close.len(), "");
        }
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

/// Copies the frontend tree `src` to `dist`, recursively, for tauri-build to
/// embed. Text files (`.js`, `.css`, `.html`) go through [`strip_comments`];
/// everything else is copied byte for byte. A directory named `tests` is
/// skipped at any depth: node test files never ship inside the exe.
pub fn copy_tree(src: &Path, dist: &Path) {
    fs::create_dir_all(dist).expect("create ui-dist dir");
    for entry in fs::read_dir(src).expect("read ui dir") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if path.is_dir() {
            if name == "tests" {
                continue;
            }
            copy_tree(&path, &dist.join(&name));
            continue;
        }
        let data = fs::read(&path).expect("read ui file");
        let out = if name.ends_with(".js") || name.ends_with(".css") || name.ends_with(".html") {
            strip_comments(&name, &String::from_utf8_lossy(&data)).into_bytes()
        } else {
            data
        };
        fs::write(dist.join(&name), out).expect("write ui-dist file");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frontend_tree_is_copied_with_subfolders_and_without_tests() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("ui");
        let dist = tmp.path().join("ui-dist");
        fs::create_dir_all(src.join("sub/tests")).unwrap();
        fs::write(
            src.join("a.js"),
            "// a comment\nconst x = 1;\n\nlet y = 2;\n",
        )
        .unwrap();
        fs::write(src.join("sub/b.css"), "/* gone */\nbody { margin: 0; }\n").unwrap();
        fs::write(src.join("sub/tests/c.mjs"), "import 'node:test';\n").unwrap();
        let blob = [0u8, 159, 146, 150, 10, 10, 255];
        fs::write(src.join("sub/d.bin"), blob).unwrap();

        copy_tree(&src, &dist);

        let a = fs::read_to_string(dist.join("a.js")).unwrap();
        assert_eq!(a, "const x = 1;\nlet y = 2;");
        let b = fs::read_to_string(dist.join("sub/b.css")).unwrap();
        assert_eq!(b, "body { margin: 0; }");
        assert_eq!(fs::read(dist.join("sub/d.bin")).unwrap(), blob);
        assert!(!dist.join("sub/tests").exists());
    }
}
