//! Minimal Wavefront `.obj` loader for the note meshes.
//!
//! Only positions and triangulated faces are needed — the note look comes
//! from the fragment shader (glow/edge), not from normals or UVs. Faces
//! with more than three vertices are fan-triangulated. Negative (relative)
//! indices are supported, as Blender emits them.

use crate::Error;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
}

#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn from_obj_str(src: &str) -> Result<Mesh, Error> {
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        // Resolves an .obj vertex reference ("v/vt/vn"), 1-based or negative.
        let resolve = |token: &str, count: usize| -> Result<u32, Error> {
            let idx_str = token.split('/').next().unwrap_or("");
            let i: i64 = idx_str.parse().map_err(|_| Error::BadObj)?;
            let resolved = if i < 0 { count as i64 + i } else { i - 1 };
            if resolved < 0 || resolved as usize >= count {
                return Err(Error::BadObj);
            }
            Ok(resolved as u32)
        };

        for line in src.lines() {
            let line = line.trim();
            let mut it = line.split_whitespace();
            match it.next() {
                Some("v") => {
                    let coords: Vec<f32> = it.filter_map(|t| t.parse().ok()).collect();
                    if coords.len() < 3 {
                        return Err(Error::BadObj);
                    }
                    positions.push([coords[0], coords[1], coords[2]]);
                }
                Some("f") => {
                    let verts: Vec<u32> = it
                        .map(|t| resolve(t, positions.len()))
                        .collect::<Result<_, _>>()?;
                    if verts.len() < 3 {
                        return Err(Error::BadObj);
                    }
                    // Fan-triangulate the polygon.
                    for k in 1..verts.len() - 1 {
                        indices.push(verts[0]);
                        indices.push(verts[k]);
                        indices.push(verts[k + 1]);
                    }
                }
                _ => {}
            }
        }

        if positions.is_empty() || indices.is_empty() {
            return Err(Error::BadObj);
        }
        let vertices = positions.into_iter().map(|pos| Vertex { pos }).collect();
        Ok(Mesh { vertices, indices })
    }

    /// Axis-aligned bounding box (min, max) over all vertices.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for v in &self.vertices {
            for a in 0..3 {
                min[a] = min[a].min(v.pos[a]);
                max[a] = max[a].max(v.pos[a]);
            }
        }
        (min, max)
    }

    /// The larger of the x/y half-extents (the note meshes are centred on
    /// the origin; different skins span ±0.8 or ±1.0).
    pub fn xy_half_extent(&self) -> f32 {
        let (min, max) = self.bounds();
        max[0]
            .abs()
            .max(min[0].abs())
            .max(max[1].abs())
            .max(min[1].abs())
    }

    /// Rescales x/y so the mesh spans exactly ±1 (z untouched), removing the
    /// per-skin size difference so a single world scale applies to any note
    /// mesh and the shader can assume unit-square local coordinates.
    pub fn normalize_xy(&mut self) {
        let h = self.xy_half_extent();
        if h > 0.0 {
            for v in &mut self.vertices {
                v.pos[0] /= h;
                v.pos[1] /= h;
            }
        }
    }

    /// A flat unit quad spanning ±1 in x/y at z=0 (two triangles, CCW). The
    /// note/cursor/border shapes are drawn onto it in the fragment shader,
    /// so no per-skin mesh file is needed.
    pub fn quad() -> Mesh {
        Mesh {
            vertices: vec![
                Vertex {
                    pos: [-1.0, -1.0, 0.0],
                },
                Vertex {
                    pos: [1.0, -1.0, 0.0],
                },
                Vertex {
                    pos: [1.0, 1.0, 0.0],
                },
                Vertex {
                    pos: [-1.0, 1.0, 0.0],
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_quad_into_two_triangles() {
        let obj = "v -1 -1 0\nv 1 -1 0\nv 1 1 0\nv -1 1 0\nf 1 2 3 4\n";
        let m = Mesh::from_obj_str(obj).unwrap();
        assert_eq!(m.vertices.len(), 4);
        assert_eq!(m.indices.len(), 6); // quad -> 2 triangles
    }

    #[test]
    fn resolves_negative_indices() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n";
        let m = Mesh::from_obj_str(obj).unwrap();
        assert_eq!(m.indices, vec![0, 1, 2]);
    }

    #[test]
    fn ignores_normals_and_uvs_in_face_refs() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1/1/1 2/2/2 3/3/3\n";
        let m = Mesh::from_obj_str(obj).unwrap();
        assert_eq!(m.indices, vec![0, 1, 2]);
    }

    #[test]
    fn empty_or_garbage_errors() {
        assert!(Mesh::from_obj_str("").is_err());
        assert!(Mesh::from_obj_str("# just a comment\n").is_err());
    }
}
