//! Rustybuzz shaping + atlas glyph lookup.

use std::collections::{HashMap, HashSet};

use rustybuzz::{
    shape, Direction, Feature, UnicodeBuffer,
    script,
};
use ttf_parser::Tag;

use crate::atlas_format::GlyphRecord;
use crate::font::Font;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ShapedGlyph {
    pub glyph_id: u32,
    pub x: f32,
    pub y: f32,
    pub atlas: GlyphRecord,
}

#[derive(Clone, Debug)]
pub(crate) struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
}

pub(crate) fn shape_text(
    font: &Font,
    glyph_table: &HashMap<u32, GlyphRecord>,
    fallback: GlyphRecord,
    warned: &mut HashSet<u32>,
    text: &str,
    start_x: f32,
    start_y: f32,
) -> ShapedRun {
    let upem = font.units_per_em() as f32;

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(Direction::LeftToRight);
    buffer.set_script(script::LATIN);
    buffer.set_language("en".parse().expect("language tag"));
    let features = [
        Feature::new(Tag::from_bytes(b"fwid"), 0, ..),
        Feature::new(Tag::from_bytes(b"halt"), 0, ..),
    ];
    let glyph_buffer = shape(font.face(), &features, buffer);

    let infos = glyph_buffer.glyph_infos();
    let positions = glyph_buffer.glyph_positions();

    let mut glyphs = Vec::with_capacity(infos.len());
    let mut x = start_x;
    let mut y = start_y;

    for (info, pos) in infos.iter().zip(positions.iter()) {
        let glyph_id = info.glyph_id;
        let gx = x + pos.x_offset as f32 / upem;
        let gy = y + pos.y_offset as f32 / upem;
        let x_adv = pos.x_advance as f32 / upem;
        let y_adv = pos.y_advance as f32 / upem;

        let atlas = glyph_table.get(&glyph_id).copied().unwrap_or_else(|| {
            if warned.insert(glyph_id) {
                eprintln!(
                    "text_msdf: glyph_id {} missing from MSDF atlas; substituting '?'. \
                     Add it to charset.txt and rebuild.",
                    glyph_id
                );
            }
            fallback
        });

        glyphs.push(ShapedGlyph {
            glyph_id,
            x: gx,
            y: gy,
            atlas,
        });

        x += x_adv;
        y += y_adv;
    }

    ShapedRun { glyphs }
}

/// Atlas-backed fallback glyph (`ch`), normally `'?'`.
pub(crate) fn fallback_glyph_record(
    glyph_table: &HashMap<u32, GlyphRecord>,
    font: &Font,
    ch: char,
) -> GlyphRecord {
    if let Some(gid) = font.glyph_index(ch) {
        if let Some(rec) = glyph_table.get(&(gid.0 as u32)) {
            return *rec;
        }
    }
    GlyphRecord {
        glyph_id: 0,
        uv_min: [0, 0],
        uv_max: [0, 0],
        plane_min_em: [0.0, 0.0],
        plane_max_em: [0.05, 0.1],
        advance_em: 0.25,
    }
}
