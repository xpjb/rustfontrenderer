//! Embedded MSDF atlas (`atlas.bin` + `atlas.png`) produced by `build.rs` into `OUT_DIR`.

use crate::atlas_format::{AtlasFile, AtlasHeader, GlyphRecord, ATLAS_FORMAT_VERSION, ATLAS_MAGIC};
use png::{ColorType, Decoder, Transformations};

const ATLAS_PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/atlas.png"));
const ATLAS_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/atlas.bin"));

pub use crate::font::FontMetrics;

/// Decoded atlas payload + RGBA8 texture bytes (`width * height * 4`).
pub struct EmbeddedAtlas {
    pub file: AtlasFile,
    pub rgba: Vec<u8>,
}

impl EmbeddedAtlas {
    pub fn header(&self) -> &AtlasHeader {
        &self.file.header
    }

    pub fn glyph_table(&self) -> std::collections::HashMap<u32, GlyphRecord> {
        self.file
            .glyphs
            .iter()
            .copied()
            .map(|g| (g.glyph_id, g))
            .collect()
    }

    pub fn distance_range_px(&self) -> f32 {
        self.file.header.distance_range_px
    }
}

pub fn load_embedded_atlas() -> Result<EmbeddedAtlas, String> {
    let file: AtlasFile =
        bincode::deserialize(ATLAS_BIN).map_err(|e| format!("atlas.bin: {e}"))?;
    if file.header.magic != *ATLAS_MAGIC {
        return Err("atlas.bin: bad magic".into());
    }
    if file.header.version != ATLAS_FORMAT_VERSION {
        return Err(format!(
            "atlas.bin: unsupported format version {} (expected {})",
            file.header.version, ATLAS_FORMAT_VERSION
        ));
    }
    let cursor = std::io::Cursor::new(ATLAS_PNG);
    let mut decoder = Decoder::new(cursor);
    // MSDF values must stay linear encoded raw bytes. The png crate does not gamma-decode by default,
    // but forbid ICC-driven tooling quirks and expansion transforms explicitly.
    decoder.set_transformations(Transformations::IDENTITY);
    decoder.set_ignore_iccp_chunk(true);
    let mut reader = decoder.read_info().map_err(|e| format!("atlas.png: {e}"))?;
    let mw = reader.info().width;
    let mh = reader.info().height;
    let color_type = reader.info().color_type;
    if mw != file.header.atlas_w || mh != file.header.atlas_h {
        return Err(format!(
            "atlas.png size {}x{} != header {}x{}",
            mw, mh, file.header.atlas_w, file.header.atlas_h
        ));
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    reader
        .next_frame(&mut buf)
        .map_err(|e| format!("atlas.png frame: {e}"))?;
    let rgba = match color_type {
        ColorType::Rgb => {
            let mut out = Vec::with_capacity(buf.len() / 3 * 4);
            for px in buf.chunks_exact(3) {
                out.extend_from_slice(px);
                out.push(255);
            }
            out
        }
        ColorType::Rgba => buf,
        ct => return Err(format!("atlas.png unsupported color type {ct:?}")),
    };
    Ok(EmbeddedAtlas { file, rgba })
}
