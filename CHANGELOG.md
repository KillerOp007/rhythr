# Changelog

rhythr is an unofficial community tool and is not affiliated with or
endorsed by Rhythia or Capo Games.

## v0.3.4 — 2026-07-18

### Added

- **Reset layout**: a button next to the *Edit HUD* switch puts every
  dragged HUD element — meters included — back to its standard
  position, without touching visibility or sizes ("Reset all to
  config" on the HUD tab still resets everything).

## v0.3.3 — 2026-07-18

Score cards, a drag-anywhere HUD editor, vertical renders for Shorts,
and the game now connects itself.

### Added

- **Score cards**: the *Save frame* button is now **Save thumbnail** —
  a shareable result card with cover, grade, stats and mods instead of
  a raw frame grab. A dropdown picks the platform format: **Discord**
  (1200×630), **YouTube** (1280×720), **TikTok/Shorts** (1080×1920) or
  **Square** (1080×1080), each with its own layout.
- **HUD editor**: flip the new **Edit HUD** switch and every HUD
  element gets a handle — drag it anywhere on the preview (overlapping
  is allowed), per side in ghost races. Positions save automatically
  and the render always matches the preview. *Reset HUD overrides*
  also restores the layout.
- **Vertical rendering**: new **1080×1920** and **720×1280** output
  sizes for Shorts/TikTok — gameplay keeps its full width and the HUD
  moves into bands above and below it. The results screen re-lays out
  in portrait too, with the cover kept exactly square.
- **The game connects itself.** A visible **Game** card on the main
  screen replaces the buried Advanced entry, and the app searches for
  the game on startup: every Steam library on every drive (Windows
  registry + defaults; native Linux, Flatpak and Snap), folder names
  matched case-insensitively, native Linux builds included — no more
  manual path picking.

### Fixed

- **Speed-mod renders no longer look too fast.** The game keeps the
  note approach constant in *real* time — at 1.45x there are simply
  more, tighter-packed notes in the air, approaching at the same
  on-screen pace. Renders compressed the approach along with the
  timeline, so notes flew in 45% faster than in-game. The approach now
  matches the game exactly at any speed.
- Replays that store wall-clock frame times (instead of song time) are
  detected and rescaled by checking the recorded hits against the
  map's note times, so a speed mod can never apply twice or get lost.
- The results screen shows the **difficulty** between the `< >`
  brackets (it repeated the map title).

## v0.3.2 — 2026-07-17

The custom-skin release: renders now match the game on ANY skin, not
just dark ones.

### Fixed

- **Colours blend exactly like the game's.** The game blends straight
  in sRGB; our renderer blended in linear light, which drifted on
  every semi-transparent pixel — worst on bright skins, where notes
  came out far too pale (a near-black note frame read 137/255 where
  the game shows 69). The whole pipeline now blends in sRGB space,
  measured to within 1-2/255 of real footage, and the HalfGhost fade
  was recalibrated to match.
- **The cursor trail uses its own texture.** Skins with a hollow
  cursor and a filled trail (`CursorTrailSkin`) rendered hollow trail
  rings; the trail now loads its configured image.
- **Stat panels sit, spread and lean like the game's.** Disabling
  panels collapsed the rest upward; the game pins the outer slots and
  spreads the enabled ones (a lone panel centres on the field).
  `PanelAngle` now fans the entries too.

### Changed

- On bright backgrounds the hit-error meters switch to near-black
  lines at strong opacity, and the aim grid's lines are thicker.

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
