//! Lane pulses: the white-cyan streaks that run along the streets between tower rows.
//!
//! Type A runs along x on a lane `z = 15 + 30*m` (between tower rows at `z = 30k`); type B runs
//! along z on a lane `x = 30*m` (between tower columns at `x = 15 + 30k`). Lane coordinates are
//! exact f32 arithmetic (`30*m`, `15 + 30*m`) so instances land precisely on the street grid.

use gibson_types::PulseInstance;
use rand::{Rng, SeedableRng, rngs::StdRng};

/// Stable salt separating the pulse pool's RNG stream from the other subsystems.
const PULSE_SALT: u64 = 0x3C6E_F372_FE94_F82B;

/// Mutable pulse state; the renderer sees the derived `PulseInstance`.
struct Pulse {
    /// True: travels along x at a fixed z lane. False: travels along z at a fixed x lane.
    along_x: bool,
    /// Lane index (see module docs for the two lane families).
    m: i32,
    /// Head position along the travel axis (x for `along_x`, z otherwise).
    pos: f32,
    /// Height above the floor.
    y: f32,
    /// Travel speed in units/second.
    speed: f32,
    /// +1/-1 along the positive axis.
    dir: f32,
    /// Ribbon length in world units.
    length: f32,
    /// Brightness.
    intensity: f32,
}

/// A pool of lane-confined pulses, resized to the settings count and animated per frame.
pub(crate) struct PulsePool {
    rng: StdRng,
    grid: u32,
    half: i32,
    pulses: Vec<Pulse>,
}

impl PulsePool {
    /// Fresh pool (no pulses yet) seeded for the given city seed.
    pub(crate) fn new(seed: u64) -> PulsePool {
        PulsePool {
            rng: StdRng::seed_from_u64(seed ^ PULSE_SALT),
            grid: 60,
            half: 30,
            pulses: Vec::new(),
        }
    }

    /// Adopt the city grid. Changing the grid re-spawns every pulse on lanes of the new extent.
    pub(crate) fn set_grid(&mut self, grid: u32) {
        if grid == self.grid {
            return;
        }
        self.grid = grid;
        self.half = (grid / 2) as i32;
        for i in 0..self.pulses.len() {
            let fresh = self.make_pulse();
            self.pulses[i] = fresh;
        }
    }

    /// Grow or shrink the pool to `count` lanes; only newly added pulses draw from the stream.
    pub(crate) fn resize(&mut self, count: usize) {
        if count > self.pulses.len() {
            self.pulses.reserve(count - self.pulses.len());
            while self.pulses.len() < count {
                let fresh = self.make_pulse();
                self.pulses.push(fresh);
            }
        } else {
            self.pulses.truncate(count);
        }
    }

    /// Half-extent bound for pulse travel: `grid/2 * 30 + 60` units from the origin.
    fn extent(&self) -> f32 {
        self.half as f32 * 30.0 + 60.0
    }

    /// Draw a fresh random pulse (all fields).
    fn make_pulse(&mut self) -> Pulse {
        let half = self.half;
        let extent = self.extent();
        Pulse {
            along_x: self.rng.random::<f32>() < 0.5,
            m: self.rng.random_range(-half..=half),
            pos: self.rng.random_range(-extent..extent),
            y: self.rng.random_range(0.6..2.5),
            speed: self.rng.random_range(120.0..200.0),
            dir: if self.rng.random::<f32>() < 0.5 { 1.0 } else { -1.0 },
            length: self.rng.random_range(8.0..16.0),
            intensity: self.rng.random_range(0.7..1.0),
        }
    }

    /// Advance every pulse by `dt` seconds; a pulse whose head leaves the grid extent respawns.
    pub(crate) fn advance(&mut self, dt: f32) {
        if dt <= 0.0 || self.pulses.is_empty() {
            return;
        }
        let extent = self.extent();
        for p in &mut self.pulses {
            p.pos += p.dir * p.speed * dt;
        }
        for i in 0..self.pulses.len() {
            if self.pulses[i].pos.abs() > extent {
                let fresh = self.make_pulse();
                self.pulses[i] = fresh;
            }
        }
    }

    /// Append this frame's `PulseInstance`s to `out`.
    pub(crate) fn write_instances(&self, out: &mut Vec<PulseInstance>) {
        for p in &self.pulses {
            let (x, z) = if p.along_x {
                (p.pos, 15.0 + 30.0 * p.m as f32)
            } else {
                (30.0 * p.m as f32, p.pos)
            };
            out.push(PulseInstance {
                position: [x, p.y, z],
                length: p.length,
                direction: if p.along_x {
                    [p.dir, 0.0, 0.0]
                } else {
                    [0.0, 0.0, p.dir]
                },
                intensity: p.intensity,
            });
        }
    }
}
