//! Mosaic text panels (0..28): stacked bands of dense mono text blocks.
//!
//! Layout (geometry): walk down the panel from the top margin placing bands; each band is a
//! full-width 244 px block (55%), two narrow blocks side by side (both <= 124 px wide, 35%), or a
//! lone narrower block (10%). Bands are separated by random vertical gaps of 4..=24 px; a band
//! stops when the next block would cross the bottom margin. Geometry never depends on text
//! content, so variant B reuses identical rectangles.
//!
//! Content (R only): each block is one of hex dumps (30%), numeric tables (20%), keyword rows
//! (20%), binary strings (10%), opcode columns (10%), bar charts (5%) or a framed keyword block
//! (5%). 10% of text blocks are inverse video (R = 255 minus glyph coverage).

use rand::Rng;
use rand::rngs::StdRng;

use crate::layout::{self, Block};
use crate::text::{Advance, GlyphCache, MONO, MONO_ADV, MONO_PX};

/// Mosaic content keywords (film-adjacent status/CPU vocabulary).
const KEYWORDS: [&str; 22] = [
    "STATUS", "ERROR", "OVERRIDE", "DUMPSEG", "IDLEPRO", "REPORT", "CONFIG", "QUE", "INIT",
    "LOAD", "SEC7", "CHK7", "NULL", "BASIC", "OUT", "MOV", "NOP", "JMP", "LD", "ACK", "SYN",
    "RST",
];

/// CPU opcode vocabulary for opcode columns.
const OPS: [&str; 16] = [
    "MOV", "NOP", "JMP", "LD", "ST", "OUT", "ADD", "SUB", "AND", "OR", "XOR", "SHL", "CALL",
    "RET", "PUSH", "POP",
];

const HEXU: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
];

fn hex_digit(rng: &mut StdRng) -> char {
    HEXU[rng.random_range(0..16)]
}

/// Deterministic band layout for one mosaic panel. Always yields >= 6 blocks (the layout caps
/// early band heights so at least six min-height rows always fit within the panel).
pub fn geometry(rng: &mut StdRng) -> Vec<Block> {
    let mut out = Vec::new();
    let mut id: u8 = 1;
    let mut y: i32 = layout::MARGIN;
    let max_y: i32 = layout::PH as i32 - layout::MARGIN; // exclusive bottom bound
    while y + 27 <= max_y {
        let lines_avail = ((max_y - y) / 9).min(16) as i32;
        // Until six blocks exist, keep each band compact enough that six always fit.
        let need_cap = (out.len() as u32) < 6;
        let line_cap = if need_cap { 12 } else { 16 };

        // Band composition: mostly full-width rows and two-up narrow pairs (the film mosaic reads
        // as a densely covered face); fewer lone narrow blocks keep the right edge ragged.
        let comp = rng.random_range(0..100);
        let mut xs = Vec::with_capacity(2);
        let mut ws = Vec::with_capacity(2);
        let mut hs = Vec::with_capacity(2);
        if comp >= 55 && comp < 90 {
            // Two narrow blocks share the band when both widths are <= 124 px.
            let a: i32 = if rng.random_range(0..100) < 50 { 64 } else { 96 };
            let b: i32 = if rng.random_range(0..100) < 50 { 64 } else { 96 };
            let gap = rng.random_range(6..=14);
            let la = rng.random_range(3..=lines_avail.min(line_cap));
            let lb = rng.random_range(3..=lines_avail.min(line_cap));
            xs.push(layout::MARGIN);
            xs.push(layout::MARGIN + a + gap);
            ws.push(a);
            ws.push(b);
            hs.push(la * 9);
            hs.push(lb * 9);
        } else {
            // Single block: a full-width row (55%) or a narrower ragged row (10%).
            let r = if comp < 55 {
                244
            } else {
                match rng.random_range(0..100) {
                    0..=44 => 160,
                    45..=69 => 128,
                    70..=89 => 96,
                    _ => 64,
                }
            };
            let lines = rng.random_range(3..=lines_avail.min(line_cap));
            xs.push(layout::MARGIN);
            ws.push(r);
            hs.push(lines * 9);
        }
        let band_h = *hs.iter().max().unwrap();
        if y + band_h > max_y {
            break;
        }
        for i in 0..xs.len() {
            layout::push_block(&mut out, &mut id, xs[i], y, ws[i] as u32, hs[i] as u32);
        }
        let vgap = if need_cap {
            rng.random_range(4..=(112 - band_h).clamp(4, 24))
        } else {
            rng.random_range(4..=24)
        };
        y += band_h + vgap;
    }
    out
}

// --- Row text builders -----------------------------------------------------
// Each builder emits one row of dense tiny characters of roughly `cols` columns (mono grid), with
// enough raggedness to look like live terminal output.

/// Hex dump row: groups of four hex nibbles or byte pairs, e.g. `8F3A 00C1 2B4D`.
pub(crate) fn hex_row(rng: &mut StdRng, cols: usize) -> String {
    let cols = cols.max(1);
    let target = rng.random_range((cols / 2).max(1)..=cols);
    let mut s = String::with_capacity(target + 8);
    while s.len() < target {
        if rng.random_range(0..100) < 60 {
            for _ in 0..4 {
                s.push(hex_digit(rng));
            }
        } else {
            s.push(hex_digit(rng));
            s.push(hex_digit(rng));
        }
        if s.len() < target && rng.random_range(0..100) < 85 {
            s.push(' ');
        }
    }
    s.truncate(target);
    s
}

/// Numeric table row: mostly digits with hex letters and column spaces, e.g. `35563 1286 20fx89c`.
fn numeric_row(rng: &mut StdRng, cols: usize) -> String {
    let cols = cols.max(1);
    let target = rng.random_range((cols * 3 / 4).max(1)..=cols);
    let mut s = String::with_capacity(target);
    while s.len() < target {
        let r = rng.random_range(0..100);
        let c = if r < 68 {
            b'0' + rng.random_range(0..10)
        } else if r < 86 {
            b'a' + rng.random_range(0..6)
        } else {
            b' '
        };
        s.push(c as char);
    }
    s.truncate(target);
    s
}

/// Keyword row: an uppercase status word plus a hex tail, e.g. `STATUS 9F2A C1`.
fn keyword_row(rng: &mut StdRng, cols: usize) -> String {
    let cols = cols.max(1);
    let word = KEYWORDS[rng.random_range(0..KEYWORDS.len())];
    let target = rng.random_range((cols / 2).max(word.len().min(cols))..=cols);
    let mut s = String::from(word);
    if s.len() < target && rng.random_range(0..100) < 40 {
        s.push(':');
    }
    let tail = target.saturating_sub(s.len());
    for _ in 0..tail {
        let r = rng.random_range(0..100);
        let c = if r < 20 {
            ' '
        } else if r < 60 {
            hex_digit(rng)
        } else {
            (b'0' + rng.random_range(0..10) as u8) as char
        };
        s.push(c);
    }
    s.truncate(target);
    s
}

/// Binary string row: 0/1 with o/x markers, e.g. `0000ox0110x`.
fn binary_row(rng: &mut StdRng, cols: usize) -> String {
    let cols = cols.max(1);
    let target = rng.random_range((cols * 3 / 4).max(1)..=cols);
    const BITS: [char; 4] = ['0', '0', '1', '1'];
    let mut s = String::with_capacity(target);
    while s.len() < target {
        let r = rng.random_range(0..100);
        let c = if r < 10 {
            ' '
        } else if r < 75 {
            BITS[rng.random_range(0..4)]
        } else if r < 90 {
            'o'
        } else {
            'x'
        };
        s.push(c);
    }
    s.truncate(target);
    s
}

/// Opcode column row: one or more `OP operand` pairs, e.g. `MOV A,#2F JMP 0x00C1`.
fn opcode_row(rng: &mut StdRng, cols: usize) -> String {
    let cols = cols.max(1);
    let target = rng.random_range((cols / 2).max(1)..=cols);
    let mut s = String::new();
    while s.len() < target {
        let op = OPS[rng.random_range(0..OPS.len())];
        let operand = match rng.random_range(0..3) {
            0 => format!("{:02X}", rng.random_range(0..=255)),
            1 => format!("0x{:04X}", rng.random_range(0..=0xFFFF)),
            _ => format!("A,#{:02X}", rng.random_range(0..=255)),
        };
        let tok = format!("{} {}", op, operand);
        s.push_str(&tok);
        if s.len() < target {
            s.push(' ');
        }
    }
    s.truncate(target);
    s
}

/// Draw `lines` mono text rows across a block using `builder` for each row's content.
fn draw_rows<F>(
    rng: &mut StdRng,
    glyphs: &mut GlyphCache,
    layer: &mut [u8],
    b: &Block,
    asc: f32,
    inverse: bool,
    mut builder: F,
) where
    F: FnMut(&mut StdRng, usize) -> String,
{
    let cols = ((b.w() - 2) / 6).max(1) as usize;
    let x0 = (b.x0 + 1) as f32;
    for r in 0..b.lines() {
        let s = builder(rng, cols);
        if s.is_empty() {
            continue;
        }
        let baseline = (b.y0 + (r as i32) * 9) as f32 + asc;
        glyphs.draw_row(
            layer,
            MONO,
            MONO_PX,
            &s,
            x0,
            baseline,
            Advance::Grid(MONO_ADV),
            b.x0,
            b.y0,
            b.x1,
            b.y1,
            inverse,
        );
    }
}

/// Fill one mosaic panel's R channel (geometry must already be in the planes).
pub fn render(rng: &mut StdRng, layer: &mut [u8], blocks: &[Block], glyphs: &mut GlyphCache) {
    let (asc, _desc) = glyphs.line_metrics(MONO, MONO_PX);
    for b in blocks {
        let roll = rng.random_range(0..100);
        if roll < 5 {
            layout::bars_r(rng, layer, b);
            continue;
        }
        let framed = roll >= 95;
        let inverse = !framed && rng.random_range(0..100) < 10;
        if framed {
            layout::ring_r(layer, b);
        } else if inverse {
            layout::fill_r(layer, b, 255);
        }
        match roll {
            5..=34 => draw_rows(rng, glyphs, layer, b, asc, inverse, hex_row),
            35..=54 => draw_rows(rng, glyphs, layer, b, asc, inverse, numeric_row),
            55..=74 => draw_rows(rng, glyphs, layer, b, asc, inverse, keyword_row),
            75..=84 => draw_rows(rng, glyphs, layer, b, asc, inverse, binary_row),
            85..=94 => draw_rows(rng, glyphs, layer, b, asc, inverse, opcode_row),
            _ => {
                // Framed keyword block: keyword header row over hex body.
                let cols = ((b.w() - 2) / 6).max(1) as usize;
                let x0 = (b.x0 + 1) as f32;
                // Header row.
                let word = KEYWORDS[rng.random_range(0..KEYWORDS.len())];
                let mut header = String::from(word);
                while header.len() < cols {
                    header.push(' ');
                }
                header.truncate(cols);
                if b.lines() > 0 {
                    let baseline = b.y0 as f32 + asc;
                    glyphs.draw_row(
                        layer,
                        MONO,
                        MONO_PX,
                        &header,
                        x0,
                        baseline,
                        Advance::Grid(MONO_ADV),
                        b.x0,
                        b.y0,
                        b.x1,
                        b.y1,
                        false,
                    );
                }
                // Hex body rows underneath.
                for r in 1..b.lines() {
                    let s = hex_row(rng, cols);
                    if s.is_empty() {
                        continue;
                    }
                    let baseline = (b.y0 + (r as i32) * 9) as f32 + asc;
                    glyphs.draw_row(
                        layer,
                        MONO,
                        MONO_PX,
                        &s,
                        x0,
                        baseline,
                        Advance::Grid(MONO_ADV),
                        b.x0,
                        b.y0,
                        b.x1,
                        b.y1,
                        false,
                    );
                }
            }
        }
    }
}
