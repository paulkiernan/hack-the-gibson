//! PCB floor-map generator for the Gibson ground plane.
//!
//! `generate(seed)` deterministically builds one toroidal (seamlessly wrap-around)
//! `FLOOR_TILE_CELLS x FLOOR_TILE_CELLS` tile from a `StdRng` seeded with `seed`.
//! Rendering samples this map as `cell = floor(xz / 2.5) mod 96`, so the tile must
//! join cleanly against itself on every edge; all neighbour arithmetic here wraps with
//! `rem_euclid(96)` and continuity is preserved by construction.
//!
//! The towers above stand on a regular 8 x 8 grid: one tower every `TOWER_PITCH`
//! (30) world units, centred at world `x = 15 + 30k`, `z = 30k`, which is cell
//! `cx = 6 + 12*kx`, `cz = 12*kz` (see [`FIRST_COL_CENTER`], [`CELLS_PER_TOWER`]).
//! Each tower footprint owns a grid-exact 7 x 7-cell keep-out square
//! (`|dx| <= 3`, `|dz| <= 3` around its centre) that stays completely empty — no trace
//! segments, chips, pads, or vias — so the renderer shows a clean dark margin of bare
//! substrate around every tower base. The keep-out squares leave 5-cell corridors
//! between them that run the full tile in both axes (the lanes the film's pulses and
//! long parallel trace runs travel down).
//!
//! Cell record (`data[z * cells + x]`, row-major z then x, four bytes):
//! - R: half-segment direction bits measured *from the cell centre* — bit 1 = +x,
//!   2 = -x, 4 = +z, 8 = -z. A Manhattan trace stepping from cell A to its +x
//!   neighbour stamps the +x bit on A and the opposite (-x) bit on the neighbour, so
//!   every physical segment is described from both ends and the trace reads as a
//!   continuous polyline through cell centres (rounded elbows come from the renderer).
//!   A segment is never allowed to point into a keep-out cell: runs that reach a tower
//!   margin stop on the free cell just outside it.
//! - G: ground feature — 0 none, 1 pad, 2 via, 3 chip body.
//! - B: unused, always 0.
//! - A: brightness, 128..=255 (128 empty; per-run base 160..=255 on segments;
//!   255 on pads, vias, and chips).
//!
//! Contents, in placement order:
//! 1. Six 4 x 8-cell chip bodies (rectangles of `G = 3`, laid down first so they never
//!    collide with one another or with tower keep-out squares across the wrap), each
//!    with straight pin stubs leaving every other cell along both long sides and
//!    running perpendicular to the side. Pins stop against tower zones and stamp a pad
//!    on the margin.
//! 2. Long straight bus bundles: 2-4 parallel tracks running together down the lane
//!    corridors (`BUS_COUNT` bundles, both orientations) — the dominant visual of long
//!    parallel runs that reads as real circuit-board bus routing.
//! 3. Tower-margin feeds: short runs from a pad on the free cell just outside a
//!    keep-out square, heading into the corridor until they merge into an earlier
//!    trace or stop at the next tower's margin — tower bases read as components whose
//!    land pads sit on the routing.
//! 4. `TRACE_COUNT` secondary random walks that weave through the corridors with long
//!    straight stretches (turns are rare), cross bus lines, merge into pads/vias, and
//!    always terminate on a pad when they stop against a tower margin.
//!
//! Runs end on a pad (60 %+) or a via; a via is never placed against a keep-out square
//! — margins always get pads, exactly like a board's component lands.

use gibson_types::{FloorMap, FLOOR_TILE_CELLS, FLOOR_TILE_UNITS, TOWER_PITCH};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// Tile edge in cells; every coordinate lives in `0..SIZE` and wraps via `rem_euclid`.
const SIZE: usize = FLOOR_TILE_CELLS as usize;

/// World units per cell (`240 / 96 = 2.5`).
const UNITS_PER_CELL: f32 = FLOOR_TILE_UNITS / FLOOR_TILE_CELLS as f32;

/// Tower pitch in cells: one tower per `TOWER_PITCH` world units, i.e. 12 cells. The
/// tile is exactly `TOWERS_PER_SIDE` towers per side (96 / 12 = 8).
const CELLS_PER_TOWER: usize = (TOWER_PITCH / UNITS_PER_CELL) as usize;

/// Towers per tile side; derived again from world units in the tests
/// (`FLOOR_TILE_UNITS / TOWER_PITCH = 8`).
const TOWERS_PER_SIDE: usize = SIZE / CELLS_PER_TOWER;

/// Cell column of the first tower centre: towers sit at world `x = 15 + 30k` (half a
/// pitch from the tile origin), and 15 world units = 6 cells.
const FIRST_COL_CENTER: usize = ((TOWER_PITCH * 0.5) / UNITS_PER_CELL) as usize;

/// Keep-out zone half-width in cells around each tower centre: zones are
/// `2 * ZONE_HALF + 1 = 7` cells square, a footprint (12 world units = 4.8 cells)
/// plus a clean margin.
const ZONE_HALF: usize = 3;

/// Width in cells of the fully-free routing corridor between neighbouring keep-out
/// squares: `12 - 7 = 5` cells (12.5 world units), along both axes.
const BAND: usize = CELLS_PER_TOWER - (2 * ZONE_HALF + 1);

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

// Feature counts shaping the film look: long bus bundles dominate the corridors,
// margin feeds ring the tower bases, walks provide secondary routing.
const TRACE_COUNT: usize = 110;
/// Straight bus bundles to emit (each 2-4 parallel tracks, mixed orientations).
const BUS_COUNT: usize = 12;
/// Tower-margin feed attempts (each places a pad plus a short run).
const FEED_ATTEMPTS: usize = 240;
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

/// Minimum toroidal distance in cells from cell `v` to the nearest point of the form
/// `first + k * step` on the `SIZE`-ring (tower centres repeat every `step` cells).
#[inline]
fn ring_dist(v: usize, first: usize, step: usize) -> usize {
    let d = (v as i64 - first as i64).rem_euclid(step as i64) as usize;
    d.min(step - d)
}

/// True when cell `(x, z)` lies inside one of the 7 x 7 keep-out squares sitting,
/// grid-exactly, over the 8 x 8 tower footprints. Zones repeat every 12 cells along
/// each axis and never overlap (spacing 12 > 7).
#[inline]
fn keep_out(x: usize, z: usize) -> bool {
    ring_dist(x, FIRST_COL_CENTER, CELLS_PER_TOWER) <= ZONE_HALF
        && ring_dist(z, 0, CELLS_PER_TOWER) <= ZONE_HALF
}

/// True when `(x, z)` is a free cell orthogonally adjacent to a keep-out square — the
/// tower-margin ring where terminating runs must become pads.
#[inline]
fn near_keep_out(x: usize, z: usize) -> bool {
    DIRS.iter().any(|(dx, dz, _)| {
        let (nx, nz) = step_to(x, z, *dx, *dz);
        keep_out(nx, nz)
    })
}

/// In-progress tile: one `[R, G, B, A]` byte quad per cell.
type Grid = Vec<[u8; 4]>;

/// Record one unit segment from `(x, z)` toward compass direction `di`: stamp the
/// outgoing bit on the source cell and the matching incoming bit on the wrapped
/// destination, painting any still-empty cell with `base`. Returns the destination.
/// Callers guarantee neither endpoint lies inside a keep-out square.
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

/// Place `CHIP_COUNT` chip bodies that never overlap one another or a tower keep-out
/// square, including across the wrapped edges. Returns the bodies in placement order.
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
                    let (px, pz) = ((x + dx) % SIZE, (z + dz) % SIZE);
                    if taken[flat(px, pz)] || keep_out(px, pz) {
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

/// Stamp the terminating feature of a run whose head stopped on open cell `(x, z)`
/// (no merge into an existing pad/via happened). Runs that end against a tower
/// keep-out square or a chip body always become pads — the component-pad look at
/// margins and chip edges — otherwise the run ends on a pad with probability
/// `1 - via_p` and on a via with probability `via_p`. A via is never left on the
/// margin ring. Cells already carrying a feature are left alone.
fn end_run(grid: &mut Grid, rng: &mut StdRng, x: usize, z: usize, via_p: f64) {
    let idx = flat(x, z);
    if grid[idx][1] != 0 {
        return; // already terminated (e.g. a pad stamped on the starting cell)
    }
    let adjacent_component = near_keep_out(x, z)
        || DIRS.iter().any(|(dx, dz, _)| {
            let (nx, nz) = step_to(x, z, *dx, *dz);
            grid[flat(nx, nz)][1] == G_CHIP
        });
    let c = &mut grid[idx];
    c[1] = if adjacent_component || !rng.random_bool(via_p) {
        G_PAD
    } else {
        G_VIA
    };
    c[3] = A_FEATURE;
}

/// Stamp a pad on `(x, z)` when it is a free cell sitting on the tower-margin ring
/// (orthogonally adjacent to a keep-out square). A run *starting* on the ring must not
/// leave an unterminated tail against the margin, so its origin becomes a pad.
fn pad_if_on_margin(grid: &mut Grid, x: usize, z: usize) {
    let idx = flat(x, z);
    if grid[idx][1] == 0 && near_keep_out(x, z) {
        grid[idx][1] = G_PAD;
        grid[idx][3] = A_FEATURE;
    }
}

/// Straight pin stub of length 2..=5 leaving one side cell of a chip and running
/// perpendicularly away from the body. Skipped when its origin lies inside another
/// chip body or a tower keep-out square; stops at keep-out squares and chip bodies
/// with a pad on the margin, and merges into any pad or via it meets.
fn emit_pin(grid: &mut Grid, rng: &mut StdRng, x0: usize, z0: usize, di: usize) {
    if grid[flat(x0, z0)][1] != 0 || keep_out(x0, z0) {
        return;
    }
    pad_if_on_margin(grid, x0, z0);
    let len: usize = rng.random_range(2..=5);
    let base: u8 = rng.random_range(160..=255);
    let (dx, dz, _) = DIRS[di];
    let (mut x, mut z) = (x0, z0);
    for _ in 0..len {
        let (nx, nz) = step_to(x, z, dx, dz);
        let g = grid[flat(nx, nz)][1];
        if keep_out(nx, nz) || g == G_CHIP {
            end_run(grid, rng, x, z, 0.0); // against a tower zone or chip: pad only
            return;
        }
        if g == G_PAD || g == G_VIA {
            mark_segment(grid, x, z, di, base); // merge into the endpoint
            return;
        }
        (x, z) = mark_segment(grid, x, z, di, base);
    }
    // Natural end: leave the stub open unless it happens to reach a tower margin.
    if near_keep_out(x, z) {
        end_run(grid, rng, x, z, 0.0);
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

/// One straight bus bundle: 2..=4 parallel tracks confined to a 5-cell corridor (the
/// fully-free lane between two tower rows or columns) running a long distance in one
/// direction. Tracks advance in lockstep; a track that meets a chip stops with an end
/// feature, one that meets a pad or via merges into it, while its siblings continue.
/// Corridor cells are never keep-out, so a clear run only ever ends at the length cap,
/// where remaining tracks receive pad/via terminations. Returns whether a bundle was
/// placed.
fn emit_bus(grid: &mut Grid, rng: &mut StdRng) -> bool {
    for _ in 0..24 {
        let horizontal = rng.random_bool(0.5);
        let band_index = rng.random_range(0..TOWERS_PER_SIDE);
        let s: usize = rng.random_range(2..=4);
        let off0 = rng.random_range(0..=(BAND - s));
        // Horizontal corridors cover z rows 12b+4..12b+8 (between tower rows b and
        // b+1); vertical corridors cover x cols 12a-2..12a+2 (between tower columns
        // a-1 and a). Tracks take `off0..off0+s-1` consecutive cells of the band so
        // they read as one bundle; the run travels the other axis.
        let (fixed_base, di) = if horizontal {
            let dir = if rng.random_bool(0.5) { 0 } else { 2 }; // E / W
            (band_index * CELLS_PER_TOWER + ZONE_HALF + 1 + off0, dir)
        } else {
            let dir = if rng.random_bool(0.5) { 3 } else { 1 }; // S / N
            (
                (band_index * CELLS_PER_TOWER + SIZE - BAND / 2 + off0) % SIZE,
                dir,
            )
        };
        let (dx, dz, _) = DIRS[di];
        // Fixed coordinate (corridor row or column) per track, wrapped.
        let fixed: Vec<usize> = (0..s).map(|k| (fixed_base + k) % SIZE).collect();
        // Find a starting line along the moving axis where all `s` first cells are free.
        let mut head0 = None;
        for _ in 0..32 {
            let h = rng.random_range(0..SIZE);
            let mut ok = true;
            for k in 0..s {
                let (x, z) = if horizontal { (h, fixed[k]) } else { (fixed[k], h) };
                if grid[flat(x, z)][1] != 0 {
                    ok = false;
                    break;
                }
            }
            if ok {
                head0 = Some(h);
                break;
            }
        }
        let Some(head0) = head0 else { continue };
        // A track that starts right against a tower margin must originate on a pad.
        for k in 0..s {
            let (x, z) = if horizontal { (head0, fixed[k]) } else { (fixed[k], head0) };
            pad_if_on_margin(grid, x, z);
        }
        let len: usize = if rng.random_bool(0.4) {
            SIZE // full lap of the torus: a run that never visibly ends
        } else {
            rng.random_range(24..=64)
        };
        let wrap = len == SIZE;
        let base: u8 = rng.random_range(170..=255);
        let mut heads = vec![head0; s];
        let mut alive = vec![true; s];
        let mut remaining = s;
        let mut moved = 0usize;
        while moved < len && remaining > 0 {
            for k in 0..s {
                if !alive[k] {
                    continue;
                }
                let h = heads[k];
                let (x, z) = if horizontal { (h, fixed[k]) } else { (fixed[k], h) };
                let (nx, nz) = step_to(x, z, dx, dz);
                let g = grid[flat(nx, nz)][1];
                if g == G_CHIP {
                    end_run(grid, rng, x, z, 0.4);
                    alive[k] = false;
                    remaining -= 1;
                    continue;
                }
                if g == G_PAD || g == G_VIA {
                    mark_segment(grid, x, z, di, base); // merge into the endpoint
                    alive[k] = false;
                    remaining -= 1;
                    continue;
                }
                // Free corridor cell: advance (crossing earlier traces is allowed).
                let (nx, nz) = mark_segment(grid, x, z, di, base);
                heads[k] = if horizontal { nx } else { nz };
            }
            moved += 1;
        }
        // Tracks still alive when the bundle ends: terminate normally, except a full
        // lap of the torus returns each track to its own starting cell with the run
        // closed — a seamless never-ending bus — so no end feature is stamped.
        if !wrap {
            for k in 0..s {
                if alive[k] {
                    let h = heads[k];
                    let (x, z) = if horizontal { (h, fixed[k]) } else { (fixed[k], h) };
                    end_run(grid, rng, x, z, 0.45);
                }
            }
        }
        return true;
    }
    false
}

/// Emit one tower-margin feed: a pad on the free cell just outside a keep-out square,
/// joined by a short straight run into the neighbouring corridor. The run merges into
/// any earlier trace it crosses (typically a bus bundle), or stops against the next
/// tower's keep-out margin with a second pad, so tower bases read as components whose
/// land pads sit on the routing. Returns whether a feed was placed.
fn emit_margin_feed(grid: &mut Grid, rng: &mut StdRng) -> bool {
    for _ in 0..16 {
        let kx = rng.random_range(0..TOWERS_PER_SIDE);
        let kz = rng.random_range(0..TOWERS_PER_SIDE);
        let di = rng.random_range(0..4);
        let t = rng.random_range(0..(2 * ZONE_HALF + 1)); // position along the side
        let cx = FIRST_COL_CENTER + kx * CELLS_PER_TOWER;
        let cz = kz * CELLS_PER_TOWER;
        // The free cell just outside the chosen side: 4-adjacent to the keep-out
        // square, sitting on the corridor at position `t` along that side.
        let (px, pz) = match di {
            0 => (
                (cx + ZONE_HALF + 1) % SIZE, // E
                (cz + t + SIZE - ZONE_HALF) % SIZE,
            ),
            1 => (
                (cx + t + SIZE - ZONE_HALF) % SIZE,
                (cz + SIZE - ZONE_HALF - 1) % SIZE, // N
            ),
            2 => (
                (cx + SIZE - ZONE_HALF - 1) % SIZE, // W
                (cz + t + SIZE - ZONE_HALF) % SIZE,
            ),
            _ => (
                (cx + t + SIZE - ZONE_HALF) % SIZE,
                (cz + ZONE_HALF + 1) % SIZE, // S
            ),
        };
        if grid[flat(px, pz)][1] != 0 {
            continue;
        }
        // Preflight the first step so the pad never ends up isolated with nowhere to
        // go (next cell must be open substrate or a mergeable trace).
        let (dx, dz, _) = DIRS[di];
        let (nx, nz) = step_to(px, pz, dx, dz);
        let g0 = grid[flat(nx, nz)][1];
        if keep_out(nx, nz) || g0 != 0 {
            continue;
        }
        let len: usize = rng.random_range(2..=9);
        let base: u8 = rng.random_range(160..=255);
        grid[flat(px, pz)][1] = G_PAD;
        grid[flat(px, pz)][3] = A_FEATURE;
        let (mut x, mut z) = (px, pz);
        let mut moves = 0usize;
        while moves < len {
            let (sx, sz) = step_to(x, z, dx, dz);
            let c = grid[flat(sx, sz)];
            if keep_out(sx, sz) || c[1] == G_CHIP {
                end_run(grid, rng, x, z, 0.4); // pad at the far tower's margin
                break;
            }
            if c[1] == G_PAD || c[1] == G_VIA {
                mark_segment(grid, x, z, di, base); // merge into the endpoint
                break;
            }
            if c[0] != 0 {
                mark_segment(grid, x, z, di, base); // merge into an earlier trace
                break;
            }
            (x, z) = mark_segment(grid, x, z, di, base);
            moves += 1;
        }
        if moves == len {
            end_run(grid, rng, x, z, 0.4); // natural end in open substrate
        }
        return true;
    }
    false
}

/// One secondary random-walk trace. Picks a free start cell and a heading, walks
/// 6..=40 unit steps with rare 90-degree turns and long enforced straight stretches,
/// stops early when the next cell is a keep-out square or a chip (endpoint pad on the
/// margin), merges into any pad or via it meets, and otherwise terminates on a pad
/// (60 %) or a via (40 %). Walks never start or step inside keep-out squares.
fn emit_trace(grid: &mut Grid, rng: &mut StdRng) {
    for _ in 0..64 {
        // Find a free start cell (not a chip, pad, via, or tower zone).
        let mut start = None;
        for _ in 0..256 {
            let x = rng.random_range(0..SIZE);
            let z = rng.random_range(0..SIZE);
            if !keep_out(x, z) && grid[flat(x, z)][1] == 0 {
                start = Some((x, z));
                break;
            }
        }
        let Some((mut x, mut z)) = start else { return };
        // A walk that starts right against a tower margin must originate on a pad.
        pad_if_on_margin(grid, x, z);
        let mut di = rng.random_range(0..4);
        let target: usize = rng.random_range(6..=40);
        let min_run: usize = 3 + rng.random_range(0..5);
        let base: u8 = rng.random_range(160..=255);
        let mut steps = 0usize;
        let mut since_turn = 0usize;
        while steps < target {
            if since_turn >= min_run && rng.random_bool(0.10) {
                di = (di + if rng.random_bool(0.5) { 1 } else { 3 }) % 4;
                since_turn = 0;
            }
            let (dx, dz, _) = DIRS[di];
            let (nx, nz) = step_to(x, z, dx, dz);
            let g = grid[flat(nx, nz)][1];
            if keep_out(nx, nz) || g == G_CHIP {
                break; // margin or chip edge: endpoint feature stamped below
            }
            if g == G_PAD || g == G_VIA {
                mark_segment(grid, x, z, di, base); // merge into the endpoint
                return;
            }
            (x, z) = mark_segment(grid, x, z, di, base);
            steps += 1;
            since_turn += 1;
        }
        if steps == 0 {
            continue;
        }
        end_run(grid, rng, x, z, 0.4);
        return;
    }
}

/// Generate the floor map deterministically from `seed`.
///
/// Returns a `FLOOR_TILE_CELLS x FLOOR_TILE_CELLS` toroidal tile: six 4 x 8 chip
/// bodies with pin stubs, straight bus bundles down the lane corridors, tower-margin
/// feeds with pads at every keep-out edge, and `TRACE_COUNT` secondary random walks —
/// with every tower footprint's 7 x 7-cell keep-out square left completely empty so
/// traces never run beneath the towers. Wrapped seamlessly on every edge.
pub fn generate(seed: u64) -> FloorMap {
    let mut rng = StdRng::seed_from_u64(seed);

    // Ordering: chip bodies first (so pads/vias/buses never collide with them), then
    // their pins, then the bus bundles, then margin feeds (which merge into the
    // buses), then the secondary walks (which merge into anything).
    let chips = place_chips(&mut rng);
    let mut grid: Grid = vec![[0u8, 0, 0, A_EMPTY]; SIZE * SIZE];
    for chip in &chips {
        for dz in 0..chip.h {
            for dx in 0..chip.w {
                let (px, pz) = ((chip.x + dx) % SIZE, (chip.z + dz) % SIZE);
                let c = &mut grid[flat(px, pz)];
                c[1] = G_CHIP;
                c[3] = A_FEATURE;
            }
        }
    }
    for chip in chips {
        emit_pins(&mut grid, &mut rng, chip);
    }
    let mut buses = 0usize;
    while buses < BUS_COUNT {
        if emit_bus(&mut grid, &mut rng) {
            buses += 1;
        } else {
            break; // substrate is saturated; enough bundles
        }
    }
    for _ in 0..FEED_ATTEMPTS {
        emit_margin_feed(&mut grid, &mut rng);
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

    /// All 8 x 8 keep-out squares: every cell centre pair `(cx + dx, cz + dz)` with
    /// `|dx|, |dz| <= ZONE_HALF`, wrap-aware, in row-major order.
    fn zone_cells() -> Vec<(usize, usize)> {
        let mut cells = Vec::with_capacity(TOWERS_PER_SIDE * TOWERS_PER_SIDE * (2 * ZONE_HALF + 1)
            * (2 * ZONE_HALF + 1));
        for kz in 0..TOWERS_PER_SIDE {
            for kx in 0..TOWERS_PER_SIDE {
                let cx = FIRST_COL_CENTER + kx * CELLS_PER_TOWER;
                let cz = kz * CELLS_PER_TOWER;
                for dz in -(ZONE_HALF as i64)..=(ZONE_HALF as i64) {
                    for dx in -(ZONE_HALF as i64)..=(ZONE_HALF as i64) {
                        cells.push((
                            (cx as i64 + dx).rem_euclid(SIZE as i64) as usize,
                            (cz as i64 + dz).rem_euclid(SIZE as i64) as usize,
                        ));
                    }
                }
            }
        }
        cells
    }

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

    // --- New: tower keep-out zones are grid-exact and completely empty. ---

    #[test]
    fn keep_out_zones_empty_and_world_aligned() {
        // The zone lattice must match the frozen world layout: towers every
        // TOWER_PITCH world units, first column at x = 15 (half a pitch), first row at
        // z = 0; the tile holds FLOOR_TILE_UNITS / TOWER_PITCH towers per side.
        assert_eq!(SIZE, FLOOR_TILE_CELLS as usize);
        assert_eq!(
            CELLS_PER_TOWER as f32,
            TOWER_PITCH / UNITS_PER_CELL,
            "tower pitch must be an exact cell count"
        );
        assert_eq!(
            TOWERS_PER_SIDE as f32,
            FLOOR_TILE_UNITS / TOWER_PITCH,
            "the tile must hold a whole number of tower pitches per side"
        );
        assert_eq!(CELLS_PER_TOWER * TOWERS_PER_SIDE, SIZE);
        // Tower centres expressed in cells must land back on the world-space grid.
        for kx in 0..TOWERS_PER_SIDE {
            let world_x = (FIRST_COL_CENTER + kx * CELLS_PER_TOWER) as f32 * UNITS_PER_CELL;
            assert!(
                (world_x.rem_euclid(TOWER_PITCH) - TOWER_PITCH * 0.5).abs() < 1e-3,
                "tower column {kx} must sit at world x = 15 mod 30"
            );
        }
        for kz in 0..TOWERS_PER_SIDE {
            let world_z = (kz * CELLS_PER_TOWER) as f32 * UNITS_PER_CELL;
            assert!(
                world_z.rem_euclid(TOWER_PITCH).abs() < 1e-3,
                "tower row {kz} must sit at world z = 0 mod 30"
            );
        }
        // 7x7 zones around every centre, spaced 12 apart, never overlap: exactly
        // 64 * 49 = 3136 distinct keep-out cells on the tile.
        let zones = zone_cells();
        let mut dedup = zones.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(
            dedup.len(),
            TOWERS_PER_SIDE * TOWERS_PER_SIDE * (2 * ZONE_HALF + 1) * (2 * ZONE_HALF + 1),
            "keep-out zones must not overlap"
        );

        let f = generate(11);
        for (x, z) in dedup {
            let c = f.data[flat(x, z)];
            assert_eq!(c[0], 0, "trace segment inside keep-out square at ({x}, {z})");
            assert_eq!(c[1], 0, "feature inside keep-out square at ({x}, {z})");
            assert_eq!(c[3], A_EMPTY, "keep-out square at ({x}, {z}) not left bare");
        }
    }

    // --- New: traces never enter tower zones and terminate with pads at the margin. ---

    #[test]
    fn traces_terminate_at_keep_out_margins() {
        let f = generate(11);
        let mut into_zone = 0u64;
        let mut pads_on_margin = 0u64;
        let mut vias_on_margin = 0u64;
        let mut dead_end_on_margin = 0u64;
        for z in 0..SIZE {
            for x in 0..SIZE {
                let c = f.data[flat(x, z)];
                for (bit, dx, dz) in [
                    (BIT_PX, 1i32, 0i32),
                    (BIT_NX, -1, 0),
                    (BIT_PZ, 0, 1),
                    (BIT_NZ, 0, -1),
                ] {
                    if c[0] & bit != 0 {
                        let (nx, nz) = step_to(x, z, dx, dz);
                        if keep_out(nx, nz) {
                            into_zone += 1;
                        }
                    }
                }
                if near_keep_out(x, z) {
                    match c[1] {
                        G_PAD => pads_on_margin += 1,
                        G_VIA => vias_on_margin += 1,
                        _ => {
                            // A run that dead-ends (exactly one segment bit, i.e. the
                            // head or tail of a trace) right against a tower zone must
                            // be terminated by a pad, never left open.
                            let bits = c[0];
                            if bits != 0 && bits & (bits - 1) == 0 {
                                dead_end_on_margin += 1;
                            }
                        }
                    }
                }
            }
        }
        println!(
            "seed 11: {pads_on_margin} pads on tower margins, {vias_on_margin} vias on margins"
        );
        assert_eq!(into_zone, 0, "segments pointing into keep-out squares");
        assert_eq!(vias_on_margin, 0, "vias must not sit against a tower margin");
        assert_eq!(
            dead_end_on_margin, 0,
            "an unterminated trace dead-ends against a tower margin"
        );
        assert!(
            pads_on_margin >= 200,
            "expected a ring of pads at the tower margins, got {pads_on_margin}"
        );
    }

    // --- New: continuity holds and coverage clears 15 % overall and on routable area. ---

    #[test]
    fn continuity_and_coverage_over_routable_area() {
        let f = generate(11);
        // Continuity across the torus (mirror bits), re-checked on this seed.
        let mut violations = 0u64;
        for z in 0..SIZE {
            for x in 0..SIZE {
                let bits = f.data[flat(x, z)][0];
                for (bit, mirror, dx, dz) in [
                    (BIT_PX, BIT_NX, 1i32, 0i32),
                    (BIT_NX, BIT_PX, -1, 0),
                    (BIT_PZ, BIT_NZ, 0, 1),
                    (BIT_NZ, BIT_PZ, 0, -1),
                ] {
                    if bits & bit != 0 {
                        let (nx, nz) = step_to(x, z, dx, dz);
                        if f.data[flat(nx, nz)][0] & mirror == 0 {
                            violations += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(violations, 0);
        // Coverage overall (9216 cells) and over just the routable (non-keep-out)
        // area, which the renderer shows as substrate between the tower zones.
        let mut traced_overall = 0u64;
        let mut routable = 0u64;
        let mut traced_routable = 0u64;
        for z in 0..SIZE {
            for x in 0..SIZE {
                let inside = keep_out(x, z);
                if !inside {
                    routable += 1;
                }
                if f.data[flat(x, z)][0] != 0 {
                    traced_overall += 1;
                    if !inside {
                        traced_routable += 1;
                    }
                }
            }
        }
        let total = (SIZE * SIZE) as u64;
        let keep_out_total =
            (TOWERS_PER_SIDE * TOWERS_PER_SIDE * (2 * ZONE_HALF + 1) * (2 * ZONE_HALF + 1)) as u64;
        assert_eq!(
            routable,
            total - keep_out_total,
            "routable area must be the complement of the keep-out squares"
        );
        let pct_overall = 100.0 * traced_overall as f64 / total as f64;
        let pct_routable = 100.0 * traced_routable as f64 / routable as f64;
        println!(
            "seed 11: {traced_overall}/{total} cells carry traces ({pct_overall:.1}%), \
             {traced_routable}/{routable} routable cells ({pct_routable:.1}%)"
        );
        assert!(
            traced_overall * 100 >= 15 * total,
            "only {traced_overall}/{total} cells carry a trace segment"
        );
        assert!(
            pct_routable >= 20.0,
            "routable-area coverage {pct_routable:.1}% is too sparse for a bus layout"
        );
    }
}
