//! Vertex format and quad generation for text rendering.
//!
//! `pos` is in window pixels (`0..width`, `0..height`). `loc_em` stays in glyph-local
//! em coordinates for the Slug fragment shader (curve atlas is em-space). Y matches the
//! old `scale(_, -font_size, _)` convention via `pen_y - ey * size`.

use bytemuck::{Pod, Zeroable};

use crate::cache::GlyphInfo;

/// 56 bytes per vertex. Layout matches the WGSL shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TextVertex {
    /// Window-space pixel position (matches orthographic `0..width`, `0..height`).
    pub pos: [f32; 2],
    /// Packed glyph metadata: band start and band max.
    pub glyph: [u32; 2],
    /// Glyph-local em-space offset from the pen (Slug sampling coord).
    pub loc_em: [f32; 2],
    /// bnd = (band_scale_x, band_scale_y, band_offset_x, band_offset_y) in em-space.
    pub bnd: [f32; 4],
    /// col = RGBA.
    pub col: [f32; 4],
}

pub(crate) fn push_glyph_quad_pixels(
    out: &mut Vec<TextVertex>,
    info: &GlyphInfo,
    pen_x: f32,
    pen_y: f32,
    size: f32,
    color: [f32; 4],
) {
    out.extend_from_slice(&glyph_quad_pixels(info, pen_x, pen_y, size, color));
}

fn glyph_quad_pixels(
    info: &GlyphInfo,
    pen_x: f32,
    pen_y: f32,
    size: f32,
    color: [f32; 4],
) -> [TextVertex; 6] {
    let (min_x, min_y, max_x, max_y) = info.bbox;
    let bw = max_x - min_x;
    let bh = max_y - min_y;
    let scale_x = if bw > 0.0001 { (info.band_max.0 + 1) as f32 / bw } else { 1.0 };
    let scale_y = if bh > 0.0001 { (info.band_max.1 + 1) as f32 / bh } else { 1.0 };
    let bnd = [scale_x, scale_y, -min_x * scale_x, -min_y * scale_y];
    let glyph = [
        ((info.band_start.1 as u32) << 16) | info.band_start.0 as u32,
        ((info.band_max.1 & 0xFF) << 16) | (info.band_max.0 & 0xFF),
    ];
    let v = |ex, ey| TextVertex {
        pos: [pen_x + ex * size, pen_y - ey * size],
        glyph,
        loc_em: [ex, ey],
        bnd,
        col: color,
    };
    [
        v(min_x, min_y),
        v(max_x, min_y),
        v(max_x, max_y),
        v(min_x, min_y),
        v(max_x, max_y),
        v(min_x, max_y),
    ]
}
