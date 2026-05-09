//! Glyph cache: combines per-glyph band data into shared GPU textures.

use crate::bands::{BandData, BAND_TEXTURE_WIDTH};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct GlyphInfo {
    pub curve_start: (u32, u32),
    pub band_start: (u32, u32),
    pub band_max: (u32, u32),
    pub bbox: (f32, f32, f32, f32),
}

pub struct GlyphCache {
    curve_texels: Vec<[f32; 4]>,
    band_texels: Vec<[u32; 4]>,
    curve_width: u32,
    curve_height: u32,
    band_width: u32,
    band_height: u32,
    glyphs: HashMap<u32, GlyphInfo>,
}

impl GlyphCache {
    pub fn new() -> Self {
        Self {
            curve_texels: Vec::new(),
            band_texels: Vec::new(),
            curve_width: BAND_TEXTURE_WIDTH,
            curve_height: 0,
            band_width: BAND_TEXTURE_WIDTH,
            band_height: 0,
            glyphs: HashMap::new(),
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
        let num_bands = (band_data.band_max.0.max(band_data.band_max.1) + 1) as usize;
        let header_count = num_bands * 2;
        let curve_start = self.alloc_curves(&band_data.curve_texels);
        let band_start = self.alloc_bands(&band_data.band_texels, header_count, curve_start);
        let info = GlyphInfo {
            curve_start,
            band_start,
            band_max: band_data.band_max,
            bbox: band_data.bbox,
        };
        self.glyphs.insert(glyph_id, info);
        info
    }

    fn alloc_curves(&mut self, texels: &[[f32; 4]]) -> (u32, u32) {
        let start = self.curve_texels.len();
        let col = (start % BAND_TEXTURE_WIDTH as usize) as u32;
        let row = (start / BAND_TEXTURE_WIDTH as usize) as u32;
        self.curve_texels.extend_from_slice(texels);
        let end = self.curve_texels.len();
        let end_row = (end + BAND_TEXTURE_WIDTH as usize - 1) / BAND_TEXTURE_WIDTH as usize;
        self.curve_height = self.curve_height.max(end_row as u32);
        (col, row)
    }

    fn alloc_bands(
        &mut self,
        texels: &[[u32; 4]],
        header_count: usize,
        curve_start: (u32, u32),
    ) -> (u32, u32) {
        let start = self.band_texels.len();
        let col = (start % BAND_TEXTURE_WIDTH as usize) as u32;
        let row = (start / BAND_TEXTURE_WIDTH as usize) as u32;
        for (i, t) in texels.iter().enumerate() {
            let mut tc = *t;
            if i >= header_count {
                tc[0] = t[0].saturating_add(curve_start.0);
                tc[1] = t[1].saturating_add(curve_start.1);
            }
            self.band_texels.push(tc);
        }
        let end = self.band_texels.len();
        let end_row = (end + BAND_TEXTURE_WIDTH as usize - 1) / BAND_TEXTURE_WIDTH as usize;
        self.band_height = self.band_height.max(end_row as u32);
        (col, row)
    }

    pub fn curve_data(&self) -> &[[f32; 4]] { &self.curve_texels }
    pub fn band_data(&self) -> &[[u32; 4]] { &self.band_texels }
    pub fn curve_size(&self) -> (u32, u32) { (self.curve_width, self.curve_height.max(1)) }
    pub fn band_size(&self) -> (u32, u32) { (self.band_width, self.band_height.max(1)) }
}

impl Default for GlyphCache {
    fn default() -> Self { Self::new() }
}
