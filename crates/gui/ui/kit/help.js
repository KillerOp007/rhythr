// crates/gui/ui/kit/help.js
// Every control of the Note Kit window in one table, shown by the Help
// button (F1): what each button, mouse action and key does. The same table
// is in docs/USER-GUIDE.md; keep the two in step.

export const HELP = [
  {
    title: "Toolbar",
    rows: [
      ["Disc, Square, Polygon, Star, Ring, Pill", "Add a part of that shape at the centre, one note wide, and select it."],
      ["SVG", "Add an SVG file: one part per colour, strokes outlined at their width; a colour drawn on another colour comes in as a Cut."],
      ["Letter field, then Enter or the Letter button", "Add the typed letter, digit, symbol or emoji (any installed font) as a part."],
      ["Undo, Redo", "Step back and forward through every change (Ctrl+Z, Ctrl+Y)."],
      ["Previews", "Show the still and the gameplay loop from the real renderer below the canvas (P)."],
      ["3D", "Show the note turned, to judge its depth (3)."],
      ["Help", "This table (F1 or ?)."],
      ["New", "Start a fresh note; asks first when there are unsaved changes."],
      ["Open", "Open a skin saved by rhythr's editors (.rhs) or a design file (.rhsd) (Ctrl+O)."],
      ["Recent", "Reopen one of the last six files."],
      ["Save .rhs", "Write the one file that is the skin for rhythr and the game with your design inside (Ctrl+S). A design the kernel cannot build is not written; the status says why."],
      ["Save as", "Write to a new .rhs file (Ctrl+Shift+S)."],
      ["From old designer", "Bring in the note from the old Note Designer's autosave."],
    ],
  },
  {
    title: "Parts list",
    rows: [
      ["Click a row", "Select the part."],
      ["Drag a row", "Reorder the parts; the top part is picked first by a click on the canvas. The order changes no geometry."],
      ["Eye", "Hide or show the part; a hidden part adds and cuts nothing."],
      ["Lock", "Lock the part against moving, scaling, rotating and point edits."],
      ["Number", "The part's Front depth. Parts at one depth merge into one surface; a higher Front stands out."],
      ["Add, Cut, Solid, Glass", "The part's mode and material, set in the inspector."],
      ["Group header", "Selects the group; its parts sit under it. A Cut group carves everything outside it with the shape of its Add parts."],
      ["Fold triangle", "Hides or shows the group's parts in the list; nothing in the design changes."],
      ["Drop a part on a group header", "The part joins the group."],
    ],
  },
  {
    title: "Canvas (front view)",
    rows: [
      ["Click", "Select the part under the pointer, top of the list first; empty space clears the selection."],
      ["Drag a part", "Move it."],
      ["Drag a corner handle", "Scale it; Shift keeps the proportions."],
      ["Drag an edge handle", "Scale along that axis only."],
      ["Drag the handle above the part", "Rotate it; Shift snaps to 15 degrees."],
      ["Double-click a part", "Edit its outline point by point (any shape is converted to points first)."],
      ["Wheel", "Zoom about the pointer."],
      ["Middle drag, or Space and drag", "Pan the view."],
      ["100% button", "Reset zoom and pan."],
      ["Snap", "Pulls moves, sizes, turns and points to a 0.05 grid and the box lines while the pointer is near one; off, everything is free."],
      ["Solid square", "The note box: whatever a part puts outside it is cut off, nothing is rescaled."],
      ["Dashed square", "The game's true hit area."],
      ["Shades", "The highest part keeps the colour, lower parts draw darker; Cut parts hatch."],
    ],
  },
  {
    title: "Point editing",
    rows: [
      ["Drag a point", "Move it; Shift snaps to a grid of the part's own frame."],
      ["Click the dashed outline", "Insert a point there and pick it up (a corner between two corners, smooth otherwise)."],
      ["Double-click a point", "Switch between smooth (round) and corner (square)."],
      ["Right-click a point, or Delete", "Remove it; a loop keeps three points, a three-point loop goes as a whole unless it is the last."],
      ["Arrows", "Nudge the selected point by 0.01, with Shift by 0.1."],
      ["Esc, a click away from the outline, or Done", "Finish editing."],
    ],
  },
  {
    title: "3D (turned view)",
    rows: [
      ["Drag", "Turn the note, up to 60 degrees each way."],
      ["Wheel", "Zoom."],
      ["Front, 3/4", "Snap to the front or to a view from the side and above."],
      ["Shaded", "Shade the faces so relief reads. The game draws the note flat; relief shows there only on the approach."],
      ["Fade", "Draw the note translucent like the game's approach fade, where overlapping parts brighten."],
    ],
  },
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
  {
    title: "Inspector",
    rows: [
      ["Name", "The part's name in the list."],
      ["Shape", "Swap the shape kind; its own fields (sides, rounding, points, thickness, roundness) follow."],
      ["To points, Edit points, Done", "Convert the outline to points, start or finish editing them."],
      ["Mode", "Add puts material, Cut takes it away from the Add parts that share its depth."],
      ["Material", "Solid, or Glass, which the game draws translucent."],
      ["X, Y, Width, Height, Rotation", "The part's place and size in note units (the box is 1 from the centre to its edge)."],
      ["Front, Back", "The depth band; Front faces the player. The game's own disc spans 0.1 to -0.1."],
      ["Duplicate, Remove", "Copy the part above itself, or delete it."],
      ["Colors", "The colours the notes cycle through, in beat order; drag to reorder, the first one colours the canvas."],
    ],
  },
  {
    title: "Keyboard",
    rows: [
      ["Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z", "Undo, redo."],
      ["Ctrl+S, Ctrl+Shift+S, Ctrl+O", "Save, save as, open."],
      ["Ctrl+D", "Duplicate the selected part."],
      ["Delete, Backspace", "Remove the selected part, or the selected point while editing points."],
      ["Arrows, Shift+Arrows", "Nudge the selected part (or point) by 0.01 or 0.1."],
      ["Esc", "Clear the selection, or finish point editing."],
      ["P, 3, F1 or ?", "Previews, 3D view, this help."],
      ["Space and drag", "Pan the front view."],
    ],
  },
];

// Builds the help tables into `root` (any element); DOM only here.
export function renderHelp(root, sections = HELP) {
  root.replaceChildren();
  for (const section of sections) {
    const h = document.createElement("h3");
    h.textContent = section.title;
    const table = document.createElement("table");
    for (const [control, action] of section.rows) {
      const tr = document.createElement("tr");
      const a = document.createElement("td");
      a.textContent = control;
      const b = document.createElement("td");
      b.textContent = action;
      tr.append(a, b);
      table.append(tr);
    }
    root.append(h, table);
  }
}

// The same table as Markdown, for the user guide.
export function helpMarkdown(sections = HELP) {
  const out = [];
  for (const section of sections) {
    out.push(`**${section.title}**`, "", "| Control | What it does |", "|---|---|");
    for (const [control, action] of section.rows) out.push(`| ${control} | ${action} |`);
    out.push("");
  }
  return out.join("\n");
}
