//! name = "Glow"
//! params = [
//!   { ident = "radius_px", ty = "f32" },
//!   { ident = "color", ty = "vec4<f32>" },
//!   { ident = "strength", ty = "f32" },
//! ]

fn material_glow(
    sd: f32,
    sd_alpha: f32,
    base_color: vec4<f32>,
    p: MaterialParams,
) -> vec4<f32> {
    let radius = max(p.p0, 0.05);
    let gcol = vec4(p.p1, p.p2, p.p3, p.p4);
    let strength = p.p5;
    let spread = abs(sd - sd_alpha);
    let fill = clamp(sd + 0.5, 0.0, 1.0);
    let fill_a = clamp(sd_alpha + 0.5, 0.0, 1.0);
    let ink = mix(fill, fill_a, smoothstep(0.04, 0.12, spread));
    let ring_med = max(-sd, 0.0);
    let ring_alpha = max(-sd_alpha, 0.0);
    let glow_dist = max(ring_med, ring_alpha * 0.45);
    let t = clamp(glow_dist / radius, 0.0, 1.0);
    let halo = (1.0 - t) * (1.0 - t) * strength;
    let glow_contrib = clamp(halo * gcol.a, 0.0, 1.0);
    let coverage = clamp(ink + glow_contrib * (1.0 - ink), 0.0, 1.0);
    let mix_t = ink / max(coverage, 1e-6);
    let rgb = mix(gcol.rgb, base_color.rgb, mix_t);
    return vec4(rgb, coverage * base_color.a);
}
