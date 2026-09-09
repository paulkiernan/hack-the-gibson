//! Mosaic text panels (0..28): film-style column-strip mosaics.
//!
//! Layout (geometry): the film faces read as **vertical column strips** — tall narrow columns
//! of dense mono text sitting side by side, separated by continuous dark gutters, with a few
//! full-width anchor bands (chunky inverse-video slabs, framed panes, labeled rules) that
//! interrupt the columns. A panel therefore fixes a 2- or 3-track column plan for its whole
//! height, drops 1..=3 full-width anchor bands at staggered heights, and then stacks text
//! segments down each column track inside the remaining vertical spans. Segment heights vary
//! per track so column bottoms stay ragged like real stacked block groups. The bottom-most span
//! of the first narrow track always starts with one tall segment, so every panel carries at
//! least one genuinely tall-narrow column block. Geometry never depends on text content, so
//! variant B reuses identical rectangles.
//!
//! Content (R only): blocks are drawn as one of
//!   - inverse-video slabs: bright fill with knocked-out dark text (chunky anchors; the
//!     top-most full-width anchor of every panel is forced to this so each face shows one),
//!   - framed panes: 1 px ring, bright header row, body split into small sub-columns by thin
//!     vertical dividers (the film's framed boxes grouping smaller sub-blocks),
//!   - small bar charts with a bright baseline,
//!   - labeled rules / thin horizontal separators,
//!   - dense mono text rows (hex dumps, numeric tables, keyword lines, binary strings, opcode
//!     columns) with tightly packed 8 px leading and occasional mid-block horizontal rules.
//!
//! The text face runs at `MONO_PX` on an 8 px pitch (see [`layout::MONO_PITCH`]): line spacing
//! is tight and rows fill their full width, matching the film's very dense, fine text.

use rand::Rng;
use rand::rngs::StdRng;

use crate::layout::{self, Block, MONO_PITCH};
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

/// A few file-ish names occasionally used as mini headers on text blocks.
const HEADERS: [&str; 10] = [
    "SYS.LOG", "CORE.DMP", "MAIN.CFG", "SEC.LST", "IDLE.PRO", "NET.STAT", "MEM.MAP",
    "USR.DIR", "IO.PORT", "ROM.DMP",
];

const HEXU: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
];

fn hex_digit(rng: &mut StdRng) -> char {
    HEXU[rng.random_range(0..16)]
}

// --- Geometry ---------------------------------------------------------------

/// Minimum lines in any stacked text segment (3 rows * 8 px = 24 px).
const MIN_LINES: i32 = 3;
/// Ordinary segments are capped here so columns segment into several stacked block groups
/// (the film faces show distinct stacked groups rather than one monolithic column).
const SEG_MAX_LINES: i32 = 20;
/// Forced tall-narrow bottom segment (the "one tall narrow column" per panel).
const TALL_MIN_LINES: i32 = 26;
const TALL_MAX_LINES: i32 = 34;
/// Room (px) that must stay below the last anchor band so the bottom column span is tall
/// enough for the forced tall segment.
const MIN_BOTTOM_SPAN: i32 = 220;

/// Deterministic column-strip layout for one mosaic panel. Always yields many blocks (>= 10),
/// with at least one full-width anchor block (blocks[0] is the top anchor) and at least one
/// tall-narrow block.
pub fn geometry(rng: &mut StdRng) -> Vec<Block> {
    let mut anchors: Vec<Block> = Vec::new();
    let mut id: u8 = 1;
    let x0 = layout::MARGIN; // 6
    let x1 = layout::PW as i32 - layout::MARGIN; // 250 (exclusive)
    let inner_w = x1 - x0; // 244
    let y0 = layout::MARGIN; // 6
    let y1 = layout::PH as i32 - layout::MARGIN; // 762 (exclusive)

    // Fixed column tracks for the whole panel: either a narrow + wide pair (film mixes one wide
    // column with narrow neighbors) or three narrow tracks. Tracks are separated by continuous
    // 8..=10 px dark gutters, the film's rhythmic vertical division.
    let col_gap = rng.random_range(8..=10);
    let mut cols: Vec<(i32, i32)> = Vec::with_capacity(3); // (x0, width)
    if rng.random_range(0..100) < 50 {
        let narrow = if rng.random_range(0..100) < 55 { 64 } else { 96 };
        let wide = inner_w - col_gap - narrow;
        if rng.random_range(0..100) < 50 {
            cols.push((x0, narrow));
            cols.push((x0 + narrow + col_gap, wide));
        } else {
            cols.push((x0, wide));
            cols.push((x0 + wide + col_gap, narrow));
        }
    } else {
        let widths: [i32; 3] = match rng.random_range(0..3) {
            0 => [64, 64, 96],
            1 => [64, 96, 64],
            _ => [96, 64, 64],
        };
        let mut cx = x0;
        for w in widths {
            cols.push((cx, w));
            cx += w + col_gap;
        }
    }

    // Full-width anchor bands, top to bottom, each leaving >= MIN_BOTTOM_SPAN below it. They are
    // pushed first so blocks[0] is always the top anchor (the renderer forces it to an
    // inverse-video slab).
    let n_anchor = 1 + i32::from(rng.random_range(0..100) < 40) + i32::from(rng.random_range(0..100) < 7);
    let mut ay = y0 + rng.random_range(20..=64);
    for _ in 0..n_anchor {
        let lines = rng.random_range(4..=10);
        let h = lines * MONO_PITCH;
        if ay + h + MIN_BOTTOM_SPAN > y1 {
            break;
        }
        let w = if rng.random_range(0..100) < 25 { 236 } else { 244 };
        layout::push_block(&mut anchors, &mut id, x0, ay, w as u32, h as u32);
        ay += h + rng.random_range(28..=64);
    }

    // The anchors split the panel height into free vertical spans; every track fills each span
    // with its own stacked segments (tracks are independent, so bottoms stay ragged).
    let mut spans: Vec<(i32, i32)> = Vec::with_capacity(anchors.len() + 1);
    let mut sy = y0;
    for a in &anchors {
        spans.push((sy, a.y0));
        sy = a.y1;
    }
    spans.push((sy, y1));

    let mut out = anchors;
    let first_narrow = cols.iter().position(|&(_, w)| w <= 96);
    for (ci, &(cx, cw)) in cols.iter().enumerate() {
        let n_spans = spans.len();
        for (si, &(sa, sb)) in spans.iter().enumerate() {
            // Tall narrow guarantee: the first narrow track's bottom-most span opens with one
            // segment of >= TALL_MIN_LINES rows.
            let force_tall =
                si + 1 == n_spans && first_narrow == Some(ci) && sb - sa >= TALL_MIN_LINES * MONO_PITCH;
            fill_span(rng, &mut out, &mut id, cx, cw, sa, sb, force_tall);
        }
    }
    out
}

/// Stack mono-text segments down one track inside `[sa, sb)`; the final segment of each span
/// flushes crisply to the span end (column groups terminate with small dark pockets that differ
/// per track, so neighboring columns end ragged like the film's broken block edges).
fn fill_span(
    rng: &mut StdRng,
    out: &mut Vec<Block>,
    id: &mut u8,
    cx: i32,
    cw: i32,
    sa: i32,
    sb: i32,
    force_tall: bool,
) {
    // Leave a small dark pocket before the next anchor/panel edge; the guaranteed tall segment
    // must not be shortened, so its span stays flush.
    let end = if force_tall { sb } else { sb - rng.random_range(0..=14) };
    let mut y = sa;
    let mut first = true;
    while end - y >= MIN_LINES * MONO_PITCH {
        let avail = (end - y) / MONO_PITCH;
        let lines = if force_tall && first && avail >= TALL_MIN_LINES {
            avail.min(TALL_MIN_LINES + rng.random_range(0..=(TALL_MAX_LINES - TALL_MIN_LINES)))
        } else if avail <= SEG_MAX_LINES {
            avail
        } else {
            rng.random_range(MIN_LINES..=SEG_MAX_LINES)
        };
        let h = lines * MONO_PITCH;
        layout::push_block(out, id, cx, y, cw as u32, h as u32);
        y += h;
        if end - y < MIN_LINES * MONO_PITCH {
            break;
        }
        y += rng.random_range(5..=11); // dark gutter between stacked block groups
        first = false;
    }
}

// --- Row text builders ------------------------------------------------------
// Each builder emits one row of dense tiny characters filling (almost) the full `cols` column
// width of its block, with just enough raggedness to look like live terminal output.

/// Hex dump row, dense: runs of 4- or 2-digit hex groups with tight spacing, e.g.
/// `8F3A 00C1 2B4D`; wide rows sometimes start with an address prefix `00C1:`.
pub(crate) fn hex_row(rng: &mut StdRng, cols: usize) -> String {
    let cols = cols.max(1);
    let mut s = String::with_capacity(cols + 8);
    let mut first = true;
    if cols >= 14 && rng.random_range(0..100) < 30 {
        for _ in 0..4 {
            s.push(hex_digit(rng));
        }
        s.push(':');
        first = false;
    }
    while s.len() < cols {
        if !first && rng.random_range(0..100) < 92 {
            s.push(' ');
        }
        first = false;
        let rem = cols - s.len();
        let g = if rem >= 4 && rng.random_range(0..100) < 55 {
            4
        } else if rem >= 2 {
            2
        } else {
            1
        };
        for _ in 0..g.min(rem) {
            s.push(hex_digit(rng));
        }
    }
    s.truncate(cols);
    s
}

/// Numeric table row: mostly digits with hex letters, e.g. `35563 1286 20fx89c`. Rows end at
/// variable lengths (ragged right edge like live terminal output).
fn numeric_row(rng: &mut StdRng, cols: usize) -> String {
    let cols = cols.max(1);
    let target = cols.saturating_sub(rng.random_range(0..=(cols / 5).max(1).min(8)));
    let mut s = String::with_capacity(target + 4);
    while s.len() < target {
        let r = rng.random_range(0..100);
        let c = if r < 72 {
            b'0' + rng.random_range(0..10)
        } else if r < 94 {
            b'a' + rng.random_range(0..6)
        } else {
            b' '
        };
        s.push(c as char);
    }
    s.truncate(target);
    s
}

/// Keyword row: an uppercase status word plus a dense hex/digit tail, e.g. `STATUS 9F2A C1`.
fn keyword_row(rng: &mut StdRng, cols: usize) -> String {
    let cols = cols.max(1);
    let word = KEYWORDS[rng.random_range(0..KEYWORDS.len())];
    let mut s = String::from(word);
    if s.len() < cols && rng.random_range(0..100) < 45 {
        s.push(':');
    }
    while s.len() < cols {
        let r = rng.random_range(0..100);
        let c = if r < 12 {
            ' '
        } else if r < 62 {
            hex_digit(rng)
        } else {
            (b'0' + rng.random_range(0..10) as u8) as char
        };
        s.push(c);
    }
    s.truncate(cols);
    s
}

/// Binary string row: dense 0/1 with a few o/x markers, e.g. `0000ox0110x`. Variable length.
fn binary_row(rng: &mut StdRng, cols: usize) -> String {
    let cols = cols.max(1);
    let target = cols.saturating_sub(rng.random_range(0..=(cols / 6).max(1).min(6)));
    let mut s = String::with_capacity(target + 2);
    while s.len() < target {
        let r = rng.random_range(0..100);
        let c = if r < 6 {
            ' '
        } else if r < 86 {
            if rng.random_range(0..2) == 0 { '0' } else { '1' }
        } else if r < 93 {
            'o'
        } else {
            'x'
        };
        s.push(c);
    }
    s.truncate(target);
    s
}

/// Opcode column row: `OP operand` pairs, e.g. `MOV A,#2F JMP 0x00C1`.
fn opcode_row(rng: &mut StdRng, cols: usize) -> String {
    let cols = cols.max(1);
    let mut s = String::new();
    while s.len() < cols {
        let op = OPS[rng.random_range(0..OPS.len())];
        let operand = match rng.random_range(0..3) {
            0 => format!("{:02X}", rng.random_range(0..=255)),
            1 => format!("0x{:04X}", rng.random_range(0..=0xFFFF)),
            _ => format!("A,#{:02X}", rng.random_range(0..=255)),
        };
        let tok = format!("{} {}", op, operand);
        if s.len() + tok.len() >= cols + 2 {
            break; // keep the tail ragged instead of a clipped half-token
        }
        s.push_str(&tok);
        if s.len() < cols {
            s.push(' ');
        }
    }
    s.truncate(cols);
    s
}

/// Column count and left pen offset for mono rows inside block `b`.
fn cols_of(b: &Block) -> (usize, f32) {
    let cols = ((b.w() - 2) / 6).max(1) as usize;
    let x0 = (b.x0 + 1) as f32;
    (cols, x0)
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
    let (cols, x0) = cols_of(b);
    for r in 0..b.lines() {
        let s = builder(rng, cols);
        if s.is_empty() {
            continue;
        }
        let baseline = (b.y0 + (r as i32) * MONO_PITCH) as f32 + asc;
        glyphs.draw_row(
            layer, MONO, MONO_PX, &s, x0, baseline, Advance::Grid(MONO_ADV), b.x0, b.y0, b.x1,
            b.y1, inverse,
        );
    }
}

/// Draw a bright 1 px horizontal rule inside block `b` in the dark gutter between text rows
/// `k - 1` and `k` (so it never slices glyphs), spanning the block's full width.
fn rule_between_rows(layer: &mut [u8], b: &Block, k: i32, v: u8) {
    let y = b.y0 + k * MONO_PITCH;
    layout::hline_r(layer, b.x0 + 1, b.x1 - 1, y, v);
}

// --- Content kinds ----------------------------------------------------------

/// Inverse-video slab: chunky bright rectangle (R = 255) with knocked-out dark text rows.
/// Most slabs carry rows (keyword / hex / numeric) so they read as bright data panels rather
/// than featureless voids; a minority are pure bright fills for solid visual anchors.
fn draw_slab(rng: &mut StdRng, glyphs: &mut GlyphCache, layer: &mut [u8], b: &Block, asc: f32) {
    layout::fill_r(layer, b, 255);
    if b.lines() < 2 || rng.random_range(0..100) < 10 {
        return; // solid slab
    }
    match rng.random_range(0..100) {
        0..=44 => draw_rows(rng, glyphs, layer, b, asc, true, keyword_row),
        45..=79 => draw_rows(rng, glyphs, layer, b, asc, true, hex_row),
        _ => draw_rows(rng, glyphs, layer, b, asc, true, numeric_row),
    }
}

/// Framed pane: 1 px ring, one bright keyword header row, and a body split into small
/// sub-columns by thin vertical dividers — the film's framed boxes grouping smaller sub-blocks.
fn draw_framed_pane(rng: &mut StdRng, glyphs: &mut GlyphCache, layer: &mut [u8], b: &Block, asc: f32) {
    layout::ring_r(layer, b);
    let lines = b.lines();
    if lines == 0 {
        return;
    }
    let (cols, x0) = cols_of(b);
    // Header row: a keyword padded to the full width, e.g. `REPORT :....`.
    let word = KEYWORDS[rng.random_range(0..KEYWORDS.len())];
    let mut header = String::with_capacity(cols);
    header.push_str(word);
    if header.len() < cols && rng.random_range(0..100) < 60 {
        header.push(':');
    }
    while header.len() < cols {
        if rng.random_range(0..100) < 40 {
            header.push(' ');
        } else if rng.random_range(0..2) == 0 {
            header.push('.');
        } else {
            header.push(' ');
        }
    }
    header.truncate(cols);
    let baseline = b.y0 as f32 + asc;
    glyphs.draw_row(
        layer, MONO, MONO_PX, &header, x0, baseline, Advance::Grid(MONO_ADV), b.x0, b.y0, b.x1,
        b.y1, false,
    );
    if lines < 2 {
        return;
    }
    // Bright rule directly under the header (gutter between header row and first body row).
    rule_between_rows(layer, b, 1, 255);
    if lines < 3 {
        return;
    }
    // Body split into sub-columns; each gets its own small style (one sub-column per frame is
    // occasionally an inverse slab). Dividers are 1 px and slightly dimmer than full text.
    let nsub = if b.w() >= 220 {
        3
    } else if b.w() >= 120 {
        2
    } else {
        1
    };
    let inv_style = rng.random_range(0..nsub.max(1)); // which sub-column is inverse
    let inner_x0 = b.x0 + 2;
    let inner_x1 = b.x1 - 2;
    let avail = inner_x1 - inner_x0;
    let sub_w = avail / nsub;
    for r in 1..lines {
        let row_base = (b.y0 + (r as i32) * MONO_PITCH) as f32 + asc;
        for j in 0..nsub {
            let sx0 = inner_x0 + j * sub_w;
            let sx1 = if j + 1 == nsub { inner_x1 } else { sx0 + sub_w };
            if sx1 <= sx0 {
                continue;
            }
            let sub_cols = ((sx1 - sx0 - 2) / 6).max(1) as usize;
            if rng.random_range(0..100) < 4 {
                continue; // ragged hole: some cells are empty
            }
            let s = match rng.random_range(0..100) {
                0..=39 => hex_row(rng, sub_cols),
                40..=69 => numeric_row(rng, sub_cols),
                70..=84 => binary_row(rng, sub_cols),
                _ => keyword_row(rng, sub_cols),
            };
            let sx = (sx0 + 1) as f32;
            if j == inv_style && r == 1 {
                // Pre-fill the inverse sub-column bright once (below the header); its rows knock
                // out dark text.
                for yy in (b.y0 + MONO_PITCH)..(b.y1 - 1) {
                    let row = yy as usize * layout::PW;
                    for xx in sx0..sx1 {
                        layer[(row + xx as usize) * 4] = 255;
                    }
                }
            }
            glyphs.draw_row(
                layer, MONO, MONO_PX, &s, sx, row_base, Advance::Grid(MONO_ADV), sx0, b.y0, sx1,
                b.y1, j == inv_style,
            );
            if j + 1 < nsub && sx1 < b.x1 - 1 {
                // Thin vertical divider between sub-columns.
                layout::vline_r(layer, sx1, b.y0 + 2, b.y1 - 2, 110);
            }
        }
    }
}

/// Labeled rule / separator: a keyword label row with a bright rule underneath, or (for short
/// blocks) a plain thin horizontal separator spanning the block.
fn draw_rule(rng: &mut StdRng, glyphs: &mut GlyphCache, layer: &mut [u8], b: &Block, asc: f32) {
    let lines = b.lines();
    if lines <= 3 {
        // Plain long thin separators at 1/3 and 2/3 height.
        let y1 = b.y0 + (lines as i32) * MONO_PITCH;
        let sep = (y1 - b.y0) / 3;
        layout::hline_r(layer, b.x0 + 2, b.x1 - 2, b.y0 + sep, 255);
        if lines >= 3 {
            layout::hline_r(layer, b.x0 + 2, b.x1 - 2, b.y0 + sep * 2, 255);
        }
        return;
    }
    let (cols, x0) = cols_of(b);
    let word = HEADERS[rng.random_range(0..HEADERS.len())];
    let mut s = String::with_capacity(cols);
    s.push_str(word);
    while s.len() < cols {
        s.push(' ');
    }
    s.truncate(cols);
    let baseline = b.y0 as f32 + asc;
    glyphs.draw_row(
        layer, MONO, MONO_PX, &s, x0, baseline, Advance::Grid(MONO_ADV), b.x0, b.y0, b.x1, b.y1,
        false,
    );
    rule_between_rows(layer, b, 1, 255);
}

/// Plain dense text rows. Occasionally the block opens with a mini header (file-name style)
/// row, and longer blocks sometimes carry a bright rule mid-way (the film's thin horizontal
/// separators inside text).
fn draw_text_block(rng: &mut StdRng, glyphs: &mut GlyphCache, layer: &mut [u8], b: &Block, asc: f32) {
    let lines = b.lines();
    let has_header = lines >= 4 && rng.random_range(0..100) < 16;
    let (cols, x0) = cols_of(b);
    if has_header {
        let word = HEADERS[rng.random_range(0..HEADERS.len())];
        let mut s = String::from(word);
        while s.len() < cols {
            s.push(' ');
        }
        s.truncate(cols);
        let baseline = b.y0 as f32 + asc;
        glyphs.draw_row(
            layer, MONO, MONO_PX, &s, x0, baseline, Advance::Grid(MONO_ADV), b.x0, b.y0, b.x1,
            b.y1, false,
        );
    }
    for r in (if has_header { 1 } else { 0 })..lines {
        let t = rng.random_range(0..100);
        let s = if t < 26 {
            hex_row(rng, cols)
        } else if t < 46 {
            numeric_row(rng, cols)
        } else if t < 68 {
            keyword_row(rng, cols)
        } else if t < 82 {
            binary_row(rng, cols)
        } else {
            opcode_row(rng, cols)
        };
        if s.is_empty() {
            continue;
        }
        let baseline = (b.y0 + (r as i32) * MONO_PITCH) as f32 + asc;
        glyphs.draw_row(
            layer, MONO, MONO_PX, &s, x0, baseline, Advance::Grid(MONO_ADV), b.x0, b.y0, b.x1,
            b.y1, false,
        );
    }
    // Occasional mid-block bright rule in a gutter between rows (never on top of glyphs). These
    // give the film's horizontal "floors" that break up the vertical text runs.
    if lines >= 6 && rng.random_range(0..100) < 24 {
        let k = ((lines as i32) * 2 / 3).max(1).min(lines as i32 - 1);
        rule_between_rows(layer, b, k, 255);
    }
}

/// Fill one mosaic panel's R channel (geometry must already be in the planes).
pub fn render(rng: &mut StdRng, layer: &mut [u8], blocks: &[Block], glyphs: &mut GlyphCache) {
    let (asc, _desc) = glyphs.line_metrics(MONO, MONO_PX);
    for (i, b) in blocks.iter().enumerate() {
        // blocks[0] is always the top-most full-width anchor: force it to a chunky slab so every
        // mosaic face shows at least one solid inverse-video anchor near the top.
        if i == 0 {
            draw_slab(rng, glyphs, layer, b, asc);
            continue;
        }
        let roll = rng.random_range(0..100);
        let tall = b.h() >= 200; // tall-narrow columns stay text, never a gimmick block
        if roll < 8 && !tall {
            draw_slab(rng, glyphs, layer, b, asc);
        } else if roll < 19 && !tall {
            draw_framed_pane(rng, glyphs, layer, b, asc);
        } else if roll < 25 && !tall && b.h() <= 140 && b.w() >= 64 {
            layout::bars_r(rng, layer, b);
        } else if roll < 29 && !tall && b.h() <= 170 {
            draw_rule(rng, glyphs, layer, b, asc);
        } else {
            draw_text_block(rng, glyphs, layer, b, asc);
        }
    }
}
