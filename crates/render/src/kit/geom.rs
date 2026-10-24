//! Fixed-scale wrappers around i_overlay. Every operation snaps to the
//! same integer grid, so a region derived twice from the same inputs is
//! bit-identical, and contours shared between neighbouring operations
//! line up vertex for vertex.

use crate::kit::{KitError, KIT_SCALE};
pub use i_overlay::core::fill_rule::FillRule;
pub use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::scale::FixedScaleFloatOverlay;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};

pub type Contour = Vec<[f32; 2]>;
pub type Shapes = Vec<Vec<Contour>>;

fn scale_err(e: i_overlay::float::scale::FixedScaleOverlayError) -> KitError {
    KitError::Scale(format!("{e:?}"))
}

/// Signed area, positive for counterclockwise.
pub fn signed_area(c: &[[f32; 2]]) -> f32 {
    let mut a = 0.0;
    for i in 0..c.len() {
        let p = c[i];
        let q = c[(i + 1) % c.len()];
        a += p[0] * q[1] - q[0] * p[1];
    }
    a * 0.5
}

/// Outer counterclockwise, holes clockwise: the wall builder encodes
/// facing in traversal direction, so this is enforced rather than
/// assumed from the kernel's defaults.
pub fn orient(shapes: &mut Shapes) {
    for s in shapes.iter_mut() {
        for (i, c) in s.iter_mut().enumerate() {
            let ccw = signed_area(c) > 0.0;
            if (i == 0) != ccw {
                c.reverse();
            }
        }
    }
}

fn tidy(mut shapes: Shapes) -> Shapes {
    for s in shapes.iter_mut() {
        s.retain(|c| c.len() >= 3);
    }
    shapes.retain(|s| !s.is_empty());
    orient(&mut shapes);
    shapes
}

/// Resolves raw contours into clean regions under `rule`.
pub fn clean(contours: &[Contour], rule: FillRule) -> Result<Shapes, KitError> {
    let subj: Vec<Contour> = contours.iter().filter(|c| c.len() >= 3).cloned().collect();
    if subj.is_empty() {
        return Ok(Vec::new());
    }
    let clip: Vec<Contour> = Vec::new();
    let shapes: Shapes = subj
        .overlay_with_fixed_scale(&clip, OverlayRule::Subject, rule, KIT_SCALE)
        .map_err(scale_err)?;
    Ok(tidy(shapes))
}

/// Boolean of two clean regions (nonzero: both are already consistent).
pub fn boolean(a: &Shapes, b: &Shapes, rule: OverlayRule) -> Result<Shapes, KitError> {
    if a.is_empty() && b.is_empty() {
        return Ok(Vec::new());
    }
    let shapes: Shapes = a
        .overlay_with_fixed_scale(b, rule, FillRule::NonZero, KIT_SCALE)
        .map_err(scale_err)?;
    Ok(tidy(shapes))
}

/// Grows (delta > 0) or shrinks (delta < 0) a region, round joins. A
/// region thinner than twice a negative delta vanishes, which is the
/// correct limit.
pub fn offset(shapes: &Shapes, delta: f32) -> Result<Shapes, KitError> {
    if shapes.is_empty() || delta == 0.0 {
        return Ok(shapes.clone());
    }
    let style = OutlineStyle::new(delta).line_join(LineJoin::Round(0.2));
    let out: Shapes = shapes
        .outline_fixed_scale(&style, KIT_SCALE)
        .map_err(scale_err)?;
    Ok(tidy(out))
}

pub fn extent(shapes: &Shapes) -> f32 {
    shapes
        .iter()
        .filter_map(|s| s.first())
        .flatten()
        .map(|p| p[0].abs().max(p[1].abs()))
        .fold(0.0f32, f32::max)
}

pub fn is_empty(shapes: &Shapes) -> bool {
    shapes.iter().all(|s| s.is_empty() || s[0].len() < 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(h: f32) -> Contour {
        vec![[-h, -h], [h, -h], [h, h], [-h, h]]
    }

    #[test]
    fn clean_orients_outer_ccw_and_holes_cw() {
        // Outer clockwise on purpose, hole counterclockwise on purpose.
        let outer: Contour = vec![[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]];
        let hole: Contour = vec![[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];
        let s = clean(&[outer, hole], FillRule::EvenOdd).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].len(), 2);
        assert!(signed_area(&s[0][0]) > 0.0);
        assert!(signed_area(&s[0][1]) < 0.0);
    }

    #[test]
    fn boolean_difference_cuts_a_hole() {
        let a = clean(&[square(1.0)], FillRule::NonZero).unwrap();
        let b = clean(&[square(0.4)], FillRule::NonZero).unwrap();
        let d = boolean(&a, &b, OverlayRule::Difference).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].len(), 2, "outer plus one hole");
        let u = boolean(&a, &b, OverlayRule::Union).unwrap();
        assert_eq!(u[0].len(), 1);
    }

    #[test]
    fn identical_inputs_give_bit_identical_vertices() {
        // The whole kit leans on this: a region's contour must come back
        // with the same bits whenever it is derived from the same inputs.
        let a = clean(&[square(1.0)], FillRule::NonZero).unwrap();
        let b = clean(&[square(0.4)], FillRule::NonZero).unwrap();
        let d1 = boolean(&a, &b, OverlayRule::Difference).unwrap();
        let d2 = boolean(&a, &b, OverlayRule::Difference).unwrap();
        assert_eq!(d1, d2);
        let i = boolean(&a, &b, OverlayRule::Intersect).unwrap();
        // The hole of the difference is the inner square, the same points
        // the intersection's outer contour has.
        let mut hole_pts: Vec<[u32; 2]> = d1[0][1]
            .iter()
            .map(|p| [p[0].to_bits(), p[1].to_bits()])
            .collect();
        let mut inner_pts: Vec<[u32; 2]> = i[0][0]
            .iter()
            .map(|p| [p[0].to_bits(), p[1].to_bits()])
            .collect();
        hole_pts.sort();
        inner_pts.sort();
        assert_eq!(hole_pts, inner_pts);
    }

    #[test]
    fn offset_shrinks_and_grows() {
        let a = clean(&[square(1.0)], FillRule::NonZero).unwrap();
        let smaller = offset(&a, -0.25).unwrap();
        assert!((extent(&smaller) - 0.75).abs() < 1e-3);
        let bigger = offset(&a, 0.25).unwrap();
        assert!(
            (extent(&bigger) - 1.25).abs() < 2e-2,
            "round joins stay near 1.25"
        );
        let gone = offset(&a, -1.5).unwrap();
        assert!(is_empty(&gone));
    }

    #[test]
    fn degenerate_input_is_empty_not_an_error() {
        let line: Contour = vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]];
        assert!(is_empty(&clean(&[line], FillRule::NonZero).unwrap()));
        assert!(is_empty(&clean(&[], FillRule::NonZero).unwrap()));
    }
}
