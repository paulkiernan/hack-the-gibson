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
//! (`|dx| <= 3`, `|dz| <= 3` around its centre) that stays completely empty, so the
//! renderer shows a clean dark margin of bare substrate around every tower base and
//! traces terminate on land pads at the boundary instead of being clipped.
//!
//! Between the keep-out squares the free substrate forms a lattice of five-cell-wide
//! lanes ([`LANE_HALF`]). The two axes are half a pitch out of phase, because the tower
//! columns sit at world `x = 15 + 30k` while the rows sit at `z = 30k`: x lanes are
//! centred on `x = 0 (mod 12)` ([`lane_x`]) and z lanes on `z = 6 (mod 12)` ([`lane_z`]).
//! That lattice is what makes the tile read as a routed board rather than a maze -
//! components sit on the lanes, bus bundles run down their centres, the power rails run
//! the full length of lanes 0 and 4 in both axes, and everything else is routed
//! octilinearly between them.
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
//!   pour, 7 silkscreen, 8 mounting hole.
//! - B: bits 0-1 trace width class (0 thin signal, 1 medium, 2 thick power/ground),
//!   bit 2 set on cells that belong to a parallel bus bundle, bit 3 set on hatched
//!   pours. Bits 4-7 stay zero.
//! - A: brightness, 128..=255 (128 bare substrate).
//!
//! Contents, in placement order (each step only ever adds copper):
//! 1. Footprints: mounting holes at the four corners of the board's 48-cell quarter, DIP
//!    packages (two pin rows 3 cells apart on a 2-cell pitch with the body between the
//!    rows), QFP packages (3 x 3 body with pins on all four sides) and rows of two-pad SMD
//!    discretes. Every
//!    footprint keeps a ring of clearance so traces can escape its pins, and no footprint
//!    ever overlaps another or a keep-out square.
//! 2. Tower land pads: one through-hole pad on each of the four tower-margin edges - the
//!    lands the tower bases sit on - plus stitching via arrays in the mid-lane pockets.
//! 3. Bus bundles: 2-3 tracks at one gauge running together down a lane centre and
//!    turning together at lane intersections with a 45-degree bus corner (each track
//!    turns through its own diagonal jog, so the bundle stays parallel through the turn).
//!    Bus cells carry the bundle flag, and a bundle passes over through-hole tower lands
//!    and is cut short where a package land pattern blocks the lane.
//! 4. Power rails: thick (gauge 2) runs down lanes 0 and 4 in both axes, crossing at four
//!    points into a single connected supply ring, breaking around package bodies rather
//!    than cutting through them.
//! 5. Nets, routed the way a board is: every pin escapes to the nearest lane, bundle,
//!    rail or tower land with a short fanout ([`MAX_ESCAPE_LEN`]), the bundles carry
//!    signals across the board, and a handful of longer point-to-point nets tie distant
//!    parts together. The reserved ground-plane regions carry a routing charge, so nets
//!    cross a plane only when they must.
//! 6. Via fanouts: a bundle that changes layer drops its tracks into a staggered row of
//!    vias.
//! 7. Termination: any trace end left in the open becomes a pad, or a via away from the
//!    tower margins, so no copper is left dangling.
//! 8. Copper pours: the emptiest rectangles the finished routing left are flooded with
//!    ground-plane fill ([`POUR_W`] x [`POUR_H`] each), with a one-cell clearance channel
//!    around every trace, land and footprint - the negative space that makes the routing
//!    read as a board. Alternating regions come out hatched.
//! 9. Silkscreen: dotted courtyard outlines around every package, pin-1 notches, corner
//!    ticks on the discretes and rings around the mounting holes, printed last so the
//!    marks also sit on the planes, never on a trace.

use gibson_types::{floor_dir_mirror, FloorMap, FLOOR_TILE_CELLS, FLOOR_TILE_UNITS, TOWER_PITCH};
use rand::rngs::StdRng;
use rand::seq::{IndexedRandom, SliceRandom};
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

/// Keep-out zone half-width in cells around each tower centre: zones are
/// `2 * ZONE_HALF + 1 = 7` cells square, a footprint (12 world units = 4.8 cells)
/// plus a clean margin.
const ZONE_HALF: usize = 3;

/// Half-width of the fully-free routing lane between neighbouring keep-out squares:
/// `12 - 7 = 5` cells wide, so a lane centre sits 2 cells clear of both margins.
const LANE_HALF: usize = (CELLS_PER_TOWER - (2 * ZONE_HALF + 1)) / 2;

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
const G_SMD: u8 = 4;
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

// Router costs. Straight and diagonal steps cost the same per unit distance; the turn
// charges are what make a 45-degree jog (two 45s) cheaper than a right angle (one 90).
const STEP_STRAIGHT: u32 = 10;
const STEP_DIAG: u32 = 15;
const TURN_45: u32 = 6;
const TURN_90: u32 = 14;
const TURN_SHARP: u32 = 28;
/// Extra charge for stepping onto a cell that already carries a trace: crossings are
/// allowed (this is a multi-layer composite view) but a router prefers not to.
const COST_CROSS_TRACE: u32 = 45;
/// Extra charge for stepping through a pad, via or pin that is not the destination.
const COST_CROSS_FEATURE: u32 = 65;
/// Extra charge for routing across a reserved ground-plane region: nets cross a plane when
/// they must, but they prefer the lanes beside it.
const COST_CROSS_PLANE: u32 = 80;

// Content budget: how much board the tile carries, tuned so roughly a third of the tile
// carries copper and the rest is substrate, pour and tower shadow.
const DIP8_COUNT: usize = 4;
const DIP16_COUNT: usize = 2;
const QFP_COUNT: usize = 6;
const DISCRETE_COUNT: usize = 15;
const BUS_COUNT: usize = 10;
/// Pins on a DIP side, and lands of a discrete, are 2 cells apart (0.2 inch at 2.5
/// units per cell).
const PIN_PITCH: usize = 2;
/// Lane indices carrying thick power/ground rails, in both axes.
const RAIL_LANES: [usize; 2] = [0, 4];
/// Long point-to-point nets tying distant parts of the board together, on top of the
/// per-pin escapes.
const LONG_NETS: usize = 18;
/// Tower land pads that also get a feed trace into the lane.
const FEED_BUDGET: usize = 40;
/// Ground-plane reservations: the top-left corner of each of the three rectangles the
/// router is charged for crossing, so the nets mostly skirt them the way signals route
/// around a plane rather than straight over it. Each reserve is a little larger than the
/// flood rectangle, so the flood can pick the emptiest window inside its own reserve.
const PLANE_RESERVES: [(usize, usize); 3] = [(12, 18), (60, 18), (36, 54)];
const RESERVE_W: usize = POUR_W + 6;
const RESERVE_H: usize = POUR_H + 4;
const POUR_W: usize = 38;
const POUR_H: usize = 24;
/// Length caps. An escape route only has to reach the nearest lane, so it is short by
/// construction; a point-to-point net may cross a good part of the board, but anything
/// longer than the cap is a walk rather than a route and is dropped.
const MAX_ESCAPE_LEN: usize = 42;
const MAX_NET_LEN: usize = 72;
/// Stitching via arrays dropped into lane intersections.
const VIA_ARRAYS: usize = 4;

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

/// Minimum toroidal distance in cells from cell `v` to the nearest point of the form
/// `first + k * step` on the `SIZE`-ring (tower centres repeat every `step` cells).
#[inline]
fn ring_dist(v: usize, first: usize, step: usize) -> usize {
    let d = (v as i64 - first as i64).rem_euclid(step as i64) as usize;
    d.min(step - d)
}

/// True when cell `(x, z)` lies inside one of the 7 x 7 keep-out squares sitting,
/// grid-exactly, over the 8 x 8 tower footprints. Zones repeat every 12 cells along each
/// axis and never overlap (spacing 12 > 7).
#[inline]
fn keep_out(x: usize, z: usize) -> bool {
    ring_dist(x, FIRST_COL_CENTER, CELLS_PER_TOWER) <= ZONE_HALF
        && ring_dist(z, 0, CELLS_PER_TOWER) <= ZONE_HALF
}

/// True when `(x, z)` is a free cell orthogonally adjacent to a keep-out square - the
/// tower-margin ring where terminating runs must become pads.
#[inline]
fn near_keep_out(x: usize, z: usize) -> bool {
    DIR_STEP[0..4].iter().any(|(dx, dz)| {
        let (nx, nz) = step_to(x, z, *dx, *dz);
        keep_out(nx, nz)
    })
}

/// Centre cell of routing lane `m` along x. Tower columns sit at world `x = 15 + 30k`
/// (cell `6 mod 12`), so the five-cell lane between two zone columns is centred on
/// `x = 0 mod 12`.
#[inline]
fn lane_x(m: usize) -> usize {
    (m * CELLS_PER_TOWER) % SIZE
}

/// Centre cell of routing lane `m` along z. Tower rows sit at world `z = 30k` (cell
/// `0 mod 12`), so the z lanes are half a pitch out of phase with the x lanes and centred
/// on `z = 6 mod 12`. Getting this phase right is what lets a package sit on a lane with
/// its pins and body clear of both neighbouring keep-out squares.
#[inline]
fn lane_z(m: usize) -> usize {
    (FIRST_COL_CENTER + m * CELLS_PER_TOWER) % SIZE
}

/// The lane really is the gap between two keep-out squares: the geometry the whole
/// layout leans on.
const _: () = assert!(2 * LANE_HALF + 1 == CELLS_PER_TOWER - 2 * ZONE_HALF - 1);

/// Toroidal Chebyshev distance between two cells.
#[inline]
fn cell_dist(a: (usize, usize), b: (usize, usize)) -> usize {
    let dx = {
        let d = (a.0 as i64 - b.0 as i64).rem_euclid(SIZE as i64) as usize;
        d.min(SIZE - d)
    };
    let dz = {
        let d = (a.1 as i64 - b.1 as i64).rem_euclid(SIZE as i64) as usize;
        d.min(SIZE - d)
    };
    dx.max(dz)
}

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

/// In-progress tile: one `[R, G, B, A]` byte quad per cell, plus the routing barrier map.
struct Board {
    grid: Vec<[u8; 4]>,
    /// Cells the router must never enter: IC bodies and mounting holes.
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
    /// still bare substrate. Keep-out cells are never painted.
    #[inline]
    fn paint(&mut self, x: usize, z: usize, bits: u8, gauge: u8, flags: u8, base: u8) {
        if keep_out(x, z) {
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
    /// Callers guarantee neither endpoint lies inside a keep-out square.
    fn mark(&mut self, x: usize, z: usize, di: usize, gauge: u8, flags: u8, base: u8) {
        let (dx, dz) = DIR_STEP[di];
        let bit = DIR_BITS[di];
        let (mirror, _, _) = floor_dir_mirror(bit).expect("single direction bit");
        let (nx, nz) = step_to(x, z, dx, dz);
        self.paint(x, z, bit, gauge, flags, base);
        self.paint(nx, nz, mirror, gauge, flags, base);
    }

    /// Stamp a whole routed polyline cell by cell.
    fn stamp(&mut self, path: &[(usize, usize)], gauge: u8, flags: u8, base: u8) {
        for pair in path.windows(2) {
            let di = step_dir(pair[0], pair[1]);
            self.mark(pair[0].0, pair[0].1, di, gauge, flags, base);
        }
    }

    /// Stamp a path in runs, skipping cells the router may not enter, so a long rail can
    /// break around a package body instead of cutting straight through it.
    fn stamp_runs(&mut self, path: &[(usize, usize)], gauge: u8, flags: u8, base: u8) {
        let mut run: Vec<(usize, usize)> = Vec::new();
        for &c in path {
            if keep_out(c.0, c.1) || self.blocked[flat(c.0, c.1)] {
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

    /// Place a copper feature (pad, via, IC body, SMD land, pin, hole).
    fn feature(&mut self, x: usize, z: usize, g: u8) {
        if keep_out(x, z) {
            return;
        }
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
    /// vias, pins, SMD lands, package bodies, holes or silkscreen. Pour fill itself does
    /// not count, so a region can flood continuously around its own moats.
    #[inline]
    fn solid(&self, x: usize, z: usize) -> bool {
        let c = self.grid[flat(x, z)];
        c[0] != 0 || (c[1] != 0 && c[1] != G_POUR)
    }
}

// ---------------------------------------------------------------------------
// Footprints
// ---------------------------------------------------------------------------

/// Package family of a placed footprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Two pin rows on a fixed pitch with the body between them.
    Dip,
    /// Square body with pins on all four sides.
    Qfp,
    /// Two-pad SMD land (resistor / capacitor).
    Discrete,
    /// Mounting hole.
    Mounting,
}

/// One placed component land pattern, in tile-wrapped cell coordinates.
#[derive(Clone, Debug)]
struct Footprint {
    kind: Kind,
    /// Package centre (QFP, mounting hole) or the first body cell (DIP), first land
    /// (discrete): the reference point the land pattern is built from.
    anchor: (usize, usize),
    /// Package body cells (`G_IC`); empty for discretes and mounting holes.
    body: Vec<(usize, usize)>,
    /// IC pin cells (`G_PIN`), in package order around the body.
    pins: Vec<(usize, usize)>,
    /// SMD land cells (`G_SMD`) of a two-pad discrete.
    pads: Vec<(usize, usize)>,
    /// Mounting hole cell (`G_HOLE`).
    hole: Option<(usize, usize)>,
    /// Pin pitch in cells within a row or side.
    pitch: usize,
    /// Pins per row (DIP) or per side (QFP).
    per_side: usize,
}

impl Footprint {
    /// Every cell the footprint occupies, body and copper alike.
    fn cells(&self) -> Vec<(usize, usize)> {
        let mut v = Vec::new();
        v.extend_from_slice(&self.body);
        v.extend_from_slice(&self.pins);
        v.extend_from_slice(&self.pads);
        v.extend(self.hole);
        v
    }

    /// A DIP package: `per_side` pins per row on [`PIN_PITCH`], the second row `pitch - 1`
    /// cells further along z, body filling the two rows between them.
    fn dip(x0: usize, zc: usize, per_side: usize) -> Self {
        let span = (per_side - 1) * PIN_PITCH;
        let mut body = Vec::with_capacity(2 * (span + 1));
        let mut pins = Vec::with_capacity(2 * per_side);
        for i in 0..per_side {
            let x = (x0 + i * PIN_PITCH) % SIZE;
            pins.push((x, (zc + SIZE - PIN_PITCH) % SIZE));
            pins.push((x, (zc + PIN_PITCH - 1) % SIZE));
        }
        for dz in 0..2 {
            for dx in 0..=span {
                body.push(((x0 + dx) % SIZE, (zc + SIZE - 1 + dz) % SIZE));
            }
        }
        Self {
            kind: Kind::Dip,
            anchor: (x0, zc),
            body,
            pins,
            pads: Vec::new(),
            hole: None,
            pitch: PIN_PITCH,
            per_side,
        }
    }

    /// A QFP package: `3 x 3` body with `per_side` pins on each of the four sides, the
    /// sides `2` cells out from the body centre and pins `pitch` apart within a side.
    /// (This generator places the two-pin-per-side variant, `per_side == 2`.)
    fn qfp(cx: usize, cz: usize, per_side: usize) -> Self {
        let mut body = Vec::with_capacity(9);
        let mut pins = Vec::with_capacity(4 * per_side);
        for dz in -1i32..=1 {
            for dx in -1i32..=1 {
                body.push(step_to(cx, cz, dx, dz));
            }
        }
        let along: Vec<i32> = match per_side {
            1 => vec![0],
            2 => vec![-1, 1],
            _ => vec![-2, 0, 2],
        };
        for &a in &along {
            pins.push(step_to(cx, cz, 2, a));
            pins.push(step_to(cx, cz, -2, a));
            pins.push(step_to(cx, cz, a, 2));
            pins.push(step_to(cx, cz, a, -2));
        }
        let per_side = along.len();
        Self {
            kind: Kind::Qfp,
            anchor: (cx, cz),
            body,
            pins,
            pads: Vec::new(),
            hole: None,
            pitch: if along.len() > 1 { PIN_PITCH } else { 0 },
            per_side,
        }
    }

    /// A two-pad SMD discrete laid along one axis: lands `pitch` cells apart.
    fn discrete(x: usize, z: usize, horizontal: bool) -> Self {
        let (x1, z1) = if horizontal {
            ((x + PIN_PITCH) % SIZE, z)
        } else {
            (x, (z + PIN_PITCH) % SIZE)
        };
        Self {
            kind: Kind::Discrete,
            anchor: (x, z),
            body: Vec::new(),
            pins: Vec::new(),
            pads: vec![(x, z), (x1, z1)],
            hole: None,
            pitch: PIN_PITCH,
            per_side: 1,
        }
    }

    /// A mounting hole.
    fn mounting(x: usize, z: usize) -> Self {
        Self {
            kind: Kind::Mounting,
            anchor: (x, z),
            body: Vec::new(),
            pins: Vec::new(),
            pads: Vec::new(),
            hole: Some((x, z)),
            pitch: 0,
            per_side: 0,
        }
    }
}

/// True when every cell of `cells` is inside the board's free area (not keep-out) and no
/// cell or its Chebyshev-1 ring is already reserved, so footprints never touch.
fn fits(occ: &[bool], cells: &[(usize, usize)]) -> bool {
    cells.iter().all(|&(x, z)| {
        if keep_out(x, z) {
            return false;
        }
        (-1i32..=1).all(|dz| {
            (-1i32..=1).all(|dx| {
                let (nx, nz) = step_to(x, z, dx, dz);
                !occ[flat(nx, nz)]
            })
        })
    })
}

/// Reserve `cells` and their Chebyshev-1 ring against later placement.
fn reserve(occ: &mut [bool], cells: &[(usize, usize)]) {
    for &(x, z) in cells {
        for dz in -1i32..=1 {
            for dx in -1i32..=1 {
                let (nx, nz) = step_to(x, z, dx, dz);
                occ[flat(nx, nz)] = true;
            }
        }
    }
}

/// Lay out every component on the board: mounting holes, DIP packages along lanes, QFP
/// packages on lane intersections and rows of two-pad discretes. Every candidate anchor is
/// rejected unless the whole footprint fits with its clearance ring, so no footprint
/// overlaps another or a tower keep-out square.
fn place_footprints(rng: &mut StdRng) -> Vec<Footprint> {
    let mut occ = vec![false; SIZE * SIZE];
    let mut out: Vec<Footprint> = Vec::new();

    // Mounting holes: one at each corner of the board's 48-cell quarter, on lane
    // intersections the power rails do not run through. Nothing is placed yet, so they
    // always fit; the lane lattice guarantees their cells are free substrate.
    for (a, b) in [(2usize, 2usize), (6, 2), (2, 6), (6, 6)] {
        let f = Footprint::mounting(lane_x(a), lane_z(b));
        assert!(fits(&occ, &f.cells()), "mounting hole does not fit the lane lattice");
        reserve(&mut occ, &f.cells());
        out.push(f);
    }

    // DIP packages run along a lane row: pins sit on the lane's two spare lines and the
    // body fills the rows between them.
    for (per_side, count) in [(4usize, DIP8_COUNT), (8, DIP16_COUNT)] {
        let mut placed = 0usize;
        for _ in 0..512 {
            if placed >= count {
                break;
            }
            let m = rng.random_range(0..TOWERS_PER_SIDE);
            let x0 = rng.random_range(0..SIZE);
            let f = Footprint::dip(x0, lane_z(m), per_side);
            if fits(&occ, &f.cells()) {
                reserve(&mut occ, &f.cells());
                out.push(f);
                placed += 1;
            }
        }
    }

    // QFP packages sit on lane intersections, leaving a one-cell escape ring all round.
    let mut qfps = 0usize;
    for _ in 0..1024 {
        if qfps >= QFP_COUNT {
            break;
        }
        let a = rng.random_range(0..TOWERS_PER_SIDE);
        let b = rng.random_range(0..TOWERS_PER_SIDE);
        let f = Footprint::qfp(lane_x(a), lane_z(b), 2);
        if fits(&occ, &f.cells()) {
            reserve(&mut occ, &f.cells());
            out.push(f);
            qfps += 1;
        }
    }

    // Discretes go down in rows of one to three, a fixed 5 cells apart along a lane.
    let mut discretes = 0usize;
    for _ in 0..2048 {
        if discretes >= DISCRETE_COUNT {
            break;
        }
        let horizontal = rng.random_bool(0.5);
        let m = rng.random_range(0..TOWERS_PER_SIDE);
        let along = rng.random_range(0..SIZE);
        let row = rng.random_range(1..=3);
        for i in 0..row {
            if discretes >= DISCRETE_COUNT {
                break;
            }
            let p = (along + i * 5) % SIZE;
            let f = if horizontal {
                Footprint::discrete(p, lane_z(m), true)
            } else {
                Footprint::discrete(lane_x(m), p, false)
            };
            if fits(&occ, &f.cells()) {
                reserve(&mut occ, &f.cells());
                out.push(f.clone());
                discretes += 1;
            }
        }
    }

    out
}

/// Paint a footprint's cells onto the board and block the router from its solid parts.
fn emit_footprint(board: &mut Board, fp: &Footprint) {
    for &(x, z) in &fp.body {
        board.feature(x, z, G_IC);
        board.blocked[flat(x, z)] = true;
    }
    for &(x, z) in &fp.pins {
        board.feature(x, z, G_PIN);
    }
    for &(x, z) in &fp.pads {
        board.feature(x, z, G_SMD);
    }
    if let Some((x, z)) = fp.hole {
        board.feature(x, z, G_HOLE);
        board.blocked[flat(x, z)] = true;
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
/// turning together at lane intersections, with a 45-degree bus corner at each turn.
/// Returns the bundle's track paths when one was placed.
fn emit_bus(board: &mut Board, rng: &mut StdRng) -> Option<Vec<Vec<(usize, usize)>>> {
    for _ in 0..32 {
        let tracks = rng.random_range(2..=3);
        let offsets: Vec<i32> = if tracks == 2 { vec![-1, 1] } else { vec![-2, 0, 2] };
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
        // A bundle runs on clear copper: bare substrate, or a through-hole land the bundle
        // passes over (tower land pads sit on the outer track lines). It keeps the longest
        // window all its tracks share, and the cut ends terminate on pads like a real
        // bundle leaving a package.
        let len = paths.iter().map(|p| p.len()).min().unwrap_or(0);
        let (mut best_start, mut best_len) = (0usize, 0usize);
        let mut start = 0usize;
        for i in 0..len {
            let clear = paths.iter().all(|p| {
                let (x, z) = p[i];
                !keep_out(x, z)
                    && !board.blocked[flat(x, z)]
                    && (board.bare(x, z) || board.at(x, z)[1] == G_PAD)
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
    if start == goal || board.blocked[goal] || keep_out(to.0, to.1) {
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
            if keep_out(nx, nz) || board.blocked[flat(nx, nz)] {
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

/// Stamp one net: route from `from` to `to` and paint it. Returns whether it routed.
fn route_net(
    board: &mut Board,
    from: (usize, usize),
    to: (usize, usize),
    gauge: u8,
    base: u8,
    max_len: usize,
) -> bool {
    match route(board, from, to) {
        Some(path) if path.len() <= max_len => {
            board.stamp(&path, gauge, 0, base);
            true
        }
        _ => false,
    }
}

/// Nearest cell of `pool` to `from` at a toroidal distance in `lo..=hi`: how a pin finds
/// the closest lane, bundle, rail or tower land to escape onto.
fn nearest_target(
    from: (usize, usize),
    pool: &[(usize, usize)],
    lo: usize,
    hi: usize,
) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    let mut best_d = usize::MAX;
    for &c in pool {
        let d = cell_dist(from, c);
        if d >= lo && d <= hi && d < best_d {
            best_d = d;
            best = Some(c);
        }
    }
    best
}

/// Guarantee a pin is not left floating: route a short stub out of it, preferring a bare
/// neighbour, and fall back to any neighbour the router may enter at all.
fn force_stub(board: &mut Board, pin: (usize, usize), base: u8) {
    if board.at(pin.0, pin.1)[0] != 0 {
        return;
    }
    let mut best: Option<usize> = None;
    for di in 0..8 {
        let (dx, dz) = DIR_STEP[di];
        let (nx, nz) = step_to(pin.0, pin.1, dx, dz);
        if keep_out(nx, nz) || board.blocked[flat(nx, nz)] {
            continue;
        }
        if board.at(nx, nz)[0] == 0 {
            best = Some(di);
            break;
        }
        if best.is_none() {
            best = Some(di);
        }
    }
    if let Some(di) = best {
        board.mark(pin.0, pin.1, di, GAUGE_THIN, 0, base);
    }
}

// ---------------------------------------------------------------------------
// Tower land pads, power rails, vias
// ---------------------------------------------------------------------------

/// Every tower gets a through-hole land pad on each of its four margin edges: the lands
/// the tower bases stand on, and the pads traces terminate at instead of being clipped by
/// the keep-out square.
fn place_margin_pads(board: &mut Board) -> Vec<(usize, usize)> {
    let mut pads = Vec::new();
    for kz in 0..TOWERS_PER_SIDE {
        for kx in 0..TOWERS_PER_SIDE {
            let cx = FIRST_COL_CENTER + kx * CELLS_PER_TOWER;
            let cz = kz * CELLS_PER_TOWER;
            for (dx, dz) in [
                (0i32, -(ZONE_HALF as i32) - 1),
                (0, ZONE_HALF as i32 + 1),
                (-(ZONE_HALF as i32) - 1, 0),
                (ZONE_HALF as i32 + 1, 0),
            ] {
                let (x, z) = step_to(cx, cz, dx, dz);
                if keep_out(x, z) || !board.bare(x, z) {
                    continue;
                }
                board.feature(x, z, G_PAD);
                pads.push((x, z));
            }
        }
    }
    pads
}

/// Thick power/ground rails: one run down each axis through lane 0 and lane 4, wrapping,
/// so the four rails form a single connected supply ring across the tile. Runs break
/// around package bodies rather than cutting through them.
fn emit_rails(board: &mut Board, rng: &mut StdRng) -> Vec<(usize, usize)> {
    let mut cells = Vec::new();
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
            cells.extend(path);
        }
    }
    cells
}

/// Via arrays: small grids of stitching vias dropped into the board's mid-lane pockets,
/// the plated-through drill pattern real boards carry all over their ground planes. They
/// sit half a pitch off the lane intersections, clear of the bundle lines.
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
                if keep_out(x, z) || near_keep_out(x, z) || !board.bare(x, z) {
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
                if keep_out(next.0, next.1)
                    || board.blocked[flat(next.0, next.1)]
                    || near_keep_out(next.0, next.1)
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

/// Every trace end left in the open gets a feature: a pad against a tower margin (never a
/// via there) or, elsewhere, a pad or a via. This is what stops runs from being clipped.
fn terminate_dead_ends(board: &mut Board, rng: &mut StdRng) {
    for z in 0..SIZE {
        for x in 0..SIZE {
            let c = board.at(x, z);
            if c[0].count_ones() != 1 || c[1] != 0 {
                continue;
            }
            let g = if near_keep_out(x, z) || rng.random_bool(0.65) {
                G_PAD
            } else {
                G_VIA
            };
            board.feature(x, z, g);
        }
    }
}

/// Silkscreen: pin-1 notches, package corner ticks, discrete body ticks and mounting hole
/// rings, placed only on cells that carry no copper so the marks stay legible.
fn emit_silkscreen(board: &mut Board, rng: &mut StdRng, fps: &[Footprint]) {
    let base: u8 = rng.random_range(205..=245);
    let mut marks: Vec<(usize, usize)> = Vec::new();
    for fp in fps {
        match fp.kind {
            // Courtyard outline: a dotted line along both long sides, just outside the
            // pin rows, closed with the pin-1 notch at the left end.
            Kind::Dip => {
                let (x0, zc) = fp.anchor;
                let span = (fp.per_side - 1) * fp.pitch;
                for dx in 0..=(span + 2) {
                    let x = (x0 + SIZE - 1 + dx) % SIZE;
                    marks.push((x, (zc + SIZE - 3) % SIZE));
                    marks.push((x, (zc + 4) % SIZE));
                }
                for dz in 0..3 {
                    let z = (zc + SIZE - 1 + dz) % SIZE;
                    marks.push(((x0 + SIZE - 1) % SIZE, z));
                    marks.push(((x0 + span + 1) % SIZE, z));
                }
            }
            // Courtyard outline on all four sides plus the package corner ticks.
            Kind::Qfp => {
                let (cx, cz) = fp.anchor;
                for (dx, dz) in [(2i32, 2i32), (-2, -2), (2, -2), (-2, 2)] {
                    marks.push(step_to(cx, cz, dx, dz));
                }
                for a in -2i32..=2 {
                    marks.push(step_to(cx, cz, 3, a));
                    marks.push(step_to(cx, cz, -3, a));
                    marks.push(step_to(cx, cz, a, 3));
                    marks.push(step_to(cx, cz, a, -3));
                }
            }
            // Body ticks either side of the gap between the two lands, and one at each
            // outer end of the land pair.
            Kind::Discrete => {
                let (x, z) = fp.anchor;
                let (x1, z1) = fp.pads[1];
                if z == z1 {
                    let mx = (x + delta(x, x1).unsigned_abs() as usize / 2) % SIZE;
                    marks.push((mx, (z + SIZE - 1) % SIZE));
                    marks.push((mx, (z + 1) % SIZE));
                    marks.push(((x + SIZE - 1) % SIZE, z));
                    marks.push(((x1 + 1) % SIZE, z));
                } else {
                    let mz = (z + delta(z, z1).unsigned_abs() as usize / 2) % SIZE;
                    marks.push(((x + SIZE - 1) % SIZE, mz));
                    marks.push(((x + 1) % SIZE, mz));
                    marks.push((x, (z + SIZE - 1) % SIZE));
                    marks.push((x, (z1 + 1) % SIZE));
                }
            }
            // A ring of silk around each mounting hole.
            Kind::Mounting => {
                let (hx, hz) = fp.anchor;
                for (dx, dz) in [(2i32, 0i32), (-2, 0), (0, 2), (0, -2)] {
                    marks.push(step_to(hx, hz, dx, dz));
                }
            }
        }
    }
    for (x, z) in marks {
        if keep_out(x, z) {
            continue;
        }
        let c = board.at(x, z);
        if c[0] != 0 || (c[1] != 0 && c[1] != G_POUR) {
            continue;
        }
        board.feature(x, z, G_SILK);
        board.grid[flat(x, z)][3] = base;
    }
}

/// True when a pour may flood this cell: bare substrate whose whole Chebyshev-1
/// neighbourhood is free of copper, so the plane keeps the one-cell clearance moat a real
/// ground plane keeps around every trace and land.
fn pour_fillable(board: &Board, x: usize, z: usize) -> bool {
    if keep_out(x, z) || board.blocked[flat(x, z)] {
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

/// Generate the floor map deterministically from `seed`.
///
/// Returns a `FLOOR_TILE_CELLS x FLOOR_TILE_CELLS` toroidal tile laid out like a routed
/// printed circuit board: component footprints on the lane lattice, parallel bus bundles
/// with 45-degree corners, thick power rails on a supply ring, octilinear signal routing,
/// via fanouts, tower land pads on every keep-out margin, silkscreen marks and copper
/// pours with clearance moats - with every tower footprint's 7 x 7-cell keep-out square
/// left completely empty and the tile wrapped seamlessly on every edge.
pub fn generate(seed: u64) -> FloorMap {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut board = Board::new();

    // Components first: everything else is routed around them.
    let fps = place_footprints(&mut rng);
    for fp in &fps {
        emit_footprint(&mut board, fp);
    }

    // Reserve the ground-plane areas before routing: nets mostly route around them.
    for &(x0, z0) in &PLANE_RESERVES {
        for dz in 0..RESERVE_H {
            for dx in 0..RESERVE_W {
                let (x, z) = ((x0 + dx) % SIZE, (z0 + dz) % SIZE);
                board.plane[flat(x, z)] = true;
            }
        }
    }

    // Tower land pads before routing, so nets can terminate on them, and the stitching
    // via arrays that are part of the board's drill pattern.
    let pads = place_margin_pads(&mut board);
    emit_via_arrays(&mut board, &mut rng);

    // Bus bundles, then the power ring that crosses them.
    let mut buses: Vec<Vec<Vec<(usize, usize)>>> = Vec::new();
    while buses.len() < BUS_COUNT {
        match emit_bus(&mut board, &mut rng) {
            Some(paths) => buses.push(paths),
            None => break,
        }
    }
    let rail_cells = emit_rails(&mut board, &mut rng);

    // Nets, the way a board is actually routed: every pin escapes to the nearest lane,
    // bundle, rail or tower land (short fanout), the bundles carry signals across the
    // board, and a handful of longer point-to-point nets tie distant parts together.
    let pins: Vec<(usize, usize)> = fps
        .iter()
        .flat_map(|f| f.pins.iter().chain(f.pads.iter()).copied())
        .collect();
    let mut lanes: Vec<(usize, usize)> = Vec::new();
    for m in 0..TOWERS_PER_SIDE {
        for s in (0..SIZE).step_by(3) {
            lanes.push((lane_x(m), s));
            lanes.push((s, lane_z(m)));
        }
    }
    let mut pool: Vec<(usize, usize)> = lanes.clone();
    pool.extend(buses.iter().flatten().flatten().copied());
    pool.extend(rail_cells.iter().copied());
    pool.extend(pads.iter().copied());

    let mut endpoints = pins.clone();
    endpoints.shuffle(&mut rng);
    for &pin in &endpoints {
        if board.at(pin.0, pin.1)[0] != 0 {
            continue; // already reached by an earlier net
        }
        let Some(target) = nearest_target(pin, &pool, 4, MAX_ESCAPE_LEN) else {
            continue;
        };
        let gauge = if rail_cells.contains(&target) {
            GAUGE_POWER
        } else {
            GAUGE_THIN
        };
        let base: u8 = rng.random_range(185..=255);
        route_net(&mut board, pin, target, gauge, base, MAX_ESCAPE_LEN);
    }
    // A few long nets across the board, at the medium gauge.
    let mut tied = 0usize;
    for _ in 0..(LONG_NETS * 6) {
        if tied == LONG_NETS {
            break;
        }
        let a = *endpoints.choose(&mut rng).unwrap();
        let Some(b) = nearest_target(a, &pool, 60, MAX_NET_LEN) else {
            continue;
        };
        let base: u8 = rng.random_range(180..=240);
        if route_net(&mut board, a, b, GAUGE_MED, base, MAX_NET_LEN) {
            tied += 1;
        }
    }
    // Every pin must leave its package, even where the router found no corridor.
    for &pin in &pins {
        force_stub(&mut board, pin, 205);
    }

    // Feeds from the tower land pads into the lanes.
    let mut feeds = 0usize;
    let mut feed_order = pads.clone();
    feed_order.shuffle(&mut rng);
    for &pad in &feed_order {
        if feeds == FEED_BUDGET {
            break;
        }
        let Some(target) = nearest_target(pad, &pool, 8, 34) else {
            continue;
        };
        let base: u8 = rng.random_range(180..=230);
        if route_net(&mut board, pad, target, GAUGE_MED, base, MAX_NET_LEN) {
            feeds += 1;
        }
    }

    // Layer changes, termination, silkscreen, then the ground planes.
    emit_via_fanout(&mut board, &mut rng, &buses);
    terminate_dead_ends(&mut board, &mut rng);

    // Ground planes: flood the emptiest rectangles the finished routing left, so the
    // copper lands where a plane can actually be seen - around the routes rather than
    // across them. Alternating regions come out hatched, then the silkscreen goes on top
    // of everything, the way a real board prints its marks over the copper.
    for (i, &(rx, rz)) in PLANE_RESERVES.iter().enumerate() {
        let mut best = ((rx, rz), 0usize);
        for dz in 0..=(RESERVE_H - POUR_H) {
            for dx in 0..=(RESERVE_W - POUR_W) {
                let a = ((rx + dx) % SIZE, (rz + dz) % SIZE);
                let n = pour_count(&board, a.0, a.1, POUR_W, POUR_H);
                if n > best.1 {
                    best = (a, n);
                }
            }
        }
        let base: u8 = rng.random_range(142..=168);
        pour_region(&mut board, best.0 .0, best.0 .1, POUR_W, POUR_H, i % 2 == 1, base);
    }
    emit_silkscreen(&mut board, &mut rng, &fps);

    FloorMap {
        cells: FLOOR_TILE_CELLS,
        data: board.grid,
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

    /// The layout `generate(seed)` uses, reproduced from the same seed so the tests can
    /// assert on package geometry.
    fn layout(seed: u64) -> Vec<Footprint> {
        let mut rng = StdRng::seed_from_u64(seed);
        place_footprints(&mut rng)
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
        // Every feature kind the tile is supposed to carry is actually present.
        for (g, what) in [
            (G_PAD, "through-hole pads"),
            (G_VIA, "vias"),
            (G_IC, "IC bodies"),
            (G_SMD, "SMD lands"),
            (G_PIN, "IC pins"),
            (G_POUR, "copper pour"),
            (G_SILK, "silkscreen"),
            (G_HOLE, "mounting holes"),
        ] {
            assert!(kinds[g as usize] > 0, "no {what} on the tile");
        }
    }

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

    // --- New: diagonal routing is continuous across the torus as well. ---

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

    // --- New: the lane lattice is phase-correct on both axes. ---

    #[test]
    fn lane_lattice_matches_the_tower_grid() {
        // Tower columns sit at x = 6 (mod 12) and tower rows at z = 0 (mod 12), so the two
        // axes are half a pitch out of phase: x lanes are centred on the columns' gaps at
        // x = 0 (mod 12), z lanes at z = 6 (mod 12). Every cell of a lane - centre,
        // shoulders and edges - is free substrate, and one cell further out is a keep-out
        // square, which is what makes the lane exactly 2 * LANE_HALF + 1 cells wide.
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
                    assert!(!keep_out(ax, az), "x lane cell ({ax}, {az}) is in a keep-out");
                    assert!(!keep_out(bx, bz), "z lane cell ({bx}, {bz}) is in a keep-out");
                }
                let edge = (LANE_HALF + 1) as i32;
                let (ex, ez) = step_to(lane_x(m), 0, edge, 0);
                assert!(keep_out(ex, ez), "x lane is wider than the gap");
                let (ex, ez) = step_to(FIRST_COL_CENTER, lane_z(m), 0, edge);
                assert!(keep_out(ex, ez), "z lane is wider than the gap");
                let (ex, ez) = step_to(lane_x(m), 0, -edge, 0);
                assert!(keep_out(ex, ez), "x lane is wider than the gap");
                let (ex, ez) = step_to(FIRST_COL_CENTER, lane_z(m), 0, -edge);
                assert!(keep_out(ex, ez), "z lane is wider than the gap");
            }
        }
    }

    // --- New: a regression back to pure Manhattan routing must fail here. ---

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
                 ({} diagonal of {traced}); routing has fallen back towards Manhattan",
                diagonal
            );
        }
        println!("diagonal share: {}", report.join(", "));
    }

    // --- New: the packages are real land patterns, and every pin is connected. ---

    #[test]
    fn footprints_are_well_formed() {
        let f = generate(11);
        let mut seen: Vec<(usize, usize)> = Vec::new();
        let mut dips = 0usize;
        let mut qfps = 0usize;
        let mut discretes = 0usize;
        let mut holes = 0usize;
        for fp in &layout(11) {
            // No footprint may touch a tower keep-out square or another footprint.
            for (x, z) in fp.cells() {
                assert!(!keep_out(x, z), "footprint cell inside a keep-out square");
                assert!(
                    !seen.contains(&(x, z)),
                    "two footprints share cell ({x}, {z})"
                );
                seen.push((x, z));
            }
            match fp.kind {
                Kind::Dip => {
                    dips += 1;
                    assert!(
                        fp.per_side == 4 || fp.per_side == 8,
                        "DIP with {} pins per row",
                        fp.per_side
                    );
                    assert_eq!(fp.pins.len(), 2 * fp.per_side);
                    assert_eq!(fp.pitch, PIN_PITCH);
                    assert_eq!(
                        fp.body.len(),
                        2 * ((fp.per_side - 1) * fp.pitch + 1),
                        "DIP body fills the rows between the pin rows"
                    );
                    let (x0, _) = fp.anchor;
                    let mut rows: Vec<usize> = fp.pins.iter().map(|p| p.1).collect();
                    rows.sort_unstable();
                    rows.dedup();
                    assert_eq!(rows.len(), 2, "a DIP has exactly two pin rows");
                    assert_eq!(
                        rows[1] - rows[0],
                        PIN_PITCH + 1,
                        "DIP row spacing: the 0.3 inch package width, 3 cells across"
                    );
                    for &row in &rows {
                        let mut expect: Vec<usize> = (0..fp.per_side)
                            .map(|i| (x0 + i * fp.pitch) % SIZE)
                            .collect();
                        expect.sort_unstable();
                        let mut got: Vec<usize> = fp
                            .pins
                            .iter()
                            .filter(|p| p.1 == row)
                            .map(|p| p.0)
                            .collect();
                        got.sort_unstable();
                        assert_eq!(got, expect, "DIP pin positions in a row");
                    }
                }
                Kind::Qfp => {
                    qfps += 1;
                    assert_eq!(fp.per_side, 2, "QFP pins per side");
                    assert_eq!(fp.pins.len(), 8, "a QFP has pins on all four sides");
                    assert_eq!(fp.body.len(), 9, "QFP body is 3 x 3");
                    assert_eq!(fp.pitch, PIN_PITCH);
                    let (cx, cz) = fp.anchor;
                    for (dx, dz) in [(2i32, 0i32), (-2, 0), (0, 2), (0, -2)] {
                        let mut side: Vec<(usize, usize)> = fp
                            .pins
                            .iter()
                            .copied()
                            .filter(|&(x, z)| {
                                let (ox, oz) = (delta(cx, x), delta(cz, z));
                                if dx == 0 {
                                    oz == dz && ox.abs() == 1
                                } else {
                                    ox == dx && oz.abs() == 1
                                }
                            })
                            .collect();
                        assert_eq!(side.len(), 2, "QFP side ({dx}, {dz}) pin count");
                        side.sort_unstable();
                        let spacing = if dx == 0 {
                            delta(side[0].0, side[1].0).abs()
                        } else {
                            delta(side[0].1, side[1].1).abs()
                        };
                        assert_eq!(spacing as usize, fp.pitch, "QFP pin pitch within a side");
                    }
                }
                Kind::Discrete => {
                    discretes += 1;
                    assert_eq!(fp.pads.len(), 2, "a discrete has two lands");
                    let d = cell_dist(fp.pads[0], fp.pads[1]);
                    assert_eq!(d, fp.pitch, "discrete land spacing");
                }
                Kind::Mounting => holes += 1,
            }
        }
        assert_eq!(dips, DIP8_COUNT + DIP16_COUNT, "DIP packages placed");
        assert_eq!(qfps, QFP_COUNT, "QFP packages placed");
        assert!(discretes >= DISCRETE_COUNT, "discretes placed: {discretes}");
        assert_eq!(holes, 4, "one mounting hole per board quarter");

        // Every pin, and every discrete land, is connected to at least one trace
        // segment - no land is left floating on the substrate.
        let mut unconnected = Vec::new();
        for fp in &layout(11) {
            for (x, z) in fp.pins.iter().chain(fp.pads.iter()) {
                if f.data[flat(*x, *z)][0] == 0 {
                    unconnected.push((*x, *z));
                }
            }
        }
        assert!(
            unconnected.is_empty(),
            "{} lands carry no trace segment, e.g. {:?}",
            unconnected.len(),
            &unconnected[..unconnected.len().min(4)]
        );
    }

    // --- New: all three width classes are used and the thick class forms real runs. ---

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

    #[test]
    fn traces_terminate_at_keep_out_margins() {
        let f = generate(11);
        let mut into_zone = 0u64;
        let mut pads_on_margin = 0u64;
        let mut vias_on_margin = 0u64;
        let mut dead_end_on_margin = 0u64;
        let mut unterminated: Vec<(usize, usize)> = Vec::new();
        let mut landed_on_feature: Vec<u8> = Vec::new();
        for z in 0..SIZE {
            for x in 0..SIZE {
                let c = f.data[flat(x, z)];
                for (bit, dx, dz) in [
                    (BIT_PX, 1i32, 0i32),
                    (BIT_NX, -1, 0),
                    (BIT_PZ, 0, 1),
                    (BIT_NZ, 0, -1),
                    (BIT_DXPZ, 1, 1),
                    (BIT_DXNZP, -1, 1),
                    (BIT_DXPZN, 1, -1),
                    (BIT_DXNZN, -1, -1),
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
                            // land on something - a pad, a land, or a package pin. Only a
                            // dead end on bare substrate is an unterminated trace.
                            let bits = c[0];
                            if bits != 0 && bits & (bits - 1) == 0 {
                                if c[1] == 0 {
                                    dead_end_on_margin += 1;
                                    unterminated.push((x, z));
                                } else {
                                    landed_on_feature.push(c[1]);
                                }
                            }
                        }
                    }
                }
            }
        }
        println!(
            "seed 11: {pads_on_margin} pads on tower margins, {vias_on_margin} vias on \
             margins, {} ends landing on a land or pin, {unterminated:?}",
            landed_on_feature.len()
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
        // Coverage overall (9216 cells) and over just the routable (non-keep-out)
        // area, which the renderer shows as substrate between the tower zones.
        let mut traced_overall = 0u64;
        let mut routable = 0u64;
        let mut traced_routable = 0u64;
        let mut poured = 0u64;
        for z in 0..SIZE {
            for x in 0..SIZE {
                let inside = keep_out(x, z);
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
