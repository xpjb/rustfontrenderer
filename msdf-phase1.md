# MSDF phase 1: `text_msdf` crate + torture comparison

Build a parallel `text_msdf` crate that mirrors the `text` crate's public
API (post-`slug-refactor.md`), backed by a precomputed MSDF atlas generated
at build time. Ship a `torture_msdf` binary that's a byte-for-byte copy of
`torture` with the import swapped, for direct A/B perf comparison.

Phase 1 is **single fill only** — median-of-three plus screen-space AA,
no outlines, glow, shadow, or material system. That's phase 2.

## Crate layout

```
text_msdf/
  Cargo.toml
  charset.txt                  edited by hand; ranges + literal chars
  build.rs
  generated/                   build.rs output; gitignored
    atlas.png                  RGB8 (plain MSDF)
    atlas.bin                  bincode header + glyph table
    manifest.json              hash, params — regen check
  src/
    lib.rs
    engine.rs                  TextEngine — same API as text crate
    atlas.rs                   on-disk format read/write
    linebreak.rs               copied verbatim from text crate
    vertex.rs                  TextVertex (MSDF flavor)
    renderer.rs                TextRenderer + TextAtlas
    shaders/
      vertex.wgsl
      pixel.wgsl
torture_msdf/
  Cargo.toml
  src/main.rs                  cp ../torture/src/main.rs, swap `use`
```

`Cargo.toml` workspace members add: `text_msdf`, `torture_msdf`.

## charset.txt

One entry per line. Empty lines and `#`-prefixed lines ignored. Two kinds:

```
# ASCII printable
0020-007E
# Latin-1 supplement
00A0-00FF
# Literal extras
…—«»""
```

`hex-hex` is a closed unicode range. Anything else on a non-comment line is
treated as literal characters (each char added individually). build.rs
collects everything into a `BTreeSet<char>` and dedupes.

For phase 1 the default charset covers ASCII printable + the few box-drawing
and punctuation chars used in `torture`'s phrases and HUD.

## Build script

[text_msdf/build.rs](text_msdf/build.rs) does:

1. `cargo:rerun-if-changed=charset.txt`
2. `cargo:rerun-if-changed=../assets/NotoSansSC-Regular.ttf`
3. `cargo:rerun-if-changed=build.rs`

4. Read charset → `Vec<char>` sorted.
5. Read font bytes. Resolve `char → GlyphId` via ttf-parser. Drop any chars
   the font lacks (warn through `cargo:warning=...`).
6. Compute `manifest_hash = blake3(font_bytes ‖ charset ‖ generator_version
   ‖ atlas_params)`. Read `generated/manifest.json` if present; if hash
   matches and the two output files exist → exit (cache hit).
7. Otherwise: regenerate. Steps below.

### Generation parameters (fixed for phase 1)

```rust
const GLYPH_PX: u32 = 32;          // em-square pixel size in the atlas
const DISTANCE_RANGE_PX: f32 = 4.0;
const ATLAS_MAX_WIDTH: u32 = 2048;
```

Cell size per glyph = `GLYPH_PX + DISTANCE_RANGE_PX` rounded up = 36 px. So
`cols = ATLAS_MAX_WIDTH / 36 = 56`. For a 256-glyph charset, `rows = 5`,
atlas is `2016 × 180` ≈ 4 MB RGB8. Plenty of room to grow.

### Per-glyph MSDF via the `msdfgen` crate

Crate: `msdfgen` (binds Chlumsky's reference C++ libmsdfgen).
[https://crates.io/crates/msdfgen](https://crates.io/crates/msdfgen)

For each `GlyphId`:

1. Build a `msdfgen::Shape` from the glyph's outline. Two paths:
   - Use `msdfgen::FontExt` (the crate's own font loader, FreeType-backed),
     **or**
   - Extract quadratic/cubic segments via `ttf-parser`'s `OutlineBuilder`
     and emit `Shape::add_contour` + per-segment `EdgeSegment::QuadraticBezier`
     / `EdgeSegment::CubicBezier`.

   The latter is preferable: we already use ttf-parser everywhere, no
   FreeType dependency, full control. Outline collection mirrors
   [text/src/outline.rs](text/src/outline.rs).

2. `shape.edge_coloring_simple(3.0, 0)` — the standard 3-color assignment
   for MSDF.

3. Compute framing:
   - Glyph bbox (em-space) → `(min_x, min_y, max_x, max_y)`.
   - `scale = GLYPH_PX as f64 / 1.0` (1 em = GLYPH_PX texels).
   - `translate = (-min_x + DISTANCE_RANGE_PX/2/scale, -min_y + ...)` so
     glyph fits with `distance_range_px / 2` padding on each side.
   - Bitmap dims: `(ceil(em_w*scale + DISTANCE_RANGE_PX),
     ceil(em_h*scale + DISTANCE_RANGE_PX))`.

4. Allocate a per-glyph `Bitmap<RGB<f32>>` of those dims. Call
   `shape.generate_msdf(&mut bitmap, framing, config)` where `config` has
   `pixel_distance_range = DISTANCE_RANGE_PX`.

5. Convert RGB<f32> → RGB<u8> via the standard `pixel_byte = clamp((d + 0.5)
   * 255, 0, 255)` mapping.

### Pack into one atlas

Fixed grid. For glyph `i` (0-indexed in the sorted charset):

```
col = i % cols
row = i / cols
x   = col * cell
y   = row * cell
```

Per-glyph bitmap is smaller than `cell × cell` (most glyphs don't fill the
em-square). Center the bitmap inside its cell and zero-pad the rest. Record
the bitmap's actual `(w, h)` so the UV rect is tight.

### Output

```rust
// atlas.bin schema (bincode)
struct AtlasFile {
    header: AtlasHeader,
    glyphs: Vec<GlyphRecord>,   // sorted by glyph_id
}
struct AtlasHeader {
    magic: [u8; 4],             // b"MSDF"
    version: u32,               // bump on schema change
    font_hash: [u8; 32],
    atlas_w: u32,
    atlas_h: u32,
    glyph_px: u32,
    distance_range_px: f32,
    units_per_em: u16,
    ascent_em: f32,             // pre-baked metrics so runtime needn't reparse
    descent_em: f32,
    line_gap_em: f32,
}
struct GlyphRecord {
    glyph_id: u32,
    uv_min: [u16; 2],           // texel coords; convert to [0,1] at runtime
    uv_max: [u16; 2],
    plane_min_em: [f32; 2],     // glyph quad in em-space, includes padding
    plane_max_em: [f32; 2],
    advance_em: f32,
}
```

`atlas.png` is a straight RGB8 dump. Use the `png` crate for write/read.

Write `generated/manifest.json`:
```json
{ "hash": "...", "glyph_px": 32, "distance_range_px": 4.0, "atlas_size": [2016, 180] }
```

### Embedding

Use `include_bytes!` in `atlas.rs`:

```rust
const ATLAS_PNG: &[u8] = include_bytes!("../generated/atlas.png");
const ATLAS_BIN: &[u8] = include_bytes!("../generated/atlas.bin");
```

Bakes both into the binary. Decoded on `TextEngine::load`.

## Runtime

### Public API (mirrors `text` crate post-refactor)

```rust
pub use engine::{TextEngine, TextArgs, Align, Measured, PushedGlyph};
pub use renderer::{TextRenderer, TextAtlas};
pub use vertex::TextVertex;
pub use atlas::FontMetrics;

impl TextEngine {
    pub fn load(font_path: &str) -> Result<Self, String>;
    pub fn metrics(&self) -> FontMetrics;
    pub fn units_per_em(&self) -> u16;
    pub fn text(&mut self, x: f32, y: f32, content: &str, args: &TextArgs)
        -> &mut [PushedGlyph];
    pub fn measure(&mut self, content: &str, args: &TextArgs) -> Measured;
    pub fn flush(&mut self) -> &[TextVertex];
}
```

All names and signatures match the slug crate. Drop-in swap of the `use`
line is the entire torture migration.

### `TextEngine` internals (MSDF flavor)

Owns:

- `font: rustybuzz::Face<'static>` — still needed for shaping.
- `atlas: AtlasFile` — decoded glyph table + image bytes.
- `glyph_table: HashMap<u32, GlyphRecord>` — by glyph_id.
- `shape_cache: HashMap<ShapeKey, CachedShape>` — same as slug.
- `frame: Vec<PushedGlyph>`, `vertex_scratch: Vec<TextVertex>`.
- `frame_counter: u64`.

`text()` looks up cached shape; for any glyph_id missing from `glyph_table`,
substitute the `?` glyph and warn-once. (Charset misses are a configuration
problem, not a runtime problem; we surface them but don't crash.)

`flush()` emits 6 vertices per `PushedGlyph` with attributes:

```rust
#[repr(C)]
pub struct TextVertex {
    pos: [f32; 2],          // pixel-space corner
    uv:  [f32; 2],          // atlas coords in [0,1]
    color: [f32; 4],
}
```

No band/curve attributes — much simpler than the Slug `TextVertex`.

### `TextAtlas`

Uploads `atlas.png` to one `Rgba8UnormSrgb`-ish texture at startup. No
`sync()` — atlas is immutable. (Actually `Rgb8Unorm` if we want to be
precise; wgpu support varies. Pad to `Rgba8Unorm` with `A = 255` for safety.)

### `TextRenderer`

Pipeline differences from Slug:
- One bind group with `(uniform, texture, sampler)`. Sampler is `Linear`
  with clamp.
- Vertex format: `pos`, `uv`, `color` only. Three attributes vs five.
- Fragment shader: median-of-three.

```wgsl
// pixel.wgsl
struct Globals {
    matrix: mat4x4<f32>,
    px_range: f32,
    _pad: vec3<f32>,
}
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

fn median(a: f32, b: f32, c: f32) -> f32 {
    return max(min(a, b), min(max(a, b), c));
}

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv:    vec2<f32>,
    @location(1) color: vec4<f32>,
}

@fragment fn main(in: VOut) -> @location(0) vec4<f32> {
    let s  = textureSample(atlas, samp, in.uv);
    let sd = median(s.r, s.g, s.b) - 0.5;
    let pxs = vec2(length(dpdx(in.uv)), length(dpdy(in.uv)));
    let aa  = globals.px_range * length(pxs);
    let a   = clamp(sd / aa + 0.5, 0.0, 1.0);
    return vec4(in.color.rgb, a * in.color.a);
}
```

`px_range` here is `DISTANCE_RANGE_PX` from build time, written into the
uniform once at startup.

## torture_msdf

[torture_msdf/src/main.rs](torture_msdf/src/main.rs) is a byte-for-byte
copy of [torture/src/main.rs](torture/src/main.rs) with one change:

```rust
- use text::{shape_text, Font, GlyphCache, TextAtlas, TextRenderer, TextVertex};
+ use text_msdf::{TextEngine, TextAtlas, TextRenderer, TextVertex, TextArgs, Align};
```

(`shape_text`, `Font`, `GlyphCache` are already gone after the slug
refactor; the torture migration to `TextEngine` happens in the slug phase.)

Window title, font size, flyer counts, phrase bank, HUD all unchanged. The
output is what matters — visual quality compared side by side at 60/30/15/
240 px font sizes, frame ms stats compared at matched flyer counts.

## Cargo.toml additions

### Root [Cargo.toml](Cargo.toml)

```toml
[workspace]
members = ["text", "text_msdf", "demo", "torture", "torture_msdf"]
```

### text_msdf/Cargo.toml

```toml
[package]
name = "text_msdf"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
wgpu       = { workspace = true }
ttf-parser = { workspace = true }
rustybuzz  = { workspace = true }
bytemuck   = { workspace = true }
glam       = { workspace = true }
png        = "0.17"
bincode    = "1.3"
serde      = { version = "1", features = ["derive"] }

[build-dependencies]
ttf-parser = { workspace = true }
msdfgen    = "0.5"
png        = "0.17"
bincode    = "1.3"
blake3     = "1"
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
```

### torture_msdf/Cargo.toml

Same as `torture/Cargo.toml` with `text` → `text_msdf`.

## .gitignore

Append: `text_msdf/generated/`

## Open items / known gotchas

- **msdfgen C++ build on Windows.** `msdfgen-sys` builds against the C++
  reference impl. Needs MSVC build tools (already required for wgpu/winit
  on Windows so likely fine). First-time `cargo build` will be slow.
- **CJK in NotoSansSC.** The font has tens of thousands of glyphs. Phase 1
  charset is ASCII + a handful of extras (~100 glyphs); CJK is out of scope
  for the atlas because per-glyph bake costs ~10 ms and the atlas would
  balloon. If we want CJK later, charset.txt can include ranges but expect
  build times in the minutes.
- **Subpixel jitter at small sizes.** MSDF starts to alias hard below ~12 px
  rendered size. The torture explicitly sweeps down to 5 px; this will look
  different from Slug there. Expected, documented as "MSDF lower-bound" in
  the comparison.
- **No SDF padding for very thin strokes.** If the font has hairlines
  thinner than 1 texel at GLYPH_PX=32, msdfgen will report them as
  zero-coverage. NotoSansSC Regular is fine; some fonts aren't.

## Order of work

1. Workspace + crate skeleton (`text_msdf/Cargo.toml`, empty `src/lib.rs`).
2. charset.txt parser + build.rs hash check (no generation yet) — verifies
   the rerun-if-changed plumbing works.
3. msdfgen + ttf-parser outline → per-glyph MSDF bitmap. Standalone test
   that writes one glyph to a debug PNG.
4. Grid pack + atlas.png + atlas.bin emission.
5. Runtime atlas load + `TextEngine` skeleton (no shape cache yet,
   re-shapes every call — gets us a picture).
6. Shape cache + frame buffer (port the slug-side implementation).
7. `TextRenderer` + shader.
8. `torture_msdf` crate; copy the file; flip the import; verify visuals.
9. Side-by-side perf run vs slug torture at matched configs.
