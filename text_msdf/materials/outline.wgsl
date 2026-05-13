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

    // Hard outline: outline band follows the median MSDF distance (`sd`).
    // No plateau-kill needed: build.rs sets DISTANCE_RANGE_PX big enough that the natural
    // `clamp(... + 0.5)` fade lands inside the encoded range.
    let body = clamp(sd + w + 0.5, 0.0, 1.0);

    // Softer outline option (alpha / true-SDF channel): use `sd_alpha` for `body` instead of `sd`.
    // The median can produce “horns” at stroke ends — different cap segments get different edge
    // colors and the median may not pick the visually nearest edge — but `sd_alpha` is monotonic
    // and avoids that bleed. Trade-off: slightly rounder joins vs. sharper MSDF outline geometry.
    // let body = clamp(sd_alpha + w + 0.5, 0.0, 1.0);

    // Fill blends median and alpha by spread so corner color stays crisp (same trick as fill.wgsl).
    let spread = abs(sd - sd_alpha);
    let fill = mix(
        clamp(sd + 0.5, 0.0, 1.0),
        clamp(sd_alpha + 0.5, 0.0, 1.0),
        smoothstep(0.04, 0.12, spread),
    );

    return vec4(
        mix(oc.rgb, base_color.rgb, fill),
        body * base_color.a,
    );
}
