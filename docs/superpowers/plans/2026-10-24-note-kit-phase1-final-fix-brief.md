# Final fix wave (one dispatch) after the whole-branch review of a740d9a..c27c958

Every item below is a controller ruling. Do them in this order, each with its pinning test, then the checks at the end. Where an item says "verbatim", the exact behaviour is spelled out; where it gives a threshold, use that number. No em or en dashes anywhere. Nothing under `docs/superpowers/`.

## 1. Critical: v1 freeform migration (`crates/render/src/kit/migrate.rs`)

v1's editor expands the stored wedge into ONE closed control loop before the backend splines it: `controlLoop()` in `crates/gui/ui/designer.js:223-245`. The sector is the wedge, preceded by its x-axis reflection reversed when `mirror` is on; the sector is then rotated `symmetry` times by `2 * pi * s / symmetry` for `s` in `0..symmetry`; `smooth` flags are kept; the loop needs at least 3 points. The backend then deduplicates points that land exactly on the axis (read `crates/gui/src/main.rs` around line 4363 and `crates/render/src/designer.rs` for the exact rule and mirror it: consecutive exactly equal points collapse to one).

The migration instead stored the wedge alone as `Shape::Points` with `repeat = symmetry` and `mirror = X`, which makes `symmetry` separate closed splines of the wedge: a four-fold design becomes four disjoint quarter blobs and a two-point wedge becomes nothing (`KitError::Empty`). Also `symmetry` defaults to 1 where v1 defaults to 4 (`designer.js:42`).

Fix: port `controlLoop()` into `migrate.rs` as a private function; store the expanded loop as `Shape::Points { control }`; set no `repeat` and no `mirror` for freeform; default a missing `symmetry` to 4; a wedge whose expanded loop has fewer than 3 points falls through to the slider branch as before (same as v1 returning null). Keep the v1 twist handling as it is.

Tests (replace `freeform_keeps_symmetry_and_mirror_as_effects`):
- `freeform_symmetry_expands_to_one_loop`: two wedge points `(0.2, 0.0)` and `(0.3, 0.3)` (smooth true, false), symmetry 4, no mirror: `Shape::Points` with 8 control points at the four rotations (check two of them numerically to 1e-5), `effects.repeat == None`, `effects.mirror == Mirror::None`.
- `freeform_mirror_reflects_and_reverses`: the same wedge with mirror: 16 points; the first four are the reflection of the wedge in reverse order followed by the wedge (check the first point is `(0.3, -0.3)` and the third is `(0.2, 0.0)`), then rotated copies.
- `migrated_freeform_evaluates_to_one_contour`: the four-fold migrated design from the first test evaluates (through `crate::kit::levels::evaluate`) to one interval whose solid is one shape with one contour; and a two-point wedge with symmetry 4 does not give `KitError::Empty`.

## 2. Important: the mtl always declares both materials (`crates/render/src/kit/export.rs`)

`mtl_text` writes `Material.001` and `Transparent` always (drop the `with_glass` parameter or ignore it; keep the signature if other code calls it, and say which). Every kit skin then bundles an identical `noteSkin/note.mtl`, so the import order of several skins cannot leave a `usemtl Transparent` undefined. Update `mtl_names_match_the_game_convention` and `the_archive_always_carries_the_mtl_and_names_glass_only_when_present`: the obj assertions stay (a solid-only obj has no `usemtl Transparent`), the mtl assertions flip (both archives contain `newmtl Transparent`). Rename the second test to `the_archive_always_carries_both_materials`.

## 3. Important: Star points cap (`crates/render/src/kit/shapes.rs`)

`let n = (*points).clamp(2, 64) as usize;` with a comment that a corrupt document must degrade, never allocate without bound. Test `a_corrupt_star_point_count_is_capped`: `Shape::Star { points: u32::MAX, inner: 0.5 }` returns one contour whose corner count before resampling is at most 128 (assert the resampled contour has at most, say, 128 + 256 points and the call returns in well under a second).

## 4. Important: raw copy pre-check before `clean` (`crates/render/src/kit/resolve.rs`)

In `resolve_part`, after building the symmetry copies and before `clean`, count the raw points over all copies; if the count exceeds `8 * MAX_PART_POINTS`, return `Err(KitError::Complexity(raw_points, copies.len()))`. Comment: the lattice drops collinear points, so the multiple stays generous and the clean still sees any source that could pass the cleaned budget. Test `an_overly_large_copy_set_is_refused_before_clean`: a 40_000-point circle (`Shape::Contours`) with `repeat = Some(8)` is `Err(KitError::Complexity(..))` and returns quickly (no multi-second clean).

## 5. Important: a wall lower bound before the subdivision (`crates/render/src/kit/mesh.rs`)

Every boundary edge of an interval's region becomes a wall quad (two triangles) for that interval, so `triangles >= 2 * (sum of all contour points over all intervals, solid and glass)`. In `build_mesh`, compute that sum before any subdivision and return `Err(KitError::Budget(bound))` when `bound > MAX_TRIANGLES`. No valid design changes behaviour: such a mesh would have failed `validate` with `Budget` after minutes of work. Test `a_mesh_over_budget_is_refused_before_subdivision`: an `Evaluation` built by hand (one interval, levels `[-0.1, 0.1]`, solid = one circle contour of 21_000 lattice-snapped points via `snap`, glass empty, parts empty, scale 1.0) gives `Err(KitError::Budget(_))` in well under a second.

## 6. Important: the box cut fails loudly beyond the chord tolerance (`crates/render/src/kit/levels.rs`)

Extract a helper `fn cut_to_box(shapes: Shapes, id: &str) -> Result<Shapes, KitError>`: if `extent(&shapes) > 1.0 + BOX_SLACK` return `Err(KitError::Invalid(format!("part {id} grew beyond the box after rounding: {e}")))` where `const BOX_SLACK: f32 = 0.02;` with the derivation in the comment (round joins at L/R = 0.2 have a sagitta of r(1 - cos 0.1), about 0.005 r, two offsets at MAX_FINISH 0.9 give at most 0.009, so 0.02 is twice the honest overshoot); if `extent > 1.0` intersect with the unit square; else return unchanged. Call it from the finish loop. Tests: `a_chord_overshoot_is_cut_quietly` (a hand-made square region with half-extent 1.01 comes back clipped to 1.0) and `a_runaway_finish_is_an_error` (half-extent 1.2 gives `Err(KitError::Invalid(_))`). Fix the comment of `a_finished_part_never_leaves_the_box`: design 1 falls back to its unfinished outline (area 0.909), it does not stay the closed pill pair. Add to the `Evaluation` doc comment that `parts` carries clipped removers when `scale > 1`.

## 7. Important: validator precision and tests (`crates/render/src/kit/validate.rs`)

Compute the edge differences in f32 (exact by Sterbenz for nearby points) but the products and the cross product in f64, then compare with `MIN_AREA2 as f64`. Tests: the existing degenerate tests keep passing; `ranges_must_tile_the_index_buffer_in_triangles` sets `m.solid = 0..4; m.glass = 4..36;` so the tiling branch is the one that fails (message contains "tile"); new `an_open_edge_in_the_glass_range_is_reported_as_glass`: the cube with its last triangle moved into the glass range (solid `0..33`, glass `33..36`) then that triangle dropped (`indices.truncate(33)`, glass `33..33`, and the solid range now has an open edge), expect the message to contain "solid: open edge"; and a second variant where the glass range keeps the single triangle (solid `0..33`, glass `33..36`): expect "glass:" in the message. Doc comments: on `validate` the check order and the error variants; on `check_range` the precondition that indices are bounds-checked, that orientation is checked for consistency not outwardness (`caps_face_outward` in mesh.rs pins outwardness), and that the balanced rule admits non-manifold contacts and a surface emitted twice.

## 8. Minor: trimmed, non-blank skin names (`crates/render/src/kit/export.rs`)

`check_name` also requires `name.trim() == name` and a non-blank name. Extend `unsafe_names_are_refused` with `" "`, `" a"` and `"a "`.

## 9. Minor: geom comments and the area-factor pin (`crates/render/src/kit/geom.rs`)

Module doc: vertex-for-vertex sharing holds for `boolean_keep_collinear` (faces); the region paths drop collinear points and micro loops. `MIN_SHAPE_AREA` comment: the threshold is in whatever units the call runs in (design units in `resolve_part`'s `clean` and the remover clip, note units after normalisation), so a design smaller than about 0.03 in extent before normalisation loses detail; Phase 2 keeps transforms in the editor's ranges. Test `the_area_threshold_is_an_area_not_twice_one`: a square of side 0.0009 (area 8.1e-7) is dropped by `clean`, a square of side 0.0011 (area 1.21e-6) is kept.

## 10. Minor: renderer comment, property comment, groups in the property

`crates/render/src/renderer.rs` around line 1166: the comment says `mtllib/usemtl` are ignored; say that a missing `.mtl` is tolerated and `usemtl` order sets the glass alpha. `crates/render/tests/kit_properties.rs`: the config comment says debug costs ten times release; measured 2.5. In `design()`, for designs with at least two parts, with probability 1/4 put two distinct random parts into one group of random mode (`Group { id: "g".into(), name: "g".into(), parts: vec![id_a, id_b], mode }`); keep the rest unchanged.

## 11. Minor: user guide (`docs/USER-GUIDE.md`)

Replace "Desktop and Downloads both work" with wording like "any folder on a local drive, such as a subfolder of your user folder on C:".

## Checks, then commits

`cargo fmt --all`; `cargo test -p rhythia-render` (debug; the 10-case property included); `cargo clippy -p rhythia-render --all-targets` with no new findings in the files you touched; `scripts/preflight.sh`; because item 10 changed the property strategy, one release run of 20 cases (throwaway copy of the config, restored afterwards) and record the wall time. Commit in two commits: (a) items 1 to 7 as "Kit: v1 freeform migrates as one loop, bounds before the heavy steps, a loud box cut, both materials always", (b) items 8 to 11 as "Kit: name trimming, comments, area-factor pin, groups in the property". Both with the trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Then regenerate the four probe archives with `cargo run --release -p rhythia-render --example kit_render_png -- <json> <png> --rhs <rhs>` for the four JSON files in `/home/killer/RhythiaRenderer/game-probes/kit/` (overwrite the .png and .rhs there) and confirm each new .rhs's `noteSkin/note.mtl` names both materials.
