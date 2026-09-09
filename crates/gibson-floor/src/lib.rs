//! PCB floor-map generator for the Gibson ground plane.
//!
//! `generate(seed)` deterministically builds one toroidal (seamlessly wrap-around)
//! `FLOOR_TILE_CELLS x FLOOR_TILE_CELLS` tile from a `StdRng` seeded with `seed`.
//! Rendering samples this map as `cell = floor(xz / 2.5) mod 96`, so the tile must
//! join cleanly against itself on every edge; all neighbour arithmetic here wraps with
//! `rem_euclid(96)` and continuity is preserved by construction.
//!
//! Cell record (`data[z * cells + x]`, row-major z then x, four bytes):
//! - R: half-segment direction bits measured *from the cell centre* — bit 1 = +x,
//!   2 = -x, 4 = +z, 8 = -z. A Manhattan trace stepping from cell A to its +x
//!   neighbour stamps the +x bit on A and the opposite (-x) bit on the neighbour, so
//!   every physical segment is described from both ends and the trace reads as a
//!   continuous polyline through cell centres (rounded elbows come from the renderer).
//! - G: ground feature — 0 none, 1 pad, 2 via, 3 chip body.
//! - B: unused, always 0.
//! - A: brightness, 128..=255 (128 empty; per-trace base 160..=255 on segments;
//!   255 on pads, vias, and chips).
//!
//! Contents: six 4 x 8-cell chip bodies (rectangles of `G = 3`, laid down first so they
//! never collide with one another across the wrap) with straight pin stubs leaving every
//! other cell along both long sides and running perpendicular to the side, then 110
//! random-walk traces that advance 6..=40 unit steps, turning +/-90 degrees at each step
//! with probability 0.22, and stop early when the next cell holds a chip or another
//! trace's endpoint. Each walk terminates on a pad (60 %) or a via (40 %). Walks never
//! step onto chips, pads, or vias, but they may cross other traces and pin stubs, so
//! nets merge where paths intersect and chips read as components the routing passes by.

use gibson_types::{FloorMap, FLOOR_TILE_CELLS};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// Tile edge in cells; every coordinate lives in `0..SIZE` and wraps via `rem_euclid`.
const SIZE: usize = FLOOR_TILE_CELLS as usize;

// R-plane half-segment direction bits, measured from the cell centre.
const BIT_PX: u8 = 1;
const BIT_NX: u8 = 2;
const BIT_PZ: u8 = 4;
const BIT_NZ: u8 = 8;

// G-plane ground features.
const G_PAD: u8 = 1;
const G_VIA: u8 = 2;
const G_CHIP: u8 = 3;

// A-plane brightness values.
const A_EMPTY: u8 = 128;
const A_FEATURE: u8 = 255;

// Feature counts fixed by the film-accurate floor look.
const TRACE_COUNT: usize = 110;
const CHIP_COUNT: usize = 6;
/// Short chip side in cells.
const CHIP_SHORT: usize = 4;
/// Long chip side in cells.
const CHIP_LONG: usize = 8;

/// Compass-ordered directions E, N, W, S as `(dx, dz, bit)`. A 90-degree turn is
/// `index +/- 1 (mod 4)`, so a single turn never reverses the heading.
const DIRS: [(i32, i32, u8); 4] = [
    (1, 0, BIT_PX),
    (0, -1, BIT_NZ),
    (-1, 0, BIT_NX),
    (0, 1, BIT_PZ),
];

/// Direction bit a neighbour must set for a segment arriving from the opposite side:
/// PX <-> NX and PZ <-> NZ.
#[inline]
const fn opposite(bit: u8) -> u8 {
    if bit <= BIT_NX { bit ^ 3 } else { bit ^ 12 }
}

/// Flat index into a row-major (z, x) backing store.
#[inline]
fn flat(x: usize, z: usize) -> usize {
    z * SIZE + x
}

/// Wrapped destination one unit step from `(x, z)` along `(dx, dz)` — the torus seam.
#[inline]
fn step_to(x: usize, z: usize, dx: i32, dz: i32) -> (usize, usize) {
    (
        (x as i64 + dx as i64).rem_euclid(SIZE as i64) as usize,
        (z as i64 + dz as i64).rem_euclid(SIZE as i64) as usize,
    )
}

/// In-progress tile: one `[R, G, B, A]` byte quad per cell.
type Grid = Vec<[u8; 4]>;

/// Record one unit segment from `(x, z)` toward compass direction `di`: stamp the
/// outgoing bit on the source cell and the matching incoming bit on the wrapped
/// destination, painting any still-empty cell with `base`. Returns the destination.
fn mark_segment(grid: &mut Grid, x: usize, z: usize, di: usize, base: u8) -> (usize, usize) {
    let (dx, dz, bit) = DIRS[di];
    let (nx, nz) = step_to(x, z, dx, dz);
    let src = &mut grid[flat(x, z)];
    src[0] |= bit;
    if src[3] == A_EMPTY {
        src[3] = base;
    }
    let dst = &mut grid[flat(nx, nz)];
    dst[0] |= opposite(bit);
    if dst[3] == A_EMPTY {
        dst[3] = base;
    }
    (nx, nz)
}

/// One chip body rectangle, allowed to wrap across the tile seam.
#[derive(Clone, Copy)]
struct Chip {
    /// First body cell along x (wrapped into `0..SIZE`).
    x: usize,
    /// First body cell along z (wrapped into `0..SIZE`).
    z: usize,
    /// Body extent along x: 4 or 8.
    w: usize,
    /// Body extent along z: 8 or 4.
    h: usize,
}

/// Place `CHIP_COUNT` chip bodies that never overlap one another, including across the
/// wrapped edges. Returns the bodies in placement order.
fn place_chips(rng: &mut StdRng) -> Vec<Chip> {
    let mut chips = Vec::with_capacity(CHIP_COUNT);
    // Occupancy for chip bodies only; pin stubs and traces are laid down later.
    let mut taken = vec![false; SIZE * SIZE];
    'place: for _ in 0..CHIP_COUNT {
        for _ in 0..512 {
            let (w, h) = if rng.random_bool(0.5) {
                (CHIP_SHORT, CHIP_LONG)
            } else {
                (CHIP_LONG, CHIP_SHORT)
            };
            let x = rng.random_range(0..SIZE);
            let z = rng.random_range(0..SIZE);
            let mut free = true;
            'probe: for dz in 0..h {
                for dx in 0..w {
                    if taken[flat((x + dx) % SIZE, (z + dz) % SIZE)] {
                        free = false;
                        break 'probe;
                    }
                }
            }
            if !free {
                continue;
            }
            for dz in 0..h {
                for dx in 0..w {
                    taken[flat((x + dx) % SIZE, (z + dz) % SIZE)] = true;
                }
            }
            chips.push(Chip { x, z, w, h });
            continue 'place;
        }
    }
    chips
}

/// Straight pin stub of length 2..=5 leaving one side cell of a chip and running
/// perpendicularly away from the body. Skipped when its origin lies inside another chip
/// body; stops early when it would run into a chip.
fn emit_pin(grid: &mut Grid, rng: &mut StdRng, x0: usize, z0: usize, di: usize) {
    if grid[flat(x0, z0)][1] != 0 {
        return;
    }
    let len: usize = rng.random_range(2..=5);
    let base: u8 = rng.random_range(160..=255);
    let (dx, dz, _) = DIRS[di];
    let (mut x, mut z) = (x0, z0);
    for _ in 0..len {
        let (nx, nz) = step_to(x, z, dx, dz);
        if grid[flat(nx, nz)][1] != 0 {
            break;
        }
        (x, z) = mark_segment(grid, x, z, di, base);
    }
}

/// Emit pin stubs along both long sides of a chip: every other cell of the side, so a
/// 4 x 8 or 8 x 4 body gets four pins per long side.
fn emit_pins(grid: &mut Grid, rng: &mut StdRng, chip: Chip) {
    // Tall bodies (4 x 8) have long sides running along z; wide bodies (8 x 4) along x.
    let along_z = chip.w < chip.h;
    let per_side = if along_z { chip.h / 2 } else { chip.w / 2 };
    for i in 0..per_side {
        let off = 2 * i;
        if along_z {
            // Outward from the left (-x) and right (+x) sides at z = z0.
            let z0 = (chip.z + off) % SIZE;
            emit_pin(grid, rng, (chip.x + SIZE - 1) % SIZE, z0, 2); // W
            emit_pin(grid, rng, (chip.x + chip.w) % SIZE, z0, 0);   // E
        } else {
            // Outward from the top (-z) and bottom (+z) sides at x = x0.
            let x0 = (chip.x + off) % SIZE;
            emit_pin(grid, rng, x0, (chip.z + SIZE - 1) % SIZE, 1); // N
            emit_pin(grid, rng, x0, (chip.z + chip.h) % SIZE, 3);   // S
        }
    }
}

/// One random-walk trace. Picks a free start cell and a random heading, walks 6..=40
/// unit steps (turning +/-90 degrees at each step with probability 0.22, stopping early
/// when the next cell is occupied by a chip or another trace's endpoint), then stamps a
/// pad (60 %) or via (40 %) on the final cell. Degenerate zero-step walks are retried
/// from a fresh start cell.
fn emit_trace(grid: &mut Grid, rng: &mut StdRng) {
    for _ in 0..64 {
        // Find a free start cell (not a chip, pad, or via).
        let mut start = None;
        for _ in 0..256 {
            let x = rng.random_range(0..SIZE);
            let z = rng.random_range(0..SIZE);
            if grid[flat(x, z)][1] == 0 {
                start = Some((x, z));
                break;
            }
        }
        let Some((mut x, mut z)) = start else { return };
        let mut di = rng.random_range(0..4);
        let target: usize = rng.random_range(6..=40);
        let base: u8 = rng.random_range(160..=255);
        let mut steps = 0usize;
        while steps < target {
            if rng.random_bool(0.22) {
                di = (di + if rng.random_bool(0.5) { 1 } else { 3 }) % 4;
            }
            let (dx, dz, _) = DIRS[di];
            let (nx, nz) = step_to(x, z, dx, dz);
            if grid[flat(nx, nz)][1] != 0 {
                break;
            }
            (x, z) = mark_segment(grid, x, z, di, base);
            steps += 1;
        }
        if steps == 0 {
            continue;
        }
        let end = &mut grid[flat(x, z)];
        end[1] = if rng.random_bool(0.6) { G_PAD } else { G_VIA };
        end[3] = A_FEATURE;
        return;
    }
}

/// Generate the floor map deterministically from `seed`.
///
/// Returns a `FLOOR_TILE_CELLS x FLOOR_TILE_CELLS` toroidal tile: six 4 x 8 chip bodies
/// with pin stubs plus 110 Manhattan random-walk traces ending in pads or vias, wrapped
/// seamlessly on every edge.
pub fn generate(seed: u64) -> FloorMap {
    let mut rng = StdRng::seed_from_u64(seed);

    // Lay down all chip bodies first so they can never collide with pads/vias, then the
    // pins (which stop at any body), then the random walks (which route around chips and
    // endpoints but may merge with other traces and pin stubs).
    let chips = place_chips(&mut rng);
    let mut grid: Grid = vec![[0u8, 0, 0, A_EMPTY]; SIZE * SIZE];
    for chip in &chips {
        for dz in 0..chip.h {
            for dx in 0..chip.w {
                let c = &mut grid[flat((chip.x + dx) % SIZE, (chip.z + dz) % SIZE)];
                c[1] = G_CHIP;
                c[3] = A_FEATURE;
            }
        }
    }
    for chip in chips {
        emit_pins(&mut grid, &mut rng, chip);
    }
    for _ in 0..TRACE_COUNT {
        emit_trace(&mut grid, &mut rng);
    }

    FloorMap {
        cells: FLOOR_TILE_CELLS,
        data: grid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_from_seed() {
        assert_eq!(generate(3), generate(3));
        // Different seeds must diverge (whole-tile output, so a collision is impossible
        // in practice); guards against accidentally ignoring the seed.
        assert_ne!(generate(3), generate(4));
    }

    #[test]
    fn shape_and_encoding_invariants() {
        let f = generate(42);
        assert_eq!(f.cells, FLOOR_TILE_CELLS);
        assert_eq!(f.data.len(), SIZE * SIZE);
        for c in &f.data {
            assert!(
                (A_EMPTY..=255).contains(&c[3]),
                "brightness A out of 128..=255: {}",
                c[3]
            );
            assert_eq!(c[2], 0, "B must stay unused (zero)");
            assert!(c[1] <= G_CHIP, "ground feature G out of 0..=3: {}", c[1]);
        }
    }

    #[test]
    fn continuity_across_torus_seam() {
        let f = generate(7);
        let mut violations = 0u64;
        for z in 0..SIZE {
            for x in 0..SIZE {
                let bits = f.data[flat(x, z)][0];
                // (set bit, mirror bit expected on the neighbour, neighbour offset)
                for (bit, mirror, dx, dz) in [
                    (BIT_PX, BIT_NX, 1i32, 0i32),
                    (BIT_NX, BIT_PX, -1, 0),
                    (BIT_PZ, BIT_NZ, 0, 1),
                    (BIT_NZ, BIT_PZ, 0, -1),
                ] {
                    if bits & bit == 0 {
                        continue;
                    }
                    let (nx, nz) = step_to(x, z, dx, dz);
                    if f.data[flat(nx, nz)][0] & mirror == 0 {
                        violations += 1;
                    }
                }
            }
        }
        assert_eq!(
            violations, 0,
            "half-segments without a mirror bit on the wrapped neighbour"
        );
    }

    #[test]
    fn chips_occupy_exactly_six_bodies() {
        for seed in [0u64, 1, 11, 99] {
            let f = generate(seed);
            let n = f.data.iter().filter(|c| c[1] == G_CHIP).count();
            assert_eq!(n, CHIP_COUNT * CHIP_SHORT * CHIP_LONG);
        }
    }

    #[test]
    fn floor_coverage_exceeds_fifteen_percent() {
        let f = generate(11);
        let traced = f.data.iter().filter(|c| c[0] != 0).count();
        assert!(
            traced * 100 >= 15 * SIZE * SIZE,
            "only {traced}/{SIZE} cells carry a trace segment"
        );
    }
}
