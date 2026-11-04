//! The hidden Autoplay mark: a key-derived noise pattern that the compositor
//! adds to every frame of an Autoplay render and that `rhythr-cli mark-check`
//! looks for. The key never enters the repository; release builds receive it
//! through `RHYTHR_MARK_KEY` at compile time (see `build.rs`).

use sha2::{Digest, Sha256};

pub mod check;
mod fft;
pub mod gpu;

/// Pattern grid, stretched to whatever frame it marks.
pub const PAT_W: usize = 640;
pub const PAT_H: usize = 360;
/// Amplitude in levels of 255, the spike's value: invisible, found after
/// every normal re-encode.
pub const AMPLITUDE: f32 = 0.8;
/// The public development key: contributor builds mark with it and the
/// checker reports such videos as "dev key".
pub const DEV_KEY_HEX: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

/// Same masks as in build.rs: the baked seed is XORed with them so no 32
/// byte run of the seed sits in the binary.
const MASKS: [[u8; 8]; 4] = [
    [0x3c, 0xa7, 0x19, 0xe2, 0x5b, 0x80, 0x4d, 0xf6],
    [0x91, 0x0e, 0x67, 0xb3, 0x2a, 0xd8, 0x75, 0x1f],
    [0xc4, 0x52, 0xee, 0x09, 0x8b, 0x37, 0xa0, 0x6d],
    [0x78, 0xbf, 0x21, 0x94, 0xd6, 0x4c, 0x03, 0xe9],
];

include!(concat!(env!("OUT_DIR"), "/mark_seed.rs"));

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyKind {
    Owner,
    Dev,
}

impl KeyKind {
    pub fn label(self) -> &'static str {
        match self {
            KeyKind::Owner => "owner",
            KeyKind::Dev => "dev",
        }
    }
}

/// SHA-256 of the 32 key bytes; everything downstream derives from it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Seed(pub [u8; 32]);

/// Parses a key (64 hex characters, case insensitive, surrounding whitespace
/// ignored) into its seed.
pub fn seed_from_hex(hex: &str) -> Result<Seed, String> {
    let h = hex.trim();
    if h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("a key is 64 hex characters".into());
    }
    let mut bytes = [0u8; 32];
    for (i, chunk) in h.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).map_err(|e| e.to_string())?;
        bytes[i] = u8::from_str_radix(s, 16).map_err(|e| e.to_string())?;
    }
    Ok(Seed(Sha256::digest(bytes).into()))
}

/// The seed baked into this build and which key it came from.
pub fn built_in() -> (Seed, KeyKind) {
    let mut seed = [0u8; 32];
    for q in 0..4 {
        for i in 0..8 {
            seed[q * 8 + i] = MASKED[q][i] ^ MASKS[q][i];
        }
    }
    let kind = if BUILT_IN_KIND_IS_DEV { KeyKind::Dev } else { KeyKind::Owner };
    (Seed(seed), kind)
}

/// xoshiro256** seeded from the 32 seed bytes.
struct Rng([u64; 4]);

impl Rng {
    fn new(seed: &Seed) -> Rng {
        let mut s = [0u64; 4];
        for (i, word) in s.iter_mut().enumerate() {
            let mut b = [0u8; 8];
            b.copy_from_slice(&seed.0[i * 8..i * 8 + 8]);
            *word = u64::from_le_bytes(b);
        }
        if s.iter().all(|&w| w == 0) {
            s = [0x9E37_79B9_7F4A_7C15, 1, 2, 3];
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
pub(crate) fn blur(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
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

/// The pattern: standard-normal noise, band-passed (sigma 1 minus sigma 3),
/// zero mean, unit variance, `PAT_H` rows of `PAT_W`.
pub fn pattern(seed: &Seed) -> Vec<f32> {
    let n = PAT_W * PAT_H;
    let mut rng = Rng::new(seed);
    let mut noise = Vec::with_capacity(n + 1);
    while noise.len() < n {
        let (a, b) = rng.next_normal_pair();
        noise.push(a);
        noise.push(b);
    }
    noise.truncate(n);
    let fine = blur(&noise, PAT_W, PAT_H, 1.0);
    let coarse = blur(&noise, PAT_W, PAT_H, 3.0);
    let mut p: Vec<f32> = fine.iter().zip(&coarse).map(|(a, b)| a - b).collect();
    let mean = p.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
    let var = p.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / n as f64;
    let inv = 1.0 / var.sqrt().max(1e-12);
    for v in &mut p {
        *v = ((*v as f64 - mean) * inv) as f32;
    }
    p
}

/// Texels for an `Rg8Unorm` texture: R carries the positive half, G the
/// negative half, both scaled so 4.0 maps to 255.
pub fn pattern_halves(p: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(p.len() * 2);
    for &v in p {
        let pos = (v.clamp(0.0, 4.0) * 255.0 / 4.0).round() as u8;
        let neg = ((-v).clamp(0.0, 4.0) * 255.0 / 4.0).round() as u8;
        out.push(pos);
        out.push(neg);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pattern_is_deterministic_zero_mean_unit_variance() {
        let s = seed_from_hex(DEV_KEY_HEX).unwrap();
        let a = pattern(&s);
        let b = pattern(&s);
        assert_eq!(a.len(), PAT_W * PAT_H);
        assert_eq!(a, b);
        let mean = a.iter().sum::<f32>() / a.len() as f32;
        let var = a.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / a.len() as f32;
        assert!(mean.abs() < 1e-3, "mean {mean}");
        assert!((var - 1.0).abs() < 0.02, "variance {var}");
    }

    #[test]
    fn different_keys_give_uncorrelated_patterns() {
        let a = pattern(&seed_from_hex(DEV_KEY_HEX).unwrap());
        let b = pattern(&seed_from_hex(&"ab".repeat(32)).unwrap());
        let corr: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum::<f32>() / a.len() as f32;
        assert!(corr.abs() < 0.03, "correlation {corr}");
    }

    #[test]
    fn the_pattern_is_band_passed() {
        // neighbouring texels correlate (the sigma 1 blur), texels 12 apart do
        // not (the sigma 3 blur removed the slow part)
        let a = pattern(&seed_from_hex(DEV_KEY_HEX).unwrap());
        let near: f32 = (0..PAT_H)
            .flat_map(|y| (0..PAT_W - 1).map(move |x| (y, x)))
            .map(|(y, x)| a[y * PAT_W + x] * a[y * PAT_W + x + 1])
            .sum::<f32>()
            / ((PAT_W - 1) * PAT_H) as f32;
        let far: f32 = (0..PAT_H)
            .flat_map(|y| (0..PAT_W - 12).map(move |x| (y, x)))
            .map(|(y, x)| a[y * PAT_W + x] * a[y * PAT_W + x + 12])
            .sum::<f32>()
            / ((PAT_W - 12) * PAT_H) as f32;
        assert!(near > 0.5, "near {near}");
        assert!(far.abs() < 0.05, "far {far}");
    }

    #[test]
    fn hex_keys_are_validated() {
        assert!(seed_from_hex("zz").is_err());
        assert!(seed_from_hex(&"0".repeat(63)).is_err());
        assert_eq!(
            seed_from_hex(&format!(" {} \n", "A".repeat(64))).unwrap(),
            seed_from_hex(&"a".repeat(64)).unwrap()
        );
    }

    #[test]
    fn halves_split_the_sign() {
        let h = pattern_halves(&[0.0, 2.0, -2.0, 9.0]);
        assert_eq!(h, vec![0, 0, 128, 0, 0, 128, 255, 0]);
    }

    #[test]
    fn the_built_in_seed_is_the_dev_key_without_the_env() {
        // this test runs in a tree built without RHYTHR_MARK_KEY
        if std::env::var_os("RHYTHR_MARK_KEY").is_none() {
            let (s, kind) = built_in();
            assert_eq!(kind, KeyKind::Dev);
            assert_eq!(s, seed_from_hex(DEV_KEY_HEX).unwrap());
        }
    }
}
