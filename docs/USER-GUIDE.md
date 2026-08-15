# rhythr User Guide

Turn your Rhythia replays (`.rhr`) into videos that look exactly like the
game, with your own skin, your HUD, your colors.

rhythr is an **unofficial community tool**, not affiliated with or
endorsed by Rhythia or Capo Games.

## Quick start

1. **Install**
   - *Windows*: run the setup exe. SmartScreen may warn about an
     "unknown publisher" because the installer is not code-signed;
     click "More info" → "Run anyway".
   - *Linux*: grab the `.AppImage`, make it executable
     (`chmod +x rhythr_*.AppImage`) and run it. Everything, including
     ffmpeg, is inside. On Debian/Ubuntu/Mint you can install the
     `.deb` instead (`sudo apt install ./rhythr_*.deb`), on
     Fedora/openSUSE the `.rpm`. On Arch-based distros (Arch, CachyOS,
     Manjaro, EndeavourOS, …) the AUR package `rhythr-bin` is the
     recommended install: native against your system libraries, which
     suits a rolling release better than the AppImage.
2. **Your game connects automatically**: on startup the app searches
   every Steam library (Windows and Linux, Proton or the native build)
   and reads the built-in skin textures and color sets directly from
   your own game. The **Game card** on the left shows "game connected"
   when it worked. If the game is installed somewhere unusual, click
   **Locate…** on that card and pick the game's executable
   (`rhythia.exe`, or the extensionless binary of the native Linux
   build). Without a connected game, built-in skins are only
   approximated.
3. **Export a replay from the game**: in Rhythia, go to the map you
   played, **right-click it and choose Export** to save the replay as a
   `.rhr` file.
4. **Export your skin from the game**: in the game's **Settings, click
   Export at the very top**. This saves your current skin/config as a
   `.rhs` file (look in `%APPDATA%\CapoRhythia\exports`). On Linux
   (Proton) that folder lives inside the game's Steam prefix:
   `~/.local/share/Steam/steamapps/compatdata/<appid>/pfx/drive_c/`
   `users/steamuser/AppData/Roaming/CapoRhythia/exports`. The numeric
   `<appid>` folder is the one that contains `pfx`. Exported replays
   land next to it in `CapoRhythia`.
   Never exported anything? The game's own `config.json`, in that same
   `CapoRhythia` folder, loads just as well: it carries your settings,
   though only a `.rhs` can bring custom textures and a colorset along.
   Skip this step entirely and the render looks like a fresh install of
   the game: Cotton Candy colors, the full one-second approach, notes
   at standard size.
5. **Drop both files into the app**: the map downloads automatically
   from rhythia.com (verified against the replay and cached), a live
   preview appears, and **Render video** does the rest. Done.
   While dragging, the **Ghost race** and **Background** cards light
   up when you hover them: drop a replay on Ghost race to load it as
   the ghost, drop an image or video on Background (or anywhere) to
   make it the backdrop.

## The app, in detail

- **Preview & timeline**: drag the timeline to scrub through the run.
  The green graph is your health, red ticks are misses.
- **Clip ranges**: render a slice instead of the whole run by dragging the
  handles or typing the bounds. **Save range** keeps the current one under
  a name, so marking a second range no longer destroys the first and
  neither dies with the session. Saved ranges belong to their replay and
  reappear whenever it is loaded; click one to make it current, ✕ to
  forget it. This is not a queue: nothing renders on its own, and it is
  still one Render button pointed at whichever range is current. What it
  saves you is holding the timestamps of a montage in your head while you
  render them one at a time.
- **Save thumbnail**: saves a shareable score card of the run (cover
  art, grade, stats and mods) as a PNG. The dropdown picks the platform
  format: **Discord** (1200 × 630), **YouTube** (1280 × 720),
  **TikTok / Shorts** (1080 × 1920) or **Square** (1080 × 1080).
- **Edit HUD**: the switch next to *Save thumbnail* outlines every HUD
  element in the preview; drag any of them wherever you like (make room
  for a handcam, stack panels, overlap things; it is up to you). Drag the
  round **corner handle** on a box to resize the element (0.4x-2.5x;
  back to ~100% removes the override). The **Grid** dropdown overlays
  a snap grid in four densities. Elements and meters snap to it
  while you drag, aligning whichever edge (or the centre) is closest
  to a line. In a ghost race the layout applies within each side's
  own half. Positions and sizes save automatically and the render
  always matches the preview. **Reset layout** next to the switch
  puts every element (meters included) back to its standard position
  and size; "Reset all to config" on the HUD tab resets everything.
- **Vertical renders**: pick a vertical resolution (1080 × 1920 or
  720 × 1280) for TikTok/Shorts: gameplay keeps its full width and the
  HUD moves into bands above and below it. The results screen re-lays
  out in portrait too, with a square cover. Rearrange the HUD with
  Edit HUD as you like.
- **Ghost races**: load a second replay of the same map (Ghost race
  card, or drop it onto the card) and the video splits into two
  side-by-side runs. The **racing delta** at the seam shows the live
  score gap with a tournament-style lead bar in the leader's colour;
  the results screen adds a race delta graph over the whole map.
  Both can be moved in Edit HUD or turned off under HUD → Ghost race.
- **Background**: an image or video shown behind the gameplay
  instead of the skin background (videos play muted and looped; any
  format your ffmpeg reads, animated GIFs included). The dim slider
  darkens only the backdrop so notes stay readable; the results
  screen keeps its own look.
- **HUD tab**: toggle any HUD element (combo ring, accuracy, score,
  miss markers, …) on or off. Your choices are saved and apply to every
  future render; the yellow dot marks elements that differ from the
  skin config. "Reset all to config" clears every override. It asks
  first, because it also discards every position and size you dragged.
- **Output tab**: resolution, frame rate, quality, results-screen length,
  output folder and file name. **Quality runs 0 to 100 and higher is
  better**; the line under the slider says what the setting is worth and
  what each encoder is actually being told, so switching encoder no longer
  quietly changes the result. 70 is the default and is already more than an
  upload needs: YouTube re-encodes everything it is given, so the top of
  the slider mostly buys file size. The
  file the next render will write is named under the render button, and
  if something is already there the app asks before overwriting it:
  **Replace**, or **Keep both**, which writes `name (2).mp4` alongside.
- **Soundtrack**: renders with a file you choose instead of the map's own
  audio. Some map files store only the song's *name* rather than the song,
  and those used to render silent with nothing saying why; the map card now
  says so and this is the fix. Leave "Keep this track at its own speed"
  unticked when the file is the map's song, so it follows a speed mod the
  way the game does. Tick it when the track is a replacement soundtrack
  that should play unaltered.
- **Target file size**: aims the encoder at a size instead of a quality
  level, for anywhere with an upload limit. Frames reach the encoder in a
  single pass and cannot be re-encoded, so it lands *near* the target
  rather than exactly on it, and it deliberately aims a little under: a
  10 MB target typically writes 8 to 9 MB. It overrides the Quality
  slider while it is set. If the clip is too long for the size you asked
  for, the render still happens at the Quality setting and the finished
  line says so rather than quietly writing a bigger file. AMF (AMD on
  Windows) cannot be aimed at a size; choose x264 there.
- **Speed-mod pitch**: what a speed mod does to the song, mirroring the
  Pitch Lock toggle the game added to its speed selector in Release 41.
  Replays from that release on record the player's choice, and "Follow
  the replay" plays what they heard; older replays were always pitched
  (faster and higher, nightcore-style) and render that way. The other
  two settings override the replay in either direction.
- **Playback speed**: how fast the video plays the run back. 0.25x
  stretches it over four times the length, which is the way to get the
  slow motion the Analyze window shows you out as a video. The music is
  slowed with it. Slow motion draws more frames from the same run, so it
  takes proportionally longer to render. The run's own speed mod is a
  separate thing and is always applied on top of this.
- **Encoder**: "Auto" picks the fastest working encoder (NVENC on
  NVIDIA, Quick Sync on Intel, AMF for AMD on Windows, VAAPI for AMD and
  Intel on Linux, otherwise x264 software). Each one is checked by actually
  encoding a moment of video with it, because a list of encoders says only
  what ffmpeg was built with, not what this machine's driver will do. If a
  hardware encoder is unavailable, the reason appears right under the
  selector. An outdated GPU driver is the most common cause. Renders
  finish quicker than they used to, a video background most of all.
- **Interface size**: the window's text and panels scale from 80% to
  160%. Ctrl+= larger, Ctrl+- smaller, Ctrl+0 back to 100%, or the
  slider under **Advanced** at the foot of the Output tab. It changes
  the interface only (the render itself is untouched), and the Analyze
  window follows the same setting.
- **Save diagnostics…** (also under Advanced) is worth attaching to a
  bug report: a plain text file with the build and operating system,
  which ffmpeg was found and whether it runs, each hardware encoder with
  ffmpeg's own reason when it is unavailable, what is loaded (including
  any integrity check that failed) and your output settings. You pick
  where it goes and can read it first; it deliberately leaves out the
  player name and any path the window does not already show, so sending
  it gives away no more than a screenshot would.
- **Hit sound choice**: under the Hit sounds slider, pick the game's
  default or one of its four alternative packs (Bass, Creamy, Pick,
  Vulnus), or choose your own audio file: anything your ffmpeg reads is
  decoded on the fly. A choice that cannot be honoured falls back to the
  default sound rather than rendering silently.
- **Mesh note skins**: the game's four built-in note meshes (cube,
  circle, plane, rounded thin) and custom `.obj` note skins bundled in an
  imported `.rhs` render as real 3D geometry, tinted by the colorset. A
  broken or missing mesh falls back to the regular note look rather than
  failing the render: notably, rhythr does not care when a skin's `.mtl`
  file was not bundled, which the game itself currently crashes on.
- **Verified badge**: every replay is integrity-checked. The hits,
  misses and accuracy are re-derived from the raw inputs and compared
  against the file's header. "inconsistent (possibly modified)" means
  the numbers don't add up. "map may not match" is the milder case:
  the recorded hits fit no note on the chart that is loaded, which
  usually means it is not the chart that was played; load the right one
  before doubting the run.

## The Analyze window

Click **Analyze** above the preview to open the replay-forensics view.
The replay fills the window; the gear (or `O`) opens an options drawer.

- **Playback**: play/pause (Space), frame stepping (arrow keys),
  smooth slow motion from 0.01x to 4x. On Windows every frame renders
  live on the GPU: seeks are instant and there is nothing to buffer.
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
- **Why a note was missed**: the Misses tab splits them into timing, aim
  and lost, measured against the game's own hit area rather than against
  the distance to the note's middle. That matters more than it sounds: the
  hit area is a *square*, so a cursor 0.6 cells away straight up is
  outside it while 0.6 away diagonally is comfortably inside. A timing
  miss means the cursor did cover the note and simply was not there while
  it was live, and the summary says by how much, on average, and in which
  direction. The dotted line on the picture now runs to the nearest edge
  of the hit area, and disappears entirely when the cursor was inside it.
- **Cutting a clip from here**: `I` marks the start of the render's clip
  at the playhead and `Shift + O` marks the end, with buttons for both in
  the playbar. "Clip this miss" takes the same window the miss loop uses,
  so what you were just watching is what gets rendered. The range is drawn
  on the scrub bar and the main window picks it up immediately. Before
  this the clip could only be typed into the main window's still-frame
  scrubber, so finding a moment here meant memorising a timestamp and
  re-entering it there.
- **Filling the screen**: `F11` goes fullscreen, and the window now
  reopens at the size and position you left it at rather than at
  1280x800 every time.
- **Hitboxes** show the game's TRUE hit area (a fixed square, larger
  than the visual note; adjacent areas genuinely overlap, that is
  the game's own rule). At the hit plane the box freezes, and a dot
  marks exactly where your cursor was at the deciding moment: dot
  inside the box = hit, outside = miss (with a distance line). The box
  then lives out the run's own hit window (the game's 55 ms stretched
  by the speed you played at, so about 80 ms on a 1.45x run), and a
  note stays visible until the cursor actually takes it, so late hits
  read correctly even in slow motion.
- **Overlays**: cursor path (recorded aim, clamped to the field
  barrier like in the game), raw cursor cross, heatmap, and per-note
  inspection: click any note to see its timing and offset.
- **Tabs**: cursor statistics, timing histogram, miss list with
  closest calls (each entry jumps to the moment), integrity signals
  (calibrated against real leaderboard plays; tablet teleports and
  acceleration spikes on fast runs are expected, not accusations),
  and exports (analysis card, JSON/CSV, and an **overlay snapshot**
  with F8 that saves exactly what you see for sharing or bug reports).

## Troubleshooting

### Reporting a problem

Every error ends with a code in brackets, like `RH-FFM-101-L`. Quote it.
The letters say which part failed (encoding, graphics, writing the file,
replays and maps, downloads, the app), the number says which failure,
and the last letter is your system: `W`indows, `L`inux, `M`acOS. The
same failure has a different cause on each, which is why it is part of
the code.

For anything that is not obvious from the message, open **Advanced** and
use **Diagnostics**: **Save to file** writes a text report, **Copy**
puts the same thing on your clipboard. It contains the app version,
your system and processor, your GPU and driver, which ffmpeg was found
and which encoders it offers, what is loaded, your last render's
timings, the settings you changed from default, and the codes of
everything that went wrong this session. Your home folder is written as
`~`, so it carries no account name, and nothing is sent anywhere: you
choose where the file goes and can read it first.

- **Antivirus complains**: the installer is new and unsigned, so
  reputation-based scanners are cautious. The file is open source and
  does nothing beyond rendering videos; add an exception for the
  install folder if needed. If rendered videos get blocked, allow the
  app in your AV's ransomware/folder protection.
- **"Map missing"**: the automatic download needs the map to exist on
  rhythia.com. For local/unpublished maps, use Browse next to Map and
  pick the `.sspm` file yourself.
- **Built-in skin looks slightly off**: check the Game card says
  "game connected"; click **Detect** after game updates to re-read the
  assets.
- **Linux: AppImage won't start, or shows only a white window**.
  The AppImage runtime needs a `fusermount` binary and `/dev/fuse`:
  install your distro's `fuse3` package (`fuse` on some), or run the
  file with `./rhythr_*.AppImage --appimage-extract-and-run`. A white
  or empty window on a rolling release usually means the bundled
  libraries clash with your brand-new graphics stack. Either way, on
  Arch-based distros (Arch, CachyOS, Manjaro, …) prefer the AUR
  package `rhythr-bin`: a native install without FUSE, built against
  your own system libraries.
- **Linux: "ffmpeg not found"**. The AppImage brings its own. The
  `.deb` installs the distro ffmpeg automatically; for the `.rpm` on
  Fedora, enable RPM Fusion and `sudo dnf install ffmpeg` (the stock
  "ffmpeg-free" lacks the x264 encoder).
- **Linux: blank window or crash on startup (Wayland)**. The app
  disables WebKitGTK's DMA-BUF renderer by itself; if you still hit
  issues, try `WEBKIT_DISABLE_COMPOSITING_MODE=1` and, as a last
  resort, `GDK_BACKEND=x11` to run through XWayland.
- **Updates**: Windows and the AppImage update themselves through the
  in-app banner. A deb/rpm install shows the banner too, but points
  you at the download page instead (package installs can't replace
  themselves).

## Fair play

This tool is strictly **read-only**. It cannot create or modify
replays: there is no code that writes `.rhr` files, and tampered
replays are flagged loudly. It renders what happened, nothing more.

The "verified" / "inconsistent" badge is rhythr's own consistency
check. It is **not** an official Rhythia score verification. Music,
beatmaps and artwork belong to their creators; you are responsible for
the rights needed to publish rendered videos.

## Licenses

rhythr is MIT-licensed. Source code:
https://github.com/KillerOp007/rhythr

Video encoding uses **ffmpeg** (bundled with the Windows installer and
the Linux AppImage, invoked as a separate program; native Linux
packages use the system ffmpeg). ffmpeg is licensed under the GPL. See
`ffmpeg-LICENSE.txt` next to the app and https://ffmpeg.org for
sources. Game assets are never bundled or redistributed: the optional
extraction reads them from *your own* game installation, locally, at
your request.
