//! The siege spread: how the under-attack palette rolls through the city.
//!
//! The frame palette handed to the renderer is always [`gibson_types::Palette::NORMAL`] — the
//! contract makes `FrameData::palette` the *normal* end of the siege blend — and the siege look
//! comes from the per-tower `TowerInstance::siege_t` the renderer mixes toward
//! [`gibson_types::Palette::SIEGE`]. This module owns that per-tower progress.
//!
//! **Propagation model: a distance-ordered wavefront from one seed tower.** Every tower gets a
//! fixed onset that grows with its ground distance from the seed; the wave's `progress` then
//! walks from 0 to 1 and each tower flips as the wave passes its onset. Chosen over
//! grid-neighbour contagion because:
//! - It is a *front*: at any instant one thin band of towers is changing and the rest are
//!   settled, which is what reads from inside the canyon. Contagion grows a blobby,
//!   directionless patch whose speed depends on the grid density.
//! - It is closed form: a tower's onset is a function of the tower itself, so its state survives
//!   frustum culling (keyed to the grid index, like the highlight state) and `update` costs one
//!   multiply-subtract-clamp per visible tower — no whole-city pass, no per-frame ordering.
//! - The front travels at a chosen world speed rather than at a rate implied by the lattice, so
//!   it crosses the camera at the same speed at any grid.
//!
//! Onsets are scattered about their distance order (about 2.5 tower rings at the default grid)
//! so the front is ragged and neighbours never flip together, and an exact tie between grid
//! neighbours is nudged away at build time so no two adjacent towers can ever share an onset.

use gibson_types::PaletteMode;
use rand::{Rng, SeedableRng, rngs::StdRng};

use crate::city::City;

/// Stable salt separating the siege RNG stream from every other subsystem's.
const SIEGE_SALT: u64 = 0x4528_21E6_38D0_1377;
/// World speed of the wavefront, units per second — two tower pitches per second, roughly the
/// camera's own ground speed on the default loop, so the front visibly sweeps *past* the camera
/// instead of outrunning it or standing still.
const SWEEP_SPEED: f32 = 60.0;
/// Grace period before the wave starts in [`PaletteMode::Siege`], seconds: a host that boots
/// straight into siege shows the intact city for a beat before the attack arrives.
pub(crate) const SIEGE_DELAY: f64 = 2.0;
/// Per-tower flip ramp, seconds: long enough to read as a building changing colour rather than
/// a frame pop, short enough that the wave stays a front rather than a city-wide fade.
pub(crate) const RAMP_SECONDS: f32 = 0.6;
/// Ceiling on the ramp as a fraction of a sweep: a short `palette_cycle_seconds` must still
/// read as a front rather than the whole city fading together.
pub(crate) const MAX_RAMP_FRACTION: f32 = 0.05;
/// Fraction of a sweep over which a tower's onset is scattered about its distance order. About
/// 2.5 tower rings at the default grid: enough that tangential neighbours are decorrelated,
/// small enough that the distance ordering still dominates and the front stays a front.
pub(crate) const JITTER_FRACTION: f32 = 0.06;
/// Separation applied when a tower would otherwise share an onset with a grid neighbour.
const ONSET_TIE_EPS: f32 = 1.0e-5;
/// Gap kept between the largest onset and 1, so the tie-break above can never push an onset
/// past the end of the sweep (100 nudges' worth of room).
const ONSET_HEADROOM: f32 = 1.0e-3;
/// Fraction of the cycle period each transition takes in [`PaletteMode::Cycle`]: the wave rolls
/// in over the first 15 % of a siege cycle and recedes over the first 15 % of the next one.
pub(crate) const CYCLE_TRANSITION: f64 = 0.15;
/// Floor on the cycle period used by the wave math (`Settings::clamped` already enforces 10 s).
const MIN_PERIOD: f64 = 1.0;

/// Siege wave state for one frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Wave {
    /// How far the wave has travelled: 0 = no tower infected yet, 1 = it has passed the city.
    pub(crate) progress: f32,
    /// A tower's flip ramp as a fraction of the sweep; `0 < ramp <= MAX_RAMP_FRACTION`.
    pub(crate) ramp: f32,
}

/// Per-tower siege onsets for one city, plus the wave geometry they were built for.
pub(crate) struct SiegeSpread {
    /// Onset `q` in 0..=1 per tower, row-major city index: 0 is the seed tower, 1 the last
    /// tower the wave reaches. Never exactly equal for two grid neighbours.
    onsets: Vec<f32>,
    /// Seconds the wave alone needs to cross this city at [`SWEEP_SPEED`].
    span: f32,
}

impl SiegeSpread {
    /// Build the city's siege wave: one seed tower in the middle of the city and a per-tower
    /// onset ordered by ground distance from it. Deterministic per `(seed, grid)`.
    pub(crate) fn new(city: &City, seed: u64) -> SiegeSpread {
        let grid = city.grid();
        let n = (grid * grid) as usize;
        let mut rng = StdRng::seed_from_u64(seed ^ SIEGE_SALT);

        // Seed tower drawn from the central half of the city so the wave has room to expand in
        // every direction and the whole skyline is covered in a bounded time; a corner seed
        // would arrive as a plane wave off one edge.
        let lo = grid / 4;
        let choices = (grid + 1) / 2;
        let si = lo + rng.random_range(0..choices);
        let sj = lo + rng.random_range(0..choices);
        let origin = city.center(si * grid + sj);

        // Ground distance from the seed, tracking the maximum that normalizes the sweep.
        let mut dist = vec![0.0f32; n];
        let mut max_d = 1.0f32;
        for idx in 0..n as u32 {
            let c = city.center(idx);
            let (dx, dz) = (c[0] - origin[0], c[2] - origin[2]);
            let d = (dx * dx + dz * dz).sqrt();
            dist[idx as usize] = d;
            max_d = max_d.max(d);
        }

        // Distance order scattered by up to `JITTER_FRACTION` of the sweep. Row-major order
        // means the left (`j - 1`) and upper (`i - 1`) neighbours are already placed, so an
        // exact tie is caught here and nudged clear: no two grid neighbours ever flip together.
        let mut onsets = vec![0.0f32; n];
        let mut above = vec![0.0f32; grid as usize];
        for i in 0..grid {
            for j in 0..grid {
                let idx = (i * grid + j) as usize;
                let mut q = dist[idx] / max_d * (1.0 - JITTER_FRACTION - ONSET_HEADROOM)
                    + rng.random::<f32>() * JITTER_FRACTION;
                let left = if j > 0 { onsets[idx - 1] } else { f32::NAN };
                let up = if i > 0 { above[j as usize] } else { f32::NAN };
                while q == left || q == up {
                    q += ONSET_TIE_EPS;
                    debug_assert!(q < 1.0, "onset tie break escaped the sweep range");
                }
                onsets[idx] = q;
                above[j as usize] = q;
            }
        }

        SiegeSpread {
            onsets,
            span: max_d / SWEEP_SPEED,
        }
    }

    /// Seconds the wave alone takes to cross this city (first onset to last). Test support: the
    /// scene only needs the per-tower progress, so this would otherwise be dead weight.
    #[cfg(test)]
    pub(crate) fn span(&self) -> f32 {
        self.span
    }

    /// The wave at absolute scene time `time` under `mode`.
    pub(crate) fn wave(&self, mode: PaletteMode, time: f64, cycle_seconds: f32) -> Wave {
        match mode {
            // Normal never infects anything: progress is exactly 0, which makes every tower's
            // `siege_t` exactly 0 at every time.
            PaletteMode::Normal => Wave {
                progress: 0.0,
                ramp: ramp_fraction(self.span),
            },
            // Siege runs the wave once and then holds full siege forever. A recovery loop would
            // read as a glitch on a host that runs for hours; `PaletteMode::Cycle` is the mode
            // for alternation.
            PaletteMode::Siege => Wave {
                progress: (((time - SIEGE_DELAY) as f32) / self.span).clamp(0.0, 1.0),
                ramp: ramp_fraction(self.span),
            },
            PaletteMode::Cycle => {
                let sweep = (CYCLE_TRANSITION as f32) * cycle_seconds.max(MIN_PERIOD as f32);
                Wave {
                    progress: cycle_progress(time, cycle_seconds),
                    ramp: ramp_fraction(sweep),
                }
            }
        }
    }

    /// How far tower `index` has travelled from the normal palette to the siege one, 0..=1.
    pub(crate) fn siege_t(&self, index: u32, wave: Wave) -> f32 {
        // Onsets are scaled into `0..=1 - ramp` so the last tower finishes exactly as the wave
        // reaches 1: its ramp is part of the sweep, not a tail hanging past the end of it.
        let onset = self.onsets[index as usize] * (1.0 - wave.ramp);
        ramp01((wave.progress - onset) / wave.ramp)
    }
}

/// Wave progress (0..=1) inside one [`PaletteMode::Cycle`] period.
///
/// Cycle 0 is normal; every odd cycle rolls the wave in over the first `CYCLE_TRANSITION` of
/// the period and then holds full siege, and every later even cycle recedes the wave back out
/// over the same fraction and then holds normal. Both ends are continuous: a siege cycle starts
/// at the 0 the previous cycle held, a normal cycle at the 1 the previous cycle held.
fn cycle_progress(time: f64, cycle_seconds: f32) -> f32 {
    let period = (cycle_seconds as f64).max(MIN_PERIOD);
    let cycle = (time / period).floor();
    let within = time - cycle * period;
    let sweep = CYCLE_TRANSITION * period;
    if cycle < 1.0 {
        // No siege has run yet, so there is no wave to recede from.
        0.0
    } else if (cycle as i64) % 2 == 1 {
        (within / sweep).clamp(0.0, 1.0) as f32
    } else {
        (1.0 - within / sweep).clamp(0.0, 1.0) as f32
    }
}

/// A tower's flip ramp as a fraction of a sweep of `sweep_seconds` seconds: the fixed
/// [`RAMP_SECONDS`], capped so a short sweep still reads as a front and not a global fade.
fn ramp_fraction(sweep_seconds: f32) -> f32 {
    (RAMP_SECONDS / sweep_seconds).min(MAX_RAMP_FRACTION)
}

/// Smoothstep of `u` clamped to 0..=1: exactly 0 before a tower's ramp starts, exactly 1 once it
/// is fully siege, smooth in between.
fn ramp01(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every onset is its tower's distance rank plus at most the jitter band: the ordering is
    /// the distance ordering, so the infected set really is a disc growing around the seed.
    ///
    /// The wave's origin is not carried in the struct, so the test recovers it the way a reader
    /// would: the seed is a tower whose own onset is pure jitter (its distance rank is 0), and it
    /// is the only tower in the city whose distance field explains every onset.
    #[test]
    fn onsets_follow_distance_order() {
        for (grid, seed) in [(8u32, 1u64), (60, 11), (120, 77)] {
            let city = City::new(grid, seed);
            let spread = SiegeSpread::new(&city, seed);
            let n = (grid * grid) as usize;
            assert_eq!(spread.onsets.len(), n);

            let mut origins = 0u32;
            let mut band_worst = 0.0f32;
            for candidate in 0..n as u32 {
                // A tower with a non-zero distance rank cannot be the seed: its onset is at
                // least its rank, which is never below the pure-jitter bound.
                if spread.onsets[candidate as usize] >= JITTER_FRACTION {
                    continue;
                }
                let origin = city.center(candidate);
                let mut dist = vec![0.0f32; n];
                let mut max_d = 1.0f32;
                for idx in 0..n as u32 {
                    let c = city.center(idx);
                    let (dx, dz) = (c[0] - origin[0], c[2] - origin[2]);
                    let d = (dx * dx + dz * dz).sqrt();
                    dist[idx as usize] = d;
                    max_d = max_d.max(d);
                }
                let mut worst = 0.0f32;
                for idx in 0..n as u32 {
                    let rank =
                        dist[idx as usize] / max_d * (1.0 - JITTER_FRACTION - ONSET_HEADROOM);
                    let q = spread.onsets[idx as usize];
                    if q < rank {
                        worst = f32::INFINITY;
                        break;
                    }
                    worst = worst.max(q - rank);
                }
                if worst <= JITTER_FRACTION + ONSET_TIE_EPS * 4.0 {
                    origins += 1;
                    band_worst = worst;
                    assert_eq!(dist[candidate as usize], 0.0);
                    assert!((spread.span - max_d / SWEEP_SPEED).abs() < 1e-3);
                }
            }
            assert_eq!(
                origins, 1,
                "grid {grid} seed {seed}: {origins} towers explain the wave's onsets"
            );
            println!(
                "grid {grid} seed {seed}: sweep {:.1}s, onset bands within {:.4} of the distance \
                 rank (jitter {JITTER_FRACTION})",
                spread.span, band_worst
            );
        }
    }

    /// No two grid neighbours ever share an onset, at any grid — adjacent towers cannot flip at
    /// the same instant.
    #[test]
    fn no_two_grid_neighbours_share_an_onset() {
        for (grid, seed) in [(8u32, 1u64), (60, 11), (120, 77)] {
            let city = City::new(grid, seed);
            let spread = SiegeSpread::new(&city, seed);
            let q = |i: u32, j: u32| spread.onsets[(i * grid + j) as usize];
            for i in 0..grid {
                for j in 0..grid {
                    if i + 1 < grid {
                        assert_ne!(q(i, j), q(i + 1, j), "grid {grid} seed {seed}: ({i},{j})+x");
                    }
                    if j + 1 < grid {
                        assert_ne!(q(i, j), q(i, j + 1), "grid {grid} seed {seed}: ({i},{j})+z");
                    }
                }
            }
        }
    }

    /// The wave is a front, not a global fade: at any point of the sweep only a small slice of
    /// the city is mid-flip, and the two settled populations dominate on either side of it.
    #[test]
    fn the_wave_is_a_front_not_a_global_fade() {
        let grid = 60u32;
        let city = City::new(grid, 11);
        let spread = SiegeSpread::new(&city, 11);
        let n = (grid * grid) as usize;
        for step in 0..=10 {
            let progress = step as f32 / 10.0;
            let wave = Wave {
                progress,
                ramp: ramp_fraction(spread.span),
            };
            let mut mid = 0usize;
            let mut early = 0usize;
            let mut late = 0usize;
            for idx in 0..n as u32 {
                let s = spread.siege_t(idx, wave);
                if s <= 0.0 {
                    early += 1;
                } else if s >= 1.0 {
                    late += 1;
                } else {
                    mid += 1;
                }
            }
            println!("progress {progress:.1}: {early} normal, {mid} mid-flip, {late} siege");
            assert!(
                mid * 8 <= n,
                "progress {progress:.1}: {mid}/{n} towers mid-flip — that is a global fade, \
                 not a front"
            );
        }
    }

    /// Endpoints and monotonicity: nothing is infected at progress 0, the whole city is at
    /// progress 1, and a tower only ever moves toward siege as the wave advances.
    #[test]
    fn siege_t_ramps_once_and_settles() {
        let city = City::new(60, 5);
        let spread = SiegeSpread::new(&city, 5);
        let ramp = ramp_fraction(spread.span);
        let at = |progress: f32, idx: u32| spread.siege_t(idx, Wave { progress, ramp });

        for idx in 0..3600u32 {
            assert_eq!(at(0.0, idx), 0.0, "tower {idx} infected at progress 0");
            assert_eq!(at(1.0, idx), 1.0, "tower {idx} not fully siege at progress 1");
            let mut prev = 0.0f32;
            for step in 1..=100 {
                let s = at(step as f32 / 100.0, idx);
                assert!(s >= prev, "tower {idx} un-sieged as the wave advanced");
                prev = s;
            }
        }
    }
}
