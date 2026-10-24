//! The baked output: one .obj with a `usemtl` section per material, the
//! .mtl it references (always bundled, the game crashes without it),
//! and the colorset, in the archive layout the game's importer accepts.

use crate::kit::doc::Design;
use crate::kit::levels::evaluate;
use crate::kit::mesh::build_mesh;
use crate::kit::validate::KitMesh;
use crate::kit::KitError;
use std::fmt::Write as _;

const MATERIAL: &str = "newmtl {name}\nNs 250.000000\nKa 1.000000 1.000000 1.000000\nKd 1.000000 1.000000 1.000000\nKs 0.000000 0.000000 0.000000\nKe 0.000000 0.000000 0.000000\nNi 1.450000\nd 1.000000\nillum 1\n";

pub fn mtl_text(with_glass: bool) -> String {
    let mut s = String::from("# rhythr note kit\n");
    s.push_str(&MATERIAL.replace("{name}", "Material.001"));
    if with_glass {
        s.push('\n');
        // The game ignores `d` and draws every material after the first
        // translucent; the name follows its built-in cube.
        s.push_str(&MATERIAL.replace("{name}", "Transparent"));
    }
    s
}

pub fn to_obj(mesh: &KitMesh, name: &str) -> String {
    let mut s = String::with_capacity(mesh.positions.len() * 36 + mesh.indices.len() * 8);
    s.push_str("# rhythr note kit\nmtllib note.mtl\n");
    let _ = writeln!(s, "o {name}");
    for p in &mesh.positions {
        let _ = writeln!(s, "v {:.6} {:.6} {:.6}", p[0], p[1], p[2]);
    }
    for (label, range) in [("Material.001", &mesh.solid), ("Transparent", &mesh.glass)] {
        if range.is_empty() {
            continue;
        }
        let _ = writeln!(s, "usemtl {label}");
        for t in mesh.indices[range.clone()].chunks_exact(3) {
            let _ = writeln!(s, "f {} {} {}", t[0] + 1, t[1] + 1, t[2] + 1);
        }
    }
    s
}

pub fn export_rhs(design: &Design, name: &str) -> Result<Vec<u8>, KitError> {
    use std::io::Write;
    let mesh = build_mesh(&evaluate(design)?)?;
    let obj = to_obj(&mesh, name);
    let colorset: String = design
        .colors
        .iter()
        .filter(|c| {
            let h = c.trim().trim_start_matches('#');
            h.len() == 6 && h.chars().all(|ch| ch.is_ascii_hexdigit())
        })
        .map(|c| format!("#{}\r\n", c.trim().trim_start_matches('#')))
        .collect();
    let config =
        format!(r#"{{"NoteSkin":{{"Value":"{name}.obj"}},"ColorSet":{{"Value":"{name}"}}}}"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let zip_err = |e: zip::result::ZipError| KitError::Invalid(format!("zip: {e}"));
    let io_err = |e: std::io::Error| KitError::Invalid(format!("zip: {e}"));
    zw.start_file("config", opts).map_err(zip_err)?;
    zw.write_all(config.as_bytes()).map_err(io_err)?;
    zw.start_file(format!("noteSkin/{name}.obj"), opts)
        .map_err(zip_err)?;
    zw.write_all(obj.as_bytes()).map_err(io_err)?;
    zw.start_file("noteSkin/note.mtl", opts).map_err(zip_err)?;
    zw.write_all(mtl_text(!mesh.glass.is_empty()).as_bytes())
        .map_err(io_err)?;
    if !colorset.is_empty() {
        zw.start_file(format!("colorSet/{name}.txt"), opts)
            .map_err(zip_err)?;
        zw.write_all(colorset.as_bytes()).map_err(io_err)?;
    }
    Ok(zw.finish().map_err(zip_err)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::doc::*;

    fn glass_design() -> Design {
        let mut d = Design::default_note();
        d.parts[0].shape = Shape::Ring { thickness: 0.28 };
        d.parts.push(Part {
            id: "g".into(),
            name: "inlay".into(),
            visible: true,
            locked: false,
            shape: Shape::Circle,
            transform: Transform {
                w: 0.5,
                h: 0.5,
                ..Transform::default()
            },
            mode: Mode::Add,
            depth: Depth::default(),
            material: Material::Glass,
            effects: Effects::default(),
        });
        d
    }

    #[test]
    fn obj_has_one_object_and_a_section_per_material() {
        let ev = crate::kit::levels::evaluate(&glass_design()).unwrap();
        let m = crate::kit::mesh::build_mesh(&ev).unwrap();
        let obj = to_obj(&m, "note");
        assert_eq!(obj.matches("\no ").count(), 1);
        assert!(obj.contains("mtllib note.mtl\n"));
        assert_eq!(obj.matches("usemtl Material.001\n").count(), 1);
        assert_eq!(obj.matches("usemtl Transparent\n").count(), 1);
        assert_eq!(obj.matches("\nf ").count(), m.indices.len() / 3);
        // rhythr's own loader reads it back with every triangle.
        let parsed = crate::mesh::Mesh::from_obj_str(&obj).unwrap();
        assert_eq!(parsed.indices.len(), m.indices.len());
    }

    #[test]
    fn mtl_names_match_the_game_convention() {
        let solid_only = mtl_text(false);
        assert!(solid_only.contains("newmtl Material.001"));
        assert!(!solid_only.contains("Transparent"));
        assert!(mtl_text(true).contains("newmtl Transparent"));
    }

    #[test]
    fn rhs_pack_loads_back_as_a_skin() {
        let bytes = export_rhs(&glass_design(), "designed").unwrap();
        let cfg = crate::config::SkinConfig::from_rhs(&bytes).unwrap();
        assert!(cfg.note_mesh.is_some());
        assert_eq!(cfg.colorset.len(), 3);
        let obj = String::from_utf8(cfg.note_mesh.unwrap()).unwrap();
        assert!(obj.contains("usemtl Transparent"));
    }

    #[test]
    fn an_empty_design_does_not_export() {
        let mut d = Design::default_note();
        d.parts[0].visible = false;
        assert_eq!(export_rhs(&d, "x").unwrap_err(), KitError::Empty);
    }
}
