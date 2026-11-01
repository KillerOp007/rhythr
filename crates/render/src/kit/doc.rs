//! The design document: the one JSON shape that undo snapshots, autosave,
//! the .rhsd file and the IPC payload all share.

use crate::kit::v1::ControlPoint;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Design {
    pub version: u32,
    pub name: String,
    /// The colorset in beat order.
    pub colors: Vec<String>,
    /// Bottom to top in the panel.
    pub parts: Vec<Part>,
    #[serde(default)]
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    /// Part ids; cuts inside a group apply to the group's adds only.
    pub parts: Vec<String>,
    pub mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub id: String,
    pub name: String,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    pub shape: Shape,
    pub transform: Transform,
    pub mode: Mode,
    pub depth: Depth,
    #[serde(default)]
    pub material: Material,
    #[serde(default)]
    pub effects: Effects,
}

fn yes() -> bool {
    true
}

/// A shape source at unit size (half-extent 1 before the transform).
/// Glyph and SVG imports arrive as `Contours`: the editor resolves fonts
/// and files once and stores the result, so the kernel never touches
/// fonts or the file system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    Circle,
    /// A square with a side flat at the bottom; rounding blends toward
    /// the circle exactly as Polygon does.
    Square {
        rounding: f32,
    },
    Polygon {
        sides: u32,
        rounding: f32,
    },
    Star {
        points: u32,
        inner: f32,
    },
    Ring {
        thickness: f32,
    },
    Pill {
        aspect: f32,
    },
    Contours {
        contours: Vec<Vec<[f32; 2]>>,
        #[serde(default)]
        label: String,
    },
    /// Freeform: `control` is the first closed loop, `loops` any further
    /// ones (a hole winds clockwise, an island counterclockwise), each
    /// splined through its points. Any shape converts to this.
    Points {
        control: Vec<ControlPoint>,
        #[serde(default)]
        loops: Vec<Vec<ControlPoint>>,
    },
}

/// Position and size in note-box units (1.0 = the full half-extent),
/// rotation in degrees counterclockwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub rotation: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            rotation: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Add,
    Cut,
}

/// A depth band; front is toward the player (+z), front > back.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Depth {
    pub front: f32,
    pub back: f32,
}

impl Default for Depth {
    fn default() -> Self {
        Depth {
            front: 0.1,
            back: -0.1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Material {
    #[default]
    Solid,
    Glass,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Effects {
    #[serde(default)]
    pub waves: Option<Waves>,
    /// Corner rounding radius in note units.
    #[serde(default)]
    pub round: Option<f32>,
    /// Keeps only the outline band of this thickness.
    #[serde(default)]
    pub rim: Option<f32>,
    #[serde(default)]
    pub mirror: Mirror,
    /// Copies around the centre (2 or more).
    #[serde(default)]
    pub repeat: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Waves {
    pub count: u32,
    /// Radial amplitude as a fraction of the radius.
    pub depth: f32,
    #[serde(default)]
    pub style: WaveStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WaveStyle {
    #[default]
    Sine,
    Spike,
    Pulse,
}

/// Reflection axes: `X` reflects across the x axis (y becomes -y), `Y`
/// across the y axis, `Both` adds the point reflection as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mirror {
    #[default]
    None,
    X,
    Y,
    Both,
}

impl Design {
    /// A plain disc in the game's Arctic colours: what a new design is.
    pub fn default_note() -> Design {
        Design {
            version: 2,
            name: "note".into(),
            colors: vec!["#ffffff".into(), "#a2e0ff".into(), "#a2e0ff".into()],
            parts: vec![Part {
                id: "p1".into(),
                name: "Disc".into(),
                visible: true,
                locked: false,
                shape: Shape::Circle,
                transform: Transform::default(),
                mode: Mode::Add,
                depth: Depth::default(),
                material: Material::Solid,
                effects: Effects::default(),
            }],
            groups: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_note_roundtrips_through_json() {
        let d = Design::default_note();
        assert_eq!(d.version, 2);
        assert_eq!(d.parts.len(), 1);
        assert_eq!(d.parts[0].mode, Mode::Add);
        assert_eq!(
            d.parts[0].depth,
            Depth {
                front: 0.1,
                back: -0.1
            }
        );
        let json = serde_json::to_string(&d).unwrap();
        let back: Design = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn missing_optional_fields_take_defaults() {
        let json = r##"{"version":2,"name":"x","colors":["#ffffff"],
          "parts":[{"id":"a","name":"a","shape":{"kind":"circle"},
          "transform":{"x":0,"y":0,"w":1,"h":1,"rotation":0},
          "mode":"add","depth":{"front":0.1,"back":-0.1}}]}"##;
        let d: Design = serde_json::from_str(json).unwrap();
        let p = &d.parts[0];
        assert!(p.visible);
        assert!(!p.locked);
        assert_eq!(p.material, Material::Solid);
        assert_eq!(p.effects, Effects::default());
        assert!(d.groups.is_empty());
    }
}
