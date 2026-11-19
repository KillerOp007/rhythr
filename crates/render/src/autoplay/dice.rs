//! The dice of the Autoplay path: a fixed table whose signs decide to which
//! side an arc or a boomerang swings. It is the same for every build and
//! every run, so a map always gets the same path. The table is band-passed
//! noise from a fixed seed (how it has always been made, kept as it is so
//! the paths stay where they are); only the signs are used.

use std::sync::OnceLock;

const W: usize = 640;
const H: usize = 360;
const SEED: [u8; 32] = [
    0x60, 0xbf, 0x07, 0xc4, 0x88, 0xaa, 0xd1, 0x8f, 0xda, 0x33, 0x9d, 0xf0, 0x7e, 0x4f, 0xbc, 0x47, 0xb4, 0xf0, 0x0b, 0xe7, 0x17, 0x11, 0x93,
    0x6f, 0x18, 0xd0, 0x4d, 0x35, 0x2a, 0xd0, 0x18, 0x90,
];

/// The dice, made once.
pub fn dice() -> &'static [f32] {
    static DICE: OnceLock<Vec<f32>> = OnceLock::new();
    DICE.get_or_init(table)
}

/// xoshiro256** seeded from the 32 seed bytes.
struct Rng([u64; 4]);

impl Rng {
    fn new(seed: &[u8; 32]) -> Rng {
        let mut s = [0u64; 4];
        for (i, word) in s.iter_mut().enumerate() {
            let mut b = [0u8; 8];
            b.copy_from_slice(&seed[i * 8..i * 8 + 8]);
            *word = u64::from_le_bytes(b);
        }
        Rng(s)
    }

    fn next_u64(&mut self) -> u64 {
        let s = &mut self.0;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Uniform in (0, 1).
    fn next_unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    /// Standard normal by Box-Muller.
    fn next_normal_pair(&mut self) -> (f32, f32) {
        let u1 = self.next_unit();
        let u2 = self.next_unit();
        let r = (-2.0 * u1.ln()).sqrt();
        let th = 2.0 * std::f64::consts::PI * u2;
        ((r * th.cos()) as f32, (r * th.sin()) as f32)
    }
}

fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let r = (3.0 * sigma).ceil() as i32;
    let mut k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = k.iter().sum();
    for v in &mut k {
        *v /= sum;
    }
    k
}

/// Separable gaussian blur with reflected edges.
fn blur(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let k = gaussian_kernel(sigma);
    let r = (k.len() / 2) as isize;
    let reflect = |i: isize, n: isize| -> usize {
        let mut i = i;
        while i < 0 || i >= n {
            if i < 0 {
                i = -i - 1;
            }
            if i >= n {
                i = 2 * n - i - 1;
            }
        }
        i as usize
    };
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (j, kv) in k.iter().enumerate() {
                let sx = reflect(x as isize + j as isize - r, w as isize);
                acc += src[y * w + sx] * kv;
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (j, kv) in k.iter().enumerate() {
                let sy = reflect(y as isize + j as isize - r, h as isize);
                acc += tmp[sy * w + x] * kv;
            }
            out[y * w + x] = acc;
        }
    }
    out
}

/// Standard-normal noise, band-passed (sigma 1 minus sigma 3), zero mean.
fn table() -> Vec<f32> {
    let n = W * H;
    let mut rng = Rng::new(&SEED);
    let mut noise = Vec::with_capacity(n + 1);
    while noise.len() < n {
        let (a, b) = rng.next_normal_pair();
        noise.push(a);
        noise.push(b);
    }
    noise.truncate(n);
    let fine = blur(&noise, W, H, 1.0);
    let coarse = blur(&noise, W, H, 3.0);
    let mut p: Vec<f32> = fine.iter().zip(&coarse).map(|(a, b)| a - b).collect();
    let mean = p.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
    let var = p.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / n as f64;
    let inv = 1.0 / var.sqrt().max(1e-12);
    for v in &mut p {
        *v = ((*v as f64 - mean) * inv) as f32;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dice_are_the_same_every_time_and_fall_to_both_sides() {
        let a = table();
        assert_eq!(a.len(), W * H);
        assert_eq!(a, table());
        assert_eq!(dice(), &a[..]);
        let plus = a.iter().filter(|v| **v >= 0.0).count() as f32 / a.len() as f32;
        assert!((plus - 0.5).abs() < 0.02, "share of one side {plus}");
    }

    /// The table is pinned: a path's arcs swing to the sides these values
    /// give, and a changed table would move every Autoplay path.
    #[test]
    fn the_dice_did_not_change() {
        let a = dice();
        let signs: String = (0..24).map(|i| if a[(i * 7919) % a.len()] >= 0.0 { '+' } else { '-' }).collect();
        assert_eq!(signs, EXPECTED_SIGNS);
    }

    const EXPECTED_SIGNS: &str = "++--+-+-+----+---+-++-+-";
}
