//! Serialized atlas payload (`atlas.bin`). Shared by `build.rs` (via `#[path]`) and the library.

use serde::{Deserialize, Serialize};

pub const ATLAS_MAGIC: &[u8; 4] = b"MSDF";
pub const ATLAS_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AtlasFile {
    pub header: AtlasHeader,
    pub glyphs: Vec<GlyphRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AtlasHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub font_hash: [u8; 32],
    pub atlas_w: u32,
    pub atlas_h: u32,
    pub glyph_px: u32,
    pub distance_range_px: f32,
    pub units_per_em: u16,
    pub ascent_em: f32,
    pub descent_em: f32,
    pub line_gap_em: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GlyphRecord {
    pub glyph_id: u32,
    pub uv_min: [u16; 2],
    pub uv_max: [u16; 2],
    pub plane_min_em: [f32; 2],
    pub plane_max_em: [f32; 2],
    pub advance_em: f32,
}
