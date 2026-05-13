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

    // Padding / neutral MSDF texels often sit on an almost flat plateau: ring_med is a small
    // constant across the whole quad. Growing `radius` used to shrink u = ring_med / tail_r and
    // leave pow(1-u) large everywhere → cyan rectangles. Kill glow where the field isn't changing.
    let dw = max(fwidth(sd), fwidth(sd_alpha));
    let edge_alive = smoothstep(3e-4, 0.028, dw);

    // Rim only near the true contour (does not widen without bound when radius grows).
    let onset_w = max(radius * 0.048, 0.09);
    let rim = smoothstep(0.0, onset_w, ring_med);

    // Falloff keyed directly to requested radius — larger r reaches farther, but same plateau u.
    let u = clamp(ring_med / max(radius * 1.06, 1e-3), 0.0, 1.0);
    let falloff = pow(max(1.0 - u, 0.0), 2.5);

    let radial_window = 1.0 - smoothstep(radius * 0.94, radius * 1.06, ring_med);

    let outward = pow(max(1.0 - ink, 0.0), 1.2);

    let halo = strength * rim * falloff * radial_window * outward * edge_alive;
    let glow_contrib = clamp(halo * gcol.a, 0.0, 1.0);

    let coverage = clamp(ink + glow_contrib * (1.0 - ink), 0.0, 1.0);
    let mix_t = ink / max(coverage, 1e-6);
    let rgb = mix(gcol.rgb, base_color.rgb, mix_t);
    return vec4(rgb, coverage * base_color.a);
}
