//! Vertex format for MSDF text (pixel pos + atlas UV + color + material).

use bytemuck::{Pod, Zeroable};

use crate::atlas_format::GlyphRecord;
use crate::Material;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TextVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub mat_tag: u32,
    pub mat_p0: [f32; 4],
    pub mat_p1: [f32; 4],
    /// Font size in screen pixels (`TextArgs.size_px`); drives MSDF coverage scaling like atlas `distanceRange * (scale/em_to_px)`.
    pub size_px: f32,
}

pub(crate) fn glyph_quad_is_visible(record: &GlyphRecord) -> bool {
    // Skip degenerate em rects (space / invisible glyphs baked without ink).
    const MIN_EM: f32 = 1e-5;
    let dx = record.plane_max_em[0] - record.plane_min_em[0];
    let dy = record.plane_max_em[1] - record.plane_min_em[1];
    dx > MIN_EM && dy > MIN_EM
}

pub(crate) fn push_glyph_quad_pixels(
    out: &mut Vec<TextVertex>,
    record: &GlyphRecord,
    atlas_w: f32,
    atlas_h: f32,
    pen_x: f32,
    pen_y: f32,
    size_px: f32,
    color: [f32; 4],
    material: Material,
) {
    out.extend_from_slice(&glyph_quad_pixels(
        record,
        atlas_w,
        atlas_h,
        pen_x,
        pen_y,
        size_px,
        color,
        material,
    ));
}

fn glyph_quad_pixels(
    record: &GlyphRecord,
    atlas_w: f32,
    atlas_h: f32,
    pen_x: f32,
    pen_y: f32,
    size: f32,
    color: [f32; 4],
    material: Material,
) -> [TextVertex; 6] {
    let (tag, mat_p0, mat_p1) = material.pack_for_vertex();

    let min_x = record.plane_min_em[0];
    let min_y = record.plane_min_em[1];
    let max_x = record.plane_max_em[0];
    let max_y = record.plane_max_em[1];

    let u0 = record.uv_min[0] as f32;
    let v0 = record.uv_min[1] as f32;
    let u1 = record.uv_max[0] as f32;
    let v1 = record.uv_max[1] as f32;

    let denom_x = (max_x - min_x).max(1.0e-6);
    let denom_y = (max_y - min_y).max(1.0e-6);

    let uv_at = |ex: f32, ey: f32| {
        let s = (ex - min_x) / denom_x;
        // After atlas bake: smaller `v` / smaller atlas row = typographic top. Map em-space
        // bottom (`min_y`) → larger `v`, top (`max_y`) → smaller `v`.
        let t = (ey - min_y) / denom_y;
        let uf = u0 + s * (u1 - u0) + 0.5;
        let vf = v1 - t * (v1 - v0) + 0.5;
        [uf / atlas_w, vf / atlas_h]
    };

    let v = |ex: f32, ey: f32| TextVertex {
        pos: [pen_x + ex * size, pen_y - ey * size],
        uv: uv_at(ex, ey),
        color,
        mat_tag: tag,
        mat_p0,
        mat_p1,
        size_px: size,
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
