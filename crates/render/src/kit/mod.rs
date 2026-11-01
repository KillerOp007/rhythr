//! The note kit: a multi-part 2.5D note is resolved into oriented 2D
//! regions per depth interval and emitted as a watertight mesh with
//! solid and glass ranges. Every boolean runs through one fixed integer
//! scale so identical inputs give bit-identical vertices, which is what
//! lets caps and walls share rings without ever drifting apart.

pub mod doc;
pub mod export;
pub mod game;
pub mod geom;
pub mod import;
pub mod levels;
pub mod mesh;
pub mod migrate;
pub mod points;
pub mod presets;
pub mod resolve;
pub mod shapes;
pub mod v1;
pub mod validate;

pub use doc::*;

/// Fixed float-to-integer scale for every overlay. A power of two keeps
/// the conversion exact in f32; 1/65536 of a note unit is far below
/// anything visible, and the largest coordinate (depth 2.0 of extent)
/// stays far inside the i32 engine's range.
pub const KIT_SCALE: f32 = 65536.0;
/// The game draws every material after the first at a fixed alpha;
/// measured from an in-game screenshot (probe 7a, 2026-10-24).
pub const GLASS_ALPHA: f32 = 0.57;
/// The exporter refuses larger meshes; 10,500 triangles played without
/// a stutter in the game, so the margin is wide.
pub const MAX_TRIANGLES: usize = 40_000;
pub const Z_MIN: f32 = -2.0;
pub const Z_MAX: f32 = 0.5;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum KitError {
    #[error("the design has no visible part that adds material")]
    Empty,
    #[error("bad document: {0}")]
    Doc(String),
    #[error("geometry scale rejected: {0}")]
    Scale(String),
    #[error("mesh invalid: {0}")]
    Invalid(String),
    #[error("too many triangles: {0} (limit {MAX_TRIANGLES})")]
    Budget(usize),
    #[error("part too complex: {0} points in {1} contours")]
    Complexity(usize, usize),
}
