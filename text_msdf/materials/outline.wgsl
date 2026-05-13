//! name = "Outline"
//! params = [
//!   { ident = "width_px", ty = "f32" },
//!   { ident = "color", ty = "vec4<f32>" },
//! ]

fn material_outline(
    sd: f32,
    sd_alpha: f32,
    base_color: vec4<f32>,
    p: MaterialParams,
) -> vec4<f32> {
    let w = p.p0;
    let oc = vec4(p.p1, p.p2, p.p3, p.p4);
    let spread = abs(sd - sd_alpha);
    let fill = clamp(sd + 0.5, 0.0, 1.0);
    let fill_a = clamp(sd_alpha + 0.5, 0.0, 1.0);
    let ink = mix(fill, fill_a, smoothstep(0.04, 0.12, spread));
    let body = clamp(sd + w + 0.5, 0.0, 1.0);
    return vec4(mix(oc.rgb, base_color.rgb, ink), body * base_color.a);
}
