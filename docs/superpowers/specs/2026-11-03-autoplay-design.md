# Autoplay: show-off videos without a replay

Date: 2026-11-03. Status: design, approved for planning by the owner on 2026-11-03.

## Intent

People in the community want show-off videos of a map: the map played perfectly, with a cursor that moves like a dancer, cut to music, for YouTube and Discord. Today rhythr needs a replay for every video. Autoplay removes that requirement: the person exports a map from the game, picks it in rhythr, chooses how the cursor should move, and renders. No replay, no import dialog, no file hunting.

The owner's one hard condition, settled before anything else: Autoplay must not break the rule the tool was built on (no code writes or re-encodes `.rhr` files) and it must not become a cheat. This document is written so that both stay true by construction, and so that a video made with Autoplay can be told apart from a real play.

## Goals

- Render a map with a synthetic cursor straight from the game's export folder, in the main window, with the existing render pipeline, output settings, HUD and skin.
- Two movement styles: Normal (soft, direct curves) and Cursor dance (the flowing style of the reference videos, scaled to Rhythia's small playfield).
- Every Autoplay video is marked as such in three layers: a visible ticker (a setting since 2026-11-09, on by default), a hidden mark in the picture, and a metadata tag. Only the owner and the Rhythia team can check the hidden mark, with a key that never enters the repository.
- A verification command in `rhythr-cli` that reports whether a video carries the hidden mark.

## Non-goals

- No sliders, no spinners, no mods other than the speed mod. Rhythia has notes only.
- No "humanised" movement. The cursor is clearly a machine: smooth, exact, never jittery. Imitating a real player's imperfections is an anti-goal, not a stretch goal.
- No Analyze view and no score card for Autoplay renders.
- No content-level mark (note colours, cursor geometry) in this round. It is described at the end as the second stage, because the owner decided to ship the pixel mark first.

## Anti-goals (what would make this a failure even if it works)

- Any code path that serialises a `Replay`. The synthetic cursor is a value in memory and nothing else; the anti-cheat statement in `README.md` stays literally true and gains a sentence about Autoplay.
- A hidden mark or metadata tag that can be switched off. There is no setting, no config key, no hidden flag for them. (The ticker stood in this list until 2026-11-09, when the owner made it a setting: the mark and the tag carry the proof, the ticker is cosmetic, and some people want the clean picture.)
- A render that looks like a real play to a casual viewer: player name and HUD say Autoplay, and so does the ticker while it is on.

## Constraints and facts the builder relies on

- The game writes exported maps as `.rhm` zip files (entries `map`, `audio`, `cover`) into `<game data folder>/exports`. The data folder is `%APPDATA%\CapoRhythia` on Windows and `<steam library>/steamapps/compatdata/2250500/pfx/drive_c/users/steamuser/AppData/Roaming/CapoRhythia` under Proton. `crates/gui/src/game_data.rs` already resolves it (`game_data_dir`, `candidates`, env override `RHYTHR_GAME_DATA_DIR`); the kit's Send to game uses the same resolver.
- `rhythia_formats::Map::from_rhm` already reads the archive: `meta` (title, song name, mappers, duration, online id), `notes` sorted by time with `time_ms, x, y`, and the embedded `audio` and `cover`.
- The renderer consumes a `rhythia_formats::Replay` (header fields plus `Vec<Frame { ms, x, y, health, hit }>`). Frame positions are in cursor space (observed range about plus or minus 1.369). The conversion from note coordinates to cursor space is the one `crates/sim/src/hitreg.rs` uses, and the field barrier is `Scene::clamp_cursor` in `crates/render/src/scene.rs` (edge inset `edgec`, the game's own clamp). Maps are not bound to the 3 by 3 grid: notes can sit anywhere in the field, and the path generator must not assume cells.
- The video pipeline is `rhythia_render::video::render_video` with `VideoOptions` (resolution, fps, quality, speed, hit sound, pitch rule). The CLI wraps it as `rhythr-cli video`.
- Pure black in the renders is luma 16 (limited range H.264), not 0. The hidden-mark detector and the embedding math use the video's real black level.
- Error codes are table driven in `crates/errcode` (`RH-<AREA>-<nnn>`), every code needs a row in `docs/ERROR-CODES.md` (regenerated with `RHYTHR_UPDATE_DOCS=1 cargo test -p rhythia-errcode`), and the diagnostics file (`main.rs diagnostics_report`) prints a `try:` line under every recorded code.
- House style: UI text in English, no em or en dashes anywhere (preflight fails on them), every new command in the user guide, changelog entry under Unreleased.
- Spike evidence (folder `~/RhythiaRenderer/autoplay-mark-test/`, scripts `mark.py`): a static key-derived band-pass noise pattern at amplitude 0.8 luma levels is invisible (PSNR 47.5 dB, SSIM 0.995) and is detected with z above 180 after YouTube and Discord re-encodes, 720p, 20 percent crop, denoising and a visible banner being erased; a wrong key scores z 1. Its known limit: a black-crush filter (every luma below 22 set to 16) removes it completely, and the method is public once the code is. The owner accepted that limit for stage one. Variants that leave black untouched were too weak on Rhythia's mostly black picture and are not to be retried.

## The flow in the main window

1. **Autoplay button** in the toolbar above the preview, next to Note Designer, styled as an accent button. Tooltip: "Render a map with a synthetic cursor: no replay needed".
2. **Map picker** (popover over the preview, like the Note Designer's presets panel): header "Exported maps" with the folder path, "How do I export?" (expands three lines: right-click the map in the game's song list, Export, come back), "Open folder", "Refresh" and "Choose folder" (a persisted manual choice for odd installs; the environment variable `RHYTHR_GAME_DATA_DIR` keeps working as the override above it, and diagnostics print which of the three resolved the folder). A search field (title, artist, mapper). A list, newest export first, one row per `.rhm`: title in bold, below it artist and mapper in muted text, on the right duration, note count and the export date. Long titles are cut with an ellipsis and the full name is the row's tooltip; nothing may overflow. Enter picks the first visible row, Esc closes. Empty state: "No exported maps yet. In the game, right-click a map and choose Export; it appears here on its own." The folder is watched while the picker is open; a new export shows up without pressing Refresh (polling every second is fine).
3. **Picking a map** turns the Replay card into the **Autoplay card**: the map's title, artist and mapper, chips for duration and note count and "from exports", and a "Back to replay" link that restores the replay flow. The Map card shows the same map with "audio and cover inside the .rhm". Dropping a `.rhr` while Autoplay is active asks nothing: it switches back to the replay flow, as "Back to replay" does.
4. **Settings**: a third tab "Autoplay" appears next to Output and HUD and is selected. Output and HUD keep working as today. The tab holds:
   - Movement: a two-way switch, Normal or Cursor dance (default Cursor dance).
   - Dance amount: three steps, subtle, medium, wild (default medium). Disabled under Normal.
   - Rest in the centre after: 2.0 to 8.0 s in steps of 0.5, default 4.5 s, with the hint "Real seconds without a note. With a speed mod the map time is divided by it, so the feel stays the same." and, when a speed mod is set, the map-time equivalent ("at 1.50x that is a gap of 6.8 s in the map").
   - Speed mod: 0.80x to 2.00x in steps of 0.05, default 1.00x, with 1.10, 1.25, 1.50, 1.75, 2.00 as quick picks. It drives song speed and pitch through the existing rules, the approach, and the path's real-time thresholds.
   - Banner (since 2026-11-09): On or Off, default On. Off leaves the ticker out of the preview and the video; the hidden mark and the metadata tag stay.
   - Fixed rows, not editable: Player name "Autoplay"; Accuracy "100.00%, full combo".
   - A note: "Marked as autoplay. Every autoplay render carries a hidden mark that survives re-encoding and a metadata tag; neither can be switched off. The banner across the top is yours to keep or drop."
   All Autoplay settings persist like the other settings.
5. **Session restore**: the app remembers the last picked `.rhm` path and the Autoplay settings. On the next launch it restores the Autoplay session when that file still exists, otherwise it starts in the replay flow. Exported maps do not enter the Recent list; the picker is their list.
6. **Preview and render**: the preview shows the synthetic run exactly as the video will look, ticker included while the banner is on. Clip range, scrubbing, the live preview and Render video work as for a replay. The render bar's ready line names the map and the output settings. The suggested file name starts with `Autoplay - ` followed by the map title. Analyze and Save thumbnail are hidden while Autoplay is active (tooltips are not needed; the buttons are simply not there). The results screen at the end of a video (`results_secs`) is forced off for Autoplay: an SS results frame with 100 percent is exactly the fake score image this design wants to avoid, so the run ends on the playfield. The Ghost race card is hidden as well; a real player's run never appears inside a marked video in this round.
7. **Status and errors**: the Autoplay card and the status line carry the messages; codes follow the error-code convention (area below).

The click-through mockup the owner accepted is at https://claude.ai/artifact/QqfHHxTsqFvFePfjfKXbu2; its layout, labels and copy are the reference for the builder.

## The synthetic cursor

### Output

The generator produces a `Replay` value in memory: `player_name` "Autoplay", `speed` the chosen speed mod, `mods` empty, `passed` true, `accuracy_pct` 100, `hits` the note count, `misses` 0, `fail_time_ms` -1, `map_id` and `legacy_map_id` from the map, and frames at the render's frame rate (at least 60 per second of map time) with `health` 1.0 and `hit` true on the frame of each note. Nothing serialises it; `rhythia_formats` keeps having no writer, and a test asserts that no function in the workspace takes a `Replay` and returns bytes or writes a file (grep-level test over the source tree is acceptable).

Everything downstream (hitreg, HUD, integrity badge, video, preview) treats it like a loaded replay, with two exceptions: the integrity check is not run (there is no header to compare against) and the HUD shows "Autoplay" where the player name goes.

### Rules of movement

The guiding principle, read off the reference videos: the cursor keeps a steady, pleasant pace. The length of the path between two notes follows the time available, so that the cursor neither rushes nor crawls. Concretely, for each gap between consecutive notes (or from the start to the first note, where the cursor begins in the centre):

- The cursor is at the note's centre exactly at the note's time, for every note that stands alone. Rhythia maps also hold doubles and clusters at the same millisecond, which one cursor cannot centre at once. For those the rule is the game's own: a note counts when the cursor is inside its hit area (half width 0.56875 in note units, adjacent areas overlap) before the note time plus the window of 55 ms times the speed mod. The generator covers two adjacent simultaneous notes from one point that lies inside both areas, and sweeps a wider cluster in chart order inside the window, arriving at the first on time. The run therefore always scores a full combo through the ordinary hit registration (`crates/sim/src/hitreg.rs`), and the HUD's 100 percent is honest, not asserted. The owed test "exact position at every note time" applies to isolated notes; for clusters the test is "every note registers as a hit".
- Let `d` be the distance between the two notes and `t` the gap in real seconds (map milliseconds divided by the speed mod). The target path length is `L = clamp(cruise * t, d, L_max)`, where `cruise` is a cruise speed chosen by the builder so that typical medium gaps look like the reference videos, and `L_max` is what fits inside the field without touching the barrier more than once.
- **Close notes** (`L` barely above `d`): a nearly straight line. In dense runs this means straight segments with sharp turns at the notes; no rounding is forced on them.
- **Medium gaps**: a curve with a belly, bulging away from the straight line. The belly grows with `L - d`.
- **Long gaps**: wide arcs that may swing around other notes. The bulge direction prefers the side with more room and continues the direction the cursor arrived with, so the trail reads as one flowing line (tangent continuity through the note; a direction change at a note is allowed and shows as the small hook seen in the references).
- **Same spot** (next note within one cursor width of the current one): short gap, the cursor holds still on the note; long gap, a boomerang: it swings out in a direction with room, the swing's reach growing with `t`, and returns on a different arc, arriving exactly on time.
- **Back and forth** between two spots: the outbound and the return take different arcs, so the trail forms a lens, never a retraced line.
- **Rest**: when `t` exceeds the "rest in the centre" threshold, the cursor leaves the note on a soft curve to the field centre, waits there, and leaves in time to arrive at the next note at cruise pace. Under Normal movement the same, with straight-ish curves.
- **Barrier**: every generated position passes through `Scene::clamp_cursor`; arcs are planned to stay inside, flatter near the edges, freer in the middle. The cursor never leaves the field, exactly as in the game.
- **Normal movement** is the same machinery with the belly, boomerang and lens disabled: direct soft curves, the hold on same-spot notes, the rest rule.
- **Dance amount** scales the belly, the boomerang reach and the lens width: subtle 0.5, medium 1.0, wild 1.5 times the builder's base values, still bounded by the barrier.
- **After the last note** the cursor eases to the centre; the run ends at the later of last note plus 3 s and the audio's end, but never beyond the audio.

The generator is deterministic: the same map, settings and build produce the same frames. Its random choices (which side to bulge, boomerang direction when both sides have room) are drawn from the hidden mark's pattern (below), not from a clock. That makes the dance and the mark one piece of code: a build that removes the pattern has no dance. A build with a different key dances differently; that is accepted.

### Tests the builder owes

Unit tests on the generator with synthetic maps: exact position at every note time; no frame outside the barrier; close notes produce a path within 2 percent of the straight distance; same-spot with a long gap produces a boomerang that leaves and returns; a gap above the rest threshold visits the centre; determinism (two runs equal); the speed mod shortens real-time gaps; an off-grid map (notes at arbitrary positions, some at the edge) renders without panics. Plus one render test that produces a short video from a test map and checks the frame count, and a property test that the hit count equals the note count.

## The marks

### Layer 1, visible: the ticker

A band across the top of the picture, inside the safe area of the HUD, with the text `AUTOPLAY   rendered with rhythr   not a real play   <movement>   <speed>x` moving continuously from left to right and wrapping, at a readable size (about 1.8 percent of the frame height), white at 65 percent opacity on a dark pill at 35 percent, drawn by the HUD layer after every other element so nothing covers it. Motion is continuous, roughly 12 s per pass at 1080p, so a frame grab always shows part of it. It appears in the preview as well. It has no setting.

Since 2026-11-09 the ticker is a setting (Banner in the Autoplay tab, on by default; `--no-banner` on the CLI). The two hidden layers are not, and the CLI flag has no counterpart for them.

### Layer 2, hidden: the picture mark

The spike's static pattern, ported to Rust and applied in the compositor as a full-screen pass on every frame of an Autoplay render (video and exported stills), never on replay renders:

- Pattern: from the key bytes, SHA-256 seeded PRNG, standard-normal field on a 640 by 360 grid, band-passed (gaussian sigma 1 minus gaussian sigma 3), zero mean, unit variance; scaled to the frame with bilinear sampling; added with amplitude 0.8 of 255 to all three colour channels equally (so it lands on luma) in the compositor pass on the final RGBA colour texture, before the NV12 conversion or the rgba feed to ffmpeg, and before stills are read back. The spike embedded on the decoded Y plane around black 16, where the pattern swings both ways; an RGBA texture clamps black at 0 and would cut the negative half. Autoplay renders therefore clear the frame to RGB 1,1,1 instead of 0 (invisible, one level) so the pattern keeps both halves on black and the conversion carries them to luma 16 plus or minus. The builder re-measures the spike's thresholds on the real pipeline (a test renders a synthetic clip, re-encodes it with ffmpeg at 2.5 Mbit/s and 720p, and expects the checker's z above 50). The pattern is generated once on the CPU (the generator samples it for its random choices) and uploaded as a texture.
- The key in the binary: whatever embeds the pattern must hold it, so a release binary can be reverse engineered for it. The build never stores the hex key as a string: the 32 seed bytes are split into several constants and recombined at runtime, which defeats `strings` and casual inspection, not a determined engineer. That is a documented limit next to the black crush: the hidden mark stops people who do not take the binary apart. The verification page says so.
- The key: 32 random bytes as 64 hex characters, kept outside the repository (the owner's copy is `~/.tauri/rhythr-autoplay-mark.key`, next to the updater key, backed up with it). Release builds receive it through the environment variable `RHYTHR_MARK_KEY` at compile time (`option_env!`), the same way the updater signing key reaches the build. The Linux container build and the Windows build instructions pass it through; `scripts/preflight.sh` and the build scripts refuse to produce release artefacts when it is unset. Without it (contributor builds) a fixed, documented development key is compiled in and the window title gets the suffix "dev build"; such builds mark with the public development pattern, which the checker also recognises and reports as "dev key".
- Verification: `rhythr-cli mark-check <video> --key <file> [--key <file> ...]`. It decodes the video with the bundled or system ffmpeg at the video's own resolution, computes the temporal average of the high-passed luma on the canonical 640 by 360 grid, cross-correlates it with the pattern over all offsets, pattern scales 0.6 to 1.6 (coarse steps of 0.05, then fine steps of 0.005 around the best) and image rotations of plus or minus 2 degrees (coarse 0.5, fine 0.1), and compares the best score with the same search for 20 random wrong keys. The fine scale and the rotation search exist because the owner's red-team round showed that a 2 percent upscale with a centre crop and a 0.25 degree rotation slip past a coarse search while leaving the mark intact. Output: one human line ("Autoplay mark found: z 343, scale 1.00, key owner") and, with `--json`, the numbers. z above 6 is "found", 3 to 6 "weak", below 3 "no mark". Several keys allow rotation: a leaked key is replaced in the next release and old videos stay checkable. The checker needs no GPU and no window.
- Metadata: the output MP4 carries the comment tag `rhythr autoplay render: not a real play`. It is the cheapest layer and the first to go in a re-encode; it is there for the honest cases.
- Documentation states the limits plainly: the hidden mark does not survive a deliberate black-crush, and a self-compiled build without the key does not carry it. The verification doc for the Rhythia team (one page in `docs/`, English) explains what a result means and what it does not prove.

### Stage two (later, not in this round)

A mark inside the rendered content that survives everything short of a re-render: per-note colour deviations of two or three levels keyed by the secret, and micro-geometry of the cursor path; checked by image analysis of the notes and a "handwriting" comparison of the cursor against the deterministic generator. Recorded here so the stage-one design leaves room for it: the generator is deterministic, the key reaches the renderer, and the checker lives in `rhythr-cli` where the second check can be added.

## Integration points

- `crates/formats`: no change beyond what the picker needs (`Map::from_rhm` exists). The builder may add a cheap header read for the list (meta without audio) if loading every archive fully is too slow for a folder of a hundred maps.
- `crates/render`: new module `autoplay` (generator, settings struct, synthetic `Replay`), the mark pattern (`mark` module: key handling, pattern generation, checker math shared with the CLI), the compositor pass, the ticker in the HUD layer, the file-name prefix. Tests as listed above plus mark tests: embedding changes the luma by the expected amount; the checker finds the mark on a synthetic 200-frame clip and not on a clean one; a wrong key scores below 3.
- `crates/cli`: `rhythr-cli autoplay --map <file.rhm> -o <out.mp4> [--movement normal|dance] [--dance subtle|medium|wild] [--rest <s>] [--speed-mod <x>]` plus the shared video options (`--speed` keeps its meaning from `video`, the playback speed of the output; the speed mod is always `--speed-mod`); `rhythr-cli mark-check`.
- `crates/gui`: commands `autoplay_exports` (list of the folder, with a `changed` token for the one-second poll), `autoplay_pick` (loads the map, builds the run, returns the summary for the card), `autoplay_settings` (set and persist), `autoplay_clear` (back to replay); the picker, the card, the tab and the hidden buttons in `index.html`, `app.js`, `style.css`; the ticker in the preview; "dev build" title suffix.
- `crates/errcode`: `Area::Autoplay`, codes `RH-AUT-901` exports folder not found (try: start the game once so its data folder exists, or pick the folder with Choose folder in the map picker), `902` map archive unreadable (try: export the map again from the game), `903` map has no notes, `904` audio missing from the archive (the render goes on silent, warning level), `900` fallback. Diagnostics get an `autoplay` section: resolved exports folder, number of maps seen, last picked map, settings, whether the build carries the owner key or the dev key (never the key itself).
- Docs: user guide section "Autoplay" (flow, settings, the marks and their limits, the export how-to), `README.md` anti-cheat statement plus one Autoplay sentence, `docs/RELEASING.md` step for the key, `docs/WINDOWS-BUILD.md` and `docs/LINUX-BUILD.md` the environment variable, `CHANGELOG.md` under Unreleased, the Rhythia-team verification page.
- Legal and fair play: rhythr has no privacy or terms documents of its own; the places that speak for it are the anti-cheat statement in `README.md` and the "Fair play" section of the user guide. Both get the same two sentences: Autoplay videos are marked as such, and removing the marks to pass a video off as a real play is misuse that the Rhythia team can act on.

## Left to the builder

- The exact curve family (cubic Hermite through notes with tangent continuity is the obvious choice), the cruise speed, the base belly, boomerang and lens sizes, and how `L_max` is derived from the field geometry.
- The watch mechanism for the exports folder (poll or notify) and the cheap header read.
- Where the compositor pass sits in the existing frame graph and how the pattern texture is sampled for non-16:9 outputs (keep the canonical grid stretched to the frame; the checker does the same).
- The Rust port of the detector's maths (FFT via an existing crate or a direct correlation on the coarse grid; speed is not critical, a minute per video is fine).
- The order of the three settings in the tab and the exact copy, as long as the mockup's wording is kept where it exists.

## As built (addendum, 2026-11-09, release 0.8.0)

The build followed this document with these departures, each decided by the owner during the test rounds:

- The dance belly scales with the distance between notes (plus the time to spare), not with time alone: on a real map almost no segment had time to spare, so Dance looked like Normal (round 1).
- A stream is planned as one flow, not note by note: runs of stream gaps, Catmull-Rom pieces, a taut string for the speed profile with ramps at speed changes, loops at reversals, exact passes at chords, a wind-up from the centre into a map's first stream. The design and its measurements are in `2026-11-08-autoplay-stream-flow-design.md`.
- A fourth setting, Aim: Centre (through every note's centre, the default) or Flow (a little inside the corners of a stream, always within the hit area). CLI `--aim centre|flow`.
- The ticker is a fifth setting, Banner, see the anti-goals above. The hidden mark and the tag stay unconditional.
- `rhythr-cli` looks for ffmpeg next to itself, in the app's install folder and on PATH, and ships on the release page for Windows and Linux; the team's guide names the files.

