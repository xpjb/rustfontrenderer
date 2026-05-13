//! name = "Outline"
//! params = [
//!   { ident = "width_px", ty = "f32" },
//!   { ident = "color", ty = "vec4<f32>" },
//! ]

fn material_outline(
    screen_px_range: f32,
    sd_med: f32,
    _sd: f32,
    _sd_alpha: f32,
    base_color: vec4<f32>,
    p: MaterialParams,
) -> vec4<f32> {
    let w = p.p0;
    let oc = vec4(p.p1, p.p2, p.p3, p.p4);

    // Friend `text.wgsl`: median `sd`, one shared coverage `alpha`, outline = same clamp + `outline_width`.
    let alpha = clamp(screen_px_range * (sd_med - 0.5) + 0.5, 0.0, 1.0);

    if w > 0.01 && oc.a > 0.001 {
        let fill_a = alpha * base_color.a;
        let outline_a =
            clamp(screen_px_range * (sd_med - 0.5) + 0.5 + w, 0.0, 1.0) * oc.a;
        let total = max(fill_a, outline_a);
        if total < 0.01 {
            discard;
        }
        let col = mix(oc.rgb, base_color.rgb, fill_a / max(total, 0.001));
        return vec4(col * total, total);
    }

    return vec4(base_color.rgb, alpha * base_color.a);
}
