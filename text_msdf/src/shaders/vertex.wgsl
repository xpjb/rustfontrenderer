struct Globals {
    matrix: mat4x4<f32>,
    /// `x` = atlas SDF span in texels, `y`/`z` = atlas size, `w` = `em_to_px` (atlas density D).
    px_meta: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mat_tag: u32,
    @location(4) mat_p0: vec4<f32>,
    @location(5) mat_p1: vec4<f32>,
    @location(6) size_px: f32,
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    // Must be flat: same for all verts of a glyph, and u32 must not be linear-smoothed.
    @location(2) @interpolate(flat) mat_tag: u32,
    @location(3) @interpolate(flat) mat_p0: vec4<f32>,
    @location(4) @interpolate(flat) mat_p1: vec4<f32>,
    @location(5) @interpolate(flat) glyph_scale_px: f32,
}

@vertex
fn main(in: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = globals.matrix * vec4<f32>(in.pos.x, in.pos.y, 0.0, 1.0);
    o.uv = in.uv;
    o.color = in.color;
    o.mat_tag = in.mat_tag;
    o.mat_p0 = in.mat_p0;
    o.mat_p1 = in.mat_p1;
    o.glyph_scale_px = in.size_px;
    return o;
}
