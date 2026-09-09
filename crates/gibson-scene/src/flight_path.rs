//! Flight-path constants rescued from the legacy Irrlicht flythrough.
//!
//! # Provenance
//! - Source: `src/fly_path.h` of the pre-Rust repo (committed history keeps the original).
//! - Irrlicht was left-handed Y-up; **every z is negated** here for a right-handed Y-up world.
//! - The source had 41 rows; rows at index 39 and 40 were exact duplicates of row 0, so the
//!   closed loop has **39 unique control points** (indices 0..=38). The duplicates were dropped.
//! - Transcribed verbatim: no rounding, no reordering; x and y copied as-is, z negated.
//!   Mechanically verified against `git show HEAD:src/fly_path.h` (see Scaffold batch report).
//!
//! # Constants the Batch 1 scene agent needs (from the same header)
//! - `GIBSON_FLY_SPEED 0.55` — fly speed in segments/second (legacy `#define GIBSON_FLY_SPEED 0.55f`).
//! - `GIBSON_FLY_TIGHTNESS 0.5` — Catmull-Rom tension (legacy `#define GIBSON_FLY_TIGHTNESS 0.5f`).
//! - `GIBSON_FLY_LOOKAHEAD_MS 150` — camera look-ahead (legacy `#define GIBSON_FLY_LOOKAHEAD_MS 150`).
//!
//! The legacy camera applied a 150 ms look-ahead offset (`camDt = elapsedMs - LOOKAHEAD_MS`)
//! *and* aimed the camera forward from position to a look-at point (`lookDt = elapsedMs`), i.e.
//! the pose at `s` looked toward `s + fly_speed * 0.15` segments. The Batch 1 spec instead
//! advances the camera by `s` and looks at `s + fly_speed * 0.15`; both formulations agree
//! because the closed loop is translation-invariant under the constant 150 ms lead.
//!
//! The legacy spline itself was a *ping-pong* (alternating-direction) Catmull-Rom over 40 spans;
//! the Batch 1 spec replaces that with a **closed Catmull-Rom that wraps continuously**, which is
//! the intended modernization (no direction reversal at the seam).

use glam::Vec3;

pub(crate) const WAYPOINTS: [[f32; 3]; 39] = [
    [0.0392192, 6.95062, -32.1667],
    [0.13452, 6.5646, -55.4633],
    [0.189138, 6.72459, -96.8121],
    [-0.139794, 7.21804, -145.059],
    [-0.489681, 16.2712, -186.951],
    [-0.401056, 16.7698, -219.447],
    [-0.291434, 17.3865, -259.642],
    [-0.52368, 17.9816, -308.385],
    [8.69042, 17.3182, -313.72],
    [59.6782, 15.8175, -314.299],
    [60.4089, 15.0665, -278.267],
    [60.5837, 15.495, -232.322],
    [60.7141, 15.5494, -200.472],
    [86.3944, 14.8672, -195.875],
    [131.81, 13.8411, -196.464],
    [159.956, 20.7447, -194.651],
    [216.099, 37.5139, -192.33],
    [258.068, 50.0952, -180.791],
    [266.522, 48.8569, -150.402],
    [270.506, 33.0024, -93.9888],
    [272.414, 17.6326, -35.1101],
    [272.369, 17.7922, 8.18755],
    [269.593, 18.0043, 39.8612],
    [229.488, 19.8768, 43.4609],
    [187.706, 19.0659, 44.2737],
    [150.242, 23.1816, 42.1108],
    [149.888, 18.2483, 9.74219],
    [149.797, 22.5784, -30.3554],
    [149.408, 26.4205, -71.9536],
    [148.835, 30.2287, -110.622],
    [149.819, 19.2602, -150.008],
    [148.399, 16.7217, -195.691],
    [104.66, 17.6327, -195.331],
    [66.8026, 14.1855, -195.546],
    [32.9041, 14.0211, -195.275],
    [31.9278, 12.607, -135.516],
    [30.6969, 9.92016, -38.4132],
    [28.3579, 10.8269, -16.8861],
    [2.34459, 9.89253, -17.0397],
];

/// Catmull-Rom tension (legacy `GIBSON_FLY_TIGHTNESS`).
const TIGHTNESS: f32 = 0.5;

/// One closed-loop flight path over the 39 unique legacy waypoints. `position(s)` samples a
/// closed Catmull-Rom spline (tension 0.5); `s` is measured in segments and wraps forever.
#[derive(Clone, Copy, Debug)]
pub struct FlightPath {
    waypoints: [[f32; 3]; WAYPOINTS.len()],
}

impl FlightPath {
    /// The legacy 39 unique control points (z negated for the right-handed world).
    pub fn default_loop() -> FlightPath {
        FlightPath {
            waypoints: WAYPOINTS,
        }
    }

    /// Length of the closed loop in segments (`39.0` for the default loop).
    pub fn len_segments(&self) -> f32 {
        self.waypoints.len() as f32
    }

    /// Position along the closed loop at `s` segments: closed Catmull-Rom (tension 0.5),
    /// segment `i` from `P[i-1], P[i], P[i+1], P[i+2]` mod 39. Negative `s` wraps; the loop is
    /// `C¹` across the seam because the neighbors wrap cyclically.
    pub fn position(&self, s: f32) -> [f32; 3] {
        let pts = &self.waypoints;
        let n = pts.len();
        let span = n as f32;
        let s = s.rem_euclid(span);
        let i = s.floor() as usize; // 0..=n-1
        let u = s - i as f32;

        let p0 = Vec3::from_array(pts[(i + n - 1) % n]);
        let p1 = Vec3::from_array(pts[i]);
        let p2 = Vec3::from_array(pts[(i + 1) % n]);
        let p3 = Vec3::from_array(pts[(i + 2) % n]);

        // Hermite basis (legacy gibson_spline_at).
        let u2 = u * u;
        let u3 = u2 * u;
        let h1 = 2.0 * u3 - 3.0 * u2 + 1.0;
        let h2 = -2.0 * u3 + 3.0 * u2;
        let h3 = u3 - 2.0 * u2 + u;
        let h4 = u3 - u2;
        let t1 = (p2 - p0) * TIGHTNESS;
        let t2 = (p3 - p1) * TIGHTNESS;

        (p1 * h1 + p2 * h2 + t1 * h3 + t2 * h4).to_array()
    }
}
