//! Upgrades v1 designer documents (the single-outline editor) into kit
//! designs: one Add part carrying the same shape, effects and symmetry,
//! so nothing anyone saved is lost.

use crate::kit::doc::*;
use crate::kit::geom::extent;
use crate::kit::resolve::{resolve_part, MAX_REPEAT};
use crate::kit::v1::ControlPoint;
use crate::kit::KitError;

/// One stored v1 freeform point, read in f64 as the editor held it.
#[derive(Clone, Copy, serde::Deserialize)]
struct V1Point {
    x: f64,
    y: f64,
    #[serde(default = "smooth_default")]
    smooth: bool,
}

fn smooth_default() -> bool {
    true
}

/// v1's `controlLoop()` (crates/gui/ui/designer.js): the stored points
/// cover ONE wedge, and the editor expanded them into one closed control
/// loop before the backend splined it. A sector is the wedge, preceded by
/// its reflection across the x axis in reverse order when `mirror` is on
/// (so the loop keeps running counterclockwise); the sector is then turned
/// by `2 pi s / symmetry` for every `s` below `symmetry`. The maths runs in
/// f64 as the editor's did, the smooth flags are kept, and consecutive
/// equal points stay: v1's backend handed the loop to `flatten_controls`
/// unchanged. Fewer than three points are no loop (v1 returned null).
fn control_loop(free: &[V1Point], symmetry: u32, mirror: bool) -> Option<Vec<ControlPoint>> {
    let sector: Vec<V1Point> = if mirror {
        free.iter()
            .rev()
            .map(|p| V1Point { y: -p.y, ..*p })
            .chain(free.iter().copied())
            .collect()
    } else {
        free.to_vec()
    };
    let mut ctrl = Vec::with_capacity(sector.len() * symmetry as usize);
    for s in 0..symmetry {
        let a = s as f64 * std::f64::consts::PI * 2.0 / symmetry as f64;
        let (ca, sa) = (a.cos(), a.sin());
        for p in &sector {
            ctrl.push(ControlPoint {
                x: (p.x * ca - p.y * sa) as f32,
                y: (p.x * sa + p.y * ca) as f32,
                smooth: p.smooth,
            });
        }
    }
    (ctrl.len() >= 3).then_some(ctrl)
}

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
    // v1 read `Number(d.symmetry) || 4`: a missing or zero symmetry is
    // four. Its selector offered at most 12; the cap keeps a corrupt
    // document from expanding the loop without bound.
    let symmetry = v
        .get("symmetry")
        .and_then(|x| x.as_u64())
        .filter(|n| *n > 0)
        .unwrap_or(4)
        .min(MAX_REPEAT as u64) as u32;
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
    let free: Option<Vec<V1Point>> = v
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
    } else if let Some(control) = free.and_then(|f| control_loop(&f, symmetry, mirror)) {
        // The symmetry and the mirror live in the expanded loop itself, so
        // the part carries no repeat and no mirror effect.
        (
            Shape::Points {
                control,
                loops: Vec::new(),
            },
            "Freeform",
            0.0,
        )
    } else {
        let waves = f("waves", 0.0) as u32;
        if waves > 0 {
            // v1 modulated outward and normalised afterwards, so its valleys
            // sat at (1 - d) / (1 + d) of the tips; the kit's waves run
            // inward with valleys at 1 - d', so d' = 2d / (1 + d) keeps the
            // look.
            let d = f("waveDepth", 0.25);
            effects.waves = Some(Waves {
                count: waves,
                depth: 2.0 * d / (1.0 + d),
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
    let mut part = Part {
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
    };
    // v1 fitted its outline to the box on every evaluation; the kit keeps
    // the box fixed and cuts to it, so the fit happens once, here: the
    // part is sized so its outline (waves and symmetry included; the rim
    // and rounding never reach further) spans the box as v1 showed it.
    // Rotation and the effects scale with the part, so the look is kept.
    if let Ok(resolved) = resolve_part(&part) {
        let e = extent(&resolved.shapes);
        if e > 1e-6 && (e - 1.0).abs() > 1e-4 {
            part.transform.w /= e;
            part.transform.h /= e;
        }
    }
    Ok(Design {
        version: 2,
        name: "note".into(),
        colors,
        parts: vec![part],
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
        // waveDepth 0.25 outward becomes 0.4 inward (2d / (1 + d)).
        assert_eq!(
            p.effects.waves,
            Some(Waves {
                count: 5,
                depth: 0.4,
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

    /// A two-point wedge, smooth then corner, as v1 stored it.
    const WEDGE: &str = r##"{"P":{"shape":"circle","n":6,"round":0.3,"hollow":false,"thick":0.22,
      "depth":0.05,"waves":0,"waveDepth":0.25,"waveform":"sine","twist":0},
      "free":[{"x":0.2,"y":0.0,"smooth":true},{"x":0.3,"y":0.3,"smooth":false}],
      "symmetry":4,"mirror":false,"colors":["#ffffff"],"imported":null}"##;

    fn control_of(d: &Design) -> &[ControlPoint] {
        match &d.parts[0].shape {
            Shape::Points { control, .. } => control,
            other => panic!("expected points, got {other:?}"),
        }
    }

    fn near(p: &ControlPoint, x: f32, y: f32) -> bool {
        (p.x - x).abs() < 1e-5 && (p.y - y).abs() < 1e-5
    }

    #[test]
    fn freeform_symmetry_expands_to_one_loop() {
        // v1's editor expanded the wedge through the symmetry into ONE
        // control loop before the backend splined it.
        let d = from_v1(WEDGE).unwrap();
        let c = control_of(&d);
        assert_eq!(c.len(), 8, "{c:?}");
        // The second rotation (90 degrees) of both wedge points.
        assert!(near(&c[2], 0.0, 0.2), "{:?}", c[2]);
        assert!(near(&c[3], -0.3, 0.3), "{:?}", c[3]);
        assert!(c[2].smooth && !c[3].smooth, "smooth flags are kept");
        let p = &d.parts[0];
        assert_eq!(p.effects.repeat, None);
        assert_eq!(p.effects.mirror, Mirror::None);
        assert_eq!(p.effects.rim, None);
        // A missing symmetry is v1's default of four.
        let json = WEDGE.replace(r#""symmetry":4,"#, "");
        assert_eq!(control_of(&from_v1(&json).unwrap()).len(), 8);
        // Two points without symmetry are no loop: v1 fell back to the
        // sliders, and so does the migration.
        let json = WEDGE.replace(r#""symmetry":4"#, r#""symmetry":1"#);
        assert_eq!(from_v1(&json).unwrap().parts[0].shape, Shape::Circle);
    }

    #[test]
    fn freeform_mirror_reflects_and_reverses() {
        let json = WEDGE.replace(r#""mirror":false"#, r#""mirror":true"#);
        let d = from_v1(&json).unwrap();
        let c = control_of(&d);
        assert_eq!(c.len(), 16, "{c:?}");
        // The sector: the wedge's reflection in reverse order, then the
        // wedge itself, so the loop keeps running counterclockwise.
        assert!(near(&c[0], 0.3, -0.3) && !c[0].smooth, "{:?}", c[0]);
        assert!(near(&c[1], 0.2, 0.0) && c[1].smooth, "{:?}", c[1]);
        assert!(near(&c[2], 0.2, 0.0) && c[2].smooth, "{:?}", c[2]);
        assert!(near(&c[3], 0.3, 0.3) && !c[3].smooth, "{:?}", c[3]);
        // Then the rotated copies: the sector turned by 90 degrees.
        assert!(near(&c[4], 0.3, 0.3), "{:?}", c[4]);
        assert!(near(&c[7], -0.3, 0.3), "{:?}", c[7]);
        let p = &d.parts[0];
        assert_eq!(p.effects.repeat, None);
        assert_eq!(p.effects.mirror, Mirror::None);
    }

    #[test]
    fn a_v1_design_is_fitted_to_the_box_once() {
        // The wedge's points reach 0.3; v1 showed the loop filling the
        // box, so the migrated part is sized to do the same. A slider
        // shape with waves reached past the box in v1 and was shrunk to
        // it; the part is sized down the same way.
        let d = from_v1(WEDGE).unwrap();
        let p = &d.parts[0];
        assert!(
            p.transform.w > 1.0 && (p.transform.w - p.transform.h).abs() < 1e-6,
            "{:?}",
            p.transform
        );
        let ev = crate::kit::levels::evaluate(&d).unwrap();
        assert!((extent(&ev.intervals[0].solid) - 1.0).abs() < 2e-2);
        let d = from_v1(SLIDERS).unwrap();
        let ev = crate::kit::levels::evaluate(&d).unwrap();
        assert!((extent(&ev.intervals[0].solid) - 1.0).abs() < 2e-2);
    }

    #[test]
    fn a_v1_wave_depth_converts_to_the_inward_formula() {
        // v1 modulated outward and normalised afterwards, so its valleys sat
        // at (1 - d) / (1 + d) of the tips; the inward formula puts them at
        // 1 - d', so d' = 2d / (1 + d) keeps the look.
        let json = r##"{"P":{"shape":"circle","n":6,"round":0.0,"hollow":false,"thick":0.22,
          "depth":0.05,"waves":6,"waveDepth":0.25,"waveform":"sine","twist":0},
          "free":null,"symmetry":1,"mirror":false,"colors":["#ffffff"],"imported":null}"##;
        let d = from_v1(json).unwrap();
        let w = d.parts[0].effects.waves.as_ref().expect("waves kept");
        assert_eq!(w.count, 6);
        assert!((w.depth - 0.4).abs() < 1e-5, "depth {}", w.depth);
    }

    #[test]
    fn migrated_freeform_evaluates_to_one_contour() {
        // Stored as the wedge alone with repeat and mirror effects, a
        // four-fold design became four disjoint quarter blobs, and a
        // two-point wedge like this one no shape at all (KitError::Empty).
        let ev = crate::kit::levels::evaluate(&from_v1(WEDGE).unwrap());
        assert!(!matches!(ev, Err(KitError::Empty)), "{ev:?}");
        let ev = ev.unwrap();
        assert_eq!(ev.intervals.len(), 1);
        let solid = &ev.intervals[0].solid;
        assert_eq!(solid.len(), 1, "one shape, not four blobs");
        assert_eq!(solid[0].len(), 1, "one contour, no holes");
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
