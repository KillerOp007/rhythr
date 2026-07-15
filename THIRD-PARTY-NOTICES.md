# Third-party notices

## Bundled components

- **ffmpeg** (Windows builds ship `ffmpeg.exe`, invoked as a separate
  process) — GPL; license text ships next to the app as
  `ffmpeg-LICENSE.txt`. Sources: https://ffmpeg.org and
  https://github.com/BtbN/FFmpeg-Builds
- **DejaVu Sans Bold** (embedded HUD font) — Bitstream Vera license
  (with DejaVu additions, public domain); text ships next to the app as
  `FONT-LICENSE.txt` and lives in the repo at
  `crates/render/assets/hud-font-LICENSE`.

## Format references (no code vendored)

The `.rhr`, `.sspm` and skin formats were implemented from scratch and
verified against these MIT-licensed projects; their work made the
byte-level verification possible:

- gerhaarrd/rhr2mp4 — https://github.com/gerhaarrd/rhr2mp4 (MIT)
- yo-ru/rhrParse — https://github.com/yo-ru/rhrParse (MIT)
- Rhythia/sound-space-plus — https://github.com/Rhythia/sound-space-plus (MIT)

## Test fixtures

The map files under `testdata/` (note data + metadata, no audio or
cover art) are charts authored by Rhythia community mappers — credited
in each file's `Mappers` field (mm1678YT, Alia) — and published on
rhythia.com. They are included solely as test fixtures for the parser
and integrity checks and are **not** covered by this repository's MIT
license; all rights remain with their authors. The replays are the
project owner's own plays.

Rhythia is a game by Capo Games; rhythr is an unofficial community
tool. This project bundles no game assets; the optional asset
extraction reads the user's own installation locally at the user's
request (file-based only — it never attaches to, injects into, or
otherwise interacts with the running game), and extracted assets stay
on that user's machine. API usage was agreed with the Rhythia team
(July 2026).
