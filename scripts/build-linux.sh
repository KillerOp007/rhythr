#!/usr/bin/env bash
set -euo pipefail
# Builds the Linux packages — AppImage (self-contained, bundles ffmpeg),
# deb and rpm — inside an Ubuntu 22.04 container so the binaries keep a
# glibc 2.35 baseline and run on any mainstream distro from 2022 onward.
# Building directly on a newer host would silently raise that floor.
#
# Prereqs: podman; a static linux64 ffmpeg + its LICENSE staged as
# crates/gui/ffmpeg and crates/gui/ffmpeg-LICENSE.txt (see
# docs/LINUX-BUILD.md); TAURI_SIGNING_PRIVATE_KEY for the updater .sig.
# Output: target-linux22/release/bundle/{appimage,deb,rpm}/

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMG=rhythr-linux-build
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/rhythr-linux-build"
mkdir -p "$CACHE/registry" "$CACHE/git" "$CACHE/tauri"

if [ ! -f "$ROOT/crates/gui/ffmpeg" ]; then
    echo "error: crates/gui/ffmpeg missing (static linux ffmpeg — docs/LINUX-BUILD.md)" >&2
    exit 1
fi

podman image exists "$IMG" || podman build -t "$IMG" -f "$ROOT/packaging/linux/Containerfile" "$ROOT/packaging/linux"

podman run --rm \
    -v "$ROOT":/work \
    -v "$CACHE/registry":/root/.cargo/registry \
    -v "$CACHE/git":/root/.cargo/git \
    -v "$CACHE/tauri":/root/.cache/tauri \
    -e CARGO_TARGET_DIR=/work/target-linux22 \
    -e APPIMAGE_EXTRACT_AND_RUN=1 \
    -e TAURI_SIGNING_PRIVATE_KEY \
    -e TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
    -w /work/crates/gui \
    "$IMG" bash -ec '
        # Native packages first (system ffmpeg via dependency/docs) …
        cargo tauri build --bundles deb,rpm
        # … then the AppImage with the bundled ffmpeg overlaid in.
        cargo tauri build --bundles appimage --config tauri.appimage.conf.json
    '

echo
echo "bundles:"
find "$ROOT/target-linux22/release/bundle" -maxdepth 2 -type f \
    \( -name "*.AppImage" -o -name "*.deb" -o -name "*.rpm" -o -name "*.sig" \) | sort
