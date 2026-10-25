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
//! or in the game (which renders meshes at their authored scale).
//!
//! The heavy lifting is delegated to proven kernels instead of bespoke
//! maths (which produced three rounds of rim artifacts before being
//! retired): i_overlay resolves self-overlap with the nonzero fill rule
//! and computes the hollow rim as a properly trimmed outline band,
//! i_triangle triangulates the resulting shapes-with-holes, and kurbo
//! flattens the editor's spline control points adaptively. A correct
//! offset changes the vertex count, so the mesh no longer assumes any
//! outer/inner correspondence: every contour extrudes its own wall and
//! the caps come straight from the triangulation.

use crate::mesh::{Mesh, Vertex};
use crate::Error;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::i_shape::base::data::Shapes;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};
use i_triangle::float::triangulatable::Triangulatable;

/// One control point of the editor's freeform loop. Smooth points get
/// spline tangents (the curve flows through them); corners break the
/// tangent so the outline can kink exactly there.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ControlPoint {
    pub x: f32,
    pub y: f32,
    #[serde(default = "default_smooth")]
    pub smooth: bool,
}

fn default_smooth() -> bool {
    true
}

/// One resolved region of the note: an outer boundary and the holes cut
/// into it. Contours arrive cleaned and oriented (outer counterclockwise,
/// holes clockwise), ready for walls and caps alike.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanShape {
    pub outer: Vec<[f32; 2]>,
    pub holes: Vec<Vec<[f32; 2]>>,
}

/// The 2D truth of a note before extrusion. The editor canvas draws
/// EXACTLY this (it asks the backend rather than porting the maths), and
/// the mesh extrudes exactly this, so the note being edited and the note
/// being rendered cannot drift apart.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NotePlan {
    pub shapes: Vec<PlanShape>,
    /// The factor the normalisation applied to the input (1.0 when the
    /// outline already sat at unit scale). The editor maps its control
    /// points through this so the handles stay ON the contour it draws.
    pub scale: f32,
}

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

/// Expands the editor's control loop into a dense outline: Catmull-Rom
/// tangents at smooth points, chord tangents at corners (two corner ends
/// make the segment an exact straight line), flattened adaptively so
/// curvature decides the point density instead of a fixed per-segment
/// sample count.
pub fn flatten_controls(points: &[ControlPoint]) -> Vec<[f32; 2]> {
    let n = points.len();
    if n < 3 {
        return points.iter().map(|p| [p.x, p.y]).collect();
    }
    let at = |i: usize| kurbo::Point::new(points[i % n].x as f64, points[i % n].y as f64);
    let mut extent = 0.0f64;
    for p in points {
        extent = extent.max(p.x.abs() as f64).max(p.y.abs() as f64);
    }
    // Tolerance follows the drawing's own scale, so a note edited at any
    // zoom flattens to the same relative smoothness.
    let tol = extent.max(0.05) * 0.0015;
    let mut path = kurbo::BezPath::new();
    path.move_to(at(0));
    for i in 0..n {
        let (p0, p1, p2, p3) = (at(i + n - 1), at(i), at(i + 1), at(i + 2));
        let c1 = if points[i % n].smooth {
            p1 + (p2 - p0) * (1.0 / 6.0)
        } else {
            p1 + (p2 - p1) * (1.0 / 3.0)
        };
        let c2 = if points[(i + 1) % n].smooth {
            p2 - (p3 - p1) * (1.0 / 6.0)
        } else {
            p2 - (p2 - p1) * (1.0 / 3.0)
        };
        path.curve_to(c1, c2, p2);
    }
    path.close_path();
    let mut out: Vec<[f32; 2]> = Vec::new();
    kurbo::flatten(path, tol, |el| match el {
        kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => out.push([p.x as f32, p.y as f32]),
        _ => {}
    });
    while out.len() > 3 && {
        let (f, l) = (out[0], out[out.len() - 1]);
        (f[0] - l[0]).abs() < 1e-6 && (f[1] - l[1]).abs() < 1e-6
    } {
        out.pop();
    }
    out
}

/// Cleans and normalises an outline and computes the rim: the single
/// source both the canvas and the mesh build from.
///
/// `hollow`: None fills the silhouette; Some(t) keeps a rim of thickness
/// `t` (in unit-square terms) and opens the middle. Self-overlapping
/// outlines are legal input: the nonzero fill rule resolves them into
/// clean regions the way the editor canvas already paints them.
pub fn plan_note(outline: &[[f32; 2]], hollow: Option<f32>) -> Result<NotePlan, Error> {
    plan_note_multi(std::slice::from_ref(&outline.to_vec()), hollow)
}

/// [`plan_note`] over several contours at once, the import path: an SVG
/// or a glyph arrives as many contours whose ORIENTATION already encodes
/// holes (clockwise inside counterclockwise counts back to zero under
/// the nonzero rule), so letters keep their counters.
pub fn plan_note_multi(contours: &[Vec<[f32; 2]>], hollow: Option<f32>) -> Result<NotePlan, Error> {
    let input: Vec<Vec<[f32; 2]>> = contours.iter().filter(|c| c.len() >= 3).cloned().collect();
    if input.is_empty()
        || input
            .iter()
            .flatten()
            .any(|p| !p[0].is_finite() || !p[1].is_finite())
    {
        return Err(Error::BadObj);
    }
    let mut shapes: Shapes<[f32; 2]> = input.simplify_shape(FillRule::NonZero);
    shapes.retain(|s| !s.is_empty() && s[0].len() >= 3);
    if shapes.is_empty() {
        return Err(Error::BadObj);
    }
    // Normalise the SILHOUETTE to the unit square (the hard size rule).
    // Only outer boundaries can define the extent; holes live inside.
    let mut m = 0.0f32;
    for s in &shapes {
        for p in &s[0] {
            m = m.max(p[0].abs()).max(p[1].abs());
        }
    }
    if m < 1e-6 {
        return Err(Error::BadObj);
    }
    let mut scale = 1.0f32;
    if (m - 1.0).abs() > 1e-3 {
        scale = 1.0 / m;
        for s in &mut shapes {
            for c in s.iter_mut() {
                for p in c.iter_mut() {
                    p[0] *= scale;
                    p[1] *= scale;
                }
            }
        }
    }
    if let Some(t) = hollow {
        let t = t.clamp(0.01, 0.9);
        // The rim is the region minus its erosion by t: a negative offset
        // moves EVERY boundary (outer and holes alike) inward into the
        // material, and the round join trims concave corners the way a
        // true offset curve does. Where the shape is thinner than two
        // rims the erosion vanishes and the rim stays solid, which is
        // the correct limit case.
        let style = OutlineStyle::new(-t).line_join(LineJoin::Round(0.2));
        let mut eroded: Shapes<[f32; 2]> = shapes.outline(&style);
        eroded.retain(|s| !s.is_empty() && s[0].len() >= 3);
        if !eroded.is_empty() {
            let mut rim: Shapes<[f32; 2]> =
                shapes.overlay(&eroded, OverlayRule::Difference, FillRule::NonZero);
            rim.retain(|s| !s.is_empty() && s[0].len() >= 3);
            if rim.is_empty() {
                return Err(Error::BadObj);
            }
            shapes = rim;
        }
    }
    // The wall builder encodes facing in the traversal direction, so the
    // orientation contract is enforced here rather than assumed from the
    // kernel's defaults: outer counterclockwise, holes clockwise.
    let mut plan_shapes = Vec::with_capacity(shapes.len());
    for s in &mut shapes {
        if signed_area(&s[0]) < 0.0 {
            s[0].reverse();
        }
        let outer = s[0].clone();
        let mut holes = Vec::with_capacity(s.len().saturating_sub(1));
        for h in s.iter_mut().skip(1) {
            if h.len() < 3 {
                continue;
            }
            if signed_area(h) > 0.0 {
                h.reverse();
            }
            holes.push(h.clone());
        }
        plan_shapes.push(PlanShape { outer, holes });
    }
    Ok(NotePlan {
        shapes: plan_shapes,
        scale,
    })
}

/// Builds the note mesh from a finished plan. `depth` is the half
/// extrusion in outline units.
pub fn mesh_from_plan(plan: &NotePlan, depth: f32) -> Result<Mesh, Error> {
    let d = depth.clamp(0.005, 0.5);
    let mut mesh = Mesh::default();
    for shape in &plan.shapes {
        let mut contours: Vec<Vec<[f32; 2]>> = Vec::with_capacity(1 + shape.holes.len());
        contours.push(shape.outer.clone());
        contours.extend(shape.holes.iter().cloned());
        // Front and back caps from the triangulation (holes respected).
        let tri = contours.triangulate().to_triangulation::<u32>();
        if tri.indices.is_empty() {
            continue;
        }
        let base = mesh.vertices.len() as u32;
        let np = tri.points.len() as u32;
        for p in &tri.points {
            mesh.vertices.push(Vertex {
                pos: [p[0], p[1], d],
                alpha: 1.0,
            });
        }
        for p in &tri.points {
            mesh.vertices.push(Vertex {
                pos: [p[0], p[1], -d],
                alpha: 1.0,
            });
        }
        for t in tri.indices.chunks_exact(3) {
            // Front face (z = +d) keeps the CCW order; back face reverses.
            mesh.indices
                .extend_from_slice(&[base + t[0], base + t[1], base + t[2]]);
            mesh.indices
                .extend_from_slice(&[base + np + t[0], base + np + t[2], base + np + t[1]]);
        }
        // One wall strip per contour. The quad pattern is written for a
        // counterclockwise outer boundary; a clockwise hole runs the same
        // code backwards along the loop, which flips the facing exactly
        // as an inner wall needs.
        for contour in &contours {
            let cb = mesh.vertices.len() as u32;
            let m = contour.len() as u32;
            if m < 3 {
                continue;
            }
            for p in contour {
                mesh.vertices.push(Vertex {
                    pos: [p[0], p[1], d],
                    alpha: 1.0,
                });
            }
            for p in contour {
                mesh.vertices.push(Vertex {
                    pos: [p[0], p[1], -d],
                    alpha: 1.0,
                });
            }
            for i in 0..m {
                let j = (i + 1) % m;
                let (fa, fb) = (cb + i, cb + j);
                let (ba, bb) = (cb + m + i, cb + m + j);
                mesh.indices.extend_from_slice(&[fa, ba, bb]);
                mesh.indices.extend_from_slice(&[fa, bb, fb]);
            }
        }
    }
    if mesh.indices.is_empty() {
        return Err(Error::BadObj);
    }
    mesh.normalize_to_unit();
    Ok(mesh)
}

/// Builds the note mesh from a closed outline (see [`plan_note`] for the
/// input contract).
pub fn build_note_mesh(
    outline: &[[f32; 2]],
    hollow: Option<f32>,
    depth: f32,
) -> Result<Mesh, Error> {
    mesh_from_plan(&plan_note(outline, hollow)?, depth)
}

/// [`build_note_mesh`] over several contours (the import path).
pub fn build_note_mesh_multi(
    contours: &[Vec<[f32; 2]>],
    hollow: Option<f32>,
    depth: f32,
) -> Result<Mesh, Error> {
    mesh_from_plan(&plan_note_multi(contours, hollow)?, depth)
}

/// Resolves raw contours under a fill rule into oriented plain contours
/// (outer counterclockwise, holes clockwise). The import path cleans
/// every SVG path with ITS OWN fill rule here, so an even-odd donut
/// stays a donut before everything merges under nonzero.
pub fn clean_contours(contours: &[Vec<[f32; 2]>], even_odd: bool) -> Vec<Vec<[f32; 2]>> {
    let rule = if even_odd {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    };
    let input: Vec<Vec<[f32; 2]>> = contours.iter().filter(|c| c.len() >= 3).cloned().collect();
    let mut shapes: Shapes<[f32; 2]> = input.simplify_shape(rule);
    let mut out = Vec::new();
    for s in &mut shapes {
        for (i, c) in s.iter_mut().enumerate() {
            if c.len() < 3 {
                continue;
            }
            let ccw = signed_area(c) > 0.0;
            if (i == 0) != ccw {
                c.reverse();
            }
            out.push(c.clone());
        }
    }
    out
}

/// The exported material. Always written alongside the .obj, with the
/// name the game's loader expects.
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
    export_rhs_with_design(mesh, name, colors_hex, None)
}

/// [`export_rhs`] with the designer's own document embedded under the
/// kit's design entry, so the one file opens again in the Note Designer
/// and converts in the Note Kit. `None` writes a plain skin.
pub fn export_rhs_with_design(
    mesh: &Mesh,
    name: &str,
    colors_hex: &[String],
    design_json: Option<&str>,
) -> Result<Vec<u8>, Error> {
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
    if let Some(doc) = design_json {
        write(&mut zw, crate::kit::export::DESIGN_ENTRY, doc.as_bytes())?;
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

    fn wavy(n: usize, waves: f32, amp: f32) -> Vec<[f32; 2]> {
        (0..n)
            .map(|i| {
                let th = i as f32 / n as f32 * std::f32::consts::TAU;
                let r = 1.0 + amp * (waves * th).sin();
                [r * th.cos(), r * th.sin()]
            })
            .collect()
    }

    /// Distance from a point to the closest segment of a contour.
    fn dist_to_contour(p: [f32; 2], c: &[[f32; 2]]) -> f32 {
        let mut best = f32::MAX;
        for i in 0..c.len() {
            let a = c[i];
            let b = c[(i + 1) % c.len()];
            let ab = [b[0] - a[0], b[1] - a[1]];
            let l2 = (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-12);
            let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0);
            let q = [a[0] + t * ab[0], a[1] + t * ab[1]];
            best = best.min(((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt());
        }
        best
    }

    fn crossings(pts: &[[f32; 2]]) -> usize {
        let n = pts.len();
        let mut count = 0;
        for i in 0..n {
            for j in i + 2..n {
                if i == 0 && j == n - 1 {
                    continue;
                }
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                let (c, d) = (pts[j], pts[(j + 1) % n]);
                let r = [b[0] - a[0], b[1] - a[1]];
                let s = [d[0] - c[0], d[1] - c[1]];
                let den = r[0] * s[1] - r[1] * s[0];
                if den.abs() < 1e-12 {
                    continue;
                }
                let t = ((c[0] - a[0]) * s[1] - (c[1] - a[1]) * s[0]) / den;
                let u = ((c[0] - a[0]) * r[1] - (c[1] - a[1]) * r[0]) / den;
                if (0.001..0.999).contains(&t) && (0.001..0.999).contains(&u) {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn filled_square_is_a_closed_slab() {
        let mesh = build_note_mesh(&square(), None, 0.05).unwrap();
        assert!(mesh.indices.len() % 3 == 0);
        assert!(!mesh.vertices.is_empty());
        for v in &mesh.vertices {
            assert!(v.pos[0].abs() <= 1.001 && v.pos[1].abs() <= 1.001);
        }
    }

    #[test]
    fn hollow_ring_has_a_real_hole() {
        let plan = plan_note(&wavy(160, 0.0, 0.0), Some(0.25)).unwrap();
        assert_eq!(plan.shapes.len(), 1);
        assert_eq!(plan.shapes[0].holes.len(), 1);
        // Orientation contract: outer CCW, hole CW.
        assert!(signed_area(&plan.shapes[0].outer) > 0.0);
        assert!(signed_area(&plan.shapes[0].holes[0]) < 0.0);
        let mesh = mesh_from_plan(&plan, 0.05).unwrap();
        assert!(!mesh.indices.is_empty());
    }

    #[test]
    fn a_wavy_circle_triangulates_despite_being_concave() {
        let mesh = build_note_mesh(&wavy(160, 8.0, 0.25), None, 0.05).unwrap();
        assert!(mesh.indices.len() >= 3 * 158);
    }

    #[test]
    fn rim_thickness_is_exact_even_in_tight_troughs() {
        // The shape class that defeated three generations of hand-rolled
        // insetting: tight concave troughs where a naive offset folds.
        let plan = plan_note(&wavy(160, 8.0, 0.25), Some(0.22)).unwrap();
        let outer = &plan.shapes[0].outer;
        assert!(!plan.shapes[0].holes.is_empty());
        for hole in &plan.shapes[0].holes {
            assert_eq!(crossings(hole), 0, "hole contour crosses itself");
            for p in hole {
                let d = dist_to_contour(*p, outer);
                assert!(
                    d > 0.20 && d < 0.26,
                    "rim thickness {d} escaped the 0.22 band"
                );
            }
        }
    }

    #[test]
    fn self_overlapping_outlines_resolve_instead_of_erroring() {
        // A pinwheel whose arms fold over each other: nonzero union turns
        // it into clean regions, hollow or solid.
        let pin: Vec<[f32; 2]> = (0..200)
            .map(|i| {
                let th = i as f32 / 200.0 * std::f32::consts::TAU;
                let r = 1.0 + 0.55 * (4.0 * th + 0.9).sin();
                [r * th.cos(), r * th.sin()]
            })
            .collect();
        let solid = plan_note(&pin, None).unwrap();
        assert!(!solid.shapes.is_empty());
        let hollow = plan_note(&pin, Some(0.22)).unwrap();
        for s in &hollow.shapes {
            assert_eq!(crossings(&s.outer), 0);
            for h in &s.holes {
                assert_eq!(crossings(h), 0);
            }
        }
        assert!(mesh_from_plan(&hollow, 0.05).is_ok());
    }

    #[test]
    fn obj_roundtrip_through_the_skin_loader() {
        let mesh = build_note_mesh(&wavy(64, 5.0, 0.2), Some(0.3), 0.05).unwrap();
        let obj = to_obj(&mesh, "test");
        let parsed = crate::mesh::Mesh::from_obj_str(&obj).unwrap();
        assert_eq!(parsed.vertices.len(), mesh.vertices.len());
        assert_eq!(parsed.indices.len(), mesh.indices.len());
    }

    #[test]
    fn exported_rhs_loads_back_as_a_skin() {
        let mesh = build_note_mesh(&square(), None, 0.05).unwrap();
        let colors = vec!["#ff0000".to_string(), "not-a-color".to_string()];
        let zip_bytes = export_rhs(&mesh, "designed", &colors).unwrap();
        let cfg = crate::config::SkinConfig::from_rhs(&zip_bytes).unwrap();
        assert!(cfg.note_mesh.is_some());
        assert_eq!(cfg.colorset.len(), 1);
        // A plain export carries no design; one with a document gives it back.
        assert_eq!(
            crate::kit::export::embedded_design(&zip_bytes).unwrap(),
            None
        );
        let doc = r##"{"P":{"shape":"circle"},"free":null,"colors":["#ff0000"],"imported":null}"##;
        let with_doc = export_rhs_with_design(&mesh, "designed", &colors, Some(doc)).unwrap();
        assert_eq!(
            crate::kit::export::embedded_design(&with_doc)
                .unwrap()
                .as_deref(),
            Some(doc)
        );
        assert!(crate::config::SkinConfig::from_rhs(&with_doc)
            .unwrap()
            .note_mesh
            .is_some());
    }

    #[test]
    fn dense_valid_outlines_build_instead_of_failing() {
        // No decimation cap anymore: thousands of points are fine.
        let mesh = build_note_mesh(&wavy(4000, 12.0, 0.3), None, 0.05).unwrap();
        assert!(!mesh.indices.is_empty());
    }

    #[test]
    fn duplicate_points_are_cleaned_not_fatal() {
        let mut pts = square();
        pts.insert(1, [-1.0, -1.0]);
        pts.push([-1.0, -1.0]);
        assert!(build_note_mesh(&pts, None, 0.05).is_ok());
    }

    #[test]
    fn spiky_hollow_keeps_the_outer_silhouette_at_unit_scale() {
        let star: Vec<[f32; 2]> = (0..24)
            .map(|i| {
                let th = i as f32 / 24.0 * std::f32::consts::TAU;
                let r = if i % 2 == 0 { 1.0 } else { 0.25 };
                [r * th.cos(), r * th.sin()]
            })
            .collect();
        let plan = plan_note(&star, Some(0.5)).unwrap();
        let m = plan
            .shapes
            .iter()
            .flat_map(|s| s.outer.iter())
            .map(|p| p[0].abs().max(p[1].abs()))
            .fold(0.0f32, f32::max);
        assert!((0.95..=1.001).contains(&m), "outer extent {m} off unit");
    }

    #[test]
    fn no_export_can_outgrow_the_hitbox() {
        // The hard size rule, attacked from several directions: oversized
        // input, spiky input, hollow with huge rims.
        let big: Vec<[f32; 2]> = square().iter().map(|p| [p[0] * 9.0, p[1] * 9.0]).collect();
        for (outline, hollow) in [
            (big, Some(0.9)),
            (wavy(160, 9.0, 0.6), Some(0.7)),
            (wavy(24, 3.0, 0.9), None),
        ] {
            let mesh = build_note_mesh(&outline, hollow, 0.05).unwrap();
            for v in &mesh.vertices {
                assert!(
                    v.pos[0].abs() <= 1.001 && v.pos[1].abs() <= 1.001,
                    "vertex escaped the unit square"
                );
            }
        }
    }

    #[test]
    fn broken_outlines_error_instead_of_spinning() {
        assert!(build_note_mesh(&[], None, 0.05).is_err());
        assert!(build_note_mesh(&[[0.0, 0.0], [1.0, 1.0]], None, 0.05).is_err());
        // Collinear points enclose no area.
        let line: Vec<[f32; 2]> = (0..10).map(|i| [i as f32 * 0.1, 0.0]).collect();
        assert!(build_note_mesh(&line, None, 0.05).is_err());
        let nan = vec![[0.0, 0.0], [f32::NAN, 1.0], [1.0, 0.0]];
        assert!(build_note_mesh(&nan, None, 0.05).is_err());
    }

    #[test]
    fn smooth_controls_curve_and_corner_controls_stay_sharp() {
        let ctrl = |x: f32, y: f32, smooth: bool| ControlPoint { x, y, smooth };
        // All-corner square: flattening yields straight edges through the
        // exact corner positions.
        let sharp = flatten_controls(&[
            ctrl(-1.0, -1.0, false),
            ctrl(1.0, -1.0, false),
            ctrl(1.0, 1.0, false),
            ctrl(-1.0, 1.0, false),
        ]);
        for corner in square() {
            let d = sharp
                .iter()
                .map(|p| ((p[0] - corner[0]).powi(2) + (p[1] - corner[1]).powi(2)).sqrt())
                .fold(f32::MAX, f32::min);
            assert!(d < 1e-3, "corner {corner:?} missing from flattening");
        }
        // The same square smooth: the curve rounds the corners, so the
        // flattening is dense and stays strictly inside the mitred box.
        let round = flatten_controls(&[
            ctrl(-1.0, -1.0, true),
            ctrl(1.0, -1.0, true),
            ctrl(1.0, 1.0, true),
            ctrl(-1.0, 1.0, true),
        ]);
        assert!(round.len() > sharp.len());
        assert!(plan_note(&round, None).is_ok());
    }
}
