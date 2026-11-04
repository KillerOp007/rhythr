// The hidden Autoplay mark: a full-screen pass that adds the key pattern to
// the finished frame. The pattern texture carries the positive half in R
// and the negative half in G (0..1 maps to 0..4 pattern units); the
// positive half is drawn with additive blending, the negative half with
// reverse-subtract blending, because a Unorm target clamps a single signed
// draw at zero.

@group(0) @binding(0) var pat_tex: texture_2d<f32>;
@group(0) @binding(1) var pat_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // One triangle that covers the whole target.
    var p = vec2<f32>(-1.0, -1.0);
    if (i == 1u) { p = vec2<f32>(3.0, -1.0); }
    if (i == 2u) { p = vec2<f32>(-1.0, 3.0); }
    var o: VsOut;
    o.pos = vec4<f32>(p, 0.0, 1.0);
    // Row 0 of the pattern is the top of the frame, like a video frame.
    o.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return o;
}

// 4 pattern units per texel unit, times the amplitude 0.8 of 255.
const SCALE: f32 = 4.0 * 0.8 / 255.0;

@fragment
fn fs_add(in: VsOut) -> @location(0) vec4<f32> {
    let v = textureSample(pat_tex, pat_samp, in.uv).r * SCALE;
    return vec4<f32>(v, v, v, 1.0);
}

@fragment
fn fs_sub(in: VsOut) -> @location(0) vec4<f32> {
    let v = textureSample(pat_tex, pat_samp, in.uv).g * SCALE;
    return vec4<f32>(v, v, v, 1.0);
}
