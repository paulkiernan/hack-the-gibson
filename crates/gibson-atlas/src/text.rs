//! Font plumbing: per-call glyph caching, line metrics, and text row drawing.
//!
//! Both faces are parsed once per `generate` call and rasterized glyphs are cached per
//! (face, size, character), so drawing 64 dense layers never re-rasterizes the same glyph.

use std::collections::HashMap;
use std::sync::Arc;

use fontdue::{Font, FontSettings, Metrics};

use crate::layout::PW;

/// Index of the IBM Plex Mono face in [`GlyphCache`].
pub const MONO: u8 = 0;
/// Index of the Michroma face in [`GlyphCache`].
pub const MICHROMA: u8 = 1;

/// Mosaic mono size in px (em).
pub const MONO_PX: f32 = 7.0;
/// Mosaic mono advance per character in px.
pub const MONO_ADV: f32 = 6.0;
/// Directory face size in px (em).
pub const DIR_PX: f32 = 18.0;

/// How pen advance is computed while stepping through a row.
#[derive(Clone, Copy)]
pub enum Advance {
    /// Fixed advance per character (the mono 6 px grid).
    Grid(f32),
    /// The face's real advance width per glyph (right-aligned directory text).
    Real,
}

/// One rasterized glyph: coverage bitmap plus metrics.
pub struct RasterGlyph {
    pub m: Metrics,
    /// Coverage 0..=255, row-major, top row first.
    pub cov: Vec<u8>,
}

/// Owns the two parsed fonts and the per-(face, px, char) raster cache.
pub struct GlyphCache {
    fonts: Vec<Font>,
    glyphs: HashMap<(u8, u32, char), Arc<RasterGlyph>>,
    lines: HashMap<(u8, u32), (f32, f32)>,
}

fn key_px(px: f32) -> u32 {
    px.to_bits()
}

impl GlyphCache {
    /// Parse both embedded faces. Panics if a font file is malformed (they are compile-time
    /// assets, so a parse failure is a build bug worth surfacing loudly).
    pub fn new(mono_bytes: &[u8], michroma_bytes: &[u8]) -> GlyphCache {
        let mono = Font::from_bytes(mono_bytes, FontSettings::default())
            .expect("IBM Plex Mono font bytes failed to parse");
        let michroma = Font::from_bytes(michroma_bytes, FontSettings::default())
            .expect("Michroma font bytes failed to parse");
        GlyphCache {
            fonts: vec![mono, michroma],
            glyphs: HashMap::new(),
            lines: HashMap::new(),
        }
    }

    /// (ascent, descent) for a face at `px`; descent is negative.
    pub fn line_metrics(&mut self, f: u8, px: f32) -> (f32, f32) {
        let k = (f, key_px(px));
        if let Some(&m) = self.lines.get(&k) {
            return m;
        }
        let (asc, desc) = match self.fonts[f as usize].horizontal_line_metrics(px) {
            Some(lm) => (lm.ascent, lm.descent),
            None => (px * 0.8, -px * 0.2),
        };
        self.lines.insert(k, (asc, desc));
        (asc, desc)
    }

    /// Fetch (rasterizing once per (face, px, char)) the glyph for `ch`.
    pub fn glyph(&mut self, f: u8, px: f32, ch: char) -> Arc<RasterGlyph> {
        let k = (f, key_px(px), ch);
        if let Some(g) = self.glyphs.get(&k) {
            return Arc::clone(g);
        }
        let (m, cov) = self.fonts[f as usize].rasterize(ch, px);
        let g = Arc::new(RasterGlyph { m, cov });
        self.glyphs.insert(k, Arc::clone(&g));
        g
    }

    /// Total advance width of `s` at face `f`, size `px`.
    pub fn width(&mut self, f: u8, px: f32, s: &str) -> f32 {
        let mut w = 0.0f32;
        for ch in s.chars() {
            w += self.glyph(f, px, ch).m.advance_width;
        }
        w
    }

    /// Draw one text row into the R channel of a layer.
    ///
    /// Glyph bitmaps are placed so their baseline sits at `baseline`; the pen starts at `x0` and
    /// steps by the advance mode. Writing is clipped to `[bx0..bx1) x [by0..by1)`. In inverse
    /// video the caller has pre-filled the region with R = 255 and `inverse` writes `255 - cov`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_row(
        &mut self,
        layer: &mut [u8],
        f: u8,
        px: f32,
        text: &str,
        x0: f32,
        baseline: f32,
        adv: Advance,
        bx0: i32,
        by0: i32,
        bx1: i32,
        by1: i32,
        inverse: bool,
    ) {
        let base = baseline.round() as i32;
        let mut pen = x0;
        for ch in text.chars() {
            let g = self.glyph(f, px, ch);
            let gx = pen.round() as i32 + g.m.xmin;
            // ymin is the offset of the bitmap's bottom edge (negative = below the baseline).
            let top = base - (g.m.ymin + g.m.height as i32);
            let bot = base - g.m.ymin; // exclusive
            let cx0 = gx.max(bx0);
            let cx1 = (gx + g.m.width as i32).min(bx1);
            let cy0 = top.max(by0);
            let cy1 = bot.min(by1);
            if cx1 > cx0 && cy1 > cy0 {
                let mw = g.m.width;
                for y in cy0..cy1 {
                    let row = y as usize * PW;
                    let cov_row = (y - top) as usize * mw;
                    for x in cx0..cx1 {
                        let c = g.cov[cov_row + (x - gx) as usize];
                        layer[(row + x as usize) * 4] = if inverse { 255 - c } else { c };
                    }
                }
            }
            match adv {
                Advance::Grid(a) => pen += a,
                Advance::Real => pen += g.m.advance_width,
            }
        }
    }
}
