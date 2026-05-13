//! Font loading: keeps font bytes alive for a `'static` rustybuzz/ttf-parser face,
//! exposes shaping, outline extraction, and per-glyph metrics.

use rustybuzz::Face as RustyFace;
use ttf_parser::GlyphId;
use crate::outline::{GlyphOutlines, OutlineCollector};

/// Vertical font metrics, in em-space.
#[derive(Clone, Copy, Debug)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

impl FontMetrics {
    /// Default line-to-line advance: ascent - descent + line_gap (descent is negative).
    pub fn line_height(&self) -> f32 {
        self.ascent - self.descent + self.line_gap
    }
}

/// A loaded font. Bytes are heap-leaked so the underlying face has a `'static` lifetime.
pub(crate) struct Font {
    #[allow(dead_code)]
    data: &'static [u8],
    face: RustyFace<'static>,
    units_per_em: u16,
    metrics: FontMetrics,
}

impl Font {
    /// Load from a file path.
    pub fn load(path: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read {}: {}", path, e))?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        Self::from_bytes_with_index(bytes, 0)
    }

    pub fn from_bytes_with_index(bytes: Vec<u8>, face_index: u32) -> Result<Self, String> {
        let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        let face = RustyFace::from_slice(leaked, face_index)
            .ok_or_else(|| "failed to parse font".to_string())?;
        let units_per_em = face.units_per_em() as u16;
        let upem = units_per_em as f32;
        let metrics = FontMetrics {
            ascent: face.ascender() as f32 / upem,
            descent: face.descender() as f32 / upem,
            line_gap: face.line_gap() as f32 / upem,
        };
        Ok(Self { data: leaked, face, units_per_em, metrics })
    }

    pub fn face(&self) -> &RustyFace<'static> { &self.face }
    pub fn units_per_em(&self) -> u16 { self.units_per_em }
    pub fn metrics(&self) -> FontMetrics { self.metrics }

    pub fn glyph_index(&self, c: char) -> Option<GlyphId> {
        self.face.glyph_index(c)
    }

    /// Em-space horizontal advance for a glyph.
    pub fn advance_em(&self, glyph_id: GlyphId) -> f32 {
        self.face.glyph_hor_advance(glyph_id)
            .map(|a| a as f32 / self.units_per_em as f32)
            .unwrap_or(0.0)
    }

    /// Em-space bounding-box width (max_x - min_x), or 0 for empty/invisible glyphs.
    pub fn glyph_width_em(&self, glyph_id: GlyphId) -> f32 {
        self.face.glyph_bounding_box(glyph_id)
            .map(|r| (r.x_max as f32 - r.x_min as f32) / self.units_per_em as f32)
            .unwrap_or(0.0)
    }

    /// Extract quadratic Bézier outlines for a glyph in em-space.
    pub fn load_glyph(&self, glyph_id: GlyphId) -> Option<GlyphOutlines> {
        let mut builder = OutlineCollector::new(self.units_per_em);
        self.face.outline_glyph(glyph_id, &mut builder).map(|_| builder.finish())
    }
}
