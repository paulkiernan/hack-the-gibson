//! Palette state machine: Normal / Siege directly, Cycle alternating between the two every
//! `palette_cycle_seconds` with a 6-second smoothstep crossfade through `Palette::lerp`.

use gibson_types::{Palette, PaletteMode};

/// Crossfade duration at each palette boundary, seconds.
const CROSSFADE: f64 = 6.0;

/// Palette active at absolute scene time `time`.
pub(crate) fn palette(mode: PaletteMode, time: f64, cycle_seconds: f32) -> Palette {
    match mode {
        PaletteMode::Normal => Palette::NORMAL,
        PaletteMode::Siege => Palette::SIEGE,
        PaletteMode::Cycle => {
            let period = (cycle_seconds as f64).max(CROSSFADE);
            // Even cycles are Normal, odd cycles are Siege.
            let base = if (time / period).floor() as i64 % 2 == 0 {
                Palette::NORMAL
            } else {
                Palette::SIEGE
            };
            let tt = time.rem_euclid(period);
            if tt < CROSSFADE {
                let u = ((tt / CROSSFADE) as f32).clamp(0.0, 1.0);
                let s = u * u * (3.0 - 2.0 * u); // smoothstep
                let prev = if base == Palette::NORMAL {
                    Palette::SIEGE
                } else {
                    Palette::NORMAL
                };
                prev.lerp(&base, s)
            } else {
                base
            }
        }
    }
}
