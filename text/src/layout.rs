//! Text shaping and layout. Wraps rustybuzz to produce a flat list of `ShapedGlyph`
//! entries with per-glyph metrics (em-space advance, width, bbox), then groups them
//! into a `ShapedRun` ready for rendering.

use rustybuzz::{shape, UnicodeBuffer};
use ttf_parser::GlyphId;

use crate::bands::{process_bands_with, BandsScratch};
use crate::cache::{GlyphCache, GlyphInfo};
use crate::font::Font;

/// One positioned glyph along a baseline. `x`/`y` are em-space pen positions
/// (glyph origin); `advance`/`width` are em-space metrics useful for layout.
#[derive(Clone, Copy, Debug)]
pub struct ShapedGlyph {
    pub glyph_id: u32,
    pub cluster: u32,
    pub x: f32,
    pub y: f32,
    pub x_advance: f32,
    pub y_advance: f32,
    /// Bounding-box width in em-space (max_x - min_x).
    pub width: f32,
    pub info: GlyphInfo,
}

/// A shaped, cached run of glyphs along a single baseline starting at (`origin_x`, `origin_y`).
#[derive(Clone, Debug)]
pub struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
    pub origin_x: f32,
    pub origin_y: f32,
    /// Total em-space advance summed across the run.
    pub total_advance: f32,
}

/// Shape `text` with the given font, ensuring all glyphs are populated in `cache`.
/// Pen positions are in em-space relative to (`start_x`, `start_y`).
pub fn shape_text(
    font: &Font,
    cache: &mut GlyphCache,
    text: &str,
    start_x: f32,
    start_y: f32,
) -> ShapedRun {
    let upem = font.units_per_em() as f32;

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let glyph_buffer = shape(font.face(), &[], buffer);

    let infos = glyph_buffer.glyph_infos();
    let positions = glyph_buffer.glyph_positions();

    let mut glyphs = Vec::with_capacity(infos.len());
    let mut x = start_x;
    let mut y = start_y;
    let mut scratch = BandsScratch::default();

    for (info, pos) in infos.iter().zip(positions.iter()) {
        let glyph_id = info.glyph_id;
        let gx = x + pos.x_offset as f32 / upem;
        let gy = y + pos.y_offset as f32 / upem;
        let x_adv = pos.x_advance as f32 / upem;
        let y_adv = pos.y_advance as f32 / upem;

        let cached = cache.get(glyph_id).or_else(|| {
            let outlines = font.load_glyph(GlyphId(glyph_id as u16))?;
            let band = process_bands_with(&outlines, &mut scratch);
            Some(cache.insert(glyph_id, band))
        });

        if let Some(gi) = cached {
            let width = (gi.bbox.2 - gi.bbox.0).max(0.0);
            glyphs.push(ShapedGlyph {
                glyph_id,
                cluster: info.cluster,
                x: gx,
                y: gy,
                x_advance: x_adv,
                y_advance: y_adv,
                width,
                info: gi,
            });
        }

        x += x_adv;
        y += y_adv;
    }

    ShapedRun {
        glyphs,
        origin_x: start_x,
        origin_y: start_y,
        total_advance: x - start_x,
    }
}
