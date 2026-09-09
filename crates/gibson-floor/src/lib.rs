//! PCB floor-map generator for the Gibson ground plane.
//!
//! Batch 0 scaffold: `generate` returns a `96 × 96` tile whose cells are all `[0, 0, 0, 128]`
//! (no traces, brightness mid-range). Batch 1 walks 110 random Manhattan traces with pads/vias
//! plus 6 chips on the toroidal grid, deterministically from `seed`.

use gibson_types::{FloorMap, FLOOR_TILE_CELLS};

/// Generate the floor map deterministically from `seed`.
///
/// Scaffold: `cells = FLOOR_TILE_CELLS`, data zero-traced at brightness 128.
pub fn generate(seed: u64) -> FloorMap {
    let _ = seed; // Batch 1: derive a StdRng from the seed for the trace walk.
    let n = FLOOR_TILE_CELLS as usize;
    FloorMap {
        cells: FLOOR_TILE_CELLS,
        data: vec![[0u8, 0, 0, 128]; n * n],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_floor_has_expected_shape() {
        let f = generate(7);
        assert_eq!(f.cells, FLOOR_TILE_CELLS);
        assert_eq!(f.data.len(), (FLOOR_TILE_CELLS * FLOOR_TILE_CELLS) as usize);
        assert!(f.data.iter().all(|c| *c == [0u8, 0, 0, 128]));
    }
}
