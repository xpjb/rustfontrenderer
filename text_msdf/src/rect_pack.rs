//! Simple shelf-first rectangle packer (fast, adequate for MSDF atlases and similar texture bins).

/// `(x, y)` of the packed rectangle's top-left in atlas texels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackedRect {
    pub x: u32,
    pub y: u32,
}

/// Shelf packer with a fixed maximum row width (`max_width`).
#[derive(Clone, Debug)]
pub struct ShelfPacker {
    max_width: u32,
    /// Rows start at this `x` (e.g. 1 leaves column 0 for a sentinel).
    cursor_x_floor: u32,
    shelf_y: u32,
    shelf_h: u32,
    cursor_x: u32,
    used_w: u32,
    used_h: u32,
}

#[allow(dead_code)] // `build.rs` `#[path]`s this file into the build-script crate, which does not reference every helper.
impl ShelfPacker {
    pub fn new(max_width: u32) -> Self {
        Self::with_x_margin(max_width, 0)
    }

    /// `x_margin`: leave `[0 .. x_margin)` empty on every shelf row (e.g. a 1-pixel MSDF sentinel column).
    pub fn with_x_margin(max_width: u32, x_margin: u32) -> Self {
        Self {
            max_width,
            cursor_x_floor: x_margin,
            shelf_y: 0,
            shelf_h: 0,
            cursor_x: x_margin,
            used_w: x_margin,
            used_h: if x_margin > 0 { 1 } else { 0 },
        }
    }

    /// Pack `w × h`; returns [`None`] if `w > max_width` or `cursor_x_floor + w > max_width`.
    pub fn pack(&mut self, w: u32, h: u32) -> Option<PackedRect> {
        if w == 0 || h == 0 {
            return None;
        }
        if self.cursor_x_floor + w > self.max_width {
            return None;
        }
        if self.cursor_x + w > self.max_width {
            self.shelf_y += self.shelf_h;
            self.cursor_x = self.cursor_x_floor;
            self.shelf_h = 0;
        }
        let pos = PackedRect {
            x: self.cursor_x,
            y: self.shelf_y,
        };
        self.cursor_x += w;
        self.shelf_h = self.shelf_h.max(h);
        self.used_w = self.used_w.max(self.cursor_x);
        self.used_h = self.used_h.max(self.shelf_y + h);
        Some(pos)
    }

    /// Smallest axis-aligned box containing every successful [`Self::pack`] result.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.used_w.max(1), self.used_h.max(1))
    }

    pub fn max_width(&self) -> u32 {
        self.max_width
    }
}
