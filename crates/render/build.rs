//! Bakes the Autoplay mark seed into the binary without storing the key as a
//! string: the 32 seed bytes (SHA-256 of the key) are split into four
//! quarters, each XORed with a fixed mask, and recombined at runtime.
//!
//! `RHYTHR_MARK_KEY` (64 hex characters) selects the owner key; without it
//! the public development key is used and the build reports itself as a
//! dev build.
use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

const DEV_KEY_HEX: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";
const MASKS: [[u8; 8]; 4] = [
    [0x3c, 0xa7, 0x19, 0xe2, 0x5b, 0x80, 0x4d, 0xf6],
    [0x91, 0x0e, 0x67, 0xb3, 0x2a, 0xd8, 0x75, 0x1f],
    [0xc4, 0x52, 0xee, 0x09, 0x8b, 0x37, 0xa0, 0x6d],
    [0x78, 0xbf, 0x21, 0x94, 0xd6, 0x4c, 0x03, 0xe9],
];

fn decode_hex(hex: &str) -> Result<[u8; 32], String> {
    let h = hex.trim();
    if h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("RHYTHR_MARK_KEY must be exactly 64 hex characters".into());
    }
    let mut out = [0u8; 32];
    for (i, chunk) in h.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).unwrap();
        out[i] = u8::from_str_radix(s, 16).unwrap();
    }
    Ok(out)
}

fn main() {
    println!("cargo:rerun-if-env-changed=RHYTHR_MARK_KEY");
    let env_key = env::var("RHYTHR_MARK_KEY").ok().filter(|s| !s.trim().is_empty());
    let (bytes, is_dev) = match env_key {
        Some(k) => (decode_hex(&k).unwrap_or_else(|e| panic!("{e}")), false),
        None => (decode_hex(DEV_KEY_HEX).unwrap(), true),
    };
    let seed: [u8; 32] = Sha256::digest(bytes).into();
    let mut src = String::new();
    src.push_str("pub(super) const MASKED: [[u8; 8]; 4] = [\n");
    for q in 0..4 {
        src.push_str("    [");
        for i in 0..8 {
            src.push_str(&format!("0x{:02x}, ", seed[q * 8 + i] ^ MASKS[q][i]));
        }
        src.push_str("],\n");
    }
    src.push_str("];\n");
    src.push_str(&format!("pub(super) const BUILT_IN_KIND_IS_DEV: bool = {is_dev};\n"));
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("mark_seed.rs");
    fs::write(out, src).unwrap();
}
