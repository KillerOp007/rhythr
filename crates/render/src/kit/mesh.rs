//! From regions per depth interval to a watertight mesh. The plane is
//! subdivided into faces of constant membership across all intervals,
//! so every cap is a face triangulated once and lifted to a level, and
//! every wall sits on a face edge whose two sides disagree about an
//! interval. All faces come from the same fixed-scale kernel, so they
//! share boundary vertices bit for bit and no T-junction can exist.

use crate::kit::geom::{boolean, is_empty, Contour, OverlayRule, Shapes};
use crate::kit::levels::Evaluation;
use crate::kit::validate::{validate, KitMesh};
use crate::kit::KitError;
use i_triangle::float::unchecked::UncheckedTriangulatable;
use std::collections::HashMap;

/// Outer contour first, then holes.
type Face = Vec<Contour>;

struct FaceInfo {
    face: Face,
    /// Bit k set: this face is material in interval k.
    members: u64,
    /// Triangles as indices into the face's own points (outer then holes).
    tris: Vec<[u32; 3]>,
}

/// Triangulates one face. Returns the point count and triangles whose
/// indices address the face's points in contour order; no points are
/// added or dropped (unchecked triangulation does not simplify). A
/// triangulation that changed the point set is an error in every build,
/// because its indices would address the wrong points.
///
/// The triangulator sorts its vertices into sweep order, so the indices
/// it returns address that order, not the contour order. Each of its
/// integer points is therefore matched back to the face's own point with
/// the same integer coordinates (the adapter is the one the triangulator
/// converted with), which keeps every position bit for bit.
pub(crate) fn triangulate_face(face: &Face) -> Result<(usize, Vec<[u32; 3]>), KitError> {
    let changed = || KitError::Invalid("triangulation changed the point set".into());
    let count: usize = face.iter().map(|c| c.len()).sum();
    let raw = face.unchecked_triangulate();
    let mut slot = HashMap::with_capacity(count);
    for (i, p) in face.iter().flatten().enumerate() {
        slot.entry(raw.adapter.float_to_int(p)).or_insert(i as u32);
    }
    let sweep = raw.raw.points();
    if sweep.len() != count {
        return Err(changed());
    }
    let own: Vec<u32> = sweep
        .iter()
        .map(|q| slot.get(q).copied().ok_or_else(changed))
        .collect::<Result<_, _>>()?;
    let indices = raw.triangle_indices::<u32>();
    if indices.iter().any(|&i| i as usize >= count) {
        return Err(changed());
    }
    let tris = indices
        .chunks_exact(3)
        .map(|t| [own[t[0] as usize], own[t[1] as usize], own[t[2] as usize]])
        .collect();
    Ok((count, tris))
}

fn subdivide(regions: &[Shapes]) -> Result<Vec<FaceInfo>, KitError> {
    let mut all: Shapes = Vec::new();
    for r in regions {
        if is_empty(r) {
            continue;
        }
        all = if all.is_empty() {
            r.clone()
        } else {
            boolean(&all, r, OverlayRule::Union)?
        };
    }
    let mut faces: Vec<(Face, u64)> = all.into_iter().map(|f| (f, 0u64)).collect();
    for (k, r) in regions.iter().enumerate() {
        if is_empty(r) {
            continue;
        }
        let mut next: Vec<(Face, u64)> = Vec::new();
        for (face, members) in faces {
            let one: Shapes = vec![face];
            for f in boolean(&one, r, OverlayRule::Intersect)? {
                next.push((f, members | (1u64 << k)));
            }
            for f in boolean(&one, r, OverlayRule::Difference)? {
                next.push((f, members));
            }
        }
        faces = next;
    }
    let mut infos = Vec::with_capacity(faces.len());
    for (face, members) in faces {
        if members == 0 {
            continue;
        }
        let (_, tris) = triangulate_face(&face)?;
        infos.push(FaceInfo {
            face,
            members,
            tris,
        });
    }
    Ok(infos)
}

/// One vertex pool for the whole mesh, keyed by float bits: the same
/// coordinates always give the same index. The map is only ever read by
/// key, so its iteration order cannot reach the output.
struct Pool {
    positions: Vec<[f32; 3]>,
    index: HashMap<[u32; 3], u32>,
}

impl Pool {
    fn get(&mut self, p: [f32; 3]) -> u32 {
        let key = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
        *self.index.entry(key).or_insert_with(|| {
            self.positions.push(p);
            (self.positions.len() - 1) as u32
        })
    }
}

fn emit_material(
    regions: &[Shapes],
    levels: &[f32],
    pool: &mut Pool,
    out: &mut Vec<u32>,
) -> Result<(), KitError> {
    if regions.len() > 64 {
        return Err(KitError::Invalid(format!(
            "{} depth intervals, at most 64",
            regions.len()
        )));
    }
    let faces = subdivide(regions)?;
    let n = regions.len();
    // Caps: per level, per face, where membership changes across the level.
    for info in &faces {
        let pts: Vec<[f32; 2]> = info.face.iter().flatten().copied().collect();
        for (li, &z) in levels.iter().enumerate() {
            let below = li > 0 && info.members & (1u64 << (li - 1)) != 0;
            let above = li < n && info.members & (1u64 << li) != 0;
            if below == above {
                continue;
            }
            for t in &info.tris {
                let a = pool.get([pts[t[0] as usize][0], pts[t[0] as usize][1], z]);
                let b = pool.get([pts[t[1] as usize][0], pts[t[1] as usize][1], z]);
                let c = pool.get([pts[t[2] as usize][0], pts[t[2] as usize][1], z]);
                if below {
                    out.extend_from_slice(&[a, b, c]); // front cap faces +z
                } else {
                    out.extend_from_slice(&[a, c, b]); // back cap faces -z
                }
            }
        }
    }
    // Walls: directed edges with the face on the left; the neighbour is
    // whoever owns the reverse edge.
    let mut owner: HashMap<([u32; 2], [u32; 2]), usize> = HashMap::new();
    let key = |p: [f32; 2]| [p[0].to_bits(), p[1].to_bits()];
    for (fi, info) in faces.iter().enumerate() {
        for c in &info.face {
            for i in 0..c.len() {
                owner.insert((key(c[i]), key(c[(i + 1) % c.len()])), fi);
            }
        }
    }
    for info in &faces {
        for c in &info.face {
            for i in 0..c.len() {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                let across = owner
                    .get(&(key(b), key(a)))
                    .map(|&j| faces[j].members)
                    .unwrap_or(0);
                for k in 0..n {
                    let mine = info.members & (1u64 << k) != 0;
                    let theirs = across & (1u64 << k) != 0;
                    if mine && !theirs {
                        let (z0, z1) = (levels[k], levels[k + 1]);
                        let fa = pool.get([a[0], a[1], z1]);
                        let fb = pool.get([b[0], b[1], z1]);
                        let ba = pool.get([a[0], a[1], z0]);
                        let bb = pool.get([b[0], b[1], z0]);
                        out.extend_from_slice(&[fa, ba, bb]);
                        out.extend_from_slice(&[fa, bb, fb]);
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn build_mesh(ev: &Evaluation) -> Result<KitMesh, KitError> {
    let mut pool = Pool {
        positions: Vec::new(),
        index: HashMap::new(),
    };
    let mut indices: Vec<u32> = Vec::new();
    let solid_regions: Vec<Shapes> = ev.intervals.iter().map(|i| i.solid.clone()).collect();
    emit_material(&solid_regions, &ev.levels, &mut pool, &mut indices)?;
    let solid_end = indices.len();
    let glass_regions: Vec<Shapes> = ev.intervals.iter().map(|i| i.glass.clone()).collect();
    emit_material(&glass_regions, &ev.levels, &mut pool, &mut indices)?;
    let mesh = KitMesh {
        positions: pool.positions,
        solid: 0..solid_end,
        glass: solid_end..indices.len(),
        indices,
    };
    validate(&mesh)?;
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::doc::*;
    use crate::kit::geom::signed_area;
    use crate::kit::levels::evaluate;
    use crate::kit::validate::validate;

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

    fn mesh_of(parts: Vec<Part>) -> KitMesh {
        build_mesh(&evaluate(&design(parts)).unwrap()).unwrap()
    }

    #[test]
    fn triangulation_keeps_the_input_points() {
        // The whole cap scheme relies on triangle indices pointing at the
        // face's own points, in order, with nothing added or dropped.
        let face: Vec<Vec<[f32; 2]>> = vec![
            vec![
                [-1.0, -1.0],
                [1.0, -1.0],
                [1.0, 1.0],
                [0.0, 0.5],
                [-1.0, 1.0],
            ],
            vec![[-0.5, -0.5], [-0.5, 0.0], [0.0, 0.0], [0.0, -0.5]],
        ];
        let (count, tris) = triangulate_face(&face).unwrap();
        let (n, holes) = (9, 1);
        assert_eq!(count, n);
        assert!(tris.iter().flatten().all(|&i| (i as usize) < n));
        assert_eq!(tris.len(), n + 2 * holes - 2, "n + 2h - 2 triangles");
        // The indices must address the face's own points, in contour
        // order: read there, every triangle is counterclockwise and
        // together they tile exactly the face (outer area minus hole).
        let pts: Vec<[f32; 2]> = face.iter().flatten().copied().collect();
        let cross = |t: &[u32; 3]| {
            let (a, b, c) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        };
        assert!(tris.iter().all(|t| cross(t) > 0.0), "{tris:?}");
        let tiled: f32 = tris.iter().map(|t| cross(t) / 2.0).sum();
        let area: f32 = face.iter().map(|c| signed_area(c)).sum();
        assert!((tiled - area).abs() < 1e-5, "tiled {tiled}, face {area}");
    }

    #[test]
    fn a_disc_is_a_closed_slab() {
        let m = mesh_of(vec![part("a", Shape::Circle, Mode::Add, 0.1, -0.1)]);
        let r = validate(&m).unwrap();
        assert!(r.triangles > 500);
        assert!(m.glass.is_empty());
        let zs: Vec<f32> = m.positions.iter().map(|p| p[2]).collect();
        assert!(zs
            .iter()
            .all(|z| (*z - 0.1).abs() < 1e-6 || (*z + 0.1).abs() < 1e-6));
    }

    #[test]
    fn ring_with_recessed_plate_is_watertight() {
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
        let m = mesh_of(vec![ring, plate]);
        validate(&m).unwrap();
        let levels: std::collections::BTreeSet<i32> = m
            .positions
            .iter()
            .map(|p| (p[2] * 1000.0).round() as i32)
            .collect();
        assert_eq!(
            levels.into_iter().collect::<Vec<_>>(),
            vec![-600, -400, -100, 100]
        );
    }

    #[test]
    fn crossing_bands_stay_watertight() {
        // A cut bar through a disc at a shallower band than the disc: the
        // bar's boundary crosses the disc's region boundary at a level
        // that is not the disc's, which is where T-junctions would arise.
        let disc = part("d", Shape::Circle, Mode::Add, 0.1, -0.3);
        let mut bar = part("b", Shape::Square { rounding: 0.0 }, Mode::Cut, 0.1, -0.1);
        bar.transform = Transform {
            w: 1.5,
            h: 0.15,
            rotation: 20.0,
            ..Transform::default()
        };
        let m = mesh_of(vec![disc, bar]);
        validate(&m).unwrap();
    }

    #[test]
    fn stacked_same_footprint_parts_fuse_without_internal_faces() {
        let a = part("a", Shape::Circle, Mode::Add, 0.1, -0.1);
        let b = part("b", Shape::Circle, Mode::Add, -0.1, -0.3);
        let fused = mesh_of(vec![a.clone(), b]);
        validate(&fused).unwrap();
        let single = mesh_of(vec![part("c", Shape::Circle, Mode::Add, 0.1, -0.3)]);
        // A cap triangle has all three vertices at one z, a wall triangle
        // never does. The outer caps match the single slab's, and the
        // shared level has none.
        let caps_at = |m: &KitMesh, z: f32| {
            m.indices
                .chunks_exact(3)
                .filter(|t| {
                    t.iter()
                        .all(|&i| (m.positions[i as usize][2] - z).abs() < 1e-6)
                })
                .count()
        };
        assert_eq!(caps_at(&fused, 0.1), caps_at(&single, 0.1));
        assert_eq!(caps_at(&fused, -0.3), caps_at(&single, -0.3));
        assert_eq!(caps_at(&fused, -0.1), 0);
    }

    #[test]
    fn glass_gets_its_own_closed_range() {
        let ring = part(
            "ring",
            Shape::Ring { thickness: 0.28 },
            Mode::Add,
            0.1,
            -0.1,
        );
        let mut inlay = part("g", Shape::Circle, Mode::Add, 0.1, -0.1);
        inlay.material = Material::Glass;
        inlay.transform = Transform {
            w: 0.5,
            h: 0.5,
            ..Transform::default()
        };
        let m = mesh_of(vec![ring, inlay]);
        validate(&m).unwrap();
        assert!(!m.glass.is_empty());
        assert!(!m.solid.is_empty());
    }

    #[test]
    fn same_design_gives_the_same_mesh_twice() {
        let ring = part(
            "ring",
            Shape::Ring { thickness: 0.28 },
            Mode::Add,
            0.1,
            -0.1,
        );
        let mut plate = part(
            "plate",
            Shape::Star {
                points: 5,
                inner: 0.5,
            },
            Mode::Add,
            -0.4,
            -0.6,
        );
        plate.transform = Transform {
            w: 0.55,
            h: 0.55,
            rotation: 13.0,
            ..Transform::default()
        };
        let d = design(vec![ring, plate]);
        let m1 = build_mesh(&evaluate(&d).unwrap()).unwrap();
        let m2 = build_mesh(&evaluate(&d).unwrap()).unwrap();
        assert_eq!(m1, m2);
    }

    #[test]
    fn caps_face_outward() {
        // Front caps at +z wind counterclockwise seen from +z, back caps
        // clockwise; the walls then follow from the validator's edge pairing.
        let m = mesh_of(vec![part("a", Shape::Circle, Mode::Add, 0.1, -0.1)]);
        let area = |t: &[u32]| {
            let p = |i: u32| m.positions[i as usize];
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        };
        let mut front = 0;
        let mut back = 0;
        for t in m.indices.chunks_exact(3) {
            let zs: Vec<f32> = t.iter().map(|&i| m.positions[i as usize][2]).collect();
            if zs.iter().all(|z| (*z - 0.1).abs() < 1e-6) {
                assert!(area(t) > 0.0, "front cap triangle {t:?} winds clockwise");
                front += 1;
            } else if zs.iter().all(|z| (*z + 0.1).abs() < 1e-6) {
                assert!(
                    area(t) < 0.0,
                    "back cap triangle {t:?} winds counterclockwise"
                );
                back += 1;
            }
        }
        assert!(front > 200 && back > 200);
    }

    #[test]
    fn more_than_64_intervals_is_a_clear_error() {
        let regions: Vec<Shapes> = vec![Vec::new(); 65];
        let levels = vec![0.0f32; 66];
        let mut pool = Pool {
            positions: Vec::new(),
            index: HashMap::new(),
        };
        let mut out = Vec::new();
        let e = emit_material(&regions, &levels, &mut pool, &mut out).unwrap_err();
        assert!(
            matches!(e, KitError::Invalid(ref s) if s.contains("64")),
            "{e}"
        );
    }
}
