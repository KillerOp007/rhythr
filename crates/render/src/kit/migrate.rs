//! Upgrades v1 designer documents (the single-outline editor) into kit
//! designs: one Add part carrying the same shape, effects and symmetry,
//! so nothing anyone saved is lost.

use crate::designer::ControlPoint;
use crate::kit::doc::*;
use crate::kit::KitError;

pub fn from_v1(json: &str) -> Result<Design, KitError> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| KitError::Doc(format!("v1 json: {e}")))?;
    let p = v
        .get("P")
        .ok_or_else(|| KitError::Doc("v1 document has no P".into()))?;
    let f = |key: &str, default: f64| p.get(key).and_then(|x| x.as_f64()).unwrap_or(default) as f32;
    let s = |key: &str, default: &str| {
        p.get(key)
            .and_then(|x| x.as_str())
            .unwrap_or(default)
            .to_string()
    };
    let colors: Vec<String> = v
        .get("colors")
        .and_then(|c| serde_json::from_value(c.clone()).ok())
        .unwrap_or_else(|| vec!["#ffffff".into()]);
    let symmetry = v.get("symmetry").and_then(|x| x.as_u64()).unwrap_or(1) as u32;
    let mirror = v.get("mirror").and_then(|x| x.as_bool()).unwrap_or(false);
    let hollow = p.get("hollow").and_then(|x| x.as_bool()).unwrap_or(false);

    let mut effects = Effects::default();
    if hollow {
        effects.rim = Some(f("thick", 0.22));
    }
    let imported: Option<Vec<Vec<[f32; 2]>>> = v
        .get("imported")
        .filter(|x| x.is_array())
        .and_then(|x| serde_json::from_value(x.clone()).ok());
    let free: Option<Vec<ControlPoint>> = v
        .get("free")
        .filter(|x| x.is_array())
        .and_then(|x| serde_json::from_value(x.clone()).ok());

    // Only the slider outline turned with twist (and the square's 45
    // degree offset) in v1; freeform and imported outlines never did, so
    // their rotation is zero even when the slider fields still hold values.
    let (shape, name, rotation) = if let Some(contours) = imported {
        (
            Shape::Contours {
                contours,
                label: "import".into(),
            },
            "Import",
            0.0,
        )
    } else if let Some(control) = free.filter(|c| c.len() >= 2) {
        if symmetry > 1 {
            effects.repeat = Some(symmetry);
        }
        if mirror {
            effects.mirror = Mirror::X;
        }
        (Shape::Points { control }, "Freeform", 0.0)
    } else {
        let waves = f("waves", 0.0) as u32;
        if waves > 0 {
            effects.waves = Some(Waves {
                count: waves,
                depth: f("waveDepth", 0.25),
                style: match s("waveform", "sine").as_str() {
                    "spike" => WaveStyle::Spike,
                    "pulse" => WaveStyle::Pulse,
                    _ => WaveStyle::Sine,
                },
            });
        }
        // v1 applied the rounding slider to squares as well; the kit's
        // Square carries rounding and its own 45 degree turn, so waves keep
        // the phase v1 gave them (they were evaluated before the turn).
        let shape = match s("shape", "circle").as_str() {
            "square" => Shape::Square {
                rounding: f("round", 0.3),
            },
            "ngon" => Shape::Polygon {
                sides: f("n", 6.0) as u32,
                rounding: f("round", 0.3),
            },
            _ => Shape::Circle,
        };
        // v1 sampled its radius at (angle + twist), which turns the shape
        // by MINUS twist; the kit rotates counterclockwise. Written as a
        // subtraction so a zero twist serialises as 0.0, not -0.0.
        (shape, "Shape", 0.0 - f("twist", 0.0))
    };
    let depth = f("depth", 0.05).clamp(0.005, 0.5);
    Ok(Design {
        version: 2,
        name: "note".into(),
        colors,
        parts: vec![Part {
            id: "p1".into(),
            name: name.into(),
            visible: true,
            locked: false,
            shape,
            transform: Transform {
                rotation,
                ..Transform::default()
            },
            mode: Mode::Add,
            depth: Depth {
                front: depth,
                back: -depth,
            },
            material: Material::Solid,
            effects,
        }],
        groups: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLIDERS: &str = r##"{"P":{"shape":"ngon","n":6,"round":0.3,"hollow":true,
      "thick":0.22,"depth":0.05,"waves":5,"waveDepth":0.25,"waveform":"spike","twist":30},
      "free":null,"symmetry":4,"mirror":false,"colors":["#ff5d6c","#2fd6d0"],"imported":null}"##;

    #[test]
    fn sliders_become_one_polygon_part_with_effects() {
        let d = from_v1(SLIDERS).unwrap();
        assert_eq!(d.version, 2);
        assert_eq!(d.colors, vec!["#ff5d6c", "#2fd6d0"]);
        assert_eq!(d.parts.len(), 1);
        let p = &d.parts[0];
        assert_eq!(
            p.shape,
            Shape::Polygon {
                sides: 6,
                rounding: 0.3
            }
        );
        assert_eq!(p.effects.rim, Some(0.22));
        assert_eq!(
            p.effects.waves,
            Some(Waves {
                count: 5,
                depth: 0.25,
                style: WaveStyle::Spike
            })
        );
        // v1 sampled the radius at angle + twist, which turns the shape
        // clockwise; the kit rotates counterclockwise, hence the sign.
        assert_eq!(p.transform.rotation, -30.0);
        assert_eq!(
            p.depth,
            Depth {
                front: 0.05,
                back: -0.05
            }
        );
    }

    #[test]
    fn a_v1_square_keeps_its_rounding() {
        let json = SLIDERS
            .replace(r#""shape":"ngon""#, r#""shape":"square""#)
            .replace(r#""twist":30"#, r#""twist":0"#);
        let d = from_v1(&json).unwrap();
        let p = &d.parts[0];
        assert_eq!(p.shape, Shape::Square { rounding: 0.3 });
        assert_eq!(p.transform.rotation, 0.0);
        assert!(
            !p.transform.rotation.is_sign_negative(),
            "a zero twist must not serialise as -0.0"
        );
    }

    #[test]
    fn freeform_and_imports_ignore_stale_slider_rotation() {
        // v1 never turned freeform or imported outlines, even when the
        // slider fields still said "square" or carried a twist.
        let json = r##"{"P":{"shape":"square","n":6,"round":0.3,"hollow":false,"thick":0.22,
          "depth":0.05,"waves":0,"waveDepth":0.25,"waveform":"sine","twist":30},
          "free":[{"x":1,"y":0,"smooth":false},{"x":0.5,"y":0.5,"smooth":true}],
          "symmetry":4,"mirror":false,"colors":["#ffffff"],"imported":null}"##;
        assert_eq!(from_v1(json).unwrap().parts[0].transform.rotation, 0.0);
        let json = json.replace(
            r#""imported":null"#,
            r#""imported":[[[-1,-1],[1,-1],[1,1],[-1,1]]]"#,
        );
        assert_eq!(from_v1(&json).unwrap().parts[0].transform.rotation, 0.0);
    }

    #[test]
    fn freeform_keeps_symmetry_and_mirror_as_effects() {
        let json = r##"{"P":{"shape":"circle","n":6,"round":0.3,"hollow":false,"thick":0.22,
          "depth":0.05,"waves":0,"waveDepth":0.25,"waveform":"sine","twist":0},
          "free":[{"x":1,"y":0,"smooth":false},{"x":0.5,"y":0.5,"smooth":true}],
          "symmetry":5,"mirror":true,"colors":["#ffffff"],"imported":null}"##;
        let d = from_v1(json).unwrap();
        let p = &d.parts[0];
        match &p.shape {
            Shape::Points { control } => assert_eq!(control.len(), 2),
            other => panic!("expected points, got {other:?}"),
        }
        assert_eq!(p.effects.repeat, Some(5));
        assert_eq!(p.effects.mirror, Mirror::X);
        assert_eq!(p.effects.rim, None);
    }

    #[test]
    fn imports_win_and_become_contours() {
        let json = r##"{"P":{"shape":"circle","n":6,"round":0.3,"hollow":true,"thick":0.2,
          "depth":0.05,"waves":0,"waveDepth":0.25,"waveform":"sine","twist":0},
          "free":[{"x":1,"y":0,"smooth":true}],"symmetry":4,"mirror":false,
          "colors":["#ffffff"],"imported":[[[-1,-1],[1,-1],[1,1],[-1,1]]]}"##;
        let d = from_v1(json).unwrap();
        match &d.parts[0].shape {
            Shape::Contours { contours, .. } => assert_eq!(contours.len(), 1),
            other => panic!("expected contours, got {other:?}"),
        }
        assert_eq!(d.parts[0].effects.rim, Some(0.2));
    }

    #[test]
    fn garbage_is_a_doc_error_not_a_panic() {
        assert!(matches!(from_v1("not json"), Err(KitError::Doc(_))));
        assert!(matches!(from_v1("{}"), Err(KitError::Doc(_))));
    }
}
