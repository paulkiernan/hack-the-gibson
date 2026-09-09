//! Tower-face text atlas generator.
//!
//! Batch 0 scaffold: `generate` returns a correctly sized, zero-filled `AtlasImage` with empty
//! per-panel block lists. Batch 1 rasterizes 64 layers of 256 × 768 (panels 0..28 mosaics,
//! 28..32 directory lists; layer `p + 32` = variant B of panel `p`) with fontdue.
//!
//! Fonts are embedded at compile time (OFL-licensed; see `assets/fonts/OFL-*.txt`). They are
//! public so Batch 1 can rasterize from them; the scaffold only proves the bytes are present.

use gibson_types::{AtlasImage, ATLAS_HEIGHT, ATLAS_LAYERS, ATLAS_PANELS, ATLAS_WIDTH};

/// Michroma Regular (OFL) — the wide Eurostile-Extended-like face used for directory lists.
pub const MICHROMA_TTF: &[u8] = include_bytes!("../../../assets/fonts/Michroma-Regular.ttf");
/// IBM Plex Mono Medium (OFL) — the dense mono face used for mosaic text blocks.
pub const IBM_PLEX_MONO_TTF: &[u8] = include_bytes!("../../../assets/fonts/IBMPlexMono-Medium.ttf");

/// Generate the full atlas deterministically from `seed`.
///
/// Scaffold: zeroed buffer of exactly `ATLAS_WIDTH * ATLAS_HEIGHT * ATLAS_LAYERS * 4` bytes and
/// `ATLAS_PANELS` empty block-id lists.
pub fn generate(seed: u64) -> AtlasImage {
    let _ = seed; // Batch 1: derive a StdRng from the seed for the deterministic layout.
    let _ = (MICHROMA_TTF.len(), IBM_PLEX_MONO_TTF.len()); // fonts embedded; Batch 1 loads them.
    AtlasImage {
        width: ATLAS_WIDTH,
        height: ATLAS_HEIGHT,
        layers: ATLAS_LAYERS,
        rgba: vec![0u8; (ATLAS_WIDTH * ATLAS_HEIGHT * ATLAS_LAYERS) as usize * 4],
        blocks_per_panel: vec![Vec::new(); ATLAS_PANELS as usize],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_are_embedded_and_non_empty() {
        assert!(!MICHROMA_TTF.is_empty());
        assert!(!IBM_PLEX_MONO_TTF.is_empty());
        // Both carry the OFL magic comment? Not required — just sanity-check they look like fonts.
        assert!(MICHROMA_TTF.len() > 10_000);
        assert!(IBM_PLEX_MONO_TTF.len() > 10_000);
    }

    #[test]
    fn scaffold_atlas_has_expected_shape() {
        let a = generate(42);
        assert_eq!(a.width, ATLAS_WIDTH);
        assert_eq!(a.height, ATLAS_HEIGHT);
        assert_eq!(a.layers, ATLAS_LAYERS);
        assert_eq!(a.rgba.len(), (ATLAS_WIDTH * ATLAS_HEIGHT * ATLAS_LAYERS) as usize * 4);
        assert_eq!(a.blocks_per_panel.len(), ATLAS_PANELS as usize);
        assert!(a.rgba.iter().all(|&b| b == 0));
    }
}
