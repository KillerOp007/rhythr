# Note Kit Phase 4: Polish and Acceptance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The kit becomes the one note designer rhythr ships: a "Send to game" button writes the note straight into the game's folders, five rim-style presets open as editable parts, the v1 designer is removed, the guide and changelog describe the kit, and the branch ends as the release candidate 0.7.0 after the owner's probe round.

**Architecture:** The game-side writer is kernel code (`kit::game`, plain file system work on a folder it is handed, unit-tested against temporary folders) so the harness example and the desktop app run the same bytes; only the folder discovery (Windows roaming folder, Proton prefixes, an environment override) lives in the GUI crate. Presets are five JSON designs embedded in the kernel and served over one command, drawn as thumbnails by a pure layer model that the front view's shading already defines. The v1 designer goes out in two cuts, Rust then front end, with the migration's helpers moved under `kit/v1.rs` so old files keep converting.

**Tech Stack:** Rust (kernel, serde_json, zip, Tauri 2 commands), plain JavaScript ES modules (no framework, no bundler), Canvas 2D, Node 22 `node --test`, playwright-core from the npx cache for the scenario runner.

**Spec:** `docs/superpowers/specs/2026-11-01-note-kit-phase4-polish-design.md` (this plan argues from it; its section 3 holds the game facts, 4 to 9 the features, 10 the acceptance). Parent spec: `docs/superpowers/specs/2026-10-24-note-kit-2-5d-design.md`.

**Branch:** `note-kit-phase4` from master (master holds the Phase 3 merge 87ceb50 and the spec commit 9f3cc94). A fresh worktree rebuilds everything; reusing the Phase 3 worktree folder with `git worktree move` and a new branch keeps the warm target cache (both are clean and merged).

## Global Constraints

- Pure Rust and plain JavaScript only: no new crates and no npm packages (`tempfile`, already in Cargo.lock through the GUI crate, may become a dev-dependency of `rhythia-render`); no bundler; the pure front-end modules run under `node --test crates/gui/ui/kit/tests/*.test.mjs` (Node 22, no package.json; a directory argument does not work).
- Nothing platform-specific in browser modules; Rust uses no `std::os::unix`. The game folder code must compile and test on Linux and Windows (the Windows check is `cargo xwin check -p rhythia-gui`).
- House style: no em or en dashes anywhere, also in JavaScript, HTML, CSS, Markdown and commit messages (`scripts/preflight.sh` fails on them); comments state constraints, not narration; UI text is English; commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Vocabulary (parent spec 5.3): Add, Cut, Combine, Depth, Glass, Mirror, Repeat, Rim, Waves, Round; never boolean, union, mesh, vertex, modifier, manifold, lattice in anything the window shows. Kernel errors reach the user only through `kit_commands::user_message`.
- The game's folder is sacred beyond the kit's own files: the kit writes only `skins/notes/rhythr-kit.obj`, `skins/notes/rhythr-kit.mtl`, `skins/colorsets/rhythr-kit.txt` and, once, `rhythr-kit.json`; it reads `framework.json` and the active profile; it never reads or writes `userconfig.json`, never deletes, never creates anything above the data folder.
- Preset values are the spec's table (section 5.1) verbatim; colours `#ffffff`, `#a2e0ff`, `#a2e0ff`.
- Run `cargo fmt --all`, `cargo test -p rhythia-render --lib kit::`, `cargo test -p rhythia-gui`, `node --test crates/gui/ui/kit/tests/*.test.mjs`, `cargo build -p rhythia-render --examples` and `bash scripts/preflight.sh` before every commit named below; `cargo test -p rhythia-render --test kit_properties` and `node crates/gui/ui/kit/tests/scenarios.mjs` once before the testbuild. One cargo process at a time on this machine.
- The scenario runner needs `cargo build -p rhythia-render --example kit_eval_json` (debug) first; it starts the harness itself (`PLAYWRIGHT_CORE`, `CHROME`, `PORT`, `SHOTS` as environment overrides).

## Review Focus

Inputs the spec implies but which the happy path would not exercise. Each line names the task whose tests pin it.

1. A game folder that was never imported into has no `skins` subfolders; Send to game creates `skins/notes` and `skins/colorsets` and succeeds. Pinned in Task 1 (`send_creates_the_skin_folders_it_needs`).
2. An active profile from an older game build lacks `NoteSkin` or `ColorSet`, or holds them as plain strings; the kit inserts `{"Value": path}` for both and copies every other key untouched. Pinned in Task 1 (`profile_rewrite_inserts_missing_keys_and_keeps_the_rest`).
3. A design with no valid colours writes no colorset file and the profile keeps the active profile's `ColorSet`, so the game is never pointed at a missing file. Pinned in Task 1 (`no_colours_means_no_colorset_file_and_the_profile_keeps_its_own`).
4. A preset picked while points are being edited, a group is selected or groups are folded: loading resets `pointEdit`, `pointSel`, `selectedGroup` and `folded`, because the new design reuses the ids `p1` to `p3`. Pinned in Task 5 (`load_resets_the_editing_state`).
5. Several Steam libraries under Proton where only the second holds the prefix: every library contributes a candidate and the first that exists wins; the failure message lists them all. Pinned in Task 2 (`candidates_cover_every_steam_library_in_order`).

---

## File structure

- Modify `crates/render/src/kit/export.rs`: `to_obj` takes the mtl file name; `colorset_text` factored out of `export_rhs`.
- Create `crates/render/src/kit/game.rs`: the writer (`send_to_game`, `game_path`, `profile_from`, atomic writes, the status texts).
- Create `crates/render/src/kit/presets.rs` and `crates/render/src/kit/presets/{target,hex-frame,crosshair,diamond-frame,notched-ring}.json`.
- Modify `crates/render/src/kit/mod.rs`: `pub mod game; pub mod presets; pub mod v1;`.
- Create `crates/render/src/kit/v1.rs` from the kept part of `crates/render/src/designer.rs` (`ControlPoint`, `flatten_controls`, `clean_contours`); delete `designer.rs`; `lib.rs` loses `pub mod designer`.
- Modify `crates/render/examples/kit_eval_json.rs`: `--presets`, `--send <dir>`; `crates/render/examples/kit_render_png.rs`: the `to_obj` call.
- Create `crates/gui/src/game_data.rs`: folder discovery (`game_data_dir`, `candidates`, `is_game_data_dir`, `steam_library_roots`).
- Modify `crates/gui/src/kit_commands.rs`: commands `kit_send_to_game`, `kit_presets`; the test fixture for an old skin built with `zip`.
- Modify `crates/gui/src/imports.rs`: `kit_import_glyph` replaces `designer_import_glyph`; `designer_import_svg` and `svg_to_contours` go; two tests evaluate through the kit.
- Modify `crates/gui/src/main.rs`: the designer window, its commands, `DesignerShape`, `designer_skin` go; `designer_renderer` becomes `preview_renderer`; `designer_scene` becomes `preview_scene`; `detect_game_scan` uses `steam_library_roots`; registration list; the kit window title.
- Modify `crates/gui/capabilities/default.json`: the `designer` window entry goes.
- Delete `crates/gui/ui/designer.html`, `designer.js`, `designer.css`.
- Modify `crates/gui/ui/index.html`, `crates/gui/ui/app.js`: one button "Note Designer" that opens the kit.
- Modify `crates/gui/ui/kit.html`, `kit.css`: buttons `kt-presets`, `kt-send`, the panel `kt-presets-panel`; `kt-import-old` goes; titles.
- Create `crates/gui/ui/kit/thumb.js` (pure layer model plus a canvas painter) and `crates/gui/ui/kit/presets.js` (the popover).
- Modify `crates/gui/ui/kit/main.js`, `files.js`, `help.js`, `render.js`, `store.js`: actions, help rows, titles, the load reset, the polish items.
- Modify tests: `crates/gui/ui/kit/tests/{files,help,render,store}.test.mjs`; create `thumb.test.mjs`, `presets.test.mjs`; modify `harness.mjs` (shim cases) and `scenarios.mjs` (groups `send`, `presets`, the `files` toolbar check, the fold check).
- Modify `docs/USER-GUIDE.md`, `CHANGELOG.md`, the parent spec (sections 8 and 9); create `docs/superpowers/plans/2026-11-01-note-kit-phase4-ledger.md`.

---

### Task 1: The game writer in the kernel

**Files:**
- Modify: `crates/render/src/kit/export.rs` (`to_obj` near line 81, `export_rhs` near line 106, the `mtllib` test near line 177)
- Create: `crates/render/src/kit/game.rs`
- Modify: `crates/render/src/kit/mod.rs` (add `pub mod game;`)
- Modify: `crates/render/examples/kit_render_png.rs` (the `to_obj` call), `crates/render/examples/kit_eval_json.rs` (`--send <dir>`)
- Modify: `crates/gui/src/main.rs:4681-4700` (`kit_skin_for` calls `to_obj(&mesh, "preview", "note.mtl")`)
- Modify: `crates/render/Cargo.toml` (`tempfile` as a dev-dependency, same version as the GUI crate)

**Interfaces:**
- Consumes: `kit::doc::Design`, `kit::levels::evaluate`, `kit::mesh::build_mesh`, `kit::export::{mtl_text, to_obj}`, `KitError`.
- Produces:
  - `pub fn to_obj(mesh: &KitMesh, name: &str, mtl_file: &str) -> String` (line 2 of the text is `mtllib {mtl_file}`); `pub fn colorset_text(colors: &[String]) -> String` (the archive's colorset: one `#rrggbb\r\n` per valid colour, empty when none).
  - In `game.rs`: `pub const PROFILE_NAME: &str = "rhythr-kit";`
    `#[derive(Serialize, Debug, PartialEq)] pub struct SendReport { pub folder: PathBuf, pub profile_created: bool, pub message: String }`
    `pub enum SendError { Design(KitError), Io { path: PathBuf, error: String }, DataPath(String) }` (`DataPath` carries the non-empty `DataPath.Value` of `framework.json`; nothing is written in that case)
    `pub fn send_to_game(design: &Design, data_dir: &Path) -> Result<SendReport, SendError>`
    `pub fn game_path(data_dir: &Path, rel: &[&str]) -> String` (the path string the game stores: under a Proton prefix, `C:\users\steamuser\AppData\Roaming\CapoRhythia\<rel joined with \>`; otherwise `data_dir.join(rel)` displayed with the host's separators)
    `pub fn profile_from(active: &serde_json::Value, obj: &str, colorset: Option<&str>) -> Result<serde_json::Value, String>` (a copy with `NoteSkin` set to `{"Value": obj}`, `ColorSet` set to `{"Value": colorset}` only when `colorset` is `Some`; an `active` that is not a JSON object is an error).
    The messages, exactly: `msg_first(folder) = format!("Sent to the game: {}. Pick the profile rhythr-kit in the game's settings.", folder.display())`, `MSG_AGAIN = "Sent to the game. Apply the profile rhythr-kit again (switch away and back) to load it."`, `MSG_NO_PROFILE = "Sent the files, but the active profile could not be read: import a saved .rhs once, then Send to game keeps it current."`; the profile file is written compact (`serde_json::to_vec`), as the game writes its own.

- [ ] **Step 1: Write the failing export tests**

In `crates/render/src/kit/export.rs` tests, change the `mtllib` assertion to `to_obj(&mesh, "s", "rhythr-kit.mtl")` containing `"mtllib rhythr-kit.mtl\n"`, and add:

```rust
#[test]
fn colorset_text_keeps_valid_colours_only() {
    let t = colorset_text(&["#ffffff".into(), "bad".into(), " a2e0ff ".into()]);
    assert_eq!(t, "#ffffff\r\n#a2e0ff\r\n");
    assert_eq!(colorset_text(&[]), "");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p rhythia-render --lib kit::export`
Expected: FAIL (`to_obj` takes two arguments; `colorset_text` not found).

- [ ] **Step 3: Implement the export changes**

`to_obj(mesh, name, mtl_file)`; `export_rhs` calls `to_obj(&mesh, name, "note.mtl")` and builds its colorset with `colorset_text(&design.colors)` (the filter it has today, moved). Fix the two other callers (`kit_render_png.rs`, `main.rs` `kit_skin_for`) to pass `"note.mtl"`.

- [ ] **Step 4: Run the export tests and the workspace check**

Run: `cargo test -p rhythia-render --lib kit::export && cargo check -p rhythia-gui --examples`
Expected: PASS, no errors.

- [ ] **Step 5: Write the failing game tests**

Create `crates/render/src/kit/game.rs` with a `#[cfg(test)] mod tests` using a `tempfile::tempdir()` per test and a helper `fn fixture(dir: &Path, profile: serde_json::Value)` that writes `framework.json` as `{"CurrentGameProfile":{"Value":"p1.json"}}` and `p1.json` as `profile`. Tests:

```rust
#[test]
fn first_send_writes_the_files_and_creates_the_profile_once() {
    // fixture with ApproachRate 3.25, NoteSkin and ColorSet pointing elsewhere
    // send_to_game(&Design::default_note(), dir): Ok, profile_created true, message == msg_first(dir)
    // skins/notes/rhythr-kit.obj starts with "# rhythr note kit\nmtllib rhythr-kit.mtl\no rhythr-kit\n"
    // skins/notes/rhythr-kit.mtl == mtl_text(); skins/colorsets/rhythr-kit.txt == "#ffffff\r\n#a2e0ff\r\n#a2e0ff\r\n"
    // rhythr-kit.json: NoteSkin.Value ends with "rhythr-kit.obj", ColorSet.Value ends with "rhythr-kit.txt", ApproachRate.Value == 3.25
    // no *.tmp file anywhere under dir
    // second call: profile_created false, message MSG_AGAIN, rhythr-kit.json byte-identical to before
}
#[test]
fn send_creates_the_skin_folders_it_needs() { /* fixture without a skins folder: Ok and both subfolders exist */ }
#[test]
fn profile_rewrite_inserts_missing_keys_and_keeps_the_rest() {
    // active {"ApproachRate":{"Value":40},"NoteSkin":"old.obj"} -> NoteSkin {"Value": obj}, ColorSet {"Value": cs}, ApproachRate untouched
    // active that is a JSON array -> Err
}
#[test]
fn no_colours_means_no_colorset_file_and_the_profile_keeps_its_own() {
    // design.colors = vec!["bad".into()]: no colorsets file written, profile ColorSet.Value stays the fixture's value
}
#[test]
fn a_missing_framework_or_profile_falls_back_to_files_only() {
    // dir without framework.json: Ok, profile_created false, message MSG_NO_PROFILE, files written, no rhythr-kit.json
    // framework.json naming a profile that does not exist: same
}
#[test]
fn a_design_the_kernel_refuses_writes_nothing() {
    // all parts hidden: Err(SendError::Design(KitError::Empty)), no file under skins
}
#[test]
fn a_relocated_data_path_is_refused_before_anything_is_written() {
    // framework.json {"CurrentGameProfile":{"Value":"p1.json"},"DataPath":{"Value":"D:\\games\\rhythia-data"}}
    // -> Err(SendError::DataPath(p)) with p == r"D:\games\rhythia-data", and no skins folder created
}
#[test]
fn game_paths_follow_the_game_s_own_syntax() {
    // data_dir ".../steamapps/compatdata/2250500/pfx/drive_c/users/steamuser/AppData/Roaming/CapoRhythia"
    //   -> r"C:\users\steamuser\AppData\Roaming\CapoRhythia\skins\notes\rhythr-kit.obj"
    // a plain dir -> data_dir.join("skins").join("notes").join("rhythr-kit.obj").display().to_string()
}
```

- [ ] **Step 6: Run them to verify they fail**

Run: `cargo test -p rhythia-render --lib kit::game`
Expected: FAIL (module missing).

- [ ] **Step 7: Implement `game.rs`**

`send_to_game`: read `framework.json` first when it exists and parses, and return `SendError::DataPath` when its `DataPath.Value` is a non-empty string (nothing else happens then); evaluate and mesh the design (errors as `SendError::Design`), `to_obj(&mesh, PROFILE_NAME, "rhythr-kit.mtl")`, create the two subfolders, write the three files through `fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()>` (write `<name>.tmp` in the same folder, then rename; a failed rename removes the temp file), skip the colorset when `colorset_text` is empty. Then the profile: read `framework.json`, take `CurrentGameProfile.Value` as a string, read that file from `data_dir`, parse; on any failure report `MSG_NO_PROFILE`; when `rhythr-kit.json` exists report `MSG_AGAIN`; else write `profile_from(..)` with `serde_json::to_vec_pretty` and report `MSG_FIRST`. Io failures carry the path. `game_path` detects the Proton prefix by the component pair `pfx`, `drive_c` and rebuilds from `drive_c` with backslashes and a `C:` drive.

- [ ] **Step 8: Run the tests**

Run: `cargo test -p rhythia-render --lib kit::game`
Expected: PASS (8 tests).

- [ ] **Step 9: Add `--send <dir>` to the harness example**

In `kit_eval_json.rs`: after the design is parsed, `--send <dir>` calls `kit::game::send_to_game(&design, Path::new(dir))` and prints the report as JSON (`{"folder","profile_created","message"}`) or `{"error": text}` where a `Design` error goes through `message(&e)` and an `Io` error reads `Could not write <path>: <error>`. Update the usage comment. Check: `cargo build -p rhythia-render --example kit_eval_json` then `echo '{...default design...}' | target/debug/examples/kit_eval_json --send /tmp/<fresh dir>` prints a report with `profile_created: false` (no framework.json there) and leaves the three files.

- [ ] **Step 10: Commit**

```bash
git add crates/render/src/kit/export.rs crates/render/src/kit/game.rs crates/render/src/kit/mod.rs crates/render/Cargo.toml Cargo.lock crates/render/examples/kit_render_png.rs crates/render/examples/kit_eval_json.rs crates/gui/src/main.rs
git commit -m "kit: the game writer, one profile and three files in the game's own folders"
```

---

### Task 2: Finding the game's data folder (GUI) and the send command

**Files:**
- Create: `crates/gui/src/game_data.rs`
- Modify: `crates/gui/src/main.rs:2340-2406` (`detect_game_scan` uses `steam_library_roots()`), the `mod` list, the command registration near line 6460
- Modify: `crates/gui/src/kit_commands.rs` (command `kit_send_to_game`)

**Interfaces:**
- Consumes: Task 1's `kit::game::{send_to_game, SendReport, SendError}`, `kit_commands::user_message`, `dirs` (already a dependency), `windows_steam_path` (exists in main.rs).
- Produces:
  - `pub const STEAM_APP_ID: &str = "2250500";`
  - `pub fn steam_library_roots() -> Vec<PathBuf>` (the default roots per platform plus every `path` in each root's `steamapps/libraryfolders.vdf`, in that order, no duplicates; moved out of `detect_game_scan`, which now starts from it)
  - `pub fn candidates(env_override: Option<PathBuf>, roaming: Option<PathBuf>, steam_roots: &[PathBuf], xdg: &[PathBuf]) -> Vec<PathBuf>` (order: override; `roaming/CapoRhythia`; for each Steam root `steamapps/compatdata/2250500/pfx/drive_c/users/steamuser/AppData/Roaming/CapoRhythia`; each `xdg/CapoRhythia`)
  - `pub fn is_game_data_dir(p: &Path) -> bool` (`p/framework.json` is a file or `p/skins` is a folder)
  - `pub fn game_data_dir() -> Result<PathBuf, Vec<PathBuf>>` (`Ok` the first candidate that `is_game_data_dir`, else `Err` with every candidate tried; reads `RHYTHR_GAME_DATA_DIR`, `dirs::config_dir()` on Windows for roaming, `dirs::config_dir()` and `dirs::data_dir()` on Linux for xdg)
  - Command: `#[tauri::command] pub async fn kit_send_to_game(design: Design) -> Result<rhythia_render::kit::game::SendReport, String>` with these error sentences: folder missing: `Game folder not found. Looked in: <candidates joined with "; ">`; `SendError::Design(e)`: `Not sent, the skin would not load in the game: {user_message(&e)}`; `SendError::Io{path,error}`: `Could not write {path}: {error}`; `SendError::DataPath(p)`: `Not sent: the game's framework.json moves its data to {p}, which Send to game does not follow yet. Save the .rhs and import it once.`

- [ ] **Step 1: Write the failing tests in `game_data.rs`**

```rust
#[test]
fn candidates_cover_every_steam_library_in_order() {
    let c = candidates(None, Some("/roam".into()), &["/lib1".into(), "/lib2".into()], &["/xdg".into()]);
    assert_eq!(c[0], PathBuf::from("/roam/CapoRhythia"));
    assert!(c[1].ends_with("lib1/steamapps/compatdata/2250500/pfx/drive_c/users/steamuser/AppData/Roaming/CapoRhythia"));
    assert!(c[2].starts_with("/lib2"));
    assert_eq!(c[3], PathBuf::from("/xdg/CapoRhythia"));
    assert_eq!(candidates(Some("/o".into()), None, &[], &[])[0], PathBuf::from("/o"));
}
#[test]
fn a_folder_counts_with_framework_json_or_a_skins_folder() { /* tempdirs: empty false; with framework.json true; with skins/ true */ }
#[test]
fn the_environment_override_wins_when_it_is_a_game_folder() { /* set RHYTHR_GAME_DATA_DIR to a tempdir with skins/: game_data_dir() == Ok(that) */ }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rhythia-gui game_data`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement `game_data.rs`, move the Steam root enumeration out of `detect_game_scan`**

`detect_game_scan` keeps its scan of `steamapps/common` but iterates `steam_library_roots()`.

- [ ] **Step 4: Run the tests and the existing detect test (if any)**

Run: `cargo test -p rhythia-gui game_data && cargo test -p rhythia-gui detect`
Expected: PASS.

- [ ] **Step 5: Write the failing command test**

In `kit_commands.rs` tests: `fn send_errors_read_as_sentences()` calls the mapping function the command uses (`pub fn send_error_text(e: SendError) -> String`, also used by the command) with a `Design(KitError::Empty)`, an `Io` and a `DataPath` value and asserts the prefixes `Not sent, the skin would not load in the game:`, `Could not write ` and `Not sent: the game's framework.json moves its data to `.

- [ ] **Step 6: Run to verify it fails, implement `kit_send_to_game` and `send_error_text`, register the command, run again**

Run: `cargo test -p rhythia-gui kit_commands`
Expected: PASS. The command runs `game_data_dir()` and `send_to_game` inside `spawn_blocking`, like `kit_export`.

- [ ] **Step 7: Commit**

```bash
git add crates/gui/src/game_data.rs crates/gui/src/main.rs crates/gui/src/kit_commands.rs
git commit -m "gui: find the game's data folder and send a kit design into it"
```

---

### Task 3: Send to game in the window

**Files:**
- Modify: `crates/gui/ui/kit.html:39-41` (button after Save as), `crates/gui/ui/kit/main.js:280-352` (the action and its binding), `crates/gui/ui/kit/help.js:8-24` (Toolbar row)
- Modify: `crates/gui/ui/kit/tests/harness.mjs:105-150` (shim case), `crates/gui/ui/kit/tests/scenarios.mjs` (fixtures, group `send`, `GROUPS`)
- Modify: `crates/gui/ui/kit/tests/help.test.mjs` (the Toolbar covers "Send to game")

**Interfaces:**
- Consumes: the command `kit_send_to_game(design) -> { folder, profile_created, message }` (Task 2); the example flag `--send <dir>` (Task 1); `exclusive`, `store.setStatus`, `errText` in main.js.
- Produces: button `#kt-send` (text `Send to game`, title `Write the note into the game's skin folders as the profile rhythr-kit; no import and no file dialog`); on success the status shows `message`; on failure the error sentence from Rust.

- [ ] **Step 1: Write the failing help test**

In `help.test.mjs`, extend `the_table_covers_groups_effects_and_snapping` (or add `the_toolbar_names_the_phase_4_buttons`) asserting the Toolbar rows include a control named `Send to game` whose action mentions `rhythr-kit`.

- [ ] **Step 2: Run to verify it fails**

Run: `node --test crates/gui/ui/kit/tests/help.test.mjs`
Expected: FAIL.

- [ ] **Step 3: Add the row, the button and the action**

Help row: `["Send to game", "Writes the note into the game's skin folders as the profile rhythr-kit: pick that profile once in the game's settings; after later sends apply it again. No import, no file dialog."]`. main.js: `const sendToGame = exclusive(async () => { const r = await invoke("kit_send_to_game", { design: store.state.design }); store.setStatus(r.message); });` bound to `#kt-send`.

- [ ] **Step 4: Run the node suite**

Run: `node --test crates/gui/ui/kit/tests/*.test.mjs`
Expected: PASS.

- [ ] **Step 5: Extend the harness and write the scenario group `send`**

Harness: `case "kit_send_to_game"`: `const out = JSON.parse(await run(["--send", process.env.RHYTHR_GAME_DATA_DIR || ""], JSON.stringify(args.design))); if (out.error) throw new Error(out.error); return out;`.

Scenarios: `writeFixtures()` adds `${WORK}/game/framework.json` (`{"CurrentGameProfile":{"Value":"p1.json"}}`) and `${WORK}/game/p1.json` (`{"ApproachRate":{"Value":3.25},"NoteSkin":{"Value":"x"},"ColorSet":{"Value":"y"},"FpsCap":{"Value":360}}`), and `process.env.RHYTHR_GAME_DATA_DIR = path.join(WORK, "game")` is set before `startHarness()`. Group `send` (added to `GROUPS`), checks:
- `first send creates files and profile`: after `click("kt-send")` and `settle()`, the status starts with `Sent to the game: ` and contains `Pick the profile rhythr-kit`, the three files exist, the obj's second line is `mtllib rhythr-kit.mtl`, `rhythr-kit.json` parses with `NoteSkin.Value` ending in `rhythr-kit.obj` and `ApproachRate.Value === 3.25` and `FpsCap.Value === 360`.
- `second send leaves the profile alone`: status starts with `Sent to the game. Apply the profile`, `rhythr-kit.json` content identical.
- `a refused design is not sent`: hide the only part (eye), click, status is an error starting with `Not sent, the skin would not load in the game`, the obj's mtime unchanged.
- `the document is untouched`: `path === null` and `dirty` unchanged by sending.

- [ ] **Step 6: Run the group**

Run: `cargo build -p rhythia-render --example kit_eval_json && node crates/gui/ui/kit/tests/scenarios.mjs send`
Expected: `4 passed, 0 failed`. Also run `files` and check its toolbar width check (`scrollW <= 1100`) still passes with the new button.

- [ ] **Step 7: Commit**

```bash
git add crates/gui/ui/kit.html crates/gui/ui/kit/main.js crates/gui/ui/kit/help.js crates/gui/ui/kit/tests/harness.mjs crates/gui/ui/kit/tests/scenarios.mjs crates/gui/ui/kit/tests/help.test.mjs
git commit -m "kit: Send to game, one button from the editor into the game's profile"
```

---

### Task 4: Presets in the kernel

**Files:**
- Create: `crates/render/src/kit/presets.rs`, `crates/render/src/kit/presets/target.json`, `hex-frame.json`, `crosshair.json`, `diamond-frame.json`, `notched-ring.json`
- Modify: `crates/render/src/kit/mod.rs` (`pub mod presets;`), `crates/render/examples/kit_eval_json.rs` (`--presets`), `crates/gui/src/kit_commands.rs` (command `kit_presets`), `crates/gui/src/main.rs` (registration)

**Interfaces:**
- Produces: `#[derive(Serialize, Clone, Debug)] pub struct Preset { pub id: &'static str, pub name: &'static str, pub design: Design }`; `pub fn presets() -> Vec<Preset>` in the order Target, Hex frame, Crosshair, Diamond frame, Notched ring with ids `target`, `hex-frame`, `crosshair`, `diamond-frame`, `notched-ring`; each JSON is a complete `Design` (version 2, `name` = the preset name, the Arctic colours, `groups: []`, every part `visible: true, locked: false`). Command `#[tauri::command] pub fn kit_presets() -> Vec<Preset>`. Example: `kit_eval_json --presets` prints the array.

The Target JSON, as the template for the other four (values from spec 5.1):

```json
{"version":2,"name":"Target","colors":["#ffffff","#a2e0ff","#a2e0ff"],"groups":[],"parts":[
 {"id":"p1","name":"Outer ring","visible":true,"locked":false,"shape":{"kind":"circle"},"transform":{"x":0,"y":0,"w":1,"h":1,"rotation":0},"mode":"add","depth":{"front":0.1,"back":-0.1},"material":"solid","effects":{"waves":null,"round":null,"rim":0.15,"mirror":"none","repeat":null}},
 {"id":"p2","name":"Inner ring","visible":true,"locked":false,"shape":{"kind":"circle"},"transform":{"x":0,"y":0,"w":0.56,"h":0.56,"rotation":0},"mode":"add","depth":{"front":0.0,"back":-0.2},"material":"solid","effects":{"waves":null,"round":null,"rim":0.14,"mirror":"none","repeat":null}},
 {"id":"p3","name":"Dot","visible":true,"locked":false,"shape":{"kind":"circle"},"transform":{"x":0,"y":0,"w":0.16,"h":0.16,"rotation":0},"mode":"add","depth":{"front":0.1,"back":-0.1},"material":"glass","effects":{"waves":null,"round":null,"rim":null,"mirror":"none","repeat":null}}
]}
```

Shapes for the others: `{"kind":"polygon","sides":6,"rounding":0.08}`, `{"kind":"square","rounding":0.15}`, `{"kind":"square","rounding":0}`; the Crosshair ticks and the Notched ring notches carry `"repeat":4` and the given `y`; the notches are `"mode":"cut"`; the Diamond carries `"rotation":45`.

- [ ] **Step 1: Write the failing tests in `presets.rs`**

```rust
#[test]
fn every_preset_parses_evaluates_and_meshes() {
    let all = presets();
    assert_eq!(all.iter().map(|p| p.name).collect::<Vec<_>>(), ["Target", "Hex frame", "Crosshair", "Diamond frame", "Notched ring"]);
    assert_eq!(all.iter().map(|p| p.design.parts.len()).collect::<Vec<_>>(), [3, 2, 3, 2, 3]);
    for p in &all {
        assert_eq!(p.design.name, p.name);
        assert!(p.design.groups.is_empty());
        assert!(p.design.parts.iter().all(|x| x.visible && !x.locked));
        let ev = levels::evaluate(&p.design).unwrap();
        let m = mesh::build_mesh(&ev).unwrap();
        let n = m.indices.len() / 3;
        assert!(n > 100 && n < 10_000, "{}: {n} triangles", p.name);
    }
}
#[test]
fn every_preset_has_a_rim_part_and_a_glass_part() { /* some part with effects.rim.is_some(), some part with Material::Glass */ }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p rhythia-render --lib kit::presets`
Expected: FAIL.

- [ ] **Step 3: Write the five JSON files and `presets.rs`**

`presets()` deserialises the five `include_str!` texts (a damaged file is a programming error: `expect` with the file name).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p rhythia-render --lib kit::presets`
Expected: PASS.

- [ ] **Step 5: The command and the example flag, then build the examples**

`kit_presets` returns `presets()`; `kit_eval_json --presets` prints `serde_json::to_string(&presets())`. Run: `cargo build -p rhythia-render --examples && cargo check -p rhythia-gui`.

- [ ] **Step 6: Commit**

```bash
git add crates/render/src/kit/presets.rs crates/render/src/kit/presets/ crates/render/src/kit/mod.rs crates/render/examples/kit_eval_json.rs crates/gui/src/kit_commands.rs crates/gui/src/main.rs
git commit -m "kit: five presets in the rim style, served by the kernel"
```

---

### Task 5: The Presets popover

**Files:**
- Create: `crates/gui/ui/kit/thumb.js`, `crates/gui/ui/kit/presets.js`, `crates/gui/ui/kit/tests/thumb.test.mjs`, `crates/gui/ui/kit/tests/presets.test.mjs`
- Modify: `crates/gui/ui/kit.html` (button `kt-presets` right after `kt-new`; the panel), `crates/gui/ui/kit.css` (panel and tiles), `crates/gui/ui/kit/main.js` (mount, `actions.loadPreset`), `crates/gui/ui/kit/store.js:456-465` (`load` resets the editing state), `crates/gui/ui/kit/help.js` (Toolbar row), `crates/gui/ui/kit/tests/store.test.mjs`, `help.test.mjs`, `harness.mjs` (case `kit_presets`), `scenarios.mjs` (group `presets`)

**Interfaces:**
- Consumes: `kit_presets() -> [{id, name, design}]` (Task 4), `kit_evaluate(design)`, `depthShade` from `view-math.js`, the glass alpha constant the view uses, `parseDesign` from `files.js`, `discardOk` and `exclusive` in main.js.
- Produces:
  - `thumb.js`: `export const THUMB_PX = 96;` `export function thumbLayers(ev, colour, size = THUMB_PX)` returns `[{ fill, alpha, polys }]` with `polys` as arrays of `[x, y]` pixel points: the solid bands back to front with `fill = depthShade(colour, iv.z1, top)` and `alpha 1`, then one entry for all glass with `fill = colour` and the view's glass alpha; one unit maps to `size / 2.4` pixels around the centre, y up. `export function drawThumbnail(canvas, ev, colour)` sizes the canvas to `THUMB_PX` squared and fills the layers with the nonzero rule.
  - `presets.js`: `export function mountPresets({ button, panel }, { invoke, onPick })` returns `{ open, close, isOpen }`; the first `open` calls `kit_presets`, builds one `button.kt-preset` per preset (a canvas plus the name, `data-id`), evaluates each design once through `kit_evaluate` and draws it; Esc, a click outside the panel and a pick close it; a pick calls `onPick(preset)`. `export function tileModel(presets)` returns `[{ id, name, parts: design.parts.length }]` (pure, for the test).
  - `store.load(next, path)` also sets `ui.pointEdit = null`, `ui.pointSel = null`, `selectedGroup = null`, `ui.folded = []`.
  - Status after a pick: `Preset <name>: <n> parts, all editable`.

- [ ] **Step 1: Write the failing node tests**

`thumb.test.mjs`: `thumbLayers` on a synthetic evaluation (two solid intervals at z1 0.1 and -0.2 as unit squares, one glass interval) yields three layers, the first fill equals `depthShade(colour, 0.1, 0.1)`, the second is darker (different string), the last has the glass alpha, and every point lies within `[0, 96]`.
`presets.test.mjs`: `tileModel` on two fake presets returns ids, names and part counts in order.
`store.test.mjs`: `load_resets_the_editing_state`: with `pointEdit` set, a selected group and a folded group, `load(design)` leaves `ui.pointEdit null`, `ui.pointSel null`, `selectedGroup null`, `ui.folded []`.
`help.test.mjs`: the Toolbar names `Presets`.

- [ ] **Step 2: Run to verify they fail**

Run: `node --test crates/gui/ui/kit/tests/*.test.mjs`
Expected: FAIL on the four new tests.

- [ ] **Step 3: Implement `thumb.js`, `presets.js`, the store reset, the help row, the markup and the wiring**

kit.html: `<button class="btn small ghost" id="kt-presets" title="Start from a finished note: five presets in the rim style">Presets</button>` after `kt-new`; `<div id="kt-presets-panel" class="kt-popover" hidden></div>` as the first child of `#kt-root` (positioned under the toolbar; `kt-popover` in kit.css: absolute, panel background, border, radius, a 5-column grid of tiles with the canvas above the name). main.js: `actions.loadPreset = exclusive(async (preset) => { if (!(await discardOk("Preset"))) return; const next = parseDesign(JSON.stringify(preset.design)); store.load(next, null); store.setStatus(\`Preset ${preset.name}: ${next.parts.length} parts, all editable\`); })`; `mountPresets({ button: $("kt-presets"), panel: $("kt-presets-panel") }, { invoke, onPick: actions.loadPreset })`. Help row: `["Presets", "Five finished notes in the rim style (Target, Hex frame, Crosshair, Diamond frame, Notched ring): a click replaces the note with it as plain parts, asking first when there are unsaved changes."]`. Harness: `case "kit_presets": return JSON.parse(await run(["--presets"]));`.

- [ ] **Step 4: Run the node suite**

Run: `node --test crates/gui/ui/kit/tests/*.test.mjs`
Expected: PASS.

- [ ] **Step 5: Write and run the scenario group `presets`**

Checks: `the panel lists five tiles in order` (names), `thumbnails are drawn` (each tile's canvas has non-zero alpha somewhere, via `getImageData`), `a preset replaces a clean note` (Target: 3 parts named Outer ring, Inner ring, Dot; `path null`; `dirty false`; design name Target; status), `a dirty note asks first` (nudge a part, `dlg({ ask: false })`, pick Hex frame: parts unchanged; `dlg({ ask: true })`, pick again: 2 parts), `escape and an outside click close the panel`, `a preset picked during point editing ends it` (double-click a part, pick Crosshair: `pointEdit null`). Run: `node crates/gui/ui/kit/tests/scenarios.mjs presets` and expected `6 passed, 0 failed`; run `files` again for the toolbar width.

- [ ] **Step 6: Commit**

```bash
git add crates/gui/ui/kit/thumb.js crates/gui/ui/kit/presets.js crates/gui/ui/kit/tests/thumb.test.mjs crates/gui/ui/kit/tests/presets.test.mjs crates/gui/ui/kit.html crates/gui/ui/kit.css crates/gui/ui/kit/main.js crates/gui/ui/kit/store.js crates/gui/ui/kit/help.js crates/gui/ui/kit/tests/store.test.mjs crates/gui/ui/kit/tests/help.test.mjs crates/gui/ui/kit/tests/harness.mjs crates/gui/ui/kit/tests/scenarios.mjs
git commit -m "kit: the Presets popover, thumbnails drawn from the designs"
```

---

### Task 6: Remove the v1 designer, Rust side

**Files:**
- Create: `crates/render/src/kit/v1.rs` (from `crates/render/src/designer.rs:37-100` and `329-354`: `ControlPoint`, `default_smooth`, `flatten_controls`, `clean_contours` with the private helpers they use)
- Delete: `crates/render/src/designer.rs`, `crates/render/examples/designer_pack.rs`, `crates/render/examples/designer_plan_json.rs`, `crates/render/examples/designer_preview_png.rs`, `crates/render/examples/game_probe_skins.rs` (the four examples built on the v1 mesh builder; the probe archives stay under `/home/killer/RhythiaRenderer/game-probes/`, new probes come from `kit_render_png --rhs`)
- Modify: `crates/render/src/lib.rs:11` (drop `pub mod designer`), `crates/render/src/kit/mod.rs` (`pub mod v1;`), `crates/render/src/kit/{doc,points,resolve,migrate,shapes}.rs` (`use crate::kit::v1::...`)
- Modify: `crates/gui/src/imports.rs` (`kit_import_glyph`; delete `designer_import_svg`, `designer_import_glyph`, `svg_to_contours` if nothing else uses it; the two tests at lines 425 and 583)
- Modify: `crates/gui/src/main.rs` (delete `open_designer_window` 4313-4387, `DesignerShape` 4389-4463, `designer_outline`, `designer_preview`, `designer_read_image`, `designer_doc_save`, `designer_doc_load`, `designer_export` 4556-4635 and 4871-4997; rename `designer_scene` to `preview_scene`, `designer_renderer` to `preview_renderer`; delete `designer_skin` and the designer branch of the kit window's destroy handler; the registration list)
- Modify: `crates/gui/capabilities/default.json` (the `designer` window)
- Modify: `crates/gui/src/kit_commands.rs` tests (the old-skin fixture built with `zip`)

**Interfaces:**
- Produces: `rhythia_render::kit::v1::{ControlPoint, flatten_controls, clean_contours}` with unchanged signatures; `#[tauri::command] pub async fn kit_import_glyph(ch: String) -> Result<Vec<Vec<[f32; 2]>>, String>` (the body of today's `designer_import_glyph`). No `designer_*` command remains registered.

- [ ] **Step 1: Write the failing tests**

In `kit_commands.rs`, rewrite `a_saved_skin_opens_again_and_other_skins_say_what_they_are` to build the "old designer skin" with a test helper `fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8>` (zip::ZipWriter, stored) holding `config`, `noteSkin/x.obj`, `noteSkin/note.mtl` and `rhythr/design.json` = `OLD_DESIGNER_FIXTURE`, and the "plain" skin as the same without the design entry; assertions unchanged (`migrated` true for the old one, the refusal sentence for the plain one, which now reads `this skin carries no editable design: only skins saved by rhythr's Note Designer open here`).
In `imports.rs`, the two tests that call `plan_note_multi` instead build a `Design` with one `Shape::Contours { contours, label }` part and assert `levels::evaluate(&design)` succeeds with a non-empty first part.
Add `fn the_glyph_command_outlines_a_letter()` calling the sync body of `kit_import_glyph` (factor `glyph_contours` is already pub) only if it does not exist yet.

- [ ] **Step 2: Run to see them fail or still compile against the old names**

Run: `cargo test -p rhythia-gui kit_commands imports`
Expected: compile errors or failures until the move is done.

- [ ] **Step 3: Move and delete**

Create `kit/v1.rs`, repoint the five `use` lines, delete `designer.rs`, the four examples and the `lib.rs` line; in `imports.rs` rename the glyph command and delete the two designer commands; in `main.rs` delete the listed items, rename the two survivors, drop `designer_skin` from the state and from the destroy handlers (the kit handler drops the renderer when its own window goes), update the registration list; remove the `designer` window from `capabilities/default.json`.

- [ ] **Step 4: Build, test, check Windows**

Run: `cargo fmt --all && cargo test -p rhythia-render --lib kit:: && cargo build -p rhythia-render --examples && cargo test -p rhythia-gui && cargo xwin check -p rhythia-gui && cargo clippy -p rhythia-gui -p rhythia-render --all-targets 2>&1 | grep -c "^warning: unused"`
Expected: all PASS; no new unused warnings (dead helpers left behind by the deletion are removed, not allowed).

- [ ] **Step 5: Grep gate**

Run: `grep -rn "designer" crates/gui/src crates/render/src --include=*.rs | grep -v -i -E "kit/v1.rs|migrate|// |//!|converted|older"`
Expected: no output (comments and the migration wording may remain).

- [ ] **Step 6: Commit**

```bash
git add -A crates/render/src crates/render/examples crates/gui/src crates/gui/capabilities
git commit -m "gui: remove the v1 note designer's window, commands and mesh builder; keep the v1 helpers the migration needs"
```

---

### Task 7: Remove the v1 designer, front end, and the name "Note Designer" for the kit

**Files:**
- Delete: `crates/gui/ui/designer.html`, `crates/gui/ui/designer.js`, `crates/gui/ui/designer.css`
- Modify: `crates/gui/ui/index.html:125-126`, `crates/gui/ui/app.js:1247-1252`
- Modify: `crates/gui/ui/kit.html:5,41,105`, `crates/gui/ui/kit/render.js:595`, `crates/gui/src/main.rs` (the kit window title `rhythr · Note Designer`)
- Modify: `crates/gui/ui/kit/main.js` (delete `importOld`, its binding and import; the glyph call uses `kit_import_glyph`; the conversion status text), `crates/gui/ui/kit/files.js` (delete `OLD_DOC_KEY`, `importOldDesigner`; header comment), `crates/gui/ui/kit/help.js:22` (row goes), `crates/gui/ui/kit/tests/files.test.mjs:78-87`, `harness.mjs` (`kit_import_glyph`; `designer_import_svg` case goes), `scenarios.mjs` (`files`: `importOldGone`; the converted-file status text)
- Modify: `crates/gui/src/kit_commands.rs` (the refusal sentence from Task 6 step 1 if not done there; `Loaded` doc comment)

**Interfaces:**
- Produces: the main window has one button `#btn-kit` labelled `Note Designer` (title `Design your own note from parts: add, cut, depth, glass, effects; one .rhs for rhythr and the game`) opening the kit; `#btn-designer` is gone. The kit's titles read `rhythr · Note Designer` (window, `<title>`, `document.title` base) and the help dialog heading `Note Designer controls`. The conversion status reads `Opened and converted from an older design format: <file>; Save will ask for a new .rhs file`.

- [ ] **Step 1: Write the failing tests**

`files.test.mjs`: delete `the_old_designer_autosave_migrates_through_the_backend`; add `the_module_exports_no_old_designer_import` asserting `importOldDesigner` and `OLD_DOC_KEY` are `undefined` on the module namespace (`import * as files`). `help.test.mjs`: no row names `From old designer`. `scenarios.mjs` `files` group: `importOldGone: !document.getElementById("kt-import-old")` in the toolbar check and the converted status assertion updated to `converted from an older design format`.

- [ ] **Step 2: Run to verify they fail**

Run: `node --test crates/gui/ui/kit/tests/files.test.mjs crates/gui/ui/kit/tests/help.test.mjs`
Expected: FAIL.

- [ ] **Step 3: Delete, rename, rewire**

Also `git rm` the three designer files; `build.rs` copies `ui/` recursively, so nothing else changes for the bundle.

- [ ] **Step 4: Run the node suite and the scenario runner**

Run: `node --test crates/gui/ui/kit/tests/*.test.mjs && cargo build -p rhythia-render --example kit_eval_json && node crates/gui/ui/kit/tests/scenarios.mjs`
Expected: all PASS (every group, including `send` and `presets`).

- [ ] **Step 5: Grep gate for user-facing words**

Run: `grep -rn -i -E "note kit|old designer|designer_|kt-import-old|OLD_DOC_KEY" crates/gui/ui crates/gui/src --include=*.html --include=*.js --include=*.mjs --include=*.rs --include=*.json`
Expected: only code comments and the runner's own comments remain; no string a user can see, no command name. Then `grep -rn "designer" crates/gui/ui/index.html crates/gui/ui/app.js` is empty.

- [ ] **Step 6: Commit**

```bash
git add -A crates/gui/ui crates/gui/src
git commit -m "gui: the kit is the Note Designer; the v1 window's files and the import of its autosave go"
```

---

### Task 8: Polish from the Phase 3 review

**Files:**
- Modify: `crates/gui/ui/kit/store.js` (`createGroup` name; the `DISCRETE` labels), `crates/gui/ui/kit/render.js` (the waves Depth range; the folded header marker), `crates/render/src/kit/migrate.rs` (the stale comment in the fit test), `crates/gui/ui/kit/tests/{store,render}.test.mjs`, `crates/gui/ui/kit/tests/scenarios.mjs` (`groups`: the fold check counts evaluate calls)

**Interfaces:**
- Produces: picking a waves style is a discrete commit (label in `DISCRETE`); the waves Depth slider spans 0.02 to 0.95 (step 0.01); a part selected inside a folded group marks its header row with the class `has-selected`; `createGroup` names a new group `Group N` with the smallest N no group uses.

- [ ] **Step 1: Write the failing tests**

`store.test.mjs`: extend `toggles_never_merge` with two consecutive style picks (two undo steps); `a_new_group_takes_the_smallest_free_name` (groups `Group 1` removed, a new group is `Group 1` again, with `Group 1` present a new one is `Group 2`). `render.test.mjs`: the Depth slider's `max` attribute is `0.95` for a part with waves; a folded group whose member is selected renders its header with `has-selected`. `scenarios.mjs` `groups`: the fold check also asserts `s.calls.filter(c => c === "kit_evaluate").length` unchanged across fold and unfold.

- [ ] **Step 2: Run to verify they fail, implement, run again**

Run: `node --test crates/gui/ui/kit/tests/*.test.mjs && node crates/gui/ui/kit/tests/scenarios.mjs groups effects`
Expected: PASS. Fix the migrate test comment and run `cargo test -p rhythia-render --lib kit::migrate`.

- [ ] **Step 3: Commit**

```bash
git add crates/gui/ui/kit/store.js crates/gui/ui/kit/render.js crates/gui/ui/kit/tests/store.test.mjs crates/gui/ui/kit/tests/render.test.mjs crates/gui/ui/kit/tests/scenarios.mjs crates/render/src/kit/migrate.rs
git commit -m "kit: review polish, discrete style picks, the full wave depth range, folded selection marker, free group names"
```

---

### Task 9: Guide, changelog and the parent spec

**Files:**
- Modify: `docs/USER-GUIDE.md:166-243` (the designer bullets), `:244-359` (the controls table), `:230-235` (the import note)
- Modify: `CHANGELOG.md:8-15` (the Unreleased designer entry)
- Modify: `docs/superpowers/specs/2026-10-24-note-kit-2-5d-design.md:384-412` (sections 8 and 9)

**Interfaces:**
- Consumes: `helpMarkdown()` from `help.js` (run `node -e 'import("./crates/gui/ui/kit/help.js").then(m => console.log(m.helpMarkdown()))'` to regenerate the table). `crates/gui/README.txt` is regenerated from the guide by `build.rs` on the next build; it is not edited by hand.

- [ ] **Step 1: Rewrite the guide**

The bullet `**Note Designer** (button in the toolbar)` describes the kit, folding in the present Note Kit paragraph (parts, Add and Cut, Depth and Glass, SVG and Letter imports, point editing, groups, effects, symmetry, snapping, previews, 3D, help, the one-file save) plus two new sentences: `Presets` (five finished notes, a click replaces the note, asks when unsaved) and `Send to game` (what it writes, the profile `rhythr-kit`, pick once, apply again after later sends, where it looks when it fails). The v1 bullet and the sentence `The Note Designer stays until the kit is complete.` go. The heading becomes `### Note Designer controls`; paste the regenerated table. The import note stays and gains, in front of it, a bullet `**Testing a note in the game**` that points at Send to game and keeps the network path warning for a manual import.

- [ ] **Step 2: Rewrite the changelog entry**

Replace the `**A note designer.**` entry with one that describes the kit as shipped (window, parts, Cut, depth, Glass, effects, groups, snapping, SVG and Letter imports, point editing, five presets, Send to game, one `.rhs` with the design inside, old files convert). The hit sound and mesh skin entries stay.

- [ ] **Step 3: Update the parent spec**

Section 8 phase 4 becomes one paragraph pointing at the Phase 4 design document and listing what landed; section 9 becomes `## 9. Decisions (closed)` with the three recorded decisions (depth range [-2.0, 0.5]; the v1 slider mode as quick-shape buttons only; new parts at [-0.1, 0.1]) and the owner's 01.11. rulings (no underlay, no twist, v1 designer removed).

- [ ] **Step 4: Check and commit**

Run: `bash scripts/preflight.sh && grep -n -i "note kit\|old designer" docs/USER-GUIDE.md CHANGELOG.md README.md`
Expected: preflight passed; the grep finds nothing.

```bash
git add docs/USER-GUIDE.md CHANGELOG.md docs/superpowers/specs/2026-10-24-note-kit-2-5d-design.md
git commit -m "Guide, changelog and spec: the Note Designer is the kit; presets and Send to game"
```

---

### Task 10: Acceptance, testbuild kit8 and the release checklist

**Files:**
- Create: `docs/superpowers/plans/2026-11-01-note-kit-phase4-ledger.md`
- Modify (release step only, after the owner's Go): `Cargo.toml`, `crates/gui/tauri.conf.json`, `CHANGELOG.md` (date), `packaging/aur/` via the existing scripts

- [ ] **Step 1: The full suites**

Run, one after the other: `cargo fmt --all --check`, `cargo test -p rhythia-render` (debug, the property test included), `cargo test -p rhythia-gui`, `node --test crates/gui/ui/kit/tests/*.test.mjs`, `node crates/gui/ui/kit/tests/scenarios.mjs`, `cargo xwin check -p rhythia-gui`, `bash scripts/preflight.sh`, the Linux container check as in Phase 1 (`podman` image `rhythr-linux-build`, `cargo check -p rhythia-render`).
Expected: everything green; record the counts in the ledger.

- [ ] **Step 2: The Windows testbuild**

From `crates/gui`: `/home/killer/tools/tauri-cli/node_modules/.bin/tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc` (a fresh worktree needs `crates/gui/ui-dist` first; a copy of `ui/` is enough). Copy the installer to `/home/killer/RhythiaRenderer/rhythr_0.6.3_x64-setup_TESTBUILD-kit8.exe`.

- [ ] **Step 3: The ledger and the owner's probe round**

Write the ledger with the per-task results and the rulings, then hand the owner the probe list of spec section 8 (three skins in play with screenshots and the glass grey; Send to game first and second send and which reload step works; the two-key profile; the single designer button and a converting v1 file; the presets in the game). Findings go back into Task 3's status text (`MSG_AGAIN`) and into section 4.1's fallback before the release step.

- [ ] **Step 4: Commit the ledger**

```bash
git add docs/superpowers/plans/2026-11-01-note-kit-phase4-ledger.md
git commit -m "Ledger: note kit phase 4 execution and the owner's probe round"
```

- [ ] **Step 5: Release candidate (only after the owner's Go; nothing here is pushed before it)**

Version 0.7.0 in `Cargo.toml` and `crates/gui/tauri.conf.json`, `## v0.7.0 (date)` in the changelog with the Unreleased entries, `bash scripts/preflight.sh 0.7.0`, the Linux build through `scripts/build-linux.sh`, the Windows build, `latest.json`, then the GitHub release and AUR exactly as for 0.6.3 (the AUR push script runs on the owner's side).

---

## Owner test round 1 (01.11., 20:12, testbuild kit8)

The owner found the Presets panel running past the window's right edge (fixed in 88f9c0a: the panel is pulled inside the window and wraps in narrow ones, pinned by the scenario check "the panel stays inside the window" at 1700 px), asked for a grouped, uniform toolbar with colour per group (variant A of the mockups of 20:30: labelled groups Add, Import, Edit, View, File, Game; one button style; Send to game filled), and for the diagnostics and the error code table to cover the designer and the game, with the fix for each code in the table and in the diagnostics file. Rulings: the live evaluate error in the status line is feedback, not a failure, and stays unstamped; everything a button does (save, open, import, send) stamps.

### Task 11: Toolbar, variant A

**Files:** `crates/gui/ui/kit.html` (the header), `crates/gui/ui/kit.css`, `crates/gui/ui/kit/tests/scenarios.mjs` (`files`: the toolbar check), `docs/USER-GUIDE.md` (one sentence in the designer bullet).

**Produces:** six groups in the toolbar in this order, each `div.group` with a class and a `span.label`: `add` ("Add": the six shapes), `import` ("Import": SVG, the Letter field, Letter), `edit` ("Edit": Undo, Redo), `view` ("View": Previews, 3D, Help), the `grow` spacer, `file` ("File": New, Presets, Open, Recent, Save .rhs, Save as), `game` ("Game": Send to game). Every toolbar button is `btn small` without `ghost`; the colour sits on the group: label and button borders tinted (add: accent, import: `#c8a3ff`, view: `--warn`, game: `--ok`; edit and file neutral); `#kt-send` carries `game` and is filled with `--ok` and dark text; a 1 px separator between groups; the `.on` outline stays. The scenario check `toolbar` asserts the labels in order, no `ghost` button in the toolbar, `#kt-send` filled (computed background not transparent) and `scrollW <= 1100`.

- [ ] Step 1: extend the `files` toolbar check (labels, ghost count, send fill); run `node crates/gui/ui/kit/tests/scenarios.mjs files`, expected FAIL.
- [ ] Step 2: rewrite the header markup and the styles; run again, expected PASS; run `presets` and `send` too (they click toolbar buttons).
- [ ] Step 3: the guide sentence; preflight; commit `kit: the toolbar in six labelled groups, one button style, colour per group`.

### Task 12: Error codes and diagnostics for the designer and the game

**Files:** `crates/errcode/src/lib.rs` (two areas, fourteen entries, tests), `docs/ERROR-CODES.md` (regenerated), `crates/gui/src/kit_commands.rs`, `crates/gui/src/imports.rs` (stamping, the last send and last design records), `crates/gui/src/main.rs` (`App` fields, the diagnostics sections, tests), `docs/USER-GUIDE.md` ("Reporting a problem").

**Produces:**
- `Area::Designer` (tag `DSN`, title "Note Designer", fallback `RH-DSN-700`) with `RH-DSN-701` nothing to show (`nothing to show`), `702` too detailed (`too detailed`), `703` too complex (`too complex`), `704` reaches too far (`reaches too far`), `705` not a design file (`not a design file`, `design file damaged`), `706` no editable design (`carries no editable design`), `707` no font (`no installed font`, `type a letter`), `708` SVG (`svg`), `709` the skin file could not be written (`os error`, `denied`); `Area::Game` (tag `GAM`, title "Sending to the game", fallback `RH-GAM-800`) with `RH-GAM-801` folder not found (`game folder not found`), `802` relocated (`moves its data to`), `803` could not write (`could not write`). Each entry has a `fix` sentence and causes per system. The fallback test and the markdown area list include both areas; `real_messages_land_on_the_right_code` gains one case per new entry with the real wording.
- Stamping: `kit_export`, `kit_doc_load`, `kit_shape_to_points`, `kit_import_svg`, `kit_import_glyph` wrap their `Err` with `stamp_in(Area::Designer, ..)`; `kit_send_to_game` stamps `SendError::Design` in the Designer area and everything else (the not-found sentence included) in the Game area. `kit_evaluate` and `kit_mesh` stay unstamped.
- `App` gains `last_send: Mutex<Option<(std::time::Instant, String)>>` and `last_design: Mutex<Option<String>>` (name, parts, groups, triangles, and the path or "sent"); `kit_export` and `kit_send_to_game` set them.
- `diagnostics_report`: under each recorded error a line `      try: <fix>` from `rhythia_errcode::find`; a section `note designer` (last design, last save or send with "n s ago"); a section `game` built by `game_section(candidates: &[PathBuf]) -> String`: one line per candidate (`found` or `not there`), and for the first found folder `active profile: <file name>`, `DataPath: (empty)` or the value redacted, and `rhythr-kit.json`, `skins/notes/rhythr-kit.obj`, `.mtl`, `skins/colorsets/rhythr-kit.txt` each as `present (<bytes> bytes, <n> min ago)` or `missing`; Steam libraries found, redacted. Tests: `game_section` over a temp fixture (found, active profile, DataPath empty, files missing then present after a `send_to_game`), the `try:` line after a stamped designer error, the two new section titles in the report.
- Guide: the "Reporting a problem" paragraph names the two new areas and the two new sections.

- [ ] Step 1: errcode tests first (fallback list, classifier cases); `cargo test -p rhythia-errcode`, expected FAIL; the entries; PASS; `RHYTHR_UPDATE_DOCS=1 cargo test -p rhythia-errcode`; commit `errcode: codes for the Note Designer and for sending to the game`.
- [ ] Step 2: diagnostics tests first (`cargo test -p rhythia-gui diagnostics`), expected FAIL; the fields, the stamping, the sections; PASS; the full gui suite and the scenario groups `files`, `imports`, `send` (the status prefixes still hold with a code appended); preflight; commit `gui: designer and game errors carry codes; the diagnostics file shows the game folder, the last send and what to try`.
- [ ] Step 3: the guide paragraph; preflight; commit; testbuild kit9.
