# macOS build

Status: test builds only. There is no Mac in the project; a community
tester runs what the GitHub Actions workflow produces, and this page is
updated from those reports. Nothing here is released.

## How it is built

`.github/workflows/build-macos.yml` runs by hand (Actions, "macOS build",
Run workflow, pick the branch, give the artifact a tag). On a macOS runner it
builds:

- a universal binary (Apple Silicon and Intel) with
  `tauri build --target universal-apple-darwin`;
- a static ffmpeg for both architectures from https://ffmpeg.martin-riedl.de
  (a GPL build; its licence text is bundled as `ffmpeg-LICENSE.txt`), joined
  with `lipo` and placed in `Contents/Resources`, where `resolve_ffmpeg`
  looks;
- an ad-hoc signature (`bundle.macOS.signingIdentity` is `-` in
  `tauri.macos.conf.json`): Apple Silicon refuses unsigned code outright, an
  ad-hoc signature lets it run, and Gatekeeper still asks (see below).

The result is `rhythr_<version>_universal_TESTBUILD-<tag>.dmg` as a workflow
artifact. `tauri.macos.conf.json` also turns the updater artifacts off:
without an Apple Developer account there is no notarized build to update to,
so updates on macOS are manual (download the next dmg).

## Opening a test build

The app is not notarized (that needs an Apple Developer account, 99 USD a
year). macOS blocks it on the first start:

1. Open the dmg and drag rhythr to Applications.
2. Start it. macOS says it cannot verify the developer.
3. macOS 15 (Sequoia) and newer: System Settings, Privacy & Security, scroll
   to the note about rhythr, click "Open Anyway", start it again. Up to
   macOS 14: right-click the app, Open, Open.
4. The alternative in Terminal, which also clears the bundled ffmpeg:
   `xattr -dr com.apple.quarantine /Applications/rhythr.app`

## Known gaps of the first test build

- No hardware encoder: rendering uses x264 (software). VideoToolbox can
  follow once the base works.
- Double-clicking a `.rhr` in Finder does not load it yet (macOS hands
  opened files to the app as an event, not on the command line); drop the
  file on the window or use Browse.
- The game data folder search knows `~/Library/Application Support` and
  `~/.config`; without the game on the Mac, point `RHYTHR_GAME_DATA_DIR` at a
  copied folder. Game assets from `rhythia.exe` need the Windows exe.
- The updater check runs and finds no macOS entry in the release feed; it
  may say so as an error.
