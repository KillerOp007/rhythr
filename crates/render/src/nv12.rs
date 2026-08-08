//! Turning a rendered frame into what the encoder actually wants.
//!
//! The renderer produces RGBA and the video loop used to push all of it
//! down a pipe (8 MB per frame at 1080p, 33 MB at 4K) for ffmpeg to
//! convert to NV12 on the other side. Measured, that pipe was the render's
//! entire speed limit: ffmpeg merely READING 900 RGBA frames and dropping
//! them takes 3.1 s of an 8.5 s render.
//!
//! NV12 is 1.5 bytes per pixel instead of 4, so converting here cuts 62% of
//! that traffic and saves ffmpeg the conversion as well. The machine has the
//! cores for it: during a render eleven of twelve threads were idle.
//!
//! The maths is BT.601 limited range, which is what ffmpeg's swscale was
//! producing for these frames, verified against it rather than assumed
//! (pure red gives Y=81, green 145, blue 41, white 235, and swscale agrees
//! on every one).

/// Bytes an NV12 frame of this size occupies: a full-resolution Y plane
/// followed by an interleaved half-resolution UV plane.
pub fn nv12_len(width: usize, height: usize) -> usize {
    width * height + width * height / 2
}

/// Whether a frame size can be encoded as NV12 at all. Chroma is shared by
/// 2x2 blocks, so both dimensions must be even; H.264 requires that anyway.
pub fn nv12_supported(width: usize, height: usize) -> bool {
    width % 2 == 0 && height % 2 == 0 && width > 0 && height > 0
}

/// Fixed-point BT.601 limited-range coefficients, scaled by 2^16. Chosen so
/// the result matches ffmpeg's own output on the primaries exactly (255,0,0
/// gives 81, 0,255,0 gives 145, 0,0,255 gives 41, white gives 235), which
/// float maths reproduces too but several times more slowly, on two million
/// pixels a frame.
const KY_R: i32 = 16830;
const KY_G: i32 = 33039;
const KY_B: i32 = 6416;
const KU_R: i32 = -9714;
const KU_G: i32 = -19070;
const KU_B: i32 = 28784;
const KV_R: i32 = 28784;
const KV_G: i32 = -24103;
const KV_B: i32 = -4681;
const HALF: i32 = 32768;

#[inline]
fn luma(r: i32, g: i32, b: i32) -> u8 {
    (16 + ((KY_R * r + KY_G * g + KY_B * b + HALF) >> 16)).clamp(0, 255) as u8
}

/// Converts one horizontal band. `rows` must be even so no 2x2 chroma block
/// is split.
///
/// Luma and chroma are produced in ONE pass over 2x2 blocks: the separate
/// loops each read the whole band, and at 8 MB a frame that second read is
/// not free.
fn convert_band(src: &[u8], width: usize, rows: usize, y: &mut [u8], uv: &mut [u8]) {
    for by in 0..rows / 2 {
        let row0 = by * 2 * width;
        let row1 = row0 + width;
        let uv_row = by * width;
        for bx in 0..width / 2 {
            let x = bx * 2;
            let mut sum = [0i32; 3];
            let mut px = |off: usize, out: usize| {
                let p = off * 4;
                let (r, g, b) = (
                    i32::from(src[p]),
                    i32::from(src[p + 1]),
                    i32::from(src[p + 2]),
                );
                sum[0] += r;
                sum[1] += g;
                sum[2] += b;
                y[out] = luma(r, g, b);
            };
            px(row0 + x, row0 + x);
            px(row0 + x + 1, row0 + x + 1);
            px(row1 + x, row1 + x);
            px(row1 + x + 1, row1 + x + 1);

            // Chroma from the block's average colour. Averaging the inputs
            // and averaging the results agree here because the transform is
            // linear, and this way it is one rounding instead of four.
            let (r, g, b) = (sum[0], sum[1], sum[2]);
            let u = 128 + ((KU_R * r + KU_G * g + KU_B * b + HALF * 4) >> 18);
            let v = 128 + ((KV_R * r + KV_G * g + KV_B * b + HALF * 4) >> 18);
            uv[uv_row + x] = u.clamp(0, 255) as u8;
            uv[uv_row + x + 1] = v.clamp(0, 255) as u8;
        }
    }
}

/// Fills `dst` with the NV12 form of `src`, spreading the work over the
/// idle cores. Returns false if the sizes do not line up.
pub fn rgba_to_nv12(src: &[u8], width: usize, height: usize, dst: &mut [u8]) -> bool {
    if !nv12_supported(width, height)
        || src.len() < width * height * 4
        || dst.len() < nv12_len(width, height)
    {
        return false;
    }
    let (y_plane, uv_plane) = dst.split_at_mut(width * height);
    let uv_plane = &mut uv_plane[..width * height / 2];

    // One band per worker, each an even number of rows so no 2x2 chroma
    // block is split across two threads.
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8)
        .max(1);
    let band_rows = {
        let raw = height.div_ceil(workers);
        (raw + 1) & !1 // round up to even
    };
    if workers == 1 || band_rows >= height {
        convert_band(src, width, height, y_plane, uv_plane);
        return true;
    }
    std::thread::scope(|scope| {
        let mut src_rest = src;
        let mut y_rest = y_plane;
        let mut uv_rest = uv_plane;
        let mut row = 0;
        while row < height {
            let rows = band_rows.min(height - row);
            let (s, s2) = src_rest.split_at(rows * width * 4);
            let (y, y2) = y_rest.split_at_mut(rows * width);
            let (uv, uv2) = uv_rest.split_at_mut(rows / 2 * width);
            src_rest = s2;
            y_rest = y2;
            uv_rest = uv2;
            row += rows;
            scope.spawn(move || convert_band(s, width, rows, y, uv));
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, rgb: [u8; 3]) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            v.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        v
    }

    /// The reference numbers come from ffmpeg itself: feeding it these
    /// colours as RGBA and asking for NV12 produces exactly these luma
    /// values, which is how BT.601 limited range was identified as the
    /// space it had been converting to all along.
    #[test]
    fn luma_matches_what_ffmpeg_produced() {
        let cases = [
            ([255, 0, 0], 81u8),
            ([0, 255, 0], 145),
            ([0, 0, 255], 41),
            ([255, 255, 255], 235),
            ([0, 0, 0], 16),
        ];
        for (rgb, expect) in cases {
            let src = solid(4, 2, rgb);
            let mut dst = vec![0u8; nv12_len(4, 2)];
            assert!(rgba_to_nv12(&src, 4, 2, &mut dst));
            for (i, got) in dst[..8].iter().enumerate() {
                assert_eq!(*got, expect, "{rgb:?} pixel {i}");
            }
        }
    }

    /// A neutral grey must land on chroma 128/128: any drift there tints
    /// the whole picture.
    #[test]
    fn grey_stays_neutral() {
        let src = solid(4, 2, [128, 128, 128]);
        let mut dst = vec![0u8; nv12_len(4, 2)];
        assert!(rgba_to_nv12(&src, 4, 2, &mut dst));
        for b in &dst[8..] {
            assert_eq!(*b, 128);
        }
    }

    /// Splitting the frame across threads must not change a single byte.
    #[test]
    fn banding_across_threads_changes_nothing() {
        let (w, h) = (64, 64);
        let mut src = Vec::with_capacity(w * h * 4);
        for i in 0..w * h {
            let v = (i % 251) as u8;
            src.extend_from_slice(&[v, v.wrapping_mul(3), v.wrapping_add(97), 255]);
        }
        let mut threaded = vec![0u8; nv12_len(w, h)];
        assert!(rgba_to_nv12(&src, w, h, &mut threaded));

        let mut single = vec![0u8; nv12_len(w, h)];
        let (y, uv) = single.split_at_mut(w * h);
        convert_band(&src, w, h, y, uv);
        assert_eq!(threaded, single);
    }

    #[test]
    fn odd_sizes_are_refused_rather_than_mangled() {
        assert!(!nv12_supported(63, 64));
        assert!(!nv12_supported(64, 63));
        assert!(nv12_supported(1920, 1080));
        assert!(nv12_supported(1080, 1920));
        let src = solid(4, 2, [10, 20, 30]);
        let mut small = vec![0u8; 4];
        assert!(!rgba_to_nv12(&src, 4, 2, &mut small));
    }
}
