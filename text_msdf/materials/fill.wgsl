//! name = "Fill"
//! params = []

fn material_fill(
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
    let a = clamp(sd / aa + 0.5, 0.0, 1.0);
    // `px_to_sd` unused; kept for a uniform signature across materials.
    let a2 = a + px_to_sd * 0.0;
    return vec4(base_color.rgb, a2 * base_color.a);
}
