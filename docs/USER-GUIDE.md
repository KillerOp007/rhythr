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
- **Note Designer** (button in the toolbar): design your own note from
  parts, no modelling tools needed. Its toolbar is six labelled groups
  (Add, Import, Edit, View, File and Game), the shape, import, view and
  game groups in their own colours. Add
  discs, squares, polygons, stars,
  rings and pills, move, scale and rotate them on the canvas, switch a
  part to Cut to carve into the others, give each part its own Depth
  band so the note gets real relief, and mark parts as Glass to draw
  them translucent like the game does. Presets opens five finished
  notes in the rim style (Target, Hex frame, Crosshair, Diamond frame,
  Notched ring): one click replaces the note with it as plain parts,
  asking first when there are unsaved changes. SVG and Letter bring an
  SVG file or a typed letter, digit or emoji (any installed font) in as
  parts of their own: an SVG's strokes count as much as its fills (line
  icons work), and every colour becomes a part of its own, a colour
  drawn on another coming in as a Cut so a two-tone badge keeps its
  figure. Double-click any part (or press To points in the inspector)
  to edit its outline point by point: drag a point, click the outline
  to add one, double-click a point to switch between smooth and corner,
  right-click or Delete removes it, Esc finishes. The canvas shows
  exactly the outline the export will have: a part keeps its size while
  you move it, and the solid square is the note box, which never
  changes. Whatever a part puts outside the box is cut off, nothing is
  ever rescaled. Parts at the same depth merge into one flat surface, in
  the game too; give a part a higher Front to make it stand out, and the
  canvas draws the lower parts darker so you can see what lies above
  what. Groups gather parts: a Cut part inside a group carves only that
  group's Add parts, and a Cut group carves everything outside it; the
  Group field of a part or a drop on a group's row in the list puts it
  in, the group's row folds its parts away. Effects sit on each part as
  switches: Waves ripple the outline inward, Round softens corners, Rim
  keeps only the outline band; Mirror reflects the part across the axes
  and Repeat turns copies around the centre. Snap (next to the zoom
  button) pulls moves, sizes, turns and points to a 0.05 grid and the
  box lines while the pointer is near one. The previews (still and
  gameplay loop through the real renderer) start hidden; the Previews
  button or the P key shows them, and the choice is remembered. 3D (or
  the 3 key) shows the baked note turned, up to 60 degrees each way, so
  you can judge the depth you are building: Shaded lights the faces so
  relief reads (the game draws the note flat and shows relief only on
  the approach), Fade draws it translucent like the approach fade.
  Designs autosave in the window. Save writes one `.rhs` file: the skin
  for rhythr and the game, with your editable design inside, so Open
  brings it straight back; a file saved by the older designer opens here
  too and is converted. Send to game writes the note straight into the
  game's own folders instead (see below). Help (F1) lists every button,
  mouse action and key; the same table is at the end of this section.
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
- **Testing a note in the game**: press Send to game in the Note
  Designer. It writes the note into the game's data folder
  (`%APPDATA%\CapoRhythia`, or the game's Proton prefix on Linux) as
  the files `skins\notes\rhythr-kit.obj` and `.mtl` plus
  `skins\colorsets\rhythr-kit.txt`, and once creates a profile named
  `rhythr-kit`, a copy of your active profile with that note and
  colorset. Pick that profile in the game's settings once; after every
  later send, restart the game (the profile stays selected) and the new
  note loads. No import dialog, no file to find. If the folder
  cannot be found the status says where it looked; if the game's
  `framework.json` relocates its data, the kit does not follow and asks
  for a manual import instead.
- **Importing a skin into the game**: copy the `.rhs` file to any folder
  on a local drive, such as a subfolder of your user folder on C:, and
  import it from there. The game refuses to read a skin from a network
  path such as a UNC share (`\\server\share\skin.rhs`) and answers "File
  path is unsafe", so a skin that sits on a shared folder or a NAS has to
  be copied to a local disk first. Nothing is wrong with the file itself.
- **Verified badge**: every replay is integrity-checked. The hits,
  misses and accuracy are re-derived from the raw inputs and compared
  against the file's header. "inconsistent (possibly modified)" means
  the numbers don't add up. "map may not match" is the milder case:
  the recorded hits fit no note on the chart that is loaded, which
  usually means it is not the chart that was played; load the right one
  before doubting the run.

### Note Designer controls

The same table the designer shows under Help (F1): every button, mouse
action and key of the Note Designer window.

**Toolbar**

| Control | What it does |
|---|---|
| Disc, Square, Polygon, Star, Ring, Pill | Add a part of that shape at the centre, one note wide, and select it. |
| SVG | Add an SVG file: one part per colour, strokes outlined at their width; a colour drawn on another colour comes in as a Cut. |
| Letter field, then Enter or the Letter button | Add the typed letter, digit, symbol or emoji (any installed font) as a part. |
| Undo, Redo | Step back and forward through every change (Ctrl+Z, Ctrl+Y). |
| Previews | Show the still and the gameplay loop from the real renderer below the canvas (P). |
| 3D | Show the note turned, to judge its depth (3). |
| Help | This table (F1 or ?). |
| New | Start a fresh note; asks first when there are unsaved changes. |
| Presets | Five finished notes in the rim style (Target, Hex frame, Crosshair, Diamond frame, Notched ring): a click replaces the note with it as plain parts, asking first when there are unsaved changes. |
| Open | Open a skin saved by rhythr's editors (.rhs) or a design file (.rhsd) (Ctrl+O). |
| Recent | Reopen one of the last six files. |
| Save .rhs | Write the one file that is the skin for rhythr and the game with your design inside (Ctrl+S). A design the kernel cannot build is not written; the status says why. |
| Save as | Write to a new .rhs file (Ctrl+Shift+S). |
| Send to game | Writes the note into the game's skin folders as the profile rhythr-kit: pick that profile once in the game's settings; after later sends restart the game, the profile stays selected. No import, no file dialog. |

**Parts list**

| Control | What it does |
|---|---|
| Click a row | Select the part. |
| Drag a row | Reorder the parts; the top part is picked first by a click on the canvas. The order changes no geometry. |
| Eye | Hide or show the part; a hidden part adds and cuts nothing. |
| Lock | Lock the part against moving, scaling, rotating and point edits. |
| Number | The part's Front depth. Parts at one depth merge into one surface; a higher Front stands out. |
| Add, Cut, Solid, Glass | The part's mode and material, set in the inspector. |
| Group header | Selects the group; its parts sit under it. A Cut group carves everything outside it with the shape of its Add parts. |
| Fold triangle | Hides or shows the group's parts in the list; nothing in the design changes. |
| Drop a part on a group header | The part joins the group. |

**Canvas (front view)**

| Control | What it does |
|---|---|
| Click | Select the part under the pointer, top of the list first; empty space clears the selection. |
| Drag a part | Move it. |
| Drag a corner handle | Scale it; Shift keeps the proportions. |
| Drag an edge handle | Scale along that axis only. |
| Drag the handle above the part | Rotate it; Shift snaps to 15 degrees. |
| Double-click a part | Edit its outline point by point (any shape is converted to points first). |
| Wheel | Zoom about the pointer. |
| Middle drag, or Space and drag | Pan the view. |
| 100% button | Reset zoom and pan. |
| Snap | Pulls moves, sizes, turns and points to a 0.05 grid and the box lines while the pointer is near one; off, everything is free. |
| Solid square | The note box: whatever a part puts outside it is cut off, nothing is rescaled. |
| Dashed square | The game's true hit area. |
| Shades | The highest part keeps the colour, lower parts draw darker; Cut parts hatch. |

**Point editing**

| Control | What it does |
|---|---|
| Drag a point | Move it; Shift snaps to a grid of the part's own frame. |
| Click the dashed outline | Insert a point there and pick it up (a corner between two corners, smooth otherwise). |
| Double-click a point | Switch between smooth (round) and corner (square). |
| Right-click a point, or Delete | Remove it; a loop keeps three points, a three-point loop goes as a whole unless it is the last. |
| Arrows | Nudge the selected point by 0.01, with Shift by 0.1. |
| Esc, a click away from the outline, or Done | Finish editing. |

**3D (turned view)**

| Control | What it does |
|---|---|
| Drag | Turn the note, up to 60 degrees each way. |
| Wheel | Zoom. |
| Front, 3/4 | Snap to the front or to a view from the side and above. |
| Shaded | Shade the faces so relief reads. The game draws the note flat; relief shows there only on the approach. |
| Fade | Draw the note translucent like the game's approach fade, where overlapping parts brighten. |

**Groups and effects**

| Control | What it does |
|---|---|
| Group (inspector) | Puts the part into a group, takes it out (None) or starts a new one. A Cut part inside a group carves only that group's Add parts. |
| Group inspector: Name, Mode | Names the group; Add contributes its parts, Cut carves them out of everything else. |
| Leave group, Ungroup | Takes one part out, or dissolves the group and keeps its parts. |
| Waves | Ripples the outline inward from the edge: count, depth and style (sine, spike, pulse). |
| Round | Softens the part's corners by the radius. |
| Rim | Keeps only the outline band of that thickness. |
| Mirror | Reflects the part across the note's axes (X, Y or both). |
| Repeat | Turns copies of the part around the centre, 2 to 12; 0 switches it off. |

**Inspector**

| Control | What it does |
|---|---|
| Name | The part's name in the list. |
| Shape | Swap the shape kind; its own fields (sides, rounding, points, thickness, roundness) follow. |
| To points, Edit points, Done | Convert the outline to points, start or finish editing them. |
| Mode | Add puts material, Cut takes it away from the Add parts that share its depth. |
| Material | Solid, or Glass, which the game draws translucent. |
| X, Y, Width, Height, Rotation | The part's place and size in note units (the box is 1 from the centre to its edge). |
| Front, Back | The depth band; Front faces the player. The game's own disc spans 0.1 to -0.1. |
| Duplicate, Remove | Copy the part above itself, or delete it. |
| Colors | The colours the notes cycle through, in beat order; drag to reorder, the first one colours the canvas. |

**Keyboard**

| Control | What it does |
|---|---|
| Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z | Undo, redo. |
| Ctrl+S, Ctrl+Shift+S, Ctrl+O | Save, save as, open. |
| Ctrl+D | Duplicate the selected part. |
| Delete, Backspace | Remove the selected part, or the selected point while editing points. |
| Arrows, Shift+Arrows | Nudge the selected part (or point) by 0.01 or 0.1. |
| Esc | Clear the selection, or finish point editing. |
| P, 3, F1 or ? | Previews, 3D view, this help. |
| Space and drag | Pan the front view. |

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
replays and maps, downloads, the app, the Note Designer, sending to the
game), the number says which failure, and the last letter is your
system: `W`indows, `L`inux, `M`acOS. The same failure has a different
cause on each, which is why it is part of the code. Every code has a
row in `docs/ERROR-CODES.md` with the next thing to try.

For anything that is not obvious from the message, open **Advanced** and
use **Diagnostics**: **Save to file** writes a text report, **Copy**
puts the same thing on your clipboard. It contains the app version,
your system and processor, your GPU and driver, which ffmpeg was found
and which encoders it offers, what is loaded, your last render's
timings, the Note Designer's last save and send, what the designer finds
in the game's folder (every folder it tried, the active profile, and
whether its own files are there), the settings you changed from default,
and the codes of everything that went wrong this session, each with the
next thing to try under it. Your home folder is written as `~`, so it
carries no account name, and nothing is sent anywhere: you choose where
the file goes and can read it first.

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
  window on a system with a very recent Mesa (SteamOS 3.8, Fedora 44,
  Arch) plus the terminal line `Could not create default EGL display:
  EGL_BAD_PARAMETER` means the 0.6.2 AppImage's bundled display
  libraries clash with your graphics stack. Fixed in 0.6.3; on the 0.6.2 file,
  extract it once and drop those libraries:

  ```sh
  ./rhythr_*.AppImage --appimage-extract
  cd squashfs-root/usr/lib
  rm libwayland-client.so.0 libwayland-cursor.so.0 libwayland-egl.so.1 \
     libwayland-server.so.0 libxkbcommon.so.0 libxcb-randr.so.0 \
     libxcb-render.so.0 libxcb-shm.so.0 libXau.so.6 libXdmcp.so.6
  cd ../.. && ./AppRun
  ```

  On Arch-based distros (Arch, CachyOS, Manjaro, …) prefer the AUR
  package `rhythr-bin` anyway: a native install without FUSE, built
  against your own system libraries.
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
