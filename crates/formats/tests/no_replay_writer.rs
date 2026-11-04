//! Hard rule #1 of the project, as a test: nothing in the workspace turns a
//! `Replay` back into bytes. The synthetic Autoplay run is a `Replay` value
//! in memory and has to stay one. This is a grep-level scan over every Rust
//! source under `crates/`: it reads signatures, not semantics, and errs on
//! the side of flagging.
//!
//! Flagged:
//! - a `Serialize` derive or impl for the `rhr` types (`Replay`, `Frame`);
//! - a function that takes a `Replay` (or replay frames) and returns bytes
//!   (`Vec<u8>`, `[u8]`, `Box<[u8]>`) or takes an `io::Write` sink;
//! - a file write whose path mentions `.rhr`.
//!
//! Rendering a video from a replay is the product and is not what this test
//! is about: a video is not a replay and nothing can be loaded from it.

use std::path::{Path, PathBuf};

fn rust_sources(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if name == "target" || name == "node_modules" || name.starts_with('.') {
                continue;
            }
            rust_sources(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The source without its `//` comment lines.
fn without_comments(src: &str) -> String {
    src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn has_word(hay: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(i) = hay[from..].find(word) {
        let at = from + i;
        let before = hay[..at].chars().next_back().is_none_or(|c| !is_word_char(c));
        let after = hay[at + word.len()..].chars().next().is_none_or(|c| !is_word_char(c));
        if before && after {
            return true;
        }
        from = at + word.len();
    }
    false
}

/// Every `fn` signature: (head up to the parameter list, parameters, the rest
/// up to the body or the semicolon, which holds the return type and a where
/// clause).
fn signatures(src: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut from = 0;
    while let Some(i) = src[from..].find("fn ") {
        let at = from + i;
        from = at + 3;
        if at > 0 && is_word_char(src[..at].chars().next_back().unwrap()) {
            continue;
        }
        let Some(open_rel) = src[at..].find('(') else { break };
        let open = at + open_rel;
        let mut depth = 0i32;
        let mut close = None;
        for (k, &b) in bytes[open..].iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + k);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { break };
        let rest_end = src[close..].find(['{', ';']).map(|k| close + k).unwrap_or(src.len());
        out.push((src[at..open].to_string(), src[open + 1..close].to_string(), src[close + 1..rest_end].to_string()));
        from = close;
    }
    out
}

/// Functions that take a replay and hand back bytes that are not the replay:
/// the pixels of a rendered frame, or a skin's icon images picked by the
/// replay's mods. A new name here needs the same kind of reason.
const PIXEL_PRODUCERS: &[&str] = &["render_still", "render_still_with_ghost", "render_results", "render_card", "active_mod_icons"];

fn fn_name(head: &str) -> &str {
    let after = head.rsplit("fn ").next().unwrap_or("");
    after.split(|c: char| !is_word_char(c)).next().unwrap_or("")
}

fn returns_bytes(rest: &str) -> bool {
    let ret = rest.split_once("->").map(|(_, r)| r).unwrap_or("");
    let ret: String = ret.chars().filter(|c| !c.is_whitespace()).collect();
    ret.contains("Vec<u8>") || ret.contains("[u8]")
}

fn takes_write_sink(head: &str, params: &str, rest: &str) -> bool {
    let all = format!("{head} {params} {rest}");
    if all.contains("io::Write") {
        return true;
    }
    has_word(&all, "Write") && !all.contains("fmt::Write")
}

#[test]
fn nothing_in_the_workspace_turns_a_replay_into_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap().join("crates");
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    assert!(files.len() > 20, "the scan found only {} Rust files under {}", files.len(), root.display());
    let mut violations = Vec::new();
    for f in &files {
        if f.ends_with("no_replay_writer.rs") {
            continue;
        }
        let raw = std::fs::read_to_string(f).unwrap();
        let src = without_comments(&raw);
        let shown = f.strip_prefix(&root).unwrap_or(f).display().to_string();
        let flat: String = src.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.contains("Serialize for Replay") || flat.contains("Serialize for Frame") {
            violations.push(format!("{shown}: a Serialize impl for a replay type"));
        }
        for (head, params, rest) in signatures(&src) {
            if PIXEL_PRODUCERS.contains(&fn_name(&head)) {
                continue;
            }
            let takes_replay = has_word(&params, "Replay") || params.contains("[Frame]") || params.contains("Vec<Frame>");
            if takes_replay && (returns_bytes(&rest) || takes_write_sink(&head, &params, &rest)) {
                violations.push(format!("{shown}: `{}(...)` takes a replay and returns bytes or writes to a sink", head.trim()));
            }
        }
        for (n, line) in src.lines().enumerate() {
            let writes = line.contains("fs::write(") || line.contains("File::create(") || line.contains("OpenOptions");
            if writes && line.to_ascii_lowercase().contains("rhr") {
                violations.push(format!("{shown}:{}: writes a path that mentions rhr", n + 1));
            }
        }
    }
    let rhr = std::fs::read_to_string(root.join("formats/src/rhr.rs")).unwrap();
    assert!(!has_word(&without_comments(&rhr), "Serialize"), "rhr.rs must stay a parser: no Serialize derive");
    assert!(violations.is_empty(), "replay writer candidates:\n{}", violations.join("\n"));
}
