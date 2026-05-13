//! name = "Shadow"
//! params = [
//!   { ident = "offset_uv", ty = "vec2<f32>" },
//!   { ident = "blur_px", ty = "f32" },
//!   { ident = "color", ty = "vec4<f32>" },
//! ]

fn material_shadow(
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
    let blur = max(p.p2 * px_to_sd, 0.001);
    let sc = vec4(p.p3, p.p4, p.p5, p.p6);
    let tri_sh = unpack_sd_tri(s_sh, uv);
    let sd_sh = tri_sh.x;
    let core = clamp(sd / aa + 0.5, 0.0, 1.0);
    let shadow_mask = clamp(sd_sh / aa + 0.5 + blur * 0.45, 0.0, 1.0);
    let shadow = shadow_mask * (1.0 - core * 0.55);
    let rgb = base_color.rgb * core + sc.rgb * shadow * sc.a;
    let a = clamp(core * base_color.a + shadow * sc.a, 0.0, 1.0);
    return vec4(rgb, a);
}
