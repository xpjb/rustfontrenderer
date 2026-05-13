//! Writes `atlas.{png,bin}` + `manifest.json` into Cargo [`OUT_DIR`] for [`include_bytes!`], and
//! mirrors them under the workspace-root `.text_msdf_atlas/` so you can invalidate the atlas by
//! deleting that folder (`rm -rf .text_msdf_atlas`) without `cargo clean` across the workspace.
//! MSDF raster uses pure-Rust [`fdsm`] + [`fdsm_ttf_parser`] (avoids `msdfgen-sys` on Windows).

#[path = "build_materials.rs"]
mod build_materials;

#[path = "src/atlas_format.rs"]
mod atlas_format;

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
use nalgebra::{convert, Affine2, Point2, Similarity2, Vector2};
use rustybuzz::{shape, script, Direction, Face as HbFace, Feature, UnicodeBuffer};
use serde::{Deserialize, Serialize};
use ttf_parser::{Face, GlyphId, Rect, Tag};

const GENERATOR_VERSION: &str = "msdf-phase1-v12-space-invisible-ssaa";
/// Persists the baked atlas next to the repo root (sibling of `text_msdf/`) for easy cache clears.
const WORKSPACE_ATLAS_CACHE_DIR: &str = ".text_msdf_atlas";
const GLYPH_PX: u32 = 64;
/// Atlas distance range in atlas pixels (this is `pxrange`; in em terms it's `range / GLYPH_PX`).
/// Each atlas pixel beyond ±range/2 from the contour saturates to the byte extreme, so any
/// material that does `clamp(sd + w + 0.5)` will paint a uniform rectangle wherever `w + 0.5`
/// exceeds the encoded distance. To support outline width w / glow radius r cleanly at 1:1 render
/// scale, need range ≥ 2(max(w, r) + 0.5). Currently 16 → safe up to ~7 px before partial leak.
const DISTANCE_RANGE_PX: f64 = 16.0;
const ATLAS_MAX_WIDTH: u32 = 2048;

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    hash: String,
    glyph_px: u32,
    distance_range_px: f64,
    atlas_size: [u32; 2],
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

    let glyph_px_f = GLYPH_PX as f64;
    let cell = ((glyph_px_f + DISTANCE_RANGE_PX).ceil()) as u32;
    let cols = ATLAS_MAX_WIDTH / cell;

    let atlas_params = serde_json::json!({
        "glyph_px": GLYPH_PX,
        "distance_range_px": DISTANCE_RANGE_PX,
        "atlas_max_width": ATLAS_MAX_WIDTH,
        "cell_px": cell,
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

    let n = resolved.len().max(1);
    let rows = ((n as u32 + cols - 1) / cols).max(1);
    let atlas_w = cols * cell;
    let atlas_h = rows * cell;

    let mut atlas_rgba = vec![0u8; (atlas_w * atlas_h * 4) as usize];

    let mut glyphs: Vec<GlyphRecord> = Vec::with_capacity(resolved.len());

    let ec_cfg = ErrorCorrectionConfig::default();

    for (i, &gid) in resolved.iter().enumerate() {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        let ox = col * cell;
        let oy = row * cell;

        let advance_em = face
            .glyph_hor_advance(gid)
            .map(|a| a as f32 / upem)
            .unwrap_or(0.0);

        let (cell_rgba, gw, gh, plane_min, plane_max) =
            raster_glyph_fdsm(&face, gid, cell, DISTANCE_RANGE_PX, &ec_cfg, upem);

        blit_cell(&mut atlas_rgba, atlas_w, ox, oy, cell, &cell_rgba);

        let (uv_min, uv_max) = if gw > 0 && gh > 0 {
            let ix = (cell - gw) / 2;
            let iy = (cell - gh) / 2;
            (
                [ox + ix, oy + iy],
                [ox + ix + gw - 1, oy + iy + gh - 1],
            )
        } else {
            ([ox, oy], [ox + cell - 1, oy + cell - 1])
        };

        glyphs.push(GlyphRecord {
            glyph_id: gid.0 as u32,
            uv_min: [uv_min[0] as u16, uv_min[1] as u16],
            uv_max: [uv_max[0] as u16, uv_max[1] as u16],
            plane_min_em: plane_min,
            plane_max_em: plane_max,
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
        glyph_px: GLYPH_PX,
        distance_range_px: DISTANCE_RANGE_PX as f32,
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
        glyph_px: GLYPH_PX,
        distance_range_px: DISTANCE_RANGE_PX,
        atlas_size: [atlas_w, atlas_h],
    };
    let manifest_json = serde_json::to_string_pretty(&manifest).expect("manifest json");
    fs::write(&manifest_out, &manifest_json).expect("write manifest");
    fs::write(&manifest_cache, manifest_json).expect("write cache manifest");
}

fn raster_glyph_fdsm(
    face: &Face<'_>,
    gid: GlyphId,
    cell: u32,
    range: f64,
    ec_cfg: &ErrorCorrectionConfig,
    upem: f32,
) -> (Vec<u8>, u32, u32, [f32; 2], [f32; 2]) {
    let fallback_plane = glyph_plane_em(face, gid, upem);
    const CELL_BPP: usize = 4;
    let mut cell_rgba = vec![127u8; (cell * cell) as usize * CELL_BPP];
    let Some(mut shape) = load_shape_from_face(face, gid) else {
        return (cell_rgba, 0, 0, fallback_plane.0, fallback_plane.1);
    };
    if shape.contours.is_empty() {
        return (cell_rgba, 0, 0, fallback_plane.0, fallback_plane.1);
    }

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

    let bw = (bbox.x_max - bbox.x_min) as f64;
    let bh = (bbox.y_max - bbox.y_min) as f64;
    let inner = (cell as f64 - 2.0 * range).max(1.0);
    let shrinkage = (upem64 / GLYPH_PX as f64)
        .max(if bw > 0.0 { bw / inner } else { 0.0 })
        .max(if bh > 0.0 { bh / inner } else { 0.0 })
        .max(1.0e-6);

    let transformation = convert::<_, Affine2<f64>>(Similarity2::new(
        Vector2::new(
            range - bbox.x_min as f64 / shrinkage,
            range - bbox.y_min as f64 / shrinkage,
        ),
        0.0,
        1.0 / shrinkage,
    ));
    shape.transform(&transformation);

    let sin_alpha = (3.0_f64.to_radians()).sin();
    let colored = Shape::edge_coloring_simple(shape, sin_alpha, 0);
    let prepared = colored.prepare();

    let w = ((bw / shrinkage + 2.0 * range).ceil() as u32).max(1).min(cell);
    let h = ((bh / shrinkage + 2.0 * range).ceil() as u32).max(1).min(cell);

    let inv = transformation.try_inverse().expect("similarity invertible");
    let wp = (w.max(1) - 1) as f64;
    let hp = (h.max(1) - 1) as f64;
    // Bitmap rows are flipped before upload; these corners are pre-flip UV/sample corners.
    let pre_flip_corners = [
        Point2::new(0.0, hp),
        Point2::new(wp, hp),
        Point2::new(0.0, 0.0),
        Point2::new(wp, 0.0),
    ];
    let mut min_px = f64::INFINITY;
    let mut max_px = f64::NEG_INFINITY;
    let mut min_py = f64::INFINITY;
    let mut max_py = f64::NEG_INFINITY;
    for c in pre_flip_corners {
        let p = inv.transform_point(&c);
        min_px = min_px.min(p.x);
        max_px = max_px.max(p.x);
        min_py = min_py.min(p.y);
        max_py = max_py.max(p.y);
    }
    let plane_min = [
        min_px as f32 / upem,
        min_py as f32 / upem,
    ];
    let plane_max = [
        max_px as f32 / upem,
        max_py as f32 / upem,
    ];

    let mut mtsdf: ImageBuffer<Rgba<f32>, Vec<f32>> = ImageBuffer::new(w, h);
    generate_mtsdf(&prepared, range, &mut mtsdf);
    correct_error_mtsdf(
        &mut mtsdf,
        &colored,
        &prepared,
        range,
        ec_cfg,
    );
    correct_sign_mtsdf(&mut mtsdf, &prepared, FillRule::Nonzero);
    flip_mtsdf_rows_top_for_font_up(&mut mtsdf);

    let ox = (cell - w) / 2;
    let oy = (cell - h) / 2;
    for y in 0..h {
        for x in 0..w {
            let p = *mtsdf.get_pixel(x, y);
            let di = (((oy + y) * cell + (ox + x)) as usize) * CELL_BPP;
            cell_rgba[di] = (p[0].clamp(0.0, 1.0) * 255.0) as u8;
            cell_rgba[di + 1] = (p[1].clamp(0.0, 1.0) * 255.0) as u8;
            cell_rgba[di + 2] = (p[2].clamp(0.0, 1.0) * 255.0) as u8;
            cell_rgba[di + 3] = (p[3].clamp(0.0, 1.0) * 255.0) as u8;
        }
    }

    (cell_rgba, w, h, plane_min, plane_max)
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

fn blit_cell(
    atlas: &mut [u8],
    atlas_w: u32,
    ox: u32,
    oy: u32,
    cell: u32,
    cell_rgba: &[u8],
) {
    let bpp = 4usize;
    for y in 0..cell {
        let dst_row = ((oy + y) * atlas_w + ox) as usize * bpp;
        let src_row = (y * cell) as usize * bpp;
        let len = cell as usize * bpp;
        atlas[dst_row..dst_row + len].copy_from_slice(&cell_rgba[src_row..src_row + len]);
    }
}

fn write_png_rgba(path: &Path, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
    // IHDR + IDAT only: default `png::Encoder` does NOT emit sRGB/gAMA/iCCP unless we call
    // `set_source_srgb` / `set_source_gamma`. Embedding those marks “display referred” sRGB data;
    // MSDF texels are linear-ish masks and must be uploaded as `Rgba8Unorm` without decode — PNG is
    // still lossless; breakage usually comes from 8-bit quantisation during bake or editors re-saving
    // with colour-management chunks plus downstream loaders treating the texture as sRGB.
    let file = fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    Ok(())
}
