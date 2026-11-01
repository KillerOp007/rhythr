//! What survives of the v1 note designer: the freeform control point and
//! the two outline helpers the kit still runs on. The kit's Points shape
//! splines its loops with `flatten_controls`, the SVG and glyph imports
//! resolve their paths with `clean_contours`, and the v1 migration reads
//! the old documents' points as `ControlPoint`. The v1 mesh builder and
//! exporter are gone; the kernel bakes every note now.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::i_shape::base::data::Shapes;

/// One control point of a freeform loop. Smooth points get spline
/// tangents (the curve flows through them); corners break the tangent so
/// the outline can kink exactly there.
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

/// Expands a control loop into a dense outline: Catmull-Rom tangents at
/// smooth points, chord tangents at corners (two corner ends make the
/// segment an exact straight line), flattened adaptively so curvature
/// decides the point density instead of a fixed per-segment sample count.
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

/// Resolves raw contours under a fill rule into clean regions and
/// orients them (outer counterclockwise, holes clockwise). The import
/// path cleans every SVG path with ITS OWN fill rule here, so an even-odd
/// donut stays a donut before everything merges under nonzero.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<[f32; 2]> {
        vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
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
        // The same square smooth: the spline still passes through every
        // control point (Catmull-Rom interpolates), but curves between them,
        // so the flattening is dense and bulges past the straight edges.
        let round = flatten_controls(&[
            ctrl(-1.0, -1.0, true),
            ctrl(1.0, -1.0, true),
            ctrl(1.0, 1.0, true),
            ctrl(-1.0, 1.0, true),
        ]);
        assert!(round.len() > sharp.len());
        for corner in square() {
            let d = round
                .iter()
                .map(|p| ((p[0] - corner[0]).powi(2) + (p[1] - corner[1]).powi(2)).sqrt())
                .fold(f32::MAX, f32::min);
            assert!(
                d < 1e-2,
                "the smooth curve left the control point {corner:?}"
            );
        }
        assert!(
            round
                .iter()
                .any(|p| p[0].abs() > 1.001 || p[1].abs() > 1.001),
            "a smooth square never bulges past its edges"
        );
    }

    #[test]
    fn clean_contours_orients_outers_and_holes() {
        // A clockwise square with a duplicate point comes back as one
        // counterclockwise contour.
        let mut cw = square();
        cw.reverse();
        cw.insert(1, cw[1]);
        let out = clean_contours(&[cw], false);
        assert_eq!(out.len(), 1);
        assert!(signed_area(&out[0]) > 0.0);
        // An even-odd donut (both rings wound the same way) keeps its hole,
        // wound clockwise.
        let ring = |r: f32| -> Vec<[f32; 2]> {
            (0..32)
                .map(|i| {
                    let th = i as f32 / 32.0 * std::f32::consts::TAU;
                    [r * th.cos(), r * th.sin()]
                })
                .collect()
        };
        let donut = clean_contours(&[ring(1.0), ring(0.5)], true);
        assert_eq!(donut.len(), 2);
        assert!(signed_area(&donut[0]) > 0.0);
        assert!(signed_area(&donut[1]) < 0.0);
    }
}
