//! Hero directory panels (28..32): right-aligned Michroma lists with triangle markers.
//!
//! Each directory entry is its own block (26 px tall, full panel width) so a highlight can light
//! exactly one line. Text is right-aligned so the last column lands at x = 236 and is followed by
//! a filled right-pointing triangle. Below the list sits one small hex-dump block. Variant B keeps
//! the same rectangles but re-samples the entry names.

use rand::Rng;
use rand::rngs::StdRng;

use crate::layout::{self, Block};
use crate::text::{Advance, GlyphCache, MICHROMA, MONO, MONO_ADV, MONO_PX};
use crate::mosaic;

/// The film's on-screen directory names (keeping the production's misspelling "FORCASTS").
const NAMES: [&str; 28] = [
    "SHIPPING FORCASTS",
    "ACCOUNTANTS",
    "SHIPPING ROUTINGS",
    "TPGC. REPORTS",
    "WRHSE. EXPEND.",
    "GARBAGE",
    "ANNUAL BUDGETS",
    "COMP. OPERATIONS",
    "KINEMATICS",
    "SEA-BOARD LAWS",
    "TPGC. EXPEND.",
    "WRHSE. LOCATION",
    "COMPANY STATUS",
    "COMPOSITE PLANTS",
    "PAYROLL",
    "PERSONNEL",
    "TANKER ROUTES",
    "OIL PLATFORMS",
    "BALLAST CTRL",
    "HULL STRESS",
    "CARGO MANIFEST",
    "LEGAL",
    "SECURITY",
    "SYS ADMIN",
    "BACKUP",
    ".WORKSPACE",
    ".GARBAGE",
    "ROOT",
];

/// Panel width available to a list row (margin 6 on each side of 256).
const ROW_W: u32 = 244;
/// Row height in px.
const ROW_H: i32 = 26;
/// Longest text width allowed before the auto-fit shrinks the face size (leaves room for the
/// triangle and keeps the left edge past the 6 px margin).
const FIT_W: f32 = 226.0;
/// Triangle geometry: gap after the text, width, height.
const TRI_GAP: i32 = 3;
const TRI_W: i32 = 8;
const TRI_H: i32 = 6;

/// Deterministic geometry for one directory panel: `14..=18` entry rows plus one hex block.
pub fn geometry(rng: &mut StdRng) -> Vec<Block> {
    let n = rng.random_range(14..=18);
    let top = rng.random_range(16..=36);
    let mut out = Vec::with_capacity(n as usize + 1);
    let mut id: u8 = 1;
    for r in 0..n {
        layout::push_block(&mut out, &mut id, layout::MARGIN, top + r * ROW_H, ROW_W, ROW_H as u32);
    }
    let h_lines = rng.random_range(4..=6);
    let hw: u32 = if rng.random_range(0..100) < 50 { 96 } else { 128 };
    let gy = top + n * ROW_H + rng.random_range(10..=22);
    layout::push_block(&mut out, &mut id, layout::MARGIN, gy, hw, (h_lines * 9) as u32);
    out
}

/// Sample `n` distinct names (partial Fisher-Yates), deterministically.
fn sample_names(rng: &mut StdRng, n: usize) -> Vec<&'static str> {
    let mut v: Vec<&str> = NAMES.to_vec();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let k = rng.random_range(i..v.len());
        v.swap(i, k);
        out.push(v[i]);
    }
    out
}

/// Fill one directory panel's R channel (planes already written from the geometry).
pub fn render(rng: &mut StdRng, layer: &mut [u8], blocks: &[Block], glyphs: &mut GlyphCache) {
    let entries = blocks.len() - 1;
    let names = sample_names(rng, entries);
    // One face size for the whole list (film lists are uniform). Long names shrink the size so the
    // widest sampled name still right-aligns before x = 236; short panels stay at 18 px.
    let mut wmax: f32 = 0.0;
    for name in &names {
        wmax = wmax.max(glyphs.width(MICHROMA, crate::text::DIR_PX, name));
    }
    let px = if wmax <= FIT_W {
        crate::text::DIR_PX
    } else {
        (crate::text::DIR_PX * FIT_W / wmax).max(12.0)
    };
    let (asc, desc) = glyphs.line_metrics(MICHROMA, px);
    for (i, b) in blocks[..entries].iter().enumerate() {
        let name = names[i];
        let wpx = glyphs.width(MICHROMA, px, name);
        let x0 = layout::DIR_RIGHT - wpx;
        // Baseline that centers the em box in the 26 px row.
        let baseline = b.y0 as f32 + (ROW_H as f32) * 0.5 + (asc + desc) * 0.5;
        glyphs.draw_row(
            layer,
            MICHROMA,
            px,
            name,
            x0,
            baseline,
            Advance::Real,
            b.x0,
            b.y0,
            b.x1,
            b.y1,
            false,
        );
        // Triangle after the text, vertically centered on the row.
        let tri_left = layout::DIR_RIGHT.round() as i32 + TRI_GAP;
        let tri_top = b.y0 + (ROW_H - TRI_H) / 2;
        layout::triangle_right_r(layer, tri_left, tri_top, TRI_W, TRI_H, 255);
    }

    // Small hex-dump block below the list (its own id, mono mosaic body).
    let hex = &blocks[entries];
    let (asc, _desc) = glyphs.line_metrics(MONO, MONO_PX);
    let cols = ((hex.w() - 2) / 6).max(1) as usize;
    let x0 = (hex.x0 + 1) as f32;
    for r in 0..hex.lines() {
        let s = mosaic::hex_row(rng, cols);
        if s.is_empty() {
            continue;
        }
        let baseline = (hex.y0 + (r as i32) * 9) as f32 + asc;
        glyphs.draw_row(
            layer,
            MONO,
            MONO_PX,
            &s,
            x0,
            baseline,
            Advance::Grid(MONO_ADV),
            hex.x0,
            hex.y0,
            hex.x1,
            hex.y1,
            false,
        );
    }
}
