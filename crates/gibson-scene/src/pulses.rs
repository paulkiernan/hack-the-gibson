//! Lane pulses: laser beams that streak along the streets between tower rows.
//!
//! Type A runs along x on a lane `z = 15 + 30*m` (between tower rows at `z = 30k`); type B runs
//! along z on a lane `x = 30*m` (between tower columns at `x = 15 + 30k`). Lane coordinates are
//! exact f32 arithmetic (`30*m`, `15 + 30*m`) so instances land precisely on the street grid and
//! never intersect a tower footprint (every lane is at least 9 units clear of the 12-unit boxes).
//!
//! Beams are randomized on every (re)spawn: a glow hue index drawn from `Palette::pulse_hues`
//! (50 / 25 / 15 / 10 percent weights), a speed drawn normal-fast (220..460 u/s) ~74 % of the
//! time or from the rare very-fast zip band (700..1500 u/s) ~26 % of the time, a ribbon length
//! proportional to speed, and an altitude — about 70 % skim the ground lanes (`y` 0.6..2.5)
//! while the rest fly between the towers at height (`y` uniform 8..=90). Because fast beams
//! respawn sooner (their respawn rate scales with speed), the fraction of beams *present* in a
//! populated frame skews slower than the spawn split: zips sit at roughly 10-12 % of the pool
//! (see `ZIP_FRACTION`). Each beam stores its hue *index*, not a color, so when the palette
//! cycles (or crossfades in `Cycle` mode) every beam recolors coherently from the frame's
//! current palette instead of freezing an old color.

use gibson_types::{Palette, PulseInstance};
use rand::{rngs::StdRng, Rng, SeedableRng};

/// Stable salt separating the pulse pool's RNG stream from the other subsystems.
const PULSE_SALT: u64 = 0x3C6E_F372_FE94_F82B;
/// Fraction of beams that fly at height; the rest stay low on the ground lanes.
const HIGH_FRACTION: f32 = 0.30;
/// Low beams ride just above the floor (world units).
const LOW_Y_MIN: f32 = 0.6;
const LOW_Y_MAX: f32 = 2.5;
/// High beams streak between the towers: uniform in y up to a sensible ceiling of 90 units
/// (towers top out at `TOWER_HEIGHT` = 110; overflown flight-path towers are shorter still, so
/// a 90-unit ceiling keeps zips visibly passing *through* the upper city volume).
const HIGH_Y_MIN: f32 = 8.0;
const HIGH_Y_MAX: f32 = 90.0;
/// Fraction of freshly spawned beams in the rare very-fast "zip" band.
///
/// Note: a beam's respawn rate is proportional to its speed (fast beams cross the grid extent
/// and re-roll sooner), so the fraction of beams *present* at any instant is lower than the
/// spawn fraction: steady-state presence ≈ C·v̄_fast/v̄_zip ≈ C·0.31. C = 0.26 therefore keeps
/// about 10 % of a populated frame as zips (>= the 8 % acceptance floor) while zips still
/// account for their ~quarter of fresh draws and the majority of beams are normal-fast.
const ZIP_FRACTION: f32 = 0.26;
/// Normal-fast band, units/second: quick bolts that read as fast-moving lasers.
const FAST_MIN: f32 = 220.0;
const FAST_MAX: f32 = 460.0;
/// Zip band, units/second: rare streaks that whip across the space.
const ZIP_MIN: f32 = 700.0;
const ZIP_MAX: f32 = 1500.0;
/// Ribbon length per unit of speed, so fast beams read as long thin streaks and slow ones as
/// short bolts (`speed * 0.055`; a 1400 u/s zip is 77 units long, a 250 u/s beam ~14).
const LENGTH_PER_SPEED: f32 = 0.055;
/// Length clamps (world units).
const LENGTH_MIN: f32 = 12.0;
const LENGTH_MAX: f32 = 90.0;
/// Hue-index weights over `Palette::pulse_hues`: index 0 (white-cyan) 50 %, 1 (bright green)
/// 25 %, 2 (electric blue) 15 %, 3 (magenta) 10 % for NORMAL (warm equivalents for SIEGE).
const HUE_0_WEIGHT: f32 = 0.50;
const HUE_1_WEIGHT: f32 = 0.75; // cumulative: 50 % + 25 %
const HUE_2_WEIGHT: f32 = 0.90; // cumulative: 75 % + 15 %
                                // The remaining 10 % (r >= 0.90) land on hue index 3.

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
    /// Ribbon length in world units (proportional to `speed`, clamped).
    length: f32,
    /// Brightness.
    intensity: f32,
    /// Index into `Palette::pulse_hues` (0..=3), re-rolled on every respawn.
    hue: u8,
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

    /// Draw a random hue index with weights 50 / 25 / 15 / 10 percent over `pulse_hues`.
    fn pick_hue(&mut self) -> u8 {
        let r = self.rng.random::<f32>();
        if r < HUE_0_WEIGHT {
            0
        } else if r < HUE_1_WEIGHT {
            1
        } else if r < HUE_2_WEIGHT {
            2
        } else {
            3
        }
    }

    /// Draw a fresh random pulse (all fields).
    fn make_pulse(&mut self) -> Pulse {
        let half = self.half;
        let extent = self.extent();
        let high = self.rng.random::<f32>() < HIGH_FRACTION;
        let speed = if self.rng.random::<f32>() < ZIP_FRACTION {
            self.rng.random_range(ZIP_MIN..ZIP_MAX)
        } else {
            self.rng.random_range(FAST_MIN..FAST_MAX)
        };
        Pulse {
            along_x: self.rng.random::<f32>() < 0.5,
            m: self.rng.random_range(-half..=half),
            pos: self.rng.random_range(-extent..extent),
            y: if high {
                self.rng.random_range(HIGH_Y_MIN..=HIGH_Y_MAX)
            } else {
                self.rng.random_range(LOW_Y_MIN..LOW_Y_MAX)
            },
            speed,
            dir: if self.rng.random::<f32>() < 0.5 {
                1.0
            } else {
                -1.0
            },
            length: (speed * LENGTH_PER_SPEED).clamp(LENGTH_MIN, LENGTH_MAX),
            intensity: self.rng.random_range(0.7..1.0),
            hue: self.pick_hue(),
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

    /// Append this frame's `PulseInstance`s to `out`, resolving each beam's stored hue index
    /// against the *current* `palette` so beams recolor when the palette changes or crossfades.
    pub(crate) fn write_instances(&self, palette: &Palette, out: &mut Vec<PulseInstance>) {
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
                color: palette.pulse_hues[p.hue as usize],
                _pad: 0.0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gibson_types::Palette;

    /// 700 beams (default `grid = 60` count) stepped at 60 Hz for 20 s. A beam's respawn rate
    /// is proportional to its speed, so warm past the slowest beam's worst-case lifetime
    /// (~8.7 s) to reach the steady-state mixture the renderer actually shows; zips then sit at
    /// roughly 10-12 % of the pool (their ~26 % share of fresh spawns diluted by faster
    /// re-rolls), and altitude/hue stay at their spawn weights (they do not affect respawn rate).
    fn populated_pool(seed: u64) -> PulsePool {
        let mut pool = PulsePool::new(seed);
        pool.resize(700);
        for _ in 0..(20 * 60) {
            pool.advance(1.0 / 60.0);
        }
        pool
    }

    #[test]
    fn beam_distribution_matches_spec() {
        let pool = populated_pool(7);
        let n = pool.pulses.len();
        assert_eq!(n, 700);

        // Speed bands: ~85 % normal-fast (220..460), ~15 % zip (700..1500).
        let mut zip = 0usize;
        let mut hue_count = [0usize; 4];
        let mut high = 0usize;
        for p in &pool.pulses {
            assert!(
                (FAST_MIN..FAST_MAX).contains(&p.speed) || (ZIP_MIN..ZIP_MAX).contains(&p.speed),
                "speed {} outside both bands",
                p.speed
            );
            if p.speed >= ZIP_MIN {
                zip += 1;
            }
            // Length correlates with speed exactly as specified.
            let expect = (p.speed * LENGTH_PER_SPEED).clamp(LENGTH_MIN, LENGTH_MAX);
            assert!(
                (p.length - expect).abs() < 1e-3,
                "length {} does not match speed {} (expected {expect})",
                p.length,
                p.speed
            );
            assert!(p.hue < 4);
            hue_count[p.hue as usize] += 1;
            // Altitude split: low ground lanes or the 8..=90 high band, nothing between.
            assert!(
                (LOW_Y_MIN..=LOW_Y_MAX).contains(&p.y) || (HIGH_Y_MIN..=HIGH_Y_MAX).contains(&p.y),
                "beam y {} outside the low or high band",
                p.y
            );
            if p.y >= HIGH_Y_MIN {
                high += 1;
            }
        }
        let zip_frac = zip as f32 / n as f32;
        let high_frac = high as f32 / n as f32;
        assert!(zip_frac >= 0.08, "zip presence too low: {zip_frac}");
        assert!(zip_frac <= 0.35, "zip presence too high: {zip_frac}");
        assert!(
            (0.15..=0.45).contains(&high_frac),
            "high-beam fraction {high_frac}"
        );
        let distinct = hue_count.iter().filter(|&&c| c > 0).count();
        assert!(
            distinct >= 3,
            "expected at least 3 hues, saw {distinct}: {hue_count:?}"
        );
        // Sanity on the weights: white-cyan (index 0) stays the plurality.
        let top = hue_count
            .iter()
            .enumerate()
            .max_by_key(|(_, &c)| c)
            .unwrap()
            .0;
        assert_eq!(
            top, 0,
            "index 0 should be the most common hue: {hue_count:?}"
        );
    }

    #[test]
    fn colors_resolve_from_the_active_palette() {
        let pool = populated_pool(21);
        for palette in [Palette::NORMAL, Palette::SIEGE] {
            let mut out = Vec::with_capacity(pool.pulses.len());
            pool.write_instances(&palette, &mut out);
            assert_eq!(out.len(), pool.pulses.len());
            for (inst, p) in out.iter().zip(pool.pulses.iter()) {
                // The instance carries the palette's current hue for the beam's stored index:
                // a later palette switch (Cycle crossfade included) recolors every beam through
                // this resolution rather than freezing an old color.
                assert_eq!(inst.color, palette.pulse_hues[p.hue as usize]);
                assert_eq!(inst._pad, 0.0);
            }
        }
    }

    #[test]
    fn same_seed_pools_are_identical() {
        let a = populated_pool(42);
        let b = populated_pool(42);
        for (pa, pb) in a.pulses.iter().zip(b.pulses.iter()) {
            assert_eq!(pa.along_x, pb.along_x);
            assert_eq!(pa.m, pb.m);
            assert_eq!(pa.pos, pb.pos);
            assert_eq!(pa.y, pb.y);
            assert_eq!(pa.speed, pb.speed);
            assert_eq!(pa.dir, pb.dir);
            assert_eq!(pa.length, pb.length);
            assert_eq!(pa.intensity, pb.intensity);
            assert_eq!(pa.hue, pb.hue);
        }
    }
}
