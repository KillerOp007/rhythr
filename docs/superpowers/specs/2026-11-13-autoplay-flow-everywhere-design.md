# Autoplay: the Flow aim everywhere, and a dance without spikes (design, 2026-11-13)

Branch `flow-aim` from release 0.8.0. Planner: crates/render/src/autoplay/path.rs (plan, plan_run,
plan_wait, plan_end, phrase_runs, phrase_knots, arc_piece), generator: crates/render/src/autoplay/mod.rs
(generate: waypoints, chords). Hit matcher unchanged. Follows the stream design of 2026-11-08.

## Problem (measured)
Owner: the Flow aim was meant to make the whole run flow, not only its streams; no third movement style;
the dance may be improved with the same means.
Measured on one map (4688 notes, 802 jumps, two long pauses), 0.8.0 against a reference video of an
automatic player (30 fps, cursor tracked in every frame, calibrated to the field):
- Jumps (125 to 250 ms). 0.8.0 Normal: straight lines, the cursor turns on the note, median 143 degrees
  within 24 ms, and passes it at a third of its pace between notes. 0.8.0 Dance: one belly per jump to a
  side the dice pick, so two bellies meet in a spike, median 127 degrees. The reference rides arcs of the
  circle through three notes in a row (median 0.03 units from the nearer circle, 0.36 from the chord),
  bulging about half a unit away from the next note, at one pace (14 units/s on the note, 17 between).
- Pauses. 0.8.0 keeps one pace over the whole gap (a crawl towards the next note from the first frame
  on) and stands still in the centre from the rest setting on. The reference carries on past the last
  note, waits moving slowly (0.6 to 1.1 units/s) and starts its way in about 0.31 s before the note.
- Streams: on a par already (the stream design of 2026-11-08). Left alone.
- Notes on one spot: 0.8.0 stands still between them. A double inside a slide (two notes 16 ms apart,
  then a unit hop in 16 ms) alternates between 0 and 60 units/s from frame to frame.

## Design
The implementation is our own on the existing sweep planner; no figure of the reference is copied.

1. Phrases. With the Flow aim or the dance a run is no longer a stream with its one jump in and out: it
   is every stream, jump, hold and (dance) boomerang in a row, from pause to pause (`phrase_runs`). One
   curve, one taut string. Normal movement with the Centre aim keeps the old runs and the old code path:
   its output is unchanged frame for frame (checked against 0.8.0 on a real map).

2. A jump is an arc (`arc_piece`). The heading through a note between two jumps is the tangent of the
   circle through the note and its neighbours: each hop's direction weighted by the other hop's length.
   The handles of the cubic are as long as a circular arc's for the angle `a` between heading and chord,
   `2 / (3 (1 + cos a))` of the chord, capped at ARC_REACH_MAX 0.4 (a reversal bulges 0.3 of the hop, a
   quarter turn 0.2), and they stop at the barrier, so the curve stays inside the field without sliding
   along the wall. At an end of the run the arc is the one that meets the inner note's heading (the
   heading mirrored across the chord). A turn sharper than 107 degrees blends towards the heading across
   the turn (the two hop directions added), fully from 143 degrees: without it a long hop next to a
   short one ends in a hairpin. A reversal (162 degrees and more) is a loop as in a stream, sides
   alternating. A note between two jumps is passed through its centre (no corner cut; that stays a
   stream's affair), within JUMP_TUBE 0.08 along the path at its time.

3. The dance is the same line swung wider. Its old belly per chord (BELLY_PER_UNIT of the distance,
   more with time to spare, times the amount) is now a swing per arc: where the pattern turns, the
   handles grow by DANCE_REACH times the swing; where it runs straight, the heading through the note is
   turned by the angle an arc of that belly leaves its chord with (at most DANCE_TURN_MAX), to the side
   the dice pick per note, so a straight row of jumps weaves. A boomerang is two arcs through a knot of
   its own at its far end, inside the phrase: no stop before it, none after. A note next to a gap of
   PAUSE_SECS or more is passed exactly on time (nothing to even out); the run winds up from its first
   piece only when a stream follows it.

4. Pauses, Flow aim only. A gap of PAUSE_SECS 0.6 real seconds or more (a rest too) is not a hop:
   - the way out (`plan_wait`): one cubic from the last note along the cursor's momentum (a handle of a
     third of speed times LEAVE_SECS, at most MOMENTUM_MAX and 0.6 of the room to the barrier), round to
     the waiting spot; when the spot lies behind the cursor the second control point moves out to the
     side (HOOK_WIDTH of the handle, at least HOOK_MIN), a hook instead of a line out and back. The pace
     falls from the phrase's own to the drift's;
   - the waiting spot: WAIT_TOWARDS 0.45 of the way from the centre to the next note; the centre itself
     from the rest setting on and before the first note;
   - the wait: a figure of eight through the spot (DRIFT_LONG by DRIFT_WIDE), entered along the heading
     the cursor arrives with, at DRIFT_SPEED 0.8 units per real second. The cursor never stands;
   - the way in: the last APPROACH_SECS 0.3 (at most half the gap). It is the first piece of the phrase
     behind the pause, an arc from where the drift is, so every rule of the run applies to it (the
     circle through the first notes, loops at a reversal, the string). Its pace starts at the drift's and
     ends at the pace behind the first note, covering exactly the string's arc (`Profile::smoothed` with
     `lead`); the first note is reached on time, on its centre. A note with nothing behind it but
     another pause is landed on at LAND_SPEED.
   After the last note the cursor carries on and comes to rest in the centre within a real second.

5. Notes on one spot, Flow aim only: the first is passed a little before the centre on the way in, the
   last a little behind it on the way out, at the pace of the hops around the spot (at least SPOT_SPEED),
   at most SPOT_DRIFT 0.2 from the centre. A reversal on the spot is taken across it. A chord's shared
   spot and the ends of a run stay where they are.

6. Chords, Flow aim only: a chord whose notes fit in a box no wider than a hit area (a corner of three,
   a block of four) is hit from the middle of that box, like the pair of neighbours with either aim.

Invariants (compile-time): CORNER_CUT_MAX + TUBE + RAMP_DEV and SPOT_DRIFT + TUBE + RAMP_DEV stay below
HITBOX_HALF - HIT_MARGIN.

## As built (2026-11-13)
Same map, 240 fps, 0.8.0 against this branch:

| | 0.8.0 | branch |
|---|---|---|
| turn on a jump's note, Normal + Flow (median, degrees) | 143 | 17 |
| turn on a jump's note, Dance (median, degrees) | 127 | 12 |
| pace on a jump's note against between notes (median) | 0.32 | 1.00 |
| sideways bulge in the middle of a jump (mean, units) | 0 | 0.48 (reference 0.49) |
| largest per-frame speed step, Normal + Flow (p99.9, units/s) | 31.8 | 4.4 |
| largest per-frame heading change, Normal + Flow (p99.9, degrees) | 150 | 53 |
| stream measures (corner offset, speed steadiness) | | unchanged |

Checked on 287 ranked maps in all four combinations at 1.00x and 1.45x (wild dance at the faster one):
every note registers everywhere, and with the Flow aim every flag frame has the cursor inside its note's
hit area. With the Centre aim two of the maps have a few flags outside it, in chords too wide for one
spot that are swept note by note; 0.8.0 has the same on the same maps with either aim, and the Flow aim's
shared spot for three and four removes it there. Tests: arcs and pace through jumps, no spike in the dance at any amount with
either aim, the pause (momentum, drift, late way in, rest and lead-in in the centre), the end, notes on
one spot, chords of three and four, and random phrases of every kind of gap in every combination.

Limits: the reference was one video of one map at 30 fps. The constants are tuned by eye and by these
measures, not fitted.
