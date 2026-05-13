//! Serialized atlas payload (`atlas.bin`). Shared by `build.rs` (via `#[path]`) and the library.

use serde::{Deserialize, Serialize};

pub const ATLAS_MAGIC: &[u8; 4] = b"MSDF";
pub const ATLAS_FORMAT_VERSION: u32 = 2;

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
    /// Atlas sampling density `D`: texels span per **1 em** of glyph-space (often mislabeled “glyph px”).
    pub em_to_px: u32,
    /// MSDF-encoded **half-span** along each axis from the glyph ink: `R_em / 2` in em-units (“safety radius”).
    ///
    /// Encoded sdf span is ±`em_extra_radius` em (~`[-R_em/2,+R_em/2]` in distance space). Padding per atlas
    /// side equals `em_extra_radius * em_to_px` texels — matching encoded range without a wasted saturated ring.
    pub em_extra_radius: f32,
    pub units_per_em: u16,
    pub ascent_em: f32,
    pub descent_em: f32,
    pub line_gap_em: f32,
}

impl AtlasHeader {
    /// Total MSDF encoded distance span in atlas texels (`R_em * D`): shader `px_meta.x`.
    #[inline]
    #[allow(dead_code)] // Used by the library; `build.rs` also `#[path]`s this module and does not call this method.
    pub fn sdf_px_range(&self) -> f32 {
        2.0 * self.em_extra_radius * self.em_to_px as f32
    }
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
