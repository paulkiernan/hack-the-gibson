//! Gibson world simulation: flight path, tower city, lane pulses, highlights, camera, and the
//! per-frame tower/pulse slices handed to the renderer.
//!
//! Scaffold state (Batch 0): `Scene` is a stub — `update` is a no-op, `camera()`/`frame()` return
//! a fixed pose at `(0, 20, 60)` looking down −z with an empty tower/pulse list, and the palette
//! is always `Palette::NORMAL`. `tower_center` and `FlightPath` are already real. Batch 1 replaces
//! the stub bodies without changing any public signature.

use gibson_types::{CameraPose, FrameData, Palette, Settings};

mod flight_path;

/// One closed-loop flight path over the waypoints rescued from the legacy Irrlicht flythrough.
/// Batch 1 implements closed Catmull-Rom interpolation (`position(s)`, s in segments, wrapping);
/// the scaffold interpolates linearly so the shape is already usable.
pub struct FlightPath {
    waypoints: [[f32; 3]; flight_path::WAYPOINTS.len()],
}

impl FlightPath {
    /// The legacy 39 unique control points (z negated for the right-handed world).
    pub fn default_loop() -> FlightPath {
        FlightPath {
            waypoints: flight_path::WAYPOINTS,
        }
    }

    /// Length of the closed loop in segments (`39.0` for the default loop).
    pub fn len_segments(&self) -> f32 {
        self.waypoints.len() as f32
    }

    /// Position along the closed loop at `s` segments. Scaffold: piecewise-linear interpolation
    /// between waypoints with wraparound. Batch 1 replaces this with closed Catmull-Rom
    /// (tension 0.5, segment `i` from `P[i-1], P[i], P[i+1], P[i+2]` mod 39).
    pub fn position(&self, s: f32) -> [f32; 3] {
        let n = self.waypoints.len();
        let span = n as f32;
        let s = s.rem_euclid(span);
        let i = s.floor() as usize;
        let u = s - s.floor();
        let a = self.waypoints[i % n];
        let b = self.waypoints[(i + 1) % n];
        [
            a[0] + (b[0] - a[0]) * u,
            a[1] + (b[1] - a[1]) * u,
            a[2] + (b[2] - a[2]) * u,
        ]
    }
}

/// Tower city and flight state. Scaffold stub; Batch 1 implements the full simulation.
pub struct Scene {
    settings: Settings,
    seed: u64,
    blocks_per_panel: Vec<Vec<u8>>,
    camera: CameraPose,
}

impl Scene {
    /// Create a city for `grid × grid` towers seeded by `seed`.
    pub fn new(settings: &Settings, seed: u64) -> Scene {
        Scene {
            settings: settings.clone(),
            seed,
            blocks_per_panel: Vec::new(),
            camera: CameraPose {
                position: [0.0, 20.0, 60.0],
                forward: [0.0, 0.0, -1.0],
                up: [0.0, 1.0, 0.0],
                fov_y_radians: 58f32.to_radians(),
            },
        }
    }

    /// Provide the atlas's per-panel block ids. Until called (or with empty panels) block
    /// highlights stay disabled because there is no valid block to light.
    pub fn set_block_ids(&mut self, blocks_per_panel: Vec<Vec<u8>>) {
        self.blocks_per_panel = blocks_per_panel;
    }

    /// Advance the simulation to `time` seconds with the given settings. Scaffold no-op.
    pub fn update(&mut self, time: f64, settings: &Settings) {
        // Batch 1: advance the flight path, animate pulses/highlights/faces, and cull.
        // Keep the stored copies exercised until then so nothing is dead.
        let _ = time;
        self.settings = settings.clone();
        let _ = &self.settings;
        self.seed = self.seed.wrapping_add(1);
        let _ = &self.blocks_per_panel;
    }

    /// Current camera pose. Scaffold: fixed pose.
    pub fn camera(&self) -> CameraPose {
        self.camera
    }

    /// Per-frame render data. Scaffold: fixed camera, empty tower/pulse lists, NORMAL palette.
    pub fn frame<'a>(&'a self, settings: &'a Settings) -> FrameData<'a> {
        let _ = &self.blocks_per_panel; // Batch 1 gates highlights on set_block_ids having run.
        FrameData {
            time: 0.0,
            camera: self.camera,
            prev_camera: self.camera,
            palette: Palette::NORMAL,
            towers: &[],
            pulses: &[],
            settings,
        }
    }
}

/// World-space base-center position of tower `(i, j)` in a `grid × grid` city.
///
/// Towers sit on `x ≡ 15 (mod 30)`, `z ≡ 0 (mod 30)`; lanes run along `x ≡ 0` and `z ≡ 15`.
/// `i`/`j` are in `0..grid`; the city is centered on the origin.
pub fn tower_center(i: i32, j: i32, grid: u32) -> [f32; 3] {
    let half = (grid / 2) as i32;
    let x = ((i - half) as f32) * 30.0 + 15.0;
    let z = ((j - half) as f32) * 30.0;
    [x, 0.0, z]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flight_path_has_39_unique_waypoints() {
        assert_eq!(flight_path::WAYPOINTS.len(), 39);
        let path = FlightPath::default_loop();
        assert_eq!(path.len_segments(), 39.0);
        // Closed loop: position wraps around the seam.
        let a = path.position(0.0);
        let b = path.position(39.0);
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() < 1e-3);
        }
    }

    #[test]
    fn tower_center_follows_grid_convention() {
        // grid 60, half = 30: tower (30, 30) is the origin cell at x = 15, z = 0.
        let c = tower_center(30, 30, 60);
        assert_eq!(c, [15.0, 0.0, 0.0]);
        // Corners of a 60-grid: i,j in 0..60 (max coordinate at index 59).
        let c0 = tower_center(0, 0, 60);
        assert_eq!(c0, [-885.0, 0.0, -900.0]);
        let c1 = tower_center(59, 59, 60);
        assert_eq!(c1, [885.0, 0.0, 870.0]);
        // All tower centers satisfy x ≡ 15 (mod 30), z ≡ 0 (mod 30).
        for i in 0..60 {
            for j in 0..60 {
                let p = tower_center(i, j, 60);
                assert_eq!(p[0].rem_euclid(30.0), 15.0);
                assert_eq!(p[2].rem_euclid(30.0), 0.0);
                assert_eq!(p[1], 0.0);
            }
        }
    }
}
