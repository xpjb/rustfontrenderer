//! Vertex format and quad generation for text rendering.
//!
//! Each glyph emits 6 vertices (two triangles) in em-space; the vertex shader
//! transforms by the caller's matrix. Per-vertex attributes carry packed glyph
//! cache locations so the pixel shader can fetch curve and band data.

use bytemuck::{Pod, Zeroable};
use crate::cache::GlyphInfo;
use crate::layout::ShapedRun;

/// 5 × vec4 = 80 bytes per vertex. Layout matches the WGSL shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TextVertex {
    /// pos.xy = em-space corner; pos.zw = corner normal (unused unless dilation enabled).
    pub pos: [f32; 4],
    /// tex.xy = em-space sample coord; tex.zw = packed glyph metadata (locations + band_max).
    pub tex: [f32; 4],
    /// jac.xy = glyph origin (px, py) used to derive glyph-local sample coord.
    pub jac: [f32; 4],
    /// bnd = (band_scale_x, band_scale_y, band_offset_x, band_offset_y).
    pub bnd: [f32; 4],
    /// col = RGBA.
    pub col: [f32; 4],
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
    let tex_z = f32::from_bits((gy << 16) | gx);
    let tex_w = f32::from_bits(((info.band_max.1 & 0xFF) << 16) | (info.band_max.0 & 0xFF));

    let jac = [px, py, 0.0, 0.0];
    let bnd = [scale_x, scale_y, off_x, off_y];

    let corners = [
        (bx, by, -1.0, -1.0),
        (bx + bw, by, 1.0, -1.0),
        (bx + bw, by + bh, 1.0, 1.0),
        (bx, by, -1.0, -1.0),
        (bx + bw, by + bh, 1.0, 1.0),
        (bx, by + bh, -1.0, 1.0),
    ];

    let mut out = [TextVertex { pos: [0.0; 4], tex: [0.0; 4], jac, bnd, col: color }; 6];
    for (i, (cx, cy, nx, ny)) in corners.iter().enumerate() {
        out[i] = TextVertex {
            pos: [*cx, *cy, *nx, *ny],
            tex: [*cx, *cy, tex_z, tex_w],
            jac,
            bnd,
            col: color,
        };
    }
    out
}

/// Build a flat vertex list for one or more shaped runs.
pub fn build_run_vertices(runs: &[(&ShapedRun, [f32; 4])]) -> Vec<TextVertex> {
    let mut out = Vec::new();
    for (run, color) in runs {
        for g in &run.glyphs {
            for v in glyph_quad(&g.info, g.x, g.y, *color) {
                out.push(v);
            }
        }
    }
    out
}
