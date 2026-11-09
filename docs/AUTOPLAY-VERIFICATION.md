# Checking a video for the Autoplay mark

For the Rhythia team, and for anyone the owner hands the key to. One page.

## What the mark is

rhythr can render a map without a replay (Autoplay: a synthetic cursor that
plays every note). Every such video carries up to three marks:

1. A visible ticker across the top: `AUTOPLAY   rendered with rhythr   not a real play`,
   plus the movement style and the speed mod. It is on by default and the
   person rendering can switch it off (Autoplay tab, Banner; CLI
   `--no-banner`), so its absence says nothing about a video. Anyone with a
   video editor can paint over it in any case.
2. A hidden mark in the picture: a faint noise pattern derived from a secret
   key, added to every frame. Measured with this checker it survives
   re-encodes down to 480p and 1 Mbit/s, cropping, and strong denoising
   (weaker); the prototype also survived sharpening, colour changes, a codec
   change to H.265 and small rotations. Only someone with the key can find it.
3. A metadata comment in the file (`rhythr autoplay render: not a real play`).
   The first thing a re-encode drops; it is there for the honest cases.

## Running the check

You need `rhythr-cli` (part of every rhythr release, next to the app) and the
key file the owner gave you (one line of 64 hex characters). Keep the key
file private: whoever has it can verify, and could also remove the mark.
The check reads the video through ffmpeg: the CLI finds the one the rhythr
installer put next to the app, one placed next to `rhythr-cli` itself, or
one on PATH; `--ffmpeg <path>` points at another.

```
rhythr-cli mark-check video.mp4 --key rhythr-autoplay-mark.key
```

Add `--json` for the numbers. The exit code is 0 when the mark is found, 2
when it is weak, 1 when there is none, so a script can branch on it. The
check reads every third frame, up to 3000 frames (`--max-frames` changes
that); 40 seconds of 1080p60 take about 25 seconds on a laptop-class CPU,
most of it the geometry search, so a longer video costs little more.

## Reading the result

`z` is how far the mark's match stands above the matches the same video
gives at wrong geometries (other scales and rotations of the pattern). The
checker says found above 10, weak above 6, none below. Wrong keys are a
separate control: `wrong_keys_max` in the JSON is the best match three wrong
keys reach, for comparison with `score`.

The numbers below were measured with this checker on a real map (40 seconds
of play rendered at 1080p60) treated the way platforms and editors treat a
video:

| Treatment | z | Verdict |
| --- | --- | --- |
| untouched render | 83 | found |
| re-encoded at 1080p, 8 Mbit/s | 83 | found |
| re-encoded at 720p, 2.5 Mbit/s (a platform's treatment) | 74 | found |
| re-encoded at 480p, 1 Mbit/s | 63 | found |
| 20 percent cropped and scaled back to 1080p | 141 | found, scale 1.25 |
| 720p plus strong denoising (hqdn3d 4:3:6:4.5) | 19 | found |
| black crush (every luma below 22 set to 16) | 4 | none |
| a video without the mark | 2 to 4 | none |

- `Autoplay mark found: z 73.6, score 0.177, scale 1.000, angle 0.0 deg, key rhythr-autoplay-mark.key`
  The video was rendered by rhythr's Autoplay with that key. `scale` and
  `angle` say how the picture was resized or rotated since; `offset` in the
  JSON says how it was shifted or cropped.
- `Autoplay mark: weak (z 7.1, ...)`: something is there but the video was
  degraded a lot. Treat as "probably Autoplay", ask for the original upload.
- `No Autoplay mark (best z 3.9, key built-in dev)`: nothing found with the
  keys tried. This proves nothing by itself (see the limits).
- `key built-in dev`: the video was rendered by a build without the owner's
  key (a contributor's own build). It is still an Autoplay video.
- `score` is the match itself: 0.20 untouched, 0.15 to 0.18 after a
  platform re-encode, 0.06 after strong denoising. `wrong_keys_max` and
  `null_mean` sit around 0.02 whatever the video; a found mark stands well
  clear of both.

## What it proves and what it does not

A found mark proves the video came out of rhythr's Autoplay with that key.
A missing mark does not prove a real play:

- A deliberate black-crush filter (every near-black pixel set to pure black)
  removes the hidden mark; the ticker and the tag are gone long before.
- Someone who compiles rhythr from source without the owner's key gets no
  owner mark (the ticker is a setting in any build).
- The key sits inside every release binary (something has to generate the
  pattern), obfuscated but not beyond a determined engineer.

The final word on any score is the Rhythia server: a real play has a replay
and a leaderboard entry there, an Autoplay video has neither. Use the check
to confirm a suspicion quickly, not to clear a video.

## Reporting

When a result matters, keep the exact file you checked (hash it) and the
`--json` output. Questions and false positives go to the owner (KillerOp007
on GitHub), with the file if possible.
