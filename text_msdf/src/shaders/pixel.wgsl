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

fn sdf_coverage(s: vec4<f32>, uv_center: vec2<f32>) -> f32 {
    let px_range = globals.px_meta.x;
    let atlas_sz = globals.px_meta.yz;
    let md = median3(s.r, s.g, s.b);
    let spread = max(abs(s.r - s.g), max(abs(s.g - s.b), abs(s.r - s.b)));
    let dist_chan = mix(md, s.a, smoothstep(0.04, 0.12, spread));
    let sd_texels = (dist_chan - 0.5) * px_range;
    let fw = max(fwidth(uv_center), vec2<f32>(1e-6));
    let sigma_texels = max(0.5 * dot(fw, atlas_sz), 1e-6);
    return clamp(sd_texels / sigma_texels + 0.5, 0.0, 1.0);
}

struct FsIn {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@fragment
fn main(in: FsIn) -> @location(0) vec4<f32> {
    let s = textureSample(atlas_tex, atlas_samp, in.uv);
    let a = sdf_coverage(s, in.uv);
    return vec4<f32>(in.color.rgb, a * in.color.a);
}
