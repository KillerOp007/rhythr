# Note Kit (2.5D) design

Status: draft for owner review, 2026-10-24.

The Note Designer becomes a building kit: a note is assembled from
parts, each a 2D shape with its own depth band, combined by adding and
cutting, with effects and symmetry applied non-destructively. The kit
is "silhouette plus depth" by design, because that is everything the
game can show. Correctness is a property of the construction, not of a
mesher: every triangle comes from a 2D triangulation of a region an
integer-exact kernel has cleaned, and an in-house validator gates every
export.

## 1. Goals and non-goals

Goals

- Free composition: several parts per note, add and cut, groups, depth
  per part, glass parts, effects (waves, rounding, rim), symmetry
  (mirror, repeat), presets, SVG and glyph import as parts.
- Zero geometry glitches: watertight, consistently oriented meshes by
  construction; a validator that no broken mesh can pass; property tests
  that try to break it.
- What you see is what the game shows: the editor and the previews draw
  exactly the geometry the export contains, including the game's
  approach fade and its fixed glass transparency.
- Approachable for players: a parts list and an inspector, not a node
  graph; no 3D gizmo as the main interaction; no modelling vocabulary in
  the UI.
- The export stays a plain .rhs (one .obj, its .mtl, a colorset) that
  the Steam client and rhythr both load.

Non-goals

- Lit shading, bevels that shade, per-part colours: the game draws note
  meshes unlit with one tint per note (verified from its shaders), so
  none of it would ever be visible in play.
- Free 3D rotation of parts, side holes, twists: not expressible in
  2.5D and not visible head-on.
- SDF or mesh CSG kernels (fidget, truck, Manifold): researched and
  rejected for robustness or build-chain reasons; see the research
  notes in the owner's Dokumente folder.

## 2. What the game does (verified 2026-10-24 with probe skins)

| Fact | Evidence | Consequence |
|---|---|---|
| Note meshes are unlit; `vn` ignored; one tint per note | shaders `instanced_tint.*` extracted from rhythia.exe | silhouette, holes, depth order and perspective are the only visible freedoms |
| No back-face culling | probe 1: a disc with a reversed front cap draws as a full disc | winding does not matter in play; rhythr culls, so the kit still emits correct winding |
| Depth stacking works with parallax | probe 2: ring at the plane, plate recessed 0.5 behind, offset grows on approach | depth bands per part are a real design tool |
| Several `o` objects all draw | probe 3 | the exporter may merge or split objects freely; it merges |
| 10,500 triangles play without stutter | probe 4 | budget of 5,000 to 10,000 triangles per note is safe |
| Approach fade double-blends overlapping geometry | probes 5 and 6: an inlay over a disc shows only on far, faded notes | parts overlapping in screen space look brighter during the fade; the preview reproduces it |
| Any material after the first draws translucent at a fixed alpha; the .mtl `d` value is ignored | probes 7a to 7d: Glass at d 0.35, 0.7, 1.0 and "Transparent" at d 1.0 all look the same | glass is a switch, not a slider; measured alpha about 0.57 (probe 7a screenshot) |
| Archives import only from local paths | "File path is unsafe" from a UNC share | documentation note for users |
| A kit skin whose obj uses only `Transparent` draws translucent | kit probe 4 (glass-only disc, 24.10. evening): clearly darker notes | the game's "first material" is the first in the .mtl, not the first the obj uses; a glass-only design works; rhythr's loader must take the material order from the bundled .mtl (Phase 2), it keys on `usemtl` order today and shows such a skin opaque |
| Kit probes 1 to 3 pass in play | ring with recessed plate shows parallax; glass inlay translucent; the cut bar shows as a notch at the rim and, seen at an angle, as a line across the disc (its walls) | Phase 1 acceptance met; grooves in an unlit note are visible only through parallax and their walls, a design hint for the editor |

Coordinate convention of every built-in mesh: x and y half-extent 1.0
(the hard size cap), front toward the player at +z, depth extends
toward -z; the built-in circle spans z in [-0.1, 0.1], the cube reaches
-2.025.

## 3. Document model

The design document is one JSON value, versioned, and remains the single
format for undo snapshots, autosave, the .rhsd file and the IPC payload.

```
Design {
  version: 2,
  name: string,
  colors: [hex],                 // the colorset, order = beat order
  parts: [Part],                 // bottom to top in the panel
  groups: [{ id, name, parts: [part id], mode: Add | Cut }],
}

Part {
  id, name, visible, locked,
  shape: Circle
       | Square { rounding }     // a side sits flat at the bottom
       | Polygon { sides, rounding }
       | Star { points, inner }
       | Ring { thickness }
       | Pill { aspect }
       | Contours { contours, label }   // an SVG or glyph import, resolved by the editor, stored as oriented unit-scaled contours; the kernel never touches fonts or files
       | Points { control: [{x, y, smooth}], loops: [[{x, y, smooth}]] },   // freeform: the first closed loop and any further ones (holes clockwise, islands counterclockwise); any shape converts to this
  transform: { x, y, w, h, rotation },   // x, y, w, h in note-box units, rotation in degrees
  mode: Add | Cut,
  depth: { front, back },        // z band; front > back; front toward the player
  material: Solid | Glass,
  effects: {
    waves: { count, depth, style } | null,
    round: pct | null,
    rim: thickness | null,       // keeps only the outline band of this part
    mirror: None | X | Y | Both,
    repeat: count | null,        // copies around the centre
  },
}
```

Rules

- Depth bands live in [-2.0, 0.5] in note units. Defaults: a new part
  spans [-0.1, 0.1] like the built-in circle.
- The note box is fixed (owner decision, 24.10. evening, after the first
  test round): transform units are note units, nothing is ever rescaled,
  and whatever a part puts outside the unit box is cut away, like an
  artboard. The editor shows the box and the true hit area. A v1 design
  is fitted to the box once, at migration, because v1 normalised on
  every evaluation.
- A v1 design (the current .rhsd and autosave) loads as one Add part:
  slider shapes become a Polygon, Square or Circle part with the same
  effects, freeform becomes Points holding the loop v1 built from the
  wedge (reflection when mirrored, then rotated symmetry times, as v1's
  controlLoop did; no repeat or mirror effect is set, because repeat
  makes separate closed copies and v1 splined one loop), imports become
  Contours. Nothing anyone saved is lost.

## 4. Geometry kernel (Rust, crates/render/src/kit/)

The existing designer.rs kernel (i_overlay nonzero cleanup, erosion
rims, i_triangle caps, kurbo spline flattening, SVG and glyph import)
is kept and extended. The kit lives in a new module tree so the v1
pipeline keeps working until the editor switches over.

4.1 Part resolution: `resolve_part(part) -> Vec<Contour>`

1. Source shape to a unit-box outline (parametric generators, spline
   flattening, or imported contours).
2. Waves, in the part's own space.
3. Transform: scale by w and h, rotate, translate.
4. Symmetry: mirror produces reflected copies, repeat produces rotated
   copies; all copies are separate contours of the same part.
5. Clean with the nonzero rule into oriented contours, under a
   complexity budget per part (points and contours) so a corrupt file
   degrades instead of running away.
6. Round (closing then opening) and rim (region minus its erosion) are a
   finish in note units: the level evaluation applies them before the
   level set, on the merged region of the part, never deleting a part (a
   region too thin to erode keeps its outline). After the finish every
   part is cut to the unit box, quietly: a part may reach past the box
   or lie beyond it, and what lies outside is not part of the note. The
   finish comes first so a part's own corners round and its rim follows
   its own outline, while the box edge stays a straight cut. Region
   results drop shapes below a visible area, because the
   erosion leaves micro loops that a later dilation would blow up.

4.2 Level evaluation: `evaluate(design) -> Evaluation`

1. Level set: every distinct `front` and `back` across visible parts,
   sorted ascending. Intervals are consecutive pairs.
2. Per interval, per material, the solid region:
   `solid(i) = union(Add parts covering i) minus union(Cut parts covering i)`,
   with groups folded first (cuts in a group apply to that group's adds;
   an Add-mode group contributes its solid adds and its glass adds as two
   contributions of their own material, a Cut-mode group one cut of all
   its adds; a part id in more than one group is a document error).
   Glass is a second region per interval: `glass(i) = union(glass adds)
   minus union(all cuts) minus solid(i)`; opaque wins where they overlap.
3. Before the level set, every part is finished (4.1 step 6) and cut to
   the unit box. There is no normalisation: the silhouette is whatever
   the parts that add material cover inside the box, and a design with
   nothing inside the box is empty (KitError::Empty).
4. All overlay operations run through one fixed-origin integer lattice
   (i_overlay with a shared adapter: cell 1/65536, origin fixed, never
   recentred on the input) so identical input polygons produce
   bit-identical output vertices across operations. This is what makes
   face sharing exact.

4.3 Mesh emission: `mesh(evaluation) -> KitMesh`

- Faces of constant membership: per material, the plane is subdivided
  into faces that belong to the same set of intervals (start with the
  union of all interval regions, split every face by each region into
  the part inside and the part outside, carrying a membership mask).
  These overlays keep collinear vertices, so neighbouring faces share
  their seam edge for edge; the region paths elsewhere may drop
  collinear points and micro loops, the face path never does.
- Caps: each face is triangulated once (i_triangle, unchecked, with its
  indices mapped back to the face's own points) and lifted to every
  level where its membership changes: material below and not above is a
  front cap (counterclockwise seen from the player), above and not below
  a back cap (reversed).
- Walls: every face edge whose neighbour across the edge is not a member
  of an interval gets a wall quad for that interval; outer contours run
  counterclockwise and holes clockwise, so the face is always on the
  left and the wall faces outward.
- Sharing: every vertex is interned by its float bits in one pool, so a
  cap edge and a wall edge on the same seam at the same level are the
  same two indices and T-junctions cannot occur between faces that share
  a seam. A material has at most 64 intervals.
- Materials: the mesh carries index ranges per material (Solid first,
  Glass second). The exporter writes one `o`, `usemtl Material.001`
  for solid and `usemtl Transparent` for glass, matching the built-in
  cube's convention, which the game renders translucent.
- Budget: a typical design lands at 2,000 to 8,000 triangles. The
  exporter refuses above 40,000 with a clear message; the probe showed
  10,500 is comfortable and the margin is wide.

4.4 Validator: `validate(mesh) -> Result<Report, Defect>`

Deterministic, dependency-free, runs on every export and in every test:

- every directed edge is balanced by its reverse (used n times one
  way, n times the other: closed and consistently oriented, per
  material range), which admits non-manifold contacts such as two faces
  touching at a vertex at the same depth, and does not by itself detect
  a surface emitted twice or two coincident opposite walls (the
  property test's T-junction oracle covers those);
- no degenerate triangles (coincident positions under distinct indices;
  the threshold sits below the smallest lattice-sized triangle), no NaN
  or infinite coordinates;
- x and y within the unit box (1.0 plus 1e-4), z within [-2.0, 0.5];
- triangle count within budget.

A defect aborts the export with the defect named; it never produces a
"best effort" file, because the game crashes on broken skins.

4.5 Renderer support

rhythr's mesh loader keeps per-material index ranges, and the note draw
renders the Transparent range alpha-blended at 0.57 (measured from
probe 7a, to be re-measured once against a fresh screenshot during
acceptance). The approach fade already exists in the renderer; the
preview therefore shows the double-blend overlap exactly as the game
does.

## 5. Editor (crates/gui/ui/kit/)

A new front-end replaces designer.js; designer.rs commands remain
until the switch is complete, then the v1 window is removed.

5.1 Architecture

- One store holding the Design plus editor state (selection, mode,
  view). Every mutation goes through `commit(label)` which snapshots
  the Design JSON for undo (sliding coalesce window as in v2), then a
  single `render()` pass rewrites the panels from state. A small
  hand-written render layer (about 150 lines, no framework) keeps
  panels and state from ever disagreeing.
- IPC: `kit_evaluate(design)` returns, for drawing and hit testing, the
  resolved contours per part, the solid and glass silhouettes per
  level, validator report and triangle count;
  `kit_mesh(design)` returns positions, indices and material ranges for
  the turntable; `kit_preview(design, time, still)` renders through the
  real renderer with the cached skin as today; `kit_export(design,
  path)` writes the .rhs after validation; document save, load,
  autosave and recent list as in v2 (format version 2).
- Calls are coalesced: at most one evaluate in flight, the latest
  design wins, and the canvas keeps the last good result on error so
  handles never vanish (a v2 defect).

5.2 Panels

- Parts list: ordered, drag to reorder, visibility and lock toggles,
  Add or Cut badge, Solid or Glass badge, group rows that fold (Phase
  3: a group's header row sits above its members, which stay contiguous
  in the parts array; the header selects the group, its triangle folds
  the members as editor state, a part row dropped on it joins the
  group). Cut parts, and the members of a Cut group, draw hatched in the
  front view.
- Inspector for the selected part: shape type and its parameters,
  numeric transform fields, depth band (two fields plus a small depth
  bar), a Group field (None, an existing group, New group), effect
  chips (Waves with count, depth and style; Round; Rim: one On/Off
  button each, the fields follow while on, defaults 6/0.15/sine, 0.1,
  0.15), material switch, symmetry controls (Mirror none/X/Y/both,
  Repeat 2 to 12 with 0 for off). A selected group shows its own
  inspector: name, mode, its parts with Leave group, Ungroup.
- Front view (orthographic, Canvas 2D): the only place geometry is
  edited. Click selects, drag moves, corner handles scale (Shift keeps
  aspect), a rotation handle rotates, arrows nudge, snapping as in v2,
  zoom about cursor and pan as in v2. The resolved silhouette draws in
  the first colorset colour band by band from the back forward, the
  highest band in the colour and lower bands shaded toward dark (toward
  light for a dark colour) by how far they lie below the top, so a raised
  or recessed part reads head-on (the game shows it only through
  parallax, which the hint says); holes cut by orientation; glass draws
  at the game's alpha; every part's own outline draws faintly so cut
  parts stay findable. The parts list shows each part's Front depth. The
  list order has no geometric meaning (adds and cuts combine per
  interval regardless of order); it sets which part a click picks first.
  Point editing (landed in the Phase 2 test rounds): double-click a part
  or press To points in the inspector; a Points part opens for editing,
  any other shape is converted first (`kit_shape_to_points`: the kernel
  thins the outline with Ramer-Douglas-Peucker to at most 64 points per
  loop, a turn of 40 degrees or more makes a corner, every loop of the
  source becomes a loop of points, the effects and transform stay). In
  editing, the points draw as handles in the part's own frame (round for
  smooth, square for a corner) with the control polygon dashed; drag
  moves a point (Shift snaps to a 0.05 grid of that frame), a click
  within 12 px of the polygon inserts a point on the nearest segment
  (smooth unless both neighbours are corners) and picks it up, double-
  click switches smooth and corner, right-click or Delete removes (a loop
  keeps three points; a three-point loop goes as a whole unless it is the
  last), arrows nudge the selected point, Esc or a click away from the
  polygon finishes. Undo, removal or a shape change of the part ends the
  editing.
- Turntable (WebGL2, inspection only; landed in the Phase 2 test
  rounds as "3D", key 3): the kit mesh (`kit_mesh`, the export's own
  triangles, expanded to flat faces) from an orbit camera limited to
  plus or minus 60 degrees in yaw and pitch, wheel zoom, two snap
  buttons (Front, 3/4), the game's tint and glass alpha, a Shaded toggle
  (a headlight on the face normals so relief reads; off is the game's
  flat look) and a Fade slider (the whole note at one alpha without a
  depth test, so overlapping parts brighten as in the approach fade).
  It lives in the band below the front view and shares it with the
  previews; its state is remembered. A refused design keeps the last
  note and says why in the caption. The note box at the hit plane draws
  as lines for orientation.
- Help (button, F1 or ?): a modal with the table of every control
  (toolbar, parts list, canvas, point editing, 3D, inspector, keyboard);
  `help.js` holds the rows, the guide carries the same table, and keys
  stay with the dialog while it is open.
- Previews: still and gameplay loop through the real renderer, as in
  v2, with the splitter.
- Presets: the v2 gallery becomes multi-part presets (a gear with a
  hub, a shield with an emblem, a ring with a recessed plate).
- Import: SVG and glyph become new parts (landed in the Phase 2 test
  rounds: the SVG button and the Letter field add `Contours` parts named
  after their source). An SVG (`kit_import_svg`) contributes every
  visible path's fill and stroke, the stroke outlined at its width with
  caps, joins and dashes; paths group into one layer per paint colour in
  order of first appearance, all layers share one unit frame, and each
  layer is a part (`<name> 1`, `<name> 2`, the inspector naming the
  colour). `kit::import::layer_modes` sets the mode: the first layer
  adds; a later layer whose area lies more than half on material already
  there is a Cut and is taken out of that material, otherwise it adds
  (the game shows one tint, so a figure on a badge can only be told
  apart as a hole; stripes side by side stay a full silhouette). Same-
  colour shapes merge. Text is not imported. The old designer keeps its
  single merged outline (`designer_import_svg`, strokes included). The
  reference underlay stays (Phase 4).

5.3 Vocabulary

Add, Cut, Combine, Depth, Glass, Mirror, Repeat, Rim, Waves, Round.
Never boolean, union, mesh, vertex, modifier, manifold.

## 6. Export

`kit_export` writes `config`, `noteSkin/<name>.obj` (one object, two
`usemtl` sections at most, solid first), `noteSkin/note.mtl`
(Material.001 and Transparent, always both, so every kit skin bundles an
identical mtl and importing several skins cannot leave a material
undefined), and `colorSet/<name>.txt`, exactly the layout the game
accepts today. The skin name is validated first: letters, digits,
spaces, `_` and `-`, 1 to 64 characters, no leading or trailing space,
because it reaches the config JSON, two archive paths and the obj's `o`
line. The design itself is written into the archive as its last entry,
`rhythr/design.json` (the game ignores entries it does not know, verified
2026-10-24 with a probe skin), so the one .rhs is both the game's skin
and the editable document: Save writes it, Open reads it back, and a
skin saved by the old designer carries its own document under the same
entry and converts on open. A plain .rhsd (version 2 text, or the old
designer's) still opens; a skin without the entry is refused with a
sentence that says so. A design the kernel refuses is not written,
because the game could not load it.

## 7. Testing

- Kernel unit tests for every shape source, effect, symmetry, depth
  evaluation and the validator itself (a hand-broken mesh must fail).
- Property tests (proptest): random designs of 1 to 4 parts with random
  modes, bands, effects (repeat up to 4, waves up to 8, the cost ruling
  of Phase 1), materials and groups, half of the transforms on a 0.1
  grid so seams run flush; every design evaluates to a clean Empty or
  Budget, or to a mesh that passes the validator, has no T-junction
  within a material (an exact oracle on the lattice), stays in the box,
  is deterministic (the archive's obj equals a second build) and loads
  back as a skin. 10 cases in a debug run, 200 in release.
- Golden images: ten reference designs rendered through the real
  renderer, compared in CI with a per-platform reference or a tolerance,
  because f32 trigonometry differs by an ulp between the MSVC and glibc
  runtimes and about one vertex in sixty snaps to a neighbouring lattice
  cell on the other platform (invisible, but not pixel-exact).
- Headless UI checks (the v2 scenario harness): undo across parts,
  reorder, group fold, cut badge, point edit, document roundtrip,
  v1 document migration.
- Acceptance: a final probe round in the game with three kit-built
  skins (stacked, glass, cut-heavy) and the glass alpha re-measured.

## 8. Phases

1. Kernel: document model, part resolution, level evaluation, mesh,
   validator, exporter, renderer material ranges, tests. Rust only; the
   v1 window keeps running.
2. Editor core: store and render layer, parts list, inspector, front
   view with transforms and selection, previews, undo, autosave, files,
   v1 migration. Testbuild.
3. Composition: groups, effects chips, symmetry, snapping, glass (point
   edit, the depth shading of the front view, the turntable and the help
   table landed early, in the Phase 2 test rounds). Built 25.10. with the
   owner's rulings: waves modulate inward (crest on the outline, trough
   at 1 minus the depth; a v1 depth d migrates as 2d / (1 + d)); groups
   form without multi-selection (the part's Group field, or a drop on a
   group's header row), members stay contiguous in the parts array, a
   group's header folds its members as editor state; Snap is on by
   default (0.05 grid, the box lines, 15 degree steps within 3 degrees,
   a 6 px radius; Shift keeps its meanings). Testbuild kit7.
4. Polish and acceptance (built 01.11., design document
   `2026-11-01-note-kit-phase4-polish-design.md`): Send to game (the
   note written into the game's data folder as the profile rhythr-kit),
   five presets in the rim style, the removal of the v1 designer (the
   kit is the Note Designer), the guide and changelog, the review polish
   (imports as parts and the one-file save landed early, in the Phase 2
   test rounds). The reference underlay was dropped by the owner. The
   owner's probe round in the game and the release candidate 0.7.0
   follow.

## 9. Decisions (closed)

- Depth range: [-2.0, 0.5] (the built-in cube's reach), decided 24.10.
- The v1 slider mode survives only as the quick-shape buttons; the
  migration turns an old design into one part with the same shape.
- A new part spans [-0.1, 0.1].
- 01.11.: no reference underlay, no Twist, no wedge editing with radial
  symmetry; the v1 designer is removed entirely and the kit carries the
  name Note Designer.
