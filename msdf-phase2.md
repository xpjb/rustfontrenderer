# MSDF phase 2: build-time material injection + ubershader

Adds the material system to `text_msdf` (outlines, glow, shadow, custom)
via a build-time codegen step. One pipeline, one draw call, statically
checked WGSL, additive without touching the renderer.

Depends on phase 1 (`text_msdf` shipped with single-fill rendering).

## Design

Best-of-both: predefined enum + ubershader **and** the ability to add new
materials by dropping a `.wgsl` file in a directory. build.rs reads the
directory, generates both the Rust enum and the WGSL switch. Materials are
plugins at compile time, dispatch at draw time.

### Crate layout additions

```
text_msdf/
  materials/                   user-editable; each file = one material
    fill.wgsl
    outline.wgsl
    glow.wgsl
    shadow.wgsl
    # users can add their own here
  build.rs                     extended: also codegens materials
  src/
    generated/                 build.rs writes here at build time
      materials.rs             enum Material { ... } + pack() + tag
      ubershader_dispatch.wgsl the switch body, included by pixel.wgsl
    shaders/
      pixel.wgsl               imports generated/ubershader_dispatch.wgsl
```

### Material file format

Each file in `materials/` is a WGSL fragment with a metadata header in
comments at the top. Convention:

```wgsl
//! name = "Outline"
//! params = [
//!   { ident = "width_px",  ty = "f32" },
//!   { ident = "color",     ty = "vec4<f32>" },
//! ]

fn material_outline(
    sd:         f32,            // signed distance, edge at 0
    aa:         f32,            // screen-space AA width
    base_color: vec4<f32>,      // from vertex.color
    p:          MaterialParams, // generated struct, see below
) -> vec4<f32> {
    let fill = clamp(sd / aa + 0.5, 0.0, 1.0);
    let body = clamp((sd + p.width_px) / aa + 0.5, 0.0, 1.0);
    return vec4(mix(p.color.rgb, base_color.rgb, fill),
                body * base_color.a);
}
```

Header is parsed as TOML inside `//!` comments. `params` is a list of
`{ ident, ty }` records. Total param bytes must fit in `[f32; 8]` (32 bytes).

### Generated `materials.rs`

```rust
// text_msdf/src/generated/materials.rs (auto-generated)
#[derive(Clone, Copy, Debug)]
pub enum Material {
    Fill,
    Outline { width_px: f32, color: [f32; 4] },
    Glow    { radius_px: f32, color: [f32; 4], strength: f32 },
    Shadow  { offset: [f32; 2], blur_px: f32, color: [f32; 4] },
    // ... auto-extended as new files appear in materials/
}

impl Material {
    pub fn tag(&self) -> u32 {
        match self {
            Material::Fill        => 0,
            Material::Outline {..} => 1,
            Material::Glow    {..} => 2,
            Material::Shadow  {..} => 3,
        }
    }
    pub fn pack(&self) -> [f32; 8] {
        let mut p = [0.0f32; 8];
        match *self {
            Material::Fill => {}
            Material::Outline { width_px, color } => {
                p[0] = width_px;
                p[1..5].copy_from_slice(&color);
            }
            Material::Glow { radius_px, color, strength } => {
                p[0] = radius_px;
                p[1..5].copy_from_slice(&color);
                p[5] = strength;
            }
            // ...
        }
        p
    }
}
```

### Generated `ubershader_dispatch.wgsl`

```wgsl
struct MaterialParams {
    p0: f32, p1: f32, p2: f32, p3: f32,
    p4: f32, p5: f32, p6: f32, p7: f32,
}

fn dispatch_material(
    tag: u32, sd: f32, aa: f32, base_color: vec4<f32>, p: MaterialParams
) -> vec4<f32> {
    switch (tag) {
        case 0u: { return material_fill(sd, aa, base_color, p); }
        case 1u: { return material_outline(sd, aa, base_color, p); }
        case 2u: { return material_glow(sd, aa, base_color, p); }
        case 3u: { return material_shadow(sd, aa, base_color, p); }
        default: { return vec4(1.0, 0.0, 1.0, 1.0); }
    }
}
```

Each `material_xxx` snippet from `materials/*.wgsl` is concatenated above
this dispatch block. The pixel shader does:

```wgsl
// pixel.wgsl
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var samp:  sampler;

// {{include generated/ubershader_dispatch.wgsl}}
// — replaced literally by build.rs before compiling

@fragment fn main(in: VOut) -> @location(0) vec4<f32> {
    let s   = textureSample(atlas, samp, in.uv);
    let sd  = median(s.r, s.g, s.b) - 0.5;
    let pxs = vec2(length(dpdx(in.uv)), length(dpdy(in.uv)));
    let aa  = globals.px_range * length(pxs);

    let p = MaterialParams(
        in.mat_p0, in.mat_p1, in.mat_p2, in.mat_p3,
        in.mat_p4, in.mat_p5, in.mat_p6, in.mat_p7,
    );
    return dispatch_material(in.mat_tag, sd, aa, in.color, p);
}
```

The `{{include ...}}` substitution is a literal text replace in build.rs —
no template engine, no runtime cost.

### Vertex format additions

`TextVertex` (phase 1) becomes:

```rust
#[repr(C)]
pub struct TextVertex {
    pub pos:        [f32; 2],
    pub uv:         [f32; 2],
    pub color:      [f32; 4],
    pub mat_tag:    u32,        // +4
    pub mat_params: [f32; 8],   // +32 — two vec4 slots
}
// 80 bytes per vertex × 6 verts per glyph
```

Replicated across the 6 verts of a glyph (uniform within the quad). For
1800 flyers × 16 chars × 6 verts × 80 B ≈ 13 MB/frame vertex throughput.
Trivial on PCIe.

If we ever care about vertex memory: switch to instanced rendering — 4 verts
of unit-quad in a static buffer, per-instance buffer carries the rest. Out
of scope here.

### Uniforms — where what lives

WGSL terminology refresher for clarity:


| Mechanism            | Granularity                       | Right for                     |
| -------------------- | --------------------------------- | ----------------------------- |
| `var<uniform>` (UBO) | Per-draw                          | Globals shared by every glyph |
| `var<storage>`       | Per-anything                      | Big or shared per-glyph data  |
| Vertex attribute     | Per-vertex (per-glyph in our use) | Material params               |


For materials: **per-glyph params go in vertex attributes**, not in any
uniform binding. Reason: mixing outlined and filled glyphs *in one draw
call* requires per-quad tag + params. UBO is one value per draw, so too
coarse. Storage buffer works but needs an instance-id attribute anyway,
no gain.

The UBO (`@group(0) @binding(0)`) keeps just the truly global things:

```wgsl
struct Globals {
    matrix:    mat4x4<f32>,
    px_range:  f32,            // baked distance range from build.rs
    time:      f32,            // optional, for animated materials
    _pad:      vec2<f32>,
}
```

`px_range` written at startup. `time` updated each frame if any material
wants it.

## Build script extensions

Phase 1 build.rs already does font hashing + msdfgen + atlas emission.
Phase 2 adds:

1. `cargo:rerun-if-changed=materials/`
2. Walk `materials/*.wgsl`. For each: parse the `//!` TOML header.
3. Validate: param byte-size ≤ 32; param identifiers unique; WGSL function
  name matches expected pattern.
4. Generate `src/generated/materials.rs` with the enum and `pack()` impl.
5. Generate `src/generated/ubershader_dispatch.wgsl` by concatenating each
  material's body and emitting the switch.
6. Substitute `{{include generated/ubershader_dispatch.wgsl}}` into a
  copy of `pixel.wgsl` written to `src/generated/pixel.wgsl`.
7. Run `naga` on the generated pixel shader. Fail the build with a
  readable error if WGSL is invalid. (This catches typos in material
   snippets at compile time, not first frame.)

```toml
# text_msdf/Cargo.toml — additional build-deps
[build-dependencies]
toml = "0.8"
naga = { version = "0.20", features = ["wgsl-in", "validate"] }
```

`src/generated/` is gitignored (same as `generated/` for the atlas).

## API additions

`TextArgs` gains a `material` field:

```rust
pub struct TextArgs {
    pub size_px:       f32,
    pub color:         [f32; 4],
    pub max_width_px:  Option<f32>,
    pub line_spacing:  f32,
    pub align:         Align,
    pub material:      Material,    // new; default Material::Fill
}
```

`PushedGlyph` gains the same:

```rust
pub struct PushedGlyph {
    pub glyph_id: u32,
    pub x: f32, pub y: f32,
    pub color:    [f32; 4],
    pub material: Material,         // new; per-glyph override
    info: GlyphInfo,
}
```

`text()` stamps `args.material` into every pushed glyph; callers mutate
individual ones via the slice when they want variation:

```rust
for g in engine.text(x, y, "Hello", &base_args) {
    if some_condition(g.glyph_id) {
        g.material = Material::Glow { radius_px: 4.0, color: WHITE, strength: 1.0 };
    }
}
```

`flush()` packs `(material.tag(), material.pack())` into every vertex.

## Slug compatibility

The `text` crate has the same `TextArgs.material` field for source
compatibility, but only `Material::Fill` works. Other variants:

- `Material::Outline` — could be supported via a second pass with inflated
geometry; defer until anyone asks.
- `Material::Glow` / `Material::Shadow` — physically not possible with
coverage (Slug computes coverage, not distance). `flush()` emits the
glyph as `Fill` and logs a warning once per material.

This keeps the surface identical between crates but honest about what each
backend supports.

## Phase 2 deliverables

1. `materials/{fill, outline, glow, shadow}.wgsl` written.
2. build.rs material codegen + naga validation.
3. `TextVertex` extended; vertex emit updated.
4. `Material` enum + `TextArgs.material` + `PushedGlyph.material`.
5. Pixel shader rewritten to dispatch.
6. Torture extended: per-style toggle (`F1` cycle Fill / Outline / Glow /
  Shadow), `[`/`]` to sweep the primary param.

## Verification

1. Single-fill output identical to phase 1 (regression check).
2. Outline visible and correctly anti-aliased at sizes 16–240 px.
3. Glow looks right with `radius_px = 2, 4, 8` — falloff smooth, no
  median-of-three artifacts visible far from the edge (if there are, the
   answer is to switch atlas channel from MSDF to MTSDF and read the alpha
   channel for glow; document this as the upgrade path).
4. Adding a new material file in `materials/` triggers a rebuild and the
  enum picks up the new variant.
5. `naga` rejects a deliberately-broken material file with a useful error.

