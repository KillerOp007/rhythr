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
     Fedora/openSUSE the `.rpm`. On Arch-based distros (Arch, CachyOS,
     Manjaro, EndeavourOS, …) the AUR package `rhythr-bin` is the
     recommended install — native against your system libraries, which
     suits a rolling release better than the AppImage.
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
   Never exported anything? The game's own `config.json`, in that same
   `CapoRhythia` folder, loads just as well — it carries your settings,
   though only a `.rhs` can bring custom textures and a colorset along.
   Skip this step entirely and the render looks like a fresh install of
   the game: Cotton Candy colors, the full one-second approach, notes
   at standard size.
5. **Drop both files into the app** — the map downloads automatically
   from rhythia.com (verified against the replay and cached), a live
   preview appears, and **Render video** does the rest. Done.
   While dragging, the **Ghost race** and **Background** cards light
   up when you hover them: drop a replay on Ghost race to load it as
   the ghost, drop an image or video on Background (or anywhere) to
   make it the backdrop.

## The app, in detail

- **Preview & timeline** — drag the timeline to scrub through the run.
  The green graph is your health, red ticks are misses.
- **Save thumbnail** — saves a shareable score card of the run (cover
  art, grade, stats and mods) as a PNG. The dropdown picks the platform
  format: **Discord** (1200 × 630), **YouTube** (1280 × 720),
  **TikTok / Shorts** (1080 × 1920) or **Square** (1080 × 1080).
- **Edit HUD** — the switch next to *Save thumbnail* outlines every HUD
  element in the preview; drag any of them wherever you like (make room
  for a handcam, stack panels, overlap things — up to you). Drag the
  round **corner handle** on a box to resize the element (0.4x–2.5x;
  back to ~100% removes the override). The **Grid** dropdown overlays
  a snap grid in four densities — elements and meters snap to it
  while you drag, aligning whichever edge (or the centre) is closest
  to a line. In a ghost race the layout applies within each side's
  own half. Positions and sizes save automatically and the render
  always matches the preview. **Reset layout** next to the switch
  puts every element (meters included) back to its standard position
  and size; "Reset all to config" on the HUD tab resets everything.
- **Vertical renders** — pick a vertical resolution (1080 × 1920 or
  720 × 1280) for TikTok/Shorts: gameplay keeps its full width and the
  HUD moves into bands above and below it. The results screen re-lays
  out in portrait too, with a square cover. Rearrange the HUD with
  Edit HUD as you like.
- **Ghost races** — load a second replay of the same map (Ghost race
  card, or drop it onto the card) and the video splits into two
  side-by-side runs. The **racing delta** at the seam shows the live
  score gap with a tournament-style lead bar in the leader's colour;
  the results screen adds a race delta graph over the whole map.
  Both can be moved in Edit HUD or turned off under HUD → Ghost race.
- **Background** — an image or video shown behind the gameplay
  instead of the skin background (videos play muted and looped; any
  format your ffmpeg reads, animated GIFs included). The dim slider
  darkens only the backdrop so notes stay readable; the results
  screen keeps its own look.
- **HUD tab** — toggle any HUD element (combo ring, accuracy, score,
  miss markers, …) on or off. Your choices are saved and apply to every
  future render; the yellow dot marks elements that differ from the
  skin config. "Reset all to config" clears every override — it asks
  first, because it also discards every position and size you dragged.
- **Output tab** — resolution, frame rate, quality, results-screen length,
  output folder and file name. **Quality runs 0 to 100 and higher is
  better**; the line under the slider says what the setting is worth and
  what each encoder is actually being told, so switching encoder no longer
  quietly changes the result. 70 is the default and is already more than an
  upload needs — YouTube re-encodes everything it is given, so the top of
  the slider mostly buys file size. The
  file the next render will write is named under the render button, and
  if something is already there the app asks before overwriting it:
  **Replace**, or **Keep both**, which writes `name (2).mp4` alongside.
- **Encoder** — "Auto" picks the fastest working encoder (NVENC on
  NVIDIA, Quick Sync on Intel, AMF for AMD on Windows, VAAPI for AMD and
  Intel on Linux, otherwise x264 software). Each one is checked by actually
  encoding a moment of video with it, because a list of encoders says only
  what ffmpeg was built with, not what this machine's driver will do. If a
  hardware encoder is unavailable, the reason appears right under the
  selector — an outdated GPU driver is the most common cause. Renders
  finish quicker than they used to, a video background most of all.
- **Interface size** — the window's text and panels scale from 80% to
  160%: Ctrl+= larger, Ctrl+- smaller, Ctrl+0 back to 100%, or the
  slider under **Advanced** at the foot of the Output tab. It changes
  the interface only — the render itself is untouched — and the Analyze
  window follows the same setting.
- **Save diagnostics…** — also under Advanced, and worth attaching to a
  bug report: a plain text file with the build and operating system,
  which ffmpeg was found and whether it runs, each hardware encoder with
  ffmpeg's own reason when it is unavailable, what is loaded (including
  any integrity check that failed) and your output settings. You pick
  where it goes and can read it first; it deliberately leaves out the
  player name and any path the window does not already show, so sending
  it gives away no more than a screenshot would.
- **Verified badge** — every replay is integrity-checked: the hits,
  misses and accuracy are re-derived from the raw inputs and compared
  against the file's header. "inconsistent — possibly modified" means
  the numbers don't add up. "map may not match" is the milder case —
  the recorded hits fit no note on the chart that is loaded, which
  usually means it is not the chart that was played; load the right one
  before doubting the run.

## The Analyze window

Click **Analyze** above the preview to open the replay-forensics view.
The replay fills the window; the gear (or `O`) opens an options drawer.

- **Playback**: play/pause (Space), frame stepping (arrow keys),
  smooth slow motion from 0.01x to 4x. On Windows every frame renders
  live on the GPU — seeks are instant and there is nothing to buffer.
  The map's own music plays along, pitch-bent with the speed so you
  can find a spot by ear; the volume slider sits in the playbar.
- **Walking your misses** is what the window is for: `,` and `.`
  (or PageUp/PageDown, or the buttons next to the frame steps) move
  through them in chart order. Each jump lands 900 ms *before* the
  note, so you watch the approach rather than the aftermath, and `L`
  loops the miss you are on until you turn it off. Your overlay,
  linger and volume choices are remembered for next time.
- **Not sure what you are looking at?** The Overlays tab opens with a
  legend for every colour, box, dot and line on the picture, and the
  View tab lists every keyboard shortcut the window has.
- **Hitboxes** show the game's TRUE hit area (a fixed square, larger
  than the visual note — adjacent areas genuinely overlap, that is
  the game's own rule). At the hit plane the box freezes, and a dot
  marks exactly where your cursor was at the deciding moment: dot
  inside the box = hit, outside = miss (with a distance line). The box
  then lives out the run's own hit window — the game's 55 ms stretched
  by the speed you played at, so about 80 ms on a 1.45x run — and a
  note stays visible until the cursor actually takes it, so late hits
  read correctly even in slow motion.
- **Overlays**: cursor path (recorded aim, clamped to the field
  barrier like in the game), raw cursor cross, heatmap, and per-note
  inspection — click any note to see its timing and offset.
- **Tabs**: cursor statistics, timing histogram, miss list with
  closest calls (each entry jumps to the moment), integrity signals
  (calibrated against real leaderboard plays; tablet teleports and
  acceleration spikes on fast runs are expected, not accusations),
  and exports — analysis card, JSON/CSV, and an **overlay snapshot**
  (F8) that saves exactly what you see for sharing or bug reports.

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
- **Linux: AppImage won't start, or shows only a white window** —
  the AppImage runtime needs a `fusermount` binary and `/dev/fuse`:
  install your distro's `fuse3` package (`fuse` on some), or run the
  file with `./rhythr_*.AppImage --appimage-extract-and-run`. A white
  or empty window on a rolling release usually means the bundled
  libraries clash with your brand-new graphics stack. Either way, on
  Arch-based distros (Arch, CachyOS, Manjaro, …) prefer the AUR
  package `rhythr-bin` — a native install without FUSE, built against
  your own system libraries.
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
