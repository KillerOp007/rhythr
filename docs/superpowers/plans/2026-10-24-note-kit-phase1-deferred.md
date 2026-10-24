# Deferred minors collected from the per-task reviews (for the final whole-branch review)

Every item below was seen by a task reviewer, judged Minor or out of scope at the time, and parked by the controller. The final reviewer triages: which are worth the one fix wave, which are recorded for Phase 2, which are not defects. Line numbers are as reported at the time and may have moved.

## Tasks 1 to 4 (doc, migrate, geom, shapes)
- Wire-format tests for the v2 document (a JSON fixture per shape and effect) are missing; serde defaults are asserted only indirectly.
- Comment provenance: a few comments restate what the code does rather than the constraint.
- `Shape::Star { points }` has no upper cap at the doc level (resolve caps the loops, not the point count of a star).
- Parameter clamps live in the generators and in resolve, not in the document model; a corrupt file is accepted and silently clamped.
- Migration edge cases: a v1 document with unknown fields, with an empty shape list, with a negative twist.
- `offset` re-snaps with FillRule::NonZero where Positive would also do; both are consistent with the orientation contract.

## Task 5 (resolve)
- The complexity budget measures the cleaned region, so `clean` runs once on a raw source of any size before the budget fires (an O(n) raw point pre-check before clean, or a file-size limit in the loader, would refuse earlier).
- `finish_part` checks the budget on its input, not its output (arc points from round joins and the rim's hole can exceed it; only MAX_TRIANGLES bounds it later).
- A NaN wave depth passes `clamp` and reaches `clean` (JSON cannot carry NaN; in-process only).
- Test gaps: `WaveStyle::Spike` and `Pulse`, `Mirror::Y` and `Mirror::Both` have no dedicated assertions beyond the counts.

## Task 6 (levels)
- When `scale > 1.0`, `Evaluation.parts` carries the clipped removers, not their full outlines (no consumer yet; Phase 2's editor must know).
- Review question answered by the box rule, recorded: exact half-extent 1.0 after rounding is not required; never larger is.

## Task 7 (validate)
- `ranges_must_tile_the_index_buffer_in_triangles` never reaches the tiling branch (the cover check fires first); that branch is the only guard before the slice.
- The glass range is never exercised by the validator's own tests (deleting the glass `check_range` call would pass them); the mesh tests cover it end to end.
- Untested arms: `MIN_AREA2` in both directions, `Budget`, the z range, index out of range, the "orientation" message.
- A precondition comment on `check_range` (indices already bounds-checked) and a doc comment on `validate` stating the check order and error variants.
- The cross product is computed in f32 (noise about 1e-7 on edges of length 2); switch to f64 if slivers ever get rejected.
- The validator checks consistent, not outward, orientation; `caps_face_outward` in mesh.rs pins outwardness.
- The balanced edge rule (Task 8 fix) also accepts a surface emitted twice.

## Task 8 (mesh)
- geom.rs module doc still says "contours shared between neighbouring operations line up vertex for vertex", which is false for the default `boolean` (only `boolean_keep_collinear` guarantees it).
- No test takes the bounding-box skip path or the levels/intervals guard.
- No intermediate complexity cap in the subdivision (faces times intervals booleans; only MAX_TRIANGLES bounds the output).
- Per-face overlays never see the neighbouring face, so a rounding interaction near a seam could in theory split the two sides differently; the validator and the property oracle are the backstops.
- The `count` half of `triangulate_face`'s return tuple is unused in production.

## Task 9 (export)
- The name rule accepts a blank or space-padded name (`" "` gives `noteSkin/ .obj`); add a non-blank requirement.
- Zip and io failures surface as "mesh invalid: zip: ..." because `KitError::Invalid` is formatted "mesh invalid: {0}".
- When no colour is valid, no `colorSet/` entry is written but the config still names one (v1 parity; a dangling reference).
- A glass-only design yields a single `usemtl Transparent` section and no `Material.001`; whether the game draws it translucent or opaque is answered by probe `kit4_glass_only` (Task 12).
- `to_obj` stays pub and unguarded by design (doc comment only).

## Task 10 (renderer)
- Stale comment at renderer.rs around 1165 says "mtllib/usemtl are ignored"; `usemtl` order now sets the glass alpha.
- Third-party obj files with interleaved material blocks draw glass before solid, and the depth write hides the solid behind it (cheap fix: the loader appends later-material triangles after material-0 triangles).
- Unreferenced `v` lines no longer enter `normalize_to_unit`'s bounds (plan-mandated by the `vertices.len() == 6` test); confirm against the game or record the change.
- `HashMap<(u32, usize), u32>` key cost in the loader (a `(u32, u32)` key or per-material `Vec` would be lighter).
- Coverage gaps: `usemtl` returning to the first material, faces before any `usemtl`, a bare `usemtl` with no name.

## Task 11 (property, micro-loop filter, box cut)
- The box cut silences the defect class it was added for: fail with `KitError::Invalid` when a finished part's extent exceeds 1.0 by more than the chord bound (about 0.005 at MAX_FINISH 0.9 with L/R 0.2) and clip only below it; a disc blown up inside the box is caught by nothing.
- The regression test comment says design 1 "stays the closed pill pair"; it falls back to its unfinished outline (area 0.909).
- `MIN_SHAPE_AREA` is documented in note units but `resolve_part`'s `clean` and the remover clip run in design units, so a design below extent 0.03 before normalisation loses detail (fix the comment, or skip the filter in resolve and snap even at scale 1.0).
- The 0.1 grid rarely produces flush seams between different parts (about 1e-4 per pair); a generator that places a second part exactly flush would make the comment true.
- The property draws repeat <= 4 and waves <= 8 while the kernel accepts 32; `KitError::Complexity` on high-end designs is unexplored (cost ruling).
- Pin the area-versus-twice-area factor with squares of side 0.0009 and 0.0011.
- Stale numbers: commit 2490805's subject says "three hundred"; the config comment says debug costs ten times release (measured 2.5).
- Nobody counted how many of the 200 cases reach the oracle (Empty about 8 percent, Budget share unknown); a counting test over the strategy asserting most designs reach it.
- Class C ("edge used 1 times but reverse 2 times", seen only while shrinking before the micro-loop fix) has no reproduction; absence in 200 cases is the only evidence it is gone.
