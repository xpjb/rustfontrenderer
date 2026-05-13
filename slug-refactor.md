# Slug-side refactor: `TextEngine` API

Refactor the existing `text` crate so its public API revolves around a single
`TextEngine` that bundles the loaded font with the internal caches, exposes an
immediate-mode `text()` call, and supports per-glyph effects via a returned
mutable slice. Prepares the API surface for `text_msdf` to mirror.

No backend logic changes — bands, curves, and the Slug shader all stay.

## Goals

- Fold `Font + GlyphCache` into one struct (`TextEngine`) so callers stop
  passing the pair side by side.
- Replace the current `shape_text(font, &mut cache, text, x, y) -> ShapedRun`
  primitive with a higher-level `text(x, y, content, &TextArgs) -> &mut [PushedGlyph]`
  that handles wrap/layout in one call.
- Add an internal shape cache (keyed on content + wrap width + font id) with
  frame-stamped LRU eviction so repeated immediate-mode calls don't re-shape.
- Keep per-glyph access via the `&mut` slice return — callers mutate position
  / color for effects, mutations are per-frame and never touch the cache.
- Migrate `demo` and `torture` to the new API.

Out of scope for this phase: materials, escape codes, multi-font fallback.

## Public API after refactor

```rust
// text/src/lib.rs
pub use engine::{TextEngine, TextArgs, Align, Measured, PushedGlyph};
pub use renderer::{TextRenderer, TextAtlas};
pub use vertex::TextVertex;
pub use font::FontMetrics;
```

### `TextEngine`

```rust
pub struct TextEngine { /* font + glyph cache + shape cache + frame buffer */ }

impl TextEngine {
    pub fn load(font_path: &str) -> Result<Self, String>;
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String>;

    pub fn metrics(&self) -> FontMetrics;
    pub fn units_per_em(&self) -> u16;

    /// Lay out `content` and append positioned glyphs to the frame buffer.
    /// Returns a mutable view of the just-pushed glyphs — mutate `.x`, `.y`,
    /// or `.color` for per-glyph effects (wave, jitter, fade). Mutations
    /// live one frame.
    pub fn text(&mut self, x: f32, y: f32, content: &str, args: &TextArgs)
        -> &mut [PushedGlyph];

    /// Measure-only. Same shape cache as `text`. No frame buffer push.
    pub fn measure(&mut self, content: &str, args: &TextArgs) -> Measured;

    /// Drain the frame buffer into a vertex slice. Call once per frame after
    /// all `text()` calls. Also evicts shape-cache entries unused for the
    /// last N frames.
    pub fn flush(&mut self) -> &[TextVertex];

    /// Returns the cache that the renderer's `TextAtlas` syncs against.
    /// Internal-ish; needed for `TextAtlas::sync`.
    pub fn glyph_cache(&self) -> &GlyphCache;
}
```

### Supporting types

```rust
pub struct TextArgs {
    pub size_px: f32,
    pub color: [f32; 4],
    pub max_width_px: Option<f32>,   // None = no wrap
    pub line_spacing: f32,           // multiplier on line_height
    pub align: Align,
}
impl Default for TextArgs { /* size 16, color black, no wrap, 1.2 spacing, Left */ }

pub enum Align { Left, Center, Right }

pub struct Measured {
    pub width_px: f32,
    pub height_px: f32,
    pub line_count: u32,
}

pub struct PushedGlyph {
    pub glyph_id: u32,
    pub x: f32,                  // baseline pixel position; mutate freely
    pub y: f32,
    pub color: [f32; 4],
    info: GlyphInfo,             // private; backend shape data
}
```

`GlyphInfo` stays an opaque type held by `PushedGlyph`; vertex emission reads
it during `flush`. Callers never touch it.

### Renderer

`TextRenderer` and `TextAtlas` keep their current shape. `TextAtlas::sync`
now takes `&engine.glyph_cache()` instead of a separate `&GlyphCache`. Per
frame:

```rust
let verts = engine.flush();
let vbuf  = TextRenderer::build_vertices(&device, verts);
atlas.sync(&device, &queue, &renderer.atlas_layout, engine.glyph_cache());
renderer.render(&queue, &mut enc, &view, &atlas, &vbuf, verts.len() as u32,
                matrix, (w, h), Some(bg));
```

(`build_run_vertices` is gone — vertex generation moves inside `flush`.)

## Internal structure

`TextEngine` owns:

- `font: Font` — same as today, owns the leaked `&'static [u8]` and rustybuzz
  face.
- `glyph_cache: GlyphCache` — unchanged. On-demand band/curve atlas.
- `shape_cache: HashMap<ShapeKey, CachedShape>` — new.
- `frame: Vec<PushedGlyph>` — new. Cleared by `flush`.
- `vertex_scratch: Vec<TextVertex>` — new. Reused buffer for `flush` output.
- `frame_counter: u64` — for LRU stamps.

### `ShapeKey`

```rust
struct ShapeKey {
    content_hash: u64,             // hash of the &str (fxhash or ahash)
    max_width_em_bits: u32,        // f32::to_bits, or 0 for None
    line_spacing_bits: u32,        // affects line break positions
}
```

`size_px` and `color` are NOT in the key — shaping is em-space; sizing and
color apply at position time. `align` is also not in the key — alignment
shifts line origins post-shape.

### `CachedShape`

```rust
struct CachedShape {
    glyphs: Vec<ShapedGlyph>,      // un-positioned, em-space offsets + advance
    line_breaks: Vec<u32>,         // glyph indices where new lines start
    bbox_em: (f32, f32, f32, f32),
    last_used_frame: u64,
}
```

`text()` flow:
1. Build `ShapeKey`. Lookup; on miss, run `break_lines` (when wrapping) +
   `rustybuzz::shape` per resulting line, and populate `glyph_cache` for any
   new glyph IDs. Insert.
2. On hit (or fresh): stamp `last_used_frame`.
3. Iterate `CachedShape.glyphs`, apply `size_px`, alignment offset per line,
   and `(x, y)` translation; push `PushedGlyph` into `frame`.
4. Return `&mut frame[start..]`.

`flush()`:
1. Walk `frame`, emit 6 vertices per glyph into `vertex_scratch`.
2. Evict any `CachedShape` with `frame_counter - last_used_frame > 60`.
3. `frame.clear()`; `frame_counter += 1`; return `&vertex_scratch`.

Eviction window of 60 frames (~1 s at 60fps) keeps the cache scoped to "the
last second of unique strings". No explicit cap; can add later if any user
hits an unbounded-growth case.

## Files touched

- `text/src/lib.rs` — re-exports.
- `text/src/font.rs` — `Font` becomes pub(crate); `TextEngine` becomes the
  public face. Keep `FontMetrics` public.
- `text/src/cache.rs` — keep `GlyphCache` and `GlyphInfo` (private mod, no
  re-export). `GlyphInfo` remains the private payload of `PushedGlyph`.
- `text/src/engine.rs` — **new**. `TextEngine`, `TextArgs`, `Align`,
  `Measured`, `PushedGlyph`, the shape cache.
- `text/src/layout.rs` — keep `shape_text` as a pub(crate) helper used by
  `TextEngine`. `ShapedGlyph`/`ShapedRun` become pub(crate).
- `text/src/linebreak.rs` — unchanged signature, now called only from
  `TextEngine`. Keep `pub fn measure` and `Line` (used in measure).
- `text/src/vertex.rs` — `TextVertex` unchanged. `build_run_vertices`
  deleted; replaced by an internal `emit_vertex` helper called from `flush`.
- `text/src/renderer.rs` — `TextAtlas::new`/`sync` accept
  `&GlyphCache` via `engine.glyph_cache()`; otherwise unchanged.

## Demo migration

[demo/src/main.rs](demo/src/main.rs) becomes:

```rust
let mut engine = TextEngine::load(font_path)?;
let renderer   = TextRenderer::new(&device, &config);
let mut atlas  = TextAtlas::new(&device, &queue, &renderer.atlas_layout,
                                engine.glyph_cache());

// per frame
let args = TextArgs {
    size_px: 64.0,
    color: [0.10, 0.10, 0.12, 1.0],
    max_width_px: Some(WINDOW_W as f32 - MARGIN * 2.0),
    line_spacing: 1.25,
    align: Align::Left,
};
engine.text(MARGIN, MARGIN + ascent_px, SAMPLE, &args);

let verts = engine.flush();
let vbuf  = TextRenderer::build_vertices(&device, verts);
atlas.sync(&device, &queue, &renderer.atlas_layout, engine.glyph_cache());
renderer.render(/* ... */);
```

The 5-line `for line in &lines { shape_text(...); y_em -= line_height; }`
loop disappears.

## Torture migration

[torture/src/main.rs](torture/src/main.rs) keeps its `PhraseBank` — that's
still the right pattern for repeatedly drawing the same phrase at thousands
of positions. The change:

- `PhraseBank` no longer stores pre-built `ShapedRun`s; it just stores the
  string + a stable `TextArgs` (base style).
- Each flyer per frame: `engine.text(flyer.x, flyer.y, phrase, &args)`.
  Shape cache hit on every call after the first; cost is layout + vertex
  emission only.
- Stat HUD uses `engine.measure(...)` for right-aligned numbers.

This validates the cache: 1800 flyers × 60fps × ~16 unique phrases = ~1.7M
calls/sec, all served from the shape cache.

## Verification

1. `cargo run -p demo` renders the same paragraph as today, pixel-identical
   except for any sub-pixel rounding from the `size_px` path through layout.
2. `cargo run -p torture` runs at the same FPS as today (within noise) with
   1800 flyers at the default font size.
3. Toggling `+/-/Up/Down` in torture still works.

## Order of work

1. Add `text/src/engine.rs` with `TextEngine`, `TextArgs`, `PushedGlyph`.
2. Internal shape cache + frame buffer + LRU.
3. Move vertex emission from `vertex.rs` into a private helper called by
   `flush`.
4. Update `TextAtlas` to take `&GlyphCache` via the engine accessor.
5. Migrate demo.
6. Migrate torture.
7. Drop now-unused public re-exports (`shape_text`, `ShapedRun`, `Font`,
   `GlyphCache`, `build_run_vertices`) from `lib.rs`. Keep their modules
   pub(crate).
