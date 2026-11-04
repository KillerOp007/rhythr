#!/usr/bin/env bash
set -euo pipefail
# Builds the Windows installer (NSIS) through cargo-xwin, exactly as
# docs/WINDOWS-BUILD.md describes, and refuses without the Autoplay mark
# key: a build without it marks Autoplay videos with the public dev key
# and is a dev build, not a release.
#
# Prereqs: nsis, clang, lld, llvm, the x86_64-pc-windows-msvc target,
# cargo-xwin, a Tauri CLI, ffmpeg.exe + ffmpeg-LICENSE.txt staged in
# crates/gui; TAURI_SIGNING_PRIVATE_KEY for the updater .sig.
# Output: target/x86_64-pc-windows-msvc/release/bundle/nsis/rhythr_<ver>_x64-setup.exe

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TAURI="${TAURI_CLI:-/home/killer/tools/tauri-cli/node_modules/.bin/tauri}"

if [ -z "${RHYTHR_MARK_KEY:-}" ]; then
    echo "error: RHYTHR_MARK_KEY is not set (the Autoplay mark key, see docs/RELEASING.md)" >&2
    exit 1
fi
if [ ! -f "$ROOT/crates/gui/ffmpeg.exe" ]; then
    echo "error: crates/gui/ffmpeg.exe missing (static windows ffmpeg, see docs/WINDOWS-BUILD.md)" >&2
    exit 1
fi
if [ ! -x "$TAURI" ]; then
    echo "error: Tauri CLI not found at $TAURI (set TAURI_CLI)" >&2
    exit 1
fi

cd "$ROOT/crates/gui"
exec "$TAURI" build --runner cargo-xwin --target x86_64-pc-windows-msvc "$@"
