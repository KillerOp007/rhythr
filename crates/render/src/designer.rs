//! Geometry for the note designer: a closed 2D outline (drawn or
//! generated in the designer window) becomes an extruded note mesh, plus
//! its `.obj`/`.mtl` export.
//!
//! The whole pipeline works in the note convention the renderer and the
//! game share: x/y half-extent 1, a thin z extrusion. That unit cap is
//! also a HARD size rule: the game's true hit area is a quarter larger
//! than the note (hitbox half 0.56875 cells over note half 0.45), and a
//! note drawn bigger than its hitbox lies to the player about what a hit
//! is. Everything this module emits is normalised to the unit square, so
//! a designed note can never outgrow the box that judges it, in rhythr
//! or in the game (which renders meshes at their authored scale). Every real note
//! mesh observed (the game's four built-ins and a player export) is
//! exactly this: a flat silhouette pulled into a slab, so the designer
//! never needs 3D editing, only a good outline.

use crate::mesh::{Mesh, Vertex};
use crate::Error;

/// One closed outline, counter-clockwise, in the unit plane.
type Outline = Vec<[f32; 2]>;

/// Signed area (positive = counter-clockwise).
fn signed_area(pts: &[[f32; 2]]) -> f32 {
    let mut a = 0.0;
    for i in 0..pts.len() {
        let p = pts[i];
        let q = pts[(i + 1) % pts.len()];
        a += p[0] * q[1] - q[0] * p[1];
    }
    a * 0.5
}

/// Ear-clipping triangulation of a simple polygon (concave is fine, which
/// wavy outlines are; self-intersecting is not and errors out instead of
/// spinning). Returns index triples into `pts`.
fn triangulate(pts: &[[f32; 2]]) -> Result<Vec<[u32; 3]>, Error> {
    let n = pts.len();
    if n < 3 {
        return Err(Error::BadObj);
    }
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| -> f32 {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let inside = |a: [f32; 2], b: [f32; 2], c: [f32; 2], p: [f32; 2]| -> bool {
        // Strictly inside the triangle abc (CCW), with tolerance RELATIVE
        // to the triangle's own size: a fixed slack swallowed the thin
        // ears of dense outlines whole (every neighbour of a razor-thin
        // ear sat within the absolute slack and "blocked" it, failing
        // perfectly valid shapes).
        let eps = cross(a, b, c).abs() * 1e-3 + 1e-12;
        cross(a, b, p) > -eps && cross(b, c, p) > -eps && cross(c, a, p) > -eps
    };
    let mut idx: Vec<u32> = (0..n as u32).collect();
    let mut tris = Vec::with_capacity(n - 2);
    let mut guard = 0usize;
    while idx.len() > 3 {
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (pi, ci, ni) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (pts[pi as usize], pts[ci as usize], pts[ni as usize]);
            if cross(a, b, c) <= 1e-9 {
                continue; // reflex or degenerate corner: not an ear
            }
            let blocked = idx
                .iter()
                .any(|&j| j != pi && j != ci && j != ni && inside(a, b, c, pts[j as usize]));
            if blocked {
                continue;
            }
            tris.push([pi, ci, ni]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // No ear anywhere: the outline crosses itself (or collapsed).
            // Erroring is the honest outcome; the caller shows the outline
            // and the user can see the crossing.
            return Err(Error::BadObj);
        }
        guard += 1;
        if guard > 100_000 {
            return Err(Error::BadObj);
        }
    }
    tris.push([idx[0], idx[1], idx[2]]);
    Ok(tris)
}

/// Insets the outline by `t` along each vertex's inward angle bisector.
/// Keeps the vertex count (and therefore the outer/inner correspondence
/// the ring faces need). A spike sharper than the inset allows can fold
/// the inner contour; the designer's live preview makes that visible, and
/// the mitre length is clamped so it cannot explode.
fn inset(pts: &[[f32; 2]], t: f32) -> Outline {
    let n = pts.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let p = pts[(i + n - 1) % n];
        let c = pts[i];
        let q = pts[(i + 1) % n];
        let norm = |v: [f32; 2]| -> [f32; 2] {
            let l = (v[0] * v[0] + v[1] * v[1]).sqrt().max(1e-9);
            [v[0] / l, v[1] / l]
        };
        // Inward normals of the two edges meeting at c (outline is CCW, so
        // inward is the right-hand side of the direction of travel).
        let d1 = norm([c[0] - p[0], c[1] - p[1]]);
        let d2 = norm([q[0] - c[0], q[1] - c[1]]);
        let n1 = [d1[1], -d1[0]];
        let n2 = [d2[1], -d2[0]];
        let mut b = norm([n1[0] + n2[0], n1[1] + n2[1]]);
        // Mitre scale so the inset distance holds through the corner,
        // clamped: a near-reversal would otherwise shoot the vertex out.
        let dot = (b[0] * n1[0] + b[1] * n1[1]).clamp(0.25, 1.0);
        b = [b[0] / dot, b[1] / dot];
        let mut p = [c[0] - b[0] * t, c[1] - b[1] * t];
        // An inner vertex further from the centre than its outer partner
        // means the mitre shot past the silhouette on a spike; pull it
        // back inside so the rim can fold visually (the preview shows it)
        // but never outgrow the shape it rims.
        let (ro, ri) = (
            (c[0] * c[0] + c[1] * c[1]).sqrt(),
            (p[0] * p[0] + p[1] * p[1]).sqrt(),
        );
        if ri > ro * 0.98 && ri > 1e-9 {
            let s = ro * 0.98 / ri;
            p = [p[0] * s, p[1] * s];
        }
        out.push(p);
    }
    // De-loop pass 1: where the rim is thicker than the shape is wide,
    // an inset segment runs BACKWARDS against its outer segment;
    // collapsing it to its midpoint removes the direct reversal.
    let n = out.len();
    for _ in 0..6 {
        let mut changed = false;
        for i in 0..n {
            let j = (i + 1) % n;
            let od = [pts[j][0] - pts[i][0], pts[j][1] - pts[i][1]];
            let id = [out[j][0] - out[i][0], out[j][1] - out[i][1]];
            if od[0] * id[0] + od[1] * id[1] < 0.0 {
                let mid = [(out[i][0] + out[j][0]) * 0.5, (out[i][1] + out[j][1]) * 0.5];
                out[i] = mid;
                out[j] = mid;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // De-loop pass 2: sharp curvature makes the inset cross NEIGHBOURING
    // segments, leaving little bow-tie loops that render as black
    // arrow-shaped notches punched into the rim. Every crossing has the
    // loop between its two segments collapsed onto the intersection
    // point, which turns the notch into a clean pinch. n is capped at
    // 512, so the quadratic sweep stays well under a millisecond.
    fn seg_hit(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> Option<[f32; 2]> {
        let r = [b[0] - a[0], b[1] - a[1]];
        let s2 = [d[0] - c[0], d[1] - c[1]];
        let den = r[0] * s2[1] - r[1] * s2[0];
        if den.abs() < 1e-12 {
            return None;
        }
        let t = ((c[0] - a[0]) * s2[1] - (c[1] - a[1]) * s2[0]) / den;
        let u = ((c[0] - a[0]) * r[1] - (c[1] - a[1]) * r[0]) / den;
        ((0.001..0.999).contains(&t) && (0.001..0.999).contains(&u))
            .then(|| [a[0] + t * r[0], a[1] + t * r[1]])
    }
    for _ in 0..8 {
        let n = out.len();
        let mut fixed = false;
        'outer: for i in 0..n {
            // Only nearby crossings: a rim loop is local by nature, and
            // distant intersections would be the shape itself.
            for off in 2..12.min(n - 1) {
                let j = (i + off) % n;
                let (a, b) = (out[i], out[(i + 1) % n]);
                let (c, d) = (out[j], out[(j + 1) % n]);
                if let Some(p) = seg_hit(a, b, c, d) {
                    let mut k = (i + 1) % n;
                    loop {
                        out[k] = p;
                        if k == j {
                            break;
                        }
                        k = (k + 1) % n;
                    }
                    fixed = true;
                    break 'outer;
                }
            }
        }
        if !fixed {
            break;
        }
    }
    out
}

/// The 2D truth of a note before extrusion: the cleaned, normalised
/// outer contour and, for hollow notes, the inset inner one. The editor
/// canvas draws EXACTLY this (it asks the backend rather than porting the
/// maths), and the mesh extrudes exactly this, so the note being edited
/// and the note being rendered cannot drift apart.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NotePlan {
    pub outer: Vec<[f32; 2]>,
    pub inner: Option<Vec<[f32; 2]>>,
    /// The factor the normalisation applied to the input (1.0 when the
    /// outline already sat at unit scale). The editor maps its control
    /// points through this so the handles stay ON the contour it draws.
    pub scale: f32,
}

/// Cleans and normalises an outline and computes the rim contour: the
/// single source both the canvas and the mesh build from.
pub fn plan_note(outline: &[[f32; 2]], hollow: Option<f32>) -> Result<NotePlan, Error> {
    if outline.len() < 3
        || outline
            .iter()
            .any(|p| !p[0].is_finite() || !p[1].is_finite())
    {
        return Err(Error::BadObj);
    }
    let mut pts: Outline = outline.to_vec();
    // A closed polygon often arrives with its first point repeated at the
    // end, and a freeform editor lets two points land on the same spot.
    // Exact or near duplicates poison ear clipping (the duplicate is
    // never an ear and blocks every ear that touches it), so they go
    // first, along with densification beyond what triangulation can hold
    // apart numerically: an outline is capped to 512 well-spaced points,
    // which is far beyond anything a note's silhouette needs.
    while pts.len() > 3 && {
        let (f, l) = (pts[0], pts[pts.len() - 1]);
        (f[0] - l[0]).abs() < 1e-6 && (f[1] - l[1]).abs() < 1e-6
    } {
        pts.pop();
    }
    pts.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4);
    if pts.len() > 512 {
        let stride = pts.len().div_ceil(512);
        pts = pts.iter().step_by(stride).copied().collect();
    }
    if signed_area(&pts) < 0.0 {
        pts.reverse(); // normalise to CCW
    }
    if signed_area(&pts).abs() < 1e-6 {
        return Err(Error::BadObj);
    }
    // Normalise the OUTLINE, not the finished mesh: the hollow inset can
    // overshoot on spiky shapes, and a mesh-wide normalisation would then
    // let runaway INNER vertices shrink the user's drawn silhouette
    // relative to its hitbox. The outer contour is the shape's truth.
    let m = pts
        .iter()
        .map(|p| p[0].abs().max(p[1].abs()))
        .fold(0.0f32, f32::max);
    let mut scale = 1.0f32;
    if m > 1e-6 && (m - 1.0).abs() > 1e-3 {
        scale = 1.0 / m;
        for p in &mut pts {
            p[0] *= scale;
            p[1] *= scale;
        }
    }
    let inner = hollow.map(|t| inset(&pts, t.clamp(0.01, 0.9)));
    Ok(NotePlan {
        outer: pts,
        inner,
        scale,
    })
}

/// Builds the note mesh from a closed outline.
///
/// `hollow`: None fills the silhouette; Some(t) keeps a rim of thickness
/// `t` (in outline units) and opens the middle, the way the game's thin
/// skins look. `depth` is the half-extrusion in the same units.
pub fn build_note_mesh(
    outline: &[[f32; 2]],
    hollow: Option<f32>,
    depth: f32,
) -> Result<Mesh, Error> {
    let plan = plan_note(outline, hollow)?;
    let pts = plan.outer;
    let d = depth.clamp(0.005, 0.5);

    let mut mesh = Mesh::default();
    // Emits one contour's front/back vertices; returns the base index and
    // stride to address (front, back) pairs.
    let mut push_contour = |pts: &[[f32; 2]]| -> u32 {
        let base = mesh.vertices.len() as u32;
        for p in pts {
            mesh.vertices.push(Vertex {
                pos: [p[0], p[1], d],
            });
        }
        for p in pts {
            mesh.vertices.push(Vertex {
                pos: [p[0], p[1], -d],
            });
        }
        base
    };
    // Walls along a contour. `flip` turns the quads inside-out for inner
    // rims, so their outward face points into the hole and survives
    // backface culling from the viewer's side.
    let wall = |mesh: &mut Mesh, base: u32, n: u32, flip: bool| {
        for i in 0..n {
            let j = (i + 1) % n;
            let (fa, fb) = (base + i, base + j);
            let (ba, bb) = (base + n + i, base + n + j);
            let quad = if flip {
                [[fa, bb, ba], [fa, fb, bb]]
            } else {
                [[fa, ba, bb], [fa, bb, fb]]
            };
            for t in quad {
                mesh.indices.extend_from_slice(&t);
            }
        }
    };

    match hollow {
        None => {
            let tris = triangulate(&pts)?;
            let n = pts.len() as u32;
            let base = push_contour(&pts);
            for [a, b, c] in &tris {
                // Front face (z = +d) keeps CCW; back face reverses.
                mesh.indices
                    .extend_from_slice(&[base + a, base + b, base + c]);
                mesh.indices
                    .extend_from_slice(&[base + n + a, base + n + c, base + n + b]);
            }
            wall(&mut mesh, base, n, false);
        }
        Some(_) => {
            let inner = plan.inner.clone().expect("hollow plan carries the rim");
            let n = pts.len() as u32;
            let ob = push_contour(&pts);
            let ib = push_contour(&inner);
            // Front and back rim faces as quads between the contours.
            for i in 0..n {
                let j = (i + 1) % n;
                let (oa, obx) = (ob + i, ob + j);
                let (ia, ibx) = (ib + i, ib + j);
                mesh.indices.extend_from_slice(&[oa, obx, ibx]);
                mesh.indices.extend_from_slice(&[oa, ibx, ia]);
                let (oa, obx) = (ob + n + i, ob + n + j);
                let (ia, ibx) = (ib + n + i, ib + n + j);
                mesh.indices.extend_from_slice(&[oa, ibx, obx]);
                mesh.indices.extend_from_slice(&[oa, ia, ibx]);
            }
            wall(&mut mesh, ob, n, false);
            wall(&mut mesh, ib, n, true);
        }
    }
    mesh.normalize_to_unit();
    Ok(mesh)
}

/// The exported material. Always written alongside the .obj, with the
/// name the faces reference: the game crashes at map end on a mesh skin
/// whose .mtl was not bundled, and no export of ours may reproduce that.
pub const NOTE_MTL: &str = "# rhythr note designer\nnewmtl Material.001\nNs 250.000000\nKa 1.000000 1.000000 1.000000\nKd 1.000000 1.000000 1.000000\nKs 0.000000 0.000000 0.000000\nKe 0.000000 0.000000 0.000000\nNi 1.450000\nd 1.000000\nillum 1\n";

/// Wavefront text for the mesh, referencing the bundled material.
pub fn to_obj(mesh: &Mesh, name: &str) -> String {
    let mut s = String::with_capacity(mesh.vertices.len() * 32);
    s.push_str("# rhythr note designer\n");
    s.push_str("mtllib note.mtl\n");
    s.push_str(&format!("o {name}\n"));
    for v in &mesh.vertices {
        s.push_str(&format!(
            "v {:.6} {:.6} {:.6}\n",
            v.pos[0], v.pos[1], v.pos[2]
        ));
    }
    s.push_str("usemtl Material.001\n");
    for t in mesh.indices.chunks_exact(3) {
        s.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
    }
    s
}

/// A complete, shareable `.rhs` skin pack: the mesh, its material, the
/// colorset, and a minimal config naming them. The same container the
/// game exports, loadable by rhythr's `--config` and importable as a
/// skin, so a note designed here works in both worlds.
pub fn export_rhs(mesh: &Mesh, name: &str, colors_hex: &[String]) -> Result<Vec<u8>, Error> {
    use std::io::Write;
    let obj = to_obj(mesh, name);
    let colorset: String = colors_hex
        .iter()
        .filter(|c| {
            let h = c.trim().trim_start_matches('#');
            h.len() == 6 && h.chars().all(|ch| ch.is_ascii_hexdigit())
        })
        .map(|c| {
            let h = c.trim().trim_start_matches('#');
            format!("#{h}\r\n")
        })
        .collect();
    let config =
        format!(r#"{{"NoteSkin":{{"Value":"{name}.obj"}},"ColorSet":{{"Value":"{name}"}}}}"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let write = |zw: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>,
                 path: &str,
                 bytes: &[u8]|
     -> Result<(), Error> {
        zw.start_file(path, opts).map_err(|_| Error::BadObj)?;
        zw.write_all(bytes).map_err(|_| Error::BadObj)?;
        Ok(())
    };
    write(&mut zw, "config", config.as_bytes())?;
    write(&mut zw, &format!("noteSkin/{name}.obj"), obj.as_bytes())?;
    // The .mtl rides along unconditionally: the game crashes at map end
    // on a mesh whose material file was not bundled.
    write(&mut zw, "noteSkin/note.mtl", NOTE_MTL.as_bytes())?;
    if !colorset.is_empty() {
        write(
            &mut zw,
            &format!("colorSet/{name}.txt"),
            colorset.as_bytes(),
        )?;
    }
    let cur = zw.finish().map_err(|_| Error::BadObj)?;
    Ok(cur.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<[f32; 2]> {
        vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
    }

    /// A wavy circle: the shape the whole designer exists for.
    fn wavy(n: usize, waves: f32, amp: f32) -> Vec<[f32; 2]> {
        (0..n)
            .map(|i| {
                let th = i as f32 / n as f32 * std::f32::consts::TAU;
                let r = 1.0 + amp * (waves * th).sin();
                [r * th.cos(), r * th.sin()]
            })
            .collect()
    }

    #[test]
    fn filled_square_is_a_closed_slab() {
        let m = build_note_mesh(&square(), None, 0.05).unwrap();
        assert_eq!(m.vertices.len(), 8);
        // 2 front + 2 back + 8 wall triangles.
        assert_eq!(m.indices.len() / 3, 12);
        // Every index in range.
        assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
    }

    #[test]
    fn hollow_ring_keeps_correspondence_and_range() {
        let m = build_note_mesh(&square(), Some(0.2), 0.05).unwrap();
        assert_eq!(m.vertices.len(), 16);
        assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
        // Rim: 4 quads front + 4 back = 16 tris, walls: 8 + 8 = 16.
        assert_eq!(m.indices.len() / 3, 32);
    }

    #[test]
    fn a_wavy_circle_triangulates_despite_being_concave() {
        let outline = wavy(96, 8.0, 0.25);
        let m = build_note_mesh(&outline, None, 0.05).unwrap();
        assert_eq!(m.indices.len() / 3, (96 - 2) + (96 - 2) + 96 * 2);
        let hollow = build_note_mesh(&outline, Some(0.15), 0.05).unwrap();
        assert!(hollow
            .indices
            .iter()
            .all(|&i| (i as usize) < hollow.vertices.len()));
    }

    /// The renderer's convention: the export must parse back through the
    /// same loader real skins take, and land at unit scale.
    #[test]
    fn obj_roundtrip_through_the_skin_loader() {
        let m = build_note_mesh(&wavy(48, 5.0, 0.3), Some(0.2), 0.06).unwrap();
        let obj = to_obj(&m, "designer_note");
        let back = Mesh::from_obj_str(&obj).unwrap();
        assert_eq!(back.vertices.len(), m.vertices.len());
        assert_eq!(back.indices, m.indices);
        let max = back
            .vertices
            .iter()
            .map(|v| v.pos[0].abs().max(v.pos[1].abs()))
            .fold(0.0f32, f32::max);
        assert!((max - 1.0).abs() < 1e-3, "unit convention, got {max}");
        assert!(obj.contains("mtllib note.mtl"), "the mtl is never optional");
        assert!(NOTE_MTL.contains("newmtl Material.001"));
    }

    /// The pack has to load back through the SAME .rhs path real skins
    /// take: mesh in the mesh slot, colours parsed, material bundled.
    #[test]
    fn exported_rhs_loads_back_as_a_skin() {
        let m = build_note_mesh(&wavy(48, 6.0, 0.2), Some(0.2), 0.05).unwrap();
        let pack = export_rhs(
            &m,
            "meine_note",
            &["#ff0080".into(), "#00a8ff".into(), "kaputt".into()],
        )
        .unwrap();
        let cfg = crate::config::SkinConfig::from_rhs(&pack).unwrap();
        let mesh_bytes = cfg.note_mesh.as_deref().expect("mesh slot");
        let back = Mesh::from_obj_str(std::str::from_utf8(mesh_bytes).unwrap()).unwrap();
        assert_eq!(back.indices, m.indices);
        assert_eq!(cfg.colorset.len(), 2, "the broken colour is dropped");
        assert!((cfg.colorset[0][0] - 1.0).abs() < 0.01, "#ff0080 red");
        // And the mtl is in the archive, name-matched to the obj's usemtl.
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(pack)).unwrap();
        let mut mtl = String::new();
        std::io::Read::read_to_string(
            &mut z.by_name("noteSkin/note.mtl").expect("mtl bundled"),
            &mut mtl,
        )
        .unwrap();
        assert!(mtl.contains("newmtl Material.001"));
    }

    /// The review's bench: a dense, perfectly valid outline used to fail
    /// as "crosses itself" because the ear test's absolute slack swallowed
    /// razor-thin ears whole. Dense input now decimates and the slack is
    /// relative, so valid shapes of any sampling density build.
    #[test]
    fn dense_valid_outlines_build_instead_of_failing() {
        for n in [1600usize, 2000, 4000] {
            let circle: Vec<[f32; 2]> = (0..n)
                .map(|i| {
                    let th = i as f32 / n as f32 * std::f32::consts::TAU;
                    [th.cos(), th.sin()]
                })
                .collect();
            let m = build_note_mesh(&circle, None, 0.05)
                .unwrap_or_else(|_| panic!("dense circle n={n} failed"));
            // Decimation keeps the cost bounded.
            assert!(m.vertices.len() <= 512 * 2 + 4, "n={n} not decimated");
        }
    }

    /// Freeform editors produce these: two points on the same spot, and
    /// the closed-polygon convention with the first point repeated last.
    #[test]
    fn duplicate_points_are_cleaned_not_fatal() {
        let mut sq = square();
        sq.push(sq[0]); // first == last
        assert!(build_note_mesh(&sq, None, 0.05).is_ok());
        let stacked = vec![
            [-1.0, -1.0],
            [1.0, -1.0],
            [1.0, -1.0], // exact duplicate from stacked corner points
            [1.0, 1.0],
            [-1.0, 1.0],
        ];
        assert!(build_note_mesh(&stacked, None, 0.05).is_ok());
        assert!(build_note_mesh(&stacked, Some(0.2), 0.05).is_ok());
    }

    /// The inset clamp: at slider maxima the mitre used to shoot past the
    /// silhouette, and normalising from the whole mesh then shrank the
    /// drawn shape ~20% against its hitbox. The outer contour now defines
    /// the scale and inner vertices stay inside it.
    #[test]
    fn spiky_hollow_keeps_the_outer_silhouette_at_unit_scale() {
        let m = build_note_mesh(&wavy(160, 24.0, 0.6), Some(0.6), 0.05).unwrap();
        let max = m
            .vertices
            .iter()
            .map(|v| v.pos[0].abs().max(v.pos[1].abs()))
            .fold(0.0f32, f32::max);
        assert!(
            (max - 1.0).abs() < 0.02,
            "outer silhouette must define the unit scale, got {max}"
        );
    }

    /// The hard rule: nothing this module emits may exceed the unit
    /// square, because the game renders meshes at authored scale and its
    /// hit area is only a quarter larger than the unit note. An outline
    /// handed in at ANY size (a buggy or hostile frontend included) must
    /// come out capped.
    #[test]
    fn no_export_can_outgrow_the_hitbox() {
        const HITBOX: f32 = 0.56875 / 0.45;
        for scale in [0.2f32, 1.0, 1.26, 5.0, 50.0] {
            let big: Vec<[f32; 2]> = wavy(96, 6.0, 0.3)
                .into_iter()
                .map(|p| [p[0] * scale, p[1] * scale])
                .collect();
            for hollow in [None, Some(0.2)] {
                let m = build_note_mesh(&big, hollow, 0.05).unwrap();
                let max = m
                    .vertices
                    .iter()
                    .map(|v| v.pos[0].abs().max(v.pos[1].abs()))
                    .fold(0.0f32, f32::max);
                assert!(
                    max <= 1.0 + 1e-4,
                    "scale {scale} hollow {hollow:?}: extent {max} escaped the unit cap"
                );
                assert!(max < HITBOX, "and therefore stays inside the hit area");
                // And the exported FILE carries the cap too: the game
                // renders what is in the file, not what we meant.
                let back = Mesh::from_obj_str(&to_obj(&m, "cap")).unwrap();
                let fmax = back
                    .vertices
                    .iter()
                    .map(|v| v.pos[0].abs().max(v.pos[1].abs()))
                    .fold(0.0f32, f32::max);
                assert!(fmax <= 1.0 + 1e-4);
            }
        }
    }

    #[test]
    fn broken_outlines_error_instead_of_spinning() {
        // Self-intersecting bowtie.
        let bow = vec![[-1.0, -1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, 1.0]];
        assert!(build_note_mesh(&bow, None, 0.05).is_err());
        // Collinear: no area.
        let line = vec![[-1.0, 0.0], [0.0, 0.0], [1.0, 0.0]];
        assert!(build_note_mesh(&line, None, 0.05).is_err());
        // Too few points, NaN.
        assert!(build_note_mesh(&[[0.0, 0.0], [1.0, 0.0]], None, 0.05).is_err());
        assert!(build_note_mesh(&[[f32::NAN, 0.0], [1.0, 0.0], [0.0, 1.0]], None, 0.05).is_err());
        // Clockwise input is fine (normalised), not an error.
        let mut cw = square();
        cw.reverse();
        assert!(build_note_mesh(&cw, None, 0.05).is_ok());
    }
}
