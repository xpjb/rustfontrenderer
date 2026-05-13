// `px_meta.x` = baked atlas `distanceRange` (friend hardcodes `const MSDF_RANGE: f32 = 4.0` from JSON).
// `screen_px_range` = `px_meta.x * (glyph_scale_px / em_to_px)` ≡ `MSDF_RANGE * (scale / 64.0)` when 64 is atlas em density.
//
// Extra scale on that product: **< 1** = shallower coverage ramp = **softer AA** (less “aggressive” edge).
// **> 1** = sharper / narrower band. Does **not** add alpha-lerp rounding; it only scales the ramp steepness.
// i would definitely delete this if its not found to be useful soon
const MSDF_COVERAGE_RANGE_SCALE: f32 = 1.0;

struct Globals {
    matrix: mat4x4<f32>,
    px_meta: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_samp: sampler;

fn median3(a: f32, b: f32, c: f32) -> f32 {
    return max(min(a, b), min(max(a, b), c));
}

/// `.x` = raw median (`median3`) for outline — matches friend’s `let sd = median3(...)`.
/// `.y` / `.z` = spread-mixed channel and alpha for fill/glow/shadow corner fix.
fn unpack_sd_normalized(s: vec4<f32>) -> vec3<f32> {
    let md = median3(s.r, s.g, s.b);
    let spread = max(abs(s.r - s.g), max(abs(s.g - s.b), abs(s.r - s.b)));
    let dist_chan = mix(md, s.a, smoothstep(0.04, 0.12, spread));
    return vec3(md, dist_chan, s.a);
}

// {{include generated/ubershader_dispatch.wgsl}}

struct FsIn {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) mat_tag: u32,
    @location(3) @interpolate(flat) mat_p0: vec4<f32>,
    @location(4) @interpolate(flat) mat_p1: vec4<f32>,
    @location(5) @interpolate(flat) glyph_scale_px: f32,
}

@fragment
fn main(in: FsIn) -> @location(0) vec4<f32> {
    let s = textureSample(atlas_tex, atlas_samp, in.uv);
    let mp0 = in.mat_p0;
    let mp1 = in.mat_p1;
    let p = MaterialParams(mp0.x, mp0.y, mp0.z, mp0.w, mp1.x, mp1.y, mp1.z, mp1.w);
    let em_to_px = max(globals.px_meta.w, 1.0);
    let screen_px_range =
        globals.px_meta.x * (in.glyph_scale_px / em_to_px) * MSDF_COVERAGE_RANGE_SCALE;
    let sd_tri = unpack_sd_normalized(s);
    return dispatch_material(
        in.mat_tag,
        screen_px_range,
        sd_tri.x,
        sd_tri.y,
        sd_tri.z,
        in.color,
        p,
    );
}
