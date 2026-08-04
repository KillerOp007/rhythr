// RGBA -> NV12 on the GPU, so the frame leaves VRAM already in the encoder's
// format: 1.5 bytes per pixel instead of 4, and no CPU pass at all.
//
// The arithmetic is a transcription of `nv12.rs` and must stay one — the
// integer coefficients, the rounding term, the shift and the clamp are all
// identical, so both paths produce byte-for-byte the same frame. A test
// renders one through each and compares. Do not "simplify" this to float
// maths; the CPU numbers were verified against ffmpeg's own output and these
// have to match them exactly.
//
// One invocation owns a 4x2 pixel block: it writes two packed luma words
// (four pixels each, one per row) and one chroma word (two 2x2 blocks, as
// u,v,u,v). That is why the width must be a multiple of four — every write
// is a whole u32 and none of them straddle a row.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;

struct Dims {
    width: u32,
    height: u32,
}
@group(0) @binding(2) var<uniform> dims: Dims;

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

// The exact byte the renderer wrote. The texture is Rgba8Unorm, so a texel
// comes back as byte/255; rounding after scaling recovers the integer without
// relying on the division having been exact.
fn texel(x: u32, y: u32) -> vec3<i32> {
    let c = textureLoad(src, vec2<i32>(i32(x), i32(y)), 0);
    return vec3<i32>(
        i32(round(c.r * 255.0)),
        i32(round(c.g * 255.0)),
        i32(round(c.b * 255.0)),
    );
}

fn luma(c: vec3<i32>) -> u32 {
    let y = 16 + ((KY_R * c.r + KY_G * c.g + KY_B * c.b + HALF) >> 16u);
    return u32(clamp(y, 0, 255));
}

// Little-endian: the first pixel of the four is the low byte, which is how it
// lands in the mapped buffer the encoder reads.
fn pack(a: u32, b: u32, c: u32, d: u32) -> u32 {
    return a | (b << 8u) | (c << 16u) | (d << 24u);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let bx = gid.x;          // 4-pixel column
    let by = gid.y;          // 2-pixel row
    if (bx * 4u >= dims.width || by * 2u >= dims.height) {
        return;
    }
    let x = bx * 4u;
    let y0 = by * 2u;
    let y1 = y0 + 1u;

    let p00 = texel(x, y0);
    let p10 = texel(x + 1u, y0);
    let p20 = texel(x + 2u, y0);
    let p30 = texel(x + 3u, y0);
    let p01 = texel(x, y1);
    let p11 = texel(x + 1u, y1);
    let p21 = texel(x + 2u, y1);
    let p31 = texel(x + 3u, y1);

    // Luma plane: one word per row of this block.
    let row_words = dims.width / 4u;
    dst[y0 * row_words + bx] = pack(luma(p00), luma(p10), luma(p20), luma(p30));
    dst[y1 * row_words + bx] = pack(luma(p01), luma(p11), luma(p21), luma(p31));

    // Chroma from each 2x2 block's summed colour — one rounding for the
    // block, exactly as the CPU path does it.
    let sa = p00 + p10 + p01 + p11;
    let sb = p20 + p30 + p21 + p31;
    let ua = 128 + ((KU_R * sa.r + KU_G * sa.g + KU_B * sa.b + HALF * 4) >> 18u);
    let va = 128 + ((KV_R * sa.r + KV_G * sa.g + KV_B * sa.b + HALF * 4) >> 18u);
    let ub = 128 + ((KU_R * sb.r + KU_G * sb.g + KU_B * sb.b + HALF * 4) >> 18u);
    let vb = 128 + ((KV_R * sb.r + KV_G * sb.g + KV_B * sb.b + HALF * 4) >> 18u);

    let uv_base = (dims.width * dims.height) / 4u;
    dst[uv_base + by * row_words + bx] = pack(
        u32(clamp(ua, 0, 255)),
        u32(clamp(va, 0, 255)),
        u32(clamp(ub, 0, 255)),
        u32(clamp(vb, 0, 255)),
    );
}
