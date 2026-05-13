//! High-level text layout API: font + caches + per-frame glyph buffer.
//!
//! Vertices from [`TextEngine::flush`] use **window pixel** coordinates for `TextVertex::pos`.
//! Pass an orthographic projection that maps `(0,0)..(width,height)` to clip space (no extra
//! translation/scale for font size — each [`TextArgs::size_px`] is baked into layout and quads).

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;

use crate::cache::{GlyphCache, GlyphInfo};
use crate::font::{Font, FontMetrics};
use crate::layout::{shape_text, ShapedGlyph};
use crate::linebreak::{break_lines, Line};
use crate::renderer::TextAtlas;
use crate::vertex::{push_glyph_quad_pixels, TextVertex};

const SHAPE_CACHE_TTL_FRAMES: u64 = 60;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct ShapeKey {
    content: Arc<str>,
    /// `f32::to_bits(max_width_px / size_px)` when wrapping; `0` when `max_width_px` is `None`.
    max_width_em_bits: u32,
    line_spacing_bits: u32,
}

fn make_shape_key(content: &str, args: &TextArgs) -> ShapeKey {
    let size_px = args.size_px.max(0.0001);
    ShapeKey {
        content: Arc::from(content),
        max_width_em_bits: args
            .max_width_px
            .map(|w| (w / size_px).to_bits())
            .unwrap_or(0),
        line_spacing_bits: args.line_spacing.to_bits(),
    }
}

/// Arguments controlling sizing, wrapping, and alignment for `text` / `measure`.
#[derive(Clone)]
pub struct TextArgs {
    pub size_px: f32,
    pub color: [f32; 4],
    pub max_width_px: Option<f32>,
    /// Multiplier on the font metrics line height (not pixel height).
    pub line_spacing: f32,
    pub align: Align,
}

impl Default for TextArgs {
    fn default() -> Self {
        Self {
            size_px: 16.0,
            color: [0.0, 0.0, 0.0, 1.0],
            max_width_px: None,
            line_spacing: 1.2,
            align: Align::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Pixel-space size of a laid-out block from `measure`.
pub struct Measured {
    pub width_px: f32,
    pub height_px: f32,
    pub line_count: u32,
}

/// One glyph pushed into the frame buffer; `x`/`y` are baseline pen coordinates in pixels.
pub struct PushedGlyph {
    pub glyph_id: u32,
    pub x: f32,
    pub y: f32,
    pub color: [f32; 4],
    pub(crate) info: GlyphInfo,
    size_px: f32,
}

struct CachedShape {
    glyphs: Vec<ShapedGlyph>,
    line_breaks: Vec<u32>,
    line_advances: Vec<f32>,
    block_width_em: f32,
    line_height_em: f32,
    line_count: u32,
    last_used_frame: u64,
}

pub struct TextEngine {
    font: Font,
    glyph_cache: GlyphCache,
    shape_cache: HashMap<ShapeKey, CachedShape>,
    frame: Vec<PushedGlyph>,
    vertex_scratch: Vec<TextVertex>,
    frame_counter: u64,
}

fn align_offset_em(align: Align, block_width_em: f32, line_advance_em: f32) -> f32 {
    match align {
        Align::Left => 0.0,
        Align::Center => (block_width_em - line_advance_em) * 0.5,
        Align::Right => block_width_em - line_advance_em,
    }
}

fn glyph_line_index(line_breaks: &[u32], glyph_index: usize) -> usize {
    let gi = glyph_index as u32;
    line_breaks.partition_point(|&start| start <= gi).saturating_sub(1)
}

fn layout_lines_to_cache(
    font: &Font,
    glyph_cache: &mut GlyphCache,
    lines: &[Line],
    line_height_em: f32,
    block_width_em: f32,
    frame_counter: u64,
) -> CachedShape {
    let mut glyphs = Vec::new();
    let mut line_breaks = Vec::new();
    let mut line_advances = Vec::with_capacity(lines.len());
    let mut y_em = 0.0f32;

    for line in lines {
        line_breaks.push(glyphs.len() as u32);
        line_advances.push(line.advance);
        // Cache only left-aligned geometry; alignment shifts at emit time.
        let run = shape_text(font, glyph_cache, &line.text, 0.0, y_em);
        glyphs.extend(run.glyphs);
        y_em -= line_height_em;
    }

    CachedShape {
        glyphs,
        line_breaks,
        line_advances,
        block_width_em,
        line_height_em,
        line_count: lines.len() as u32,
        last_used_frame: frame_counter,
    }
}

impl TextEngine {
    fn ensure_cached_shape(&mut self, content: &str, args: &TextArgs) {
        let key = make_shape_key(content, args);
        let fc = self.frame_counter;
        let size_px = args.size_px.max(0.0001);

        match self.shape_cache.entry(key) {
            Entry::Occupied(mut e) => {
                e.get_mut().last_used_frame = fc;
            }
            Entry::Vacant(e) => {
                let max_width_em = args.max_width_px.map(|w| w / size_px);
                let metrics = self.font.metrics();
                let line_height_em = metrics.line_height() * args.line_spacing;
                let max_w_em = max_width_em.unwrap_or(f32::MAX).max(0.0);
                let lines = break_lines(&self.font, content, max_w_em);
                let block_width_em = if let Some(w_em) = max_width_em {
                    w_em
                } else {
                    lines.iter().map(|l| l.advance).fold(0.0f32, f32::max)
                };
                let cached = layout_lines_to_cache(
                    &self.font,
                    &mut self.glyph_cache,
                    &lines,
                    line_height_em,
                    block_width_em,
                    fc,
                );
                e.insert(cached);
            }
        }
    }

    pub fn load(font_path: &str) -> Result<Self, String> {
        Font::load(font_path).map(Self::from_font)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        Font::from_bytes(bytes).map(Self::from_font)
    }

    fn from_font(font: Font) -> Self {
        Self {
            font,
            glyph_cache: GlyphCache::new(),
            shape_cache: HashMap::new(),
            frame: Vec::new(),
            vertex_scratch: Vec::new(),
            frame_counter: 0,
        }
    }

    pub fn metrics(&self) -> FontMetrics {
        self.font.metrics()
    }

    pub fn units_per_em(&self) -> u16 {
        self.font.units_per_em()
    }

    /// Curve atlas dimensions `(width, height)` for debugging / diagnostics.
    pub fn curve_atlas_size(&self) -> (u32, u32) {
        self.glyph_cache.curve_size()
    }

    /// Band atlas dimensions `(width, height)` for debugging / diagnostics.
    pub fn band_atlas_size(&self) -> (u32, u32) {
        self.glyph_cache.band_size()
    }

    pub fn new_atlas(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> TextAtlas {
        TextAtlas::new(device, queue, layout, &self.glyph_cache)
    }

    pub fn sync_atlas(
        &self,
        atlas: &mut TextAtlas,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) {
        TextAtlas::sync(atlas, device, queue, layout, &self.glyph_cache);
    }

    pub fn measure(&mut self, content: &str, args: &TextArgs) -> Measured {
        self.ensure_cached_shape(content, args);
        let key = make_shape_key(content, args);
        let shape = self.shape_cache.get(&key).expect("cached shape");
        let w_px = shape.block_width_em * args.size_px;
        let h_px = shape.line_count as f32 * shape.line_height_em * args.size_px;
        Measured {
            width_px: w_px,
            height_px: h_px,
            line_count: shape.line_count,
        }
    }

    pub fn text(&mut self, x: f32, y: f32, content: &str, args: &TextArgs) -> &mut [PushedGlyph] {
        self.ensure_cached_shape(content, args);
        let key = make_shape_key(content, args);
        let size_px = args.size_px.max(0.0001);

        let (glyphs, line_breaks, line_advances, block_w) = {
            let shape = self.shape_cache.get(&key).expect("cached shape");
            (
                shape.glyphs.as_slice(),
                shape.line_breaks.as_slice(),
                shape.line_advances.as_slice(),
                shape.block_width_em,
            )
        };

        let start = self.frame.len();
        for (i, g) in glyphs.iter().enumerate() {
            let li = glyph_line_index(line_breaks, i);
            let adv = line_advances[li];
            let ox = align_offset_em(args.align, block_w, adv);
            let gx = g.x + ox;
            let gy = g.y;
            let px = x + gx * size_px;
            let py = y - gy * size_px;
            self.frame.push(PushedGlyph {
                glyph_id: g.glyph_id,
                x: px,
                y: py,
                color: args.color,
                info: g.info,
                size_px,
            });
        }

        &mut self.frame[start..]
    }

    pub fn flush(&mut self) -> &[TextVertex] {
        self.vertex_scratch.clear();

        for g in &self.frame {
            push_glyph_quad_pixels(
                &mut self.vertex_scratch,
                &g.info,
                g.x,
                g.y,
                g.size_px,
                g.color,
            );
        }

        let fc = self.frame_counter;
        self.shape_cache.retain(|_, shape| {
            fc.saturating_sub(shape.last_used_frame) <= SHAPE_CACHE_TTL_FRAMES
        });

        self.frame.clear();
        self.frame_counter = self.frame_counter.wrapping_add(1);
        &self.vertex_scratch
    }
}
