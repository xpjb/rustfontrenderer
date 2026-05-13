struct Globals {
    matrix: mat4x4<f32>,
    px_meta: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn main(in: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = globals.matrix * vec4<f32>(in.pos.x, in.pos.y, 0.0, 1.0);
    o.uv = in.uv;
    o.color = in.color;
    return o;
}
