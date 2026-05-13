//! name = "Outline"
//! params = [
//!   { ident = "width_px", ty = "f32" },
//!   { ident = "color", ty = "vec4<f32>" },
//! ]

fn material_outline(
    sd: f32,
    sd_alpha: f32,
    aa: f32,
    uv: vec2<f32>,
    base_color: vec4<f32>,
    s: vec4<f32>,
    s_sh: vec4<f32>,
    p: MaterialParams,
    px_to_sd: f32,
) -> vec4<f32> {
    let w = p.p0 * px_to_sd;
    let oc = vec4(p.p1, p.p2, p.p3, p.p4);
    let fill = clamp(sd / aa + 0.5, 0.0, 1.0);
    let body = clamp((sd + w) / aa + 0.5, 0.0, 1.0);
    return vec4(mix(oc.rgb, base_color.rgb, fill), body * base_color.a);
}
