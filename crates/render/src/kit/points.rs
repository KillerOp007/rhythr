//! Freeform conversion: any shape source becomes loops of control points
//! the editor can drag. The dense outline is thinned to the points that
//! carry its form (Ramer-Douglas-Peucker on each closed loop), and a point
//! where the outline turns sharply becomes a corner while every other point
//! is smooth, so a disc converts to a ring of smooth points that spline back
//! to a disc and a star keeps its ten corners. Imports (a letter, an SVG)
//! convert the same way, holes and islands included: every loop of the
//! source is a loop of points.

use crate::kit::doc::Shape;
use crate::kit::geom::{signed_area, Contour};
use crate::kit::shapes::base_contours;
use crate::kit::v1::ControlPoint;
use crate::kit::KitError;

/// Simplification tolerance in note units (half-extent 1): below what the
/// eye sees at the editor's zoom, above the flattening step of imports.
pub const TOLERANCE: f32 = 0.012;
/// A loop with more points than this is thinned harder: a handle per
/// point is only useful while the hand can tell the handles apart.
pub const MAX_POINTS_PER_LOOP: usize = 64;
/// Turning angle in degrees at or above which a point is a corner.
pub const CORNER_DEGREES: f32 = 40.0;
/// Loops smaller than this (note units squared) are flattening debris.
const MIN_LOOP_AREA: f32 = 1e-4;

/// Ramer-Douglas-Peucker over the open polyline `pts[a..=b]`, marking the
/// points to keep.
fn rdp(pts: &[[f32; 2]], a: usize, b: usize, tol: f32, keep: &mut [bool]) {
    if b <= a + 1 {
        return;
    }
    let (p, q) = (pts[a], pts[b]);
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    let len = dx.hypot(dy);
    let mut far = a;
    let mut far_d = -1.0f32;
    for i in a + 1..b {
        let r = pts[i];
        // Distance to the chord; with a degenerate chord the distance to
        // its one point.
        let d = if len > 1e-9 {
            ((r[0] - p[0]) * dy - (r[1] - p[1]) * dx).abs() / len
        } else {
            (r[0] - p[0]).hypot(r[1] - p[1])
        };
        if d > far_d {
            far_d = d;
            far = i;
        }
    }
    if far_d > tol {
        keep[far] = true;
        rdp(pts, a, far, tol, keep);
        rdp(pts, far, b, tol, keep);
    }
}

/// Thins a closed loop: the loop is cut at its first point and at the
/// point farthest from it, so two open runs are simplified and no chord
/// ever spans the whole loop.
fn simplify_loop(c: &[[f32; 2]], tol: f32) -> Vec<[f32; 2]> {
    let n = c.len();
    if n <= 3 {
        return c.to_vec();
    }
    let first = c[0];
    let far = (1..n)
        .max_by(|&i, &j| {
            let di = (c[i][0] - first[0]).hypot(c[i][1] - first[1]);
            let dj = (c[j][0] - first[0]).hypot(c[j][1] - first[1]);
            di.total_cmp(&dj)
        })
        .unwrap_or(n / 2);
    // Index n stands for the closing return to the first point.
    let mut ext: Vec<[f32; 2]> = c.to_vec();
    ext.push(first);
    let mut keep = vec![false; n + 1];
    keep[0] = true;
    keep[far] = true;
    rdp(&ext, 0, far, tol, &mut keep);
    rdp(&ext, far, n, tol, &mut keep);
    let kept: Vec<[f32; 2]> = (0..n).filter(|&i| keep[i]).map(|i| c[i]).collect();
    prune_collinear(kept, tol)
}

/// The two cut points were kept whether or not the outline turns there;
/// a kept point that lies on the chord between its neighbours (the middle
/// of a square's side, say) goes again.
fn prune_collinear(mut pts: Vec<[f32; 2]>, tol: f32) -> Vec<[f32; 2]> {
    let mut changed = true;
    while changed && pts.len() > 3 {
        changed = false;
        let n = pts.len();
        for i in 0..n {
            let (p, q, r) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let (dx, dy) = (r[0] - p[0], r[1] - p[1]);
            let len = dx.hypot(dy);
            let d = if len > 1e-9 {
                ((q[0] - p[0]) * dy - (q[1] - p[1]) * dx).abs() / len
            } else {
                (q[0] - p[0]).hypot(q[1] - p[1])
            };
            if d <= tol {
                pts.remove(i);
                changed = true;
                break;
            }
        }
    }
    pts
}

/// Thins toward the handle budget: the tolerance grows until the loop
/// fits, so a very detailed import still gives a loop a hand can edit.
fn thin(c: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut tol = TOLERANCE;
    let mut out = simplify_loop(c, tol);
    while out.len() > MAX_POINTS_PER_LOOP && tol < 0.5 {
        tol *= 1.5;
        out = simplify_loop(c, tol);
    }
    out
}

fn turning_degrees(prev: [f32; 2], p: [f32; 2], next: [f32; 2]) -> f32 {
    let (ax, ay) = (p[0] - prev[0], p[1] - prev[1]);
    let (bx, by) = (next[0] - p[0], next[1] - p[1]);
    let la = ax.hypot(ay);
    let lb = bx.hypot(by);
    if la < 1e-9 || lb < 1e-9 {
        return 0.0;
    }
    let cos = ((ax * bx + ay * by) / (la * lb)).clamp(-1.0, 1.0);
    cos.acos().to_degrees()
}

fn control_loop(c: &[[f32; 2]]) -> Vec<ControlPoint> {
    let n = c.len();
    (0..n)
        .map(|i| {
            let prev = c[(i + n - 1) % n];
            let next = c[(i + 1) % n];
            ControlPoint {
                x: c[i][0],
                y: c[i][1],
                smooth: turning_degrees(prev, c[i], next) < CORNER_DEGREES,
            }
        })
        .collect()
}

/// The control loops of any shape, in the shape's own unit frame, in the
/// orientation the source had (outer loops counterclockwise, holes
/// clockwise), so the kernel's nonzero clean keeps every hole.
pub fn control_loops(shape: &Shape) -> Vec<Vec<ControlPoint>> {
    base_contours(shape)
        .iter()
        .filter(|c: &&Contour| c.len() >= 3 && signed_area(c).abs() >= MIN_LOOP_AREA)
        .map(|c| thin(c))
        .filter(|c| c.len() >= 3)
        .map(|c| control_loop(&c))
        .collect()
}

/// The shape as freeform points; a shape with nothing to outline (a
/// hidden-by-size import, an empty loop) is refused with a sentence.
pub fn to_points(shape: &Shape) -> Result<Shape, KitError> {
    let mut loops = control_loops(shape);
    if loops.is_empty() {
        return Err(KitError::Doc("this part has no outline to convert".into()));
    }
    let control = loops.remove(0);
    Ok(Shape::Points { control, loops })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::geom::extent;
    use crate::kit::resolve::resolve_part;
    use crate::kit::{Depth, Effects, Material, Mode, Part, Transform};

    fn loops_of(shape: &Shape) -> Vec<Vec<ControlPoint>> {
        match to_points(shape).unwrap() {
            Shape::Points { control, loops } => std::iter::once(control).chain(loops).collect(),
            other => panic!("expected points, got {other:?}"),
        }
    }

    fn part(shape: Shape) -> Part {
        Part {
            id: "p".into(),
            name: "p".into(),
            visible: true,
            locked: false,
            shape,
            transform: Transform::default(),
            mode: Mode::Add,
            depth: Depth::default(),
            material: Material::Solid,
            effects: Effects::default(),
        }
    }

    #[test]
    fn a_disc_becomes_a_ring_of_smooth_points_that_splines_back_to_a_disc() {
        let loops = loops_of(&Shape::Circle);
        assert_eq!(loops.len(), 1);
        let n = loops[0].len();
        assert!((12..=32).contains(&n), "{n} points");
        assert!(loops[0].iter().all(|p| p.smooth));
        // Resolved through the spline, every point sits near radius 1.
        let r = resolve_part(&part(to_points(&Shape::Circle).unwrap())).unwrap();
        let radii: Vec<f32> = r.shapes[0][0].iter().map(|v| v[0].hypot(v[1])).collect();
        let (min, max) = (
            radii.iter().cloned().fold(f32::MAX, f32::min),
            radii.iter().cloned().fold(f32::MIN, f32::max),
        );
        assert!(min > 0.97 && max < 1.03, "radii {min}..{max}");
    }

    #[test]
    fn a_star_keeps_its_ten_corners() {
        let loops = loops_of(&Shape::Star {
            points: 5,
            inner: 0.45,
        });
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 10);
        assert!(loops[0].iter().all(|p| !p.smooth));
        assert!(loops[0]
            .iter()
            .any(|p| (p.x - 1.0).abs() < 1e-4 && p.y.abs() < 1e-4));
    }

    #[test]
    fn a_square_has_four_corners_and_a_ring_two_loops() {
        let sq = loops_of(&Shape::Square { rounding: 0.0 });
        assert_eq!(sq[0].len(), 4);
        assert!(sq[0].iter().all(|p| !p.smooth));
        let ring = loops_of(&Shape::Ring { thickness: 0.3 });
        assert_eq!(ring.len(), 2);
        let area = |l: &Vec<ControlPoint>| {
            let c: Vec<[f32; 2]> = l.iter().map(|p| [p.x, p.y]).collect();
            signed_area(&c)
        };
        assert!(area(&ring[0]) > 0.0, "outer counterclockwise");
        assert!(area(&ring[1]) < 0.0, "hole clockwise");
        // And the ring survives the conversion as a ring.
        let r = resolve_part(&part(to_points(&Shape::Ring { thickness: 0.3 }).unwrap())).unwrap();
        assert_eq!(r.shapes.len(), 1);
        assert_eq!(r.shapes[0].len(), 2, "outer plus hole");
        assert!((extent(&r.shapes) - 1.0).abs() < 0.03);
    }

    #[test]
    fn an_import_with_a_hole_converts_loop_for_loop_and_debris_is_dropped() {
        let dense_square: Vec<[f32; 2]> = (0..400)
            .map(|i| {
                // A square walked densely, a point every 0.01.
                let t = (i as f32 / 100.0) % 4.0;
                let side = t.floor() as u32;
                let f = t - t.floor();
                match side {
                    0 => [-1.0 + 2.0 * f, -1.0],
                    1 => [1.0, -1.0 + 2.0 * f],
                    2 => [1.0 - 2.0 * f, 1.0],
                    _ => [-1.0, 1.0 - 2.0 * f],
                }
            })
            .collect();
        let mut hole: Vec<[f32; 2]> = vec![[-0.5, -0.5], [-0.5, 0.5], [0.5, 0.5], [0.5, -0.5]];
        hole.dedup();
        let debris = vec![[0.0, 0.0], [0.001, 0.0], [0.0, 0.001]];
        let loops = loops_of(&Shape::Contours {
            contours: vec![dense_square, hole, debris],
            label: "x".into(),
        });
        assert_eq!(loops.len(), 2, "outer and hole, no debris");
        assert_eq!(loops[0].len(), 4, "the dense square thins to its corners");
        assert_eq!(loops[1].len(), 4);
    }

    #[test]
    fn a_very_detailed_loop_is_thinned_to_the_handle_budget() {
        let wobbly: Vec<[f32; 2]> = (0..4000)
            .map(|i| {
                let t = i as f32 / 4000.0 * std::f32::consts::TAU;
                let r = 1.0 + 0.05 * (t * 40.0).sin();
                [r * t.cos(), r * t.sin()]
            })
            .collect();
        let loops = loops_of(&Shape::Contours {
            contours: vec![wobbly],
            label: String::new(),
        });
        assert!(
            loops[0].len() <= MAX_POINTS_PER_LOOP,
            "{} points",
            loops[0].len()
        );
        assert!(loops[0].len() >= 3);
    }

    #[test]
    fn nothing_to_outline_is_a_sentence() {
        let e = to_points(&Shape::Points {
            control: vec![],
            loops: vec![],
        })
        .unwrap_err();
        assert_eq!(
            e,
            KitError::Doc("this part has no outline to convert".into())
        );
    }

    #[test]
    fn points_with_further_loops_resolve_with_their_holes() {
        let ctrl = |x: f32, y: f32| ControlPoint {
            x,
            y,
            smooth: false,
        };
        let shape = Shape::Points {
            control: vec![
                ctrl(-1.0, -1.0),
                ctrl(1.0, -1.0),
                ctrl(1.0, 1.0),
                ctrl(-1.0, 1.0),
            ],
            loops: vec![
                // A clockwise hole and a second counterclockwise island
                // beside the square (outside it, so it stays material).
                vec![
                    ctrl(-0.5, -0.5),
                    ctrl(-0.5, 0.5),
                    ctrl(0.5, 0.5),
                    ctrl(0.5, -0.5),
                ],
            ],
        };
        let r = resolve_part(&part(shape)).unwrap();
        assert_eq!(r.shapes.len(), 1);
        assert_eq!(r.shapes[0].len(), 2, "the square keeps its hole");
    }
}
