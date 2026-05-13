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

    // Fill keeps the spread-mix for crisp corner color.
    let spread = abs(sd - sd_alpha);
    let fill = mix(
        clamp(sd + 0.5, 0.0, 1.0),
        clamp(sd_alpha + 0.5, 0.0, 1.0),
        smoothstep(0.04, 0.12, spread),
    );

    // Glow tail uses alpha-only (true SDF). Soft halos don't benefit from median sharpness and
    // do suffer from median discontinuities at corner color-transitions.
    let outside_dist = max(-sd_alpha, 0.0);
    let u = clamp(outside_dist / radius, 0.0, 1.0);
    let halo = strength * pow(1.0 - u, 2.5);

    let halo_a = clamp(halo * gcol.a, 0.0, 1.0);
    let coverage = clamp(fill + halo_a * (1.0 - fill), 0.0, 1.0);
    let mt = fill / max(coverage, 1e-6);
    return vec4(mix(gcol.rgb, base_color.rgb, mt), coverage * base_color.a);
}
