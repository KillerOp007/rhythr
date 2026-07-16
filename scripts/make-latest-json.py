#!/usr/bin/env python3
"""Builds the updater manifest (latest.json) for a GitHub release.

Run after building both platforms with updater artifacts enabled:
  python3 scripts/make-latest-json.py <version>
Reads the .sig files tauri emitted next to the bundles and writes
latest.json to the repo root; upload it to the release alongside the
installers (the app checks releases/latest/download/latest.json).
"""
import json
import sys
import datetime
from pathlib import Path

version = sys.argv[1].lstrip("v")
root = Path(__file__).resolve().parent.parent
win = root / f"target/x86_64-pc-windows-msvc/release/bundle/nsis/rhythr_{version}_x64-setup.exe"
appimage = root / f"target/release/bundle/appimage/rhythr_{version}_amd64.AppImage"
base = f"https://github.com/KillerOp007/rhythr/releases/download/v{version}"

platforms = {}
for key, artifact, name in [
    ("windows-x86_64", win, f"rhythr_{version}_x64-setup.exe"),
    ("linux-x86_64", appimage, f"rhythr_{version}_amd64.AppImage"),
]:
    sig = artifact.with_name(artifact.name + ".sig")
    if not artifact.exists() or not sig.exists():
        print(f"skip {key}: missing {artifact.name} or its .sig")
        continue
    platforms[key] = {"signature": sig.read_text().strip(), "url": f"{base}/{name}"}

if not platforms:
    sys.exit("no signed artifacts found — build with TAURI_SIGNING_PRIVATE_KEY set")

manifest = {
    "version": version,
    "notes": f"rhythr {version} — see the GitHub release page for details.",
    "pub_date": datetime.datetime.now(datetime.timezone.utc)
        .isoformat(timespec="seconds").replace("+00:00", "Z"),
    "platforms": platforms,
}
out = root / "latest.json"
out.write_text(json.dumps(manifest, indent=2) + "\n")
print(f"wrote {out} with {', '.join(platforms)}")
