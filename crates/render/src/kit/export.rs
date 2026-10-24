//! The baked output: one .obj with a `usemtl` section per material, the
//! .mtl it references (always bundled, the game crashes without it),
//! the colorset, in the archive layout the game's importer accepts, and
//! the design itself as one more entry, so the one file is both the skin
//! for the game and the editable document the editor opens again.

use crate::kit::doc::Design;
use crate::kit::levels::evaluate;
use crate::kit::mesh::build_mesh;
use crate::kit::validate::KitMesh;
use crate::kit::KitError;
use std::fmt::Write as _;

const MATERIAL: &str = "newmtl {name}\nNs 250.000000\nKa 1.000000 1.000000 1.000000\nKd 1.000000 1.000000 1.000000\nKs 0.000000 0.000000 0.000000\nKe 0.000000 0.000000 0.000000\nNi 1.450000\nd 1.000000\nillum 1\n";

/// The one `.mtl` every kit skin bundles. Both materials are always
/// declared, glass or not: every skin carries the same `noteSkin/note.mtl`,
/// so however several skins are imported, no `usemtl Transparent` can be
/// left without its material.
pub fn mtl_text() -> String {
    let mut s = String::from("# rhythr note kit\n");
    s.push_str(&MATERIAL.replace("{name}", "Material.001"));
    s.push('\n');
    // The game ignores `d` and draws every material after the first
    // translucent; the name follows its built-in cube.
    s.push_str(&MATERIAL.replace("{name}", "Transparent"));
    s
}

/// Longest accepted skin name, in characters.
const MAX_NAME_CHARS: usize = 64;

/// The archive entry that carries the editable design. The game ignores
/// entries it does not know (verified 2026-10-24 with a probe skin), and
/// rhythr's own reader only looks under the skin folders, so the entry
/// rides along unseen until an editor opens the file.
pub const DESIGN_ENTRY: &str = "rhythr/design.json";

/// The design text embedded in a skin archive, `None` when the archive is
/// a plain game skin (made by the game or by another tool) and carries
/// none. The text is returned as stored: the old designer embeds its own
/// document format, so the caller decides what it is.
pub fn embedded_design(bytes: &[u8]) -> Result<Option<String>, KitError> {
    let mut zr = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| KitError::Invalid("not a skin archive".into()))?;
    let mut entry = match zr.by_name(DESIGN_ENTRY) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(_) => return Err(KitError::Invalid("not a skin archive".into())),
    };
    let mut text = String::new();
    std::io::Read::read_to_string(&mut entry, &mut text)
        .map_err(|_| KitError::Invalid("the embedded design is not readable text".into()))?;
    Ok(Some(text))
}

/// The name reaches the config JSON, two archive entry paths and the obj's
/// `o` line unescaped, so only characters that are inert in all of them
/// pass: a quote, a slash, a newline or `..` would break the archive or
/// leave the skin folder. A space at either end does not survive
/// everywhere (Windows drops a trailing space from file names, obj readers
/// trim the `o` line), and a blank name names nothing.
fn check_name(name: &str) -> Result<(), KitError> {
    let inert = |c: char| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-');
    if name.trim().is_empty()
        || name.trim() != name
        || name.chars().count() > MAX_NAME_CHARS
        || !name.chars().all(inert)
    {
        return Err(KitError::Invalid(format!(
            "skin name {name:?} must be letters, digits, spaces, _ or -, with no space at either end"
        )));
    }
    Ok(())
}

/// Writes `mesh` as one object named `name`. The name is written
/// unescaped: callers must pass one that `export_rhs` accepts (ASCII
/// letters, digits, spaces, `_` and `-`, 1 to 64 characters, no space at
/// either end).
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

/// Bakes `design` into a `.rhs` skin archive named `name`, with the design
/// itself embedded under [`DESIGN_ENTRY`] so the file opens again in the
/// editor. The name is checked first and must be 1 to 64 ASCII letters,
/// digits, spaces, `_` or `-`, with no space at either end; anything else
/// is `KitError::Invalid` and nothing is built.
pub fn export_rhs(design: &Design, name: &str) -> Result<Vec<u8>, KitError> {
    use std::io::Write;
    check_name(name)?;
    let design_json =
        serde_json::to_string(design).map_err(|e| KitError::Invalid(format!("design: {e}")))?;
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
    zw.write_all(mtl_text().as_bytes()).map_err(io_err)?;
    if !colorset.is_empty() {
        zw.start_file(format!("colorSet/{name}.txt"), opts)
            .map_err(zip_err)?;
        zw.write_all(colorset.as_bytes()).map_err(io_err)?;
    }
    zw.start_file(DESIGN_ENTRY, opts).map_err(zip_err)?;
    zw.write_all(design_json.as_bytes()).map_err(io_err)?;
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
        let mtl = mtl_text();
        assert!(mtl.contains("newmtl Material.001"));
        assert!(mtl.contains("newmtl Transparent"));
        assert!(mtl.find("Material.001") < mtl.find("Transparent"));
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
    fn the_archive_carries_its_design_and_gives_it_back() {
        let design = glass_design();
        let bytes = export_rhs(&design, "designed").unwrap();
        let text = embedded_design(&bytes).unwrap().expect("design entry");
        let back: Design = serde_json::from_str(&text).unwrap();
        assert_eq!(back, design);
        // The entry sits after the game's own entries and changes nothing
        // for the skin reader.
        let mut zr = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
        let names: Vec<String> = (0..zr.len())
            .map(|i| zr.by_index(i).unwrap().name().to_string())
            .collect();
        assert_eq!(names.last().map(String::as_str), Some(DESIGN_ENTRY));
        assert_eq!(names[0], "config");
        let cfg = crate::config::SkinConfig::from_rhs(&bytes).unwrap();
        assert!(cfg.note_mesh.is_some());
    }

    #[test]
    fn a_plain_game_skin_has_no_design_and_junk_is_refused() {
        use std::io::Write;
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("config", opts).unwrap();
        zw.write_all(b"{}").unwrap();
        let plain = zw.finish().unwrap().into_inner();
        assert_eq!(embedded_design(&plain).unwrap(), None);
        assert!(matches!(
            embedded_design(b"{\"version\":2}"),
            Err(KitError::Invalid(_))
        ));
        assert!(matches!(embedded_design(b""), Err(KitError::Invalid(_))));
    }

    #[test]
    fn an_empty_design_does_not_export() {
        let mut d = Design::default_note();
        d.parts[0].visible = false;
        assert_eq!(export_rhs(&d, "x").unwrap_err(), KitError::Empty);
    }

    #[test]
    fn unsafe_names_are_refused() {
        let long = "x".repeat(65);
        for bad in [
            "",
            " ",
            " a",
            "a ",
            "a\"b",
            "a/b",
            "a\\b",
            "a\nb",
            "..",
            long.as_str(),
        ] {
            assert!(
                matches!(export_rhs(&glass_design(), bad), Err(KitError::Invalid(_))),
                "{bad:?}"
            );
        }
        assert!(export_rhs(&glass_design(), "My Note_2-b").is_ok());
    }

    #[test]
    fn the_archive_always_carries_both_materials() {
        // Every kit skin bundles the same noteSkin/note.mtl, so whichever
        // skin the game imported last, a `usemtl Transparent` finds its
        // material.
        let entries = |bytes: &[u8]| -> Vec<(String, String)> {
            let mut zr = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
            (0..zr.len())
                .map(|i| {
                    let mut f = zr.by_index(i).unwrap();
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut f, &mut s).unwrap();
                    (f.name().to_string(), s)
                })
                .collect()
        };
        let with_glass = entries(&export_rhs(&glass_design(), "g").unwrap());
        let mtl = with_glass
            .iter()
            .find(|(n, _)| n == "noteSkin/note.mtl")
            .expect("mtl bundled");
        assert!(mtl.1.contains("newmtl Transparent"));
        let solid_only = entries(&export_rhs(&Design::default_note(), "s").unwrap());
        let mtl = solid_only
            .iter()
            .find(|(n, _)| n == "noteSkin/note.mtl")
            .expect("mtl bundled");
        assert!(mtl.1.contains("newmtl Material.001"));
        assert!(mtl.1.contains("newmtl Transparent"));
        let obj = solid_only
            .iter()
            .find(|(n, _)| n == "noteSkin/s.obj")
            .unwrap();
        assert!(!obj.1.contains("usemtl Transparent"));
    }

    #[test]
    fn sections_carry_their_own_ranges_in_order() {
        let ev = crate::kit::levels::evaluate(&glass_design()).unwrap();
        let m = crate::kit::mesh::build_mesh(&ev).unwrap();
        let obj = to_obj(&m, "note");
        let solid_at = obj.find("usemtl Material.001\n").unwrap();
        let glass_at = obj.find("usemtl Transparent\n").unwrap();
        assert!(solid_at < glass_at);
        let faces = |s: &str| s.matches("\nf ").count();
        assert_eq!(faces(&obj[solid_at..glass_at]), m.solid.len() / 3);
        assert_eq!(faces(&obj[glass_at..]), m.glass.len() / 3);
    }

    #[test]
    fn a_glass_only_design_has_one_transparent_section() {
        // Pinned, not endorsed: the game decides which material counts as
        // the opaque first one, and Task 12 probes a glass-only note in play.
        let mut d = Design::default_note();
        d.parts[0].material = Material::Glass;
        let ev = crate::kit::levels::evaluate(&d).unwrap();
        let m = crate::kit::mesh::build_mesh(&ev).unwrap();
        let obj = to_obj(&m, "note");
        assert!(!obj.contains("usemtl Material.001"));
        assert_eq!(obj.matches("usemtl Transparent\n").count(), 1);
    }
}
