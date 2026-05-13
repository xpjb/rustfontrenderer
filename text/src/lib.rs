//! GPU font renderer based on the Slug algorithm (Lengyel, 2017).
//!
//! Loads TTF/OTF fonts, shapes text with rustybuzz, builds a GPU glyph cache of
//! quadratic Bézier curves and bands, and renders solid filled text via a wgpu
//! pipeline.

mod font;
mod outline;
mod bands;
mod cache;
mod layout;
mod linebreak;
mod vertex;
mod renderer;
pub mod engine;

pub use engine::{Align, Measured, PushedGlyph, TextArgs, TextEngine};
pub use font::FontMetrics;
pub use renderer::{TextAtlas, TextRenderer};
pub use vertex::TextVertex;
