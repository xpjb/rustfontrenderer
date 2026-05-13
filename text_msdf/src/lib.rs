//! MSDF text renderer — mirrors `text` crate exports.

mod atlas;
mod atlas_format;
mod engine;
mod font;
mod layout;
mod linebreak;
mod renderer;
mod vertex;

pub use atlas::FontMetrics;
pub use engine::{Align, Measured, PushedGlyph, TextArgs, TextEngine};
pub use renderer::{TextAtlas, TextRenderer};
pub use vertex::TextVertex;
