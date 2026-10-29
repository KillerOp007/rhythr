# Building the Linux packages

Three artifacts per release, all built by one script:

| Artifact | For | ffmpeg |
| --- | --- | --- |
| `rhythr_x.y.z_amd64.AppImage` | any distro (the recommended download) | bundled |
| `rhythr_x.y.z_amd64.deb` | Debian/Ubuntu/Mint | apt dependency |
| `rhythr-x.y.z-1.x86_64.rpm` | Fedora/openSUSE | user-installed (RPM Fusion) |

## Why a container

The binaries inherit the glibc of the build system. Built on the dev
machine (Ubuntu 24.04, glibc 2.39) they refuse to start on Ubuntu 22.04,
Debian 12 or Mint 21. `scripts/build-linux.sh` therefore builds inside an
**Ubuntu 22.04** container (glibc 2.35 baseline: the oldest possible,
since Tauri 2 needs webkit2gtk-4.1, which first shipped in 22.04). The
result runs on any mainstream distro from 2022 onward, including SteamOS
3.5+ and Arch derivatives.

## One-time setup

```sh
sudo apt install podman        # or docker; the script uses podman
```

Stage the static ffmpeg the AppImage bundles (mirrors the Windows flow):
download the latest **n7.1** `linux64-gpl` build from
https://github.com/BtbN/FFmpeg-Builds/releases, then

```sh
cp .../bin/ffmpeg        crates/gui/ffmpeg
cp .../LICENSE.txt       crates/gui/ffmpeg-LICENSE.txt
```

Both are gitignored, like `ffmpeg.exe` for Windows.

## Build

```sh
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/rhythr-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
scripts/build-linux.sh
```

First run builds the container image (~10 min); after that only the app
compiles. Bundles land in `target-linux22/release/bundle/`, with a
`.sig` next to the AppImage for the auto-updater.
`scripts/make-latest-json.py <version>` picks it up from there.

## Notes

- The AppImage carries webkit/GTK and ffmpeg: self-contained by design.
  The deb/rpm stay slim and use the system webkit 4.1 + ffmpeg instead.
- The display stack stays with the host. Tauri CLI 2.12.1+ is required:
  its linuxdeploy keeps libwayland, libxkbcommon, libxcb, libXau and
  libXdmcp out of the AppImage. Older bundlers shipped Ubuntu 22.04
  copies that shadow the host's, and newer Mesa (SteamOS 3.8, Fedora 44)
  then aborts at startup with `Could not create default EGL display:
  EGL_BAD_PARAMETER` (the 0.6.2 AppImage has this bug). The script
  rebuilds the image when its CLI is older and fails the build if any
  of those libraries land in the AppDir.
- On Linux the app prefers the **system** ffmpeg when one is installed
  (its VAAPI/NVENC are linked against the system's driver stack; the
  bundled static build is the fallback so the AppImage works stand-alone).
- The updater self-installs only for the AppImage (and the Windows
  installer); deb/rpm users get a "download page" banner instead.
- The AUR package (`packaging/aur/`) repackages the release deb. See
  its README for the publishing steps.
