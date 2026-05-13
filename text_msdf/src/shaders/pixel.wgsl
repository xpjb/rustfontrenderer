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

fn unpack_sd_tri(s: vec4<f32>, uv: vec2<f32>) -> vec3<f32> {
    let px_range = globals.px_meta.x;
    let atlas_sz = globals.px_meta.yz;
    let md = median3(s.r, s.g, s.b);
    let spread = max(abs(s.r - s.g), max(abs(s.g - s.b), abs(s.r - s.b)));
    let dist_chan = mix(md, s.a, smoothstep(0.04, 0.12, spread));
    let sd_texels = (dist_chan - 0.5) * px_range;
    let sd_a_texels = (s.a - 0.5) * px_range;
    let fw = max(fwidth(uv), vec2<f32>(1e-6));
    let sigma_est = 0.5 * dot(fw, atlas_sz);
    let sigma = max(sigma_est, px_range * 0.35);
    let sd = sd_texels / sigma;
    let sd_a = sd_a_texels / sigma;
    return vec3(sd, sd_a, sigma);
}

// {{include generated/ubershader_dispatch.wgsl}}

struct FsIn {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) mat_tag: u32,
    @location(3) @interpolate(flat) mat_p0: vec4<f32>,
    @location(4) @interpolate(flat) mat_p1: vec4<f32>,
}

@fragment
fn main(in: FsIn) -> @location(0) vec4<f32> {
    let s = textureSample(atlas_tex, atlas_samp, in.uv);
    var s_sh = s;
    let mp0 = in.mat_p0;
    let mp1 = in.mat_p1;
    let p = MaterialParams(mp0.x, mp0.y, mp0.z, mp0.w, mp1.x, mp1.y, mp1.z, mp1.w);
    if (in.mat_tag == MATERIAL_TAG_SHADOW) {
        s_sh = textureSample(atlas_tex, atlas_samp, in.uv + vec2(p.p0, p.p1));
    }
    let tri = unpack_sd_tri(s, in.uv);
    let sd = tri.x;
    let sd_a = tri.y;
    let px_range = globals.px_meta.x;
    // Map author-facing pixel widths (outline/glow/blur) into the same units as `sd`.
    let px_to_sd = tri.z / max(px_range, 1e-3);
    let aa = 1.0;
    return dispatch_material(in.mat_tag, sd, sd_a, aa, in.uv, in.color, s, s_sh, p, px_to_sd);
}
