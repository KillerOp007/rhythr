//! The presets: five finished notes in the rim style, each a complete
//! design that opens as plain parts. They live as JSON next to this file
//! so the kernel is their single source and a test keeps them buildable.

use crate::kit::doc::Design;
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub design: Design,
}

/// The five presets in display order; the files are part of the build,
/// so a damaged one is a programming error, not a runtime condition.
pub fn presets() -> Vec<Preset> {
    const FILES: [(&str, &str, &str); 5] = [
        ("target", "Target", include_str!("presets/target.json")),
        (
            "hex-frame",
            "Hex frame",
            include_str!("presets/hex-frame.json"),
        ),
        (
            "crosshair",
            "Crosshair",
            include_str!("presets/crosshair.json"),
        ),
        (
            "diamond-frame",
            "Diamond frame",
            include_str!("presets/diamond-frame.json"),
        ),
        (
            "notched-ring",
            "Notched ring",
            include_str!("presets/notched-ring.json"),
        ),
    ];
    FILES
        .iter()
        .map(|(id, name, text)| Preset {
            id,
            name,
            design: serde_json::from_str(text)
                .unwrap_or_else(|e| panic!("preset {id} does not parse: {e}")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::doc::Material;
    use crate::kit::{levels, mesh};

    #[test]
    fn every_preset_parses_evaluates_and_meshes() {
        let all = presets();
        assert_eq!(
            all.iter().map(|p| p.name).collect::<Vec<_>>(),
            [
                "Target",
                "Hex frame",
                "Crosshair",
                "Diamond frame",
                "Notched ring"
            ]
        );
        assert_eq!(
            all.iter().map(|p| p.id).collect::<Vec<_>>(),
            [
                "target",
                "hex-frame",
                "crosshair",
                "diamond-frame",
                "notched-ring"
            ]
        );
        assert_eq!(
            all.iter().map(|p| p.design.parts.len()).collect::<Vec<_>>(),
            [3, 2, 3, 2, 3]
        );
        for p in &all {
            assert_eq!(p.design.name, p.name);
            assert_eq!(p.design.version, 2);
            assert!(p.design.groups.is_empty());
            assert!(p.design.parts.iter().all(|x| x.visible && !x.locked));
            let ev = levels::evaluate(&p.design).unwrap();
            let m = mesh::build_mesh(&ev).unwrap();
            let n = m.indices.len() / 3;
            assert!(n > 100 && n < 10_000, "{}: {n} triangles", p.name);
        }
    }

    #[test]
    fn every_preset_has_a_rim_part_and_a_glass_part() {
        for p in presets() {
            assert!(
                p.design.parts.iter().any(|x| x.effects.rim.is_some()),
                "{} has no rim part",
                p.name
            );
            assert!(
                p.design.parts.iter().any(|x| x.material == Material::Glass),
                "{} has no glass part",
                p.name
            );
        }
    }
}
