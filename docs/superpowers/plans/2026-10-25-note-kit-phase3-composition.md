# Note Kit Phase 3: Composition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Note Kit window gains everything spec section 8 lists for Phase 3 that has not landed yet: groups (a cut inside a group bites only that group's adds), effect chips (Waves, Round, Rim), symmetry controls (Mirror, Repeat), snapping in the front view and in point editing, plus a committed headless scenario runner; shipped as testbuild kit7.

**Architecture:** The kernel already folds groups, applies waves, rounding, rims, mirror and repeat (Phase 1); this phase is almost entirely editor work in the existing modules: new store operations with the invariant that a group's members stay contiguous in the parts array, a pure `listRows` model that the parts list renders (group headers above their members, folding as UI state), inspector sections built with the existing `slider`, `number` and `segmented` helpers, snapping maths in `view-math.js` applied by the view's gestures, and the help table extended. Two small kernel changes: waves modulate inward so a wavy part stays inside its own extent (with a v1 depth conversion in the migration), and the SVG and glyph import flattens finer. The headless scenario scripts of the test rounds move into the repo as one runner.

**Tech Stack:** Rust (the Phase 1 kit kernel, serde), plain JavaScript ES modules (no framework, no bundler), Canvas 2D, Node 22 `node --test`, playwright-core from the npx cache for the scenario runner (optional dev tool, never a dependency).

**Spec:** `docs/superpowers/specs/2026-10-24-note-kit-2-5d-design.md` (sections 3, 4.1, 4.2, 5.2, 7, 8 phase 3) and the owner's decisions recorded in the Phase 2 plan header and ledger (depth range stays [-2.0, 0.5]; the old slider mode survives only as presets; a new part spans [-0.1, 0.1]; the note box is fixed and nothing is rescaled). Rulings proposed by this plan, to be confirmed by the owner before execution: waves modulate inward; groups are formed without multi-selection (the part inspector's Group field and a drop on a group header); snapping is on by default with a 0.05 grid and the box lines, Shift keeps its present meanings.

## Global Constraints

- Pure Rust and plain JavaScript only: no new crates, no npm packages, no bundler; the pure front-end modules run under `node --test crates/gui/ui/kit/tests/*.test.mjs` (Node 22, no package.json).
- Nothing platform-specific in browser modules; Rust uses no `std::os::unix`.
- House style: no em or en dashes anywhere (`scripts/preflight.sh` fails on them, also in JavaScript, HTML, CSS and Markdown); comments state constraints, not narration; UI text is English; commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Vocabulary (spec 5.3): Add, Cut, Combine, Depth, Glass, Mirror, Repeat, Rim, Waves, Round; never boolean, union, mesh, vertex, modifier, manifold, lattice. Kernel errors reach the user only through `kit_commands::user_message`.
- Document format: the JSON of `rhythia_render::kit::doc::Design` exactly as serde writes it. Groups are `{"id":"g1","name":"Group 1","parts":["p2","p3"],"mode":"add"}`; a part belongs to at most one group (the kernel refuses otherwise). Effects are `{"waves":{"count":6,"depth":0.15,"style":"sine"}|null,"round":0.1|null,"rim":0.15|null,"mirror":"none"|"x"|"y"|"both","repeat":4|null}`. Folding is editor state, never written to the document.
- Ranges the editor offers: waves count 2 to 16 (step 1), waves depth 0.02 to 0.6 (step 0.01), style sine, spike or pulse; round 0.01 to 0.5; rim 0.02 to 0.5; repeat 2 to 12 (0 or 1 in the field means off, stored as null); x and y in [-1.5, 1.5], w and h in [0.02, 1.5], rotation in [-180, 180]. Snap grid 0.05, snap radius 6 px, angle snap 15 degrees within 3 degrees.
- Default values when a chip is switched on: waves `{count: 6, depth: 0.15, style: "sine"}`, round 0.1, rim 0.15.
- The old designer (`designer.html`, `designer.js`, the `designer_*` commands) keeps working untouched.
- Run `cargo fmt --all`, `cargo test -p rhythia-render --lib kit::`, `cargo test -p rhythia-gui`, `node --test crates/gui/ui/kit/tests/*.test.mjs`, `cargo build -p rhythia-render --examples` and `bash scripts/preflight.sh` before every commit named below; `cargo test -p rhythia-render --test kit_properties` once before the testbuild. One cargo process at a time on this machine.

## Review Focus

Inputs the spec implies but which the happy path would not exercise. Each line names the task whose tests pin it.

1. A part dragged into a group that sits far away in the list must end up next to the group's members, and a part removed from the design must leave its group, an emptied group disappearing with it; undo restores both. Pinned in Task 2 (`a_joining_part_moves_next_to_its_group`, `removing_the_last_member_removes_the_group`).
2. A wavy part at full size must stay inside its own handles and the box (the owner's round 2 note: outward waves were cut at the box), and a migrated v1 wave must keep its valley depth. Pinned in Task 1 (`waves_stay_inside_the_part`, `a_v1_wave_depth_converts_to_the_inward_formula`).
3. Snapping must never fight a deliberate placement: a value farther than the radius from any grid line or box line passes through unchanged, and Shift's meanings (keep proportions, 15 degree steps, grid in point editing) stay as they are. Pinned in Task 3 (`snapping_only_pulls_within_the_radius`).
4. Switching an effect chip on and off must be one undo step each and leave the other effects alone; a repeat of 1 or 0 typed in the field stores null. Pinned in Task 4 (`chips_toggle_discretely_and_repeat_below_two_is_off`).
5. Folding a group must hide only its members and never change the document (the autosave and the evaluate must not fire on a fold). Pinned in Task 2 (`folding_is_editor_state_only`) and the scenario runner in Task 5.

---

## File structure

- Modify `crates/render/src/kit/resolve.rs`: `apply_waves` modulates inward; tests.
- Modify `crates/render/src/kit/migrate.rs`: v1 wave depth converted; the fit test adjusted.
- Modify `crates/gui/src/imports.rs`: finer flattening tolerance in `contours_from_bezpath`; a test on a large circle's point count.
- Modify `crates/gui/ui/kit/store.js`: groups (`createGroup`, `addToGroup`, `removeFromGroup`, `ungroup`, `updateGroup`, `selectGroup`, `groupOf`, `toggleFold`), `listRows`, `ui.snap`, selection rules, new discrete labels.
- Modify `crates/gui/ui/kit/view-math.js`: `snapValue`, `snapPosition`, `snapSize`, `snapAngle`.
- Modify `crates/gui/ui/kit/view.js`: snapping in move, scale, rotate and point drags; group highlight; Cut-group hatching; the Snap button.
- Modify `crates/gui/ui/kit/render.js`: parts list from `listRows` (group headers, fold, drop on a header), group inspector, part inspector sections Group, Effects, Symmetry.
- Modify `crates/gui/ui/kit/main.js`, `kit.html`, `kit.css`: the Snap button, the snap state, styles for headers and chips.
- Modify `crates/gui/ui/kit/help.js`: rows for groups, effects, symmetry, snapping.
- Create `crates/gui/ui/kit/tests/scenarios.mjs`: the headless scenario runner (starts the harness, drives kit.html, prints PASS/FAIL), consolidating the round 3 to 5 scripts kept under `crates/gui/ui/kit/tests/scenarios/` as raw material (deleted at the end of Task 5).
- Modify tests: `store.test.mjs`, `view-math.test.mjs`, `render.test.mjs`, `help.test.mjs`.
- Modify `docs/USER-GUIDE.md` (the Note Kit paragraph and the controls table, regenerated from `helpMarkdown`), the spec (5.2 and 8), and append to the Phase 2 ledger's successor `docs/superpowers/plans/2026-10-25-note-kit-phase3-ledger.md`.

---

### Task 1: Inward waves and finer import flattening (kernel)

**Files:**
- Modify: `crates/render/src/kit/resolve.rs` (`apply_waves`, test `waves_change_the_radius`)
- Modify: `crates/render/src/kit/migrate.rs` (the `waves` branch near line 124; test `a_v1_design_is_fitted_to_the_box_once` near line 333)
- Modify: `crates/gui/src/imports.rs` (`contours_from_bezpath` tolerance; a new test)

**Interfaces:**
- Consumes: `Waves { count, depth, style }`, `wave_fn`.
- Produces: unchanged signatures; the radius factor becomes `1 + depth * (wave - 1) / 2`, so the tips sit at the part's own extent and the valleys at `1 - depth`.

- [ ] **Step 1: Write the failing kernel tests**

In `crates/render/src/kit/resolve.rs` replace the test `waves_change_the_radius` with:

```rust
    #[test]
    fn waves_stay_inside_the_part() {
        // Tips at the part's own extent, valleys at 1 minus the depth: a
        // wavy part at full size never reaches past its handles or the box.
        let mut p = part(Shape::Circle);
        p.effects.waves = Some(Waves {
            count: 6,
            depth: 0.2,
            style: WaveStyle::Sine,
        });
        let r = resolve_part(&p).unwrap();
        let radii: Vec<f32> = r.shapes[0][0].iter().map(|v| v[0].hypot(v[1])).collect();
        let (min, max) = (
            radii.iter().cloned().fold(f32::MAX, f32::min),
            radii.iter().cloned().fold(f32::MIN, f32::max),
        );
        assert!(max <= 1.0 + 1e-3, "tips past the extent: {max}");
        assert!((min - 0.8).abs() < 0.02, "valleys at {min}, expected 0.8");
    }
```

In `crates/render/src/kit/migrate.rs` add next to the other migration tests:

```rust
    #[test]
    fn a_v1_wave_depth_converts_to_the_inward_formula() {
        // v1 modulated outward and normalised afterwards, so its valleys sat
        // at (1 - d) / (1 + d) of the tips; the inward formula puts them at
        // 1 - d', so d' = 2d / (1 + d) keeps the look.
        let json = r##"{"P":{"shape":"circle","n":6,"round":0.0,"hollow":false,"thick":0.22,
          "depth":0.05,"waves":6,"waveDepth":0.25,"waveform":"sine","twist":0},
          "free":null,"symmetry":1,"mirror":false,"colors":["#ffffff"],"imported":null}"##;
        let d = from_v1(json).unwrap();
        let w = d.parts[0].effects.waves.as_ref().expect("waves kept");
        assert_eq!(w.count, 6);
        assert!((w.depth - 0.4).abs() < 1e-5, "depth {}", w.depth);
    }
```

Read the existing test `a_v1_design_is_fitted_to_the_box_once` and change its premise: a v1 shape "with waves reached past the box" no longer happens with inward waves, so the test asserts that the migrated part's resolved extent is at most 1 (plus 1e-3) and that its wave count survives, instead of asserting a shrink factor below 1. Keep any assertion about the fit of shapes without waves as it is.

In `crates/gui/src/imports.rs` add to the tests:

```rust
    #[test]
    fn a_large_disc_imports_with_a_smooth_outline() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
          <circle fill="#ff4500" cx="128" cy="128" r="128"/>
        </svg>"##;
        let layers = svg_layers(svg.as_bytes()).unwrap();
        let n = layers[0].contours[0].len();
        assert!(n >= 64, "a disc with {n} points shows its facets");
        assert!(n <= 400, "{n} points is more than the kit needs");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rhythia-render --lib kit::resolve::tests::waves_stay_inside_the_part kit::migrate::tests::a_v1_wave_depth` and `cargo test -p rhythia-gui imports::tests::a_large_disc`
Expected: FAIL (tips at 1.2; depth 0.25 kept; 44 points).

- [ ] **Step 3: Implement**

In `resolve.rs` change the factor line of `apply_waves` to:

```rust
            // Inward only: the wave's crest sits on the source outline and
            // the trough at 1 minus the depth, so the part stays within its
            // own extent (the handles and the box hold).
            let f = 1.0 + depth * (wave_fn(w.style, count as f32 * th) - 1.0) * 0.5;
```

and update the module doc's waves sentence to say "inward". In `migrate.rs`, where `effects.waves = Some(Waves { count: waves, depth: ..., style })` is built from `waveDepth` (`d`), store `depth: 2.0 * d / (1.0 + d)` with a comment giving the derivation above. In `imports.rs` change `let tol = extent.max(1.0) * 0.0015;` to `let tol = extent.max(1.0) * 0.0005;` with the comment "a 256 unit disc flattens to about 80 segments: no visible facets on a note, far below the part budget".

- [ ] **Step 4: Run the kernel and gui tests**

Run: `cargo test -p rhythia-render --lib kit::` then `cargo test -p rhythia-gui`
Expected: all pass (adjust `a_v1_design_is_fitted_to_the_box_once` as described if it still asserts a shrink).

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/kit/resolve.rs crates/render/src/kit/migrate.rs crates/gui/src/imports.rs
git commit -m "kit: waves modulate inward, imports flatten finer"
```

---

### Task 2: Groups and snap state in the store

**Files:**
- Modify: `crates/gui/ui/kit/store.js`
- Test: `crates/gui/ui/kit/tests/store.test.mjs`

**Interfaces:**
- Produces: `store.groupOf(partId) -> group | null`, `store.createGroup(partId, name?) -> groupId`, `store.addToGroup(partId, groupId)`, `store.removeFromGroup(partId)`, `store.ungroup(groupId)`, `store.updateGroup(groupId, label, patch)`, `store.selectGroup(groupId | null)`, `store.toggleFold(groupId)`, `store.nextGroupId()`, state `s.selectedGroup` (null or id), `s.ui.folded` (array of group ids), `s.ui.snap` (boolean, default true), and the pure export `listRows(design, folded) -> [{kind: "part", id, group} | {kind: "group", id, name, mode, folded, count}]` in DOM order (bottom to top; a group header follows its topmost member so the column-reverse list shows it above them).
- Invariant: a group's members are contiguous in `design.parts`; a part is in at most one group; an emptied group is deleted; `select(id)` clears `selectedGroup`, `selectGroup(id)` clears `selected`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/gui/ui/kit/tests/store.test.mjs` (import `listRows` from `../store.js` in the header import):

```js
function fourParts() {
  const d = design();
  for (const id of ["p2", "p3", "p4"]) d.parts.push({ ...newPart(id, "square"), id });
  return d;
}

test("a_group_forms_from_a_part_and_members_stay_contiguous", () => {
  const s = createStore(fourParts());
  const gid = s.createGroup("p1", "Badge");
  assert.equal(gid, "g1");
  assert.deepEqual(s.state.design.groups, [{ id: "g1", name: "Badge", parts: ["p1"], mode: "add" }]);
  assert.equal(s.state.selectedGroup, "g1");
  assert.equal(s.state.selected, null, "a selected group means no selected part");
  // p4 sits at the top; joining moves it next to p1 at the bottom.
  s.addToGroup("p4", "g1");
  assert.deepEqual(s.state.design.parts.map((p) => p.id), ["p1", "p4", "p2", "p3"]);
  assert.deepEqual(s.groupOf("p4").parts, ["p1", "p4"]);
  // A member joining another group leaves the first.
  const g2 = s.createGroup("p3");
  s.addToGroup("p4", g2);
  assert.deepEqual(s.groupOf("p4").id, g2);
  assert.deepEqual(s.state.design.groups[0].parts, ["p1"]);
  assert.deepEqual(s.state.design.parts.map((p) => p.id), ["p1", "p2", "p3", "p4"]);
  assert.equal(s.state.undo.length, 4, "every group edit is one undo step");
  s.undo();
  assert.deepEqual(s.state.design.groups[0].parts, ["p1", "p4"]);
});

test("removing_the_last_member_removes_the_group", () => {
  const s = createStore(fourParts());
  s.createGroup("p2");
  s.addToGroup("p3", "g1");
  s.removeFromGroup("p2");
  assert.deepEqual(s.state.design.groups[0].parts, ["p3"]);
  s.removePart("p3");
  assert.deepEqual(s.state.design.groups, [], "an emptied group goes");
  assert.equal(s.state.selectedGroup, null);
  s.undo();
  assert.equal(s.state.design.groups.length, 1);
  s.ungroup("g1");
  assert.deepEqual(s.state.design.groups, []);
  assert.equal(s.state.design.parts.length, 4, "ungroup keeps the parts");
});

test("group_edits_and_selection_rules", () => {
  const s = createStore(fourParts());
  s.createGroup("p1");
  s.updateGroup("g1", "group-mode", (g) => (g.mode = "cut"));
  s.updateGroup("g1", "group-name", (g) => (g.name = "Holes"));
  assert.equal(s.state.design.groups[0].mode, "cut");
  assert.equal(s.state.design.groups[0].name, "Holes");
  s.select("p2");
  assert.equal(s.state.selectedGroup, null);
  assert.equal(s.state.selected, "p2");
  s.selectGroup("g1");
  assert.equal(s.state.selected, null);
  s.ungroup("g1");
  assert.equal(s.state.selectedGroup, null, "a gone group is not selected");
  assert.equal(s.nextGroupId(), "g1", "ids are reused when free");
});

test("folding_is_editor_state_only", () => {
  const s = createStore(fourParts());
  s.createGroup("p1");
  const before = s.serialize();
  const steps = s.state.undo.length;
  s.toggleFold("g1");
  assert.deepEqual(s.state.ui.folded, ["g1"]);
  assert.equal(s.serialize(), before, "the document is untouched");
  assert.equal(s.state.undo.length, steps, "no undo step");
  s.toggleFold("g1");
  assert.deepEqual(s.state.ui.folded, []);
  assert.equal(s.state.ui.snap, true, "snapping starts on");
});

test("list_rows_put_a_group_header_after_its_top_member_and_fold_hides_members", () => {
  const d = fourParts();
  d.groups.push({ id: "g1", name: "G", parts: ["p2", "p3"], mode: "cut" });
  assert.deepEqual(listRows(d, []), [
    { kind: "part", id: "p1", group: null },
    { kind: "part", id: "p2", group: "g1" },
    { kind: "part", id: "p3", group: "g1" },
    { kind: "group", id: "g1", name: "G", mode: "cut", folded: false, count: 2 },
    { kind: "part", id: "p4", group: null },
  ]);
  assert.deepEqual(listRows(d, ["g1"]).map((r) => `${r.kind}:${r.id}`), ["part:p1", "group:g1", "part:p4"]);
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `node --test crates/gui/ui/kit/tests/store.test.mjs`
Expected: FAIL (`s.createGroup is not a function`, `listRows` undefined).

- [ ] **Step 3: Implement in `store.js`**

Add the labels `"group", "group-add", "group-remove", "ungroup", "group-mode", "group-name", "effect-waves", "effect-round", "effect-rim", "mirror", "repeat"` to `DISCRETE`. Add `selectedGroup: null` to the state and `folded: [], snap: true` to `ui`. Add the module-level helpers and the export:

```js
// A part leaves whatever group holds it; a group left empty goes with it.
function leaveGroup(d, id) {
  for (const g of d.groups) g.parts = g.parts.filter((x) => x !== id);
  d.groups = d.groups.filter((g) => g.parts.length > 0);
}

// Members stay contiguous in the parts array: a joining part moves to sit
// directly above the group's topmost member, so the list shows the group
// in one block whatever order the parts had.
function placeNextToGroup(d, id, g) {
  const idx = d.parts.findIndex((p) => p.id === id);
  const others = g.parts.filter((x) => x !== id).map((x) => d.parts.findIndex((p) => p.id === x)).filter((i) => i >= 0);
  if (idx < 0 || !others.length) return;
  const top = Math.max(...others);
  const [p] = d.parts.splice(idx, 1);
  // After the removal the old top member sits one lower when it was above
  // the moved part; either way the part lands directly above it.
  d.parts.splice(idx < top ? top : top + 1, 0, p);
}

// The parts list in DOM order (the list is column-reverse, so the last row
// draws at the top): parts bottom to top, a group's header after its
// topmost member, members hidden while the group is folded.
export function listRows(design, folded = []) {
  const groupOf = (id) => design.groups.find((g) => g.parts.includes(id)) || null;
  const top = new Map();
  design.parts.forEach((p, i) => {
    const g = groupOf(p.id);
    if (g) top.set(g.id, i);
  });
  const rows = [];
  design.parts.forEach((p, i) => {
    const g = groupOf(p.id);
    const isFolded = Boolean(g) && folded.includes(g.id);
    if (!isFolded) rows.push({ kind: "part", id: p.id, group: g ? g.id : null });
    if (g && top.get(g.id) === i) rows.push({ kind: "group", id: g.id, name: g.name, mode: g.mode, folded: isFolded, count: g.parts.length });
  });
  return rows;
}
```

In `fixSelection` add, before the point-edit block:

```js
    if (s.selectedGroup && !s.design.groups.some((g) => g.id === s.selectedGroup)) s.selectedGroup = null;
    s.ui.folded = s.ui.folded.filter((id) => s.design.groups.some((g) => g.id === id));
```

Change `select(id)` to also set `s.selectedGroup = null`. In `removePart`, replace the group pruning line with `leaveGroup(d, id);`. Add to the api:

```js
    nextGroupId() {
      const ids = new Set(s.design.groups.map((g) => g.id));
      let n = 1;
      while (ids.has(`g${n}`)) n++;
      return `g${n}`;
    },
    groupOf(id) {
      return s.design.groups.find((g) => g.parts.includes(id)) || null;
    },
    selectGroup(id) {
      s.selectedGroup = id;
      if (id) s.selected = null;
      emit();
    },
    // A new group holds one part and is selected, so it can be named and
    // given its mode at once; more parts join from their inspector or by
    // a drop on the group's row.
    createGroup(partId, name) {
      const id = api.nextGroupId();
      const n = Number(id.slice(1));
      api.commit("group", (d) => {
        if (!d.parts.some((p) => p.id === partId)) return;
        leaveGroup(d, partId);
        d.groups.push({ id, name: String(name || "").trim() || `Group ${n}`, parts: [partId], mode: "add" });
        s.selectedGroup = id;
        s.selected = null;
      });
      return id;
    },
    addToGroup(partId, groupId) {
      api.commit("group-add", (d) => {
        const g = d.groups.find((x) => x.id === groupId);
        if (!g || !d.parts.some((p) => p.id === partId) || g.parts.includes(partId)) return;
        leaveGroup(d, partId);
        const again = d.groups.find((x) => x.id === groupId);
        if (!again) return;
        again.parts.push(partId);
        placeNextToGroup(d, partId, again);
      });
    },
    removeFromGroup(partId) {
      api.commit("group-remove", (d) => leaveGroup(d, partId));
    },
    ungroup(groupId) {
      api.commit("ungroup", (d) => {
        d.groups = d.groups.filter((g) => g.id !== groupId);
      });
    },
    updateGroup(groupId, label, patch) {
      api.commit(label, (d) => {
        const g = d.groups.find((x) => x.id === groupId);
        if (g) patch(g);
      });
    },
    // Folding is editor state: no commit, no undo step, no evaluate.
    toggleFold(groupId) {
      s.ui.folded = s.ui.folded.includes(groupId) ? s.ui.folded.filter((x) => x !== groupId) : [...s.ui.folded, groupId];
      emit();
    },
```

Note on `addToGroup`: `leaveGroup` may delete the target group when the part was its only member; the second lookup (`again`) guards that.

- [ ] **Step 4: Run the store tests**

Run: `node --test crates/gui/ui/kit/tests/store.test.mjs`
Expected: PASS (all, including the earlier ones).

- [ ] **Step 5: Commit**

```bash
git add crates/gui/ui/kit/store.js crates/gui/ui/kit/tests/store.test.mjs
git commit -m "kit: groups and folding in the store, list rows model"
```

---

### Task 3: Snapping and group feedback in the front view

**Files:**
- Modify: `crates/gui/ui/kit/view-math.js`, `crates/gui/ui/kit/view.js`, `crates/gui/ui/kit/main.js`, `crates/gui/ui/kit.html`, `crates/gui/ui/kit.css`
- Test: `crates/gui/ui/kit/tests/view-math.test.mjs`

**Interfaces:**
- Produces in `view-math.js`: `SNAP_GRID = 0.05`, `SNAP_PX = 6`, `snapValue(v, tol, anchors = [], grid = SNAP_GRID) -> number`, `snapPosition(x, y, tol) -> [x, y]` (grid plus the box centre and edges, anchors -1, 0, 1), `snapSize(hw, hh, tol) -> [hw, hh]` (grid plus 0.5 and 1), `snapAngle(deg, tolDeg = 3) -> deg` (15 degree steps).
- Consumes: `store.state.ui.snap`, `store.state.selectedGroup`, `design.groups`.
- The Snap button: `#kt-snap` in the canvas box next to `#kt-view-reset`, class `on` while snapping, remembered under `localStorage` key `rhythr.kit.snap`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/gui/ui/kit/tests/view-math.test.mjs` (extend the import with `snapValue, snapPosition, snapSize, snapAngle`):

```js
test("snapping_only_pulls_within_the_radius", () => {
  // Within 0.02 of a grid line: pulled; farther: left alone.
  assert.equal(snapValue(0.512, 0.02), 0.5);
  assert.equal(snapValue(0.537, 0.02), 0.537);
  // The box edge wins over the grid when it is nearer.
  assert.deepEqual(snapPosition(0.99, -0.012, 0.02), [1, 0]);
  assert.deepEqual(snapPosition(0.74, 0.3, 0.005), [0.74, 0.3]);
  assert.deepEqual(snapSize(0.49, 1.004, 0.02), [0.5, 1]);
  assert.equal(snapAngle(44, 3), 45);
  assert.equal(snapAngle(40, 3), 40);
  assert.equal(snapAngle(-178.5, 3), -180);
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test crates/gui/ui/kit/tests/view-math.test.mjs`
Expected: FAIL (`snapValue` is not exported).

- [ ] **Step 3: Implement the maths**

Append to `view-math.js`:

```js
// Snapping: a value is pulled to the nearest grid line or anchor when that
// lies within `tol` (world units, the pixel radius over the view's unit);
// anything farther passes through, so a deliberate placement is never
// fought. Results carry three decimals like every other edit.
export const SNAP_GRID = 0.05;
export const SNAP_PX = 6;
export function snapValue(v, tol, anchors = [], grid = SNAP_GRID) {
  let best = v;
  let bd = Infinity;
  for (const c of [Math.round(v / grid) * grid, ...anchors]) {
    const d = Math.abs(v - c);
    if (d <= tol && d < bd) {
      bd = d;
      best = c;
    }
  }
  return Math.round(best * 1000) / 1000;
}
export function snapPosition(x, y, tol) {
  return [snapValue(x, tol, [-1, 0, 1]), snapValue(y, tol, [-1, 0, 1])];
}
export function snapSize(hw, hh, tol) {
  return [snapValue(hw, tol, [0.5, 1]), snapValue(hh, tol, [0.5, 1])];
}
export function snapAngle(deg, tolDeg = 3) {
  const s = Math.round(deg / 15) * 15;
  return Math.abs(deg - s) <= tolDeg ? s : deg;
}
```

- [ ] **Step 4: Apply it in `view.js`**

Import the four functions and `SNAP_PX`. In the pointermove handler compute `const tol = SNAP_PX / unit(view, size);` and `const snapping = store.state.ui.snap;` once, then:

- move: after `dx, dy`, `let nx = gesture.x + dx, ny = gesture.y + dy; if (snapping) [nx, ny] = snapPosition(nx, ny, tol);` and clamp as before.
- scale: after `scaleFrom`, `let { hw, hh } = ...; if (snapping && !keep) [hw, hh] = snapSize(hw, hh, tol);`.
- rotate: replace the Shift line with `if (e.shiftKey) a = Math.round(a / 15) * 15; else if (snapping) a = snapAngle(a, 3);`.
- point drag: `if (e.shiftKey) q = snapUnit(q, POINT_SNAP); else if (snapping) q = [snapValue(q[0], tol / (r.hw || 1), [], POINT_SNAP), snapValue(q[1], tol / (r.hh || 1), [], POINT_SNAP)];` (the tolerance moves into the part's unit frame).

In `draw()`, before the per-part outline loop, compute `const cutGroups = new Set(s.design.groups.filter((g) => g.mode === "cut").flatMap((g) => g.parts));` and `const groupSel = s.selectedGroup ? new Set((s.design.groups.find((g) => g.id === s.selectedGroup) || { parts: [] }).parts) : null;`. Treat a part as a remover when `part.mode === "cut" || cutGroups.has(part.id)` (hatch and dashed stroke), and treat it as selected when `part.id === s.selected || (groupSel && groupSel.has(part.id))`. The handles stay bound to `store.part()` (a selected group has none).

- [ ] **Step 5: The Snap button**

In `kit.html` add inside `#kt-canvas-box`, after the reset button: `<button id="kt-snap" class="btn small ghost on" title="Snap moves, sizes, turns and points to a 0.05 grid and the box lines">Snap</button>`. In `kit.css` add `#kt-snap { position: absolute; top: 8px; right: 72px; z-index: 2; opacity: 0.75; } #kt-snap.on { outline: 2px solid var(--accent); }`. In `main.js` after the view is created:

```js
  // Snapping is remembered; the button mirrors the state.
  const SNAP_KEY = "rhythr.kit.snap";
  const setSnap = (on) => {
    store.setUi({ snap: on });
    $("kt-snap").classList.toggle("on", on);
    try { storage.setItem(SNAP_KEY, on ? "1" : "0"); } catch { /* best effort */ }
  };
  setSnap(storage.getItem(SNAP_KEY) !== "0");
  $("kt-snap").addEventListener("click", () => setSnap(!store.state.ui.snap));
```

- [ ] **Step 6: Run the maths tests and the whole Node suite**

Run: `node --test crates/gui/ui/kit/tests/*.test.mjs`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/gui/ui/kit/view-math.js crates/gui/ui/kit/view.js crates/gui/ui/kit/main.js crates/gui/ui/kit.html crates/gui/ui/kit.css crates/gui/ui/kit/tests/view-math.test.mjs
git commit -m "kit: snapping in the front view, group highlight and cut-group hatching"
```

---

### Task 4: Parts list with groups, group inspector, effect chips and symmetry

**Files:**
- Modify: `crates/gui/ui/kit/render.js`, `crates/gui/ui/kit.css`
- Test: `crates/gui/ui/kit/tests/render.test.mjs`

**Interfaces:**
- Consumes: `listRows`, `store.groupOf`, `store.createGroup`, `store.addToGroup`, `store.removeFromGroup`, `store.ungroup`, `store.updateGroup`, `store.selectGroup`, `store.toggleFold`, `store.state.selectedGroup`, `store.state.ui.folded`.
- Produces: pure exports `EFFECT_DEFAULTS = { waves: { count: 6, depth: 0.15, style: "sine" }, round: 0.1, rim: 0.15 }`, `repeatFromField(v) -> number | null` (null below 2, integer 2 to 12 otherwise), `inspectorKeyFor(part) -> string` (id, kind and the three chip states), `KIND_LABELS` unchanged.
- DOM: group rows `div.kt-part.kt-group` with `button.fold` (text `▾` open, `▸` folded), `span.name`, `span.badge` (Add or Cut), `span.count`; member part rows carry class `member`; the group inspector has `input[data-role="group-name"]`, the Mode segmented control, `button[data-role="ungroup"]`, and one `button[data-role="leave"]` per member; the part inspector has `select[data-role="group"]` (options `none`, every group id, `new`), chip buttons `button[data-role="chip-waves"]`, `chip-round`, `chip-rim`, the Mirror segmented control and `input[data-role="repeat"]`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/gui/ui/kit/tests/render.test.mjs` (extend the import with `EFFECT_DEFAULTS, repeatFromField, inspectorKeyFor`):

```js
test("chips_toggle_discretely_and_repeat_below_two_is_off", () => {
  assert.deepEqual(EFFECT_DEFAULTS.waves, { count: 6, depth: 0.15, style: "sine" });
  assert.equal(EFFECT_DEFAULTS.round, 0.1);
  assert.equal(EFFECT_DEFAULTS.rim, 0.15);
  assert.equal(repeatFromField("0"), null);
  assert.equal(repeatFromField("1"), null);
  assert.equal(repeatFromField(""), null);
  assert.equal(repeatFromField("4.7"), 5);
  assert.equal(repeatFromField("40"), 12);
  const p = { id: "p1", shape: { kind: "circle" }, effects: { waves: null, round: 0.1, rim: null, mirror: "none", repeat: null } };
  assert.equal(inspectorKeyFor(p), "p1|circle|0|1|0");
  p.effects.waves = EFFECT_DEFAULTS.waves;
  assert.equal(inspectorKeyFor(p), "p1|circle|1|1|0", "a chip switching on rebuilds the inspector");
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test crates/gui/ui/kit/tests/render.test.mjs`
Expected: FAIL (exports missing).

- [ ] **Step 3: Implement the pure parts**

In `render.js` add:

```js
export const EFFECT_DEFAULTS = { waves: { count: 6, depth: 0.15, style: "sine" }, round: 0.1, rim: 0.15 };
export const REPEAT_MAX = 12;

// The Repeat field: below two copies there is nothing to repeat, so the
// effect is off (null); the kernel caps far higher, the editor at twelve.
export function repeatFromField(v) {
  const n = Math.round(Number(v));
  if (!Number.isFinite(n) || n < 2) return null;
  return Math.min(REPEAT_MAX, n);
}

// What the inspector was built for: the part, its shape kind and which
// chips are on (a chip switching on adds its fields, so it rebuilds).
export function inspectorKeyFor(p) {
  const e = p.effects || {};
  return `${p.id}|${p.shape.kind}|${e.waves ? 1 : 0}|${e.round != null ? 1 : 0}|${e.rim != null ? 1 : 0}`;
}
```

- [ ] **Step 4: The parts list from `listRows`**

Rewrite `buildParts(store, root)` to iterate `listRows(store.state.design, store.state.ui.folded)`. For `kind === "part"` keep the present row (add class `member` when `row.group`); for `kind === "group"` build:

```js
    const row = el("div", { class: "kt-part kt-group" });
    const fold = el("button", { class: "btn small ghost fold", onclick: (e) => { e.stopPropagation(); store.toggleFold(id); } });
    const name = el("span", { class: "name" });
    const mode = el("span", { class: "badge" });
    const count = el("span", { class: "count" });
    row.append(fold, name, mode, count);
    row.addEventListener("click", () => store.selectGroup(id));
    // A part row dropped on a group header joins the group.
    row.addEventListener("dragover", (e) => e.preventDefault());
    row.addEventListener("drop", (e) => {
      e.preventDefault();
      if (!dragPartId) return;
      const from = dragPartId;
      dragPartId = null;
      store.addToGroup(from, id);
    });
    root.append(row);
    partRows.push({ kind: "group", id, row, fold, name, mode, count });
```

`renderParts` computes its key from the rows (`rows.map((r) => `${r.kind}:${r.id}:${r.folded ? 1 : 0}`).join(",")`), rebuilds on change, then patches: group rows get `selected` when `s.selectedGroup === id`, `fold` text `▾` or `▸`, the name, the badge (`Cut` with class `cut`, else `Add`), the count (`${n} parts`); part rows as today, plus `selected` only when `p.id === s.selected`.

In `kit.css` add `.kt-part.kt-group { background: var(--accent-soft); font-weight: 600; } .kt-part.member { margin-left: 1.2rem; } .kt-part .count { font-size: var(--fs-sm); color: var(--muted); } .kt-part .fold { width: 1.6rem; padding: 0; }`.

- [ ] **Step 5: The group inspector and the part inspector sections**

In `renderInspector`: when `store.state.selectedGroup` names an existing group, the key is `g|${id}|${members.join(",")}` and `buildGroupInspector(store, body, g)` runs:

```js
function buildGroupInspector(store, body, g) {
  const id = g.id;
  const get = () => store.state.design.groups.find((x) => x.id === id);
  body.append(el("h2", { text: "Group" }));
  const name = el("input", { type: "text", "data-role": "group-name" });
  setShown(name, g.name);
  name.addEventListener("change", () => {
    const next = name.value.trim();
    if (next && next !== get()?.name) store.updateGroup(id, "group-name", (x) => (x.name = next));
    const now = get();
    if (now) setShown(name, now.name);
  });
  inspectorWidgets.push(() => { const now = get(); if (now) showText(name, now.name); });
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Name" }), name]));
  const modeButtons = [["add", "Add"], ["cut", "Cut"]].map(([v, text]) => [v, el("button", { class: "btn small", text,
    onclick: () => { if (get()?.mode !== v) store.updateGroup(id, "group-mode", (x) => (x.mode = v)); } })]);
  const mark = () => { const now = get(); for (const [v, b] of modeButtons) b.classList.toggle("on", now?.mode === v); };
  mark();
  inspectorWidgets.push(mark);
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Mode" }), el("div", { class: "kt-seg" }, modeButtons.map(([, b]) => b))]));
  body.append(el("div", { class: "hint", text: "A Cut part inside the group carves only this group's Add parts. A Cut group carves everything outside it with the shape of its Add parts." }));
  body.append(el("h2", { text: "Parts" }));
  for (const pid of g.parts) {
    const p = store.part(pid);
    if (!p) continue;
    body.append(el("div", { class: "kt-field" }, [
      el("label", { text: p.name }),
      el("button", { class: "btn small ghost", text: "Select", onclick: () => store.select(pid) }),
      el("button", { class: "btn small ghost", text: "Leave group", "data-role": "leave", onclick: () => store.removeFromGroup(pid) }),
    ]));
  }
  body.append(el("div", { class: "kt-field" }, [el("button", { class: "btn small ghost", text: "Ungroup", "data-role": "ungroup", onclick: () => store.ungroup(id) })]));
}
```

The member list is part of the key, so joins and leaves rebuild it. In `buildInspector` (the part inspector), after the Material row add the Group field:

```js
  const groupSel = el("select", { "data-role": "group" });
  const fillGroups = (q) => {
    const current = store.groupOf(q.id);
    groupSel.replaceChildren(el("option", { value: "none", text: "None" }));
    for (const g of store.state.design.groups) groupSel.append(el("option", { value: g.id, text: g.name }));
    groupSel.append(el("option", { value: "new", text: "New group" }));
    groupSel.value = current ? current.id : "none";
  };
  fillGroups(p);
  groupSel.addEventListener("change", () => {
    const v = groupSel.value;
    if (v === "new") store.createGroup(id);
    else if (v === "none") store.removeFromGroup(id);
    else store.addToGroup(id, v);
  });
  inspectorWidgets.push(fillGroups);
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Group" }), groupSel]));
```

After the Depth section (before Duplicate and Remove) add Effects and Symmetry:

```js
  body.append(el("h2", { text: "Effects" }));
  // A chip: one button that switches the effect on (with its defaults) or
  // off; its fields follow while it is on. Switching rebuilds the panel
  // (the key carries the chip states), which is fine inside a click.
  const chip = (key, label, fields) => {
    const on = get()?.effects[key] != null;
    const btn = el("button", { class: `btn small${on ? " on" : ""}`, text: label, "data-role": `chip-${key}`,
      onclick: () => up(`effect-${key}`, (x) => (x.effects[key] = x.effects[key] == null ? JSON.parse(JSON.stringify(EFFECT_DEFAULTS[key])) : null)) });
    body.append(el("div", { class: "kt-field" }, [el("label", { text: on ? `${label} on` : label }), btn]));
    if (on) for (const f of fields) body.append(f);
  };
  chip("waves", "Waves", [
    slider(p, "Count", (q) => q.effects.waves?.count ?? 6, 2, 16, 1, (v) => up(`waves.count:${id}`, (x) => { if (x.effects.waves) x.effects.waves.count = v; })),
    slider(p, "Depth", (q) => q.effects.waves?.depth ?? 0.15, 0.02, 0.6, 0.01, (v) => up(`waves.depth:${id}`, (x) => { if (x.effects.waves) x.effects.waves.depth = v; })),
    segmented(p, "Style", [["sine", "Sine"], ["spike", "Spike"], ["pulse", "Pulse"]], (q) => q.effects.waves?.style ?? "sine",
      (v) => { if (get()?.effects.waves?.style !== v) up(`waves.style:${id}`, (x) => { if (x.effects.waves) x.effects.waves.style = v; }); }),
  ]);
  chip("round", "Round", [slider(p, "Radius", (q) => q.effects.round ?? 0.1, 0.01, 0.5, 0.01, (v) => up(`round:${id}`, (x) => (x.effects.round = v)))]);
  chip("rim", "Rim", [slider(p, "Thickness", (q) => q.effects.rim ?? 0.15, 0.02, 0.5, 0.01, (v) => up(`rim:${id}`, (x) => (x.effects.rim = v)))]);
  body.append(el("div", { class: "hint", text: "Waves run inward from the part's edge, so a wavy part keeps its size. Round softens corners, Rim keeps only the outline band." }));

  body.append(el("h2", { text: "Symmetry" }));
  body.append(segmented(p, "Mirror", [["none", "None"], ["x", "X"], ["y", "Y"], ["both", "Both"]], (q) => q.effects.mirror,
    (v) => { if (get()?.effects.mirror !== v) up("mirror", (x) => (x.effects.mirror = v)); }));
  const repeat = el("input", { type: "number", min: 0, max: REPEAT_MAX, step: 1, "data-role": "repeat" });
  const showRepeat = (q) => showText(repeat, q.effects.repeat == null ? "0" : String(q.effects.repeat));
  setShown(repeat, p.effects.repeat == null ? "0" : String(p.effects.repeat));
  const commitRepeat = () => {
    const q = get();
    if (!q) return;
    const next = repeatFromField(repeat.value);
    if (next !== q.effects.repeat) up("repeat", (x) => (x.effects.repeat = next));
    const now = get();
    if (now) setShown(repeat, now.effects.repeat == null ? "0" : String(now.effects.repeat));
  };
  repeat.addEventListener("change", commitRepeat);
  repeat.addEventListener("keydown", (e) => { if (e.key === "Enter") commitRepeat(); });
  inspectorWidgets.push(showRepeat);
  body.append(el("div", { class: "kt-field" }, [el("label", { text: "Repeat" }), repeat]));
  body.append(el("div", { class: "hint", text: "Mirror reflects the part across the note's axes; Repeat turns copies around the centre (2 to 12, 0 is off). Copies that overlap merge." }));
```

Change `renderInspector` to use `inspectorKeyFor(p)` for a part (so a chip toggle rebuilds) and to handle the group case first. Extend `render(store, actions)` so that `renderParts` also patches group rows.

- [ ] **Step 6: Run the Node suite**

Run: `node --test crates/gui/ui/kit/tests/*.test.mjs`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/gui/ui/kit/render.js crates/gui/ui/kit.css crates/gui/ui/kit/tests/render.test.mjs
git commit -m "kit: group rows and inspector, effect chips, symmetry controls"
```

---

### Task 5: Help, guide, spec, the committed scenario runner, testbuild kit7

**Files:**
- Modify: `crates/gui/ui/kit/help.js`, `crates/gui/ui/kit/tests/help.test.mjs`, `docs/USER-GUIDE.md`, `docs/superpowers/specs/2026-10-24-note-kit-2-5d-design.md` (5.2, 8)
- Create: `crates/gui/ui/kit/tests/scenarios.mjs`; delete `crates/gui/ui/kit/tests/scenarios/kit-round*.mjs` once consolidated
- Create: `docs/superpowers/plans/2026-10-25-note-kit-phase3-ledger.md` (the controller keeps it)

**Interfaces:**
- `help.js`: new rows in "Parts list" (group header, fold, drop on a header), a new section "Groups and effects" (Group field, Mode of a group, Leave group, Ungroup, Waves, Round, Rim, Mirror, Repeat), "Canvas" gains the Snap button row, "Keyboard" unchanged.
- `scenarios.mjs`: `node crates/gui/ui/kit/tests/scenarios.mjs [filter]` starts `harness.mjs` on a free port, launches headless Chrome through `playwright-core` (path from `PLAYWRIGHT_CORE`, default the npx cache path used in rounds 3 to 5; Chrome from `CHROME`, default `/opt/google/chrome/chrome`), runs the scenario groups `files`, `imports`, `points`, `depth`, `turntable`, `help`, `groups`, `effects`, `snap`, prints PASS/FAIL lines and exits non-zero on a FAIL, closes the browser and stops the harness in `finally`. Not picked up by `node --test` (no `.test.` in the name); the build script skips `tests/`.

- [ ] **Step 1: Write the failing help test**

In `help.test.mjs` extend the shortcut test with a sections test:

```js
test("the_table_covers_groups_effects_and_snapping", () => {
  const titles = HELP.map((s) => s.title);
  assert.ok(titles.includes("Groups and effects"));
  const all = HELP.flatMap((s) => s.rows.map((r) => r[0])).join(" ");
  for (const control of ["Snap", "Group", "Fold", "Ungroup", "Waves", "Round", "Rim", "Mirror", "Repeat", "Leave group"]) {
    assert.ok(all.includes(control), `missing ${control}`);
  }
});
```

Run: `node --test crates/gui/ui/kit/tests/help.test.mjs`
Expected: FAIL.

- [ ] **Step 2: Extend `HELP`**

Add to "Parts list": `["Group header", "Selects the group; its parts sit under it. A Cut group carves everything outside it with the shape of its Add parts."]`, `["Fold triangle", "Hides or shows the group's parts in the list; nothing in the design changes."]`, `["Drop a part on a group header", "The part joins the group."]`. Add to "Canvas (front view)": `["Snap", "Pulls moves, sizes, turns and points to a 0.05 grid and the box lines while the pointer is near one; off, everything is free."]`. Add the section:

```js
  {
    title: "Groups and effects",
    rows: [
      ["Group (inspector)", "Puts the part into a group, takes it out (None) or starts a new one. A Cut part inside a group carves only that group's Add parts."],
      ["Group inspector: Name, Mode", "Names the group; Add contributes its parts, Cut carves them out of everything else."],
      ["Leave group, Ungroup", "Takes one part out, or dissolves the group and keeps its parts."],
      ["Waves", "Ripples the outline inward from the edge: count, depth and style (sine, spike, pulse)."],
      ["Round", "Softens the part's corners by the radius."],
      ["Rim", "Keeps only the outline band of that thickness."],
      ["Mirror", "Reflects the part across the note's axes (X, Y or both)."],
      ["Repeat", "Turns copies of the part around the centre, 2 to 12; 0 switches it off."],
    ],
  },
```

Regenerate the guide table: `node -e 'import("./crates/gui/ui/kit/help.js").then(m => process.stdout.write(m.helpMarkdown()))'` and replace the table under "### Note Kit controls" in `docs/USER-GUIDE.md`; extend the Note Kit paragraph with one sentence each on groups, effects, symmetry and Snap. Spec 5.2: mark groups, effect chips, symmetry and snapping as landed with the rulings (inward waves, no multi-selection, snap defaults); spec 8: Phase 3 complete except what the owner's test round adds.

Run: `node --test crates/gui/ui/kit/tests/*.test.mjs`
Expected: PASS.

- [ ] **Step 3: The scenario runner**

Create `crates/gui/ui/kit/tests/scenarios.mjs` from the round 3 to 5 scripts (`tests/scenarios/kit-round3.mjs`, `kit-round4.mjs`, `kit-round5.mjs`): one file, the shared helpers (`settle`, `S`, `dlg`, `keys`, `geo`, `pixel`, `stats`, `drag`) at the top, fixtures written into a temp directory the runner creates (the badge and cloud `.svg` stand-ins, the v2 `d.json` and the old designer `old.rhsd` texts from round 3 inlined as strings), the harness started with `spawn(process.execPath, [harness], { env: { ...process.env, PORT } })` and killed in `finally`, Chrome launched with `--use-gl=angle --use-angle=swiftshader --enable-unsafe-swiftshader`. Scenario groups as functions `files(page)`, `imports(page)`, `points(page)`, `depth(page)`, `turntable(page)`, `help(page)` carrying the existing checks, plus the new:

- `groups(page)`: select p2 of a two-part design, choose "New group" in the Group select, expect a header row above p2 with the group selected and the inspector showing Name and Mode; drag p1's row onto the header (`dragstart`, `dragover`, `drop` dispatched with a `DataTransfer`), expect both members contiguous and the count "2 parts"; set the group Mode to Cut and expect both member outlines hatched (a pixel inside p2 differs from the solid fill); fold hides member rows and the serialised design is unchanged; "Leave group" on p1; "Ungroup"; undo chain back to no groups.
- `effects(page)`: switch Waves on, expect `effects.waves` equal to the defaults and the Count slider present, change Count to 8, expect one undo step per gesture; Round on then off leaves Waves alone; Rim on a square shows a hole in the evaluated geometry (the part's shapes carry a second contour); Mirror X on a part at x 0.5 yields two resolved shapes; Repeat 4 yields four; Repeat 1 stores null.
- `snap(page)`: with Snap on, drag the disc to about x 0.512 and expect x exactly 0.5; switch Snap off (button) and the same drag leaves 0.51-ish; scale a corner to about hw 0.49 and expect 0.5; rotate to about 44 degrees and expect 45; in point editing drag a point to about 0.26 and expect 0.25; Snap state survives a reload.

Run: `node crates/gui/ui/kit/tests/scenarios.mjs`
Expected: every line PASS; the final summary names the counts.

Delete the three raw scripts under `tests/scenarios/` once the runner covers their checks.

- [ ] **Step 4: The suites and the testbuild**

Run in this order (one cargo at a time): `cargo fmt --all`, `cargo test -p rhythia-render --lib kit::`, `cargo test -p rhythia-gui`, `cargo test -p rhythia-render --test kit_properties`, `cargo build -p rhythia-render --examples`, `node --test crates/gui/ui/kit/tests/*.test.mjs`, `node crates/gui/ui/kit/tests/scenarios.mjs`, `bash scripts/preflight.sh`. Then from `crates/gui`: `/home/killer/tools/tauri-cli/node_modules/.bin/tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc` (the updater signing error at the end is expected) and copy `target/x86_64-pc-windows-msvc/release/bundle/nsis/rhythr_0.6.2_x64-setup.exe` to `/home/killer/RhythiaRenderer/rhythr_0.6.2_x64-setup_TESTBUILD-kit7.exe`.

- [ ] **Step 5: Commit**

```bash
git add crates/gui/ui/kit/help.js crates/gui/ui/kit/tests/help.test.mjs crates/gui/ui/kit/tests/scenarios.mjs docs/USER-GUIDE.md docs/superpowers/specs/2026-10-24-note-kit-2-5d-design.md docs/superpowers/plans/2026-10-25-note-kit-phase3-ledger.md
git rm -r crates/gui/ui/kit/tests/scenarios
git commit -m "kit: help and guide for groups, effects and snapping; headless scenario runner"
```

Then the Discord note to the owner with the testbuild path, as in the earlier rounds (real time from `date +%H:%M`).

---

## Self-review notes

- Spec coverage for Phase 3 (section 8): groups (Tasks 2, 4), effects chips (Task 4), symmetry (Task 4), snapping (Task 3), glass (landed in Phase 2: material switch, canvas alpha, turntable alpha), point edit and turntable (landed early). The headless UI checks of spec section 7 become the committed runner (Task 5).
- Placeholders: none; every step carries its code or an exact instruction.
- Type consistency: `listRows` rows carry `kind`, `id`, `group` or `name`, `mode`, `folded`, `count` in Tasks 2 and 4; `EFFECT_DEFAULTS`, `repeatFromField`, `inspectorKeyFor` are defined in Task 4 and used only there; the snapping functions are defined in Task 3 and used only there; store labels listed in Task 2 are the ones the inspector commits with in Task 4 (`effect-waves`, `effect-round`, `effect-rim`, `mirror`, `repeat`, `group-mode`, `group-name`).
- Review Focus: item 1 pinned in Task 2, item 2 in Task 1, item 3 in Task 3, item 4 in Task 4, item 5 in Task 2 and the runner.
- Open for the owner before execution (answers change Task 1, 2 and 3 text only): inward waves (yes or no), groups without multi-selection (yes or no), Snap on by default with 0.05 (yes or no).
