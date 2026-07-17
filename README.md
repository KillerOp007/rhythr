# rhythr

**rhythr** is an **unofficial community tool** that turns Rhythia
(`.rhr`) replays into videos. It is not affiliated with or endorsed by
Rhythia or Capo Games.

**[Download from the releases page](https://github.com/KillerOp007/rhythr/releases/latest)** —
then connect your game once via *Advanced → Detect game* and drop a
replay. GPU renderer (wgpu) aiming for pixel-parity with the game, so a
rendered video looks exactly like watching the replay in-game with your
own skin.

- **Windows** — `rhythr_x.y.z_x64-setup.exe`. Click through SmartScreen
  ("More info" → "Run anyway"; the installer is not code-signed).
- **Linux (any distro)** — `rhythr_x.y.z_amd64.AppImage`. One
  self-contained file (ffmpeg included): `chmod +x`, run. Needs glibc
  2.35+ (Ubuntu 22.04 / Debian 12 / Fedora 36 or newer, Arch, openSUSE
  Leap 15.6+, SteamOS 3.5+). If your distro lacks FUSE2, install
  `libfuse2`, or run with `--appimage-extract-and-run`.
- **Debian/Ubuntu/Mint** — `rhythr_x.y.z_amd64.deb`
  (`sudo apt install ./rhythr_*.deb`; uses the system ffmpeg and
  registers the `.rhr` file type).
- **Fedora/openSUSE** — `rhythr-x.y.z-1.x86_64.rpm`. Install ffmpeg
  separately (RPM Fusion on Fedora) or use the AppImage.
- **Arch** — install from the AUR (`rhythr-bin`), or use the AppImage.

The game runs through Proton on Linux; *Detect game* finds the Steam
(native, Flatpak or Snap) install and reads the built-in assets from it
just like on Windows.

**Status: working end-to-end.** Parsers, integrity check, the GPU renderer
(pixel-calibrated against real footage: notes, skins, HUD, results screen)
and a desktop app are functional, with Windows and Linux packages on the
releases page.

## Anti-cheat statement

This tool is a strict **read-only renderer**:

- No code path writes or re-encodes `.rhr` files — there is no replay
  serializer in this codebase, and none will be accepted.
- Nothing can alter cursor positions, hits, misses, timing, accuracy or
  mods. Only visuals are configurable (skin, camera, HUD, resolution, FPS).
- Every replay is run through an **integrity check**: hits/misses/accuracy
  are re-derived from the raw input frames and compared against the header.
  On mismatch the tool warns loudly and burns a "replay data inconsistent —
  possibly manipulated" notice into the rendered video.
- This check (and any rendered video) is a heuristic by this tool — it is
  **not** an official Rhythia score verification.

## Workspace

| Crate | Purpose |
|---|---|
| `crates/formats` | Read-only parsers: `.rhr` replays (all 5 version gates), map cache JSON, `.rhm` |
| `crates/sim` | Hit registration (frame flags → per-note results), integrity check |
| `crates/render` | wgpu GPU renderer: scene, skins/`.rhs` import, HUD, results screen, video export |
| `crates/cli` | `rhythia-render` binary: `info`, `verify`, `check`, `frame`, `video` |
| `crates/gui` | Desktop app (Tauri): preview + scrubber, HUD overrides, map auto-download, video export |

```sh
cargo run -p rhythia-cli -- info testdata/pass_long_score77.rhr
cargo run -p rhythia-cli -- verify testdata/fail_score131.rhr --map testdata/fail_map_map_json.json
cargo run -p rhythia-cli -- check testdata

# A single still frame:
cargo run -p rhythia-cli -- frame testdata/pass_long_score77.rhr \
    --map testdata/pass_long_map_map_json.json --at 1:30 -o frame.png

# A video clip (frames → ffmpeg + audio; needs ffmpeg on PATH):
cargo run -p rhythia-cli -- video testdata/pass_long_score77.rhr \
    --map testdata/pass_long_map_map_json.json --start 1:25 --end 1:37 -o clip.mp4
```

The renderer runs headlessly on the GPU (Vulkan) — the same wgpu backend
the game itself uses. The camera model (perspective FOV, note approach,
grid placement) was derived from the game's own shaders and calibrated
against real footage; no game code or assets are vendored.

## Desktop app

```sh
cargo run -p rhythia-gui
```

Drop a `.rhr` anywhere in the window: the map auto-downloads from
rhythia.com (hash-verified against the replay header and cached), the
replay is integrity-checked (a "verified" / "inconsistent" badge on the
replay card), and a live GPU preview with a scrubbable timeline (health
graph + miss markers) appears. Individual HUD elements can be toggled per
user preference — overrides persist across restarts and apply to every
render. Building the Windows installer: see
[docs/WINDOWS-BUILD.md](docs/WINDOWS-BUILD.md).

`testdata/` holds real replays verified against the game's own database
(`testdata_manifest.json`). Audio/cover files are third-party content and
are not part of the repository.

## Attribution

The `.rhr` wire format was verified against three independent sources:
the official `parseRhr` in the rhythia.com web bundle,
[yo-ru/rhrParse](https://github.com/yo-ru/rhrParse) (MIT) and
[gerhaarrd/rhr2mp4](https://github.com/gerhaarrd/rhr2mp4) (MIT), plus 311
local replays.

## License

MIT — see [LICENSE](LICENSE).
