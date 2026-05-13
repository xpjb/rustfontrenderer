//! name = "Fill"
//! params = []

fn material_fill(
    sd: f32,
    sd_alpha: f32,
    base_color: vec4<f32>,
    _p: MaterialParams,
) -> vec4<f32> {
    let spread = abs(sd - sd_alpha);
    let fill = clamp(sd + 0.5, 0.0, 1.0);
    let fill_a = clamp(sd_alpha + 0.5, 0.0, 1.0);
    let a = mix(fill, fill_a, smoothstep(0.04, 0.12, spread));
    return vec4(base_color.rgb, a * base_color.a);
}
