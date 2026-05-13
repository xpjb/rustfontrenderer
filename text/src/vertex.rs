//! Vertex format and quad generation for text rendering.
//!
//! Each glyph emits 6 vertices (two triangles) in em-space; the vertex shader
//! transforms by the caller's matrix. Per-vertex attributes carry packed glyph
//! cache locations so the pixel shader can fetch curve and band data.

use bytemuck::{Pod, Zeroable};

use crate::cache::GlyphInfo;

/// 56 bytes per vertex. Layout matches the WGSL shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TextVertex {
    /// em-space corner.
    pub pos: [f32; 2],
    /// Packed glyph metadata: band start and band max.
    pub glyph: [u32; 2],
    /// Glyph origin in em-space, used to derive glyph-local sample coord.
    pub jac: [f32; 2],
    /// bnd = (band_scale_x, band_scale_y, band_offset_x, band_offset_y).
    pub bnd: [f32; 4],
    /// col = RGBA.
    pub col: [f32; 4],
}

pub(crate) fn push_glyph_vertices(
    out: &mut Vec<TextVertex>,
    info: &GlyphInfo,
    pen_em_x: f32,
    pen_em_y: f32,
    color: [f32; 4],
) {
    out.extend_from_slice(&glyph_quad(info, pen_em_x, pen_em_y, color));
}

fn glyph_quad(info: &GlyphInfo, px: f32, py: f32, color: [f32; 4]) -> [TextVertex; 6] {
    let (min_x, min_y, max_x, max_y) = info.bbox;
    let (bx, by) = (px + min_x, py + min_y);
    let (bw, bh) = (max_x - min_x, max_y - min_y);

    let num_bands_x = info.band_max.0 + 1;
    let num_bands_y = info.band_max.1 + 1;
    let scale_x = if bw > 0.0001 { num_bands_x as f32 / bw } else { 1.0 };
    let scale_y = if bh > 0.0001 { num_bands_y as f32 / bh } else { 1.0 };
    let off_x = -min_x * scale_x;
    let off_y = -min_y * scale_y;

    let gx = info.band_start.0 as u32;
    let gy = info.band_start.1 as u32;
    let glyph = [
        (gy << 16) | gx,
        ((info.band_max.1 & 0xFF) << 16) | (info.band_max.0 & 0xFF),
    ];

    let jac = [px, py];
    let bnd = [scale_x, scale_y, off_x, off_y];

    let corners = [
        (bx, by),
        (bx + bw, by),
        (bx + bw, by + bh),
        (bx, by),
        (bx + bw, by + bh),
        (bx, by + bh),
    ];

    let mut out = [TextVertex { pos: [0.0; 2], glyph, jac, bnd, col: color }; 6];
    for (i, (cx, cy)) in corners.iter().enumerate() {
        out[i] = TextVertex {
            pos: [*cx, *cy],
            glyph,
            jac,
            bnd,
            col: color,
        };
    }
    out
}
