//! Finding the mark in a video: the temporal average of the high-passed
//! luma on the pattern grid, cross-correlated with the pattern over scales,
//! rotations and every offset, judged against the peaks of all the wrong
//! geometries (a pattern at the wrong scale or angle is as good as a wrong
//! key, so those peaks are the null distribution for free).
use super::fft::{Fft2, ImagePrep, PatternPrep};
use super::{blur, PAT_H, PAT_W};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerdictKind {
    Found,
    Weak,
    None,
}

#[derive(Clone, Debug)]
pub struct Verdict {
    pub score: f32,
    pub scale: f32,
    pub angle_deg: f32,
    pub offset: (i32, i32),
    pub null_mean: f32,
    pub null_std: f32,
    pub null_n: usize,
    pub z: f32,
    pub wrong_keys_max: f32,
    pub kind: VerdictKind,
}

/// Every third frame goes into the average; the pattern is static, so three
/// times fewer frames cost nothing in confidence and save two thirds of the
/// work.
pub const FRAME_STEP: usize = 3;

/// Bilinear resample of a row-major image.
pub fn resample(src: &[f32], sh: usize, sw: usize, dh: usize, dw: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; dh * dw];
    if sh == 0 || sw == 0 || dh == 0 || dw == 0 {
        return out;
    }
    let fy = sh as f32 / dh as f32;
    let fx = sw as f32 / dw as f32;
    for y in 0..dh {
        let sy = ((y as f32 + 0.5) * fy - 0.5).max(0.0);
        let y0 = (sy as usize).min(sh - 1);
        let y1 = (y0 + 1).min(sh - 1);
        let ty = sy - y0 as f32;
        for x in 0..dw {
            let sx = ((x as f32 + 0.5) * fx - 0.5).max(0.0);
            let x0 = (sx as usize).min(sw - 1);
            let x1 = (x0 + 1).min(sw - 1);
            let tx = sx - x0 as f32;
            let a = src[y0 * sw + x0] * (1.0 - tx) + src[y0 * sw + x1] * tx;
            let b = src[y1 * sw + x0] * (1.0 - tx) + src[y1 * sw + x1] * tx;
            out[y * dw + x] = a * (1.0 - ty) + b * ty;
        }
    }
    out
}

/// Area average of a luma plane to the pattern grid: exact block means when
/// the frame is a multiple of the grid, bilinear otherwise.
pub fn canonical(y: &[u8], width: usize, height: usize) -> Vec<f32> {
    if height.is_multiple_of(PAT_H) && width.is_multiple_of(PAT_W) && height > 0 && width > 0 {
        let fy = height / PAT_H;
        let fx = width / PAT_W;
        let inv = 1.0 / (fy * fx) as f32;
        let mut out = vec![0.0f32; PAT_H * PAT_W];
        for (cy, row) in out.chunks_mut(PAT_W).enumerate() {
            for (cx, v) in row.iter_mut().enumerate() {
                let mut acc = 0u32;
                for yy in 0..fy {
                    let base = (cy * fy + yy) * width + cx * fx;
                    for xx in 0..fx {
                        acc += y[base + xx] as u32;
                    }
                }
                *v = acc as f32 * inv;
            }
        }
        return out;
    }
    let f: Vec<f32> = y.iter().map(|&v| v as f32).collect();
    resample(&f, height, width, PAT_H, PAT_W)
}

/// The canonical image minus its slow part (gaussian sigma 3), zero mean.
pub fn highpass(c: &[f32]) -> Vec<f32> {
    let low = blur(c, PAT_W, PAT_H, 3.0);
    let mut hp: Vec<f32> = c.iter().zip(&low).map(|(a, b)| a - b).collect();
    let mean = hp.iter().sum::<f32>() / hp.len() as f32;
    for v in &mut hp {
        *v -= mean;
    }
    hp
}

/// Rotates a canonical image about its centre (degrees, bilinear, zero fill).
pub fn rotate(img: &[f32], deg: f32) -> Vec<f32> {
    if deg == 0.0 {
        return img.to_vec();
    }
    let (h, w) = (PAT_H, PAT_W);
    let (cy, cx) = ((h as f32 - 1.0) / 2.0, (w as f32 - 1.0) / 2.0);
    let (s, c) = deg.to_radians().sin_cos();
    let mut out = vec![0.0f32; h * w];
    for y in 0..h {
        for x in 0..w {
            let dy = y as f32 - cy;
            let dx = x as f32 - cx;
            let sx = c * dx + s * dy + cx;
            let sy = -s * dx + c * dy + cy;
            if sx < 0.0 || sy < 0.0 || sx > (w - 1) as f32 || sy > (h - 1) as f32 {
                continue;
            }
            let x0 = sx as usize;
            let y0 = sy as usize;
            let x1 = (x0 + 1).min(w - 1);
            let y1 = (y0 + 1).min(h - 1);
            let tx = sx - x0 as f32;
            let ty = sy - y0 as f32;
            let a = img[y0 * w + x0] * (1.0 - tx) + img[y0 * w + x1] * tx;
            let b = img[y1 * w + x0] * (1.0 - tx) + img[y1 * w + x1] * tx;
            out[y * w + x] = a * (1.0 - ty) + b * ty;
        }
    }
    out
}

/// Collects the temporal average of the high-passed canonical luma.
pub struct Accumulator {
    width: usize,
    height: usize,
    #[allow(dead_code)]
    black: f32,
    acc: Vec<f64>,
    pushed: usize,
    used: usize,
}

impl Accumulator {
    pub fn new(width: usize, height: usize, black: f32) -> Accumulator {
        Accumulator { width, height, black, acc: vec![0.0; PAT_H * PAT_W], pushed: 0, used: 0 }
    }

    /// One frame's luma plane, row major `width * height`.
    pub fn push_luma(&mut self, y: &[u8]) {
        self.pushed += 1;
        if !(self.pushed - 1).is_multiple_of(FRAME_STEP) {
            return;
        }
        let hp = highpass(&canonical(y, self.width, self.height));
        for (a, v) in self.acc.iter_mut().zip(&hp) {
            *a += *v as f64;
        }
        self.used += 1;
    }

    pub fn frames_used(&self) -> usize {
        self.used
    }

    pub fn average(&self) -> Vec<f32> {
        let n = self.used.max(1) as f64;
        let mut avg: Vec<f32> = self.acc.iter().map(|&a| (a / n) as f32).collect();
        let mean = avg.iter().sum::<f32>() / avg.len() as f32;
        for v in &mut avg {
            *v -= mean;
        }
        avg
    }
}

/// One evaluated geometry.
#[derive(Clone, Copy, Debug)]
pub struct Peak {
    pub score: f32,
    pub scale: f32,
    pub angle: f32,
    pub offset: (i32, i32),
}

fn pattern_size(scale: f32) -> (usize, usize) {
    (((PAT_H as f32) * scale).round().max(2.0) as usize, ((PAT_W as f32) * scale).round().max(2.0) as usize)
}

/// Evaluates every (angle, scale) pair; angles run on their own threads.
pub fn search(avg: &[f32], pattern: &[f32], scales: &[f32], angles: &[f32]) -> Vec<Peak> {
    let smax = scales.iter().cloned().fold(0.0f32, f32::max);
    let (pmh, pmw) = pattern_size(smax);
    let fft = Fft2::new(PAT_H + pmh, PAT_W + pmw);
    let pats: Vec<(f32, PatternPrep)> = scales
        .iter()
        .map(|&s| {
            let (ph, pw) = pattern_size(s);
            let mut p = if s == 1.0 { pattern.to_vec() } else { resample(pattern, PAT_H, PAT_W, ph, pw) };
            let mean = p.iter().sum::<f32>() / p.len() as f32;
            for v in &mut p {
                *v -= mean;
            }
            (s, PatternPrep::new(&fft, &p, ph, pw))
        })
        .collect();
    let mut peaks = Vec::with_capacity(scales.len() * angles.len());
    std::thread::scope(|sc| {
        let handles: Vec<_> = angles
            .iter()
            .map(|&a| {
                let fft = &fft;
                let pats = &pats;
                sc.spawn(move || {
                    let img = rotate(avg, a);
                    let prep = ImagePrep::new(fft, &img, PAT_H, PAT_W);
                    pats.iter()
                        .map(|(s, pp)| {
                            let (score, offset) = fft.peak(&prep, pp);
                            Peak { score, scale: *s, angle: a, offset }
                        })
                        .collect::<Vec<Peak>>()
                })
            })
            .collect();
        for h in handles {
            peaks.extend(h.join().expect("search thread"));
        }
    });
    peaks
}

fn best(peaks: &[Peak]) -> Peak {
    peaks.iter().cloned().fold(Peak { score: f32::MIN, scale: 1.0, angle: 0.0, offset: (0, 0) }, |b, p| if p.score > b.score { p } else { b })
}

fn grid(from: f32, to: f32, step: f32) -> Vec<f32> {
    let mut v = Vec::new();
    let mut x = from;
    while x <= to + step * 0.01 {
        v.push((x * 10000.0).round() / 10000.0);
        x += step;
    }
    v
}

/// Coarse scales: 0.05 apart over the whole range, 0.025 apart near 1.0,
/// because the pattern decorrelates within a few percent of scale and a
/// weak mark two percent off would otherwise score like a wrong key at
/// every coarse step.
fn coarse_scales() -> Vec<f32> {
    let mut v = grid(0.7, 1.4, 0.05);
    v.extend(grid(0.9, 1.1, 0.025));
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v.dedup();
    v
}

/// The three-pass search: coarse, medium around the best, fine around that.
/// Returns the best peak and the coarse peaks (for the null).
pub fn search3(avg: &[f32], pattern: &[f32]) -> (Peak, Vec<Peak>) {
    let coarse = search(avg, pattern, &coarse_scales(), &grid(-2.0, 2.0, 0.5));
    let b = best(&coarse);
    let mid_scales: Vec<f32> = grid(b.scale - 0.05, b.scale + 0.05, 0.0125).into_iter().filter(|&s| s > 0.3).collect();
    let mid = best(&search(avg, pattern, &mid_scales, &grid(b.angle - 0.5, b.angle + 0.5, 0.25)));
    let fine_scales: Vec<f32> = grid(mid.scale - 0.0125, mid.scale + 0.0125, 0.005).into_iter().filter(|&s| s > 0.3).collect();
    let fine = best(&search(avg, pattern, &fine_scales, &grid(mid.angle - 0.25, mid.angle + 0.25, 0.1)));
    let winner = [fine, mid, b].into_iter().fold(b, |w, p| if p.score > w.score { p } else { w });
    (winner, coarse)
}

/// The null: coarse peaks in the homogeneous band near scale 1 that are not
/// the best geometry; when too few, every mismatched coarse peak.
fn null_samples(coarse: &[Peak], b: &Peak) -> Vec<f32> {
    let near: Vec<f32> = coarse
        .iter()
        .filter(|p| (0.85..=1.15).contains(&p.scale) && ((p.scale - b.scale).abs() > 0.03 || (p.angle - b.angle).abs() > 0.5))
        .map(|p| p.score)
        .collect();
    if near.len() >= 20 {
        return near;
    }
    coarse
        .iter()
        .filter(|p| (p.scale - b.scale).abs() > 0.1 || (p.angle - b.angle).abs() > 0.5)
        .map(|p| p.score)
        .collect()
}

/// The verdict for one averaged video against one key's pattern.
pub fn check(avg: &[f32], pattern: &[f32], wrong_keys: &[Vec<f32>]) -> Verdict {
    let (b, coarse) = search3(avg, pattern);
    let null = null_samples(&coarse, &b);
    let n = null.len().max(1) as f32;
    let null_mean = null.iter().sum::<f32>() / n;
    let null_std = (null.iter().map(|v| (v - null_mean).powi(2)).sum::<f32>() / n).sqrt().max(1e-6);
    let z = (b.score - null_mean) / null_std;
    let wrong_keys_max = wrong_keys
        .iter()
        .map(|p| best(&search(avg, p, &grid(0.7, 1.4, 0.1), &[-1.0, 0.0, 1.0])).score)
        .fold(0.0f32, f32::max);
    // the score is a maximum over a few hundred geometries, which alone sits
    // about three sigma above one mismatched peak: hence the higher bars
    let kind = if z > 10.0 {
        VerdictKind::Found
    } else if z > 6.0 {
        VerdictKind::Weak
    } else {
        VerdictKind::None
    };
    Verdict {
        score: b.score,
        scale: b.scale,
        angle_deg: b.angle,
        offset: b.offset,
        null_mean,
        null_std,
        null_n: null.len(),
        z,
        wrong_keys_max,
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mark::{pattern, seed_from_hex, AMPLITUDE, DEV_KEY_HEX};

    /// `frames` luma frames of `w` by `h`: black 16, eight bright squares that
    /// drift a pixel per frame, optionally plus the stretched pattern at the
    /// mark's amplitude, optionally scaled about the centre and cropped back.
    fn synthetic(p: Option<&[f32]>, frames: usize, w: usize, h: usize, scale: f32) -> Vec<f32> {
        let stretched = p.map(|p| resample(p, PAT_H, PAT_W, h, w));
        let mut acc = Accumulator::new(w, h, 16.0);
        for f in 0..frames {
            let mut y = vec![16u8; w * h];
            for s in 0..8 {
                let x0 = (s * 211 + f) % (w - 120);
                let y0 = (s * 137 + f) % (h - 120);
                for yy in y0..y0 + 120 {
                    for xx in x0..x0 + 120 {
                        y[yy * w + xx] = 200;
                    }
                }
            }
            if let Some(st) = &stretched {
                for (v, m) in y.iter_mut().zip(st) {
                    *v = (*v as f32 + AMPLITUDE * m).round().clamp(0.0, 255.0) as u8;
                }
            }
            if scale != 1.0 {
                let f: Vec<f32> = y.iter().map(|&v| v as f32).collect();
                let (cy, cx) = ((h as f32 - 1.0) / 2.0, (w as f32 - 1.0) / 2.0);
                for yy in 0..h {
                    for xx in 0..w {
                        let sy = ((yy as f32 - cy) / scale + cy).clamp(0.0, (h - 1) as f32);
                        let sx = ((xx as f32 - cx) / scale + cx).clamp(0.0, (w - 1) as f32);
                        let (y0, x0) = (sy as usize, sx as usize);
                        let (y1, x1) = ((y0 + 1).min(h - 1), (x0 + 1).min(w - 1));
                        let (ty, tx) = (sy - y0 as f32, sx - x0 as f32);
                        let a = f[y0 * w + x0] * (1.0 - tx) + f[y0 * w + x1] * tx;
                        let b = f[y1 * w + x0] * (1.0 - tx) + f[y1 * w + x1] * tx;
                        y[yy * w + xx] = (a * (1.0 - ty) + b * ty).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            acc.push_luma(&y);
        }
        acc.average()
    }

    fn synthetic_average(p: &[f32], frames: usize, scale: f32) -> Vec<f32> {
        synthetic(Some(p), frames, 1920, 1080, scale)
    }

    fn synthetic_average_unmarked(frames: usize) -> Vec<f32> {
        synthetic(None, frames, 1920, 1080, 1.0)
    }

    fn synthetic_average_sized(p: &[f32], frames: usize, w: usize, h: usize) -> Vec<f32> {
        synthetic(Some(p), frames, w, h, 1.0)
    }

    fn dev() -> Vec<f32> {
        pattern(&seed_from_hex(DEV_KEY_HEX).unwrap())
    }

    #[test]
    fn a_marked_clip_is_found_and_a_clean_one_is_not() {
        let p = dev();
        let wrong: Vec<Vec<f32>> = (1..=3).map(|i| pattern(&seed_from_hex(&format!("{:02x}", i).repeat(32)).unwrap())).collect();
        let marked = check(&synthetic_average(&p, 60, 1.0), &p, &wrong);
        assert_eq!(marked.kind, VerdictKind::Found, "{marked:?}");
        assert!(marked.score > 0.2 && marked.z > 10.0, "{marked:?}");
        assert!((marked.scale - 1.0).abs() < 0.006 && marked.angle_deg.abs() < 0.11, "{marked:?}");
        let clean = check(&synthetic_average_unmarked(60), &p, &wrong);
        assert_eq!(clean.kind, VerdictKind::None, "{clean:?}");
    }

    #[test]
    fn a_wrong_key_finds_nothing() {
        let p = dev();
        let other = pattern(&seed_from_hex(&"cd".repeat(32)).unwrap());
        let v = check(&synthetic_average(&p, 60, 1.0), &other, &[]);
        assert_eq!(v.kind, VerdictKind::None, "{v:?}");
    }

    #[test]
    fn a_two_percent_upscale_with_a_centre_crop_is_found() {
        let p = dev();
        let v = check(&synthetic_average(&p, 60, 1.02), &p, &[]);
        assert_eq!(v.kind, VerdictKind::Found, "{v:?}");
        assert!((v.scale - 1.02).abs() < 0.006, "scale {}", v.scale);
    }

    #[test]
    fn a_vertical_frame_is_found() {
        let p = dev();
        let v = check(&synthetic_average_sized(&p, 60, 1080, 1920), &p, &[]);
        assert_eq!(v.kind, VerdictKind::Found, "{v:?}");
    }

    #[test]
    fn the_accumulator_uses_every_third_frame() {
        let mut acc = Accumulator::new(1920, 1080, 16.0);
        for _ in 0..10 {
            acc.push_luma(&vec![16u8; 1920 * 1080]);
        }
        assert_eq!(acc.frames_used(), 4);
    }
}
