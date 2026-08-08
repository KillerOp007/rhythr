# Changelog

rhythr is an unofficial community tool and is not affiliated with or
endorsed by Rhythia or Capo Games.

## Unreleased

The default render is measured against the game instead of approximating it,
the hit window follows the run's speed the way the game does, renders got
substantially faster, and a round of quality-of-life work on top.

### What changes in your renders

Read this bit even if you skip the rest. The three things below will make
your videos different from the ones you made on 0.5.0, on purpose.

**If you render without loading a skin config, everything about the default
look changed.** It was assembled from footage and guesswork; it is now read
out of the game's own source. The camera sits further back, notes approach
from further away and stay visible about twice as long, they are rounded
rather than square, the palette is the game's own Cotton Candy instead of an
invented one, the border is thin and sharp instead of a thick rounded box,
the camera drifts toward your cursor rather than away from it, and the cursor
trail is off because the game ships it off. A render you made before will not
line up with one you make now.

Nothing changes if you load your own config: your settings win, as they
always did, and that is the way to get exactly what you had.

**Miss markers appear where the game puts them.** The hit window was a flat
80 ms and is now 55 ms scaled by the run's speed, which is what the game
does. On a 1.00x run rhythr used to accept hits up to 25 ms after the game
had already scored a miss, so a handful of notes near the edge can flip
between hit and miss compared to an older render. The new behaviour is the
one that matches your score.

**And the quality number runs the other way round.** 0 to 100, higher is
better, where it used to be a CRF where lower was better. Your saved setting
is converted on upgrade, so your first render after updating produces the
file it would have produced before, but the slider will be sitting somewhere
new, and the default for a fresh install is deliberately lower than the old
one, because the old one wrote several times what an upload needs.

### Faster

- **Renders are 30-40% faster**, and more on the formats that were worst off.
  The bottleneck turned out to be neither the GPU nor the encoder: it was
  pushing 8 MB of uncompressed RGBA per frame (33 MB at 4K) down a pipe. With
  ffmpeg fed NV12 instead (1.5 bytes per pixel rather than 4), the same
  picture crosses in 38% of the bytes and ffmpeg no longer converts it on the
  far side either.

  | | before | after |
  |---|---|---|
  | 1080p, hardware encoder | 8.34 s | **5.63 s** |
  | 1080p, x264 | 9.71 s | 7.91 s |
  | vertical 1080×1920 | 10.34 s | 7.24 s |
  | 1080p with a video background | 14.15 s | 11.66 s |

  (15 s of video on a Ryzen 5 with integrated graphics.) The colour space is
  not a guess: ffmpeg was asked what it had been producing all along, which
  is BT.601 limited range, and the result was checked against its own output,
  against the previous path on real gameplay, and against an uncompressed
  still from the renderer. The new path is marginally closer to that still
  than the old one was.

- **Frames go to ffmpeg in 16 KiB pieces** instead of one write per frame:
  8.6 ms down to 6.2 ms for a 4K frame. One large write fills the pipe and
  then waits for the reader to drain it, so the two processes take turns.
  Enlarging the pipe itself was measured too and did not survive it (7.40 ms
  against 7.51 ms, inside the spread), so it is not here. There is also a
  loopback-socket transport, now the default, which is worth **160 fps to
  210 fps at 3840x2160/240** on a machine whose encoder is not the
  bottleneck. It is nothing at all where the encoder is (which is why it
  took a second machine to settle), and a socket can fail to connect where a
  pipe cannot, so the whole path is walked once with a small frame before any
  render depends on it, and falls back to the pipe on its own if it does not
  hold up. The switch is still in Advanced for turning it off.

  A pipe and a socket also want opposite things from a write, which was not
  obvious: a pipe is fastest fed in 16 KiB pieces and a socket is fastest
  given the whole frame at once (4.25 ms against 6.14 ms at 4K). Chunking
  both, as the first version did, left a third of the socket's advantage
  unclaimed.

- **The colour conversion moved onto the GPU**, which is what was left once
  the pipe stopped being the limit. Timing each stage separately at
  3840x2160/120 showed the GPU building a frame in 0.64 ms and the CPU then
  spending 10.84 ms converting it. The card idled 97% of the time, and
  swapping encoders, down to x264 ultrafast, changed nothing because the
  encoder was never the bottleneck. A compute pass now converts before the
  readback, so a frame leaves VRAM already in the encoder's format and the
  bus carries 1.5 bytes per pixel instead of 4.

  | | before | after |
  |---|---|---|
  | 4K, 120 fps | 23.7 s | **18.5 s** |
  | 1080p, 60 fps | 4.0 s | **3.0 s** |

  The picture does not change: the shader is a transcription of the CPU
  converter down to the integer coefficients and the rounding, a test
  compares both outputs byte for byte, and a 960-frame 4K render through each
  path decodes to the same checksum. Frame sizes the shader cannot address
  fall back to the CPU converter, and `RHYTHR_NO_GPU_NV12=1` forces that
  fallback.

- **The Output tab's speed preset reaches the hardware encoders.** It only
  ever configured x264; NVENC ran a fixed `p5` near the slow end of its own
  scale and QuickSync got no preset at all, so on hardware encoding the
  control did nothing. NVENC is also no longer asked for a pixel format it
  has to convert to first.

- **A video background costs a third of what it did** (20.1 s → 11.7 s on the
  same clip). It was decoded in the middle of the render loop, blocking on a
  pipe for a whole frame before the GPU got any work; it now runs on its own
  thread. It also copied every frame an extra time for no reason, and (worst
  of it) a 30 fps background under a 60 fps render made ffmpeg duplicate
  every frame and pushed both copies through the pipe and up to the GPU. At
  4K/120 over a 30 fps background that was four copies of 33 MB per picture.

- The Analyze window stopped recomputing a still picture forty times a second
  while paused: it was rebuilding the screen geometry of every note in the
  map, 3400 of them on a dense chart, describing something that was not
  moving.

### Changed

- **Quality runs the other way round now: 0 to 100, higher is better.** The
  number used to be an x264 CRF, where lower meant better (correct for CRF,
  and wrong for what a slider promises, since people drag it right for more
  and got less). It also meant four different things at once: the same value
  went to libx264 as `-crf`, nvenc as `-cq`, QuickSync as `-global_quality`
  and VAAPI as `-qp`, which are a rate factor, a target hint and two flat
  quantisers on incompatible scales, so changing encoder silently changed
  the output while the number on screen stood still. One scale now maps onto
  each of them, and the line under the slider says both what the setting is
  worth and what the selected encoder is actually being told. The mapping is
  approximate and says so: there is no published equivalence between these
  scales, which is exactly why the resolved value is shown rather than
  hidden.

  The default is now 70, x264 CRF 20. At 4K60 the old bottom of the slider
  produced several times what YouTube documents for the format and
  re-encodes away regardless. On a ten-second 1080p60 clip: 4.8 MB at
  quality 100, **2.9 MB at the new default**. Settings and scripts written
  before this are converted, so a saved value still means the encode it
  always meant. `--crf` still works on the command line and is converted
  the same way.

- **AMD gets hardware encoding on Windows.** Auto-selection went NVENC →
  QuickSync → VAAPI → software, and VAAPI looks for `/dev/dri/renderD128`,
  a path Windows does not have, so every AMD machine on Windows rendered on
  the CPU. AMF is now in the list. The encoder dropdown also stops calling
  VAAPI "AMD/Intel" without saying that only holds on Linux.

- Hardware encoders are checked by running **the same arguments a render
  will use**, instead of a bare codec name. An option a driver rejects now
  makes that encoder unavailable and auto-selection moves past it, rather
  than surfacing as a failed encode after someone has waited through a
  render. It matters: `av1_vaapi` is listed on the machine this was written
  on and fails with "No usable encoding profile found" the moment it is
  asked to do anything.

- VAAPI asks for intelligent-constant-quality where the driver has it and
  falls back to a flat quantiser where it does not. Also not hypothetical:
  the driver here answers "does not support ICQ RC mode (supported modes:
  CQP, CBR, VBR)". The DRM render node is enumerated rather than assumed,
  which is what a hybrid laptop needs, since its first node is often the
  display-only chip.

### Fixed

- **The hit window is no longer a constant.** The game misses a note once
  `ms > note_t + hit_window`, and that window is 55 ms scaled by the speed
  multiplier. Its own settings screen documents it ("[def. 55ms] … 1.5x =
  83; 2x = 110"). rhythr carried a flat 80 ms, which is exactly the 1.45×
  case generalised to every run: on a 1.00× replay it accepted hits 25 ms
  past the point the game had already scored a miss, and the miss X came up
  just as late. Measured across 37 leaderboard replays covering every mod
  combination: at 55 × speed every recorded flag finds a note and the latest
  one lands just under the window at each speed, never above.

- **The playfield is the size the game draws it.** The camera sat too close,
  making the field about 6% larger than the game's. The game's own source
  says the camera is parked at 3.75 units, but a screenshot says otherwise
  and the screenshot wins: the border plane is 3.04 units and its texture
  covers 99.6% of it, so at that distance the border could only ever project
  to 834 px at 1440p, and the game's measures 876. At 3.5 rhythr now
  measures 882, within 0.7% of the game.

- **The default look now comes from the game.** `SkinConfig::default()` was
  documented as the game's defaults but reproduced one player's exported
  config value for value. Every value is now the game's own, cross-checked
  against both its source and the "[def. X]" tooltips on its settings page:

  | | was | now |
  |---|---|---|
  | Camera distance | 3.25 | **3.75** (the playfield rendered 15% oversized) |
  | Approach rate | 24.5 | **40** (notes were visible 0.49 s instead of 1.0 s) |
  | Spawn distance | 12 | **40** |
  | Note scale | 0.9 of 0.5 | **1.0 of 0.45** (an imported NoteScale was 11% too big) |
  | Note shape | square | **rounded**, the mesh a fresh install selects |
  | Parallax | 0, swaying away from the cursor at 0.003/unit | **6.5, toward it at 0.025/unit** |
  | Cursor trail | on | **off** |
  | Colorset | an invented four-hue palette | **Cotton Candy** (#00ffed / #ff8ff9) |
  | Playfield border | 5× too thick, rounded corners, opaque | **0.0152 units, sharp corners, 58.8% opacity** |
  | HalfGhost floor | 0.26 | **0.20** |

- **Ghost and nearsighted runs render as they were played.** The replay's
  visibility mods were ignored, so a run played half blind came out as a
  comfortable read of itself.
- The cursor trail was a third too short under a speed mod: its lifetime is
  real seconds in the game but was being measured in song time.
- Cover art is no longer minified into aliasing on the results screen and the
  score card, and the grade letter follows the HUD's opacity like everything
  around it.
- The song-info header is fitted to the frame. A long name and a long song
  ran off the edge, and on a split-screen race into the other player's half.
- **ffmpeg failures say what happened.** Its stderr was inherited, which in a
  windowed app means discarded, so every encode failure read "exited with
  exit status: 1". ffmpeg is also probed before a render instead of x264
  being advertised regardless, and a corrected path re-probes.
- **A mismatched map no longer accuses your replay.** Loading the wrong chart
  reported "inconsistent, possibly modified"; it now says the map may not
  match, and only when the frames actually back the header (an inflated
  header still reads as inconsistent, because that is what it is).
- **A GPU that will not start says what to try**, and the live path honours
  `WGPU_BACKEND` so there is something to try.
- **Rendering twice no longer overwrites the first video** without asking.
- **Your own `config.json` loads.** The backend always accepted it; only the
  file dialog and drop handler insisted on `.rhs`.
- **A crafted `.sspm` can no longer take the process down.** Nested marker
  arrays recursed without a depth limit.
- A panic while rendering an Analyze segment left the window waiting on a
  progress pill forever; the startup sweep of leftover files deleted the
  segments a second running instance was playing from, and ran before the
  window was usable.
- **The analyze window's keyboard survives a slider.** One click on the
  volume or speed control used to swallow space and the arrow keys for the
  rest of the session. Focused checkboxes and radios keep their own keys.
- The advertised 0.01× slow motion now exists (the bottom sixteenth of the
  speed slider did nothing).
- The playbar wraps instead of being cut off at the window's own minimum
  width.
- Scrubbing resumes playback if it was playing; window geometry, and the
  analyze window's overlay and playback settings, are remembered; deleting a
  preset and resetting the HUD ask first; `settings.json` is written
  atomically and a broken one is kept instead of silently replaced.
- **The hit-sound volume slider works over its whole range.** Anything above
  100% was flattened back to 100%, so the top third of the control did
  nothing; it now boosts up to 150% the way the music slider always has.

### Added

- **Walk your misses.** `,` and `.` (or PageUp/PageDown, or the buttons in
  the transport) step through them in chart order, landing 900 ms before the
  note so you see the approach rather than the aftermath. `L` loops the one
  you are on.
- **An overlay legend**, and a list of every shortcut, in the Analyze window.
- **An interface scale**, 80% to 160%, on Ctrl+= / Ctrl+- / Ctrl+0 or a
  slider under Advanced.
- **A diagnostics file** under Advanced: build, OS and version, processor,
  your GPU with its backend and driver, which ffmpeg was resolved and whether
  it runs, each hardware encoder with ffmpeg's own reason when it does not,
  what is loaded, the output settings, the timings of your last render, and
  the settings you changed from a fresh install. There is a Copy button next
  to it if you would rather paste than attach. You choose where it goes and
  can read it first; your home folder is written as `~` throughout, so the
  file carries no account name.
- **Error codes.** Anything that goes wrong now ends with a code like
  `RH-FFM-101-L`: the area, the failure, and the system it happened on (the
  same failure has a different cause on Windows and on Linux). Quote the code
  in a bug report and it can be looked up directly instead of guessed at. The
  codes are also listed at the top of the diagnostics file, with how often
  each one came up.
- The render target's file name is shown before the render, not after.
- The empty preview panel can be clicked, not just dropped onto.
- Optional source cards fold away, and open themselves when they hold
  something.
- Focus is visible on every control, and the preset and recent lists can be
  reached by keyboard.

### Not done, and why

- Handing frames to a writer thread, and running more frames in flight: both
  built, both measured, both slower or flat, both reverted. There is nothing
  to overlap with: the render side of a frame is done in 0.6 ms and the GPU
  wait is 0.02 ms.
- Skipping the asset bytes when the ghost side's config is cloned each frame:
  the struct-update syntax it used evaluates the whole clone first, so it did
  strictly more work than the plain clone it replaced. Removed rather than
  patched with a field list that would silently drop the next field added.
- Shadows behind the HUD text on bright backgrounds: the game does not have
  them, and this release is about matching the game.
- Mipmaps for the glyph atlas: inspected at 4× magnification and there are no
  visible artefacts to fix.

## v0.5.0 (2026-08-01)

The replay analyzer: a full forensics view for any run, plus a layout
editor that grows presets, clip export and real undo.

### Added

- **Analyze window**: a dedicated pop-out for studying a replay.
  The picture fills the window; a gear opens an options drawer with
  overlays, cursor stats, timing, misses, per-note inspection,
  integrity signals, exports and view settings.
- **Live GPU playback** (Windows): every displayed frame is rendered
  on the fly against a virtual clock (no buffering, instant seeks,
  and slow motion from 0.01x to 4x stays butter smooth at full frame
  rate). Linux keeps the proven pre-rendered playback engines.
- **Song audio**: the map's own music plays during analysis,
  rate-locked to the clock. Slowing down bends the pitch like a
  record, so you can find a spot by ear. Volume lives in the playbar
  (default 20%).
- **True hit areas**: hitboxes show the game's real hit square
  (1.1375 cells, straight from the game source), follow each note in,
  and FREEZE at the hit plane (the one place where cursor vs. box is
  geometrically honest). A verdict dot marks exactly where the cursor
  was at the deciding moment (inside = hit, outside = miss, with a
  distance line on misses), and a note stays visible until the cursor
  actually takes it. Box linger is configurable (0-1 s).
- **Cursor-guided hit attribution**: near-simultaneous double notes
  used to get their verdicts crossed when matched by time alone; the
  recorded cursor now decides which note a hit belongs to. Totals
  always match the game exactly. Validated against 37 real
  leaderboard replays (all mods, top to bottom) plus reference runs.
- **Integrity signals, recalibrated on the real population**:
  acceleration spikes and tablet teleports no longer flag legitimate
  plays; incomplete recordings (the game's recorder drops frames
  under load) are reported honestly instead of as "corrupted".
- **Overlay snapshot** (Export tab / F8): saves exactly what you see
  (picture plus overlays) as one PNG. Ideal for bug reports.
- **Layout presets**: save and apply complete looks (layout, sizes,
  meters, skin config, resolution, background incl. placement); an
  automatic "Before reset" preset makes Reset layout reversible.
- **Clip export**: Set start/end under the timeline (or drag the
  handles) to render just a section, with suggestions from the run
  (best streak, toughest part, finish). Score/combo/accuracy are
  exact from the clip's first frame.
- **Real undo/redo** in the HUD editor (Ctrl+Z / Ctrl+Y, 50 steps)
  and live preview while dragging.
- **Background placement**: zoom and position controls for custom
  backdrops; video backgrounds can start at any second and loop from
  that point.

### Fixed

- Playfield border and cursor size now match the game scene exactly
  (border half-width 1.52, cursor 0.263 units, both from song.tscn);
  edge notes stay inside the border like in the real client.
- The renderer removes a taken note at the recorded hit frame instead
  of its chart time, and the miss X waits for the hit window to close.
  Slow motion no longer looks out of sync.
- Placeholder map metadata ("Artist Name - Song Name") falls back to
  the real map title everywhere.
- Replay playback clock no longer drifts on high-refresh displays.

## v0.4.1 (2026-07-26)

### Added

- **Resize in the HUD editor**: every box in Edit HUD grows a round
  corner handle. Drag it to scale the element freely (0.4x to 2.5x,
  about its centre, text included). Dragging back to ~100% removes
  the override; *Reset layout* now restores sizes as well as
  positions.
- **Snap grid**: a *Grid* dropdown next to *Reset layout*
  (Off / Fine / Small / Medium / Large) overlays a grid on the
  preview. Dragged elements and meters snap to it live, magnetic
  per axis: whichever of the element's real edges or its centre is
  closest to a grid line lands on it, so resized elements align
  their edges instead of hovering between lines. The choice is
  remembered.
- Arch installs (the AUR package) now get an honest update banner:
  "update via your AUR helper (rhythr-bin)" with a release-notes
  link, instead of a download-page button that can't update a
  pacman-managed install anyway.

## v0.4.0 (2026-07-26)

Ghost races become real races, and the playfield gets your own
backdrop.

### Added

- **Racing delta** (ghost races): a score-lead widget at the split
  seam: the live score gap, the accuracy gap and a tournament-style
  lead bar growing from the centre toward whoever leads, in that
  player's colour. The lead is note-synchronized: it only moves once
  a note is answered by BOTH sides and came out differently, so an
  even race shows a calm zero instead of flickering with every hit.
  The number rolls like a counter and the bar glides. The results
  screen adds a **race delta graph** showing the gap over the whole map,
  area-filled in the leader's colour, with lead changes and the peak lead
  annotated: where the race was decided, at a glance. The widget is
  draggable in Edit HUD and lives under HUD → Ghost race
  (`--no-racing-delta` in the CLI).
- **Custom backgrounds**: give the play its own backdrop, an image
  or a video shown behind the gameplay instead of the skin
  background. Videos play muted and looped, scaled to cover the
  frame, in any format your ffmpeg reads (animated GIFs actually
  animate); detection is by file content, not extension. A dim
  slider (default 60%) darkens only the background so the notes stay
  readable, and the results screen keeps its own look. New
  Background card in the app; `--background` / `--background-dim`
  in the CLI.
- **Drop targeting**: drag a file over the Ghost race or Background
  card and it lights up, and the drop lands right there. A replay
  dropped on Ghost race becomes the ghost without the Browse detour
  (or, with nothing loaded yet, your replay plus a hint that a race
  needs a second one). Images and videos dropped anywhere become the
  background. Dropping into the middle behaves as before.

### Fixed

- **"Reset layout" no longer wipes your HUD on/off choices.** The
  toolbar button and the HUD tab's "Reset all to config" shared one
  element id, so resetting the layout silently cleared every
  override too, and the HUD-tab button did nothing at all.
- A **failed side in a ghost race now freezes at its fail time** on
  every surface (side HUD, widget, results, graph): its numbers
  stop, notes it never played are not counted as misses, and a
  FAILED badge marks the dead side. Solo renders always ended at the
  fail, so nothing changes there.

### Changed

- Linux install guidance: the AppImage runtime needs your distro's
  `fuse3` (a `fusermount` binary), not the old `libfuse2`. And on
  Arch-based distros (Arch, CachyOS, Manjaro, EndeavourOS) the AUR
  package `rhythr-bin` is now the explicit recommendation over the
  AppImage.

## v0.3.4 (2026-07-18)

### Added

- **Reset layout**: a button next to the *Edit HUD* switch puts every
  dragged HUD element (meters included) back to its standard
  position, without touching visibility or sizes ("Reset all to
  config" on the HUD tab still resets everything).

## v0.3.3 (2026-07-18)

Score cards, a drag-anywhere HUD editor, vertical renders for Shorts,
and the game now connects itself.

### Added

- **Score cards**: the *Save frame* button is now **Save thumbnail**,
  a shareable result card with cover, grade, stats and mods instead of
  a raw frame grab. A dropdown picks the platform format: **Discord**
  (1200×630), **YouTube** (1280×720), **TikTok/Shorts** (1080×1920) or
  **Square** (1080×1080), each with its own layout.
- **HUD editor**: flip the new **Edit HUD** switch and every HUD
  element gets a handle. Drag it anywhere on the preview (overlapping
  is allowed), per side in ghost races. Positions save automatically
  and the render always matches the preview. *Reset HUD overrides*
  also restores the layout.
- **Vertical rendering**: new **1080×1920** and **720×1280** output
  sizes for Shorts/TikTok. Gameplay keeps its full width and the HUD
  moves into bands above and below it. The results screen re-lays out
  in portrait too, with the cover kept exactly square.
- **The game connects itself.** A visible **Game** card on the main
  screen replaces the buried Advanced entry, and the app searches for
  the game on startup: every Steam library on every drive (Windows
  registry + defaults; native Linux, Flatpak and Snap), folder names
  matched case-insensitively, native Linux builds included, so there is no
  more manual path picking.

### Fixed

- **Speed-mod renders no longer look too fast.** The game keeps the
  note approach constant in *real* time (at 1.45x there are simply
  more, tighter-packed notes in the air, approaching at the same
  on-screen pace). Renders compressed the approach along with the
  timeline, so notes flew in 45% faster than in-game. The approach now
  matches the game exactly at any speed.
- Replays that store wall-clock frame times (instead of song time) are
  detected and rescaled by checking the recorded hits against the
  map's note times, so a speed mod can never apply twice or get lost.
- The results screen shows the **difficulty** between the `< >`
  brackets (it repeated the map title).

## v0.3.2 (2026-07-17)

The custom-skin release: renders now match the game on ANY skin, not
just dark ones.

### Fixed

- **Colours blend exactly like the game's.** The game blends straight
  in sRGB; our renderer blended in linear light, which drifted on
  every semi-transparent pixel, worst on bright skins, where notes
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

## v0.3.1 (2026-07-17)

### Fixed

- **Speed mods play back at their real speed.** A 1.45x run rendered at
  1x with normal-pitch audio (matching the website's replay viewer, not
  the game). The video timeline now compresses by the replay's speed
  factor and the song is rate-shifted (faster and higher-pitched),
  exactly like the run felt in-game. Hit sounds follow along.
- The top bar shows the actual app version (it was stuck saying v0.2).

## v0.3.0 (2026-07-17)

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
  instead, since they update through the package manager.
- **Ghost races**: load a second replay of the same map and the video
  becomes a side-by-side **split screen**, each run with its own
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
  app. Set the slider to 0% to turn them off. The CLI defaults to
  off.

### Fixed

- **Mirror and hardrock replays rendered the unmodified field.** The
  notes now transform to what the player actually saw (mirror axis
  recovered from the run itself, hardrock's wider grid and border), in
  video, preview, frame export and CLI stills.
- **Blank window or crash on startup on many Linux/Wayland systems**:
  the app disables WebKitGTK's DMA-BUF renderer by itself.
- The progress clock no longer disappears when the title above it is
  hidden, since it belongs to the progress bar.

### Notes

- Renders of the same replay can look slightly different than v0.2.1:
  the fail-vignette colour is now the game's exact red, and the combo
  ring no longer decays over time.
- Hit sounds, the game font and mod icons need the game assets. Run
  *Advanced → Detect game* once (and re-run it after game updates).
- The update check runs once at startup and fails silently when
  offline; there is currently no setting to turn it off.
- The Chaos mod randomises note positions with a seed the replay does
  not store; it renders unmodified.
- Skin background layers render statically: layer rotation and
  camera-coupled movement (parallax/spin) are approximated.
- The .rpm does not pull in ffmpeg (Fedora's stock repos lack a
  package by that name). Install it via RPM Fusion, or use the
  AppImage.

## v0.2.1 (2026-07-15)

### Changed

- API usage and labelling per agreement with the Rhythia team:
  identifying User-Agent, backoff on 429/5xx, "unofficial community
  tool" labelling throughout, and a clarified verified badge (rhythr's
  own consistency check, not an official Rhythia score verification).

## v0.2.0 (2026-07-15)

### Added

- First public release as **rhythr**: Windows installer, desktop app
  with live preview and timeline, automatic map download with caching
  and hash verification, replay integrity check ("verified" badge),
  skin support from exported `.rhs` configs, built-in asset extraction
  from your own game install, HUD element overrides, results screen,
  hardware-encoder auto-pick (NVENC / Quick Sync / VAAPI, x264
  fallback), `.rhr` file association and a CLI.
