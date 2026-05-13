//! Text shaping and layout. Wraps rustybuzz to produce a flat list of `ShapedGlyph`
//! entries, then groups them into a `ShapedRun`.

use rustybuzz::{shape, UnicodeBuffer};
use ttf_parser::GlyphId;

use crate::bands::{process_bands_with, BandsScratch};
use crate::cache::{GlyphCache, GlyphInfo};
use crate::font::Font;

/// One positioned glyph along a baseline (`x`/`y` are em-space pen positions).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShapedGlyph {
    pub glyph_id: u32,
    pub x: f32,
    pub y: f32,
    pub info: GlyphInfo,
}

/// A shaped run of glyphs along a single baseline.
#[derive(Clone, Debug)]
pub(crate) struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
}

/// Shape `text` with the given font, ensuring all glyphs are populated in `cache`.
/// Pen positions are in em-space relative to (`start_x`, `start_y`).
pub(crate) fn shape_text(
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
            glyphs.push(ShapedGlyph {
                glyph_id,
                x: gx,
                y: gy,
                info: gi,
            });
        }

        x += x_adv;
        y += y_adv;
    }

    ShapedRun { glyphs }
}
