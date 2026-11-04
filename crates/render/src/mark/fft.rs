//! Two-dimensional real cross-correlation on `rustfft`: the image's spectra
//! are prepared once per rotation, the pattern's once per scale, so each
//! (angle, scale) pair costs three inverse transforms.
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

/// Smallest size at or above `n` whose factors are 2, 3 and 5 only.
fn friendly(n: usize) -> usize {
    let mut m = n.max(1);
    loop {
        let mut k = m;
        for f in [2, 3, 5] {
            while k % f == 0 {
                k /= f;
            }
        }
        if k == 1 {
            return m;
        }
        m += 1;
    }
}

pub struct Fft2 {
    pub h: usize,
    pub w: usize,
    row_f: Arc<dyn Fft<f32>>,
    row_i: Arc<dyn Fft<f32>>,
    col_f: Arc<dyn Fft<f32>>,
    col_i: Arc<dyn Fft<f32>>,
}

impl Fft2 {
    /// Plans for a padded canvas of at least `min_h` by `min_w`.
    pub fn new(min_h: usize, min_w: usize) -> Fft2 {
        let h = friendly(min_h);
        let w = friendly(min_w);
        let mut planner = FftPlanner::<f32>::new();
        Fft2 {
            h,
            w,
            row_f: planner.plan_fft_forward(w),
            row_i: planner.plan_fft_inverse(w),
            col_f: planner.plan_fft_forward(h),
            col_i: planner.plan_fft_inverse(h),
        }
    }

    /// Forward transform of a real image placed at the canvas origin.
    pub fn forward(&self, img: &[f32], ih: usize, iw: usize) -> Vec<Complex<f32>> {
        let (h, w) = (self.h, self.w);
        let mut data = vec![Complex::new(0.0f32, 0.0); h * w];
        for y in 0..ih.min(h) {
            for x in 0..iw.min(w) {
                data[y * w + x].re = img[y * iw + x];
            }
        }
        for row in data.chunks_mut(w) {
            self.row_f.process(row);
        }
        let mut col = vec![Complex::new(0.0f32, 0.0); h];
        for x in 0..w {
            for y in 0..h {
                col[y] = data[y * w + x];
            }
            self.col_f.process(&mut col);
            for y in 0..h {
                data[y * w + x] = col[y];
            }
        }
        data
    }

    /// Inverse transform, real part, scaled.
    pub fn inverse(&self, spec: &[Complex<f32>]) -> Vec<f32> {
        let (h, w) = (self.h, self.w);
        let mut data = spec.to_vec();
        let mut col = vec![Complex::new(0.0f32, 0.0); h];
        for x in 0..w {
            for y in 0..h {
                col[y] = data[y * w + x];
            }
            self.col_i.process(&mut col);
            for y in 0..h {
                data[y * w + x] = col[y];
            }
        }
        for row in data.chunks_mut(w) {
            self.row_i.process(row);
        }
        let scale = 1.0 / (h * w) as f32;
        data.iter().map(|c| c.re * scale).collect()
    }

    /// Normalised cross-correlation peak of a prepared pattern over a
    /// prepared image: local energy floored at a tenth of the image's total,
    /// offsets with less than 60 percent of the pattern inside ignored.
    pub fn peak(&self, img: &ImagePrep, pat: &PatternPrep) -> (f32, (i32, i32)) {
        let (h, w) = (self.h, self.w);
        let mul = |a: &[Complex<f32>], b: &[Complex<f32>]| -> Vec<Complex<f32>> { a.iter().zip(b).map(|(x, y)| x * y.conj()).collect() };
        let cc = self.inverse(&mul(&img.fa, &pat.fp));
        let e = self.inverse(&mul(&img.fa2, &pat.fo));
        let cnt = self.inverse(&mul(&img.f1, &pat.fo));
        let floor = 0.1 * img.total;
        let min_cnt = 0.6 * pat.size as f32;
        let mut best = (0.0f32, (0i32, 0i32));
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if cnt[i] < min_cnt {
                    continue;
                }
                let v = cc[i] / (e[i].max(floor).sqrt() * pat.norm);
                if v > best.0 {
                    let dy = if y > h / 2 { y as i32 - h as i32 } else { y as i32 };
                    let dx = if x > w / 2 { x as i32 - w as i32 } else { x as i32 };
                    best = (v, (dy, dx));
                }
            }
        }
        best
    }
}

pub struct ImagePrep {
    pub fa: Vec<Complex<f32>>,
    pub fa2: Vec<Complex<f32>>,
    pub f1: Vec<Complex<f32>>,
    pub total: f32,
}

impl ImagePrep {
    pub fn new(fft: &Fft2, img: &[f32], ih: usize, iw: usize) -> ImagePrep {
        let sq: Vec<f32> = img.iter().map(|v| v * v).collect();
        let ones = vec![1.0f32; ih * iw];
        ImagePrep {
            fa: fft.forward(img, ih, iw),
            fa2: fft.forward(&sq, ih, iw),
            f1: fft.forward(&ones, ih, iw),
            total: sq.iter().sum(),
        }
    }
}

pub struct PatternPrep {
    pub fp: Vec<Complex<f32>>,
    pub fo: Vec<Complex<f32>>,
    pub norm: f32,
    pub size: usize,
}

impl PatternPrep {
    pub fn new(fft: &Fft2, pat: &[f32], ph: usize, pw: usize) -> PatternPrep {
        let ones = vec![1.0f32; ph * pw];
        PatternPrep {
            fp: fft.forward(pat, ph, pw),
            fo: fft.forward(&ones, ph, pw),
            norm: pat.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9),
            size: ph * pw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_sizes_have_small_factors() {
        assert_eq!(friendly(864), 864);
        assert_eq!(friendly(1537), 1600);
        assert_eq!(friendly(7), 8);
    }

    #[test]
    fn a_pattern_correlates_with_itself_at_zero_offset() {
        let (ph, pw) = (12, 20);
        let pat: Vec<f32> = (0..ph * pw).map(|i| ((i * 7919) % 13) as f32 - 6.0).collect();
        let fft = Fft2::new(ph + ph, pw + pw);
        let img = ImagePrep::new(&fft, &pat, ph, pw);
        let pp = PatternPrep::new(&fft, &pat, ph, pw);
        let (score, off) = fft.peak(&img, &pp);
        assert!((score - 1.0).abs() < 0.02, "score {score}");
        assert_eq!(off, (0, 0));
    }
}
