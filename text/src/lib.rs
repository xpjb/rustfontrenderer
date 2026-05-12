//! GPU font renderer based on the Slug algorithm (Lengyel, 2017).
//!
//! Loads TTF/OTF fonts, shapes text with rustybuzz, builds a GPU glyph cache of
//! quadratic Bézier curves and bands, and renders solid filled text via a wgpu
//! pipeline. Exposes per-glyph layout metrics (advance / width) for caller-side
//! layout, and includes a naive whitespace-based line breaker for a given width.
//!
//! Slug computes per-pixel *coverage* (fraction filled), not signed distance,
//! so this crate does fills only — strokes/outlines wider than 1 px need a
//! different technique (SDF, inflated-geometry passes, etc.).

pub mod font;
pub mod outline;
pub mod bands;
pub mod cache;
pub mod layout;
pub mod linebreak;
pub mod vertex;
pub mod renderer;

pub use font::{Font, FontMetrics};
pub use outline::{GlyphOutlines, QuadraticCurve};
pub use bands::{process_bands, process_bands_with, BandData, BandsScratch};
pub use cache::{GlyphCache, GlyphInfo};
pub use layout::{shape_text, ShapedGlyph, ShapedRun};
pub use linebreak::{break_lines, Line};
pub use vertex::{TextVertex, build_run_vertices};
pub use renderer::{TextRenderer, TextAtlas};
