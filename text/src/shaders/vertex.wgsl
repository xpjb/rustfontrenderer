// Vertex shader for the Slug-style font renderer.
// Translated from Lengyel's reference HLSL/WGSL.

struct Params {
    matrix: mat4x4f,
    viewport: vec4f,
}

@group(0) @binding(0) var<uniform> params: Params;

struct VsIn {
    @location(0) pos: vec4f,
    @location(1) tex: vec4f,
    @location(2) jac: vec4f,
    @location(3) bnd: vec4f,
    @location(4) col: vec4f,
}

struct VsOut {
    @builtin(position) position: vec4f,
    @location(0) color: vec4f,
    @location(1) texcoord: vec2f,
    @location(2) @interpolate(flat) banding: vec4f,
    @location(3) @interpolate(flat) glyph: vec4i,
}

@vertex
fn main(input: VsIn) -> VsOut {
    var out: VsOut;

    let vpos = vec2f(input.pos.x, input.pos.y);
    // Glyph-local sample coord for curve lookup (curves stored per-glyph in local em-space).
    let texcoord = vec2f(input.pos.x - input.jac.x, input.pos.y - input.jac.y);

    out.texcoord = texcoord;
    out.position = params.matrix * vec4f(vpos.x, vpos.y, 0.0, 1.0);

    let gx_bits = bitcast<u32>(input.tex.z);
    let gy_bits = bitcast<u32>(input.tex.w);
    out.glyph = vec4i(
        i32(gx_bits & 0xFFFFu),
        i32(gx_bits >> 16u),
        i32(gy_bits & 0xFFFFu),
        i32(gy_bits >> 16u),
    );
    out.banding = input.bnd;
    out.color = input.col;
    return out;
}
