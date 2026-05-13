//! Glyph cache: combines per-glyph band data into shared GPU textures.

use std::collections::HashMap;

use crate::bands::{BandData, BAND_TEXTURE_WIDTH, CURVE_TEXTURE_WIDTH};

#[derive(Clone, Copy, Debug)]
pub(crate) struct GlyphInfo {
    pub curve_start: (u32, u32),
    pub band_start: (u32, u32),
    pub band_max: (u32, u32),
    pub bbox: (f32, f32, f32, f32),
}

pub struct GlyphCache {
    curve_texels: Vec<[u16; 4]>,
    band_texels: Vec<[u16; 2]>,
    curve_width: u32,
    curve_height: u32,
    band_width: u32,
    band_height: u32,
    glyphs: HashMap<u32, GlyphInfo>,
    revision: u64,
}

impl GlyphCache {
    pub fn new() -> Self {
        Self {
            curve_texels: Vec::new(),
            band_texels: Vec::new(),
            curve_width: CURVE_TEXTURE_WIDTH,
            curve_height: 0,
            band_width: BAND_TEXTURE_WIDTH,
            band_height: 0,
            glyphs: HashMap::new(),
            revision: 0,
        }
    }

    pub fn get(&self, glyph_id: u32) -> Option<GlyphInfo> {
        self.glyphs.get(&glyph_id).copied()
    }

    pub fn contains(&self, glyph_id: u32) -> bool {
        self.glyphs.contains_key(&glyph_id)
    }

    /// Insert a glyph's processed band data; returns its assigned `GlyphInfo`.
    pub fn insert(&mut self, glyph_id: u32, band_data: BandData) -> GlyphInfo {
        let header_count =
            (band_data.band_max.0 + 1 + band_data.band_max.1 + 1) as usize;
        let curve_start = self.alloc_curves(&band_data.curve_texels);
        let band_start = self.alloc_bands(&band_data.band_texels, header_count, curve_start);
        let info = GlyphInfo {
            curve_start,
            band_start,
            band_max: band_data.band_max,
            bbox: band_data.bbox,
        };
        self.glyphs.insert(glyph_id, info);
        self.revision = self.revision.wrapping_add(1);
        info
    }

    fn alloc_curves(&mut self, texels: &[[u16; 4]]) -> (u32, u32) {
        let w = CURVE_TEXTURE_WIDTH as usize;
        let start = self.curve_texels.len();
        let col = (start % w) as u32;
        let row = (start / w) as u32;
        self.curve_texels.extend_from_slice(texels);
        let end = self.curve_texels.len();
        let end_row = (end + w - 1) / w;
        self.curve_height = self.curve_height.max(end_row as u32);
        (col, row)
    }

    fn alloc_bands(
        &mut self,
        texels: &[[u16; 2]],
        header_count: usize,
        curve_start: (u32, u32),
    ) -> (u32, u32) {
        let w = BAND_TEXTURE_WIDTH as usize;
        let start = self.band_texels.len();
        let col = (start % w) as u32;
        let row = (start / w) as u32;
        for (i, t) in texels.iter().enumerate() {
            let mut tc = *t;
            if i >= header_count {
                let abs = offset_curve_coord(curve_start, (t[0] as u32, t[1] as u32));
                tc[0] = abs.0 as u16;
                tc[1] = abs.1 as u16;
            }
            self.band_texels.push(tc);
        }
        let end = self.band_texels.len();
        let end_row = (end + w - 1) / w;
        self.band_height = self.band_height.max(end_row as u32);
        (col, row)
    }

    pub fn curve_data(&self) -> &[[u16; 4]] { &self.curve_texels }
    pub fn band_data(&self) -> &[[u16; 2]] { &self.band_texels }
    pub fn curve_size(&self) -> (u32, u32) { (self.curve_width, self.curve_height.max(1)) }
    pub fn band_size(&self) -> (u32, u32) { (self.band_width, self.band_height.max(1)) }
    pub fn revision(&self) -> u64 { self.revision }
}

fn offset_curve_coord(start: (u32, u32), local: (u32, u32)) -> (u32, u32) {
    let w = CURVE_TEXTURE_WIDTH as usize;
    let absolute = start.1 as usize * w
        + start.0 as usize
        + local.1 as usize * w
        + local.0 as usize;
    ((absolute % w) as u32, (absolute / w) as u32)
}

impl Default for GlyphCache {
    fn default() -> Self { Self::new() }
}
