# Checking whether a video is an Autoplay render

For the Rhythia team, and for anyone who wants to know. One page.

## How an Autoplay video is recognised

rhythr can render a map without a replay (Autoplay: a synthetic cursor that
plays every note). Such a video is recognised by the path its cursor takes:

- The Autoplay path of a map is not recorded anywhere, it is computed from
  the map and the settings (movement, aim, dance amount, speed mod), and it
  can be computed again at any time.
- The checker computes the paths of the map for every setting, finds the
  cursor in the video frame by frame, and measures in how many frames the
  cursor sits on one of those curves.
- A player's hand passes the same notes at the same times, but not on the
  same curve: within a few pixels, frame after frame, only the machine does
  that.

Nothing is hidden in the picture, so nothing can be filtered out of it, and
it makes no difference which skin, cursor, background or colours the video
uses, or whether its ticker is on. The check needs no secret key.

Two more signs, both easy to lose, are there for the honest cases:

1. A visible ticker across the top: `AUTOPLAY   rendered with rhythr   not a real play`,
   plus the movement style and the speed mod. It is on by default and the
   person rendering can switch it off (Autoplay tab, Banner; CLI
   `--no-banner`), so its absence says nothing about a video.
2. A metadata comment in the file (`rhythr autoplay render: not a real play`).
   The first thing a re-encode drops.

## Running the check

You need `rhythr-cli` (on every release page next to the app's installers:
`rhythr-cli_<version>_x64.exe` for Windows, `rhythr-cli_<version>_amd64` for
Linux), the video as a file, and the map the video shows (any map file
rhythr reads: `.sspm`, `.rhm`, the game's cache JSON). The check reads the
video through ffmpeg: the CLI finds the one the rhythr installer put next
to the app, one placed next to `rhythr-cli` itself, or one on PATH;
`--ffmpeg <path>` points at another.

```
rhythr-cli autoplay-check video.mp4 --map song.sspm
```

- `--speed-mod 1.25` when you know the video's speed mod: only that one is
  tried, which is several times faster. Without it every speed mod from
  0.80 to 2.00 is tried.
- `--start 1:30` when the video does not begin where the map begins (a
  render of a part of the map): the map time at the video's first frame.
- `--json` for the numbers. The exit code is 0 when the path is found, 2
  when the result is unclear, 1 when it is not there, so a script can
  branch on it.
- `--max-frames` (default 7200, two minutes at 60 fps): how much of the
  video is looked at, from its start.

A 45 second clip at 1080p60 takes about 15 seconds with the speed mod given
and about 20 seconds without, on a laptop-class CPU; a 720p copy half of that.

## Reading the result

The number that decides is the share of frames in which the cursor lies on
the best path's curve, within 4 pixels (of a 1920 wide picture) and within
a frame or so of time. Only frames in which the path moves count: a cursor
at rest sits on any path that rests. The checker says found at 60 percent
or more, unclear from 25, not there below.

Measured with this checker on a real map (45 seconds rendered at 1080p60,
cursor dance), treated the way platforms and editors treat a video:

| Video | On the curve | Verdict |
| --- | --- | --- |
| untouched render | 97.5 % | found |
| re-encoded at 720p, 2.5 Mbit/s (a platform's treatment) | 96.8 % | found |
| re-encoded as VP9 at 720p, 2 Mbit/s | 96.7 % | found |
| re-encoded at 480p, 1 Mbit/s | 96.8 % | found |
| 15 percent cropped and scaled to 720p | 95.6 % | found |
| black crush (every luma below 22 set to 16) | 97.2 % | found |
| converted to 30 fps | 90.6 % | found |
| another skin on a bright photo background, no ticker | 95.8 % | found |
| the same, re-encoded at 720p | 96.6 % | found |
| normal movement, flow aim (another setting) | 96.6 % | found |
| a real play of the same map, rendered from its replay | no cursor stays on any path | not there |

- `Autoplay path found: 96.8 % of the frames on the curve (cursor dance medium, aim centre, 1.00x, key built-in owner; 2544 moving frames, median distance 1.9 px)`
  The cursor in the video runs on the Autoplay path of that setting. The
  settings named are the ones whose path fits best; neighbouring settings
  (another dance amount) often fit almost as well, so read them as "about".
- `Autoplay path: unclear, ...`: part of the video follows a path. Typical
  for a video that is cut together from several pieces, has an overlay
  over the playfield, or was rendered from a build with a changed movement.
  Ask for the original upload and look at it yourself.
- `No Autoplay path: ...`: the cursor does not run on any Autoplay path of
  this map (the real play above), or no cursor was found along one.
- `key built-in owner` or `key dev`: the key decides to which side some
  arcs of the path swing, nothing else. A release build carries the owner's
  key, a build someone compiled themselves the public development key. The
  paths of two keys differ in a few percent to a tenth of the time, so the checker
  names the key whose path fits best; a video that fits `dev` better was
  made with a self-compiled rhythr. It is an Autoplay video either way.
- `on_time_pct` in the JSON is the same share at each frame's exact time,
  without the allowance of a frame. It drops for a video whose frame rate
  was converted, while the share on the curve holds.

## What it proves and what it does not

A found path proves the cursor in the video is rhythr's Autoplay running on
that map. A path that is not found does not prove a real play:

- The check needs the right map. With another map nothing fits.
- It is tested for whole renders and for re-encodes of them. A video that
  starts somewhere in the middle needs `--start`; an intro in front of the
  render, a picture-in-picture layout or a mirrored picture are not handled.
- Someone who changes the movement in their own build of rhythr gets a path
  this checker does not know.
- Videos made with rhythr 0.8.0 took slightly different paths. They carry a
  hidden mark in the picture instead and are checked with
  `rhythr-cli mark-check video.mp4 --key <key file>` (the key file comes
  from the owner). 0.8.0 was the only version with that mark: it showed on
  some screens as a faint pattern on black, which is why it is gone.

The final word on any score is the Rhythia server: a real play has a replay
and a leaderboard entry there, an Autoplay video has neither. Use the check
to confirm a suspicion quickly, not to clear a video.

## For maintainers: the path is the proof

From the first release with this checker on, a change to the Autoplay
movement changes what the checker accepts. Videos of a released version
must keep verifying, so a release that changes any path keeps the previous
generator next to the new one and lets the checker try both. The test
`a_released_path_stays_where_it_is` in `crates/render/src/autoplay/check.rs`
fails when a path moves and says this again.

## Reporting

When a result matters, keep the exact file you checked (hash it), the map
file and the `--json` output. Questions and false positives go to the owner
(KillerOp007 on GitHub), with the files if possible.
