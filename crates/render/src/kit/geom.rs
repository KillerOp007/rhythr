//! Fixed-scale, fixed-origin wrappers around i_overlay. Every operation
//! snaps to ONE integer lattice (cell 1/65536, origin at the note's
//! centre, spanning plus and minus KIT_RANGE), so a region derived twice
//! from the same inputs is bit-identical. Contours shared between
//! neighbouring results line up vertex for vertex where every vertex is
//! kept, which is `boolean_keep_collinear` (the mesh faces); the region
//! paths drop collinear points and micro loops. i_overlay's own
//! fixed-scale entry points fix the cell size but centre the lattice on
//! each call's bounding box, which is why the adapter is built here.

use crate::kit::{KitError, KIT_SCALE};
pub use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::ShapeType;
pub use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::core::solver::Solver;
use i_overlay::float::overlay::{FloatOverlay, OverlayOptions};
use i_overlay::i_float::adapter::FloatPointAdapter;
use i_overlay::i_float::float::rect::FloatRect;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};

pub type Contour = Vec<[f32; 2]>;
pub type Shapes = Vec<Vec<Contour>>;

/// Half-extent of the lattice in note units. Notes live in the unit box
/// and depth never exceeds 2, so 64 leaves room for any transform the
/// editor allows, while 64 * 65536 stays far inside the i32 engine.
pub const KIT_RANGE: f32 = 64.0;

/// Smallest area a region result keeps: a square of side 0.001 on a note
/// two units wide. Offsets leave micro loops of a few lattice cells at
/// inner corners, and a later dilation turns each one into a full disc of
/// the offset radius. The threshold is in note units, which are the
/// design's own units (the box is fixed, nothing is rescaled). A part
/// smaller than about 0.03 in extent therefore loses detail; the editor
/// keeps transforms in its ranges.
pub const MIN_SHAPE_AREA: f32 = 1e-6;

/// Signed area, positive for counterclockwise. Summed in f64 relative to
/// the first vertex: a sliver one lattice cell wide would otherwise
/// cancel in f32 and come out with the wrong sign.
pub fn signed_area(c: &[[f32; 2]]) -> f32 {
    if c.len() < 3 {
        return 0.0;
    }
    let (ox, oy) = (c[0][0] as f64, c[0][1] as f64);
    let mut a = 0.0f64;
    for i in 0..c.len() {
        let p = c[i];
        let q = c[(i + 1) % c.len()];
        let (px, py) = (p[0] as f64 - ox, p[1] as f64 - oy);
        let (qx, qy) = (q[0] as f64 - ox, q[1] as f64 - oy);
        a += px * qy - qx * py;
    }
    (a * 0.5) as f32
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
    // A shape whose outer boundary degenerates is dropped whole: keeping
    // a surviving hole would promote it to a filled region.
    shapes.retain(|s| s.first().is_some_and(|outer| outer.len() >= 3));
    for s in shapes.iter_mut() {
        s.retain(|c| c.len() >= 3);
    }
    orient(&mut shapes);
    shapes
}

fn check_range<'a>(pts: impl Iterator<Item = &'a [f32; 2]>) -> Result<(), KitError> {
    for p in pts {
        let ok = p[0].is_finite()
            && p[1].is_finite()
            && p[0].abs() <= KIT_RANGE
            && p[1].abs() <= KIT_RANGE;
        if !ok {
            return Err(KitError::Scale(format!(
                "point {p:?} is outside the lattice range of {KIT_RANGE}"
            )));
        }
    }
    Ok(())
}

/// The one lattice every operation shares.
fn adapter() -> FloatPointAdapter<[f32; 2], i32> {
    FloatPointAdapter::with_scale(
        FloatRect::new(-KIT_RANGE, KIT_RANGE, -KIT_RANGE, KIT_RANGE),
        KIT_SCALE,
    )
}

fn overlay(
    subject: &Shapes,
    clip: &Shapes,
    rule: OverlayRule,
    fill: FillRule,
) -> Result<Shapes, KitError> {
    overlay_with(subject, clip, rule, fill, false, MIN_SHAPE_AREA)
}

/// `keep_collinear` sets both of i_overlay's collinear options (input and
/// output); false is exactly the library default every other caller uses.
/// `min_area` (note units squared) is i_overlay's `min_output_area`: a
/// contour whose area is smaller is dropped from the result. The field
/// compares the AREA, not twice the area (core/extract.rs shifts the
/// doubled area right by one before comparing), and the float wrapper
/// multiplies it by the adapter's scale squared, which is KIT_SCALE *
/// KIT_SCALE here, so the note-unit value goes in unconverted. Region
/// paths pass MIN_SHAPE_AREA; the face path passes 0 so a subdivision
/// stays an exact partition.
fn overlay_with(
    subject: &Shapes,
    clip: &Shapes,
    rule: OverlayRule,
    fill: FillRule,
    keep_collinear: bool,
    min_area: f32,
) -> Result<Shapes, KitError> {
    check_range(subject.iter().flatten().flatten())?;
    check_range(clip.iter().flatten().flatten())?;
    let points = subject.iter().flatten().map(|c| c.len()).sum::<usize>()
        + clip.iter().flatten().map(|c| c.len()).sum::<usize>();
    let mut options = OverlayOptions::default();
    options.preserve_input_collinear = keep_collinear;
    options.preserve_output_collinear = keep_collinear;
    options.min_output_area = min_area;
    let out: Shapes =
        FloatOverlay::new_custom(adapter(), options, Solver::default(), points.max(16))
            .unsafe_add_source(subject, ShapeType::Subject)
            .unsafe_add_source(clip, ShapeType::Clip)
            .overlay(rule, fill);
    Ok(tidy(out))
}

/// Resolves raw contours into clean regions under `rule`.
pub fn clean(contours: &[Contour], rule: FillRule) -> Result<Shapes, KitError> {
    let raw: Vec<Contour> = contours.iter().filter(|c| c.len() >= 3).cloned().collect();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    overlay(&vec![raw], &Vec::new(), OverlayRule::Subject, rule)
}

/// Passes a region through the shared lattice once more, so coordinates
/// produced outside i_overlay (a scale, a hand-written contour) sit on
/// the same grid as every overlay result.
pub fn snap(shapes: &Shapes) -> Result<Shapes, KitError> {
    if shapes.is_empty() {
        return Ok(Vec::new());
    }
    overlay(shapes, &Vec::new(), OverlayRule::Subject, FillRule::NonZero)
}

/// Boolean of two clean regions (nonzero: both are already consistent).
pub fn boolean(a: &Shapes, b: &Shapes, rule: OverlayRule) -> Result<Shapes, KitError> {
    if a.is_empty() && b.is_empty() {
        return Ok(Vec::new());
    }
    overlay(a, b, rule, FillRule::NonZero)
}

/// `boolean` with i_overlay's collinear options on. The default drops an
/// exactly collinear vertex from the output, so a corner that one face
/// keeps can vanish from its neighbour and the two stop sharing edges one
/// to one. Faces that must keep every shared vertex come from here. It
/// also drops no small contour (no minimum area): faces are an exact
/// partition, and a missing sliver face would leave open edges.
pub fn boolean_keep_collinear(
    a: &Shapes,
    b: &Shapes,
    rule: OverlayRule,
) -> Result<Shapes, KitError> {
    if a.is_empty() && b.is_empty() {
        return Ok(Vec::new());
    }
    overlay_with(a, b, rule, FillRule::NonZero, true, 0.0)
}

/// Grows (delta > 0) or shrinks (delta < 0) a region, round joins. A
/// region thinner than twice a negative delta vanishes, which is the
/// correct limit. The outline call centres its own lattice on the input,
/// so its result passes through the shared lattice once more; the input
/// is oriented first because the outline builder reads facing from the
/// traversal direction.
pub fn offset(shapes: &Shapes, delta: f32) -> Result<Shapes, KitError> {
    if !delta.is_finite() {
        return Err(KitError::Scale(format!("offset {delta} is not finite")));
    }
    if shapes.is_empty() || delta == 0.0 {
        return Ok(shapes.clone());
    }
    check_range(shapes.iter().flatten().flatten())?;
    let mut input = shapes.clone();
    orient(&mut input);
    let style = OutlineStyle::new(delta).line_join(LineJoin::Round(0.2));
    let grown: Shapes = input
        .outline_fixed_scale(&style, KIT_SCALE)
        .map_err(|e| KitError::Scale(format!("{e:?}")))?;
    let grown = tidy(grown);
    if grown.is_empty() {
        return Ok(Vec::new());
    }
    overlay(&grown, &Vec::new(), OverlayRule::Subject, FillRule::NonZero)
}

/// The largest |x| or |y| over every outer contour.
pub fn extent(shapes: &Shapes) -> f32 {
    shapes
        .iter()
        .filter_map(|s| s.first())
        .flatten()
        .map(|p| p[0].abs().max(p[1].abs()))
        .fold(0.0f32, f32::max)
}

/// True when no shape has a usable outer contour.
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
    fn keep_collinear_variant_keeps_a_vertex_the_default_drops() {
        // A square with an extra vertex in the middle of its bottom side.
        let side: Shapes = vec![vec![vec![
            [-1.0, -1.0],
            [0.0, -1.0],
            [1.0, -1.0],
            [1.0, 1.0],
            [-1.0, 1.0],
        ]]];
        let none = Shapes::new();
        let kept = boolean_keep_collinear(&side, &none, OverlayRule::Union).unwrap();
        assert_eq!(kept.len(), 1);
        assert!(kept[0][0].contains(&[0.0, -1.0]), "{:?}", kept[0][0]);
        let dropped = boolean(&side, &none, OverlayRule::Union).unwrap();
        assert!(!dropped[0][0].contains(&[0.0, -1.0]), "{:?}", dropped[0][0]);
        assert_eq!(dropped[0][0].len(), 4);
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

    #[test]
    fn off_centre_inputs_share_ring_vertices_bit_for_bit() {
        // i_overlay's own fixed-scale calls centre their lattice on each
        // call's bounding box, so a ring and the cap cut around it came
        // back on different lattices. The shared adapter ends that: the
        // cap's hole must be the ring, bit for bit, for inputs that are
        // not centred and for inputs already on the lattice but with a
        // bounding-box centre half a cell off it.
        let rect = |x0: f32, y0: f32, x1: f32, y1: f32| -> Contour {
            vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
        };
        let bits = |c: &Contour| {
            let mut v: Vec<[u32; 2]> = c.iter().map(|p| [p[0].to_bits(), p[1].to_bits()]).collect();
            v.sort();
            v
        };
        for inner in [
            rect(0.1, -0.3, 0.7, 0.45),
            rect(0.25, -0.25, 0.5 + 1.0 / 65536.0, 0.5),
        ] {
            let r1 = clean(&[inner], FillRule::NonZero).unwrap();
            let r2 = clean(&[rect(-1.0, -1.0, 1.3, 1.0)], FillRule::NonZero).unwrap();
            let cap = boolean(&r2, &r1, OverlayRule::Difference).unwrap();
            assert_eq!(cap.len(), 1);
            assert_eq!(cap[0].len(), 2, "outer plus the hole the ring cut");
            assert_eq!(bits(&cap[0][1]), bits(&r1[0][0]));
        }
    }

    #[test]
    fn offset_grows_a_region_and_shrinks_its_hole() {
        let mut hole = square(0.5);
        hole.reverse();
        let ring = clean(&[square(1.0), hole], FillRule::NonZero).unwrap();
        assert_eq!(ring[0].len(), 2);
        let grown = offset(&ring, 0.1).unwrap();
        assert!((extent(&grown) - 1.1).abs() < 2e-2);
        let hole_extent = grown[0][1]
            .iter()
            .map(|p| p[0].abs().max(p[1].abs()))
            .fold(0.0f32, f32::max);
        assert!(
            (hole_extent - 0.4).abs() < 1e-3,
            "hole extent {hole_extent}"
        );
    }

    #[test]
    fn out_of_range_or_nan_input_is_a_scale_error() {
        let far: Contour = vec![[100.0, 0.0], [101.0, 0.0], [100.0, 1.0]];
        assert!(matches!(
            clean(&[far], FillRule::NonZero),
            Err(KitError::Scale(_))
        ));
        let a = clean(&[square(1.0)], FillRule::NonZero).unwrap();
        assert!(matches!(offset(&a, f32::NAN), Err(KitError::Scale(_))));
    }

    #[test]
    fn snap_puts_every_coordinate_on_the_lattice() {
        let tri: Shapes = vec![vec![vec![[0.1, 0.2], [0.3, 0.2], [0.3, 0.7]]]];
        let snapped = snap(&tri).unwrap();
        assert_eq!(snapped[0][0].len(), 3);
        for p in snapped.iter().flatten().flatten() {
            for v in p {
                let k = v * KIT_SCALE;
                assert!((k - k.round()).abs() < 1e-2, "{v} is off the lattice");
            }
        }
    }

    #[test]
    fn micro_loops_vanish_from_region_results() {
        // A unit square and a three-point loop 1e-4 across (area 5e-9).
        let square = vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let micro = vec![[2.0, 2.0], [2.0001, 2.0], [2.0, 2.0001]];
        let r = clean(&[square.clone(), micro.clone()], FillRule::NonZero).unwrap();
        assert_eq!(r.len(), 1, "the micro loop must go");
        let snapped = snap(&vec![vec![square], vec![micro]]).unwrap();
        assert_eq!(snapped.len(), 1);
    }

    #[test]
    fn the_area_threshold_is_an_area_not_twice_one() {
        // Side 0.0009 is area 8.1e-7, below MIN_SHAPE_AREA, but twice it
        // is above; side 0.0011 is area 1.21e-6, above, but half of it is
        // below. Only a comparison of the area itself drops the first and
        // keeps the second.
        let sq = |s: f32| vec![[0.0, 0.0], [s, 0.0], [s, s], [0.0, s]];
        assert!(is_empty(&clean(&[sq(0.0009)], FillRule::NonZero).unwrap()));
        assert_eq!(clean(&[sq(0.0011)], FillRule::NonZero).unwrap().len(), 1);
    }

    #[test]
    fn small_but_visible_shapes_survive() {
        let tiny = vec![[0.0, 0.0], [0.01, 0.0], [0.01, 0.01], [0.0, 0.01]];
        let r = clean(&[tiny], FillRule::NonZero).unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn face_results_keep_every_sliver() {
        let square: Shapes = vec![vec![vec![
            [-1.0, -1.0],
            [1.0, -1.0],
            [1.0, 1.0],
            [-1.0, 1.0],
        ]]];
        let micro: Shapes = vec![vec![vec![[2.0, 2.0], [2.0001, 2.0], [2.0, 2.0001]]]];
        let r = boolean_keep_collinear(&square, &micro, OverlayRule::Union).unwrap();
        assert_eq!(
            r.len(),
            2,
            "faces are an exact partition, nothing is dropped"
        );
    }
}
