//! PCB floor-map generator for the Gibson ground plane.
//!
//! `generate(seed)` deterministically builds one toroidal (seamlessly wrap-around)
//! `FLOOR_TILE_CELLS x FLOOR_TILE_CELLS` tile from a `StdRng` seeded with `seed`.
//! Rendering samples this map as `cell = floor(xz / 2.5) mod 96`, so the tile must
//! join cleanly against itself on every edge; all neighbour arithmetic here wraps with
//! `rem_euclid(96)` and continuity is preserved by construction.
//!
//! # The towers are the integrated circuits
//!
//! The towers stand on an 8 x 8 grid - one tower every `TOWER_PITCH` (30) world units,
//! centred on world `x = 15 + 30k`, `z = 30k`, i.e. cell `cx = 6 + 12*kx`, `cz = 12*kz`
//! (see [`FIRST_COL_CENTER`], [`CELLS_PER_TOWER`]) - and each tower base is the land
//! pattern of a packaged chip ([`TowerPkg`]):
//! - a 5 x 5 body block (`|dx| <= 2`, `|dz| <= 2`) marked `G_IC`, hidden under the tower
//!   and blocked to the router, so no net ever routes through a package;
//! - 12 pins (`G_PIN`) on the surrounding 7 x 7 ring, three per side at the -2 / 0 / +2
//!   positions of that side, which is where all copper attaches;
//! - a dotted silkscreen courtyard on the ring cells between pins and on the four corners,
//!   plus a reserved pin-1 dot just outside the -x/-z corner - which is what makes a
//!   package read as a package rather than as a bare patch of substrate;
//! - a guaranteed one-cell escape from every pin outward to a free "port" cell one step
//!   beyond the ring, so no pin is ever left floating.
//!
//! Between the packages the free substrate forms a lattice of five-cell-wide lanes
//! ([`LANE_HALF`]): pin ring, escape port, three cells of lane, escape port, ring. Bus
//! bundles and the thick power rails run down the lane centres, and every net is a
//! tower-to-tower run from one package's port to a neighbouring package's port - the local
//! chip-to-chip wiring of a real board, not a scatter of packages on a board that happens
//! to have towers on it.
//!
//! Cell record (`data[z * cells + x]`, row-major z then x, four bytes), exactly the
//! [`FloorMap`] encoding:
//! - R: direction mask of half-segments leaving the cell centre. Bits 0-3 are orthogonal
//!   (1 = +x, 2 = -x, 4 = +z, 8 = -z), bits 4-7 diagonal (16 = +x+z, 32 = -x+z,
//!   64 = +x-z, 128 = -x-z). Every stamped segment sets the outgoing bit on the cell and
//!   the mirror bit on the neighbour via [`floor_dir_mirror`], so continuity across any
//!   edge - wrap or diagonal - holds by construction rather than by later repair.
//!   Right-angle corners in signal traces are avoided: the router charges a 45-degree
//!   jog (two cheap 45-degree turns) less than a right angle (one expensive 90-degree
//!   turn), which is the preference a real board router has.
//! - G: 0 none, 1 through-hole pad, 2 via, 3 IC body, 4 SMD pad, 5 IC pin, 6 copper
//!   pour, 7 silkscreen, 8 mounting hole. (The tile carries no lone SMD discretes: the
//!   towers are the components, and a scattering of loose passives would dilute that.)
//! - B: bits 0-1 trace width class (0 thin signal, 1 medium, 2 thick power/ground),
//!   bit 2 set on cells that belong to a parallel bus bundle, bit 3 set on hatched
//!   pours. Bits 4-7 stay zero.
//! - A: brightness, 128..=255 (128 bare substrate).
//!
//! Contents, in placement order (each step only ever adds copper):
//! 1. Tower packages: body, pin ring, courtyard silk, pin-1 marker.
//! 2. Mounting holes on four free lane intersections.
//! 3. Bus bundles: 2-3 tracks at one gauge running together down a lane centre and
//!    turning together at lane intersections with a 45-degree bus corner (each track
//!    turns through its own diagonal jog, so the bundle stays parallel through the turn).
//!    Bus cells carry the bundle flag; a bundle is cut short where a package, land or
//!    mounting hole blocks the lane.
//! 4. Power rails: thick (gauge 2) runs down lanes 0 and 4 in both axes, crossing at four
//!    points into a single connected supply ring.
//! 5. Nets: every pin escapes outward to its port, and every port is routed octilinearly
//!    to a port of a neighbouring tower's package, so each of the 768 pins terminates on
//!    another pin and the tile reads as chip-to-chip wiring. A net the router cannot place
//!    lands on a via at its source port instead.
//! 6. Via fanouts: a bundle that changes layer drops its tracks into a staggered row of
//!    vias.
//! 7. Termination: any trace end left in the open becomes a pad, or a via, so no copper
//!    is left dangling in mid-air.
//! 8. Copper pours: the emptiest rectangles the finished routing left are flooded with
//!    ground-plane fill ([`POUR_W`] x [`POUR_H`] each), with a one-cell clearance channel
//!    around every trace, land and footprint - the negative space that makes the routing
//!    read as a board. Alternating regions come out hatched.
//! 9. Silkscreen: rings around the mounting holes, printed last so the marks sit on the
//!    planes, never on a trace. (The package courtyards were printed with the packages.)

use gibson_types::{floor_dir_mirror, FloorMap, FLOOR_TILE_CELLS, FLOOR_TILE_UNITS, TOWER_PITCH};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

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

/// Package half-extent in cells: the land pattern is a `2 * PKG_HALF + 1 = 7` cell square
/// around each tower centre, which covers the 12-unit tower box (4.8 cells wide) plus the
/// pin ring just outside it.
const PKG_HALF: i32 = 3;

/// Body half-extent: the 5 x 5 IC body block sitting under the tower itself.
const BODY_HALF: i32 = 2;

/// The three pin positions along each package side, as offsets from the side's centre.
const PIN_OFFSETS: [i32; 3] = [-2, 0, 2];

/// Pin index bases within [`TowerPkg::pins`]: three pins per side, in the order the four
/// package sides are emitted.
const SIDE_PX: usize = 0;
const SIDE_PZ: usize = 3;
const SIDE_NX: usize = 6;
const SIDE_NZ: usize = 9;

/// Half-width of the fully-free routing lane between neighbouring packages:
/// `12 - 7 = 5` cells wide, so a lane centre sits 2 cells clear of both pin rings. The
/// five cells run port, bus/rail, bus/rail, bus/rail, port.
const LANE_HALF: usize = (CELLS_PER_TOWER - (2 * PKG_HALF as usize + 1)) / 2;

// R-plane half-segment direction bits, in `FLOOR_DIR_MIRRORS` order: four orthogonal
// steps then four diagonals.
const BIT_PX: u8 = 1;
const BIT_NX: u8 = 2;
const BIT_PZ: u8 = 4;
const BIT_NZ: u8 = 8;
const BIT_DXPZ: u8 = 16;
const BIT_DXNZP: u8 = 32;
const BIT_DXPZN: u8 = 64;
const BIT_DXNZN: u8 = 128;

/// The eight unit steps, indexed so that `DIR_BITS[i]` is the direction mask of
/// `DIR_STEP[i]` - the same 1, 2, 4, 8, 16, 32, 64, 128 order as the R plane.
const DIR_STEP: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (-1, 1),
    (1, -1),
    (-1, -1),
];

/// Direction mask per [`DIR_STEP`] entry.
const DIR_BITS: [u8; 8] = [
    BIT_PX, BIT_NX, BIT_PZ, BIT_NZ, BIT_DXPZ, BIT_DXNZP, BIT_DXPZN, BIT_DXNZN,
];

// G-plane feature kinds.
const G_PAD: u8 = 1;
const G_VIA: u8 = 2;
const G_IC: u8 = 3;
const G_PIN: u8 = 5;
const G_POUR: u8 = 6;
const G_SILK: u8 = 7;
const G_HOLE: u8 = 8;

// B-plane gauge classes and flags.
const GAUGE_MASK: u8 = 0b0011;
const GAUGE_THIN: u8 = 0;
const GAUGE_MED: u8 = 1;
const GAUGE_POWER: u8 = 2;
const B_BUS: u8 = 0b0100;
const B_HATCH: u8 = 0b1000;

// A-plane brightness; everything written is >= A_EMPTY, as the encoding demands.
const A_EMPTY: u8 = 128;
const A_FEATURE: u8 = 255;
/// Silkscreen ink: one brightness for the courtyards, and a brighter dot for the pin-1
/// marker so a consumer can tell the marker from the outline.
const A_SILK: u8 = 228;
const A_SILK_PIN1: u8 = 250;

// Router costs. Straight and diagonal steps cost the same per unit distance; the turn
// charges are what make a 45-degree jog (two 45s) cheaper than a right angle (one 90).
const STEP_STRAIGHT: u32 = 10;
const STEP_DIAG: u32 = 15;
const TURN_45: u32 = 6;
const TURN_90: u32 = 14;
const TURN_SHARP: u32 = 28;
/// Extra charge for stepping onto a cell that already carries a trace: crossings are
/// allowed (this is a multi-layer composite view) but a router prefers not to.
const COST_CROSS_TRACE: u32 = 34;
/// Extra charge for stepping through a pad, via or pin that is not the destination.
const COST_CROSS_FEATURE: u32 = 65;
/// Extra charge for routing across a reserved ground-plane region: nets cross a plane when
/// they must, but they prefer the lanes beside it.
const COST_CROSS_PLANE: u32 = 80;

// Content budget: how much board the tile carries.
const BUS_COUNT: usize = 10;
/// Lane indices carrying thick power/ground rails, in both axes.
const RAIL_LANES: [usize; 2] = [0, 4];
/// Stitching via arrays dropped into lane pockets.
const VIA_ARRAYS: usize = 4;
/// A net may detour around packages and bundles, but a route longer than this is a walk
/// rather than a wiring run and is dropped in favour of a via at the source port.
const NET_LEN_CAP: usize = 48;
/// Ground-plane reservations: the top-left corner of each of the three rectangles the
/// router is charged for crossing, so the nets mostly skirt them the way signals route
/// around a plane rather than straight over it. They also leave the emptiest copper-free
/// pockets on an otherwise dense board, which is where the pour pass then floods.
const PLANE_RESERVES: [(usize, usize); 3] = [(12, 18), (60, 18), (36, 54)];
const RESERVE_W: usize = POUR_W + 6;
const RESERVE_H: usize = POUR_H + 4;
const POUR_W: usize = 38;
const POUR_H: usize = 24;

/// Flat index into a row-major (z, x) backing store.
#[inline]
fn flat(x: usize, z: usize) -> usize {
    z * SIZE + x
}

/// Wrapped destination one unit step from `(x, z)` along `(dx, dz)` - the torus seam.
#[inline]
fn step_to(x: usize, z: usize, dx: i32, dz: i32) -> (usize, usize) {
    (
        (x as i64 + dx as i64).rem_euclid(SIZE as i64) as usize,
        (z as i64 + dz as i64).rem_euclid(SIZE as i64) as usize,
    )
}

/// Signed wrapped offset from `a` to `b` along one axis, in `-(SIZE/2)..=(SIZE/2)`.
#[inline]
fn delta(a: usize, b: usize) -> i32 {
    let d = (b as i64 - a as i64).rem_euclid(SIZE as i64) as i32;
    if d > SIZE as i32 / 2 {
        d - SIZE as i32
    } else {
        d
    }
}

/// Signed offset of `v` from the lattice `first + k * step` on the `SIZE` ring. Only
/// meaningful well inside half a period, which is the case for every package cell here
/// (a package spans 7 of the 12 cells between tower centres).
#[inline]
fn lattice_offset(v: usize, first: usize, step: usize) -> i32 {
    let d = (v as i64 - first as i64).rem_euclid(step as i64) as i32;
    if d > step as i32 / 2 {
        d - step as i32
    } else {
        d
    }
}

/// Signed offset of `(x, z)` from its nearest tower centre, per axis.
#[inline]
fn tower_offset(x: usize, z: usize) -> (i32, i32) {
    (
        lattice_offset(x, FIRST_COL_CENTER, CELLS_PER_TOWER),
        lattice_offset(z, 0, CELLS_PER_TOWER),
    )
}

/// True when `(x, z)` lies inside a tower's 7 x 7 package: body, pins, courtyard and the
/// ring's free corners. The router never enters a package; copper attaches at the pin
/// escapes on its boundary.
#[inline]
fn in_package(x: usize, z: usize) -> bool {
    let (dx, dz) = tower_offset(x, z);
    dx.abs() <= PKG_HALF && dz.abs() <= PKG_HALF
}

/// True when `(x, z)` is one of the 12 IC pins of its package, returning that pin's
/// outward direction index: three pins per side, at the -2 / 0 / +2 positions of the side,
/// pointing away from the package centre.
#[inline]
fn pin_dir(x: usize, z: usize) -> Option<usize> {
    let (dx, dz) = tower_offset(x, z);
    let on_side = |along: i32| along == -2 || along == 0 || along == 2;
    if dx == PKG_HALF && on_side(dz) {
        return Some(0);
    }
    if dx == -PKG_HALF && on_side(dz) {
        return Some(1);
    }
    if dz == PKG_HALF && on_side(dx) {
        return Some(2);
    }
    if dz == -PKG_HALF && on_side(dx) {
        return Some(3);
    }
    None
}

/// True when `(x, z)` is a free cell orthogonally adjacent to a package - the port ring
/// the pin escapes run onto.
#[inline]
fn near_package(x: usize, z: usize) -> bool {
    (-1i32..=1).any(|dz| {
        (-1i32..=1).any(|dx| {
            let (nx, nz) = step_to(x, z, dx, dz);
            in_package(nx, nz)
        })
    })
}

/// Centre cell of routing lane `m` along x. Tower columns sit at world `x = 15 + 30k`
/// (cell `6 mod 12`), so the five-cell lane between two package columns is centred on
/// `x = 0 mod 12`.
#[inline]
fn lane_x(m: usize) -> usize {
    (m * CELLS_PER_TOWER) % SIZE
}

/// Centre cell of routing lane `m` along z. Tower rows sit at world `z = 30k` (cell
/// `0 mod 12`), so the z lanes are half a pitch out of phase with the x lanes and centred
/// on `z = 6 mod 12`.
#[inline]
fn lane_z(m: usize) -> usize {
    (FIRST_COL_CENTER + m * CELLS_PER_TOWER) % SIZE
}

/// The lane really is the gap between two pin rings: the geometry the whole layout leans on.
const _: () = assert!(2 * LANE_HALF + 1 == CELLS_PER_TOWER - 2 * PKG_HALF as usize - 1);

/// Index of the unit step `(dx, dz)`, if it is one of the eight directions.
#[inline]
fn dir_index(dx: i32, dz: i32) -> Option<usize> {
    DIR_STEP.iter().position(|&d| d == (dx, dz))
}

/// Direction index from cell `a` to its wrapped neighbour `b`.
#[inline]
fn step_dir(a: (usize, usize), b: (usize, usize)) -> usize {
    dir_index(delta(a.0, b.0), delta(a.1, b.1)).expect("consecutive cells must be adjacent")
}

/// True when direction `i` is one of the four diagonals.
#[inline]
fn is_diagonal(i: usize) -> bool {
    i >= 4
}

// ---------------------------------------------------------------------------
// IC packages
// ---------------------------------------------------------------------------

/// One IC pin: its package cell, its outward direction index, and the free cell one step
/// further out that the trace from it runs onto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pin {
    cell: (usize, usize),
    dir: usize,
    port: (usize, usize),
}

/// One tower's package: the land pattern the tower above stands on. `pins` holds 12 pins
/// in a fixed order - three on the `+x` side ([`SIDE_PX`]), then `+z`, `-x`, `-z`.
#[derive(Clone, Debug)]
struct TowerPkg {
    centre: (usize, usize),
    pins: Vec<Pin>,
}

/// The three pins of one package side: `dir` is the outward direction, `along_x` selects
/// whether the side's normal runs along x (so the pins vary in z) or along z.
fn side_pins(cx: usize, cz: usize, dir: usize, along_x: bool) -> Vec<Pin> {
    let (dx, dz) = DIR_STEP[dir];
    let sign = if dx + dz > 0 { PKG_HALF } else { -PKG_HALF };
    PIN_OFFSETS
        .iter()
        .map(|&o| {
            let cell = if along_x {
                step_to(cx, cz, sign, o)
            } else {
                step_to(cx, cz, o, sign)
            };
            Pin {
                cell,
                dir,
                port: step_to(cell.0, cell.1, dx, dz),
            }
        })
        .collect()
}

/// Index of the tower a cell belongs to, row-major in `(kz, kx)`. Used by the tests to
/// confirm a net joins two different towers.
#[cfg(test)]
#[inline]
fn tower_of(x: usize, z: usize) -> usize {
    let (dx, dz) = tower_offset(x, z);
    let cx = (x as i64 - dx as i64).rem_euclid(SIZE as i64) as usize;
    let cz = (z as i64 - dz as i64).rem_euclid(SIZE as i64) as usize;
    let kx = (cx - FIRST_COL_CENTER) / CELLS_PER_TOWER;
    let kz = cz / CELLS_PER_TOWER;
    kz * TOWERS_PER_SIDE + kx
}

// ---------------------------------------------------------------------------
// Board
// ---------------------------------------------------------------------------

/// In-progress tile: one `[R, G, B, A]` byte quad per cell, plus the routing barrier map.
struct Board {
    grid: Vec<[u8; 4]>,
    /// Cells the router must never enter: IC packages and mounting holes.
    blocked: Vec<bool>,
    /// Cells reserved for the ground planes, so nets route around them.
    plane: Vec<bool>,
}

impl Board {
    fn new() -> Self {
        Self {
            grid: vec![[0u8, 0, 0, A_EMPTY]; SIZE * SIZE],
            blocked: vec![false; SIZE * SIZE],
            plane: vec![false; SIZE * SIZE],
        }
    }

    #[inline]
    fn at(&self, x: usize, z: usize) -> [u8; 4] {
        self.grid[flat(x, z)]
    }

    /// Merge one cell's copper contribution: OR the direction bits, raise the gauge to the
    /// heaviest class seen, OR the flags, and paint brightness only where the cell is
    /// still bare substrate. Package cells are never painted by the router - copper reaches
    /// them only through the deliberate pin escape in [`Board::pin_escape`].
    #[inline]
    fn paint(&mut self, x: usize, z: usize, bits: u8, gauge: u8, flags: u8, base: u8) {
        if in_package(x, z) {
            return;
        }
        let c = &mut self.grid[flat(x, z)];
        c[0] |= bits;
        if gauge > c[2] & GAUGE_MASK {
            c[2] = (c[2] & !GAUGE_MASK) | gauge;
        }
        c[2] |= flags;
        if c[3] == A_EMPTY {
            c[3] = base;
        }
    }

    /// Record one unit segment from `(x, z)` toward direction `di`: the outgoing bit on
    /// the source and the mirror bit (from the shared [`floor_dir_mirror`] table) on the
    /// wrapped destination, so continuity holds on the torus and across diagonals.
    fn mark(&mut self, x: usize, z: usize, di: usize, gauge: u8, flags: u8, base: u8) {
        let (dx, dz) = DIR_STEP[di];
        let bit = DIR_BITS[di];
        let (mirror, _, _) = floor_dir_mirror(bit).expect("single direction bit");
        let (nx, nz) = step_to(x, z, dx, dz);
        self.paint(x, z, bit, gauge, flags, base);
        self.paint(nx, nz, mirror, gauge, flags, base);
    }

    /// The mandatory one-cell escape from an IC pin to its port: the pin's outward
    /// half-segment written straight onto the package cell the router may never enter,
    /// plus the port's mirror. Every pin gets exactly this one bit, so a pin always leaves
    /// its package outward even where no net reached it.
    fn pin_escape(&mut self, pin: &Pin, base: u8) {
        let bit = DIR_BITS[pin.dir];
        let (mirror, _, _) = floor_dir_mirror(bit).expect("single direction bit");
        self.grid[flat(pin.cell.0, pin.cell.1)][0] |= bit;
        self.paint(pin.port.0, pin.port.1, mirror, GAUGE_THIN, 0, base);
    }

    /// Stamp a whole routed polyline cell by cell.
    fn stamp(&mut self, path: &[(usize, usize)], gauge: u8, flags: u8, base: u8) {
        for pair in path.windows(2) {
            let di = step_dir(pair[0], pair[1]);
            self.mark(pair[0].0, pair[0].1, di, gauge, flags, base);
        }
    }

    /// Stamp a path in runs, skipping cells the router may not enter, so a long rail can
    /// break around a package instead of cutting straight through it.
    fn stamp_runs(&mut self, path: &[(usize, usize)], gauge: u8, flags: u8, base: u8) {
        let mut run: Vec<(usize, usize)> = Vec::new();
        for &c in path {
            if self.blocked[flat(c.0, c.1)] {
                if run.len() >= 2 {
                    self.stamp(&run, gauge, flags, base);
                }
                run.clear();
            } else {
                run.push(c);
            }
        }
        if run.len() >= 2 {
            self.stamp(&run, gauge, flags, base);
        }
    }

    /// Place a copper feature (pad, via, IC body, pin, pour, silk, hole).
    fn feature(&mut self, x: usize, z: usize, g: u8) {
        let c = &mut self.grid[flat(x, z)];
        c[1] = g;
        c[3] = A_FEATURE;
    }

    /// True when the cell is bare substrate: no copper, no feature, no silkscreen.
    #[inline]
    fn bare(&self, x: usize, z: usize) -> bool {
        let c = self.grid[flat(x, z)];
        c[0] == 0 && c[1] == 0
    }

    /// True when the cell carries something a pour must keep clear of: traces, pads,
    /// vias, pins, bodies, holes or silkscreen. Pour fill itself does not count, so a
    /// region can flood continuously around its own moats.
    #[inline]
    fn solid(&self, x: usize, z: usize) -> bool {
        let c = self.grid[flat(x, z)];
        c[0] != 0 || (c[1] != 0 && c[1] != G_POUR)
    }
}

// ---------------------------------------------------------------------------
// Tower packages
// ---------------------------------------------------------------------------

/// Lay the IC package under every tower: the 5 x 5 body, the 12-pin ring, the courtyard
/// silkscreen and the pin-1 marker. The whole package is blocked to the router, so a net
/// can only reach it through the deliberate pin escapes drawn later.
fn emit_tower_packages(board: &mut Board) -> Vec<TowerPkg> {
    let mut pkgs = Vec::with_capacity(TOWERS_PER_SIDE * TOWERS_PER_SIDE);
    for kz in 0..TOWERS_PER_SIDE {
        for kx in 0..TOWERS_PER_SIDE {
            let cx = FIRST_COL_CENTER + kx * CELLS_PER_TOWER;
            let cz = kz * CELLS_PER_TOWER;

            // Body: the chip under the tower. Marked, blocked, and carrying no copper.
            for dz in -BODY_HALF..=BODY_HALF {
                for dx in -BODY_HALF..=BODY_HALF {
                    let (x, z) = step_to(cx, cz, dx, dz);
                    board.feature(x, z, G_IC);
                    board.blocked[flat(x, z)] = true;
                }
            }

            // The ring: 12 pins at the -2/0/+2 positions, courtyard silk everywhere else
            // (the cells between pins and the four corners).
            for dz in -PKG_HALF..=PKG_HALF {
                for dx in -PKG_HALF..=PKG_HALF {
                    if dx.abs() <= BODY_HALF && dz.abs() <= BODY_HALF {
                        continue;
                    }
                    let (x, z) = step_to(cx, cz, dx, dz);
                    board.blocked[flat(x, z)] = true;
                    if pin_dir(x, z).is_none() {
                        board.feature(x, z, G_SILK);
                        board.grid[flat(x, z)][3] = A_SILK;
                    }
                }
            }

            let mut pins = Vec::with_capacity(12);
            pins.extend(side_pins(cx, cz, 0, true));
            pins.extend(side_pins(cx, cz, 2, false));
            pins.extend(side_pins(cx, cz, 1, true));
            pins.extend(side_pins(cx, cz, 3, false));
            for pin in &pins {
                board.feature(pin.cell.0, pin.cell.1, G_PIN);
            }

            // Pin-1 marker: a reserved silkscreen dot just outside the -x/-z corner, the
            // corner beside the first pin of the -x side. Blocked so the dot always prints
            // and never ends up under a trace.
            let (mx, mz) = step_to(cx, cz, -(PKG_HALF + 1), -(PKG_HALF + 1));
            board.feature(mx, mz, G_SILK);
            board.grid[flat(mx, mz)][3] = A_SILK_PIN1;
            board.blocked[flat(mx, mz)] = true;

            pkgs.push(TowerPkg {
                centre: (cx, cz),
                pins,
            });
        }
    }
    pkgs
}

/// One mounting hole at each corner of the board's 48-cell quarter, on lane intersections
/// the power rails do not run through.
fn emit_mounting_holes(board: &mut Board) -> Vec<(usize, usize)> {
    let mut holes = Vec::new();
    for (a, b) in [(2usize, 2usize), (6, 2), (2, 6), (6, 6)] {
        let (x, z) = (lane_x(a), lane_z(b));
        assert!(!in_package(x, z), "mounting hole must sit on a free lane intersection");
        board.feature(x, z, G_HOLE);
        board.blocked[flat(x, z)] = true;
        holes.push((x, z));
    }
    holes
}

// ---------------------------------------------------------------------------
// Nets
// ---------------------------------------------------------------------------

/// One routed net: the two IC pins it joins and the routed length in cells (`None` when
/// the router found no corridor and the net landed on a via instead).
#[derive(Clone, Copy, Debug)]
struct Net {
    from: (usize, usize),
    to: (usize, usize),
    len: Option<usize>,
}

/// The free cell a pin's escape runs onto: one step outward from the pin cell.
#[inline]
fn port_of(pin: (usize, usize)) -> (usize, usize) {
    let dir = pin_dir(pin.0, pin.1).expect("a net endpoint must be an IC pin");
    let (dx, dz) = DIR_STEP[dir];
    step_to(pin.0, pin.1, dx, dz)
}

/// Plan every net: each tower's `+x` pins wire to the `-x` pins of the tower one pitch
/// east, and its `+z` pins to the `-z` pins of the tower one pitch north. The cyclic shift
/// of the pins along a side turns a straight run into a 45-degree jog - a real board's
/// wiring - while staying a bijection, so each of the 768 pins carries exactly one net and
/// every net joins two different towers.
fn plan_nets(rng: &mut StdRng, pkgs: &[TowerPkg]) -> Vec<Net> {
    let index = |kx: usize, kz: usize| -> usize {
        (kz % TOWERS_PER_SIDE) * TOWERS_PER_SIDE + (kx % TOWERS_PER_SIDE)
    };
    let mut nets = Vec::with_capacity(TOWERS_PER_SIDE * TOWERS_PER_SIDE * 6);
    for kz in 0..TOWERS_PER_SIDE {
        for kx in 0..TOWERS_PER_SIDE {
            let here = &pkgs[index(kx, kz)];
            let east = &pkgs[index(kx + 1, kz)];
            let north = &pkgs[index(kx, kz + 1)];
            let sx = rng.random_range(1..=2);
            let sz = rng.random_range(1..=2);
            for i in 0..3 {
                nets.push(Net {
                    from: here.pins[SIDE_PX + i].cell,
                    to: east.pins[SIDE_NX + (i + sx) % 3].cell,
                    len: None,
                });
                nets.push(Net {
                    from: here.pins[SIDE_PZ + i].cell,
                    to: north.pins[SIDE_NZ + (i + sz) % 3].cell,
                    len: None,
                });
            }
        }
    }
    nets
}

/// Draw every pin's escape, then route every net port-to-port octilinearly. A net the
/// router cannot place inside [`NET_LEN_CAP`] terminates on a via at its source port
/// rather than being left in the air.
fn emit_nets(board: &mut Board, rng: &mut StdRng, pkgs: &[TowerPkg], nets: &mut [Net]) {
    for pkg in pkgs {
        for pin in &pkg.pins {
            let base: u8 = rng.random_range(190..=250);
            board.pin_escape(pin, base);
        }
    }
    for net in nets.iter_mut() {
        let base: u8 = rng.random_range(185..=255);
        let from_port = port_of(net.from);
        match route(board, from_port, port_of(net.to)) {
            Some(path) if path.len() <= NET_LEN_CAP => {
                board.stamp(&path, GAUGE_THIN, 0, base);
                net.len = Some(path.len());
            }
            _ => {
                board.feature(from_port.0, from_port.1, G_VIA);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Bus bundles, with 45-degree bus corners
// ---------------------------------------------------------------------------

/// Split a cell path into maximal runs that share a step direction, returning each run's
/// direction index and its cells.
fn legs_of(path: &[(usize, usize)]) -> Vec<(usize, Vec<(usize, usize)>)> {
    let mut legs: Vec<(usize, Vec<(usize, usize)>)> = Vec::new();
    for pair in path.windows(2) {
        let di = step_dir(pair[0], pair[1]);
        match legs.last_mut() {
            Some((last, cells)) if *last == di => cells.push(pair[1]),
            _ => legs.push((di, vec![pair[0], pair[1]])),
        }
    }
    legs
}

/// Left-hand normal of a direction, in cell steps: `(dz, -dx)`. Integer for all eight
/// directions, which is what lets a bundle hold its track order through a corner.
#[inline]
fn left_normal(di: usize) -> (i32, i32) {
    let (dx, dz) = DIR_STEP[di];
    (dz, -dx)
}

/// Shift a cell by `o` steps along a normal.
#[inline]
fn shifted(c: (usize, usize), n: (i32, i32), o: i32) -> (usize, usize) {
    step_to(c.0, c.1, o * n.0, o * n.1)
}

/// Offset a routed centreline into one track of a parallel bundle: each leg runs on the
/// leg's line shifted `o` cells along its left normal, and consecutive legs are joined by
/// the connector `o * (normal_next - normal_prev)` - a pure 45-degree jog at a right
/// angle, which is exactly the bus corner a board router produces. Every track of the
/// bundle sees the same corner geometry shifted by its own offset, so the bundle stays
/// parallel and never crosses itself through the turn.
fn bus_track(centerline: &[(usize, usize)], o: i32) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (di, cells) in legs_of(centerline) {
        let n = left_normal(di);
        let start = shifted(cells[0], n, o);
        if let Some(&prev_end) = out.last() {
            // Join the previous leg's shifted end to this leg's shifted start: diagonal
            // steps first, then straight, which at a 90-degree corner is one clean jog.
            let dx = delta(prev_end.0, start.0);
            let dz = delta(prev_end.1, start.1);
            let (sx, sz) = (dx.signum(), dz.signum());
            let diag = dx.abs().min(dz.abs());
            let mut cur = prev_end;
            for _ in 0..diag {
                cur = step_to(cur.0, cur.1, sx, sz);
                out.push(cur);
            }
            for _ in 0..(dx.abs() - diag) {
                cur = step_to(cur.0, cur.1, sx, 0);
                out.push(cur);
            }
            for _ in 0..(dz.abs() - diag) {
                cur = step_to(cur.0, cur.1, 0, sz);
                out.push(cur);
            }
        }
        for (i, &c) in cells.iter().enumerate() {
            let s = shifted(c, n, o);
            if i == 0 {
                if out.last() != Some(&s) {
                    out.push(s);
                }
            } else {
                out.push(s);
            }
        }
    }
    out
}

/// Emit one parallel bus bundle: 2-3 tracks at one gauge running together down a lane and
/// turning together at lane intersections, with a 45-degree bus corner at each turn. The
/// tracks sit on the three lane cells between the two port columns, so a bundle never
/// lands on a pin escape. Returns the bundle's track paths when one was placed.
fn emit_bus(board: &mut Board, rng: &mut StdRng) -> Option<Vec<Vec<(usize, usize)>>> {
    for _ in 0..32 {
        let tracks = rng.random_range(2..=3);
        let offsets: Vec<i32> = if tracks == 2 { vec![-1, 1] } else { vec![-1, 0, 1] };
        let axis = rng.random_bool(0.5);
        let legs = rng.random_range(1..=3);
        let (m, n) = (
            rng.random_range(0..TOWERS_PER_SIDE),
            rng.random_range(0..TOWERS_PER_SIDE),
        );
        let mut centerline: Vec<(usize, usize)> = vec![(lane_x(m), lane_z(n))];
        for l in 0..legs {
            let along_x = if axis { l % 2 == 0 } else { l % 2 == 1 };
            let steps = rng.random_range(2..=4) as i32 * if rng.random_bool(0.5) { 1 } else { -1 };
            let (dx, dz) = if along_x {
                (steps.signum(), 0)
            } else {
                (0, steps.signum())
            };
            // One lane step is one whole tower pitch: walk it cell by cell so the
            // centreline is a real cell path the offsetting can follow.
            for _ in 0..(steps.abs() as usize * CELLS_PER_TOWER) {
                let last = *centerline.last().unwrap();
                centerline.push(step_to(last.0, last.1, dx, dz));
            }
        }
        if centerline.len() < 14 {
            continue;
        }
        let mut paths: Vec<Vec<(usize, usize)>> =
            offsets.iter().map(|&o| bus_track(&centerline, o)).collect();
        // A bundle runs on clear copper: bare substrate. It keeps the longest window all
        // its tracks share; the cut ends terminate on pads or vias like a real bundle.
        let len = paths.iter().map(|p| p.len()).min().unwrap_or(0);
        let (mut best_start, mut best_len) = (0usize, 0usize);
        let mut start = 0usize;
        for i in 0..len {
            let clear = paths.iter().all(|p| {
                let (x, z) = p[i];
                !board.blocked[flat(x, z)] && board.bare(x, z)
            });
            if clear {
                if i + 1 - start > best_len {
                    best_len = i + 1 - start;
                    best_start = start;
                }
            } else {
                start = i + 1;
            }
        }
        if best_len < 24 {
            continue;
        }
        for p in paths.iter_mut() {
            *p = p[best_start..best_start + best_len].to_vec();
        }
        let base: u8 = rng.random_range(180..=245);
        for path in &paths {
            board.stamp(path, GAUGE_MED, B_BUS, base);
        }
        return Some(paths);
    }
    None
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

/// Charge for turning between two headings: never reverse, and charge a 45-degree jog
/// (two cheap 45s) less than a 90-degree corner, which is what keeps right angles out of
/// the signal routing.
fn turn_cost(prev: usize, next: usize) -> u32 {
    if prev == next {
        return 0;
    }
    let (ax, az) = DIR_STEP[prev];
    let (bx, bz) = DIR_STEP[next];
    if (ax, az) == (-bx, -bz) {
        return u32::MAX / 4;
    }
    match ax * bx + az * bz {
        1 => TURN_45,
        0 => TURN_90,
        _ => TURN_SHARP,
    }
}

/// Octilinear A* between two cells: least-turn routing over the eight directions with a
/// turn charge, so paths come out as long straight runs joined by 45-degree jogs. Returns
/// the cell path including both endpoints.
fn route(board: &Board, from: (usize, usize), to: (usize, usize)) -> Option<Vec<(usize, usize)>> {
    let start = flat(from.0, from.1);
    let goal = flat(to.0, to.1);
    if start == goal || board.blocked[goal] || in_package(to.0, to.1) {
        return None;
    }
    let h = |cx: usize, cz: usize| -> u32 {
        let adx = {
            let d = (cx as i64 - to.0 as i64).rem_euclid(SIZE as i64) as usize;
            d.min(SIZE - d) as u32
        };
        let adz = {
            let d = (cz as i64 - to.1 as i64).rem_euclid(SIZE as i64) as usize;
            d.min(SIZE - d) as u32
        };
        (adx.max(adz) - adx.min(adz)) * STEP_STRAIGHT + adx.min(adz) * STEP_DIAG
    };
    let nodes = SIZE * SIZE * 9;
    let mut dist = vec![u32::MAX; nodes];
    let mut prev = vec![u32::MAX; nodes];
    let mut heap: BinaryHeap<Reverse<(u32, u32, u32)>> = BinaryHeap::new();
    let start_node = (start * 9 + 8) as u32;
    dist[start * 9 + 8] = 0;
    heap.push(Reverse((h(from.0, from.1), 0, start_node)));
    let mut found = None;
    while let Some(Reverse((_, g, node))) = heap.pop() {
        let node = node as usize;
        let cell = node / 9;
        let dir = node % 9;
        if cell == goal {
            found = Some(node as u32);
            break;
        }
        if g > dist[node] {
            continue;
        }
        let (cx, cz) = (cell % SIZE, cell / SIZE);
        for nd in 0..8 {
            let (dx, dz) = DIR_STEP[nd];
            let (nx, nz) = step_to(cx, cz, dx, dz);
            if board.blocked[flat(nx, nz)] || in_package(nx, nz) {
                continue;
            }
            let mut cost = if is_diagonal(nd) { STEP_DIAG } else { STEP_STRAIGHT };
            if dir != 8 {
                cost += turn_cost(dir, nd);
            }
            if board.plane[flat(nx, nz)] {
                cost += COST_CROSS_PLANE;
            }
            if flat(nx, nz) != goal {
                let c = board.at(nx, nz);
                if c[0] != 0 {
                    cost += COST_CROSS_TRACE;
                } else if (G_PAD..=G_PIN).contains(&c[1]) {
                    cost += COST_CROSS_FEATURE;
                }
            }
            let ng = g + cost;
            let nnode = flat(nx, nz) * 9 + nd;
            if ng < dist[nnode] {
                dist[nnode] = ng;
                prev[nnode] = node as u32;
                heap.push(Reverse((ng + h(nx, nz), ng, nnode as u32)));
            }
        }
    }
    let goal_node = found?;
    let mut path = Vec::new();
    let mut cur = goal_node;
    while cur != start_node {
        let cell = (cur as usize) / 9;
        path.push((cell % SIZE, cell / SIZE));
        let p = prev[cur as usize];
        if p == u32::MAX {
            break;
        }
        cur = p;
    }
    path.push(from);
    path.reverse();
    Some(path)
}

// ---------------------------------------------------------------------------
// Power rails, vias
// ---------------------------------------------------------------------------

/// Thick power/ground rails: one run down each axis through lane 0 and lane 4, wrapping,
/// so the four rails form a single connected supply ring across the tile. Runs break
/// around packages and mounting holes rather than cutting through them.
fn emit_rails(board: &mut Board, rng: &mut StdRng) {
    let base: u8 = rng.random_range(165..=205);
    for &m in &RAIL_LANES {
        for axis in 0..2 {
            let mut path = Vec::with_capacity(SIZE + 1);
            for i in 0..SIZE {
                let cell = if axis == 0 {
                    let x = lane_x(m);
                    (x, (i + x) % SIZE)
                } else {
                    let z = lane_z(m);
                    ((i + z) % SIZE, z)
                };
                path.push(cell);
            }
            path.push(path[0]);
            board.stamp_runs(&path, GAUGE_POWER, 0, base);
        }
    }
}

/// Via arrays: small grids of stitching vias dropped into the board's mid-lane pockets,
/// the plated-through drill pattern real boards carry all over their ground planes. They
/// sit half a pitch off the lane intersections, clear of the bundle lines and the ports.
fn emit_via_arrays(board: &mut Board, rng: &mut StdRng) -> usize {
    let mut made = 0usize;
    for _ in 0..512 {
        if made == VIA_ARRAYS {
            break;
        }
        let a = rng.random_range(0..TOWERS_PER_SIDE);
        let b = rng.random_range(0..TOWERS_PER_SIDE);
        let (cx, cz) = ((lane_x(a) + CELLS_PER_TOWER / 2) % SIZE, lane_z(b));
        let mut cells = Vec::new();
        'grid: for dz in [-1i32, 1] {
            for dx in [-3i32, -1, 1, 3] {
                let (x, z) = step_to(cx, cz, dx, dz);
                if near_package(x, z) || !board.bare(x, z) {
                    cells.clear();
                    break 'grid;
                }
                cells.push((x, z));
            }
        }
        if cells.len() == 8 {
            for (x, z) in cells {
                board.feature(x, z, G_VIA);
            }
            made += 1;
        }
    }
    made
}

/// A bundle that changes layer drops its tracks into a staggered row of vias at the end of
/// the run: each track runs a little further and terminates on a via.
fn emit_via_fanout(board: &mut Board, rng: &mut StdRng, buses: &[Vec<Vec<(usize, usize)>>]) -> usize {
    let mut made = 0usize;
    let mut order: Vec<usize> = (0..buses.len()).collect();
    order.shuffle(rng);
    for &b in &order {
        if made == 6 {
            break;
        }
        let mut ok = true;
        let mut pending: Vec<(usize, usize)> = Vec::new();
        for track in &buses[b] {
            let (last, prev) = match (track.last(), track.get(track.len().saturating_sub(2))) {
                (Some(&l), Some(&p)) => (l, p),
                _ => {
                    ok = false;
                    break;
                }
            };
            let di = step_dir(prev, last);
            let extra = rng.random_range(1..=3);
            let mut cur = last;
            for _ in 0..extra {
                let (dx, dz) = DIR_STEP[di];
                let next = step_to(cur.0, cur.1, dx, dz);
                if in_package(next.0, next.1)
                    || board.blocked[flat(next.0, next.1)]
                    || near_package(next.0, next.1)
                    || board.at(next.0, next.1)[0] != 0
                {
                    break;
                }
                // Extend the track one more cell, then the end lands on a via.
                board.mark(cur.0, cur.1, di, GAUGE_THIN, 0, 210);
                cur = next;
            }
            if cur != last {
                pending.push(cur);
            }
        }
        if !ok || pending.is_empty() {
            continue;
        }
        for (x, z) in pending {
            board.feature(x, z, G_VIA);
        }
        made += 1;
    }
    made
}

// ---------------------------------------------------------------------------
// Finishing passes
// ---------------------------------------------------------------------------

/// Every trace end left in the open gets a feature: a pad, or a via. This is what stops
/// runs from being left dangling in mid-air - after this pass a cell holding exactly one
/// half-segment is always a pin, a pad, a via or a mounting hole.
fn terminate_dead_ends(board: &mut Board, rng: &mut StdRng) {
    for z in 0..SIZE {
        for x in 0..SIZE {
            let c = board.at(x, z);
            if c[0].count_ones() != 1 || c[1] != 0 {
                continue;
            }
            let g = if near_package(x, z) || rng.random_bool(0.65) {
                G_PAD
            } else {
                G_VIA
            };
            board.feature(x, z, g);
        }
    }
}

/// Silkscreen rings around the mounting holes, placed only on cells that carry no copper
/// so the marks stay legible. (Package courtyards were printed with the packages.)
fn emit_silkscreen(board: &mut Board, holes: &[(usize, usize)]) {
    for &(hx, hz) in holes {
        for (dx, dz) in [(2i32, 0i32), (-2, 0), (0, 2), (0, -2)] {
            let (x, z) = step_to(hx, hz, dx, dz);
            let c = board.at(x, z);
            if c[0] != 0 || (c[1] != 0 && c[1] != G_POUR) {
                continue;
            }
            board.feature(x, z, G_SILK);
            board.grid[flat(x, z)][3] = A_SILK;
        }
    }
}

/// True when a pour may flood this cell: bare substrate whose whole Chebyshev-1
/// neighbourhood is free of copper, so the plane keeps the one-cell clearance moat a real
/// ground plane keeps around every trace and land.
fn pour_fillable(board: &Board, x: usize, z: usize) -> bool {
    if board.blocked[flat(x, z)] {
        return false;
    }
    let c = board.at(x, z);
    if c[0] != 0 || c[1] != 0 {
        return false;
    }
    // Clearance channel on the four sides: at 2.5 units per cell a real etch gap is far
    // under one cell, but one cell of separation is what keeps the plane reading as a
    // plane instead of merging with the traces it flows around.
    [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)].iter().all(|&(mx, mz)| {
        let (nx, nz) = step_to(x, z, mx, mz);
        !board.solid(nx, nz)
    })
}

/// How much copper `pour_region` would flood over a rectangle, without flooding it.
fn pour_count(board: &Board, x0: usize, z0: usize, w: usize, h: usize) -> usize {
    let mut n = 0usize;
    for dz in 0..h {
        for dx in 0..w {
            if pour_fillable(board, (x0 + dx) % SIZE, (z0 + dz) % SIZE) {
                n += 1;
            }
        }
    }
    n
}

/// Flood one ground plane over a rectangle: the hatched flag only changes how the
/// renderer draws the fill.
fn pour_region(
    board: &mut Board,
    x0: usize,
    z0: usize,
    w: usize,
    h: usize,
    hatched: bool,
    base: u8,
) -> usize {
    let mut filled = 0usize;
    for dz in 0..h {
        for dx in 0..w {
            let (x, z) = ((x0 + dx) % SIZE, (z0 + dz) % SIZE);
            if !pour_fillable(board, x, z) {
                continue;
            }
            let c = &mut board.grid[flat(x, z)];
            c[1] = G_POUR;
            c[2] |= if hatched { B_HATCH } else { 0 };
            c[3] = base;
            filled += 1;
        }
    }
    filled
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// A generated tile plus the packaging and net plan it was built from, so tests can
/// assert on the routing itself rather than reverse-engineering it from the cells.
struct Plan {
    map: FloorMap,
    /// The package plan, kept for tests and callers that want the land pattern.
    #[allow(dead_code)]
    packages: Vec<TowerPkg>,
    /// The net plan, kept for tests and callers that want the routing.
    #[allow(dead_code)]
    nets: Vec<Net>,
}

/// Build the whole tile: packages, mounting holes, plane reservations, buses, rails, nets,
/// via stitching and fanouts, dead-end termination, copper pours and silkscreen.
fn generate_planned(seed: u64) -> Plan {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut board = Board::new();

    // IC packages first: everything else is routed around them.
    let packages = emit_tower_packages(&mut board);
    for pkg in &packages {
        assert!(
            in_package(pkg.centre.0, pkg.centre.1),
            "package centre must sit inside its own package"
        );
    }
    let holes = emit_mounting_holes(&mut board);

    // Reserve the ground-plane areas before routing: nets mostly route around them.
    for &(x0, z0) in &PLANE_RESERVES {
        for dz in 0..RESERVE_H {
            for dx in 0..RESERVE_W {
                let (x, z) = ((x0 + dx) % SIZE, (z0 + dz) % SIZE);
                board.plane[flat(x, z)] = true;
            }
        }
    }

    // Stitching via arrays are part of the board's drill pattern; place them before the
    // bundles so the bundles keep the clear lanes they need.
    emit_via_arrays(&mut board, &mut rng);

    // Bus bundles, then the power ring that crosses them.
    let mut buses: Vec<Vec<Vec<(usize, usize)>>> = Vec::new();
    while buses.len() < BUS_COUNT {
        match emit_bus(&mut board, &mut rng) {
            Some(paths) => buses.push(paths),
            None => break,
        }
    }
    emit_rails(&mut board, &mut rng);

    // Nets: every pin escapes outward, then every port routes to a neighbouring tower's
    // port, so the dominant copper is chip-to-chip wiring.
    let mut nets = plan_nets(&mut rng, &packages);
    emit_nets(&mut board, &mut rng, &packages, &mut nets);

    // Layer changes, termination, silkscreen, then the ground planes.
    emit_via_fanout(&mut board, &mut rng, &buses);
    terminate_dead_ends(&mut board, &mut rng);

    // Ground planes: flood the three emptiest windows the finished routing left, so the
    // copper lands where a plane can actually be seen - in the negative space around the
    // routes rather than across them. The board is dense now, so the windows are found
    // with a cheap coarse sweep over the whole tile and then refined to the cell, rather
    // than being locked to a fixed rectangle. Alternating regions come out hatched, then
    // the silkscreen goes on top of everything.
    for i in 0..PLANE_RESERVES.len() {
        let mut best = ((0usize, 0usize), 0usize);
        for z0 in (0..SIZE).step_by(4) {
            for x0 in (0..SIZE).step_by(4) {
                let n = pour_count(&board, x0, z0, POUR_W, POUR_H);
                if n > best.1 {
                    best = ((x0, z0), n);
                }
            }
        }
        let (bx, bz) = best.0;
        for dz in -3i32..=3 {
            for dx in -3i32..=3 {
                let (x0, z0) = step_to(bx, bz, dx, dz);
                let n = pour_count(&board, x0, z0, POUR_W, POUR_H);
                if n > best.1 {
                    best = ((x0, z0), n);
                }
            }
        }
        let base: u8 = rng.random_range(142..=168);
        pour_region(&mut board, best.0 .0, best.0 .1, POUR_W, POUR_H, i % 2 == 1, base);
    }
    emit_silkscreen(&mut board, &holes);

    Plan {
        map: FloorMap {
            cells: FLOOR_TILE_CELLS,
            data: board.grid,
        },
        packages,
        nets,
    }
}

/// Generate the floor map deterministically from `seed`.
///
/// Returns a `FLOOR_TILE_CELLS x FLOOR_TILE_CELLS` toroidal tile laid out like a routed
/// printed circuit board whose components are the towers: an IC package under every tower
/// (a 5 x 5 body, 12 pins on the ring, a silkscreen courtyard and a pin-1 dot), tower-to-
/// tower octilinear nets between neighbouring packages, parallel bus bundles with
/// 45-degree corners down the lanes, thick power rails on a supply ring, via stitching and
/// fanouts, and copper pours with clearance moats.
pub fn generate(seed: u64) -> FloorMap {
    generate_planned(seed).map
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The package plan alone: package geometry does not depend on the seed.
    fn packages() -> Vec<TowerPkg> {
        let mut board = Board::new();
        emit_tower_packages(&mut board)
    }

    /// All package cells: every `(cx + dx, cz + dz)` with `|dx|, |dz| <= PKG_HALF`.
    fn package_cells() -> Vec<(usize, usize)> {
        let mut cells = Vec::with_capacity(
            TOWERS_PER_SIDE * TOWERS_PER_SIDE * (2 * PKG_HALF as usize + 1).pow(2),
        );
        for kz in 0..TOWERS_PER_SIDE {
            for kx in 0..TOWERS_PER_SIDE {
                let cx = FIRST_COL_CENTER + kx * CELLS_PER_TOWER;
                let cz = kz * CELLS_PER_TOWER;
                for dz in -PKG_HALF..=PKG_HALF {
                    for dx in -PKG_HALF..=PKG_HALF {
                        cells.push(step_to(cx, cz, dx, dz));
                    }
                }
            }
        }
        cells
    }

    /// Cells carrying a trace half-segment.
    fn traced(f: &FloorMap) -> usize {
        f.data.iter().filter(|c| c[0] != 0).count()
    }

    /// Cells carrying a diagonal half-segment.
    fn diagonal(f: &FloorMap) -> usize {
        f.data.iter().filter(|c| c[0] & 0b1111_0000 != 0).count()
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
        let mut kinds = [0usize; 9];
        for c in &f.data {
            assert!(
                (A_EMPTY..=255).contains(&c[3]),
                "brightness A out of 128..=255: {}",
                c[3]
            );
            assert_eq!(c[2] & 0b1111_0000, 0, "reserved B bits must stay zero: {:#010b}", c[2]);
            assert!(c[1] <= G_HOLE, "ground feature G out of 0..=8: {}", c[1]);
            assert!(
                !(c[1] == G_POUR && c[0] != 0),
                "a pour cell must be fill, not a trace segment"
            );
            kinds[c[1] as usize] += 1;
        }
        // Every feature kind the tile is supposed to carry is actually present, and the
        // two structural kinds have exactly the package counts the layout implies.
        for (g, what) in [
            (G_PAD, "through-hole pads"),
            (G_VIA, "vias"),
            (G_IC, "IC bodies"),
            (G_PIN, "IC pins"),
            (G_POUR, "copper pour"),
            (G_SILK, "silkscreen"),
            (G_HOLE, "mounting holes"),
        ] {
            assert!(kinds[g as usize] > 0, "no {what} on the tile");
        }
        let towers = TOWERS_PER_SIDE * TOWERS_PER_SIDE;
        assert_eq!(kinds[G_IC as usize], towers * 25, "one 5 x 5 body per tower");
        assert_eq!(kinds[G_PIN as usize], towers * 12, "twelve pins per tower");
        assert_eq!(kinds[G_HOLE as usize], 4, "one mounting hole per board quarter");
    }

    // --- The packages are the ICs: body, pin ring, courtyard, pin-1 marker. ---

    #[test]
    fn tower_packages_are_well_formed() {
        let f = generate(11);
        let pkgs = packages();
        assert_eq!(pkgs.len(), TOWERS_PER_SIDE * TOWERS_PER_SIDE, "one package per tower");

        let mut body = 0usize;
        let mut pins = 0usize;
        for pkg in &pkgs {
            let (cx, cz) = pkg.centre;
            for dz in -BODY_HALF..=BODY_HALF {
                for dx in -BODY_HALF..=BODY_HALF {
                    let (x, z) = step_to(cx, cz, dx, dz);
                    let c = f.data[flat(x, z)];
                    assert_eq!(c[1], G_IC, "({x}, {z}) must be an IC body cell");
                    assert_eq!(c[0], 0, "no trace may run through a body cell ({x}, {z})");
                    body += 1;
                }
            }
            // Exactly the 12 pins sit on the ring, at the -2/0/+2 positions of each side;
            // every other ring cell (gaps and corners) carries courtyard silk and no copper.
            let mut expected: Vec<(i32, i32)> = Vec::new();
            for &o in &PIN_OFFSETS {
                expected.push((PKG_HALF, o));
                expected.push((-PKG_HALF, o));
                expected.push((o, PKG_HALF));
                expected.push((o, -PKG_HALF));
            }
            for dz in -PKG_HALF..=PKG_HALF {
                for dx in -PKG_HALF..=PKG_HALF {
                    if dx.abs() <= BODY_HALF && dz.abs() <= BODY_HALF {
                        continue;
                    }
                    let (x, z) = step_to(cx, cz, dx, dz);
                    let c = f.data[flat(x, z)];
                    if expected.contains(&(dx, dz)) {
                        assert_eq!(c[1], G_PIN, "({dx}, {dz}) must be a pin");
                        assert_eq!(c[0].count_ones(), 1, "a pin carries its one escape bit");
                        pins += 1;
                    } else {
                        assert_ne!(c[1], G_PIN, "({dx}, {dz}) must not be a pin");
                        assert_eq!(c[1], G_SILK, "ring cell ({dx}, {dz}) is courtyard silk");
                        assert_eq!(c[0], 0, "courtyard silk carries no copper");
                    }
                }
            }
            // Pin-1 marker just outside the -x/-z corner.
            let (mx, mz) = step_to(cx, cz, -(PKG_HALF + 1), -(PKG_HALF + 1));
            assert_eq!(f.data[flat(mx, mz)][1], G_SILK, "missing pin-1 marker");
            assert_eq!(f.data[flat(mx, mz)][0], 0, "pin-1 marker must stay clear of copper");
            assert_eq!(f.data[flat(mx, mz)][3], A_SILK_PIN1, "pin-1 marker brightness");
        }
        assert_eq!(body, pkgs.len() * 25, "every package carries a 5 x 5 body");
        assert_eq!(pins, pkgs.len() * 12, "every package carries exactly 12 pins");
    }

    // --- Acceptance 2: every pin leaves its package outward, onto its port. ---

    #[test]
    fn every_pin_escapes_outward() {
        let f = generate(11);
        let pkgs = packages();
        let mut unconnected = Vec::new();
        for pkg in &pkgs {
            for pin in &pkg.pins {
                let c = f.data[flat(pin.cell.0, pin.cell.1)];
                let outward = DIR_BITS[pin.dir];
                assert_eq!(c[1], G_PIN, "pin {:?} lost its land", pin.cell);
                if c[0] != outward {
                    unconnected.push((pin.cell, c[0], outward));
                }
                // The port carries the mirror half-segment, so the escape is continuous
                // and the pin is not a false dangling end.
                let (mirror, _, _) = floor_dir_mirror(outward).expect("single direction bit");
                assert_ne!(
                    f.data[flat(pin.port.0, pin.port.1)][0] & mirror,
                    0,
                    "port {:?} misses the mirror of the pin escape",
                    pin.port
                );
            }
        }
        assert!(
            unconnected.is_empty(),
            "{} of {} pins carry no outward escape, e.g. {:?}",
            unconnected.len(),
            pkgs.len() * 12,
            &unconnected[..unconnected.len().min(4)]
        );
    }

    // --- Acceptance 3: no trace ends in mid-air. ---

    #[test]
    fn no_trace_ends_in_mid_air() {
        for seed in [0u64, 1, 11, 42] {
            let f = generate(seed);
            let mut dangling: Vec<(usize, usize, u8)> = Vec::new();
            let mut singles = [0usize; 9];
            for z in 0..SIZE {
                for x in 0..SIZE {
                    let c = f.data[flat(x, z)];
                    if c[0].count_ones() != 1 {
                        continue;
                    }
                    singles[c[1] as usize] += 1;
                    if !matches!(c[1], G_PIN | G_PAD | G_VIA | G_HOLE) {
                        dangling.push((x, z, c[1]));
                    }
                }
            }
            assert!(
                dangling.is_empty(),
                "seed {seed}: {} trace ends in mid-air, e.g. {:?}",
                dangling.len(),
                &dangling[..dangling.len().min(4)]
            );
            assert!(singles[G_PIN as usize] > 0 && singles[G_PAD as usize] > 0);
            assert!(singles[G_VIA as usize] > 0, "no net ever dropped to a via");
            println!(
                "seed {seed}: single-segment cells - pin {}, pad {}, via {}, hole {}",
                singles[G_PIN as usize],
                singles[G_PAD as usize],
                singles[G_VIA as usize],
                singles[G_HOLE as usize]
            );
        }
    }

    // --- Acceptance 4: nets run tower to tower, mostly to nearby towers. ---

    #[test]
    fn nets_join_neighbouring_towers() {
        let plan = generate_planned(11);
        let mut cross = 0usize;
        let mut lengths: Vec<usize> = Vec::new();
        for net in &plan.nets {
            let Some(len) = net.len else {
                continue;
            };
            if tower_of(net.from.0, net.from.1) != tower_of(net.to.0, net.to.1) {
                cross += 1;
                lengths.push(len);
            }
        }
        println!(
            "seed 11: {} nets planned, {cross} join two different towers",
            plan.nets.len()
        );
        assert!(
            cross >= plan.nets.len() * 3 / 4,
            "only {cross} of {} nets join two different towers",
            plan.nets.len()
        );
        lengths.sort_unstable();
        let n = lengths.len();
        let mut buckets = [0usize; 6];
        for &l in &lengths {
            buckets[(l / 4).min(5)] += 1;
        }
        println!(
            "net length in cells: min {}, median {}, max {}; histogram (4-cell buckets) {:?}",
            lengths[0],
            lengths[n / 2],
            lengths[n - 1],
            buckets
        );
        assert!(lengths[n - 1] <= NET_LEN_CAP, "a net exceeded the route cap");
        assert!(lengths[n / 2] <= 16, "median net is not a local chip-to-chip run");
    }

    // --- Continuity, on the torus and across diagonals. ---

    #[test]
    fn continuity_across_torus_seam() {
        let f = generate(7);
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
    fn diagonal_continuity_holds() {
        let mut worst = 0u64;
        for seed in [0u64, 1, 7, 11, 42, 99] {
            let f = generate(seed);
            let mut violations = 0u64;
            for z in 0..SIZE {
                for x in 0..SIZE {
                    let bits = f.data[flat(x, z)][0];
                    for &bit in &[BIT_DXPZ, BIT_DXNZP, BIT_DXPZN, BIT_DXNZN] {
                        if bits & bit == 0 {
                            continue;
                        }
                        let (mirror, dx, dz) =
                            floor_dir_mirror(bit).expect("single direction bit");
                        let (nx, nz) = step_to(x, z, dx, dz);
                        if f.data[flat(nx, nz)][0] & mirror == 0 {
                            violations += 1;
                        }
                    }
                }
            }
            worst = worst.max(violations);
        }
        assert_eq!(worst, 0, "diagonal half-segments without a mirror bit");
    }

    // --- The lane lattice is phase-correct on both axes. ---

    #[test]
    fn lane_lattice_matches_the_tower_grid() {
        // Tower columns sit at x = 6 (mod 12) and tower rows at z = 0 (mod 12), so the two
        // axes are half a pitch out of phase: x lanes are centred on the columns' gaps at
        // x = 0 (mod 12), z lanes at z = 6 (mod 12). Every cell of a lane - centre,
        // shoulders and edges - is free substrate, and one cell further out is a package,
        // which is what makes the lane exactly 2 * LANE_HALF + 1 cells wide.
        for m in 0..TOWERS_PER_SIDE {
            assert_eq!(lane_x(m) % CELLS_PER_TOWER, 0, "x lane phase");
            assert_eq!(
                lane_z(m) % CELLS_PER_TOWER,
                FIRST_COL_CENTER % CELLS_PER_TOWER,
                "z lane phase"
            );
            for s in 0..SIZE {
                for off in -(LANE_HALF as i32)..=(LANE_HALF as i32) {
                    let (ax, az) = step_to(lane_x(m), s, off, 0);
                    let (bx, bz) = step_to(s, lane_z(m), 0, off);
                    assert!(!in_package(ax, az), "x lane cell ({ax}, {az}) is in a package");
                    assert!(!in_package(bx, bz), "z lane cell ({bx}, {bz}) is in a package");
                }
                let edge = (LANE_HALF + 1) as i32;
                let (ex, ez) = step_to(lane_x(m), 0, edge, 0);
                assert!(in_package(ex, ez), "x lane is wider than the gap");
                let (ex, ez) = step_to(FIRST_COL_CENTER, lane_z(m), 0, edge);
                assert!(in_package(ex, ez), "z lane is wider than the gap");
                let (ex, ez) = step_to(lane_x(m), 0, -edge, 0);
                assert!(in_package(ex, ez), "x lane is wider than the gap");
                let (ex, ez) = step_to(FIRST_COL_CENTER, lane_z(m), 0, -edge);
                assert!(in_package(ex, ez), "z lane is wider than the gap");
            }
        }
    }

    // --- A regression back to pure Manhattan routing must fail here. ---

    #[test]
    fn diagonal_share_of_routing_is_meaningful() {
        let mut report = Vec::new();
        for seed in [0u64, 1, 7, 11, 42, 99] {
            let f = generate(seed);
            let traced = traced(&f);
            let diagonal = diagonal(&f);
            let share = 100.0 * diagonal as f64 / traced as f64;
            report.push(format!("seed {seed}: {diagonal}/{traced} = {share:.1}%"));
            assert!(
                share >= 20.0,
                "only {share:.1}% of traced cells carry a 45-degree half-segment \
                 ({diagonal} diagonal of {traced}); routing has fallen back towards Manhattan"
            );
        }
        println!("diagonal share: {}", report.join(", "));
    }

    // --- All three width classes are used and the thick class forms real runs. ---

    #[test]
    fn gauge_classes_are_used() {
        let f = generate(11);
        let mut count = [0usize; 3];
        let mut power = vec![false; SIZE * SIZE];
        let mut bus = 0usize;
        for z in 0..SIZE {
            for x in 0..SIZE {
                let c = f.data[flat(x, z)];
                if c[0] == 0 {
                    continue;
                }
                count[(c[2] & GAUGE_MASK) as usize] += 1;
                if c[2] & GAUGE_MASK == GAUGE_POWER {
                    power[flat(x, z)] = true;
                }
                if c[2] & B_BUS != 0 {
                    bus += 1;
                }
            }
        }
        for (i, n) in count.iter().enumerate() {
            assert!(*n > 0, "gauge class {i} never used");
        }
        assert!(bus > 200, "only {bus} cells carry the bus-bundle flag");
        // Power/ground copper is routed as runs, not as isolated cells: flood the class
        // over 8-connected neighbours and check the components look like traces.
        let mut seen = vec![false; SIZE * SIZE];
        let mut sizes: Vec<usize> = Vec::new();
        for z in 0..SIZE {
            for x in 0..SIZE {
                if !power[flat(x, z)] || seen[flat(x, z)] {
                    continue;
                }
                let mut stack = vec![(x, z)];
                seen[flat(x, z)] = true;
                let mut size = 0usize;
                while let Some((cx, cz)) = stack.pop() {
                    size += 1;
                    for dz in -1i32..=1 {
                        for dx in -1i32..=1 {
                            let (nx, nz) = step_to(cx, cz, dx, dz);
                            if power[flat(nx, nz)] && !seen[flat(nx, nz)] {
                                seen[flat(nx, nz)] = true;
                                stack.push((nx, nz));
                            }
                        }
                    }
                }
                sizes.push(size);
            }
        }
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        let biggest = sizes.first().copied().unwrap_or(0);
        println!(
            "power/ground: {} cells in {} runs, longest {biggest}",
            count[GAUGE_POWER as usize],
            sizes.len()
        );
        assert!(
            biggest >= 150,
            "the thickest class does not form long runs: longest run {biggest} cells"
        );
        let singles = sizes.iter().filter(|s| **s == 1).count();
        assert!(
            singles * 20 <= sizes.len(),
            "{singles} of {} power runs are single isolated cells",
            sizes.len()
        );
    }

    #[test]
    fn floor_coverage_exceeds_fifteen_percent() {
        let f = generate(11);
        let traced = traced(&f);
        assert!(
            traced * 100 >= 15 * SIZE * SIZE,
            "only {traced}/{SIZE} cells carry a trace segment"
        );
    }

    #[test]
    fn tower_grid_is_world_aligned() {
        // The package lattice must match the frozen world layout: towers every
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
        // 7x7 packages around every centre, spaced 12 apart, never overlap: exactly
        // 64 * 49 = 3136 distinct package cells on the tile, each carrying its feature.
        let mut cells = package_cells();
        cells.sort_unstable();
        cells.dedup();
        assert_eq!(
            cells.len(),
            TOWERS_PER_SIDE * TOWERS_PER_SIDE * (2 * PKG_HALF as usize + 1).pow(2),
            "packages must not overlap"
        );
        let f = generate(11);
        for (x, z) in cells {
            let c = f.data[flat(x, z)];
            assert!(
                matches!(c[1], G_IC | G_PIN | G_SILK),
                "package cell ({x}, {z}) has kind {}",
                c[1]
            );
            assert!(
                (A_SILK..=A_FEATURE).contains(&c[3]),
                "package cell ({x}, {z}) has brightness {}",
                c[3]
            );
        }
    }

    #[test]
    fn continuity_and_coverage_over_routable_area() {
        let f = generate(11);
        // Continuity across the torus (mirror bits, diagonals included), re-checked on
        // this seed.
        let mut violations = 0u64;
        for z in 0..SIZE {
            for x in 0..SIZE {
                let bits = f.data[flat(x, z)][0];
                for i in 0..8 {
                    let bit = DIR_BITS[i];
                    if bits & bit == 0 {
                        continue;
                    }
                    let (mirror, dx, dz) = floor_dir_mirror(bit).expect("single direction bit");
                    let (nx, nz) = step_to(x, z, dx, dz);
                    if f.data[flat(nx, nz)][0] & mirror == 0 {
                        violations += 1;
                    }
                }
            }
        }
        assert_eq!(violations, 0);
        // Coverage overall (9216 cells) and over just the routable (non-package)
        // area, which the renderer shows as substrate between the tower packages.
        let mut traced_overall = 0u64;
        let mut routable = 0u64;
        let mut traced_routable = 0u64;
        let mut poured = 0u64;
        for z in 0..SIZE {
            for x in 0..SIZE {
                let inside = in_package(x, z);
                if !inside {
                    routable += 1;
                }
                let c = f.data[flat(x, z)];
                if c[1] == G_POUR {
                    poured += 1;
                }
                if c[0] != 0 {
                    traced_overall += 1;
                    if !inside {
                        traced_routable += 1;
                    }
                }
            }
        }
        let total = (SIZE * SIZE) as u64;
        let package_total =
            (TOWERS_PER_SIDE * TOWERS_PER_SIDE * (2 * PKG_HALF as usize + 1).pow(2)) as u64;
        assert_eq!(
            routable,
            total - package_total,
            "routable area must be the complement of the packages"
        );
        let pct_overall = 100.0 * traced_overall as f64 / total as f64;
        let pct_routable = 100.0 * traced_routable as f64 / routable as f64;
        println!(
            "seed 11: {traced_overall}/{total} cells carry traces ({pct_overall:.1}%), \
             {traced_routable}/{routable} routable cells ({pct_routable:.1}%), \
             {poured}/{routable} routable cells are copper pour ({:.1}%)",
            100.0 * poured as f64 / routable as f64
        );
        assert!(
            traced_overall * 100 >= 15 * total,
            "only {traced_overall}/{total} cells carry a trace segment"
        );
        assert!(
            pct_routable >= 20.0,
            "routable-area coverage {pct_routable:.1}% is too sparse for a bus layout"
        );
        assert!(poured > 200, "the ground planes are missing: {poured} pour cells");
    }
}
