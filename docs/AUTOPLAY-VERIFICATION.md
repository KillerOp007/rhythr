# Checking a video for the Autoplay mark

For the Rhythia team, and for anyone the owner hands the key to. One page.

## What the mark is

rhythr can render a map without a replay (Autoplay: a synthetic cursor that
plays every note). Every such video carries three marks:

1. A visible ticker across the top: `AUTOPLAY   rendered with rhythr   not a real play`,
   plus the movement style and the speed mod. Anyone can see it, and anyone
   with a video editor can paint over it.
2. A hidden mark in the picture: a faint noise pattern derived from a secret
   key, added to every frame. It survives YouTube and Discord re-encodes,
   resizing, cropping, denoising, sharpening, colour changes, a codec
   change to H.265 and small rotations. Only someone with the key can find it.
3. A metadata comment in the file (`rhythr autoplay render: not a real play`).
   The first thing a re-encode drops; it is there for the honest cases.

## Running the check

You need `rhythr-cli` (part of every rhythr release, next to the app) and the
key file the owner gave you (one line of 64 hex characters). Keep the key
file private: whoever has it can verify, and could also remove the mark.

```
rhythr-cli mark-check video.mp4 --key rhythr-autoplay-mark.key
```

Add `--json` for the numbers. The exit code is 0 when the mark is found, 2
when it is weak, 1 when there is none, so a script can branch on it. A video
of 30 seconds takes about half a minute on a laptop; longer videos are read
up to 3000 frames (`--max-frames` changes that).

## Reading the result

- `Autoplay mark found: z 206, score 0.321, scale 1.015, angle 0.2 deg, key rhythr-autoplay-mark.key`
  The video was rendered by rhythr's Autoplay with the owner's key. `z` is
  the distance from what wrong geometries and wrong keys score; untouched
  videos sit around 300, heavily processed ones around 100 to 200. `scale`
  and `angle` say how the picture was resized or rotated since.
- `Autoplay mark: weak (z 7.1, ...)`: something is there but the video was
  degraded a lot. Treat as "probably Autoplay", ask for the original upload.
- `No Autoplay mark (best z 1.3, key built-in dev)`: nothing found with the
  keys tried. This proves nothing by itself (see the limits).
- `key built-in dev`: the video was rendered by a build without the owner's
  key (a contributor's own build). It is still an Autoplay video.

## What it proves and what it does not

A found mark proves the video came out of rhythr's Autoplay with that key.
A missing mark does not prove a real play:

- A deliberate black-crush filter (every near-black pixel set to pure black)
  removes the hidden mark; the ticker and the tag are gone long before.
- Someone who compiles rhythr from source without the owner's key gets no
  owner mark; the ticker is in the source too and can be removed there.
- The key sits inside every release binary (something has to generate the
  pattern), obfuscated but not beyond a determined engineer.

The final word on any score is the Rhythia server: a real play has a replay
and a leaderboard entry there, an Autoplay video has neither. Use the check
to confirm a suspicion quickly, not to clear a video.

## Reporting

When a result matters, keep the exact file you checked (hash it) and the
`--json` output. Questions and false positives go to the owner (KillerOp007
on GitHub), with the file if possible.
