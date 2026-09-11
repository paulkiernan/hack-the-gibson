//! Tower-face text atlas generator.
//!
//! Produces 64 layers of 256 x 768 RGBA: panel `p` variant A lives in layer `p` and its variant B
//! in layer `p + 32` (32 = `ATLAS_LAYERS / 2`). Panels 0..28 are dense mono-text mosaics, panels
//! 28..32 are hero directory lists. Variants A and B share byte-identical block geometry (the
//! G/B/A planes) and differ only in the rasterized glyph content (R). Everything is fully
//! deterministic from the seed via `StdRng`.
//!
//! Channel semantics per pixel (see the frozen `gibson-types` contract):
//! - R: glyph coverage (0 = empty, 255 = solid glyph; inverse-video blocks store 255 - coverage).
//! - G: block-local vertical position, 0 at the block's top row ..= 255 at its bottom row.
//! - B: block id (0 outside any block, 1..=255 inside).
//! - A: 255 inside a block, 0 outside.
//!
//! Fonts are embedded at compile time (OFL-licensed; see `assets/fonts/OFL-*.txt`).

mod directory;
mod layout;
mod mosaic;
mod text;

use gibson_types::{AtlasImage, ATLAS_HEIGHT, ATLAS_LAYERS, ATLAS_PANELS, ATLAS_WIDTH};
use rand::rngs::StdRng;
use rand::SeedableRng;

/// Michroma Regular (OFL) — the wide Eurostile-Extended-like face used for directory lists.
pub const MICHROMA_TTF: &[u8] = include_bytes!("../../../assets/fonts/Michroma-Regular.ttf");
/// IBM Plex Mono Medium (OFL) — the dense mono face used for mosaic text blocks.
pub const IBM_PLEX_MONO_TTF: &[u8] = include_bytes!("../../../assets/fonts/IBMPlexMono-Medium.ttf");

/// Generate the full atlas deterministically from `seed`.
pub fn generate(seed: u64) -> AtlasImage {
    generate_internal(seed).0
}

/// Generate the atlas and also return each panel's block geometry. Geometry is used by the tests
/// to verify the channel invariants; it is not part of the public contract.
pub(crate) fn generate_internal(seed: u64) -> (AtlasImage, Vec<Vec<layout::Block>>) {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut glyphs = text::GlyphCache::new(IBM_PLEX_MONO_TTF, MICHROMA_TTF);
    let layer_bytes = layout::LAYER_BYTES;
    let mut rgba = vec![0u8; ATLAS_LAYERS as usize * layer_bytes];
    let mut blocks_per_panel: Vec<Vec<u8>> = vec![Vec::new(); ATLAS_PANELS as usize];
    let mut geometry_all = Vec::with_capacity(ATLAS_PANELS as usize);

    // Layers 0..32 hold variant A of each panel, layers 32..64 variant B.
    let (half_a, half_b) = rgba.split_at_mut((ATLAS_LAYERS as usize / 2) * layer_bytes);
    for p in 0..ATLAS_PANELS as usize {
        let blocks = if p < 28 {
            mosaic::geometry(&mut rng)
        } else {
            directory::geometry(&mut rng)
        };
        let la = &mut half_a[p * layer_bytes..(p + 1) * layer_bytes];
        let lb = &mut half_b[p * layer_bytes..(p + 1) * layer_bytes];
        // Identical geometry -> byte-identical G/B/A planes in both layers.
        layout::fill_planes(la, &blocks);
        layout::fill_planes(lb, &blocks);
        if p < 28 {
            mosaic::render(&mut rng, la, &blocks, &mut glyphs);
            mosaic::render(&mut rng, lb, &blocks, &mut glyphs);
        } else {
            directory::render(&mut rng, la, &blocks, &mut glyphs);
            directory::render(&mut rng, lb, &blocks, &mut glyphs);
        }
        ensure_r_differs(la, lb, &blocks);
        blocks_per_panel[p] = blocks.iter().map(|b| b.id).collect();
        geometry_all.push(blocks);
    }

    let image = AtlasImage {
        width: ATLAS_WIDTH,
        height: ATLAS_HEIGHT,
        layers: ATLAS_LAYERS,
        rgba,
        blocks_per_panel,
    };
    (image, geometry_all)
}

/// Variants must differ in glyph content. Variant A and B content streams are independent, so an
/// identical R plane is effectively impossible; should it ever happen, flip one pixel inside the
/// first block to make the guarantee structural.
fn ensure_r_differs(a: &mut [u8], b: &mut [u8], blocks: &[layout::Block]) {
    let same = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .all(|(pa, pb)| pa[0] == pb[0]);
    if same {
        let bl = &blocks[0];
        let x = (bl.x0 + bl.x1) / 2;
        let y = (bl.y0 + bl.y1) / 2;
        let i = (y as usize * layout::PW + x as usize) * 4;
        a[i] = a[i].wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYER_BYTES: usize = layout::LAYER_BYTES;

    fn layer<'a>(rgba: &'a [u8], l: usize) -> &'a [u8] {
        &rgba[l * LAYER_BYTES..(l + 1) * LAYER_BYTES]
    }

    #[test]
    fn fonts_are_embedded_and_non_empty() {
        assert!(!MICHROMA_TTF.is_empty());
        assert!(!IBM_PLEX_MONO_TTF.is_empty());
        // Both carry the OFL magic comment? Not required — just sanity-check they look like fonts.
        assert!(MICHROMA_TTF.len() > 10_000);
        assert!(IBM_PLEX_MONO_TTF.len() > 10_000);
    }

    #[test]
    fn shape_matches_contract() {
        let a = generate(42);
        assert_eq!(a.width, ATLAS_WIDTH);
        assert_eq!(a.height, ATLAS_HEIGHT);
        assert_eq!(a.layers, ATLAS_LAYERS);
        assert_eq!(
            a.rgba.len(),
            (ATLAS_WIDTH as usize) * (ATLAS_HEIGHT as usize) * (ATLAS_LAYERS as usize) * 4
        );
        assert_eq!(a.blocks_per_panel.len(), ATLAS_PANELS as usize);
    }

    #[test]
    fn deterministic_given_seed() {
        let a = generate(7);
        let b = generate(7);
        assert_eq!(a, b);
    }

    #[test]
    fn every_panel_has_unique_ascending_in_range_blocks() {
        let a = generate(11);
        assert_eq!(a.blocks_per_panel.len(), ATLAS_PANELS as usize);
        for (p, ids) in a.blocks_per_panel.iter().enumerate() {
            assert!(ids.len() >= 6, "panel {p} has only {} blocks", ids.len());
            assert!(
                ids.windows(2).all(|w| w[0] < w[1]),
                "panel {p} ids are not strictly ascending: {ids:?}"
            );
            assert!(
                ids.iter().all(|&b| (1..=255).contains(&b)),
                "panel {p} id out of range: {ids:?}"
            );
        }
    }

    #[test]
    fn every_layer_has_minimum_glyph_coverage() {
        let a = generate(3);
        let wpx = (ATLAS_WIDTH * ATLAS_HEIGHT) as usize;
        let threshold = wpx as f64 * 0.02;
        for l in 0..ATLAS_LAYERS as usize {
            let lay = layer(&a.rgba, l);
            let nz = lay.chunks_exact(4).filter(|px| px[0] != 0).count();
            assert!(
                nz as f64 >= threshold,
                "layer {l} has {nz} nonzero-R pixels, below 2% of {wpx}"
            );
        }
    }

    #[test]
    fn layout_invariants_hold_across_many_seeds() {
        // Coverage density and the >= 6 block minimum are layout-statistical; sweep seeds to keep
        // the guarantees honest.
        const SEEDS: [u64; 10] = [0, 1, 2, 5, 7, 11, 19, 42, 99, 1234];
        let wpx = (ATLAS_WIDTH * ATLAS_HEIGHT) as usize;
        let threshold = wpx as f64 * 0.02;
        for &seed in &SEEDS {
            let a = generate(seed);
            for (p, ids) in a.blocks_per_panel.iter().enumerate() {
                assert!(
                    ids.len() >= 6,
                    "seed {seed} panel {p}: only {} blocks",
                    ids.len()
                );
                assert!(
                    ids.windows(2).all(|w| w[0] < w[1]),
                    "seed {seed} panel {p} ids not ascending: {ids:?}"
                );
            }
            for l in 0..ATLAS_LAYERS as usize {
                let lay = layer(&a.rgba, l);
                let nz = lay.chunks_exact(4).filter(|px| px[0] != 0).count();
                assert!(
                    nz as f64 >= threshold,
                    "seed {seed} layer {l}: {nz} nonzero-R pixels, below 2% of {wpx}"
                );
            }
        }
    }

    #[test]
    fn variants_share_planes_but_differ_in_glyphs() {
        let a = generate(9);
        for p in 0..ATLAS_PANELS as usize {
            let va = layer(&a.rgba, p);
            let vb = layer(&a.rgba, p + ATLAS_PANELS as usize);
            for (pa, pb) in va.chunks_exact(4).zip(vb.chunks_exact(4)) {
                assert_eq!(pa[1], pb[1], "panel {p} G plane differs");
                assert_eq!(pa[2], pb[2], "panel {p} B plane differs");
                assert_eq!(pa[3], pb[3], "panel {p} A plane differs");
            }
            let r_differs = va
                .chunks_exact(4)
                .zip(vb.chunks_exact(4))
                .any(|(pa, pb)| pa[0] != pb[0]);
            assert!(r_differs, "panel {p} variants share an identical R plane");
        }
    }

    #[test]
    fn block_channels_are_consistent() {
        let (img, panels) = generate_internal(19);
        assert_eq!(panels.len(), ATLAS_PANELS as usize);
        for (p, blocks) in panels.iter().enumerate() {
            let lay = layer(&img.rgba, p);
            assert_eq!(
                img.blocks_per_panel[p],
                blocks.iter().map(|b| b.id).collect::<Vec<u8>>(),
                "panel {p} blocks_per_panel mismatch"
            );
            for b in blocks {
                let mid = (b.x0 + b.x1) / 2;
                let mut prev: i32 = -1;
                for y in b.y0..b.y1 {
                    let i = (y as usize * layout::PW + mid as usize) * 4;
                    assert_eq!(
                        lay[i + 3],
                        255,
                        "panel {p} block {}: A not 255 inside",
                        b.id
                    );
                    assert_eq!(lay[i + 2], b.id, "panel {p}: B != block id inside");
                    let g = lay[i + 1] as i32;
                    assert!(
                        g >= prev,
                        "panel {p} block {}: G decreased at row {y}",
                        b.id
                    );
                    prev = g;
                }
                let top_i = (b.y0 as usize * layout::PW + mid as usize) * 4;
                assert_eq!(lay[top_i + 1], 0, "panel {p} block {}: top G != 0", b.id);
                let bot_i = ((b.y1 - 1) as usize * layout::PW + mid as usize) * 4;
                assert_eq!(
                    lay[bot_i + 1],
                    255,
                    "panel {p} block {}: bottom G != 255",
                    b.id
                );
            }
            // Outside every block: A = 0 and B = 0.
            let mut inside = vec![false; (ATLAS_WIDTH * ATLAS_HEIGHT) as usize];
            for b in blocks {
                for y in b.y0..b.y1 {
                    for x in b.x0..b.x1 {
                        inside[y as usize * ATLAS_WIDTH as usize + x as usize] = true;
                    }
                }
            }
            for (idx, &inb) in inside.iter().enumerate() {
                if !inb {
                    let i = idx * 4;
                    assert_eq!(
                        lay[i + 2],
                        0,
                        "panel {p}: B nonzero outside blocks at {idx}"
                    );
                    assert_eq!(
                        lay[i + 3],
                        0,
                        "panel {p}: A nonzero outside blocks at {idx}"
                    );
                }
            }
        }
    }

    /// Contract B: the renderer stacks panels vertically as bands on a tall face, so every layer
    /// must keep a few pixel rows of empty margin at its top and bottom edges (band seams read
    /// as natural gaps, and no design element may sit at the very edge of a face).
    #[test]
    fn every_layer_has_empty_top_and_bottom_margin_rows() {
        const SEEDS: [u64; 6] = [0, 5, 11, 42, 99, 1234];
        let wpx = ATLAS_WIDTH as usize;
        for &seed in &SEEDS {
            let a = generate(seed);
            for l in 0..ATLAS_LAYERS as usize {
                let lay = layer(&a.rgba, l);
                for y in 0..layout::MARGIN as usize {
                    let row = &lay[y * wpx * 4..(y + 1) * wpx * 4];
                    assert!(
                        row.iter().all(|&b| b == 0),
                        "seed {seed} layer {l}: top margin row {y} has coverage"
                    );
                }
                for y in (ATLAS_HEIGHT as usize - layout::MARGIN as usize)..ATLAS_HEIGHT as usize {
                    let row = &lay[y * wpx * 4..(y + 1) * wpx * 4];
                    assert!(
                        row.iter().all(|&b| b == 0),
                        "seed {seed} layer {l}: bottom margin row {y} has coverage"
                    );
                }
            }
        }
    }

    /// Film composition: every mosaic panel must mix structure like the film faces — at least one
    /// tall narrow column block, at least one full-width anchor that is drawn as an inverse-video
    /// slab, and a higher mean block count than the old horizontal-band layout (which averaged
    /// 11.5 blocks per mosaic panel; the column-strip layout must stay clearly above that).
    #[test]
    fn mosaic_composition_is_film_like() {
        const SEEDS: [u64; 8] = [0, 1, 3, 5, 11, 42, 99, 1234];
        let mut total_mean = 0.0f64;
        for &seed in &SEEDS {
            let (img, panels) = generate_internal(seed);
            let mut panel_means = Vec::new();
            for p in 0..28 {
                let blocks = &panels[p];
                assert!(
                    blocks.len() >= 10,
                    "seed {seed} mosaic panel {p}: only {} blocks",
                    blocks.len()
                );
                // Tall narrow column block (the film's column strips): >= 200 px tall, <= 96 wide.
                assert!(
                    blocks.iter().any(|b| b.w() <= 96 && b.h() >= 200),
                    "seed {seed} mosaic panel {p}: no tall narrow column block"
                );
                // A full-width-ish anchor exists (>= 160 wide, short band).
                let anchor = blocks
                    .iter()
                    .find(|b| b.w() >= 160 && b.h() <= layout::MONO_PITCH * 12)
                    .unwrap_or_else(|| panic!("seed {seed} mosaic panel {p}: no wide anchor"));
                // And that anchor is an inverse-video slab in variant A: bright fill (mean R
                // over the rectangle high) with knocked-out glyphs.
                let lay = layer(&img.rgba, p);
                let (mut sum, mut n) = (0u64, 0u64);
                for y in anchor.y0..anchor.y1 {
                    let row = y as usize * layout::PW;
                    for x in anchor.x0..anchor.x1 {
                        sum += lay[(row + x as usize) * 4] as u64;
                        n += 1;
                    }
                }
                let mean_r = sum as f64 / n as f64;
                assert!(
                    mean_r >= 170.0,
                    "seed {seed} mosaic panel {p}: top anchor (w={}, h={}) mean R {mean_r:.0} < 170 (not an inverse slab)",
                    anchor.w(),
                    anchor.h()
                );
                panel_means.push(blocks.len() as f64);
            }
            total_mean += panel_means.iter().sum::<f64>() / panel_means.len() as f64;
        }
        let overall = total_mean / SEEDS.len() as f64;
        // 11.5 was the old band layout's mosaic mean; require the column layout to stay well
        // above it across the seed sweep.
        assert!(
            overall >= 14.0,
            "mosaic mean block count {overall:.1} dropped below the 11.5 of the old band layout"
        );
    }
}
