// HUD overlay: flat 2D quads in pixel space, drawn over the 3D scene. Solid
// fills (mode 0), glyph coverage from the font atlas (mode 1), and for the
// results screen the map cover (mode 2) and its blurred variant (mode 3).

struct Screen {
    size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var cover: texture_2d<f32>;
@group(0) @binding(4) var cover_blur: texture_2d<f32>;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) mode: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let ndc = vec2<f32>(
        in.pos.x / screen.size.x * 2.0 - 1.0,
        1.0 - in.pos.y / screen.size.y * 2.0,
    );
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let a = textureSample(atlas, samp, in.uv).r;
    let c = textureSample(cover, samp, in.uv);
    let cb = textureSample(cover_blur, samp, in.uv);
    if (in.mode > 2.5) {
        return vec4<f32>(cb.rgb * in.color.rgb, in.color.a);
    }
    if (in.mode > 1.5) {
        return vec4<f32>(c.rgb * in.color.rgb, in.color.a);
    }
    if (in.mode > 0.5) {
        return vec4<f32>(in.color.rgb, in.color.a * a);
    }
    return in.color;
}
