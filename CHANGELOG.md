# Changelog

rhythr is an unofficial community tool and is not affiliated with or
endorsed by Rhythia or Capo Games.

## v0.3.1 — 2026-07-17

### Fixed

- **Speed mods play back at their real speed.** A 1.45x run rendered at
  1x with normal-pitch audio (matching the website's replay viewer, not
  the game). The video timeline now compresses by the replay's speed
  factor and the song is rate-shifted — faster and higher-pitched —
  exactly like the run felt in-game. Hit sounds follow along.
- The top bar shows the actual app version (it was stuck saying v0.2).

## v0.3.0 — 2026-07-17

The Linux release. Also: auto-updates, ghost races, game sounds, and a
round of pixel-parity upgrades pulled from the game's own files.

### Added

- **Linux packages**: a self-contained **AppImage** (ffmpeg included),
  a **.deb** for Debian/Ubuntu/Mint, an **.rpm** for Fedora/openSUSE
  and an **AUR package** (`rhythr-bin`) for Arch-based distros. Built
  against glibc 2.35, so they run on any mainstream distro from 2022
  onward (Ubuntu 22.04+, Debian 12+, Mint 21+, Fedora 36+, openSUSE
  Leap 15.6+, Arch, SteamOS 3.5+).
- **Detect game on Linux**: the game runs through Proton, and the app
  searches native, Flatpak and Snap Steam libraries and reads the
  built-in assets from the same `rhythia.exe` as on Windows.
- `.rhr` files open from Linux file managers (deb/rpm register the
  file type).
- **Auto-updates**: the app checks GitHub on startup and offers a
  one-click **Install & restart** (Windows installer and AppImage).
  Updates are cryptographically signed and verified against a key
  pinned in the app. deb/rpm installs get a download-page link
  instead — they update through the package manager.
- **Ghost races**: load a second replay of the same map and the video
  becomes a side-by-side **split screen** — each run with its own
  playfield, full HUD, stats and player name, the ghost's cursor and
  trail in a distinct colour, and both results in one frame under a
  shared map header. Also in the CLI (`--ghost-replay`). Each side
  plays on its own field: mirror and hardrock recorded in a replay
  apply to that side's notes. Speed mods must match (one timeline,
  one audio track); mismatches are rejected with a clear message.
- **Game hit/miss sounds** in rendered videos, at the exact registered
  hit times (the miss sound only when a combo of 5+ breaks, matching
  the game). Needs the extracted game assets.
- **Music volume** and **Hit sounds** sliders in the app,
  `--music-volume` / `--hitsound-volume` in the CLI.
- **Hit-error meters** (off by default, labelled as renderer extras):
  a timing bar plotting how late each hit was across the 0..+80 ms
  hit window with a gliding average marker, and an aim scatter showing
  the cursor's offset from each note's centre. Drag them anywhere in
  the preview; size and opacity are adjustable; in a ghost split each
  side positions its meters independently, with the timing bar's
  anchor and average marker in its player's colour.
- **Motion blur** (Off / Light / Strong, also `--motion-blur` in the
  CLI) at no extra render time.
- **Render-time estimate** in the Ready line, based on your last
  render's speed.
- Skins with custom **background layers** (`.rhs` with
  `BackgroundImages`) render their layered background art, respecting
  fit, placement, scale, flip and tint.
- Asset extraction also pulls the game's **shaders, hit/miss sounds,
  mod icons and UI fonts** (re-run *Detect game* to get them).
- The HUD renders with the game's **actual font** when assets are
  connected; the results screen shows the game's **real mod icons**.

### Changed

- The **fail vignette** uses the game's exact shader formula (smooth
  radial gradient, exact red) instead of an approximation.
- The **combo ring** follows the game's true rule: one side lost per
  miss, no decay over time.
- On Linux the app prefers the **distro's own ffmpeg** when installed
  (best hardware-encoder support); the AppImage additionally bundles
  its own copy, so it renders with no ffmpeg installed at all.
- With game assets connected, hit sounds default to **50%** in the
  app — set the slider to 0% to turn them off. The CLI defaults to
  off.

### Fixed

- **Mirror and hardrock replays rendered the unmodified field** — the
  notes now transform to what the player actually saw (mirror axis
  recovered from the run itself, hardrock's wider grid and border), in
  video, preview, frame export and CLI stills.
- **Blank window or crash on startup on many Linux/Wayland systems**:
  the app disables WebKitGTK's DMA-BUF renderer by itself.
- The progress clock no longer disappears when the title above it is
  hidden — it belongs to the progress bar.

### Notes

- Renders of the same replay can look slightly different than v0.2.1:
  the fail-vignette colour is now the game's exact red, and the combo
  ring no longer decays over time.
- Hit sounds, the game font and mod icons need the game assets — run
  *Advanced → Detect game* once (and re-run it after game updates).
- The update check runs once at startup and fails silently when
  offline; there is currently no setting to turn it off.
- The Chaos mod randomises note positions with a seed the replay does
  not store; it renders unmodified.
- Skin background layers render statically: layer rotation and
  camera-coupled movement (parallax/spin) are approximated.
- The .rpm does not pull in ffmpeg (Fedora's stock repos lack a
  package by that name) — install it via RPM Fusion, or use the
  AppImage.

## v0.2.1 — 2026-07-15

### Changed

- API usage and labelling per agreement with the Rhythia team:
  identifying User-Agent, backoff on 429/5xx, "unofficial community
  tool" labelling throughout, and a clarified verified badge (rhythr's
  own consistency check, not an official Rhythia score verification).

## v0.2.0 — 2026-07-15

### Added

- First public release as **rhythr**: Windows installer, desktop app
  with live preview and timeline, automatic map download with caching
  and hash verification, replay integrity check ("verified" badge),
  skin support from exported `.rhs` configs, built-in asset extraction
  from your own game install, HUD element overrides, results screen,
  hardware-encoder auto-pick (NVENC / Quick Sync / VAAPI, x264
  fallback), `.rhr` file association and a CLI.
