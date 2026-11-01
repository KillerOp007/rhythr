//! A part becomes clean regions: source outline, waves (keyed on each
//! point's angle, running inward from the outline, applied before the
//! transform so a turned shape keeps its wave phase), the 2D transform,
//! symmetry copies, then one
//! nonzero clean so overlapping copies merge. Rounding and the rim are
//! deferred to `finish_part`, which the evaluation runs on the merged
//! region of the part, in note units (the box is fixed, so design units
//! are note units), before the part is cut to the box.

use crate::kit::doc::*;
use crate::kit::geom::{boolean, clean, offset, Contour, FillRule, OverlayRule, Shapes};
use crate::kit::shapes::base_contours;
use crate::kit::KitError;

/// Caps on stored parameters: a corrupt document degrades, it never
/// allocates or loops without bound.
pub(crate) const MAX_REPEAT: u32 = 32;
const MAX_WAVES: u32 = 32;
const MAX_WAVE_DEPTH: f32 = 0.95;
const MAX_FINISH: f32 = 0.9;
/// Complexity budget of one resolved part. The caps above bound the
/// loops, not the geometry: thirty-two wavy copies can merge into
/// thousands of contours, and an offset on that runs for minutes and
/// takes gigabytes. A part beyond this budget is a clear error instead.
pub const MAX_PART_POINTS: usize = 16_384;
pub const MAX_PART_CONTOURS: usize = 512;

fn complexity_check(shapes: &Shapes) -> Result<(), KitError> {
    let contours: usize = shapes.iter().map(|s| s.len()).sum();
    let points: usize = shapes.iter().flatten().map(|c| c.len()).sum();
    if contours > MAX_PART_CONTOURS || points > MAX_PART_POINTS {
        return Err(KitError::Complexity(points, contours));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPart {
    pub id: String,
    pub mode: Mode,
    pub material: Material,
    pub depth: Depth,
    /// Corner rounding radius and rim thickness in note units, applied
    /// by `finish_part` on the merged region.
    pub round: Option<f32>,
    pub rim: Option<f32>,
    pub shapes: Shapes,
}

fn wave_fn(style: WaveStyle, k: f32) -> f32 {
    let s = k.sin();
    match style {
        WaveStyle::Sine => s,
        WaveStyle::Spike => (2.0 / std::f32::consts::PI) * s.asin(),
        WaveStyle::Pulse => (3.0 * s).tanh() / 3.0f32.tanh(),
    }
}

/// Radial modulation around the part's own centre; a point's angle
/// picks the phase, so any dense outline (not only circles) waves.
fn apply_waves(contours: &mut [Contour], w: &Waves) {
    let count = w.count.min(MAX_WAVES);
    let depth = w.depth.clamp(-MAX_WAVE_DEPTH, MAX_WAVE_DEPTH);
    if count == 0 || depth == 0.0 {
        return;
    }
    for c in contours.iter_mut() {
        for p in c.iter_mut() {
            let th = p[1].atan2(p[0]);
            // Inward only: the wave's crest sits on the source outline and
            // the trough at 1 minus the depth, so the part stays within its
            // own extent (the handles and the box hold).
            let f = 1.0 + depth * (wave_fn(w.style, count as f32 * th) - 1.0) * 0.5;
            p[0] *= f;
            p[1] *= f;
        }
    }
}

fn apply_transform(contours: &mut [Contour], t: &Transform) {
    let (s, c) = t.rotation.to_radians().sin_cos();
    for contour in contours.iter_mut() {
        for p in contour.iter_mut() {
            let x = p[0] * t.w;
            let y = p[1] * t.h;
            p[0] = x * c - y * s + t.x;
            p[1] = x * s + y * c + t.y;
        }
    }
}

fn reflect(contours: &[Contour], fx: f32, fy: f32) -> Vec<Contour> {
    contours
        .iter()
        .map(|c| {
            let mut r: Contour = c.iter().map(|p| [p[0] * fx, p[1] * fy]).collect();
            // A single reflection flips orientation; reverse to keep the
            // nonzero winding consistent across copies.
            if fx * fy < 0.0 {
                r.reverse();
            }
            r
        })
        .collect()
}

fn symmetry_copies(contours: Vec<Contour>, e: &Effects) -> Vec<Contour> {
    let mut out: Vec<Contour> = contours.clone();
    match e.mirror {
        Mirror::None => {}
        Mirror::X => out.extend(reflect(&contours, 1.0, -1.0)),
        Mirror::Y => out.extend(reflect(&contours, -1.0, 1.0)),
        Mirror::Both => {
            out.extend(reflect(&contours, 1.0, -1.0));
            out.extend(reflect(&contours, -1.0, 1.0));
            out.extend(reflect(&contours, -1.0, -1.0));
        }
    }
    if let Some(n) = e.repeat.filter(|n| *n >= 2).map(|n| n.min(MAX_REPEAT)) {
        let base = out.clone();
        for k in 1..n {
            let a = std::f32::consts::TAU * k as f32 / n as f32;
            let (s, c) = a.sin_cos();
            out.extend(base.iter().map(|contour| {
                contour
                    .iter()
                    .map(|p| [p[0] * c - p[1] * s, p[0] * s + p[1] * c])
                    .collect::<Contour>()
            }));
        }
    }
    out
}

fn finish_value(v: Option<f32>) -> Option<f32> {
    v.filter(|x| x.is_finite() && *x > 1e-4)
        .map(|x| x.min(MAX_FINISH))
}

pub fn resolve_part(part: &Part) -> Result<ResolvedPart, KitError> {
    let empty = || ResolvedPart {
        id: part.id.clone(),
        mode: part.mode,
        material: part.material,
        depth: part.depth,
        round: finish_value(part.effects.round),
        rim: finish_value(part.effects.rim),
        shapes: Vec::new(),
    };
    let mut contours = base_contours(&part.shape);
    if contours.is_empty() || part.transform.w.abs() < 1e-4 || part.transform.h.abs() < 1e-4 {
        return Ok(empty());
    }
    if let Some(w) = &part.effects.waves {
        apply_waves(&mut contours, w);
    }
    apply_transform(&mut contours, &part.transform);
    let copies = symmetry_copies(contours, &part.effects);
    // The clean is the first heavy step, so a copy set far beyond the
    // budget is refused before it. The lattice drops collinear points, so
    // the multiple stays generous and the clean still sees any source that
    // could pass the cleaned budget.
    let raw_points: usize = copies.iter().map(|c| c.len()).sum();
    if raw_points > 8 * MAX_PART_POINTS {
        return Err(KitError::Complexity(raw_points, copies.len()));
    }
    let shapes = clean(&copies, FillRule::NonZero)?;
    complexity_check(&shapes)?;
    Ok(ResolvedPart { shapes, ..empty() })
}

/// Rounding (closing, then opening) and the rim (the region minus its
/// erosion) in note units, on a merged region. Where the
/// region is thinner than the rounding or the rim allows, it stays as it
/// is: a finish never deletes a part. Offsets are the expensive step,
/// so the complexity budget is checked before them.
pub fn finish_part(
    shapes: &Shapes,
    round: Option<f32>,
    rim: Option<f32>,
) -> Result<Shapes, KitError> {
    complexity_check(shapes)?;
    let mut region = shapes.clone();
    if let Some(r) = finish_value(round) {
        let closed = offset(&offset(&region, r)?, -r)?;
        let opened = offset(&closed, -r)?;
        if !opened.is_empty() {
            region = offset(&opened, r)?;
        }
    }
    if let Some(t) = finish_value(rim) {
        let eroded = offset(&region, -t)?;
        if !eroded.is_empty() {
            region = boolean(&region, &eroded, OverlayRule::Difference)?;
        }
    }
    Ok(region)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::geom::{extent, signed_area};

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
    fn plain_circle_resolves_to_one_shape() {
        let r = resolve_part(&part(Shape::Circle)).unwrap();
        assert_eq!(r.shapes.len(), 1);
        assert_eq!(r.shapes[0].len(), 1);
        assert!((extent(&r.shapes) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn transform_scales_rotates_and_moves() {
        let mut p = part(Shape::Square { rounding: 0.0 });
        p.transform = Transform {
            x: 0.5,
            y: 0.0,
            w: 0.5,
            h: 0.25,
            rotation: 0.0,
        };
        let r = resolve_part(&p).unwrap();
        let xs: Vec<f32> = r.shapes[0][0].iter().map(|q| q[0]).collect();
        let ys: Vec<f32> = r.shapes[0][0].iter().map(|q| q[1]).collect();
        let (min_x, max_x) = (
            xs.iter().cloned().fold(f32::MAX, f32::min),
            xs.iter().cloned().fold(f32::MIN, f32::max),
        );
        let (min_y, max_y) = (
            ys.iter().cloned().fold(f32::MAX, f32::min),
            ys.iter().cloned().fold(f32::MIN, f32::max),
        );
        assert!((min_x - 0.0).abs() < 1e-3 && (max_x - 1.0).abs() < 1e-3);
        assert!((min_y + 0.25).abs() < 1e-3 && (max_y - 0.25).abs() < 1e-3);
        p.transform = Transform {
            rotation: 90.0,
            w: 0.5,
            h: 0.25,
            ..Transform::default()
        };
        let r = resolve_part(&p).unwrap();
        let ys: Vec<f32> = r.shapes[0][0].iter().map(|q| q[1]).collect();
        assert!((ys.iter().cloned().fold(f32::MIN, f32::max) - 0.5).abs() < 1e-3);
    }

    fn hole_extent(shapes: &Shapes) -> f32 {
        shapes[0][1]
            .iter()
            .map(|p| p[0].abs().max(p[1].abs()))
            .fold(0.0f32, f32::max)
    }

    #[test]
    fn finish_rims_and_rounds_in_note_units() {
        let r = resolve_part(&part(Shape::Square { rounding: 0.0 })).unwrap();
        assert_eq!(r.rim, None, "resolution carries the rim, finish applies it");
        let rimmed = finish_part(&r.shapes, None, Some(0.3)).unwrap();
        assert_eq!(rimmed[0].len(), 2, "outer plus hole");
        assert!(signed_area(&rimmed[0][1]) < 0.0);
        assert!((hole_extent(&rimmed) - 0.7).abs() < 1e-3);
        let rounded = finish_part(&r.shapes, Some(0.3), None).unwrap();
        // The corner at (1,1) is gone while the flat sides still touch 1:
        // the diagonal reach of a sharp corner is 2.0, of this rounding
        // about 1.78.
        let max_diag = rounded[0][0]
            .iter()
            .map(|v| v[0] + v[1])
            .fold(f32::MIN, f32::max);
        assert!(
            max_diag < 1.85 && max_diag > 1.7,
            "diagonal reach {max_diag}"
        );
        assert!((extent(&rounded) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_finish_never_deletes_a_part() {
        // A ring whose wall is thinner than the rounding or the rim keeps
        // its region instead of vanishing.
        let r = resolve_part(&part(Shape::Ring { thickness: 0.4 })).unwrap();
        let rounded = finish_part(&r.shapes, Some(0.25), None).unwrap();
        assert_eq!(rounded.len(), 1);
        assert_eq!(rounded[0].len(), 2);
        let rimmed = finish_part(&r.shapes, None, Some(0.5)).unwrap();
        assert_eq!(rimmed[0].len(), 2);
    }

    #[test]
    fn rim_is_uniform_after_an_anisotropic_transform() {
        // A rim is a note-unit thickness, so a part scaled differently in
        // x and y still gets the same band on every side.
        let mut p = part(Shape::Square { rounding: 0.0 });
        p.transform = Transform {
            w: 0.5,
            h: 0.25,
            ..Transform::default()
        };
        let r = resolve_part(&p).unwrap();
        let rimmed = finish_part(&r.shapes, None, Some(0.05)).unwrap();
        let hole = &rimmed[0][1];
        let hx = hole.iter().map(|q| q[0].abs()).fold(0.0f32, f32::max);
        let hy = hole.iter().map(|q| q[1].abs()).fold(0.0f32, f32::max);
        assert!((hx - 0.45).abs() < 1e-3, "hole half-width {hx}");
        assert!((hy - 0.2).abs() < 1e-3, "hole half-height {hy}");
    }

    #[test]
    fn rotation_is_counterclockwise() {
        // A triangle with its apex up: turned 90 degrees counterclockwise
        // the apex points left, so x never exceeds zero.
        let ctrl = |x: f32, y: f32| crate::kit::v1::ControlPoint {
            x,
            y,
            smooth: false,
        };
        let mut p = part(Shape::Points {
            control: vec![ctrl(1.0, 0.0), ctrl(0.0, 1.0), ctrl(-1.0, 0.0)],
            loops: vec![],
        });
        p.transform.rotation = 90.0;
        let r = resolve_part(&p).unwrap();
        let xs: Vec<f32> = r.shapes[0][0].iter().map(|q| q[0]).collect();
        assert!(xs.iter().cloned().fold(f32::MAX, f32::min) < -0.9);
        assert!(xs.iter().cloned().fold(f32::MIN, f32::max) < 0.01);
    }

    #[test]
    fn corrupt_parameters_are_capped_not_fatal() {
        // A three-point source keeps this cheap: the caps are what is
        // under test, not the geometry. Never run this against code
        // without the caps.
        let ctrl = |x: f32, y: f32| crate::kit::v1::ControlPoint {
            x,
            y,
            smooth: false,
        };
        let mut p = part(Shape::Points {
            control: vec![ctrl(0.6, 0.0), ctrl(0.9, 0.1), ctrl(0.6, 0.2)],
            loops: vec![],
        });
        p.effects.repeat = Some(u32::MAX);
        p.effects.waves = Some(Waves {
            count: u32::MAX,
            depth: 40.0,
            style: WaveStyle::Sine,
        });
        let r = resolve_part(&p).unwrap();
        assert!(!r.shapes.is_empty());
        assert!(r.shapes.len() <= 32, "repeat is capped at 32 copies");
        let finished = finish_part(&r.shapes, Some(5.0), Some(5.0)).unwrap();
        assert!(!finished.is_empty());
    }

    #[test]
    fn an_overly_complex_part_is_a_clear_error() {
        // The caps bound the loops, not the geometry; a region beyond the
        // point budget is refused before any offset can run away. The
        // lattice drops collinear points (20,000 source points left 15,004,
        // inside the budget), so the source needs well over the budget.
        let huge: Vec<[f32; 2]> = (0..40_000)
            .map(|i| {
                let t = i as f32 / 40_000.0 * std::f32::consts::TAU;
                [t.cos(), t.sin()]
            })
            .collect();
        let p = part(Shape::Contours {
            contours: vec![huge],
            label: String::new(),
        });
        assert!(matches!(resolve_part(&p), Err(KitError::Complexity(..))));
    }

    #[test]
    fn an_overly_large_copy_set_is_refused_before_clean() {
        // Eight copies of 40,000 points are 320,000 raw points, far past
        // eight times the budget: refused before the clean ever runs.
        // Never run this against code without the pre-check.
        let huge: Vec<[f32; 2]> = (0..40_000)
            .map(|i| {
                let t = i as f32 / 40_000.0 * std::f32::consts::TAU;
                [t.cos(), t.sin()]
            })
            .collect();
        let mut p = part(Shape::Contours {
            contours: vec![huge],
            label: String::new(),
        });
        p.effects.repeat = Some(8);
        let t = std::time::Instant::now();
        let r = resolve_part(&p);
        assert!(t.elapsed().as_secs_f32() < 0.5, "took {:?}", t.elapsed());
        assert_eq!(r, Err(KitError::Complexity(320_000, 8)));
    }

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

    #[test]
    fn repeat_and_mirror_make_copies_that_merge() {
        let mut p = part(Shape::Circle);
        p.transform = Transform {
            x: 0.6,
            y: 0.0,
            w: 0.3,
            h: 0.3,
            rotation: 0.0,
        };
        p.effects.repeat = Some(4);
        let r = resolve_part(&p).unwrap();
        assert_eq!(r.shapes.len(), 4, "four separate discs");
        let mut q = part(Shape::Circle);
        q.transform = Transform {
            x: 0.0,
            y: 0.4,
            w: 0.5,
            h: 0.5,
            rotation: 0.0,
        };
        q.effects.mirror = Mirror::X;
        let r = resolve_part(&q).unwrap();
        assert_eq!(
            r.shapes.len(),
            1,
            "overlapping mirror copies merge into one region"
        );
        let min_y = r.shapes[0][0].iter().map(|v| v[1]).fold(f32::MAX, f32::min);
        assert!((min_y + 0.9).abs() < 1e-2);
    }

    #[test]
    fn degenerate_parts_resolve_to_nothing() {
        let mut p = part(Shape::Points {
            control: vec![],
            loops: vec![],
        });
        let r = resolve_part(&p).unwrap();
        assert!(r.shapes.is_empty());
        p.shape = Shape::Circle;
        p.transform.w = 0.0;
        let r = resolve_part(&p).unwrap();
        assert!(r.shapes.is_empty());
    }
}
