//! Depth levels and the solid and glass regions between them. Groups
//! fold first (a group's cuts only touch its own adds), then everything
//! combines per interval: union of adds minus union of cuts, with glass
//! as a second region that yields to solid wherever both overlap.

use crate::kit::doc::*;
use crate::kit::geom::{boolean, extent, is_empty, snap, OverlayRule, Shapes};
use crate::kit::resolve::{finish_part, resolve_part, ResolvedPart};
use crate::kit::{KitError, Z_MAX, Z_MIN};
use std::borrow::Cow;
use std::collections::HashSet;

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

/// The regions of the parts with this mode (and material, when given).
fn gather<'a>(
    parts: &[&'a ResolvedPart],
    mode: Mode,
    material: Option<Material>,
) -> Vec<&'a Shapes> {
    parts
        .iter()
        .filter(|p| p.mode == mode && material.is_none_or(|m| p.material == m))
        .map(|p| &p.shapes)
        .collect()
}

/// A part belongs to at most one group: its cuts would otherwise fold
/// into two groups at once and mean two things.
fn check_groups(groups: &[Group]) -> Result<(), KitError> {
    let mut claimed: HashSet<&str> = HashSet::new();
    for g in groups {
        let mut own: HashSet<&str> = HashSet::new();
        for id in &g.parts {
            if own.insert(id.as_str()) && !claimed.insert(id.as_str()) {
                return Err(KitError::Doc(format!(
                    "part {id} is in more than one group"
                )));
            }
        }
    }
    Ok(())
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
    check_groups(&design.groups)?;
    let in_group: HashSet<&str> = design
        .groups
        .iter()
        .flat_map(|g| g.parts.iter().map(String::as_str))
        .collect();
    let in_cut_group: HashSet<&str> = design
        .groups
        .iter()
        .filter(|g| g.mode == Mode::Cut)
        .flat_map(|g| g.parts.iter().map(String::as_str))
        .collect();
    // A remover only takes material away: a Cut part, or any member of a
    // Cut-mode group.
    let removes = |p: &ResolvedPart| p.mode == Mode::Cut || in_cut_group.contains(p.id.as_str());

    let mut parts: Vec<ResolvedPart> = design
        .parts
        .iter()
        .filter(|p| p.visible)
        .map(resolve_part)
        .collect::<Result<_, _>>()?;
    parts.retain(|p| !p.shapes.is_empty());

    // The silhouette is what the adds cover; removers never count. The
    // extent of a union is the largest extent of its inputs, so no union
    // is needed to find it.
    let m = parts
        .iter()
        .filter(|p| !removes(p))
        .map(|p| extent(&p.shapes))
        .fold(0.0f32, f32::max);
    if m < 1e-6 {
        return Err(KitError::Empty);
    }
    let scale = if (m - 1.0).abs() > 1e-4 { 1.0 / m } else { 1.0 };
    if scale > 1.0 {
        // Scaling up would push a remover that reaches far past the
        // silhouette beyond the lattice range, failing a valid design.
        // Outside the silhouette square a remover takes nothing away, so
        // it is clipped to the square first.
        let square: Shapes = vec![vec![vec![[-m, -m], [m, -m], [m, m], [-m, m]]]];
        for p in parts.iter_mut().filter(|p| removes(p)) {
            p.shapes = boolean(&p.shapes, &square, OverlayRule::Intersect)?;
        }
    }
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
            // The multiply leaves the lattice; an interval with a single
            // contributor never meets an overlay, so snap it here to keep
            // every region on the same grid.
            p.shapes = snap(&p.shapes)?;
        }
    }
    // Rounding and rims are note-unit finishes, so they run on each part's
    // normalised, merged region (the v1 editor normalised first too). A
    // finish never makes a part larger (a rim keeps the outer loop, a
    // rounding pulls convex corners inward), so the box is never exceeded.
    for p in &mut parts {
        if p.round.is_some() || p.rim.is_some() {
            p.shapes = finish_part(&p.shapes, p.round, p.rim)?;
        }
    }
    parts.retain(|p| !p.shapes.is_empty());

    // Level set. NaN depths are rejected above, so the order is total.
    let mut levels: Vec<f32> = parts
        .iter()
        .flat_map(|p| [p.depth.back, p.depth.front])
        .collect();
    levels.sort_by(f32::total_cmp);
    levels.dedup_by(|a, b| (*a - *b).abs() < LEVEL_EPS);

    let mut intervals = Vec::with_capacity(levels.len().saturating_sub(1));
    let mut any_region = false;
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
            // The group's own cuts bite its own adds, material by material
            // (a cut defaults to solid and must not turn glass opaque). An
            // Add-mode group then adds both results; a Cut-mode group
            // removes the union of its adds, whatever their material.
            let cuts = union_all(&gather(&members, Mode::Cut, None))?;
            let kinds: &[Option<Material>] = match g.mode {
                Mode::Add => &[Some(Material::Solid), Some(Material::Glass)],
                Mode::Cut => &[None],
            };
            for material in kinds {
                let adds = union_all(&gather(&members, Mode::Add, *material))?;
                let folded = subtract(&adds, &cuts)?;
                if is_empty(&folded) {
                    continue;
                }
                contributions.push(Contribution {
                    mode: g.mode,
                    material: material.unwrap_or(Material::Solid),
                    shapes: Cow::Owned(folded),
                });
            }
        }
        for p in covering
            .iter()
            .filter(|p| !in_group.contains(p.id.as_str()))
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
        any_region |= !is_empty(&solid) || !is_empty(&glass);
        intervals.push(Interval {
            z0,
            z1,
            solid,
            glass,
        });
    }
    if !any_region {
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

    #[test]
    fn glass_keeps_its_material_inside_a_group() {
        let mut lens = part("lens", Shape::Circle, Mode::Add, 0.1, -0.1);
        lens.material = Material::Glass;
        let mut hole = part(
            "hole",
            Shape::Square { rounding: 0.0 },
            Mode::Cut,
            0.1,
            -0.1,
        );
        hole.transform = Transform {
            w: 0.5,
            h: 0.5,
            ..Transform::default()
        };
        let mut d = design(vec![lens, hole]);
        d.groups = vec![Group {
            id: "g".into(),
            name: "g".into(),
            parts: vec!["lens".into(), "hole".into()],
            mode: Mode::Add,
        }];
        let ev = evaluate(&d).unwrap();
        // The cut defaults to Solid, which must not turn the lens solid.
        assert!(is_empty(&ev.intervals[0].solid));
        assert_eq!(ev.intervals[0].glass.len(), 1);
        assert_eq!(ev.intervals[0].glass[0].len(), 2, "the lens keeps its hole");
    }

    #[test]
    fn a_cut_group_removes_the_union_of_its_adds() {
        let plate = part(
            "plate",
            Shape::Square { rounding: 0.0 },
            Mode::Add,
            0.1,
            -0.1,
        );
        let mut a = part("a", Shape::Circle, Mode::Add, 0.1, -0.1);
        a.transform = Transform {
            x: -0.5,
            w: 0.3,
            h: 0.3,
            ..Transform::default()
        };
        let mut b = part("b", Shape::Circle, Mode::Add, 0.1, -0.1);
        b.transform = Transform {
            x: 0.9,
            w: 0.3,
            h: 0.3,
            ..Transform::default()
        };
        let mut d = design(vec![plate, a, b]);
        d.groups = vec![Group {
            id: "g".into(),
            name: "g".into(),
            parts: vec!["a".into(), "b".into()],
            mode: Mode::Cut,
        }];
        let ev = evaluate(&d).unwrap();
        // One hole inside, one bite at the edge; the discs add nothing.
        assert_eq!(ev.intervals[0].solid.len(), 1);
        assert_eq!(ev.intervals[0].solid[0].len(), 2);
        // The discs do not count toward the silhouette: the plate stays the box.
        assert!((ev.scale - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_part_in_two_groups_is_a_doc_error() {
        let base = part("base", Shape::Circle, Mode::Add, 0.1, -0.1);
        let mut d = design(vec![base]);
        d.groups = vec![
            Group {
                id: "g1".into(),
                name: "g1".into(),
                parts: vec!["base".into()],
                mode: Mode::Add,
            },
            Group {
                id: "g2".into(),
                name: "g2".into(),
                parts: vec!["base".into()],
                mode: Mode::Cut,
            },
        ];
        assert!(matches!(evaluate(&d), Err(KitError::Doc(_))));
    }

    #[test]
    fn a_far_cut_does_not_fail_a_small_note() {
        // The disc has half-extent 0.01, so the scale is 100; the cut's far
        // edge would land at 100, past KIT_RANGE, without the clip.
        let mut disc = part("disc", Shape::Circle, Mode::Add, 0.1, -0.1);
        disc.transform = Transform {
            w: 0.01,
            h: 0.01,
            ..Transform::default()
        };
        let mut cut = part("cut", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        cut.transform = Transform {
            x: 0.5,
            w: 0.5,
            h: 0.5,
            ..Transform::default()
        };
        let ev = evaluate(&design(vec![disc, cut])).unwrap();
        assert!((ev.scale - 100.0).abs() < 0.1);
        let solid = &ev.intervals[0].solid;
        assert_eq!(solid.len(), 1);
        // The cut covers x >= 0: the left half-disc remains and reaches -1.
        assert!(solid[0][0].iter().all(|p| p[0] <= 1e-3));
        assert!((extent(solid) - 1.0).abs() < 2e-3);
    }

    #[test]
    fn glass_survives_where_it_does_not_overlap_solid() {
        let mut solid = part(
            "solid",
            Shape::Square { rounding: 0.0 },
            Mode::Add,
            0.1,
            -0.1,
        );
        solid.transform = Transform {
            x: -0.5,
            w: 0.5,
            h: 0.5,
            ..Transform::default()
        };
        let mut glass = part(
            "glass",
            Shape::Square { rounding: 0.0 },
            Mode::Add,
            0.1,
            -0.1,
        );
        glass.material = Material::Glass;
        glass.transform = Transform {
            x: 0.5,
            w: 0.5,
            h: 0.5,
            ..Transform::default()
        };
        let ev = evaluate(&design(vec![solid, glass])).unwrap();
        let iv = &ev.intervals[0];
        // Side by side, both survive and together fill the box.
        assert_eq!(iv.solid.len(), 1);
        assert_eq!(iv.glass.len(), 1);
        assert!(iv.solid[0][0].iter().all(|p| p[0] <= 1e-4));
        assert!(iv.glass[0][0].iter().all(|p| p[0] >= -1e-4));
        assert!((extent(&iv.glass) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_cut_stays_inside_its_band() {
        let plate = part(
            "plate",
            Shape::Square { rounding: 0.0 },
            Mode::Add,
            0.1,
            -0.3,
        );
        let mut hole = part("hole", Shape::Circle, Mode::Cut, 0.1, -0.1);
        hole.transform = Transform {
            w: 0.4,
            h: 0.4,
            ..Transform::default()
        };
        let ev = evaluate(&design(vec![plate, hole])).unwrap();
        assert_eq!(ev.levels.len(), 3);
        // Lower band: the plate is whole. Upper band: the plate has the hole.
        assert_eq!(ev.intervals[0].solid[0].len(), 1);
        assert_eq!(ev.intervals[1].solid[0].len(), 2);
    }
}
