# Autoplay: streams planned as one flow (design, 2026-11-08)

Branch `autoplay` from commit 8dc65fe onward; code in `crates/render/src/autoplay/` (as built, see the end).
Planner: crates/render/src/autoplay/path.rs (plan, sample, Segment, stream_curve, in_sweep),
generator: crates/render/src/autoplay/mod.rs (generate: waypoints, chords, frames, hit flags),
hit matcher: crates/sim/src/hitreg.rs (match_hits, HITBOX_HALF 0.56875, BASE_HIT_WINDOW_MS 55 * speed mod,
cursor-guided reattribution in phase 2). Grid spacing 1.0 unit, field bound ~1.37.

## Problem (measured)
Owner: "bei Streams sieht es an jeder Note snappy aus, soll ein fliessender Uebergang sein, gleichmaessige
Geschwindigkeit wenn sie sich nicht aendert, nicht von Note zu Note huepfen; wir planen pro Note statt pro Pattern".
Lionheart map (2897 notes, 2204 in streams, gaps 41 and 84 ms, hops mostly 1.0 with 2.0 and 2.5 mixed in).
Current planner (8dc65fe): passes exactly through every note centre at the exact chart time; stream segments
are Catmull-Rom cubics with control points 1/3 of the chord along the neighbour direction; even pace per
segment.
- Direction change at each stream note (heading 20 ms before vs after): median 58 deg, p90 69 deg.
  Real players (replay pass_long_score77, 2002 stream notes): median 15.6 deg, p90 31.6 deg.
- Real players pass the note centre at a distance: closest approach p50 0.26, p90 0.55 (units); at the
  chart time they are p50 0.56 away (they hit within the 55 ms window, median 37 ms late). The hitbox half
  width is 0.569 per axis.
- Speed inside Lionheart streams for the current planner: coefficient of variation p50 0.145, max 0.37
  (changes come from 2.0 hops between 1.0 hops and 1/16 <-> 1/32 changes, each hop gets its own speed).
  In the owner's video section 5 to 9 s the speed is already uniform (19 units/s +-13 %), so the visible
  snap there is the turning at every note, not the speed.

## Design
A "sweep run" = maximal range of consecutive waypoints joined by stream gaps (real move within
STREAM_GAP_SECS 0.125 real seconds), extended by one ordinary segment on each side (the jump into and
out of the stream), skipping zero-length (same spot) segments when looking for that neighbour. Rests
(>= rest_secs) and long same-spot gaps (boomerangs) are never part of a run. Everything outside runs is
planned as today.

Per run, two steps:

1. Spatial: knot positions Q_i. Inner stream knots are moved off the note centre by the uniform cubic
   B-spline knot offset (P_{i-1} - 2 P_i + P_{i+1}) / 6, clamped to CORNER_CUT_MAX 0.3 units (collinear
   notes: no offset; 90 deg corner with unit hops: 0.236 inward; 180 deg reversal: 0.333 -> 0.3).
   The run's end knots (the ordinary notes before and after) stay exact. Through the Q_i a Catmull-Rom
   chain as today (control points 1/3 of the chord along the direction Q_{i+1} - Q_{i-1}, straight
   fallback when undefined). Arc length of the chain is tabulated per piece (existing Segment arc table).

2. Temporal: let S_i be the cumulative arc length at knot Q_i and t_i the chart times. Find the
   monotone time->arc function S(t) that is as uniform as possible subject to |S(t_i) - S_i| <= TUBE
   (0.25 units) at inner knots and exact at the two end knots: the taut string through that tube
   (shortest path in the (t, S) plane). It is piecewise linear: constant speed over stretches, the speed
   changes only at knots where a tube wall forces it (a 2.0 hop among 1.0 hops, a 1/16 to 1/32 change).
   The pass time t'_i of knot i is the time at which S(t) = S_i; the plan's Segment for piece
   [Q_i, Q_{i+1}] covers [t'_i, t'_{i+1}] with even pace, which reproduces S(t) exactly because the string
   is linear between knots.

3. Hit flags: for a swept note the flagged frame is placed at the time of closest approach to the note
   centre P_i within +-min(window/2, half gap) of t_i (today: exactly t_i). Distance at that moment is at
   most CORNER_CUT_MAX 0.3 (inside the 0.569 hitbox, margin 0.27). Flag times stay monotone
   non-decreasing in note order. Non-swept notes keep t_i exactly.

Field barrier: sample() still clamps to the field; corner cuts point inward (away from the edge).
Dance and Normal are identical inside runs (as today). Determinism: no randomness in runs.

## Constants (tuning knobs)
STREAM_GAP_SECS 0.125, STREAM_TANGENT 1/3, CORNER_CUT_MAX 0.3, TUBE 0.25.

## Acceptance (tests to write)
- Lionheart, Lava Chicken, pass_long (5004 notes), hardrock map: every note registers (match_hits derived
  hits == notes, orphan flags 0) at speed mods 1.0, 1.5, 2.0.
- Ring stream (1.0 hops at 48 ms): turn at notes (20 ms before/after) median < 35 deg; closest approach
  <= 0.3; a line of notes stays a line (off-chord < 0.03).
- Mixed stream 1.0,1.0,1.0,2.0,1.0,1.0 at 48 ms: speed is constant over the 1.0 stretches (ratio of
  consecutive frame steps within 1.05 away from the 2.0 hop) and changes at most at two knots.
- 1/16 -> 1/32 change: speed changes once, not per note.
- Chord pair (0.5 ms same spot) does not start a run; a same-spot double inside a run is passed without
  leaving the run.
- Entry/exit: the jump into a run arrives tangent-continuous (existing test) and the dance arcs outside
  runs are unchanged (existing tests).

## As built (08.11.2026, after the hit-safety critique)
- Run detection: Gap classification (Degenerate, Rest, Still, Boomerang, Stream, Jump); a run is a maximal
  range of stream gaps plus holds between them plus a lone jump between two streams, extended by the one
  jump into it and out of it; runs never overlap (a bug in the first cut skipped every stream after the
  first on the Lionheart map).
- Knots: the ends exact; inner knots cut by the B-spline pull capped at CORNER_CUT_MAX 0.3; reversal tips
  (turn sharper than TIP_COS -0.95) cut by at most TIP_CUT 0.1 and given a direction across the line,
  alternating sides (a jack becomes a narrow ellipse); knots marked `exact` (a chord's shared spot, the
  run's start) are not cut and get a zero tube.
- Tube: TUBE 0.18 along the arc, but never more than SLACK_MS 25 of the slower neighbouring hop's time
  (a 1 s jump into a stream cannot arrive 150 ms early). Taut string through the tube, then a speed profile
  that replaces every slope change by a ramp of up to RAMP_MS 150 centred on it, shortened so the ramp
  moves the cursor at most RAMP_DEV 0.04 off the string's arc and never reaches across or sits on an
  exact knot (a step with no room stays a step). Pieces carry the profile as pace knots; Segment::at
  integrates it.
- Flags: at the chart time (never early; a note arms at its time), falling back to the first covered
  moment within window/2 after it. Invariant pinned at compile time:
  CORNER_CUT_MAX + TUBE + RAMP_DEV < HITBOX_HALF - HIT_MARGIN (0.52 < 0.539).
- Tests: geometric coverage at every flag (per axis, the game's test with the planner's margin; the chord
  rule's own limit for chord notes), flags monotone and never early, identity attribution; random streams
  (48 cases, chords, jitter, four speed mods, both movements); real maps pass_long and hardrock at
  0.8/1.0/1.5/2.0; ring corners, line, long hop, rhythm ramp, jack loop, chord inside a stream, many
  streams per map, entry acceleration.
- Measured (planner on Lionheart, 2136 stream notes): closest approach p50 0.24, turn at note p50 41
  degrees (was 58), p90 52 (was 139 with the first cut), max 104; speed CV p50 0.06 (was 0.18); every note
  covered at its flag, none early. On pass_long: turn p50 24 (was 39, players 16), CV 0.015.

## Round 3c (critique of the two remaining lenses, 08.11. 10:07)
Both reviewers verified that the worktree already carried the must-fixes (ramps, time-capped tube, exact
chord knots, one-sided flags, the budget assert, per-axis coverage tests). Adopted from their should-consider
lists:
- Wind-up into a run (algorithm + motion): the first piece of a run that is a jump (the start of the map
  counts) no longer gets a symmetric ramp capped at RAMP_DEV; it holds a slower pace (rests in the centre
  at the start) and accelerates linearly over up to WIND_UP_FRACTION 0.6 of the piece so the stream's
  pace is reached exactly at the first stream note, with the arc there conserved (the flat pace is solved
  for it). Test: a_map_that_opens_with_a_stream_winds_up_from_the_centre and the string/profile unit
  tests in path.rs.
- Time-weighted knot direction (motion): dir = (Q[j+1]-Q[j])/dt_out + (Q[j]-Q[j-1])/dt_in, so a long jump
  in does not drag the heading at the first stream note.
- Symmetric corner cut (motion): off = (w_hat - u_hat) * min(|u|,|w|) / 6, so a corner next to a long hop
  is cut like any other instead of being pulled into the long hop.
- Constants in real seconds (algorithm): SLACK_SECS 0.025 and RAMP_SECS 0.15 scale with the speed mod like
  STREAM_GAP_SECS and the hit window.
- Reversals right after the start in the centre are tips too (the earlier exclusion made the entry piece
  cusp when the second note is the centre).
- Unit tests for the taut string and the profile (uniform stream is one straight span; the long hop bends
  at the ceiling before and the floor after; an exact knot pins the string and the profile keeps its arc;
  the wind-up waits and reaches the stream's pace at the first note).
Not adopted (noted for a later round): slowing the cursor at reversal tips (0.6 v at the tip, 1.4 v mid
loop); a decelerating ease after the last note of a run; STREAM_TANGENT 0.45 (measured on Lionheart:
frames turning more than 45 degrees 34 at a third vs 61 at 0.45, so a third stays).
