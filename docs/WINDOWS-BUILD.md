# Windows build — desktop app + NSIS installer

## Preferred: cross-build from Linux

The NSIS installer cross-builds on Linux (~45 MB, ffmpeg bundled) —
this is the tested path. One-time setup:

```sh
sudo apt install nsis clang lld llvm
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin            # fetches the Windows SDK on first use (~1.1 GB cache)
npm install @tauri-apps/cli         # in any scratch dir; or cargo install tauri-cli
# drop ffmpeg.exe + ffmpeg-LICENSE.txt into crates/gui/ (see "ffmpeg bundling")
```

Build:

```sh
cd crates/gui
tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc
# -> target/x86_64-pc-windows-msvc/release/bundle/nsis/rhythr_<ver>_x64-setup.exe
```

The LNK4099 "cannot use debug info" linker warnings are expected (the
xwin SDK ships no PDBs) and harmless. The installer is unsigned —
Windows SmartScreen will show "unknown publisher" on first run.

## Alternative: native build on Windows

### One-time setup

1. **Rust** — https://rustup.rs (MSVC toolchain, the default).
2. **Visual Studio Build Tools 2022** — workload "Desktop development with
   C++" (the rustup installer offers to install this for you).
3. **WebView2 runtime** — preinstalled on Windows 10/11; the installer
   bundles a bootstrapper for machines that lack it.
4. **Tauri CLI**:
   ```
   cargo install tauri-cli --version "^2"
   ```

## ffmpeg bundling

The app looks for ffmpeg in this order (see `resolve_ffmpeg` in
`crates/gui/src/main.rs`):

1. explicit path from the app settings (Advanced → ffmpeg path)
2. `ffmpeg.exe` **next to the app exe** (this is how the installer ships it)
3. `ffmpeg` on PATH

To ship it in the installer:

1. Download a Windows build of ffmpeg (e.g. https://www.gyan.dev/ffmpeg/builds/
   "release essentials", or BtbN's builds). Only `bin/ffmpeg.exe` is needed;
   NVENC support is included in the standard builds.
2. Copy `ffmpeg.exe` into `crates/gui/` (next to `tauri.conf.json`).
3. `crates/gui/tauri.conf.json` already lists it under bundle resources:
   ```json
   "bundle": {
     ...
     "resources": ["ffmpeg.exe"]
   }
   ```
   Resources land in the install directory next to the exe, which is exactly
   where `resolve_ffmpeg` looks.

**License note:** ffmpeg's prebuilt binaries are GPL. Shipping the unmodified
exe and invoking it as a separate process keeps this project MIT — but the
installer must mention it. Include ffmpeg's license text (the builds ship a
`LICENSE` file — add it to `resources` too) and state in the README/about
where the ffmpeg source can be obtained (the build pages above link it).
Do **not** link ffmpeg as a library.

### Build

```
cd crates\gui
cargo tauri build
```

Outputs:

- App exe: `target\release\rhythr.exe`
- NSIS installer: `target\release\bundle\nsis\rhythr_<version>_x64-setup.exe`

The installer registers the `.rhr` file association (double-clicking a replay
opens the app with it loaded — the path arrives as the first CLI argument)
and installs per-user (no admin prompt, `installMode: currentUser`).

## What to verify on first Windows run

- On NVIDIA GPUs the topbar should show "Hardware encoder: NVENC". Auto mode probes nvenc → qsv → vaapi → x264.
- GPU renderer initializes via wgpu/Vulkan on the NVIDIA driver; if the
  driver is exotic, wgpu falls back through DX12 — no code change needed.
- Render a short replay end-to-end and check audio sync + the results
  screen tail.
- `.rhr` double-click opens the app with the replay loaded.

## Headless testing on Linux

- Under Xvfb, force `GDK_BACKEND=x11` and set
  `WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1`,
  otherwise the window attaches to a live Wayland session or renders
  blank. Irrelevant on Windows (WebView2, not WebKitGTK).
