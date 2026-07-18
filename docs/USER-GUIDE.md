# rhythr — User Guide

Turn your Rhythia replays (`.rhr`) into videos that look exactly like the
game — with your own skin, your HUD, your colors.

rhythr is an **unofficial community tool**, not affiliated with or
endorsed by Rhythia or Capo Games.

## Quick start

1. **Install**
   - *Windows*: run the setup exe. SmartScreen may warn about an
     "unknown publisher" because the installer is not code-signed;
     click "More info" → "Run anyway".
   - *Linux*: grab the `.AppImage`, make it executable
     (`chmod +x rhythr_*.AppImage`) and run it — everything, including
     ffmpeg, is inside. On Debian/Ubuntu/Mint you can install the
     `.deb` instead (`sudo apt install ./rhythr_*.deb`), on
     Fedora/openSUSE the `.rpm`, on Arch the AUR package `rhythr-bin`.
2. **Your game connects automatically** — on startup the app searches
   every Steam library (Windows and Linux, Proton or the native build)
   and reads the built-in skin textures and color sets directly from
   your own game. The **Game card** on the left shows "game connected"
   when it worked. If the game is installed somewhere unusual, click
   **Locate…** on that card and pick the game's executable
   (`rhythia.exe`, or the extensionless binary of the native Linux
   build). Without a connected game, built-in skins are only
   approximated.
3. **Export a replay from the game** — in Rhythia, go to the map you
   played, **right-click it and choose Export** to save the replay as a
   `.rhr` file.
4. **Export your skin from the game** — in the game's **Settings, click
   Export at the very top**. This saves your current skin/config as a
   `.rhs` file (look in `%APPDATA%\CapoRhythia\exports`). On Linux
   (Proton) that folder lives inside the game's Steam prefix:
   `~/.local/share/Steam/steamapps/compatdata/<appid>/pfx/drive_c/`
   `users/steamuser/AppData/Roaming/CapoRhythia/exports` — the numeric
   `<appid>` folder is the one that contains `pfx`. Exported replays
   land next to it in `CapoRhythia`.
5. **Drop both files into the app** — the map downloads automatically
   from rhythia.com (verified against the replay and cached), a live
   preview appears, and **Render video** does the rest. Done.

## The app, in detail

- **Preview & timeline** — drag the timeline to scrub through the run.
  The green graph is your health, red ticks are misses. **Save frame**
  exports the current preview as a PNG at full output resolution.
- **Edit HUD** — the switch next to *Save thumbnail* outlines every HUD
  element in the preview; drag any of them wherever you like (make room
  for a handcam, stack panels, overlap things — up to you). Positions
  save instantly and apply to every render; "Reset all to config" on
  the HUD tab puts everything back.
- **Vertical renders** — pick a vertical resolution (1080 × 1920) for
  TikTok/Shorts: the playfield fills the width and the stats move into
  rows above and below it. Rearrange them with Edit HUD as you like.
- **HUD tab** — toggle any HUD element (combo ring, accuracy, score,
  miss markers, …) on or off. Your choices are saved and apply to every
  future render; the yellow dot marks elements that differ from the
  skin config. "Reset all to config" clears every override.
- **Output tab** — resolution, frame rate, quality (CRF: lower = better,
  bigger file), results-screen length, output folder and file name.
- **Encoder** — "Auto" picks the fastest working encoder (NVENC on
  NVIDIA, Quick Sync on Intel, VAAPI, otherwise x264 software). If a
  hardware encoder is unavailable, the reason appears right under the
  selector — an outdated GPU driver is the most common cause.
- **Verified badge** — every replay is integrity-checked: the hits,
  misses and accuracy are re-derived from the raw inputs and compared
  against the file's header. "inconsistent — possibly modified" means
  the numbers don't add up.

## Troubleshooting

- **Antivirus complains** — the installer is new and unsigned, so
  reputation-based scanners are cautious. The file is open source and
  does nothing beyond rendering videos; add an exception for the
  install folder if needed. If rendered videos get blocked, allow the
  app in your AV's ransomware/folder protection.
- **"Map missing"** — the automatic download needs the map to exist on
  rhythia.com. For local/unpublished maps, use Browse next to Map and
  pick the `.sspm` file yourself.
- **Built-in skin looks slightly off** — check the Game card says
  "game connected"; click **Detect** after game updates to re-read the
  assets.
- **Linux: AppImage won't start** — some distros lack FUSE2. Install
  `libfuse2` (Ubuntu/Debian) / `fuse2` (Arch), or run the file with
  `./rhythr_*.AppImage --appimage-extract-and-run`.
- **Linux: "ffmpeg not found"** — the AppImage brings its own. The
  `.deb` installs the distro ffmpeg automatically; for the `.rpm` on
  Fedora, enable RPM Fusion and `sudo dnf install ffmpeg` (the stock
  "ffmpeg-free" lacks the x264 encoder).
- **Linux: blank window or crash on startup (Wayland)** — the app
  disables WebKitGTK's DMA-BUF renderer by itself; if you still hit
  issues, try `WEBKIT_DISABLE_COMPOSITING_MODE=1` and, as a last
  resort, `GDK_BACKEND=x11` to run through XWayland.
- **Updates** — Windows and the AppImage update themselves through the
  in-app banner. A deb/rpm install shows the banner too, but points
  you at the download page instead (package installs can't replace
  themselves).

## Fair play

This tool is strictly **read-only**. It cannot create or modify
replays — there is no code that writes `.rhr` files, and tampered
replays are flagged loudly. It renders what happened, nothing more.

The "verified" / "inconsistent" badge is rhythr's own consistency
check — it is **not** an official Rhythia score verification. Music,
beatmaps and artwork belong to their creators; you are responsible for
the rights needed to publish rendered videos.

## Licenses

rhythr is MIT-licensed. Source code:
https://github.com/KillerOp007/rhythr

Video encoding uses **ffmpeg** (bundled with the Windows installer and
the Linux AppImage, invoked as a separate program; native Linux
packages use the system ffmpeg). ffmpeg is licensed under the GPL — see
`ffmpeg-LICENSE.txt` next to the app and https://ffmpeg.org for
sources. Game assets are never bundled or redistributed: the optional
extraction reads them from *your own* game installation, locally, at
your request.
