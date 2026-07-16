//! GPU renderer for Rhythia replays.
//!
//! Read-only: consumes parsed replays/maps and produces pixels. It never
//! writes replay data.

pub mod audio;
pub mod builtin;
pub mod config;
pub mod exe_assets;
pub mod hud;
pub mod mesh;
pub mod renderer;
pub mod scene;
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
    #[error("skin config: {0}")]
    Config(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
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
