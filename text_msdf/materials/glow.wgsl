//! name = "Glow"
//! params = [
//!   { ident = "radius_px", ty = "f32" },
//!   { ident = "color", ty = "vec4<f32>" },
//!   { ident = "strength", ty = "f32" },
//! ]

fn material_glow(
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
    let radius = max(p.p0 * px_to_sd, 0.05);
    let gcol = vec4(p.p1, p.p2, p.p3, p.p4);
    let strength = p.p5;
    let fill = clamp(sd / aa + 0.5, 0.0, 1.0);
    // Outside the edge: median `sd` and MTSDF `sd_alpha` both go negative. Prefer whichever
    // gives a cleaner halo (Latin fonts often have a dull alpha channel vs RGB median).
    let ring_med = max(-sd, 0.0);
    let ring_alpha = max(-sd_alpha, 0.0);
    let glow_dist = max(ring_med, ring_alpha * 0.45);
    let t = clamp(glow_dist / radius, 0.0, 1.0);
    let halo = (1.0 - t) * (1.0 - t) * strength;
    let glow_contrib = halo * gcol.a;
    let rgb = base_color.rgb * fill + gcol.rgb * glow_contrib * (1.0 - fill);
    let a = clamp(fill * base_color.a + glow_contrib * (1.0 - fill * 0.85), 0.0, 1.0);
    return vec4(rgb, a);
}
