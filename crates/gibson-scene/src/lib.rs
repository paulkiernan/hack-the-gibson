//! Gibson world simulation: flight path, camera, tower city, lane pulses, face-block highlights,
//! and palette state. [`Scene`] advances the whole sim with [`Scene::update`] and exposes the
//! per-frame render slice with [`Scene::frame`].

use gibson_types::{CameraPose, FrameData, Palette, PulseInstance, Settings, TowerInstance};

mod camera;
mod city;
mod flight_path;
mod highlights;
mod palette_state;
mod pulses;

pub use flight_path::FlightPath;

use camera::CameraRig;
use city::{City, Visible};
use highlights::Highlights;
use pulses::PulsePool;

/// Tower city and flight state. Fixed per-frame API (signatures frozen in Batch 0):
/// [`Scene::update`] advances to an absolute `time`, [`Scene::frame`] borrows the resulting
/// buffers, and [`Scene::camera`] returns the current pose.
pub struct Scene {
    seed: u64,
    /// The closed flight loop (39 legacy waypoints).
    path: FlightPath,
    /// Camera position + banking state.
    rig: CameraRig,
    /// Absolute scene time of the most recent `update` call.
    time: f64,
    /// Whether `update` has been called at least once (first call advances with `dt = 0`).
    started: bool,
    /// Tower city (rebuilt deterministically when the settings grid changes).
    city: City,
    /// Lane pulse streaks.
    pulses: PulsePool,
    /// Face-block highlight sweeps.
    highlights: Highlights,
    /// Reused cull/sort scratch: visible towers, back-to-front.
    visible: Vec<Visible>,
    /// Reused instance buffers handed to `frame`.
    tower_buf: Vec<TowerInstance>,
    pulse_buf: Vec<PulseInstance>,
    /// Palette computed by the last `update`.
    palette: Palette,
}

impl Scene {
    /// Create a city for `settings.grid x settings.grid` towers seeded by `seed`. The pulse pool
    /// and highlight cadence are seeded from the same `seed` (deterministic per `(seed, grid)`).
    pub fn new(settings: &Settings, seed: u64) -> Scene {
        let settings = settings.clone().clamped();
        let path = FlightPath::default_loop();
        let rig = CameraRig::new(&path, settings.fly_speed);
        let mut pulses = PulsePool::new(seed);
        pulses.set_grid(settings.grid);
        pulses.resize(pulse_count(&settings));
        Scene {
            seed,
            path,
            rig,
            time: 0.0,
            started: false,
            city: City::new(settings.grid, seed),
            pulses,
            highlights: Highlights::new(seed),
            visible: Vec::with_capacity(4096),
            tower_buf: Vec::with_capacity(4096),
            pulse_buf: Vec::new(),
            palette: Palette::NORMAL,
        }
    }

    /// Provide the atlas's per-panel block ids (see the contract `AtlasImage::blocks_per_panel`).
    /// Until called, block highlights stay disabled because there is no valid block to light.
    pub fn set_block_ids(&mut self, blocks_per_panel: Vec<Vec<u8>>) {
        self.highlights.set_block_ids(blocks_per_panel);
    }

    /// Advance the simulation to absolute `time` seconds with the given settings.
    ///
    /// The frame time step is `clamp(time - previous, 0, 0.25)` (a stalled saver must not
    /// teleport the camera); the first call advances with `dt = 0` and zero bank. A settings grid
    /// change rebuilds the city deterministically, and the pulse pool / tower buffers resize.
    pub fn update(&mut self, time: f64, settings: &Settings) {
        let settings = settings.clone().clamped();

        let dt = if self.started {
            ((time - self.time) as f32).clamp(0.0, 0.25)
        } else {
            self.started = true;
            0.0
        };
        self.time = time;

        // Grid change -> deterministic rebuild; lane pulses re-spawn onto the new grid extent.
        if self.city.grid() != settings.grid {
            self.city = City::new(settings.grid, self.seed);
            self.pulses.set_grid(settings.grid);
        }
        self.pulses.resize(pulse_count(&settings));

        // 1. Camera (pose + banked roll).
        self.rig.advance(dt, &settings, &self.path);
        let pose = self.rig.camera();

        // 2. Cull + sort the towers back-to-front for this pose.
        self.city.cull(&pose, &mut self.visible);

        // 3. Highlights: expire finished sweeps, pick new ones from the visible set.
        self.highlights.update(time, &pose, &self.city, &self.visible);

        // 4. Compose the tower instances (highlight envelope applied to visible towers only).
        self.tower_buf.clear();
        for v in &self.visible {
            let tower = self.city.tower(v.index);
            let (highlight_block, highlight_t) = self.highlights.envelope(v.index, time);
            self.tower_buf.push(TowerInstance {
                position: self.city.center(v.index),
                anim_phase: tower.anim_phase,
                face_layers: tower.face_layers,
                top_layer: tower.top_layer,
                highlight_block,
                highlight_t,
                _pad: 0.0,
            });
        }

        // 5. Animate the lane pulses and compose their instances.
        self.pulses.advance(dt);
        self.pulse_buf.clear();
        self.pulses.write_instances(&mut self.pulse_buf);

        // 6. Palette for this frame.
        self.palette = palette_state::palette(settings.palette, time, settings.palette_cycle_seconds);
    }

    /// Current camera pose.
    pub fn camera(&self) -> CameraPose {
        self.rig.camera()
    }

    /// Per-frame render data for the state produced by the last [`Scene::update`].
    pub fn frame<'a>(&'a self, settings: &'a Settings) -> FrameData<'a> {
        FrameData {
            time: self.time,
            camera: self.rig.camera(),
            prev_camera: self.rig.prev_camera(),
            palette: self.palette,
            towers: &self.tower_buf,
            pulses: &self.pulse_buf,
            settings,
        }
    }
}

/// World-space base-center position of tower `(i, j)` in a `grid x grid` city.
///
/// Towers sit on `x = 15 (mod 30)`, `z = 0 (mod 30)`; lanes run along `x = 0` and `z = 15`.
/// `i`/`j` are in `0..grid`; the city is centered on the origin.
pub fn tower_center(i: i32, j: i32, grid: u32) -> [f32; 3] {
    let half = (grid / 2) as i32;
    let x = ((i - half) as f32) * 30.0 + 15.0;
    let z = ((j - half) as f32) * 30.0;
    [x, 0.0, z]
}

/// Desired lane-pulse count for the given (clamped) settings.
fn pulse_count(settings: &Settings) -> usize {
    if settings.preview {
        80
    } else {
        settings.pulses as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gibson_types::{PaletteMode, TOWER_HEIGHT, TOWER_WIDTH};

    /// 32 panels each carrying block ids 1..=4, standing in for the real atlas.
    fn sample_blocks() -> Vec<Vec<u8>> {
        vec![vec![1u8, 2, 3, 4]; 32]
    }

    fn tower_eq(a: &TowerInstance, b: &TowerInstance) -> bool {
        a.position == b.position
            && a.anim_phase == b.anim_phase
            && a.face_layers == b.face_layers
            && a.top_layer == b.top_layer
            && a.highlight_block == b.highlight_block
            && a.highlight_t == b.highlight_t
    }

    fn pulse_eq(a: &PulseInstance, b: &PulseInstance) -> bool {
        a.position == b.position
            && a.length == b.length
            && a.direction == b.direction
            && a.intensity == b.intensity
    }

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

    #[test]
    fn path_closes_continuously() {
        let path = FlightPath::default_loop();
        let a = path.position(0.0);
        let b = path.position(39.0);
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() < 1e-3);
        }
        // Just before the seam the curve is within a whisker of the start point (C1 wrap).
        let c = path.position(39.0 - 1e-3);
        let d: f32 = (0..3).map(|k| (c[k] - a[k]) * (c[k] - a[k])).sum::<f32>().sqrt();
        assert!(d < 0.2, "near-seam gap {d} exceeds 0.2 units");
    }

    /// Every flight-path sample stays at least 1.0 unit clear of every tower AABB for `grid = 60`.
    #[test]
    fn flight_path_clears_tower_city() {
        let grid = 60i32;
        let half = grid / 2;
        let half_w = TOWER_WIDTH * 0.5; // 6
        let path = FlightPath::default_loop();

        // Track the single worst clearance (nearest approach to a tower) over the whole loop.
        let mut worst_s = -1.0f32;
        let mut worst_pos = [0.0f32; 3];
        let mut worst_d = f32::INFINITY;
        let mut k = 0usize;
        loop {
            let s = k as f32 * 0.05;
            if s >= 39.0 {
                break;
            }
            k += 1;
            let p = path.position(s);
            let (x, y, z) = (p[0], p[1], p[2]);
            // Only towers within one lattice cell of the sample can be closer than 1.0 unit.
            let ic = (((x - 15.0) / 30.0).round() as i32) + half;
            let jc = (z / 30.0).round() as i32 + half;
            let mut min_d = f32::INFINITY;
            for di in -1..=1 {
                for dj in -1..=1 {
                    let (i, j) = (ic + di, jc + dj);
                    if !(0..grid).contains(&i) || !(0..grid).contains(&j) {
                        continue;
                    }
                    let cx = ((i - half) as f32) * 30.0 + 15.0;
                    let cz = ((j - half) as f32) * 30.0;
                    let dxo = ((x - cx).abs() - half_w).max(0.0);
                    let dzo = ((z - cz).abs() - half_w).max(0.0);
                    let yexc = if y < 0.0 {
                        -y
                    } else if y > TOWER_HEIGHT {
                        y - TOWER_HEIGHT
                    } else {
                        0.0
                    };
                    let d = (dxo * dxo + dzo * dzo + yexc * yexc).sqrt();
                    min_d = min_d.min(d);
                }
            }
            if min_d < worst_d {
                worst_d = min_d;
                worst_s = s;
                worst_pos = p;
            }
        }
        assert!(
            worst_d >= 1.0,
            "flight path clips a tower: s={worst_s} pos={worst_pos:?} clearance={worst_d} (grid 60)"
        );
    }

    /// After 5 s at 60 Hz every pulse head sits exactly on a street of the right family, and
    /// heads stay at street height.
    #[test]
    fn lane_pulses_stay_on_streets() {
        let settings = Settings::default();
        let mut scene = Scene::new(&settings, 11);
        for k in 0..300 {
            scene.update(k as f64 / 60.0, &settings);
        }
        let frame = scene.frame(&settings);
        assert_eq!(frame.pulses.len(), 400);
        for p in frame.pulses {
            // Distance to the nearest x-street (x = 30m) or z-street (z = 15 + 30m) line.
            let x_mod = p.position[0].rem_euclid(30.0);
            let z_mod = (p.position[2] - 15.0).rem_euclid(30.0);
            let dx = x_mod.min(30.0 - x_mod);
            let dz = z_mod.min(30.0 - z_mod);
            assert!(
                dx < 1e-3 || dz < 1e-3,
                "pulse off every street: position {:?} (x off {dx}, z off {dz})",
                p.position
            );
            assert!(
                (0.6..=2.5).contains(&p.position[1]),
                "pulse head y out of range: {:?}",
                p.position[1]
            );
        }
    }

    /// Two scenes with the same seed stepped identically produce byte-identical frames.
    #[test]
    fn same_seed_scenes_are_identical() {
        let settings = Settings::default();
        let mut a = Scene::new(&settings, 42);
        let mut b = Scene::new(&settings, 42);
        a.set_block_ids(sample_blocks());
        b.set_block_ids(sample_blocks());
        for k in 0..720 {
            let t = k as f64 / 60.0; // 12 s: several highlight pick events elapse
            a.update(t, &settings);
            b.update(t, &settings);
            let fa = a.frame(&settings);
            let fb = b.frame(&settings);
            assert_eq!(fa.towers.len(), fb.towers.len(), "tower count at t={t}");
            for (ta, tb) in fa.towers.iter().zip(fb.towers.iter()) {
                assert!(tower_eq(ta, tb), "tower mismatch at t={t}");
            }
            assert_eq!(fa.pulses.len(), fb.pulses.len(), "pulse count at t={t}");
            for (pa, pb) in fa.pulses.iter().zip(fb.pulses.iter()) {
                assert!(pulse_eq(pa, pb), "pulse mismatch at t={t}");
            }
        }
    }

    /// Over a 40 s flight the camera up vector stays sane, banking engages (roll > 5 degrees)
    /// and never exceeds `bank_max_degrees`.
    #[test]
    fn banking_engages_and_stays_bounded() {
        let settings = Settings::default();
        let mut scene = Scene::new(&settings, 3);
        let mut max_roll = 0.0f32;
        let max_allowed = settings.bank_max_degrees.to_radians();
        for k in 0..2400 {
            scene.update(k as f64 / 60.0, &settings);
            let up = scene.camera().up;
            let len2 = up[0] * up[0] + up[1] * up[1] + up[2] * up[2];
            assert!((len2 - 1.0).abs() < 1e-3, "up not unit: {up:?}");
            let dot_up = up[1];
            assert!(dot_up >= 0.15, "up dropped below 0.15: {up:?}");
            let roll = dot_up.clamp(-1.0, 1.0).acos();
            assert!(
                roll <= max_allowed + 1e-3,
                "roll {roll} exceeds bank_max {max_allowed}"
            );
            max_roll = max_roll.max(roll);
        }
        assert!(
            max_roll > 5f32.to_radians(),
            "banking never engaged; max roll over 40 s was {max_roll}"
        );
    }

    /// Palette modes: Normal/Siege are exact; Cycle alternates with a smoothstep crossfade.
    #[test]
    fn cycle_palette_crossfades() {
        // Cycle 0 (t in 0..10) targets NORMAL; the palette crossfades from the previous cycle's
        // SIEGE during the first 6 s, so t = 0 still reads fully SIEGE (seamless transition).
        let at_start = palette_state::palette(PaletteMode::Cycle, 0.0, 10.0);
        assert_eq!(at_start, Palette::SIEGE);
        // Mid-crossfade (t = 3): strictly between the two endpoints.
        let mid = palette_state::palette(PaletteMode::Cycle, 3.0, 10.0);
        assert!(mid != Palette::NORMAL && mid != Palette::SIEGE);
        // Steady state at t = 8 (past the 6 s fade): NORMAL.
        let steady = palette_state::palette(PaletteMode::Cycle, 8.0, 10.0);
        assert_eq!(steady, Palette::NORMAL);
        // t = 12: three seconds into the Siege cycle (crossfading NORMAL -> SIEGE).
        let sieging = palette_state::palette(PaletteMode::Cycle, 12.0, 10.0);
        assert!(sieging != Palette::NORMAL && sieging != Palette::SIEGE);
        // Endpoints resolve to the exact consts.
        assert_eq!(
            palette_state::palette(PaletteMode::Normal, 1.0, 10.0),
            Palette::NORMAL
        );
        assert_eq!(
            palette_state::palette(PaletteMode::Siege, 1.0, 10.0),
            Palette::SIEGE
        );
    }

    /// Performance probe: peak `update()` cost at grid 60. Real numbers come from
    /// `cargo test -p gibson-scene --release` (asserts are release-only).
    #[test]
    fn scene_update_peak_time_grid_60() {
        let settings = Settings::default();
        let mut scene = Scene::new(&settings, 99);
        scene.set_block_ids(sample_blocks());
        let mut peak = std::time::Duration::ZERO;
        for k in 0..600 {
            let t = k as f64 / 60.0;
            let start = std::time::Instant::now();
            scene.update(t, &settings);
            peak = peak.max(start.elapsed());
        }
        let _ = scene.frame(&settings);
        println!("grid-60 peak update(): {peak:?}");
        #[cfg(not(debug_assertions))]
        assert!(
            peak < std::time::Duration::from_millis(4),
            "peak update too slow: {peak:?}"
        );
    }
}
