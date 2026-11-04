//! The ticker across the top of every Autoplay frame: its size, place and
//! motion. The vertices are built in `hud.rs`, which owns the text layout.

/// One pass from the left edge to the right edge takes this long.
pub const PASS_SECS: f64 = 12.0;
/// Text height as a fraction of the frame height.
pub const HEIGHT_FRAC: f32 = 0.018;
/// Top of the band as a fraction of the frame height.
pub const TOP_FRAC: f32 = 0.03;

/// The band's left edge at `song_time_ms`: it enters from the left at minus
/// its width and leaves at the right edge every `PASS_SECS`, wrapping.
pub fn x_offset(song_time_ms: f64, text_width_px: f32, frame_w: f32) -> f32 {
    let span = frame_w + text_width_px;
    let phase = ((song_time_ms / 1000.0) / PASS_SECS).rem_euclid(1.0) as f32;
    -text_width_px + phase * span
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_band_moves_left_to_right_and_wraps() {
        let w = 1920.0;
        let a = x_offset(0.0, 600.0, w);
        let b = x_offset(3000.0, 600.0, w);
        let c = x_offset(12_000.0, 600.0, w);
        assert!(b > a, "moves right");
        assert!((c - a).abs() < 1e-3, "one pass per 12 s");
        assert!(a >= -600.0 && a <= w);
    }
}
