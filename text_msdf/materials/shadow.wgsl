//! name = "Shadow"
//! params = [
//!   { ident = "offset_px", ty = "vec2<f32>" },
//!   { ident = "blur_px", ty = "f32" },
//!   { ident = "color", ty = "vec4<f32>" },
//! ]

fn material_shadow(
    sd: f32,
    sd_alpha: f32,
    _base_color: vec4<f32>,
    p: MaterialParams,
) -> vec4<f32> {
    let blur = max(p.p2, 1e-3);
    let sc = vec4(p.p3, p.p4, p.p5, p.p6);
    let spread = abs(sd - sd_alpha);
    let core = clamp(sd + 0.5, 0.0, 1.0);
    let core_a = clamp(sd_alpha + 0.5, 0.0, 1.0);
    let ink = mix(core, core_a, smoothstep(0.04, 0.12, spread));
    let dilated = clamp(sd + blur + 0.5, 0.0, 1.0);
    let dil_a = clamp(sd_alpha + blur + 0.5, 0.0, 1.0);
    let dilated_ink = mix(dilated, dil_a, smoothstep(0.04, 0.12, spread));
    let shadow_only = dilated_ink * (1.0 - ink * 0.85);
    let a = clamp(shadow_only * sc.a, 0.0, 1.0);
    return vec4(sc.rgb, a);
}
