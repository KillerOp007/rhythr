//! Depth levels and the solid and glass regions between them. Groups
//! fold first (a group's cuts only touch its own adds), then everything
//! combines per interval: union of adds minus union of cuts, with glass
//! as a second region that yields to solid wherever both overlap.

use crate::kit::doc::*;
use crate::kit::geom::{boolean, extent, is_empty, OverlayRule, Shapes};
use crate::kit::resolve::{finish_part, resolve_part, ResolvedPart};
use crate::kit::{KitError, Z_MAX, Z_MIN};
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    pub z0: f32,
    pub z1: f32,
    pub solid: Shapes,
    pub glass: Shapes,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    /// The factor applied to every x and y to put the silhouette on the
    /// unit box (1.0 when it already was).
    pub scale: f32,
    pub levels: Vec<f32>,
    pub intervals: Vec<Interval>,
    pub parts: Vec<ResolvedPart>,
}

const LEVEL_EPS: f32 = 1e-5;

fn union_all(shapes: &[&Shapes]) -> Result<Shapes, KitError> {
    let mut acc: Shapes = Vec::new();
    for s in shapes {
        if is_empty(s) {
            continue;
        }
        acc = if acc.is_empty() {
            (*s).clone()
        } else {
            boolean(&acc, s, OverlayRule::Union)?
        };
    }
    Ok(acc)
}

fn subtract(a: &Shapes, b: &Shapes) -> Result<Shapes, KitError> {
    if is_empty(a) {
        return Ok(Vec::new());
    }
    if is_empty(b) {
        return Ok(a.clone());
    }
    boolean(a, b, OverlayRule::Difference)
}

/// One contribution to an interval after group folding.
struct Contribution<'a> {
    mode: Mode,
    material: Material,
    shapes: Cow<'a, Shapes>,
}

fn covers(p: &ResolvedPart, z0: f32, z1: f32) -> bool {
    let zm = (z0 + z1) * 0.5;
    p.depth.back < zm && zm < p.depth.front
}

pub fn evaluate(design: &Design) -> Result<Evaluation, KitError> {
    for p in design.parts.iter().filter(|p| p.visible) {
        // Written as a binding so a NaN depth (which compares false both
        // ways) is rejected here instead of reaching the level sort.
        let ordered = p.depth.front > p.depth.back;
        if !ordered {
            return Err(KitError::Doc(format!(
                "part {}: front must be above back",
                p.id
            )));
        }
        if p.depth.back < Z_MIN || p.depth.front > Z_MAX {
            return Err(KitError::Doc(format!(
                "part {}: depth must stay within {Z_MIN}..{Z_MAX}",
                p.id
            )));
        }
    }
    let mut parts: Vec<ResolvedPart> = design
        .parts
        .iter()
        .filter(|p| p.visible)
        .map(resolve_part)
        .collect::<Result<_, _>>()?;
    parts.retain(|p| !p.shapes.is_empty());

    // Normalise the silhouette (union of every add) to the unit box.
    let adds: Vec<&Shapes> = parts
        .iter()
        .filter(|p| p.mode == Mode::Add)
        .map(|p| &p.shapes)
        .collect();
    let silhouette = union_all(&adds)?;
    let m = extent(&silhouette);
    if m < 1e-6 {
        return Err(KitError::Empty);
    }
    let scale = if (m - 1.0).abs() > 1e-4 { 1.0 / m } else { 1.0 };
    if scale != 1.0 {
        for p in &mut parts {
            for s in &mut p.shapes {
                for c in s.iter_mut() {
                    for q in c.iter_mut() {
                        q[0] *= scale;
                        q[1] *= scale;
                    }
                }
            }
        }
    }
    // Rounding and rims are note-unit finishes, so they run on the
    // normalised, merged regions (the v1 editor normalised first too); a
    // finish never changes the outer extent, so the scale stays exact.
    for p in &mut parts {
        if p.round.is_some() || p.rim.is_some() {
            p.shapes = finish_part(&p.shapes, p.round, p.rim)?;
        }
    }
    parts.retain(|p| !p.shapes.is_empty());

    // Level set.
    let mut levels: Vec<f32> = parts
        .iter()
        .flat_map(|p| [p.depth.back, p.depth.front])
        .collect();
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap());
    levels.dedup_by(|a, b| (*a - *b).abs() < LEVEL_EPS);

    let in_group: std::collections::HashMap<&str, &Group> = design
        .groups
        .iter()
        .flat_map(|g| g.parts.iter().map(move |id| (id.as_str(), g)))
        .collect();

    let mut intervals = Vec::with_capacity(levels.len().saturating_sub(1));
    let mut any_solid = false;
    for w in levels.windows(2) {
        let (z0, z1) = (w[0], w[1]);
        let covering: Vec<&ResolvedPart> = parts.iter().filter(|p| covers(p, z0, z1)).collect();
        let mut contributions: Vec<Contribution> = Vec::new();
        // Groups fold first.
        for g in &design.groups {
            let members: Vec<&ResolvedPart> = covering
                .iter()
                .copied()
                .filter(|p| g.parts.iter().any(|id| id == &p.id))
                .collect();
            if members.is_empty() {
                continue;
            }
            let adds: Vec<&Shapes> = members
                .iter()
                .filter(|p| p.mode == Mode::Add)
                .map(|p| &p.shapes)
                .collect();
            let cuts: Vec<&Shapes> = members
                .iter()
                .filter(|p| p.mode == Mode::Cut)
                .map(|p| &p.shapes)
                .collect();
            let folded = subtract(&union_all(&adds)?, &union_all(&cuts)?)?;
            let material = if members.iter().all(|p| p.material == Material::Glass) {
                Material::Glass
            } else {
                Material::Solid
            };
            contributions.push(Contribution {
                mode: g.mode,
                material,
                shapes: Cow::Owned(folded),
            });
        }
        for p in covering
            .iter()
            .filter(|p| !in_group.contains_key(p.id.as_str()))
        {
            contributions.push(Contribution {
                mode: p.mode,
                material: p.material,
                shapes: Cow::Borrowed(&p.shapes),
            });
        }
        let pick = |mode: Mode, material: Option<Material>| -> Vec<&Shapes> {
            contributions
                .iter()
                .filter(|c| c.mode == mode && material.is_none_or(|m| c.material == m))
                .map(|c| c.shapes.as_ref())
                .collect()
        };
        let cuts = union_all(&pick(Mode::Cut, None))?;
        let solid = subtract(&union_all(&pick(Mode::Add, Some(Material::Solid)))?, &cuts)?;
        let glass = subtract(
            &subtract(&union_all(&pick(Mode::Add, Some(Material::Glass)))?, &cuts)?,
            &solid,
        )?;
        any_solid |= !is_empty(&solid) || !is_empty(&glass);
        intervals.push(Interval {
            z0,
            z1,
            solid,
            glass,
        });
    }
    if !any_solid {
        return Err(KitError::Empty);
    }
    Ok(Evaluation {
        scale,
        levels,
        intervals,
        parts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(id: &str, shape: Shape, mode: Mode, front: f32, back: f32) -> Part {
        Part {
            id: id.into(),
            name: id.into(),
            visible: true,
            locked: false,
            shape,
            transform: Transform::default(),
            mode,
            depth: Depth { front, back },
            material: Material::Solid,
            effects: Effects::default(),
        }
    }

    fn design(parts: Vec<Part>) -> Design {
        Design {
            version: 2,
            name: "t".into(),
            colors: vec!["#ffffff".into()],
            parts,
            groups: vec![],
        }
    }

    #[test]
    fn one_part_is_one_interval() {
        let ev = evaluate(&design(vec![part(
            "a",
            Shape::Circle,
            Mode::Add,
            0.1,
            -0.1,
        )]))
        .unwrap();
        assert_eq!(ev.levels, vec![-0.1, 0.1]);
        assert_eq!(ev.intervals.len(), 1);
        assert!(!is_empty(&ev.intervals[0].solid));
        assert!(is_empty(&ev.intervals[0].glass));
    }

    #[test]
    fn ring_with_recessed_plate_has_three_levels() {
        let ring = part(
            "ring",
            Shape::Ring { thickness: 0.28 },
            Mode::Add,
            0.1,
            -0.1,
        );
        let mut plate = part("plate", Shape::Circle, Mode::Add, -0.4, -0.6);
        plate.transform = Transform {
            w: 0.55,
            h: 0.55,
            ..Transform::default()
        };
        let ev = evaluate(&design(vec![ring, plate])).unwrap();
        assert_eq!(ev.levels, vec![-0.6, -0.4, -0.1, 0.1]);
        assert_eq!(ev.intervals.len(), 3);
        assert!(!is_empty(&ev.intervals[0].solid), "plate band");
        assert!(
            is_empty(&ev.intervals[1].solid),
            "the gap between plate and ring"
        );
        assert_eq!(
            ev.intervals[2].solid[0].len(),
            2,
            "ring band: outer plus hole"
        );
    }

    #[test]
    fn cuts_remove_and_glass_yields_to_solid() {
        let disc = part("d", Shape::Circle, Mode::Add, 0.1, -0.1);
        let mut bar = part("b", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        bar.transform = Transform {
            w: 1.2,
            h: 0.2,
            ..Transform::default()
        };
        let ev = evaluate(&design(vec![disc.clone(), bar])).unwrap();
        assert_eq!(
            ev.intervals[0].solid.len(),
            2,
            "the bar splits the disc in two"
        );
        let mut glass = part("g", Shape::Square { rounding: 0.0 }, Mode::Add, 0.1, -0.1);
        glass.material = Material::Glass;
        glass.transform = Transform {
            w: 0.5,
            h: 0.5,
            ..Transform::default()
        };
        let ev = evaluate(&design(vec![disc, glass])).unwrap();
        assert!(
            is_empty(&ev.intervals[0].glass),
            "glass fully inside solid is swallowed"
        );
    }

    #[test]
    fn groups_keep_their_cuts_to_themselves() {
        let base = part("base", Shape::Circle, Mode::Add, 0.1, -0.1);
        let mut small = part("small", Shape::Circle, Mode::Add, 0.1, -0.1);
        small.transform = Transform {
            x: 0.5,
            w: 0.4,
            h: 0.4,
            ..Transform::default()
        };
        let mut cut = part("cut", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        cut.transform = Transform {
            x: 0.5,
            w: 0.2,
            h: 0.2,
            ..Transform::default()
        };
        let mut d = design(vec![base, small, cut]);
        d.groups = vec![Group {
            id: "g".into(),
            name: "g".into(),
            parts: vec!["small".into(), "cut".into()],
            mode: Mode::Add,
        }];
        let ev = evaluate(&d).unwrap();
        // The cut lives inside the group: the base disc stays whole, so
        // the solid region is one shape without a hole.
        assert_eq!(ev.intervals[0].solid.len(), 1);
        assert_eq!(ev.intervals[0].solid[0].len(), 1);
    }

    #[test]
    fn silhouette_is_normalised_to_the_box() {
        let mut big = part("a", Shape::Circle, Mode::Add, 0.1, -0.1);
        big.transform = Transform {
            x: 2.0,
            w: 3.0,
            h: 3.0,
            ..Transform::default()
        };
        let ev = evaluate(&design(vec![big])).unwrap();
        assert!((extent(&ev.intervals[0].solid) - 1.0).abs() < 1e-3);
        assert!(ev.scale < 1.0);
    }

    #[test]
    fn all_cut_or_hidden_is_an_error() {
        let mut hidden = part("a", Shape::Circle, Mode::Add, 0.1, -0.1);
        hidden.visible = false;
        assert_eq!(
            evaluate(&design(vec![hidden])).unwrap_err(),
            KitError::Empty
        );
        let only_cut = part("c", Shape::Circle, Mode::Cut, 0.1, -0.1);
        assert_eq!(
            evaluate(&design(vec![only_cut])).unwrap_err(),
            KitError::Empty
        );
        let disc = part("d", Shape::Circle, Mode::Add, 0.1, -0.1);
        let mut eraser = part("e", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        eraser.transform = Transform {
            w: 2.0,
            h: 2.0,
            ..Transform::default()
        };
        assert_eq!(
            evaluate(&design(vec![disc, eraser])).unwrap_err(),
            KitError::Empty
        );
    }

    #[test]
    fn touching_bands_share_one_level() {
        let a = part("a", Shape::Circle, Mode::Add, 0.1, -0.1);
        let mut b = part("b", Shape::Circle, Mode::Add, -0.1, -0.3);
        b.transform = Transform {
            w: 0.5,
            h: 0.5,
            ..Transform::default()
        };
        let ev = evaluate(&design(vec![a, b])).unwrap();
        assert_eq!(ev.levels, vec![-0.3, -0.1, 0.1]);
        assert_eq!(ev.intervals.len(), 2);
    }

    #[test]
    fn bad_depths_are_doc_errors() {
        let upside_down = part("a", Shape::Circle, Mode::Add, -0.1, 0.1);
        assert!(matches!(
            evaluate(&design(vec![upside_down])),
            Err(KitError::Doc(_))
        ));
        let too_deep = part("a", Shape::Circle, Mode::Add, 0.1, -3.0);
        assert!(matches!(
            evaluate(&design(vec![too_deep])),
            Err(KitError::Doc(_))
        ));
    }

    #[test]
    fn rim_is_applied_after_normalisation() {
        // A square twice the box with a rim of 0.3: normalised first, the
        // hole sits at 0.7; rimmed first and scaled after, it would sit
        // at 0.85. The v1 editor normalised first, and so does the kit.
        let mut big = part("a", Shape::Square { rounding: 0.0 }, Mode::Add, 0.1, -0.1);
        big.transform = Transform {
            w: 2.0,
            h: 2.0,
            ..Transform::default()
        };
        big.effects.rim = Some(0.3);
        let ev = evaluate(&design(vec![big])).unwrap();
        let solid = &ev.intervals[0].solid;
        assert_eq!(solid[0].len(), 2, "outer plus hole");
        let hole = solid[0][1]
            .iter()
            .map(|p| p[0].abs().max(p[1].abs()))
            .fold(0.0f32, f32::max);
        assert!((hole - 0.7).abs() < 1e-3, "hole extent {hole}");
        assert!((extent(solid) - 1.0).abs() < 1e-3);
    }
}
