# Note Kit Phase 4: Polish and acceptance (design)

Date: 2026-11-01. Owner decisions from the brainstorm of 01.11. are marked
"owner"; everything marked "builder's call" was decided by the author of
this document and can be overruled. The parent spec is
`docs/superpowers/specs/2026-10-24-note-kit-2-5d-design.md`; this
document replaces its section 8, phase 4, and closes its section 9.

## 1. Intent

Phases 1 to 3 built the kit: kernel, editor, composition. Phase 4 makes it
the one note designer rhythr ships, and makes the loop from an edit to a
look in the game short. The owner named the loop as the place where time
and motivation go today: save the skin, dig the file out, open the game's
import dialog, find the file, import, and every import leaves another
profile and three more files behind. After this phase the loop is: edit,
press one button, pick the profile in the game once, play.

Everything else in this phase is finishing: five presets, the guide, the
removal of the v1 designer, a probe round in the game, a release
candidate. The owner's ruling on scope: nothing further goes in before the
release; what is missing will come back through the community.

## 2. Goals, non-goals, anti-goals

Goals

- One button sends the current design into the game's own folders so
  that no file dialog and no import are needed again.
- Five presets in the rim style open as editable parts with one click.
- The v1 designer is gone; its files still open in the kit.
- The guide, the help table and the changelog describe the kit as the
  designer.
- A probe round in the game answers the open questions below, and the
  result is a release candidate (0.7.0, builder's call: a minor bump,
  because the designer, hit sounds and mesh skins are new features).

Non-goals (owner, 01.11.)

- No reference underlay (the parent spec listed it; the owner does not
  need it).
- No Twist effect and no wedge editing with radial symmetry (the two v1
  features the kit lacks; the owner does not miss them).
- No new effects, shapes or import kinds.
- No automatic switching of the game's active profile, no writing to the
  game's `framework.json` or `userconfig.json`, no deleting of anything
  in the game's folders.
- No new dependencies (crates or npm packages).

Anti-goals (what would make the phase a failure even if it works)

- Touching the player's settings beyond the one profile the kit owns.
  `userconfig.json` holds the online session token and a protected
  password; the kit never reads or writes it.
- Sending a skin the game then crashes on. The archive rules stay: both
  materials always declared, the mtl always next to the obj.
- A failure that says nothing. Every refusal names the reason and, for a
  missing folder, where the kit looked.
- Leaving the v1 designer half present (a dead button, a command that
  still registers, a guide paragraph that describes it).

## 3. What the game does (verified 01.11.2026)

Source: the owner's Windows installation (Rhythia Release 45, file
version 2.5.29), the folder listing of `%APPDATA%\CapoRhythia`, the
runtime log of 24.10. and the method names in the game binary.

- Data folder: `%APPDATA%\CapoRhythia\` on Windows. Under Proton the same
  folder sits inside the prefix of app id 2250500:
  `<steam library>/steamapps/compatdata/2250500/pfx/drive_c/users/steamuser/AppData/Roaming/CapoRhythia/`.
- Profiles are JSON files directly in the data folder, named
  `<name>.json`, a flat object of `"Key": {"Value": ...}` pairs (about 206
  keys in Release 45). For an imported skin `NoteSkin.Value` and
  `ColorSet.Value` hold absolute Windows paths into `skins\notes\` and
  `skins\colorsets\`; built-in assets use `Textures/Game/...` paths.
  `BorderSkin`, `CursorSkin`, `CursorTrailSkin` and `HitSound` follow the
  same scheme. Verified values from an imported kit skin's profile
  (`kit4_glass_only-20261024-174816.json`):
  `"NoteSkin":{"Value":"C:\\Users\\Killer Op\\AppData\\Roaming\\CapoRhythia\\skins\\notes\\kit4_glass_only-20261024-174816.obj"}`
  and
  `"ColorSet":{"Value":"C:\\Users\\Killer Op\\AppData\\Roaming\\CapoRhythia\\skins\\colorsets\\kit4_glass_only-20261024-174816.txt"}`,
  the file extension included. Profiles are compact JSON without a byte
  order mark.
- `framework.json` holds `CurrentGameProfile.Value`, the file name of the
  active profile with its extension (`probe8_extra_entry-20261024-233905.json`),
  and a key `DataPath` (`{"Value":""}` on the owner's machine; the binary
  reveals nothing about what a non-empty value relocates, the runtime log
  never mentions it). `userconfig.json` holds account data and secrets.
- Skin files live flat: `skins\notes\<name>.obj` with `<name>.mtl` next
  to it (the obj's `mtllib` line names that mtl), `skins\colorsets\<name>.txt`
  (one `#rrggbb` per line), `skins\borders\`, and whole `.rhs` files under
  `skins\packages\`.
- The importer creates a profile `<archive>-<yyyymmdd-hhmmss>.json`,
  copies the archive entries from `noteSkin/`, `colorSet/`, `cursorSkin/`,
  `borderSkin/` and `hitSound/` under that timestamped name, rewrites the
  obj's `mtllib` line, applies the archive's `config` to the new profile
  and shows "Skin imported" or "Full settings imported". A file on a
  network path is refused with "File path is unsafe".
- The note mesh is uploaded to the GPU at game start and in the second a
  profile is imported or applied. A map start uploads nothing. So a
  changed file on disk shows after the profile is applied again, not
  before. Whether re-selecting the same profile counts as applying is
  unknown; the probe round answers it (section 8).

## 4. Send to game

### 4.1 Behaviour

- A toolbar button "Send to game" next to "Save .rhs". No shortcut
  (builder's call; the toolbar is already wide, the scenario runner pins
  its width at 1100 px).
- Preconditions as for Save: a design the kernel refuses is not sent and
  the status says why. The document's path and dirty state do not change;
  autosave already holds the work.
- The kit writes, overwriting each time, with a temporary file and a
  rename so the game never reads a half-written file:
  - `skins/notes/rhythr-kit.obj` (object name `rhythr-kit`, line
    `mtllib rhythr-kit.mtl`),
  - `skins/notes/rhythr-kit.mtl` (the same text the archive carries),
  - `skins/colorsets/rhythr-kit.txt` (the design's colours as the archive
    writes them).
  `skins/notes` and `skins/colorsets` are created when missing; nothing
  above the data folder is ever created.
- The profile `rhythr-kit.json` is created only when it does not exist:
  a copy of the active profile (`framework.json`, `CurrentGameProfile`,
  read and parsed as a JSON object) with `NoteSkin.Value` and
  `ColorSet.Value` replaced by the absolute paths of the two written files
  in the game's own path syntax (Windows separators; under Proton the
  prefix path `C:\users\steamuser\AppData\Roaming\CapoRhythia\...`),
  written as compact JSON like the game's own. Every other key is copied
  unchanged, so the player's approach rate, cursor and hit sound stay as
  they play. Later sends never touch the profile; the player's edits to
  it in the game survive.
- A `framework.json` whose `DataPath.Value` is not empty stops the send
  before anything is written: the kit cannot know what that path
  relocates, and writing into a folder the game does not read would be
  the silent failure of section 2. The status names the path and points
  at the manual import (builder's call, 01.11.; the owner's value is
  empty, so nothing changes for him).
- Fallback when `framework.json` or the active profile is missing or does
  not parse: the three files are still written and the status says that
  the profile could not be created and that one manual import of a saved
  `.rhs` sets it up. Section 8 checks whether the game accepts a profile
  with only the two keys; if it does, that two-key profile replaces this
  fallback.
- Never read or written: `userconfig.json`, `framework.json` (read only,
  for the active profile's name), any other profile, any file not named
  `rhythr-kit.*`. Nothing is deleted.

### 4.2 Finding the data folder

In this order, the first folder that exists wins. A folder counts when it
contains `framework.json` or a `skins` folder.

1. `RHYTHR_GAME_DATA_DIR` (environment; tests and unusual setups).
2. Windows: the roaming application data folder plus `CapoRhythia`.
3. Linux: for every Steam library root that `detect_game_scan` already
   enumerates, `steamapps/compatdata/2250500/pfx/drive_c/users/steamuser/AppData/Roaming/CapoRhythia`.
4. Linux, native build (unverified; only when it exists): the XDG config
   and data folders plus `CapoRhythia`.

The status line on failure lists the candidates that were tried.

### 4.3 Status texts (user wording, no kernel vocabulary)

- First send, profile created: "Sent to the game: <data folder>. Pick the
  profile rhythr-kit in the game's settings." The folder is named once,
  so a wrong target is visible instead of silent.
- Later sends: "Sent to the game. Apply the profile rhythr-kit again
  (switch away and back) to load it." The exact instruction follows the
  probe round's finding on reloading.
- Relocated data folder: "Not sent: the game's framework.json moves its
  data to <DataPath>, which Send to game does not follow yet. Save the
  .rhs and import it once."
- Profile fallback: "Sent the files, but the active profile could not be
  read: import a saved .rhs once, then Send to game keeps it current."
- Folder not found: "Game folder not found. Looked in: <list>."
- Write failure: "Could not write <path>: <operating system reason>."

### 4.4 Code shape (builder's call)

- A Tauri command `kit_send_to_game(design)` in `kit_commands.rs`; the
  folder search and the profile rewrite in Rust, with unit tests against
  a temporary folder: profile created once with the two values replaced
  and all other keys intact (`serde_json::Value`, unknown keys preserved),
  a second send leaves the profile untouched, the obj's `mtllib` line,
  atomic writes, the fallback paths, the candidate list in the error.
- `to_obj` takes the mtl file name as a parameter; `export_rhs` keeps
  `note.mtl`.
- Front end: the button, the status texts, a help row under Toolbar, a
  guide section "Testing in the game" (the manual import note with the
  network path warning stays, after it).
- Scenario group `send` in `crates/gui/ui/kit/tests/scenarios.mjs` with
  `RHYTHR_GAME_DATA_DIR` pointing at a fixture folder (a `framework.json`
  and one profile): files exist, profile created once, second send leaves
  it, unbuildable design refused, missing folder message. The harness
  `kit_eval_json` gains `--send <dir>` so the real Rust code runs.

## 5. Presets

### 5.1 Behaviour (owner)

- A preset is a finished note built from parts, and it opens as plain
  parts: nothing locked, nothing grouped, every value editable at once.
- Choosing a preset replaces the whole note. When there are unsaved
  changes the kit asks first, exactly as New does. The new document has no
  path, is not dirty, and carries the preset's name.
- Five presets, chosen by the owner from eight rendered candidates on
  01.11. (the three rejected were Ring and plate, Gear, Shield). The common
  thread: thin rims, a glass element in the centre, strict symmetry,
  nothing figurative. Colours are the game's Arctic set
  (`#ffffff`, `#a2e0ff`, `#a2e0ff`); depth bands are given as front to back.

| Preset | Parts |
|---|---|
| Target | Outer ring: Disc, Rim 0.15, 0.1 to -0.1. Inner ring: Disc, w 0.56, Rim 0.14, 0.0 to -0.2. Dot: Disc, w 0.16, Glass, 0.1 to -0.1. |
| Hex frame | Frame: Polygon 6, rounding 0.08, Rim 0.2, 0.1 to -0.1. Pane: Polygon 6, rounding 0.08, w 0.56, Glass, -0.15 to -0.35. |
| Crosshair | Ring: Disc, Rim 0.16, 0.1 to -0.1. Ticks: Square, w 0.07, h 0.3, y 0.66, Repeat 4, 0.1 to -0.1. Dot: Disc, w 0.14, Glass, 0.1 to -0.1. |
| Diamond frame | Frame: Square, rounding 0.15, Rim 0.2, 0.1 to -0.1. Diamond: Square, w 0.42, rotation 45, Glass, -0.1 to -0.3. |
| Notched ring | Ring: Disc, Rim 0.24, 0.1 to -0.1. Notches: Square, w 0.1, h 0.3, y 0.9, Cut, Repeat 4, 0.1 to -0.1. Core: Disc, w 0.3, Glass, 0.0 to -0.2. |

Widths and heights are the kit's transform values (1.0 spans the note
box); where only w is given, h equals w. The candidate JSON files and
their renders through the real renderer are under
`/home/killer/.cache/rhythr-claude/presets/` (not in the repo; the
builder re-creates the five from this table).

### 5.2 Code shape (builder's call)

- The five designs live as JSON under `crates/render/src/kit/presets/`,
  embedded with `include_str!`, exposed by `kit::presets()` and a Tauri
  command `kit_presets` that returns `[{id, name, design}]`. A Rust test
  parses, evaluates and meshes each one and bounds the triangle count
  (under 10 000). The harness `kit_eval_json --presets` serves them to the
  scenario runner.
- UI: a toolbar button "Presets" opens a popover (not a dialog) with five
  tiles, each a thumbnail and the name; Esc or a click outside closes it.
  The thumbnail is drawn by the kit's own front-view painter from the
  design (the same code as the canvas, small, with the depth shading), so
  there are no image files in the repo and a changed preset cannot go
  stale.
- Help row under Toolbar, a guide paragraph, scenario group `presets`
  (each preset loads with the expected part count and names, the prompt on
  a dirty document, the fresh document state).

## 6. Removal of the v1 designer (owner: everything goes)

- Remove: `designer.html`, `designer.js`, `designer.css`, the command
  `open_designer_window`, the commands `designer_scene`, `designer_outline`,
  `designer_preview`, `designer_read_image`, `designer_doc_save`,
  `designer_doc_load`, `designer_export`, `designer_import_svg`,
  `designer_import_glyph`, the designer window's capability, its destroy
  handler, the main window's "Note Designer" button, and the kit's
  "From old designer" button (it read the old window's autosave; the old
  designer never shipped, so nobody but the owner has one).
- Rename: the main window's "Note Kit (beta)" button becomes
  "Note Designer" and the kit window's title becomes "Note Designer"
  (builder's call: the v1 designer only ever existed in the Unreleased
  changelog, so the name is free). File and module names stay `kit`.
- Keep: everything the v1 migration and the Points shape use from
  `crates/render/src/designer.rs` (`ControlPoint`, `flatten_controls`,
  `clean_contours`, the v1 document types), trimmed of the v1 mesh
  builder and exporter, under a name that says what it is (`kit/v1.rs`,
  builder's call). Opening a v1 `.rhs` or `.rhsd` keeps converting; the
  migration tests keep passing.
- `preview_frame` and the shared preview renderer become kit-only; the
  designer's share of the destroy and lock handling goes with it.
- The four examples that build on the v1 mesh builder go with it:
  `crates/render/examples/designer_pack.rs`, `designer_plan_json.rs`,
  `designer_preview_png.rs` and `game_probe_skins.rs` (the probe
  generator of 24.10.; its archives stay under
  `/home/killer/RhythiaRenderer/game-probes/` and new probes come from
  kit designs through `kit_render_png --rhs`). Builder's call; the ledger
  records it.
- The guide loses the "Note Designer" paragraph of the v1 window and the
  mention "The Note Designer stays until the kit is complete".

## 7. Guide, help and changelog

- Guide: the section is titled "Note Designer" and describes the kit
  (parts, Add and Cut, depth, glass, effects, groups, snapping, imports,
  point editing, presets, Send to game, the one-file save). The controls
  table is regenerated from `helpMarkdown`. New paragraphs: "Presets" and
  "Testing in the game"; the network path warning stays under the manual
  import.
- Help table (F1): rows for "Presets" and "Send to game" under Toolbar.
- `crates/gui/README.txt`, the guide copy that ships next to the exe, is
  regenerated from `docs/USER-GUIDE.md` by `build.rs` on every build and
  is never edited by hand.
- Changelog, Unreleased: the entry "A note designer" is rewritten for the
  kit (it described the v1 window); the hit sound and mesh skin entries
  stay. The version becomes 0.7.0 when the release is cut.
- The parent spec: section 8 phase 4 points at this document; section 9
  is replaced by the recorded decisions (depth range [-2.0, 0.5], the v1
  slider mode as quick-shape buttons only, new parts at [-0.1, 0.1]).

## 8. Probe round in the game (owner, testbuild kit8)

The owner runs these with the Phase 4 testbuild; the results go into the
ledger and decide the final wording of 4.3.

1. Three kit-built skins in play: a stacked one (Target), a glass one
   (Hex frame), a cut-heavy one (Notched ring). Screenshots; the glass
   grey measured against rhythr's constant alpha.
2. Send to game: the first send creates the profile; pick it; play. Change
   the design, send again. Which of these loads the new file: re-selecting
   the same profile, switching away and back, restarting the game. The
   status text of later sends follows the answer.
3. A profile file with only `NoteSkin` and `ColorSet`: does the game accept
   it as a profile? If yes, the fallback of 4.1 becomes that two-key file.
4. The main window shows one designer button, "Note Designer"; the kit
   opens; a v1 file still converts.
5. Each preset, sent to the game, looks like its render.

## 9. Polish from the Phase 3 review (cheap, in this phase)

Waves style clicks coalesce into one undo step; migrated v1 wave depths
above 0.6 sit past the Depth slider (the slider shows the clamped value
and the inspector says so, or the range grows to the kernel's 0.95); the
stale comment in the migrate fit test; `toggles_never_merge` learns the
new discrete labels; the fold scenario counts evaluate calls; a selected
member of a folded group shows a marker on the header; re-grouping the
sole member of a group reuses the free name.

## 10. Acceptance

- Suites green: render kit, gui, kit_properties, node, the scenario runner
  with the new groups `presets` and `send`, preflight.
- Windows xwin check and the Linux container check pass.
- Testbuild kit8 accepted by the owner; the probe round recorded.
- No `designer_` command, file or guide paragraph left.
- Release candidate 0.7.0 built with the existing scripts; GitHub release
  and AUR only after the owner's Go, as always.

## 11. Left to the builder

Popover layout and thumbnail size; the exact wording of the status texts
within 4.3; how the Proton prefix is located on a machine with several
Steam libraries; the trimmed module's name; the order of tasks. Rulings
that change behaviour go into the plan header and the phase ledger as in
Phases 1 to 3.
