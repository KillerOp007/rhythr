//! The export gate. The game crashes on broken skins, so a mesh that is
//! not a closed, consistently oriented surface inside the box never
//! leaves the kernel. Deterministic and dependency-free, which is what
//! lets it double as the oracle for the property tests.

use crate::kit::{KitError, MAX_TRIANGLES, Z_MAX, Z_MIN};
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct KitMesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Index ranges per material; solid first, glass second.
    pub solid: Range<usize>,
    pub glass: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub triangles: usize,
    pub vertices: usize,
}

const BOX_EPS: f32 = 1e-4;
/// Twice the smallest area a real triangle can have here: two lattice
/// steps (1/65536 each) span about 2.3e-10, a lattice step times the
/// smallest level gap (1e-5) about 1.5e-10. Anything below is a fold of
/// coincident positions, not a face.
const MIN_AREA2: f32 = 1e-11;

fn invalid(msg: impl Into<String>) -> KitError {
    KitError::Invalid(msg.into())
}

fn check_range(mesh: &KitMesh, range: &Range<usize>, label: &str) -> Result<(), KitError> {
    if range.start > range.end
        || range.end > mesh.indices.len()
        || !(range.end - range.start).is_multiple_of(3)
    {
        return Err(invalid(format!(
            "{label} range {range:?} does not tile triangles"
        )));
    }
    if range.is_empty() {
        return Ok(());
    }
    // Ordered so the edge named in an error is the same on every run.
    let mut directed: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for t in mesh.indices[range.clone()].chunks_exact(3) {
        let (a, b, c) = (t[0], t[1], t[2]);
        if a == b || b == c || a == c {
            return Err(invalid(format!("{label}: degenerate triangle {t:?}")));
        }
        let (pa, pb, pc) = (
            mesh.positions[a as usize],
            mesh.positions[b as usize],
            mesh.positions[c as usize],
        );
        let u = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let v = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let area2 = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        if area2 < MIN_AREA2 {
            return Err(invalid(format!(
                "{label}: degenerate triangle {t:?} (zero area)"
            )));
        }
        for (x, y) in [(a, b), (b, c), (c, a)] {
            *directed.entry((x, y)).or_insert(0) += 1;
        }
    }
    // Balanced counts rather than "exactly once": two faces of one
    // material that touch at a single vertex at the same depth share the
    // vertical edge at the contact between four walls. That surface is
    // closed and consistently oriented, merely non-manifold, and the game
    // draws it fine, so every directed edge only has to be matched by as
    // many opposite uses.
    for (&(x, y), &n) in &directed {
        let r = directed.get(&(y, x)).copied().unwrap_or(0);
        if r == 0 {
            return Err(invalid(format!(
                "{label}: open edge {x}-{y} (surface not closed)"
            )));
        }
        if r != n {
            return Err(invalid(format!(
                "{label}: edge {x}-{y} used {n} times but {y}-{x} {r} times (inconsistent orientation)"
            )));
        }
    }
    Ok(())
}

pub fn validate(mesh: &KitMesh) -> Result<Report, KitError> {
    let triangles = mesh.indices.len() / 3;
    if !mesh.indices.len().is_multiple_of(3) {
        return Err(invalid("index count is not a multiple of three"));
    }
    if triangles == 0 {
        return Err(invalid("no triangles"));
    }
    if triangles > MAX_TRIANGLES {
        return Err(KitError::Budget(triangles));
    }
    if mesh.solid.start != 0
        || mesh.solid.end != mesh.glass.start
        || mesh.glass.end != mesh.indices.len()
    {
        return Err(invalid(
            "material ranges must cover the index buffer in order",
        ));
    }
    for (i, p) in mesh.positions.iter().enumerate() {
        if !p.iter().all(|v| v.is_finite()) {
            return Err(invalid(format!("vertex {i} is not finite")));
        }
        if p[0].abs() > 1.0 + BOX_EPS || p[1].abs() > 1.0 + BOX_EPS {
            return Err(invalid(format!("vertex {i} leaves the unit box: {p:?}")));
        }
        if p[2] < Z_MIN - BOX_EPS || p[2] > Z_MAX + BOX_EPS {
            return Err(invalid(format!("vertex {i} leaves the depth range: {p:?}")));
        }
    }
    if let Some(&i) = mesh
        .indices
        .iter()
        .find(|&&i| i as usize >= mesh.positions.len())
    {
        return Err(invalid(format!("index {i} out of range")));
    }
    check_range(mesh, &mesh.solid, "solid")?;
    check_range(mesh, &mesh.glass, "glass")?;
    Ok(Report {
        triangles,
        vertices: mesh.positions.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed unit cube, outward-facing, 12 triangles.
    fn cube() -> KitMesh {
        let p = |x: f32, y: f32, z: f32| [x, y, z];
        let positions = vec![
            p(-1.0, -1.0, -0.1),
            p(1.0, -1.0, -0.1),
            p(1.0, 1.0, -0.1),
            p(-1.0, 1.0, -0.1),
            p(-1.0, -1.0, 0.1),
            p(1.0, -1.0, 0.1),
            p(1.0, 1.0, 0.1),
            p(-1.0, 1.0, 0.1),
        ];
        let indices: Vec<u32> = vec![
            4, 5, 6, 4, 6, 7, // front (+z)
            0, 2, 1, 0, 3, 2, // back (-z)
            0, 1, 5, 0, 5, 4, // bottom (-y)
            1, 2, 6, 1, 6, 5, // right (+x)
            2, 3, 7, 2, 7, 6, // top (+y)
            3, 0, 4, 3, 4, 7, // left (-x)
        ];
        let n = indices.len();
        KitMesh {
            positions,
            indices,
            solid: 0..n,
            glass: n..n,
        }
    }

    #[test]
    fn a_closed_cube_passes() {
        let r = validate(&cube()).unwrap();
        assert_eq!(r.triangles, 12);
        assert_eq!(r.vertices, 8);
    }

    #[test]
    fn a_missing_triangle_is_an_open_edge() {
        let mut m = cube();
        m.indices.truncate(m.indices.len() - 3);
        m.solid = 0..m.indices.len();
        m.glass = m.indices.len()..m.indices.len();
        let e = validate(&m).unwrap_err();
        assert!(
            matches!(e, KitError::Invalid(ref s) if s.contains("open")),
            "{e}"
        );
    }

    #[test]
    fn a_flipped_triangle_is_inconsistent() {
        let mut m = cube();
        m.indices.swap(0, 1);
        let e = validate(&m).unwrap_err();
        assert!(matches!(e, KitError::Invalid(_)), "{e}");
    }

    #[test]
    fn degenerate_out_of_box_and_nan_fail() {
        let mut m = cube();
        m.indices[0] = m.indices[1];
        assert!(
            matches!(validate(&m).unwrap_err(), KitError::Invalid(ref s) if s.contains("degenerate"))
        );
        let mut m = cube();
        m.positions[0][0] = -1.5;
        assert!(matches!(validate(&m).unwrap_err(), KitError::Invalid(ref s) if s.contains("box")));
        let mut m = cube();
        m.positions[0][2] = f32::NAN;
        assert!(
            matches!(validate(&m).unwrap_err(), KitError::Invalid(ref s) if s.contains("finite"))
        );
    }

    #[test]
    fn ranges_must_tile_the_index_buffer_in_triangles() {
        let mut m = cube();
        m.solid = 0..4;
        assert!(matches!(validate(&m).unwrap_err(), KitError::Invalid(_)));
    }
}
