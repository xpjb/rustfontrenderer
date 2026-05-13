# MSDF phase 2 code review

Scope: correctness + simplicity. Sources read: `text_msdf/build_materials.rs`,
`text_msdf/materials/*.wgsl`, `text_msdf/src/shaders/{pixel,vertex}.wgsl`,
`text_msdf/src/{vertex,renderer,engine,lib}.rs`, `demo_msdf/src/main.rs`,
`torture_msdf/src/main.rs`.

The reported symptom — "the full quad is set to the material colour" — has a
single dominant root cause: the materials add a parameter value (`width_px`,
`blur_px`) directly to `sd`, but the parameter is silently clamped to a
constant expressed in **atlas texels** while `sd` is in **screen pixels**.
The constant is far larger than the screen-px range of `sd`, so the additive
term dominates the formula and the per-fragment coverage saturates to 1
across the entire quad. Bugs 1 and 3 below both reduce to this; bug 4 is a
secondary blending issue that explains why even the unsaturated regions look
wrong.

## Correctness bugs (highest impact first)

### 1. Outline/glow/shadow add a "texel-magnitude" constant to a "screen-pixel" sd → full quad floods

In [`pixel.wgsl:14-30`](text_msdf/src/shaders/pixel.wgsl:14), `unpack_sd_tri`
divides `sd_texels` by `sigma_est` (texels per screen-pixel), so the `sd`
handed to materials is in **screen pixels** and the typical magnitude across
a glyph quad is small — for `DISTANCE_RANGE_PX = 4`, the encoded distance
range is ±2 texels, so `sd` ranges roughly ±2/sigma screen-px (e.g. ±1.5 px
when the glyph is rendered at 48 px from a 64-px atlas).

The material formulas then add a parameter whose clamp ceiling is in atlas
texels:

[`materials/outline.wgsl:18-22`](text_msdf/materials/outline.wgsl:18):

```wgsl
let sdf_range = globals.px_meta.x;            // 4.0 texels
let w = min(p.p0 * px_to_sd, sdf_range * 0.92); // p.p0 is screen-px, cap = texels
...
let body = clamp((sd + w) / aa + 0.5, 0.0, 1.0);
```

When the caller asks for `width_px = 4` or `8`, `w` clamps to `~3.68`. That
number is meant to read as "atlas texels", but it's added to `sd` (screen
pixels) and pushed through `clamp(... + 0.5, 0, 1)`. With `sd ∈ [-1.5, +1.5]`
across the quad, `sd + 3.68 ∈ [+2.18, +5.18]`, so `body` is 1 **everywhere
in the quad**. Result: the entire quad renders as outline colour.

Same shape for glow, [`materials/glow.wgsl:19-20`](text_msdf/materials/glow.wgsl:19),
where `radius` clamped at `~4.6` and used as `t = glow_dist / radius` keeps
the halo strong across the whole quad rather than falling off near the edge.

Same shape for shadow, [`materials/shadow.wgsl:19-31`](text_msdf/materials/shadow.wgsl:19):

```wgsl
let blur = min(max(p.p2 * px_to_sd, 0.001), 12.0);
...
let sh_fill = clamp((sd_sh + blur * 0.7) / aa + 0.5, 0.0, 1.0);
```

With `blur_px = 8` from the demo, `blur * 0.7 = 5.6` — added to `sd_sh`
(screen-px, range ~±1.5) it saturates `sh_fill = 1` across the quad and the
whole rectangle becomes shadow.

**Root fix.** Drop the clamp and the dead `px_to_sd`/`aa` scaling. The
parameter `width_px` / `blur_px` / `radius_px` should be added directly to
`sd` only after **`sd` is denominated in the same units as the parameter**.
Two ways to make units match:

- Multiply the parameter by `sigma_est` so it converts screen-px → atlas
  texels, and keep `sd` in texels (skip the `/ sigma` in `unpack_sd_tri`).
- Or keep `sd` in screen-px (as now), and don't clamp the parameter at all:
  let saturation happen naturally via the final `clamp(.., 0, 1)`. Distances
  beyond the bake range read as solidly inside/outside, which is the desired
  behaviour anyway.

The second option is the simpler change and is what the original design
described. Either way the magic constants `* 0.92`, `* 1.15`, `* 0.7` go
away.

### 2. Glow can't extend beyond the baked distance range

Even after (1), glow physically cannot extend past `±DISTANCE_RANGE_PX / 2`
texels from the edge — see [`build.rs:34`](text_msdf/build.rs:34) and
[`build.rs:320`](text_msdf/build.rs:320). fdsm's `generate_mtsdf` writes
`(distance / range) * 0.5 + 0.5`, so the encoded range saturates at
`±range/2 = ±2` texels. Outside that, both the median channels and the alpha
channel pin at 0, so `-sd` flatlines and the halo can't grow.

Phase 2's verification step 3 anticipated this and suggested using the MTSDF
alpha channel — but in fdsm the alpha channel is bounded by the same `range`
argument, so reading alpha alone doesn't help. The only physical fix is to
bake with a wider range (e.g. 12–16 texels) when you want glow > ~4 screen-px
on a native-size render.

### 3. Shadow samples atlas-UV space, reads into neighbouring glyphs

Independent of (1), the shadow material's second sample is broken even when
the additive-offset issue is fixed.

[`pixel.wgsl:50-52`](text_msdf/src/shaders/pixel.wgsl:50) and
[`materials/shadow.wgsl:20-22`](text_msdf/materials/shadow.wgsl:20):

```wgsl
s_sh = textureSample(atlas_tex, atlas_samp, in.uv + vec2(p.p0, p.p1));
```

`p.p0` / `p.p1` is `offset_uv` in atlas-space — e.g. `[0.004, 0.005]` in
[`demo_msdf/src/main.rs:358`](demo_msdf/src/main.rs:358). On a 2048-wide
atlas that's 8–10 texels of offset, easily into a neighbouring glyph's cell
or the inter-cell `127` padding ([`build.rs:245`](text_msdf/build.rs:245),
[`build.rs:332-342`](text_msdf/build.rs:332)). The sampler returns either
the wrong glyph's ink or neutral gray, so `s_sh` is unrelated to the actual
glyph and the resulting shadow is the wrong shape. The `trust` smoothstep
([`materials/shadow.wgsl:26-28`](text_msdf/materials/shadow.wgsl:26)) is a
patch that hides exactly this — the gate's existence is the signature of the
underlying bug.

Two cleaner paths, simpler one first:

1. **Drop the second sample.** Emit a separate shadow quad: same atlas UVs,
   same vertex order, screen-space-offset positions, shadow material applied.
   One texture sample per fragment, no `s_sh`, no `trust`, no
   `MATERIAL_TAG_SHADOW` branch in `pixel.wgsl`.
2. If single-pass is required: switch `offset` to screen pixels, convert to
   per-glyph atlas UV at sample time using the glyph's UV rect (pass
   `uv_min`/`uv_max` as an extra flat vertex attribute), and clamp the
   displaced UV to that rect before sampling.

### 4. Glow and shadow output premultiplied-style RGB into a non-premultiplied blend

Renderer uses `BlendState::ALPHA_BLENDING`
([`renderer.rs:232`](text_msdf/src/renderer.rs:232)) which is
`src.rgb * src.a + dst.rgb * (1 - src.a)`.

Outline returns `vec4(mix(oc.rgb, base.rgb, fill), body * base.a)` — RGB is
the un-premultiplied colour, alpha controls coverage. Correct.

Glow ([`materials/glow.wgsl:32-34`](text_msdf/materials/glow.wgsl:32)):

```wgsl
let rgb = base_color.rgb * fill + gcol.rgb * glow_contrib * (1.0 - fill);
let a   = fill * base_color.a + glow_contrib * (1.0 - fill * 0.85);
return vec4(rgb, a);
```

In the halo region `fill ≈ 0`, so `rgb ≈ gcol.rgb * glow_contrib` and
`a ≈ glow_contrib`. The blend op multiplies rgb by `a` again, so the glow
reaches the framebuffer as `gcol.rgb * glow_contrib²` — the falloff is
squared. Even after (1) is fixed, glow halos look dim and the colour shifts
toward black at the edges.

Shadow has the same shape
([`materials/shadow.wgsl:34-35`](text_msdf/materials/shadow.wgsl:34)).

Fix: return un-premultiplied RGB everywhere. Sketch for glow:

```wgsl
let coverage = clamp(fill + glow_contrib * (1.0 - fill), 0.0, 1.0);
let mix_t    = fill / max(coverage, 1e-6);
let rgb      = mix(gcol.rgb, base_color.rgb, mix_t);
return vec4(rgb, coverage);
```

### 5. `sigma` floor blurs zoomed-in text

[`pixel.wgsl:26`](text_msdf/src/shaders/pixel.wgsl:26):

```wgsl
let sigma = max(sigma_est, px_range * 0.06);  // floor = 0.24 texels/screen-px
```

When the glyph is rendered larger than its atlas footprint, `sigma_est` is
small and the floor activates. With `sigma = 0.24`, the `sd / sigma`
amplification is ~4.17 — each atlas texel of distance becomes ~4 screen-px
of transition. Sharp edges go to a 4-px ramp. The comment claims this
prevents "outline / glow widths blowing out into solid quads"; once (1) is
fixed, that fear goes away and the floor is just a zoom-dependent blur.
Replace with a small divide-by-zero epsilon (`1e-4`) or remove.

### 6. Hardwired shadow branch in pixel.wgsl defeats the "drop-a-wgsl-file" plugin model

[`pixel.wgsl:50-52`](text_msdf/src/shaders/pixel.wgsl:50) special-cases
`MATERIAL_TAG_SHADOW` to take a second texture sample. The build script
finds the tag by string match on `stem == "shadow"`
([`build_materials.rs:286-288`](text_msdf/build_materials.rs:286)). The
design said materials are pluggable by dropping a `.wgsl` file — but any
future material that needs a second sample now requires editing `pixel.wgsl`
*and* the build script.

After fix (3.1), the conditional sample, the `MATERIAL_TAG_SHADOW` const,
and the `s_sh` parameter all disappear. If single-pass shadows must be kept
(3.2), expose a `sample_atlas_local(uv_offset)` helper to materials and let
them opt in — no per-material branch in pixel.wgsl.

### 7. Unused parameters bloat every material signature

Every material takes `(sd, sd_alpha, aa, uv, base_color, s, s_sh, p,
px_to_sd)`. Fill uses only the first three. Outline uses none of `s`,
`s_sh`, `sd_alpha`, `uv`. After (1)+(3)+(6) the signature can collapse to
`(sd, base_color, p)` plus optionally `sd_alpha` if a material wants the
fallback channel. The `let a2 = a + px_to_sd * 0.0;` line in
[`materials/fill.wgsl:16-17`](text_msdf/materials/fill.wgsl:16) is a tell —
the author noticed the parameters were dead and worked around it instead of
removing them.

## Smaller correctness / simplicity issues

8. **`px_to_sd = 1.0` hardcoded** at [`pixel.wgsl:59`](text_msdf/src/shaders/pixel.wgsl:59).
   Always 1. Every material multiplies by it defensively for no effect.
   Remove the parameter; if (1) is fixed via the "convert param to texels"
   route, replace this with the actual `sigma_est` instead.

9. **`aa = 1.0` hardcoded** at [`pixel.wgsl:60`](text_msdf/src/shaders/pixel.wgsl:60).
   Same story — every material divides by `aa` as if it were variable. Pick
   one transition function and inline it. `smoothstep(-0.5, 0.5, sd)` gives
   a nicer gradient than `clamp(sd + 0.5, 0, 1)`.

10. **`build_materials.rs:359`** emits `cargo:rerun-if-changed=materials`.
    Cargo recurses into directories for this directive, so editing a single
    `.wgsl` does trigger a rebuild — but emitting one line per discovered
    file makes the intent explicit and shows up clearly in `cargo build -vv`.

11. **`build_materials.rs:284-339`** emits an `if/else if` chain instead of
    a WGSL `switch`. The comment "avoid switch on some naga versions" is
    out of date — current naga supports `switch`. The chain forces the
    `i == 0` special-case at line 322; a switch is shorter and uniform.

12. **`build_materials.rs:175`**: header validation in `validate_header`
    runs *after* `load_material` already parsed and partially validated the
    TOML. `param_slot_count` panics during the rust-codegen pass too. Three
    passes that can panic for the same input — merge into one validation
    pass at parse time.

13. **`build_materials.rs:33`**: `expected_fn` is stored on `ParsedMaterial`
    *and* recomputed inside `load_material`. Pick one.

14. **`Material` enum** is `Copy` with up to 32 bytes of params, and
    `pack_for_vertex` is its only consumer. The whole rust codegen could
    collapse to: each material stores its packed `[f32; 8]` and a tag
    directly; constructors emit typed packs. Eliminates the big match arms
    in [`build_materials.rs:228-272`](text_msdf/build_materials.rs:228).

15. **[`vertex.rs:16`](text_msdf/src/vertex.rs:16)** carries an explicit
    `_pad_mat: [u32; 3]` between `mat_tag` and `mat_p0`. WGPU vertex
    attributes only require 4-byte alignment, not 16; the pad just makes
    `mat_p0` land on a 16-byte boundary as a stylistic choice. Dropping it
    (and updating renderer offsets 36/52 for `mat_p0`/`mat_p1`) saves 12 B
    per vertex × 6 verts × ~30k glyphs ≈ 2 MB/frame on the torture workload.

16. **Demo passes `offset_uv` in atlas-UV** at
    [`demo_msdf/src/main.rs:358`](demo_msdf/src/main.rs:358), while
    `width_px`, `radius_px`, `blur_px` are in screen pixels. Inconsistent;
    after fix (3) this becomes `offset_px: [f32; 2]`.

## Suggested order of attack

1. **Fix the unit mismatch (#1).** This is the cause of the full-quad
   flooding. Remove the texel-magnitude clamps and the dead
   `px_to_sd`/`aa` scaffolding in one pass. Outline and shadow should
   immediately stop painting the whole quad.
2. **Drop the shadow second sample (#3.1).** Emit a separate shadow quad
   with offset positions. Removes `s_sh`, `MATERIAL_TAG_SHADOW`, `trust`,
   and the hardwired branch (#6) at the same time.
3. **Fix the premul-vs-non-premul output in glow/shadow (#4).** Now the
   halos and shadows reach the framebuffer at the intended intensity.
4. **Strip dead parameters (#7–9).** Material signature shrinks to
   `(sd, base_color, p)`.
5. **Bake a wider distance range (#2)** if glow radii > ~4 px need to
   actually grow.
6. **Drop the `sigma` floor (#5).**
