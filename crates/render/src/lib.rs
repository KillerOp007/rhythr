//! GPU renderer for Rhythia replays.
//!
//! Read-only: consumes parsed replays/maps and produces pixels. It never
//! writes replay data.

pub mod analysis;
pub mod audio;
pub mod background;
pub mod builtin;
pub mod config;
pub mod designer;
pub mod exe_assets;
pub mod hud;
pub mod kit;
pub mod mesh;
pub mod mods;
pub mod nv12;
pub mod nv12_gpu;
pub mod present;
pub mod quality;
pub mod race;
pub mod renderer;
pub mod scene;
pub mod transport;
pub mod video;

pub use builtin::BuiltinAssets;
pub use config::SkinConfig;
pub use renderer::Renderer;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("malformed .obj mesh")]
    BadObj,
    #[error("no compatible GPU adapter found")]
    NoAdapter,
    #[error("GPU device request failed: {0}")]
    Device(String),
    #[error("PNG encode failed: {0}")]
    Png(String),
    #[error("video export failed: {0}")]
    Ffmpeg(String),
    #[error("render cancelled")]
    Cancelled,
    #[error("ghost race: {0}")]
    Ghost(String),
    #[error("skin config: {0}")]
    Config(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Helpers for tests and debug examples.
pub mod testing {
    /// A one-frame idle replay in the current file format, for rendering
    /// synthetic scenes.
    pub fn idle_replay() -> rhythia_formats::rhr::Replay {
        let mut b: Vec<u8> = Vec::new();
        b.extend((20260803i32).to_le_bytes());
        b.extend(0i64.to_le_bytes());
        b.push(0); // player ""
        b.push(0); // legacy ""
        b.extend(0i32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.push(0); // mode ""
        b.push(1); // passed
        b.push(2);
        b.extend(b"[]");
        b.push(0); // spin
        b.extend(1.0f32.to_le_bytes());
        b.extend(0i64.to_le_bytes());
        b.extend(100.0f32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.extend(0.0f32.to_le_bytes());
        b.extend((-1i32).to_le_bytes());
        b.push(0); // hash ""
        b.push(0); // pitch lock
        b.extend(1i32.to_le_bytes()); // frames
        b.extend(0i32.to_le_bytes());
        b.extend(0.0f32.to_le_bytes());
        b.extend(0.0f32.to_le_bytes());
        b.extend(1.0f32.to_le_bytes());
        b.push(0);
        rhythia_formats::rhr::Replay::parse(&b).expect("synthetic replay parses")
    }
}

/// Encodes RGBA8 pixels (row-major, `width`×`height`×4 bytes) as a PNG file.
pub fn write_png(
    path: &std::path::Path,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<(), Error> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| Error::Png(e.to_string()))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| Error::Png(e.to_string()))?;
    Ok(())
}
