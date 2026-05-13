//! Writes `atlas.{png,bin}` + `manifest.json` into Cargo [`OUT_DIR`] for [`include_bytes!`], and
//! mirrors them under the workspace-root `.text_msdf_atlas/` so you can invalidate the atlas by
//! deleting that folder (`rm -rf .text_msdf_atlas`) without `cargo clean` across the workspace.
//! MSDF raster uses pure-Rust [`fdsm`] + [`fdsm_ttf_parser`] (avoids `msdfgen-sys` on Windows).

#[path = "build_materials.rs"]
mod build_materials;

#[path = "src/atlas_format.rs"]
mod atlas_format;

#[path = "src/rect_pack.rs"]
mod rect_pack;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use atlas_format::{AtlasFile, AtlasHeader, GlyphRecord, ATLAS_FORMAT_VERSION, ATLAS_MAGIC};
use fdsm::bezier::scanline::FillRule;
use fdsm::correct_error::{correct_error_mtsdf, ErrorCorrectionConfig};
use fdsm::generate::generate_mtsdf;
use fdsm::render::correct_sign_mtsdf;
use fdsm::shape::Shape;
use fdsm::transform::Transform;
use fdsm_ttf_parser::load_shape_from_face;
use image::{ImageBuffer, Rgba};
use nalgebra::{convert, Affine2, Similarity2, Vector2};
use rect_pack::{PackedRect, ShelfPacker};
use rustybuzz::{shape, script, Direction, Face as HbFace, Feature, UnicodeBuffer};
use serde::{Deserialize, Serialize};
use ttf_parser::{Face, GlyphId, Rect, Tag};

const GENERATOR_VERSION: &str =
    "msdf-phase1-v13-density-em-extra-radius-shelf-pack";
/// Persists the baked atlas next to the repo root (sibling of `text_msdf/`) for easy cache clears.
const WORKSPACE_ATLAS_CACHE_DIR: &str = ".text_msdf_atlas";
/// Atlas sampling density `D` (constant texels span per **1 em**).
const ATLAS_PX_PER_EM: u32 = 96;
/// Half of MSDF encoded span (`R_em/2`): padding per atlas side equals this × [`ATLAS_PX_PER_EM`]
/// texels. Total encoded sdf span (`pxrange` for fdsm/shader) is `2 * EM_EXTRA_RADIUS * D`.
/// Larger values allow thicker outline/glow in screen px before the field saturates (~`em_extra_radius * size_px`).
const EM_EXTRA_RADIUS: f64 = 0.2;

/// Minimum shelf row width (power of two). Keeps small glyphs from producing absurdly tall atlases.
const ATLAS_SHELF_WIDTH_MIN: u32 = 2048;
/// Maximum shelf row width; must stay a power of two for GPU texture rules.
const ATLAS_SHELF_WIDTH_CAP: u32 = 8192;

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    hash: String,
    em_to_px: u32,
    em_extra_radius: f64,
    atlas_size: [u32; 2],
}

struct RasterGlyph {
    pixels: Vec<u8>,
    w: u32,
    h: u32,
    plane_min_em: [f32; 2],
    plane_max_em: [f32; 2],
    has_ink: bool,
}

fn main() {
    println!("cargo:rerun-if-changed=charset.txt");
    println!("cargo:rerun-if-changed=../assets/Hack-Regular.ttf");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/shaders/pixel.wgsl");

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.join("..");
    let cache_dir = workspace_root.join(WORKSPACE_ATLAS_CACHE_DIR);
    fs::create_dir_all(&cache_dir).expect("workspace atlas cache dir");

    // When this file is removed (e.g. whole `.text_msdf_atlas/` deleted), Cargo re-runs the script.
    println!("cargo:rerun-if-changed=../.text_msdf_atlas/manifest.json");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR from Cargo"));
    fs::create_dir_all(&out_dir).expect("OUT_DIR");

    build_materials::run(&manifest_dir, &out_dir);

    let charset_path = manifest_dir.join("charset.txt");
    let font_path = manifest_dir.join("../assets/Hack-Regular.ttf");

    let charset_src = fs::read_to_string(&charset_path).expect("charset.txt");
    let chars = parse_charset(&charset_src);
    let chars_vec: Vec<char> = chars.iter().copied().collect();

    let font_bytes = fs::read(&font_path).expect("read font");
    let face = Face::parse(&font_bytes, 0).expect("parse font");

    let mut atlas_gids: BTreeSet<u32> = BTreeSet::new();
    for &ch in &chars_vec {
        if let Some(gid) = face.glyph_index(ch) {
            atlas_gids.insert(gid.0 as u32);
        }
    }
    for gid in collect_shaped_glyph_ids(&font_bytes, &chars_vec) {
        atlas_gids.insert(gid);
    }
    let resolved: Vec<GlyphId> = atlas_gids
        .into_iter()
        .map(|u| GlyphId(u as u16))
        .collect();

    let mut max_pack_glyph_w = 0u32;
    let mut need_sentinel = false;
    for &gid in &resolved {
        match glyph_bitmap_wh_for_atlas(&face, gid) {
            Some((w, _)) => max_pack_glyph_w = max_pack_glyph_w.max(w),
            None => need_sentinel = true,
        }
    }
    let x_margin = u32::from(need_sentinel);
    let atlas_max_width = atlas_pot_shelf_width(max_pack_glyph_w, x_margin);

    let sdf_px_span = (2.0_f64 * EM_EXTRA_RADIUS * f64::from(ATLAS_PX_PER_EM)) as f32;

    let atlas_params = serde_json::json!({
        "em_to_px": ATLAS_PX_PER_EM,
        "em_extra_radius": EM_EXTRA_RADIUS,
        "atlas_shelf_width": atlas_max_width,
        "sdf_px_range": sdf_px_span,
        "generator": GENERATOR_VERSION,
    });

    let mut manifest_hasher_input = Vec::new();
    manifest_hasher_input.extend_from_slice(&font_bytes);
    for &c in &chars_vec {
        let mut buf = [0u8; 4];
        manifest_hasher_input.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    manifest_hasher_input.extend_from_slice(GENERATOR_VERSION.as_bytes());
    manifest_hasher_input.extend_from_slice(atlas_params.to_string().as_bytes());

    let manifest_hash = blake3::hash(&manifest_hasher_input);
    let hash_hex = manifest_hash.to_hex().to_string();

    let png_out = out_dir.join("atlas.png");
    let bin_out = out_dir.join("atlas.bin");
    let manifest_out = out_dir.join("manifest.json");

    let png_cache = cache_dir.join("atlas.png");
    let bin_cache = cache_dir.join("atlas.bin");
    let manifest_cache = cache_dir.join("manifest.json");

    if png_cache.is_file() && bin_cache.is_file() && manifest_cache.is_file() {
        if let Ok(existing) = fs::read_to_string(&manifest_cache) {
            if let Ok(parsed) = serde_json::from_str::<Manifest>(&existing) {
                if parsed.hash == hash_hex {
                    println!("cargo:warning=text_msdf atlas cache hit ({})", hash_hex);
                    fs::copy(&png_cache, &png_out).expect("cache → OUT_DIR atlas.png");
                    fs::copy(&bin_cache, &bin_out).expect("cache → OUT_DIR atlas.bin");
                    fs::copy(&manifest_cache, &manifest_out).expect("cache → OUT_DIR manifest");
                    return;
                }
            }
        }
    }

    println!("cargo:warning=text_msdf regenerating MSDF atlas ({})", hash_hex);

    let upem = face.units_per_em() as f32;
    let font_hash = *blake3::hash(&font_bytes).as_bytes();

    let missing: Vec<char> = chars_vec
        .iter()
        .copied()
        .filter(|c| face.glyph_index(*c).is_none())
        .collect();
    for c in &missing {
        println!(
            "cargo:warning=text_msdf charset char U+{:04X} '{}' missing from font",
            *c as u32, c
        );
    }

    let ec_cfg = ErrorCorrectionConfig::default();

    let mut baked: Vec<(GlyphId, RasterGlyph, f32)> = Vec::with_capacity(resolved.len());

    for &gid in &resolved {
        let advance_em = face
            .glyph_hor_advance(gid)
            .map(|a| a as f32 / upem)
            .unwrap_or(0.0);
        let rg = raster_glyph_fdsm(&face, gid, &ec_cfg);
        baked.push((gid, rg, advance_em));
    }

    let mut pack_order: Vec<(usize, u32, u32)> = Vec::new();
    for (i, (_, ref rg, _)) in baked.iter().enumerate() {
        if rg.has_ink && rg.w > 0 && rg.h > 0 {
            pack_order.push((i, rg.w, rg.h));
        }
    }
    pack_order.sort_by(|a, b| b.2.cmp(&a.2));

    let mut packer = ShelfPacker::with_x_margin(atlas_max_width, x_margin);
    let mut placements: Vec<Option<PackedRect>> = vec![None; baked.len()];
    for (idx, rw, rh) in pack_order {
        let packed = packer.pack(rw, rh).unwrap_or_else(|| {
            let gid = baked[idx].0;
            panic!(
                "glyph_id {} MSDF bake {}×{} does not fit shelf width {} (atlas packer bug — width should have been sized from glyph bounds)",
                gid.0 as u32,
                rw,
                rh,
                atlas_max_width
            )
        });
        placements[idx] = Some(packed);
    }

    let (atlas_w, atlas_h) = packer.dimensions();
    let mut atlas_rgba = vec![0u8; atlas_w as usize * atlas_h as usize * 4];
    if need_sentinel {
        for b in atlas_rgba.iter_mut().take(4) {
            *b = 127;
        }
    }

    let mut glyphs: Vec<GlyphRecord> = Vec::with_capacity(baked.len());

    for (i, &(gid, ref rg, advance_em)) in baked.iter().enumerate() {
        let (uv_min, uv_max) = if let Some(p) = placements[i] {
            blit_rgba(
                &mut atlas_rgba,
                atlas_w,
                p.x,
                p.y,
                &rg.pixels,
                rg.w,
                rg.h,
            );
            let ux0 = p.x;
            let uy0 = p.y;
            let ux1 = p.x + rg.w.saturating_sub(1);
            let uy1 = p.y + rg.h.saturating_sub(1);
            (
                [ux0 as u16, uy0 as u16],
                [ux1 as u16, uy1 as u16],
            )
        } else if need_sentinel {
            ([0_u16; 2], [0_u16; 2])
        } else {
            ([0_u16; 2], [0_u16; 2])
        };

        glyphs.push(GlyphRecord {
            glyph_id: gid.0 as u32,
            uv_min,
            uv_max,
            plane_min_em: rg.plane_min_em,
            plane_max_em: rg.plane_max_em,
            advance_em,
        });
    }

    glyphs.sort_by_key(|g| g.glyph_id);

    let header = AtlasHeader {
        magic: *ATLAS_MAGIC,
        version: ATLAS_FORMAT_VERSION,
        font_hash,
        atlas_w,
        atlas_h,
        em_to_px: ATLAS_PX_PER_EM,
        em_extra_radius: EM_EXTRA_RADIUS as f32,
        units_per_em: face.units_per_em(),
        ascent_em: face.ascender() as f32 / upem,
        descent_em: face.descender() as f32 / upem,
        line_gap_em: face.line_gap() as f32 / upem,
    };

    let atlas_file = AtlasFile { header, glyphs };
    let encoded = bincode::serialize(&atlas_file).expect("bincode atlas");
    fs::write(&bin_out, &encoded).expect("write atlas.bin");
    fs::write(&bin_cache, &encoded).expect("write cache atlas.bin");

    write_png_rgba(&png_out, &atlas_rgba, atlas_w, atlas_h).expect("write atlas.png");
    fs::copy(&png_out, &png_cache).expect("write cache atlas.png");

    let manifest = Manifest {
        hash: hash_hex.clone(),
        em_to_px: ATLAS_PX_PER_EM,
        em_extra_radius: EM_EXTRA_RADIUS,
        atlas_size: [atlas_w, atlas_h],
    };
    let manifest_json = serde_json::to_string_pretty(&manifest).expect("manifest json");
    fs::write(&manifest_out, &manifest_json).expect("write manifest");
    fs::write(&manifest_cache, manifest_json).expect("write cache manifest");
}

fn sdf_px_span() -> f64 {
    2.0 * EM_EXTRA_RADIUS * f64::from(ATLAS_PX_PER_EM)
}

/// Smallest power-of-two shelf width that fits every glyph (`max_glyph_w + x_margin`), with clamping.
fn atlas_pot_shelf_width(max_glyph_w: u32, x_margin: u32) -> u32 {
    let need = max_glyph_w.saturating_add(x_margin).max(1);
    let mut shelf = need.next_power_of_two();
    shelf = shelf.max(ATLAS_SHELF_WIDTH_MIN);
    if shelf > ATLAS_SHELF_WIDTH_CAP {
        panic!(
            "text_msdf: MSDF atlas needs shelf width >= {} texels (widest glyph {} + margin {}). \
             Raise ATLAS_SHELF_WIDTH_CAP, lower ATLAS_PX_PER_EM, or shrink the charset.",
            shelf,
            max_glyph_w,
            x_margin
        );
    }
    shelf
}

/// Bitmap size for MSDF bake — matches [`raster_glyph_fdsm`] (outline glyphs only).
fn bbox_padded_bounds(face: &Face<'_>, gid: GlyphId) -> (f64, f64, f64, f64) {
    let upem64 = face.units_per_em() as f64;
    let bbox = face.glyph_bounding_box(gid).unwrap_or_else(|| {
        let adv = face.glyph_hor_advance(gid).unwrap_or(face.units_per_em() / 4);
        Rect {
            x_min: 0,
            y_min: face.descender(),
            x_max: adv as i16,
            y_max: face.ascender(),
        }
    });
    let pad_min_x = bbox.x_min as f64 - EM_EXTRA_RADIUS * upem64;
    let pad_min_y = bbox.y_min as f64 - EM_EXTRA_RADIUS * upem64;
    let pad_max_x = bbox.x_max as f64 + EM_EXTRA_RADIUS * upem64;
    let pad_max_y = bbox.y_max as f64 + EM_EXTRA_RADIUS * upem64;
    (pad_min_x, pad_min_y, pad_max_x, pad_max_y)
}

fn bitmap_wh_from_padded(
    (pad_min_x, pad_min_y, pad_max_x, pad_max_y): (f64, f64, f64, f64),
    upem64: f64,
) -> (u32, u32) {
    let span_x = pad_max_x - pad_min_x;
    let span_y = pad_max_y - pad_min_y;
    let d = ATLAS_PX_PER_EM as f64;
    let w = ((((span_x / upem64) * d).ceil()) as i64).clamp(1, i64::from(u32::MAX)) as u32;
    let h = ((((span_y / upem64) * d).ceil()) as i64).clamp(1, i64::from(u32::MAX)) as u32;
    (w, h)
}

fn bitmap_wh_from_bbox(face: &Face<'_>, gid: GlyphId) -> (u32, u32) {
    let upem64 = face.units_per_em() as f64;
    bitmap_wh_from_padded(bbox_padded_bounds(face, gid), upem64)
}

fn glyph_bitmap_wh_for_atlas(face: &Face<'_>, gid: GlyphId) -> Option<(u32, u32)> {
    let shape = load_shape_from_face(face, gid)?;
    if shape.contours.is_empty() {
        return None;
    }
    Some(bitmap_wh_from_bbox(face, gid))
}

fn raster_glyph_fdsm(face: &Face<'_>, gid: GlyphId, ec_cfg: &ErrorCorrectionConfig) -> RasterGlyph {
    let upem = face.units_per_em() as f32;
    let upem64 = face.units_per_em() as f64;
    let (plane_lo, plane_hi) = glyph_plane_em(face, gid, upem);
    let fallback = RasterGlyph {
        pixels: Vec::new(),
        w: 0,
        h: 0,
        plane_min_em: plane_lo,
        plane_max_em: plane_hi,
        has_ink: false,
    };

    let Some(mut shape) = load_shape_from_face(face, gid) else {
        return fallback;
    };
    if shape.contours.is_empty() {
        return fallback;
    }

    let pads = bbox_padded_bounds(face, gid);
    let (w, h) = bitmap_wh_from_padded(pads, upem64);
    let (pad_min_x, pad_min_y, pad_max_x, pad_max_y) = pads;

    let d = ATLAS_PX_PER_EM as f64;
    let s = d / upem64;
    let affine = convert::<_, Affine2<f64>>(Similarity2::new(
        Vector2::new(-pad_min_x * s, -pad_min_y * s),
        0.0,
        s,
    ));
    shape.transform(&affine);

    let sin_alpha = (3.0_f64.to_radians()).sin();
    let colored = Shape::edge_coloring_simple(shape, sin_alpha, 0);
    let prepared = colored.prepare();

    let plane_min_em = [
        (pad_min_x / upem64) as f32,
        (pad_min_y / upem64) as f32,
    ];
    let plane_max_em = [
        (pad_max_x / upem64) as f32,
        (pad_max_y / upem64) as f32,
    ];

    let pxrange = sdf_px_span();
    let mut mtsdf: ImageBuffer<Rgba<f32>, Vec<f32>> = ImageBuffer::new(w, h);
    generate_mtsdf(&prepared, pxrange, &mut mtsdf);
    correct_error_mtsdf(&mut mtsdf, &colored, &prepared, pxrange, ec_cfg);
    correct_sign_mtsdf(&mut mtsdf, &prepared, FillRule::Nonzero);
    flip_mtsdf_rows_top_for_font_up(&mut mtsdf);

    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let p = *mtsdf.get_pixel(x, y);
            let i = ((y * w + x) * 4) as usize;
            pixels[i] = (p[0].clamp(0.0, 1.0) * 255.0) as u8;
            pixels[i + 1] = (p[1].clamp(0.0, 1.0) * 255.0) as u8;
            pixels[i + 2] = (p[2].clamp(0.0, 1.0) * 255.0) as u8;
            pixels[i + 3] = (p[3].clamp(0.0, 1.0) * 255.0) as u8;
        }
    }

    RasterGlyph {
        pixels,
        w,
        h,
        plane_min_em,
        plane_max_em,
        has_ink: true,
    }
}

/// fdsm generates with Y pointing opposite to our atlas / ttf “y up” convention — flip rows so
/// bitmap row 0 is the typographic top (matches PNG + `queue.write_texture` top-first).
fn flip_mtsdf_rows_top_for_font_up(img: &mut ImageBuffer<Rgba<f32>, Vec<f32>>) {
    let w = img.width();
    let h = img.height();
    if h <= 1 {
        return;
    }
    for y in 0..h / 2 {
        for x in 0..w {
            let a = *img.get_pixel(x, y);
            let b = *img.get_pixel(x, h - 1 - y);
            img.put_pixel(x, y, b);
            img.put_pixel(x, h - 1 - y, a);
        }
    }
}

fn rustybuzz_shape_features() -> [Feature; 2] {
    [
        Feature::new(Tag::from_bytes(b"liga"), 0, ..),
        Feature::new(Tag::from_bytes(b"clig"), 0, ..),
    ]
}

/// Extra glyph IDs produced by HarfBuzz for `charset.txt` codepoints (same settings as `layout.rs`).
fn collect_shaped_glyph_ids(font_bytes: &[u8], chars: &[char]) -> BTreeSet<u32> {
    let hb_face = HbFace::from_slice(font_bytes, 0).expect("rustybuzz parse font");
    let feats = rustybuzz_shape_features();
    let mut out = BTreeSet::new();
    for &ch in chars {
        let mut ub = UnicodeBuffer::new();
        let mut enc = [0u8; 4];
        ub.push_str(ch.encode_utf8(&mut enc));
        ub.set_direction(Direction::LeftToRight);
        ub.set_script(script::LATIN);
        ub.set_language("en".parse().expect("language tag"));
        let gb = shape(&hb_face, &feats, ub);
        for info in gb.glyph_infos() {
            out.insert(info.glyph_id);
        }
    }
    out
}

fn parse_charset(src: &str) -> BTreeSet<char> {
    let mut set = BTreeSet::new();
    for raw_line in src.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((a, b)) = line.split_once('-') {
            let start = u32::from_str_radix(a.trim(), 16).expect("range start");
            let end = u32::from_str_radix(b.trim(), 16).expect("range end");
            for cp in start..=end {
                if let Some(ch) = char::from_u32(cp) {
                    set.insert(ch);
                }
            }
        } else {
            for ch in line.chars() {
                set.insert(ch);
            }
        }
    }
    set
}

fn glyph_plane_em(face: &Face<'_>, gid: GlyphId, upem: f32) -> ([f32; 2], [f32; 2]) {
    // Advance-only glyphs (space, etc.): old fallback used a ~0.01em-tall quad → one-pixel baseline streak.
    const EPS: f32 = 1e-6;
    if let Some(bb) = face.glyph_bounding_box(gid) {
        let min_x = bb.x_min as f32 / upem;
        let min_y = bb.y_min as f32 / upem;
        let max_x = bb.x_max as f32 / upem;
        let max_y = bb.y_max as f32 / upem;
        if (max_x - min_x).abs() <= EPS && (max_y - min_y).abs() <= EPS {
            return ([min_x, min_y], [min_x, min_y]);
        }
        ([min_x, min_y], [max_x, max_y])
    } else {
        ([0.0, 0.0], [0.0, 0.0])
    }
}

fn blit_rgba(
    atlas: &mut [u8],
    atlas_w: u32,
    dst_x: u32,
    dst_y: u32,
    src: &[u8],
    src_w: u32,
    src_h: u32,
) {
    debug_assert!(
        src.len() >= src_w as usize * src_h as usize * 4,
        "src buffer too small"
    );
    for y in 0..src_h {
        let dst_base = (((dst_y + y) * atlas_w + dst_x) * 4) as usize;
        let src_base = ((y * src_w) * 4) as usize;
        atlas[dst_base..dst_base + src_w as usize * 4]
            .copy_from_slice(&src[src_base..src_base + src_w as usize * 4]);
    }
}

fn write_png_rgba(path: &Path, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
    let file = fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    Ok(())
}
