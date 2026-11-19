# Telling an Autoplay video from a real play

For the Rhythia team, and for anyone who wants to know. One page.

rhythr can render a map without a replay (Autoplay: a synthetic cursor that
plays every note). Such a video is meant as a showcase of a map, and it is
marked as what it is:

1. **A visible ticker across the top**:
   `AUTOPLAY   rendered with rhythr   not a real play`, plus the movement
   style and the speed mod. It is on by default. The person rendering can
   switch it off for a cleaner picture (Autoplay tab, Banner; CLI
   `--no-banner`), so its absence says nothing about a video.
2. **A metadata comment in the file**:
   `rhythr autoplay render: not a real play`. `ffmpeg -i video.mp4` or any
   media info tool shows it. A re-encode drops it, and so does an upload to
   a video platform, so it only helps with the original file.

There is nothing else: no hidden mark in the picture and no checker.

## What settles a question

- **The Rhythia server.** A real play has a replay and a score there. An
  Autoplay run has neither: rhythr never writes a replay file, the run
  exists only in memory while it renders. Ask for the replay; rhythr's
  integrity check (`rhythr-cli verify replay.rhr --map <map>`) then says
  whether its hits follow from its input frames.
- **The movement.** The Autoplay cursor never misses and runs on smooth
  computed curves, without the small corrections and the jitter of a hand.
- **The run itself.** The accuracy is 100 percent with a full combo, where
  the HUD shows a player name it reads `Autoplay`, and there is no results
  screen and no score card at the end.

None of this is an official verification. rhythr is an unofficial
community tool, and the Rhythia team's own records are the final word on
any score.

## Videos made with rhythr 0.8.0

0.8.0 put a faint noise pattern into every Autoplay frame as a hidden mark.
It showed on some screens as a grain on black, so later versions leave the
picture alone and carry no checker. The `rhythr-cli` of 0.8.0 (still on
that version's release page) reads the mark of 0.8.0 videos with
`rhythr-cli mark-check video.mp4 --key <key file>`; the key file comes from
the owner.

## Questions

Questions go to the owner (KillerOp007 on GitHub).
