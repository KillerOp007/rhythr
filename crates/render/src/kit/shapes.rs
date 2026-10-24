//! Shape sources at unit size. Generated outlines are dense (so waves
//! and rounding have points to work with) and counterclockwise; imports
//! pass through as stored.

use crate::designer::flatten_controls;
use crate::kit::doc::Shape;
use crate::kit::geom::Contour;
use std::f32::consts::{PI, TAU};

const SAMPLES: usize = 256;

fn polar(samples: usize, radius: impl Fn(f32) -> f32) -> Contour {
    (0..samples)
        .map(|i| {
            let th = i as f32 / samples as f32 * TAU;
            let r = radius(th);
            [r * th.cos(), r * th.sin()]
        })
        .collect()
}

/// Regular n-gon radius at angle `th`, blended toward the circle by
/// `rounding` in 0..=1. The square is the 4-gon turned so a side sits
/// flat at the bottom.
fn ngon_radius(sides: u32, rounding: f32, th: f32, offset: f32) -> f32 {
    let n = sides.max(3) as f32;
    let sector = TAU / n;
    let a = ((th + offset).rem_euclid(sector) - sector * 0.5).abs();
    let ngon = (PI / n).cos() / a.cos();
    ngon + (1.0 - ngon) * rounding.clamp(0.0, 1.0)
}

/// Scale a contour about the origin to half-extent 1. The turned 4-gon
/// has its corners on the unit circle, which leaves its flat sides at
/// 0.707; every other source reaches 1 on its own.
fn fit_half_extent(mut c: Contour) -> Contour {
    let m = c.iter().flatten().fold(0.0f32, |m, v| m.max(v.abs()));
    for p in &mut c {
        p[0] /= m;
        p[1] /= m;
    }
    c
}

pub fn base_contours(shape: &Shape) -> Vec<Contour> {
    match shape {
        Shape::Circle => vec![polar(SAMPLES, |_| 1.0)],
        Shape::Square { rounding } => {
            vec![fit_half_extent(polar(SAMPLES, |th| {
                ngon_radius(4, *rounding, th, PI / 4.0)
            }))]
        }
        Shape::Polygon { sides, rounding } => {
            // Vertex at angle zero, as the v1 editor drew its polygons;
            // only the square is turned so a side sits flat.
            vec![polar(SAMPLES, |th| ngon_radius(*sides, *rounding, th, 0.0))]
        }
        Shape::Star { points, inner } => {
            let n = (*points).max(2) as f32;
            let inner = inner.clamp(0.05, 0.95);
            // Linear blend between tip and valley radius: straight edges.
            vec![polar(SAMPLES, move |th| {
                let sector = TAU / n;
                let t = (th.rem_euclid(sector) / sector * 2.0 - 1.0).abs();
                inner + (1.0 - inner) * t
            })]
        }
        Shape::Ring { thickness } => {
            let t = thickness.clamp(0.02, 0.98);
            let mut hole = polar(SAMPLES, move |_| 1.0 - t);
            hole.reverse();
            vec![polar(SAMPLES, |_| 1.0), hole]
        }
        Shape::Pill { aspect } => {
            // A stadium: half-extent 1 along x, `aspect` along y.
            let a = aspect.clamp(0.1, 1.0);
            let r = a;
            let cx = 1.0 - r;
            let half = SAMPLES / 2;
            let mut c = Vec::with_capacity(SAMPLES);
            for i in 0..half {
                let th = -PI / 2.0 + PI * i as f32 / half as f32;
                c.push([cx + r * th.cos(), r * th.sin()]);
            }
            for i in 0..half {
                let th = PI / 2.0 + PI * i as f32 / half as f32;
                c.push([-cx + r * th.cos(), r * th.sin()]);
            }
            vec![c]
        }
        Shape::Contours { contours, .. } => contours.clone(),
        Shape::Points { control } => {
            if control.len() < 3 {
                return Vec::new();
            }
            vec![flatten_controls(control)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::geom::signed_area;

    fn extent(c: &[[f32; 2]]) -> f32 {
        c.iter()
            .map(|p| p[0].abs().max(p[1].abs()))
            .fold(0.0, f32::max)
    }

    #[test]
    fn generated_shapes_are_dense_unit_and_ccw() {
        for shape in [
            Shape::Circle,
            Shape::Square { rounding: 0.0 },
            Shape::Polygon {
                sides: 6,
                rounding: 0.3,
            },
            Shape::Star {
                points: 5,
                inner: 0.45,
            },
            Shape::Pill { aspect: 0.5 },
        ] {
            let cs = base_contours(&shape);
            assert_eq!(cs.len(), 1, "{shape:?}");
            assert!(cs[0].len() >= 64, "{shape:?} has {} points", cs[0].len());
            assert!((extent(&cs[0]) - 1.0).abs() < 1e-3, "{shape:?}");
            assert!(signed_area(&cs[0]) > 0.0, "{shape:?}");
        }
    }

    #[test]
    fn ring_is_outer_plus_cw_hole() {
        let cs = base_contours(&Shape::Ring { thickness: 0.3 });
        assert_eq!(cs.len(), 2);
        assert!(signed_area(&cs[0]) > 0.0);
        assert!(signed_area(&cs[1]) < 0.0);
        assert!((extent(&cs[1]) - 0.7).abs() < 1e-3);
    }

    #[test]
    fn polygon_rounding_moves_between_ngon_and_circle() {
        let sharp = base_contours(&Shape::Polygon {
            sides: 4,
            rounding: 0.0,
        });
        let soft = base_contours(&Shape::Polygon {
            sides: 4,
            rounding: 1.0,
        });
        // At 45 degrees a diamond's radius is 1/sqrt(2); a circle's is 1.
        let r = |c: &[[f32; 2]]| {
            c.iter()
                .map(|p| {
                    (
                        p[0].hypot(p[1]),
                        (p[1].atan2(p[0]) - std::f32::consts::FRAC_PI_4).abs(),
                    )
                })
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap()
                .0
        };
        assert!((r(&sharp[0]) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.03);
        assert!((r(&soft[0]) - 1.0).abs() < 0.03);
    }

    #[test]
    fn sharp_square_is_the_unit_box_with_a_flat_bottom() {
        let c = &base_contours(&Shape::Square { rounding: 0.0 })[0];
        assert!(c
            .iter()
            .all(|p| p[0].abs() < 1.0 + 1e-3 && p[1].abs() < 1.0 + 1e-3));
        assert!(c
            .iter()
            .any(|p| (p[0] - 1.0).abs() < 1e-3 && (p[1] - 1.0).abs() < 1e-3));
        let bottom = c.iter().filter(|p| (p[1] + 1.0).abs() < 1e-3).count();
        assert!(bottom >= 60, "only {bottom} points on the bottom side");
    }

    #[test]
    fn square_rounding_blends_toward_the_unit_circle() {
        let radii = |rounding: f32| -> Vec<f32> {
            base_contours(&Shape::Square { rounding })[0]
                .iter()
                .map(|p| p[0].hypot(p[1]))
                .collect()
        };
        let circle = radii(1.0);
        assert!(circle.iter().all(|r| (r - 1.0).abs() < 1e-3));
        let max = |v: &[f32]| v.iter().cloned().fold(0.0, f32::max);
        assert!(max(&radii(0.5)) < max(&radii(0.0)));
        let soft = base_contours(&Shape::Square { rounding: 0.5 });
        assert!((extent(&soft[0]) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn polygon_has_a_vertex_at_angle_zero() {
        let c = &base_contours(&Shape::Polygon {
            sides: 6,
            rounding: 0.0,
        })[0];
        assert!((c[0][0] - 1.0).abs() < 1e-4 && c[0][1].abs() < 1e-4);
    }

    #[test]
    fn points_flatten_through_the_spline_and_contours_pass_through() {
        let ctrl = |x: f32, y: f32| crate::designer::ControlPoint { x, y, smooth: true };
        let pts = base_contours(&Shape::Points {
            control: vec![
                ctrl(-1.0, -1.0),
                ctrl(1.0, -1.0),
                ctrl(1.0, 1.0),
                ctrl(-1.0, 1.0),
            ],
        });
        assert!(pts[0].len() > 4);
        let raw = vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]];
        let cs = base_contours(&Shape::Contours {
            contours: raw.clone(),
            label: String::new(),
        });
        assert_eq!(cs, raw);
    }
}
