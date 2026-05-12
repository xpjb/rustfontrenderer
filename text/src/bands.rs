//! Band division and curve sorting.
//!
//! Slug splits each glyph's bbox into N horizontal and N vertical bands; each band
//! lists curves that intersect it, sorted by descending max coordinate so the pixel
//! shader can early-out. Output: a compact curve texel stream (RGBA32F, 2 texels
//! per curve) and a band/index texture stream (RGBA32U).

use std::cmp::Ordering;

use crate::outline::GlyphOutlines;

pub const BAND_TEXTURE_WIDTH: u32 = 4096;
const MAX_BANDS: usize = 16;

pub struct BandData {
    pub curve_texels: Vec<[f32; 4]>,
    pub band_texels: Vec<[u32; 4]>,
    pub band_max: (u32, u32),
    pub bbox: (f32, f32, f32, f32),
}

pub fn process_bands(outlines: &GlyphOutlines) -> BandData {
    let curves = &outlines.curves;
    let (min_x, min_y, max_x, max_y) = outlines.bounding_box();
    let bbox = (min_x, min_y, max_x, max_y);

    let num_bands = (curves.len() / 4).max(1).min(MAX_BANDS);
    let band_w_x = if max_x > min_x { (max_x - min_x) / num_bands as f32 } else { 1.0 };
    let band_w_y = if max_y > min_y { (max_y - min_y) / num_bands as f32 } else { 1.0 };

    let mut h_bands: Vec<Vec<usize>> = vec![Vec::new(); num_bands];
    for (idx, c) in curves.iter().enumerate() {
        let (cy_min, cy_max) = (c.min_y(), c.max_y());
        for b in 0..num_bands {
            let lo = min_y + b as f32 * band_w_y;
            let hi = min_y + (b + 1) as f32 * band_w_y;
            if cy_max >= lo && cy_min <= hi {
                h_bands[b].push(idx);
            }
        }
    }
    for band in &mut h_bands {
        band.sort_by(|a, b| {
            curves[*b]
                .max_x()
                .partial_cmp(&curves[*a].max_x())
                .unwrap_or(Ordering::Equal)
        });
    }

    let mut v_bands: Vec<Vec<usize>> = vec![Vec::new(); num_bands];
    for (idx, c) in curves.iter().enumerate() {
        let (cx_min, cx_max) = (c.min_x(), c.max_x());
        for b in 0..num_bands {
            let lo = min_x + b as f32 * band_w_x;
            let hi = min_x + (b + 1) as f32 * band_w_x;
            if cx_max >= lo && cx_min <= hi {
                v_bands[b].push(idx);
            }
        }
    }
    for band in &mut v_bands {
        band.sort_by(|a, b| {
            curves[*b]
                .max_y()
                .partial_cmp(&curves[*a].max_y())
                .unwrap_or(Ordering::Equal)
        });
    }

    let mut curve_texels = Vec::with_capacity(curves.len() * 2);
    for c in curves {
        curve_texels.push([c.p1.0, c.p1.1, c.p2.0, c.p2.1]);
        curve_texels.push([c.p3.0, c.p3.1, 0.0, 0.0]);
    }

    let mut h_headers: Vec<(u32, u32)> = Vec::new();
    let mut v_headers: Vec<(u32, u32)> = Vec::new();
    let mut curve_locs: Vec<(u32, u32)> = Vec::new();
    let mut offset = 0u32;
    for band in &h_bands {
        h_headers.push((band.len() as u32, offset));
        for &ci in band {
            curve_locs.push(curve_texel_coord((ci * 2) as u32));
            offset += 1;
        }
    }
    for band in &v_bands {
        v_headers.push((band.len() as u32, offset));
        for &ci in band {
            curve_locs.push(curve_texel_coord((ci * 2) as u32));
            offset += 1;
        }
    }

    BandData {
        curve_texels,
        band_texels: build_band_texture(num_bands, &h_headers, &v_headers, &curve_locs),
        band_max: (
            (num_bands as u32).saturating_sub(1),
            (num_bands as u32).saturating_sub(1),
        ),
        bbox,
    }
}

fn curve_texel_coord(index: u32) -> (u32, u32) {
    (index % BAND_TEXTURE_WIDTH, index / BAND_TEXTURE_WIDTH)
}

fn build_band_texture(
    num_bands: usize,
    h: &[(u32, u32)],
    v: &[(u32, u32)],
    curve_locs: &[(u32, u32)],
) -> Vec<[u32; 4]> {
    let mut texels = Vec::new();
    let mut curve_offset = 0u32;
    let header_texels = (num_bands * 2) as u32;
    for (count, _) in h {
        let off = header_texels + curve_offset;
        curve_offset += *count;
        texels.push([*count, off, 0, 0]);
    }
    for _ in h.len()..num_bands {
        texels.push([0, header_texels + curve_offset, 0, 0]);
    }
    for (count, _) in v {
        let off = header_texels + curve_offset;
        curve_offset += *count;
        texels.push([*count, off, 0, 0]);
    }
    for _ in v.len()..num_bands {
        texels.push([0, header_texels + curve_offset, 0, 0]);
    }
    for (x, y) in curve_locs {
        texels.push([*x, *y, 0, 0]);
    }
    texels
}
