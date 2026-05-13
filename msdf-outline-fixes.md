# MSDF atlas + outline redesign

Current build.rs is built on a confused abstraction: `GLYPH_PX` pretends to be
both a per-glyph density AND a uniform cell size, and `DISTANCE_RANGE_PX` is
mixed into both. When you change one, the other silently changes too. That's
why bumping `DISTANCE_RANGE_PX` from 12 → 32 silently halved glyph resolution
without anyone noticing, and why padding ended up being twice what's encoded.

The reference binary (msdf-atlas-gen by Chlumsky) treats these as fully
independent concepts: a fixed atlas density, an em-relative distance range,
per-glyph rects sized to actual bboxes, rect-packed. We should do the same.

## Concepts (the ones we should be using)

- **Density** `D` (atlas-pixels per em). Sets sharpness. Constant across all
  glyphs in an atlas. This is what `GLYPH_PX` *should have meant*.
- **emrange** `R_em` (em fraction). The SDF's encoded distance range, in em.
  Replaces `DISTANCE_RANGE_PX`. Scale-invariant: `pxrange = R_em * D` is
  derived, not a primary param.
- **Per-glyph atlas rect**: `(bbox_em * D)` atlas-pixels for the ink, plus
  `(R_em/2 * D)` of padding on **each side**. That's it — padding equals what
  the SDF actually encodes, no wasted saturated annulus.
- **Atlas layout**: rect-packed. No grid. No uniform cell. Each glyph
  occupies exactly its padded bbox in the atlas. Same packer the reference
  uses (shelf or skyline is plenty for a font atlas).

Everything else falls out of these:
- Pxrange = `R_em * D` (for the shader to consume).
- Quad em-extent for a glyph = `bbox_em` inflated by `R_em/2` on each side.
- Max outline width / glow radius **at any rendering scale** = `R_em/2` em.

## Current code in terms of these concepts

| Concept | Current code | Notes |
|---|---|---|
| Density `D` | `GLYPH_PX = 64` | But silently dropped to `D * (inner/GLYPH_PX)` for wide glyphs |
| `R_em` | `DISTANCE_RANGE_PX / GLYPH_PX = 0.25` | Stored as pxrange, not emrange |
| Padding per side | `range = DISTANCE_RANGE_PX` atlas-px | 2× what's encoded; outer half is saturated and inside the quad |
| Per-glyph rect | Fits in `inner = GLYPH_PX − DISTANCE_RANGE_PX` | Wide glyphs squished; density is *not* actually constant |
| Atlas layout | Fixed grid of `cell = GLYPH_PX + DISTANCE_RANGE_PX` | Wastes atlas memory; not actually needed for correctness |

The bugs we've been hitting are direct consequences of this:
- "Cyan rectangle" leak → outer half of padding is saturated and rendered.
- "Resolution drops with bigger range" → density coupled to padding via `inner`.
- "Outline behaves differently at different window sizes" → width is in screen
  px while the encoded range is in atlas px, so the conversion factor shifts
  with render scale.

## Fixes

### Fix #1 — Replace the atlas pipeline with density + emrange + rect packer

**Where:** [text_msdf/build.rs](text_msdf/build.rs).

**New constants:**
```rust
const ATLAS_PX_PER_EM: u32 = 64;      // density D
const DISTANCE_RANGE_EM: f64 = 0.25;  // R_em, ≈ 0.2-0.25 is typical
// pxrange = ATLAS_PX_PER_EM as f64 * DISTANCE_RANGE_EM = 16.0, derived
```

**Per glyph:**
1. Read `bbox` from `face.glyph_bounding_box(gid)`, convert to em-units by
   dividing by `upem`. Get `bbox_em = (w_em, h_em)`.
2. Render bitmap of size
   `((bbox_em + R_em) * D).ceil()` atlas-pixels per dimension.
3. Position the glyph inside the bitmap so its `bbox.x_min` lands at
   atlas-pixel offset `R_em/2 * D` (the `range/2` padding).
4. Call `generate_mtsdf(prepared, pxrange, bitmap)` where
   `pxrange = R_em * D`. fdsm's `range` param is the *encoded total range* —
   not a padding amount — so we pass `pxrange` here even though our bitmap is
   sized to half of what the current code uses.
5. **No `shrinkage = max(...)`.** Density is fixed at `D`; the bitmap size
   follows the glyph, not the other way around.

**Atlas layout:**
1. Compute per-glyph rects from step 1-2.
2. Rect-pack with `rectangle-pack` crate or a small shelf packer (~50 lines).
   Shelf packing is fine for fonts because glyph heights cluster around one
   or two values (cap height, ascender+descender).
3. Atlas size = result of the pack. No fixed grid.
4. Write each glyph's bitmap into its packed atlas rect.

**Per-glyph metadata:**
- Atlas UV rect (already stored in `GlyphRecord` — verify).
- Plane bounds in em-units: `bbox_em` inflated by `R_em/2` per side, which is
  what the screen quad spans.

**Shader-side:** unchanged. `pxrange` (uniform `globals.px_meta.x`) is now
`R_em * D = 16.0` with the new constants. Coincidentally the same as the
current value, so the shader doesn't need to know anything changed.

**Effect:**
- Glyph quads now cover only the encoded SDF region. **No saturated annulus
  inside the quad** → the cyan rectangle bug can't appear, ever, regardless
  of outline width. Outlines wider than `R_em/2` em clip at the quad edge
  (clean hard cut, not a ghost rectangle).
- Density is genuinely constant. Wide glyphs render at the same sharpness as
  narrow ones, instead of being silently squished.
- Atlas is tighter (no wasted cells, no saturated padding ring).
- `DISTANCE_RANGE_EM` is now the param to tune for thicker effects, and it
  composes correctly with `ATLAS_PX_PER_EM` (bumping density doesn't break
  the encoded distance range).

**Risks:**
- Atlas format may need a small change to store per-glyph atlas rects if it
  currently relies on uniform cell math. Check `GlyphRecord`. Likely already
  stores per-glyph UVs, in which case no format change.
- The `rectangle-pack` crate adds a dep; alternatively roll a shelf packer
  inline (~50 lines).
- Existing atlas binaries get invalidated (hash includes generator params),
  which is the expected/desired behavior.

### Fix #2 — Em-proportional outline width in the demo

**Where:** [demo_msdf/src/main.rs](demo_msdf/src/main.rs).

**Helper:**
```rust
fn outline_width_px(font_size_px: f32, em_fraction: f32) -> f32 {
    (font_size_px * em_fraction).max(1.0)
}
```

`em_fraction` is bounded by `DISTANCE_RANGE_EM / 2` (= 0.125 with `R_em=0.25`).
Anything bigger exceeds the encoded range and clips at the quad edge.

**Swatch update:** the outline row currently passes literal `width_px` values
of 1/2/4/8. Replace with em-fractions like 1/64, 1/32, 1/16, 1/12 and let the
helper compute screen px. Update the swatch labels accordingly (e.g. `1/16 em`
instead of `w=4`). Same treatment for the glow row's `radius_px`.

**Effect:** outline thickness stays a fixed fraction of glyph height at any
window size. Resizing can't shove a fixed-px width past the encoded range.
The `max(1.0)` floor keeps it visible when the font is rendered very small.

**Risks:** none — pure caller-side change, shader and atlas unaffected.

## Order of operations

1. **Fix #2 first** (demo-side, isolated). Verify resizing no longer triggers
   the rectangle at the current (range=16) atlas. Even with the old layout
   still in place, em-proportional widths stay within the encoded range at
   every render scale.
2. **Fix #1** (atlas pipeline). Big change but it's mostly in one file. After
   this lands, the rectangle leak is gone *architecturally*, not by virtue of
   width discipline.
3. Re-tune `DISTANCE_RANGE_EM` if needed. 0.25 covers em_fraction up to 0.125
   — comfortable for the demo's range of swatch params. Bump to 0.3 if we
   ever want thicker.

## Out of scope (intentionally)

- The `unpack_sd_tri` `mix(median, alpha, smoothstep(spread))` blend is fine.
  That's MSDF corner-handling, unrelated to the layout problems.
- Glow's `pow(1 − u, 2.5)` falloff is a tuning choice, not a bug.
- Tightening the atlas image size further with a perfect packer (vs. shelf).
  Shelf is good enough; only optimize this if atlas memory becomes a problem.
