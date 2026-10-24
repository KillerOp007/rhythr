//! A part becomes clean regions: source outline, effects in a fixed
//! order (waves, round, rim), the 2D transform, symmetry copies, then
//! one nonzero clean so overlapping copies merge.

use crate::kit::doc::*;
use crate::kit::geom::{boolean, clean, offset, Contour, FillRule, OverlayRule, Shapes};
use crate::kit::shapes::base_contours;
use crate::kit::KitError;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPart {
    pub id: String,
    pub mode: Mode,
    pub material: Material,
    pub depth: Depth,
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
    if w.count == 0 || w.depth == 0.0 {
        return;
    }
    for c in contours.iter_mut() {
        for p in c.iter_mut() {
            let th = p[1].atan2(p[0]);
            let f = 1.0 + w.depth * wave_fn(w.style, w.count as f32 * th);
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
    if let Some(n) = e.repeat.filter(|n| *n >= 2) {
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

pub fn resolve_part(part: &Part) -> Result<ResolvedPart, KitError> {
    let empty = || ResolvedPart {
        id: part.id.clone(),
        mode: part.mode,
        material: part.material,
        depth: part.depth,
        shapes: Vec::new(),
    };
    let mut contours = base_contours(&part.shape);
    if contours.is_empty() || part.transform.w.abs() < 1e-4 || part.transform.h.abs() < 1e-4 {
        return Ok(empty());
    }
    if let Some(w) = &part.effects.waves {
        apply_waves(&mut contours, w);
    }
    // Round and rim work on clean regions; both are offsets.
    let mut region: Shapes = clean(&contours, FillRule::NonZero)?;
    if let Some(r) = part.effects.round.filter(|r| *r > 1e-4) {
        // Closing then opening: concave corners first, then convex ones.
        region = offset(&offset(&region, r)?, -2.0 * r)?;
        region = offset(&region, r)?;
    }
    if let Some(t) = part.effects.rim.filter(|t| *t > 1e-4) {
        let eroded = offset(&region, -t)?;
        if !eroded.is_empty() {
            region = boolean(&region, &eroded, OverlayRule::Difference)?;
        }
    }
    let mut flat: Vec<Contour> = region.into_iter().flatten().collect();
    apply_transform(&mut flat, &part.transform);
    let copies = symmetry_copies(flat, &part.effects);
    let shapes = clean(&copies, FillRule::NonZero)?;
    Ok(ResolvedPart { shapes, ..empty() })
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

    #[test]
    fn rim_makes_a_hole_and_round_softens_corners() {
        let mut p = part(Shape::Square { rounding: 0.0 });
        p.effects.rim = Some(0.3);
        let r = resolve_part(&p).unwrap();
        assert_eq!(r.shapes[0].len(), 2, "outer plus hole");
        assert!(signed_area(&r.shapes[0][1]) < 0.0);
        let mut q = part(Shape::Square { rounding: 0.0 });
        q.effects.round = Some(0.3);
        let r = resolve_part(&q).unwrap();
        // The corner at (1,1) is gone: no point further than 1.3 from the
        // centre along the diagonal direction.
        let max_diag = r.shapes[0][0]
            .iter()
            .map(|v| v[0] + v[1])
            .fold(f32::MIN, f32::max);
        assert!(max_diag < 1.9, "diagonal reach {max_diag}");
    }

    #[test]
    fn waves_change_the_radius() {
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
        assert!(min < 0.9 && max > 1.1, "radii {min}..{max}");
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
    fn hidden_or_degenerate_parts_resolve_to_nothing() {
        let mut p = part(Shape::Points { control: vec![] });
        let r = resolve_part(&p).unwrap();
        assert!(r.shapes.is_empty());
        p.shape = Shape::Circle;
        p.transform.w = 0.0;
        let r = resolve_part(&p).unwrap();
        assert!(r.shapes.is_empty());
    }
}
