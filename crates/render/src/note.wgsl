// Notes, cursor and playfield border, all drawn on one flat unit quad
// (local xy in [-1,1]) with signed-distance shapes — so the note skin
// (thin / square / rounded / circle) and the border style come from
// parameters, not from separate meshes, and edges anti-alias via fwidth.

struct Globals {
    view_proj: mat4x4<f32>,
    glow_tint: vec4<f32>,      // rgb tint, a = glow strength
    // x = note corner radius, y = note outline width,
    // z = border mode (0 = full frame, 1 = corners), w = border corner radius
    params: vec4<f32>,
    // Which imported skin textures are present: x=note y=border z=cursor.
    tex_flags: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

// Imported-skin textures (group 1). Bound to 1×1 transparent dummies when a
// pack ships no texture — the tex_flags then keep the shader on the SDF path.
@group(1) @binding(0) var note_tex: texture_2d<f32>;
@group(1) @binding(1) var border_tex: texture_2d<f32>;
@group(1) @binding(2) var cursor_tex: texture_2d<f32>;
@group(1) @binding(3) var skin_samp: sampler;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) m0: vec4<f32>,
    @location(2) m1: vec4<f32>,
    @location(3) m2: vec4<f32>,
    @location(4) m3: vec4<f32>,
    @location(5) color: vec4<f32>,   // rgb + opacity
    @location(6) kind: f32,          // 0 note, 1 cursor dot, 2 border
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) @interpolate(flat) kind: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let model = mat4x4<f32>(in.m0, in.m1, in.m2, in.m3);
    var out: VsOut;
    out.clip = globals.view_proj * model * vec4<f32>(in.pos, 1.0);
    out.color = in.color;
    out.local = in.pos.xy;
    out.kind = in.kind;
    return out;
}

// Signed distance to a rounded rectangle covering local [-1,1]² with the
// given corner radius. <0 inside, 0 on the edge, >0 outside.
fn rrect_sd(p: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - (vec2<f32>(1.0, 1.0) - radius);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let ax = abs(in.local.x);
    let ay = abs(in.local.y);
    // Quad-local (±1) → texture uv, V flipped (image top = +y).
    let uv = vec2<f32>(in.local.x * 0.5 + 0.5, 0.5 - in.local.y * 0.5);

    if in.kind > 2.5 {
        // Solid quad (playfield grid lines etc.).
        return in.color;
    }
    if in.kind > 1.5 {
        if globals.tex_flags.y > 0.5 {
            // Imported border texture, tinted by the border colour.
            let t = textureSample(border_tex, skin_samp, uv);
            return vec4<f32>(in.color.rgb * t.rgb, t.a * in.color.a);
        }
        if globals.params.z > 0.5 {
            // "small-corners": sharp L-brackets at the four corners.
            let ow = 0.035;   // arm thickness (fraction of the half-size)
            let bl = 0.48;    // arm length from each corner
            let aax = fwidth(ax) + 1e-4;
            let aay = fwidth(ay) + 1e-4;
            // On the right/left edge (ax≈1) near a top/bottom corner (ay large).
            let vband = smoothstep(1.0 - ow - aax, 1.0 - ow, ax);
            let arm_v = vband * smoothstep(1.0 - bl, 1.0 - bl + 0.02, ay);
            // On the top/bottom edge (ay≈1) near a left/right corner (ax large).
            let hband = smoothstep(1.0 - ow - aay, 1.0 - ow, ay);
            let arm_h = hband * smoothstep(1.0 - bl, 1.0 - bl + 0.02, ax);
            let mask = clamp(max(arm_v, arm_h), 0.0, 1.0);
            return vec4<f32>(in.color.rgb, in.color.a * mask);
        }
        // Full rounded-rect frame.
        let d = rrect_sd(in.local, globals.params.w);
        let aa = fwidth(d) + 1e-4;
        let outline = 0.05;
        let ring = (1.0 - smoothstep(0.0, aa, d)) * smoothstep(0.0, aa, d + outline);
        return vec4<f32>(in.color.rgb, in.color.a * ring);
    }
    if in.kind > 0.5 {
        if globals.tex_flags.z > 0.5 {
            // Imported cursor/trail texture, tinted by the cursor colour.
            let t = textureSample(cursor_tex, skin_samp, uv);
            return vec4<f32>(in.color.rgb * t.rgb, t.a * in.color.a);
        }
        // Cursor / trail: a crisp filled circle.
        let r = length(in.local);
        let aa = fwidth(r) * 1.5 + 1e-4;
        let a = (1.0 - smoothstep(1.0 - aa, 1.0, r)) * in.color.a;
        return vec4<f32>(in.color.rgb, a);
    }
    if globals.tex_flags.x > 0.5 {
        // Imported note texture, tinted by the colorset colour (the game
        // multiplies a mostly-white note skin by the note colour).
        let t = textureSample(note_tex, skin_samp, uv);
        return vec4<f32>(in.color.rgb * t.rgb, t.a * in.color.a);
    }
    // Note: an outlined rounded rect (skin corner radius + outline) with a
    // faint dark interior fill, like the game's thin/square skins.
    let radius = globals.params.x;
    let outline = globals.params.y;
    let d = rrect_sd(in.local, radius);
    let aa = fwidth(d) + 1e-4;
    let outer = 1.0 - smoothstep(0.0, aa, d);            // inside the shape
    let inner = 1.0 - smoothstep(0.0, aa, d + outline);  // inside the hole
    let band = outer - inner;                            // the outline ring
    // Transparent interior: only the outline ring is drawn, so overlapping
    // notes show through (matching the game's thin skin).
    let alpha = band * in.color.a;
    return vec4<f32>(in.color.rgb, alpha);
}
