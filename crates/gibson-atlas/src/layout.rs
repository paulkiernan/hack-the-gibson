//! Shared panel geometry and raw R-channel drawing helpers.
//!
//! A panel is `PW x PH` pixels. Its visible structure is a set of non-overlapping `Block`
//! rectangles. The G/B/A planes are derived purely from the block rectangles (see
//! [`fill_planes`]), so two variants of the same panel share byte-identical planes whenever they
//! share the same block geometry. Glyph / bar / triangle content is drawn afterwards into the R
//! channel only.

use rand::Rng;
use rand::rngs::StdRng;

use gibson_types::{ATLAS_HEIGHT, ATLAS_WIDTH};

/// Layer pixel width.
pub const PW: usize = ATLAS_WIDTH as usize; // 256
/// Layer pixel height.
pub const PH: usize = ATLAS_HEIGHT as usize; // 768
/// Bytes in one 256 x 768 RGBA layer.
pub const LAYER_BYTES: usize = PW * PH * 4;
/// Margin on every side of a mosaic panel (px).
pub const MARGIN: i32 = 6;
/// Right edge for directory text (px); the last glyph column lands here.
pub const DIR_RIGHT: f32 = 236.0;

/// One opaque block rectangle (half-open: `[x0, x1) x [y0, y1)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    /// Block id in 1..=255, unique within the panel.
    pub id: u8,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Block {
    #[inline]
    pub fn w(&self) -> i32 {
        self.x1 - self.x0
    }
    #[inline]
    pub fn h(&self) -> i32 {
        self.y1 - self.y0
    }
    /// Number of 9 px text rows this block holds.
    #[inline]
    pub fn lines(&self) -> usize {
        (self.h() / 9) as usize
    }
}

/// Append a block allocating the next id.
pub fn push_block(out: &mut Vec<Block>, id: &mut u8, x0: i32, y0: i32, w: u32, h: u32) {
    out.push(Block {
        id: *id,
        x0,
        y0,
        x1: x0 + w as i32,
        y1: y0 + h as i32,
    });
    *id += 1;
}

/// Write the frozen G/B/A planes for every block into a (zeroed) layer and clear R inside each
/// block so content drawing starts from a known state.
///
/// G is the block-local row: 0 at the block's top row ..= 255 at its bottom row (rounded).
/// Pixels outside every block keep whatever the layer held (callers start from zeros, so those
/// stay A = 0, B = 0, G = 0).
pub fn fill_planes(layer: &mut [u8], blocks: &[Block]) {
    for b in blocks {
        let h = (b.y1 - b.y0) as u32;
        debug_assert!(h >= 2);
        let denom = h - 1;
        for y in b.y0..b.y1 {
            let g = (((y - b.y0) as u32 * 255) + denom / 2) / denom;
            let row = y as usize * PW;
            for x in b.x0..b.x1 {
                let i = (row + x as usize) * 4;
                layer[i] = 0; // R cleared; content drawn later
                layer[i + 1] = g as u8;
                layer[i + 2] = b.id;
                layer[i + 3] = 255;
            }
        }
    }
}

/// Fill every pixel of `b`'s rectangle in the R channel with `v`.
pub fn fill_r(layer: &mut [u8], b: &Block, v: u8) {
    for y in b.y0..b.y1 {
        let row = y as usize * PW;
        for x in b.x0..b.x1 {
            layer[(row + x as usize) * 4] = v;
        }
    }
}

/// Horizontal run of R at row `y` from `x0..x1`, clamped to the layer.
#[inline]
pub fn hline_r(layer: &mut [u8], x0: i32, x1: i32, y: i32, v: u8) {
    if y < 0 || y >= PH as i32 {
        return;
    }
    let cx0 = x0.max(0).min(PW as i32);
    let cx1 = x1.max(0).min(PW as i32);
    if cx1 <= cx0 {
        return;
    }
    let row = y as usize * PW;
    for x in cx0..cx1 {
        layer[(row + x as usize) * 4] = v;
    }
}

/// Vertical run of R at column `x` from `y0..y1`, clamped to the layer.
#[inline]
pub fn vline_r(layer: &mut [u8], x: i32, y0: i32, y1: i32, v: u8) {
    if x < 0 || x >= PW as i32 {
        return;
    }
    let cy0 = y0.max(0).min(PH as i32);
    let cy1 = y1.max(0).min(PH as i32);
    for y in cy0..cy1 {
        layer[(y as usize * PW + x as usize) * 4] = v;
    }
}

/// 1 px bright ring around a block (the "framed block" border), in R.
pub fn ring_r(layer: &mut [u8], b: &Block) {
    hline_r(layer, b.x0, b.x1, b.y0, 255);
    hline_r(layer, b.x0, b.x1, b.y1 - 1, 255);
    vline_r(layer, b.x0, b.y0, b.y1, 255);
    vline_r(layer, b.x1 - 1, b.y0, b.y1, 255);
}

/// Filled right-pointing triangle with a vertical left edge (`left, top` .. width `tw`, height
/// `th`), apex pointing right at the middle row. Drawn in R.
pub fn triangle_right_r(layer: &mut [u8], left: i32, top: i32, tw: i32, th: i32, v: u8) {
    debug_assert!(tw > 0 && th > 0);
    for r in 0..th {
        if th <= 1 {
            hline_r(layer, left, left + tw, top, v);
            continue;
        }
        let t = r as f32 / (th - 1) as f32;
        // Row extent = tw * 2 * min(t, 1 - t): flat vertical left edge, apex at the middle row.
        let xw = (tw as f32 * 2.0 * t.min(1.0 - t)).floor() as i32;
        hline_r(layer, left, left + xw + 1, top + r, v);
    }
}

/// A bar-chart block: solid 3 px-wide vertical bars of random height with a 1 px gap, bottom
/// aligned inside the block. Content only touches R.
pub fn bars_r(rng: &mut StdRng, layer: &mut [u8], b: &Block) {
    let h = b.h();
    if h < 8 {
        return;
    }
    let bottom = b.y1 - 2;
    let hi_max = ((h * 2) / 5).max(3).min(h - 3);
    let mut x = b.x0 + 2;
    while x + 3 <= b.x1 - 2 {
        let hi = rng.random_range(3..=hi_max);
        let top = bottom - hi + 1;
        for y in top..=bottom {
            let row = y as usize * PW;
            for dx in 0..3 {
                layer[(row + (x + dx) as usize) * 4] = 255;
            }
        }
        x += 5;
    }
}
