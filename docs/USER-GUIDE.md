# rhythr — User Guide

Turn your Rhythia replays (`.rhr`) into videos that look exactly like the
game — with your own skin, your HUD, your colors.

## Quick start

1. **Install** — run the setup exe. Windows SmartScreen may warn about an
   "unknown publisher" because the installer is not code-signed; click
   "More info" → "Run anyway".
2. **Connect your game (recommended, once)** — open the app, go to
   **Output → Advanced → Detect game**. The app finds your Steam
   installation and reads the built-in skin textures and color sets
   directly from your own `rhythia.exe`. Without this step, built-in
   skins are only approximated. If Steam lives somewhere unusual, use
   **From rhythia.exe…** and pick the exe yourself.
3. **Export a replay from the game** — in Rhythia, go to the map you
   played, **right-click it and choose Export** to save the replay as a
   `.rhr` file.
4. **Export your skin from the game** — in the game's **Settings, click
   Export at the very top**. This saves your current skin/config as a
   `.rhs` file (look in `%APPDATA%\CapoRhythia\exports`).
5. **Drop both files into the app** — the map downloads automatically
   from rhythia.com (verified against the replay and cached), a live
   preview appears, and **Render video** does the rest. Done.

## The app, in detail

- **Preview & timeline** — drag the timeline to scrub through the run.
  The green graph is your health, red ticks are misses. **Save frame**
  exports the current preview as a PNG at full output resolution.
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
- **Built-in skin looks slightly off** — run **Detect game** (step 2).
  Re-run it after game updates.

## Fair play

This tool is strictly **read-only**. It cannot create or modify
replays — there is no code that writes `.rhr` files, and tampered
replays are flagged loudly. It renders what happened, nothing more.

## Licenses

rhythr is MIT-licensed. Source code:
https://github.com/KillerOp007/rhythr

Video encoding uses **ffmpeg** (bundled as `ffmpeg.exe`, invoked as a
separate program). ffmpeg is licensed under the GPL — see
`ffmpeg-LICENSE.txt` next to the app and https://ffmpeg.org for
sources. Game assets are never bundled or redistributed: the optional
extraction reads them from *your own* game installation, locally, at
your request.
