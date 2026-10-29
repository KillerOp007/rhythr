#!/usr/bin/env bash
set -euo pipefail
# Builds the Linux packages: AppImage (self-contained, bundles ffmpeg),
# deb and rpm. The build runs inside an Ubuntu 22.04 container so the
# binaries keep a glibc 2.35 baseline and run on any mainstream distro
# from 2022 onward.
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
    echo "error: crates/gui/ffmpeg missing (static linux ffmpeg, see docs/LINUX-BUILD.md)" >&2
    exit 1
fi

# Tauri CLI 2.12.1 is the floor: older bundlers ship a linuxdeploy whose
# excludelist lets libwayland/libxkbcommon/libxcb into the AppImage, which
# aborts with EGL_BAD_PARAMETER against newer Mesa (SteamOS 3.8, Fedora 44).
# An image built before that floor is rebuilt from scratch.
MIN_CLI=2.12.1
cli_ok() {
    local have
    have="$(podman run --rm "$IMG" cargo-tauri --version 2>/dev/null | awk '{print $2}')"
    [ -n "$have" ] && [ "$(printf '%s\n' "$MIN_CLI" "$have" | sort -V | head -n1)" = "$MIN_CLI" ]
}
if ! podman image exists "$IMG" || ! cli_ok; then
    podman build --no-cache -t "$IMG" -f "$ROOT/packaging/linux/Containerfile" "$ROOT/packaging/linux"
fi

# A linked worktree keeps its .git outside the tree; mount the repository's
# git dir read-only at its host path so build.rs can stamp the build.
GIT_COMMON="$(cd "$ROOT" && git rev-parse --path-format=absolute --git-common-dir)"
GIT_MOUNT=()
case "$GIT_COMMON" in "$ROOT"/*) ;; *) GIT_MOUNT=(-v "$GIT_COMMON":"$GIT_COMMON":ro) ;; esac

podman run --rm \
    -v "$ROOT":/work "${GIT_MOUNT[@]}" \
    -v "$CACHE/registry":/root/.cargo/registry \
    -v "$CACHE/git":/root/.cargo/git \
    -v "$CACHE/tauri":/root/.cache/tauri \
    -e CARGO_TARGET_DIR=/work/target-linux22 \
    -e APPIMAGE_EXTRACT_AND_RUN=1 \
    -e TAURI_SIGNING_PRIVATE_KEY \
    -e TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
    -w /work/crates/gui \
    "$IMG" bash -ec '
        # Native packages first: deb and rpm declare ffmpeg as a dependency,
        # so they carry none themselves (no ffmpeg in their bundle.*.files).
        cargo tauri build --bundles deb,rpm
        # Then the AppImage, which bundles its own ffmpeg via
        # bundle.linux.appimage.files in tauri.conf.json. There is no overlay
        # config, because that file was deleted (it was named after a bundle
        # format, which Tauri never reads).
        cargo tauri build --bundles appimage
    '

# The AppImage must leave the display stack to the host: these libraries
# clash with newer Mesa (EGL_BAD_PARAMETER at startup, docs/LINUX-BUILD.md).
APPDIR="$ROOT/target-linux22/release/bundle/appimage/rhythr.AppDir"
[ -d "$APPDIR/usr/lib" ] || { echo "error: AppDir not found at $APPDIR" >&2; exit 1; }
stray="$(find "$APPDIR/usr/lib" \( -name 'libwayland-*' -o -name 'libxkbcommon*' \
    -o -name 'libxcb*' -o -name 'libXau*' -o -name 'libXdmcp*' \) -printf '%f\n' | sort)"
if [ -n "$stray" ]; then
    echo "error: the AppImage bundles display libraries that break on newer Mesa:" >&2
    echo "$stray" >&2
    exit 1
fi

echo
echo "bundles:"
find "$ROOT/target-linux22/release/bundle" -maxdepth 2 -type f \
    \( -name "*.AppImage" -o -name "*.deb" -o -name "*.rpm" -o -name "*.sig" \) | sort
