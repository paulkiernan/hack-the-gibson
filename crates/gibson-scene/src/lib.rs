//! Gibson world simulation: flight path, camera, tower city, lane pulses, face-block highlights,
//! and the siege spread. [`Scene`] advances the whole sim with [`Scene::update`] and exposes the
//! per-frame render slice with [`Scene::frame`].

use gibson_types::{CameraPose, FrameData, Palette, PulseInstance, Settings, TowerInstance};

mod camera;
mod city;
mod flight_path;
mod highlights;
mod pulses;
mod siege;

pub use flight_path::FlightPath;

use camera::CameraRig;
use city::{City, Visible};
use highlights::Highlights;
use pulses::PulsePool;
use siege::SiegeSpread;

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
    /// Per-tower siege onsets (rebuilt with the city).
    siege: SiegeSpread,
    /// Palette the lane pulses are composed against this frame: the frame's `NORMAL` blended
    /// toward `SIEGE` by the wave's progress, so beams warm and cool with the siege. Towers are
    /// *not* baked against this — the renderer mixes each one by `TowerInstance::siege_t`.
    pulse_palette: Palette,
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
        let mut city = City::new(settings.grid, seed);
        // Per-tower clearance caps are flight-path geometry: built once per city, never per
        // frame (grid changes in `update` rebuild and re-cap identically).
        city.cap_heights(&path);
        let siege = SiegeSpread::new(&city, seed);
        Scene {
            seed,
            path,
            rig,
            time: 0.0,
            started: false,
            city,
            pulses,
            highlights: Highlights::new(seed),
            visible: Vec::with_capacity(4096),
            tower_buf: Vec::with_capacity(4096),
            pulse_buf: Vec::new(),
            siege,
            pulse_palette: Palette::NORMAL,
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
            // Same one-time flight-path cap as `Scene::new` (never per frame).
            self.city.cap_heights(&self.path);
            // The siege wave is laid out over the city's geometry, so it is rebuilt with it.
            self.siege = SiegeSpread::new(&self.city, self.seed);
            self.pulses.set_grid(settings.grid);
        }
        self.pulses.resize(pulse_count(&settings));

        // 1. Camera (pose + banked roll).
        self.rig.advance(dt, &settings, &self.path);
        let pose = self.rig.camera();

        // 2. Siege wave for this frame: how far it has travelled, and the per-tower ramp width
        //    under this mode / cycle period (resolved before the pulses below are composed, so
        //    each beam's stored hue index recolors against the palette actually in effect).
        let wave = self
            .siege
            .wave(settings.palette, time, settings.palette_cycle_seconds);
        self.pulse_palette = Palette::NORMAL.lerp(&Palette::SIEGE, wave.progress);

        // 3. Cull + sort the towers back-to-front for this pose.
        self.city.cull(&pose, &mut self.visible);

        // 4. Highlights: expire finished sweeps, pick new ones from the visible set.
        self.highlights
            .update(time, &pose, &self.city, &self.visible);

        // 5. Compose the tower instances (highlight envelope and siege blend applied to visible
        //    towers only; both are keyed to the tower's grid index, so culling cannot lose them).
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
                // Per-tower height: random biased-tall base capped below the flight path where
                // the camera overflies the footprint (see `City::cap_heights`).
                height: tower.height,
                siege_t: self.siege.siege_t(v.index, wave),
            });
        }

        // 6. Animate the lane pulses and compose their instances.
        self.pulses.advance(dt);
        self.pulse_buf.clear();
        self.pulses
            .write_instances(&self.pulse_palette, &mut self.pulse_buf);
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
            // The frame palette is the NORMAL end of the siege blend in every mode: the siege
            // look is entirely per tower, carried by `TowerInstance::siege_t`, and the renderer
            // mixes `Palette::NORMAL` toward `Palette::SIEGE` itself. Blending here as well
            // would double-apply it.
            palette: Palette::NORMAL,
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
    use crate::siege::{CYCLE_TRANSITION, RAMP_SECONDS, SIEGE_DELAY};
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
            && a.height == b.height
            && a.siege_t == b.siege_t
    }

    fn pulse_eq(a: &PulseInstance, b: &PulseInstance) -> bool {
        a.position == b.position
            && a.length == b.length
            && a.direction == b.direction
            && a.intensity == b.intensity
            && a.color == b.color
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
        let d: f32 = (0..3)
            .map(|k| (c[k] - a[k]) * (c[k] - a[k]))
            .sum::<f32>()
            .sqrt();
        assert!(d < 0.2, "near-seam gap {d} exceeds 0.2 units");
    }

    /// Every flight-path sample stays at least 1.0 unit clear of every tower AABB for `grid = 60`,
    /// where each tower's AABB uses that tower's real capped height from `TowerInstance::height`.
    ///
    /// The rescued legacy path sweeps over tower row z = -180 (column x = 255) at s ≈ 16.75
    /// (pos [250.3, 48.0, -185.2], y ≈ 48). That tower is capped to min(y over its overflown
    /// footprint) - 1.5, so the sample clears its roof with ~1.5 units to spare; towers the path
    /// never overflies keep their full biased-tall height. The invariant below therefore holds
    /// for every tower: within the ±2.0-unit capping margin the vertical clearance is ≥ 1.5, and
    /// outside it the horizontal clearance is ≥ 2.0.
    #[test]
    fn flight_path_clears_tower_city() {
        let grid = 60i32;
        let half = grid / 2;
        let half_w = TOWER_WIDTH * 0.5; // 6
        let settings = Settings {
            grid: grid as u32,
            ..Settings::default()
        };
        let scene = Scene::new(&settings, 11);
        let path = FlightPath::default_loop();

        // Track the single worst clearance (nearest approach to a tower) over the whole loop.
        let mut worst_s = -1.0f32;
        let mut worst_pos = [0.0f32; 3];
        let mut worst_d = f32::INFINITY;
        // The s ≈ 16.75 crossing the ignored version of this test flagged (row z = -180,
        // column x = 255): confirm that specific tower now clears and report its capped height.
        let tower_255_z180 = (((255.0 - 15.0) / 30.0) as i32 + half) as usize * (grid as usize)
            + ((-180.0 / 30.0) as i32 + half) as usize;
        let mut h255 = scene.city.tower(tower_255_z180 as u32).height;
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
            let ncol = ((x - 15.0) / 30.0).round() as i32;
            let nrow = (z / 30.0).round() as i32;
            let mut min_d = f32::INFINITY;
            for nci in (ncol - 1)..=(ncol + 1) {
                for nrj in (nrow - 1)..=(nrow + 1) {
                    if !(-half..grid - half).contains(&nci) || !(-half..grid - half).contains(&nrj)
                    {
                        continue;
                    }
                    let idx = ((nci + half) as usize) * (grid as usize) + ((nrj + half) as usize);
                    let cx = nci as f32 * 30.0 + 15.0;
                    let cz = nrj as f32 * 30.0;
                    let h = scene.city.tower(idx as u32).height;
                    if idx == tower_255_z180 {
                        h255 = h;
                    }
                    let dxo = ((x - cx).abs() - half_w).max(0.0);
                    let dzo = ((z - cz).abs() - half_w).max(0.0);
                    let yexc = if y < 0.0 {
                        -y
                    } else if y > h {
                        y - h
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
        println!(
            "clearance: worst {worst_d:.3} units at s={worst_s} pos={worst_pos:?}; \
             tower row z=-180 col x=255 capped height = {h255:.2}"
        );
        assert!(
            worst_d >= 1.0,
            "flight path clips a tower: s={worst_s} pos={worst_pos:?} clearance={worst_d} (grid 60)"
        );
    }

    /// The height model: every emitted tower height lies in `8.0..=TOWER_HEIGHT`, the skyline is
    /// predominantly tall (>= 60 % above 80 units at grid 60), and no tower AABB contains any
    /// flight-path sample (the per-tower clearance caps make this hold for every seed).
    #[test]
    fn tower_heights_form_tall_canyon_and_clear_the_path() {
        let grid = 60u32;
        let half = (grid / 2) as i32;
        let half_w = TOWER_WIDTH * 0.5;
        let settings = Settings {
            grid,
            ..Settings::default()
        };
        let scene = Scene::new(&settings, 11);
        let path = FlightPath::default_loop();

        let mut tall = 0usize;
        let mut min_h = f32::INFINITY;
        for idx in 0..(grid * grid) {
            let h = scene.city.tower(idx).height;
            assert!(
                (8.0..=TOWER_HEIGHT).contains(&h),
                "tower {idx} height {h} outside 8..=TOWER_HEIGHT"
            );
            min_h = min_h.min(h);
            if h > 80.0 {
                tall += 1;
            }
        }
        let tall_frac = tall as f64 / (grid * grid) as f64;
        assert!(
            tall_frac >= 0.60,
            "only {tall_frac:.3} of towers exceed 80 units (need >= 0.60); min height {min_h:.1}"
        );
        println!("height model: {tall}/{grid}x{grid} towers > 80 units ({tall_frac:.3}); min height {min_h:.1}");

        // No flight-path sample sits inside any tower's AABB (true ±6 footprint, own height).
        let mut k = 0usize;
        loop {
            let s = k as f32 * 0.05;
            if s >= 39.0 {
                break;
            }
            k += 1;
            let p = path.position(s);
            let ncol = ((p[0] - 15.0) / 30.0).round() as i32;
            let nrow = (p[2] / 30.0).round() as i32;
            for nci in (ncol - 1)..=(ncol + 1) {
                for nrj in (nrow - 1)..=(nrow + 1) {
                    if !(-half..grid as i32 - half).contains(&nci)
                        || !(-half..grid as i32 - half).contains(&nrj)
                    {
                        continue;
                    }
                    let idx = ((nci + half) as usize) * (grid as usize) + ((nrj + half) as usize);
                    let cx = nci as f32 * 30.0 + 15.0;
                    let cz = nrj as f32 * 30.0;
                    let h = scene.city.tower(idx as u32).height;
                    let contained = (p[0] - cx).abs() <= half_w
                        && (p[2] - cz).abs() <= half_w
                        && (0.0..=h).contains(&p[1]);
                    assert!(
                        !contained,
                        "flight path inside tower {idx} AABB at s={s} pos={p:?} height {h:.2}"
                    );
                }
            }
        }
    }

    /// After 5 s at 60 Hz every pulse head sits exactly on a street of the right family, and
    /// heads stay at a lane height: either low on the ground lanes (0.6..2.5) or flying at
    /// height (8..=90) — never between, since the two bands are the designed altitude split.
    #[test]
    fn lane_pulses_stay_on_streets() {
        let settings = Settings::default();
        let mut scene = Scene::new(&settings, 11);
        for k in 0..300 {
            scene.update(k as f64 / 60.0, &settings);
        }
        let frame = scene.frame(&settings);
        assert_eq!(frame.pulses.len(), settings.pulses as usize);
        let mut high = 0usize;
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
            let y = p.position[1];
            assert!(
                (0.6..=2.5).contains(&y) || (8.0..=90.0).contains(&y),
                "pulse head y {y} outside the low (0.6..2.5) or high (8..=90) band: {:?}",
                p.position
            );
            if y >= 8.0 {
                high += 1;
            }
        }
        let high_frac = high as f32 / frame.pulses.len() as f32;
        assert!(
            (0.15..=0.45).contains(&high_frac),
            "high-altitude fraction {high_frac} outside the ~30 % split"
        );
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

    /// Normal mode is untouched by the siege machinery: every emitted `siege_t` is exactly 0 at
    /// every time, and the frame palette stays the normal end of the blend.
    #[test]
    fn normal_mode_never_sieges() {
        let settings = Settings::default(); // Normal palette, grid 60.
        let mut scene = Scene::new(&settings, 11);
        for t in [0.0, 1.0, 5.0, 30.0, 600.0] {
            scene.update(t, &settings);
            let frame = scene.frame(&settings);
            assert_eq!(frame.palette, Palette::NORMAL);
            assert!(!frame.towers.is_empty(), "no towers visible at t={t}");
            for tower in frame.towers {
                assert_eq!(tower.siege_t, 0.0, "tower sieged in Normal mode at t={t}");
            }
        }
    }

    /// Siege mode turns towers one by one: early in the wave the city is split between towers
    /// already red and towers still normal (that split *is* the feature), and by the end of the
    /// sweep every tower is fully siege and stays there.
    #[test]
    fn siege_mode_turns_towers_one_by_one() {
        let grid = 60u32;
        let settings = Settings {
            palette: PaletteMode::Siege,
            grid,
            ..Settings::default()
        };
        let mut scene = Scene::new(&settings, 11);
        let n = grid * grid;
        let span = scene.siege.span();

        // Early: the wave is about a third of the way across the city.
        let early = SIEGE_DELAY + 0.35 * span as f64;
        let wave = scene
            .siege
            .wave(settings.palette, early, settings.palette_cycle_seconds);
        let mut red = 0usize;
        let mut normal = 0usize;
        for idx in 0..n {
            let s = scene.siege.siege_t(idx, wave);
            if s > 0.99 {
                red += 1;
            } else if s < 0.01 {
                normal += 1;
            }
        }
        assert!(
            red > 0 && normal > 0,
            "at t={early:.1} the city had {red} red and {normal} normal towers, not a spread"
        );
        println!("siege at t={early:.1}s of a {span:.1}s sweep: {red} red, {normal} normal of {n}");

        // The instances the scene actually emits carry the same split.
        scene.update(early, &settings);
        let frame = scene.frame(&settings);
        assert_eq!(
            frame.palette,
            Palette::NORMAL,
            "Siege must hand over the normal palette"
        );
        assert!(
            frame.towers.iter().any(|t| t.siege_t > 0.99),
            "no red tower among the {} visible at t={early:.1}",
            frame.towers.len()
        );
        assert!(
            frame.towers.iter().any(|t| t.siege_t == 0.0),
            "no normal tower among the {} visible at t={early:.1}",
            frame.towers.len()
        );

        // Late: the whole city is siege, and it holds for hours.
        let late = [
            SIEGE_DELAY + span as f64 + RAMP_SECONDS as f64,
            120.0,
            3600.0,
        ];
        for t in late {
            scene.update(t, &settings);
            let frame = scene.frame(&settings);
            assert_eq!(frame.palette, Palette::NORMAL);
            for tower in frame.towers {
                assert!(
                    tower.siege_t >= 0.999,
                    "tower not fully siege at t={t}: {}",
                    tower.siege_t
                );
            }
        }
        let wave = scene
            .siege
            .wave(settings.palette, 3600.0, settings.palette_cycle_seconds);
        for idx in 0..n {
            assert!(
                scene.siege.siege_t(idx, wave) >= 0.999,
                "tower {idx} not siege at t=3600"
            );
        }
    }

    /// Cycle mode runs the same spread on the `palette_cycle_seconds` period: the wave sweeps
    /// in tower by tower over the first `CYCLE_TRANSITION` of a siege cycle, holds, then sweeps
    /// back out over the first `CYCLE_TRANSITION` of the next cycle — and the frame palette
    /// stays the normal end of the blend the whole way through.
    #[test]
    fn cycle_mode_sweeps_in_and_back_out() {
        fn progress(scene: &Scene, t: f64, period: f32) -> f32 {
            scene.siege.wave(PaletteMode::Cycle, t, period).progress
        }
        let period = 40.0f32;
        let settings = Settings {
            palette: PaletteMode::Cycle,
            palette_cycle_seconds: period,
            grid: 30,
            ..Settings::default()
        };
        let scene = Scene::new(&settings, 11);
        let p = period as f64;
        let edge = CYCLE_TRANSITION * p; // 6 s of the 40 s period.

        // Cycle 0 boots normal and stays there: no siege has run yet, so there is nothing to
        // recede from.
        for t in [0.0, 0.5 * p, p - 1e-3] {
            assert_eq!(progress(&scene, t, period), 0.0, "cycle 0 moved at t={t}");
        }
        // Cycle 1 sweeps in over the first 15 % of the period, then holds at full siege.
        assert_eq!(
            progress(&scene, p, period),
            0.0,
            "siege cycle did not start from normal"
        );
        let half_way = progress(&scene, p + 0.5 * edge, period);
        assert!(
            (0.4..=0.6).contains(&half_way),
            "mid-sweep progress {half_way}"
        );
        assert_eq!(
            progress(&scene, p + edge, period),
            1.0,
            "sweep did not complete"
        );
        assert_eq!(progress(&scene, 1.5 * p, period), 1.0, "siege did not hold");
        assert_eq!(
            progress(&scene, 2.0 * p, period),
            1.0,
            "siege dropped at the boundary"
        );
        // Cycle 2 sweeps back out, then holds normal; cycle 3 sieges again, cycle 4 recedes.
        let receding = progress(&scene, 2.0 * p + 0.5 * edge, period);
        assert!(
            (0.4..=0.6).contains(&receding),
            "mid-recede progress {receding}"
        );
        assert_eq!(
            progress(&scene, 2.0 * p + edge, period),
            0.0,
            "siege did not clear"
        );
        for t in [2.5 * p, 3.0 * p - 1e-3] {
            assert_eq!(progress(&scene, t, period), 0.0, "siege returned at t={t}");
        }
        assert_eq!(
            progress(&scene, 3.0 * p + edge, period),
            1.0,
            "second siege did not complete"
        );
        assert_eq!(
            progress(&scene, 4.0 * p + edge, period),
            0.0,
            "second recede did not clear"
        );

        // The emitted towers follow the same timeline, and the frame palette never leaves the
        // normal end of the blend.
        let mut scene = scene;
        for t in [
            0.0,
            p,
            p + 0.5 * edge,
            p + edge,
            1.5 * p,
            2.0 * p,
            2.0 * p + edge,
            3.0 * p,
        ] {
            scene.update(t, &settings);
            let frame = scene.frame(&settings);
            assert_eq!(
                frame.palette,
                Palette::NORMAL,
                "Cycle palette left NORMAL at t={t}"
            );
            let cleared = progress(&scene, t, period) == 0.0;
            let full = progress(&scene, t, period) >= 1.0;
            for tower in frame.towers {
                if cleared {
                    assert_eq!(tower.siege_t, 0.0, "tower still sieged at t={t}");
                }
                if full {
                    assert!(
                        tower.siege_t >= 0.999,
                        "tower not siege during the hold at t={t}"
                    );
                }
            }
        }
    }

    /// The spread is a pure function of `(seed, grid)`: two scenes with the same seed emit
    /// identical `siege_t` sequences, and a different seed lays the wave out differently.
    #[test]
    fn siege_spread_is_deterministic() {
        let settings = Settings {
            palette: PaletteMode::Siege,
            ..Settings::default()
        };
        let mut a = Scene::new(&settings, 21);
        let mut b = Scene::new(&settings, 21);
        let other = Scene::new(&settings, 22);
        let n = settings.grid * settings.grid;
        let span = a.siege.span();
        assert_eq!(span, b.siege.span(), "same seed, different sweep span");

        let end = SIEGE_DELAY + span as f64 + RAMP_SECONDS as f64;
        let mut differing = 0usize;
        for step in 0..=40 {
            let t = end * step as f64 / 40.0;
            a.update(t, &settings);
            b.update(t, &settings);
            let (wa, wb) = (
                a.siege
                    .wave(settings.palette, t, settings.palette_cycle_seconds),
                b.siege
                    .wave(settings.palette, t, settings.palette_cycle_seconds),
            );
            let wc = other
                .siege
                .wave(settings.palette, t, settings.palette_cycle_seconds);
            for idx in 0..n {
                let sa = a.siege.siege_t(idx, wa);
                assert_eq!(sa, b.siege.siege_t(idx, wb), "tower {idx} differs at t={t}");
                if other.siege.siege_t(idx, wc) != sa {
                    differing += 1;
                }
            }
            let (fa, fb) = (a.frame(&settings), b.frame(&settings));
            assert_eq!(
                fa.towers.len(),
                fb.towers.len(),
                "visible set differs at t={t}"
            );
            for (ta, tb) in fa.towers.iter().zip(fb.towers) {
                assert!(tower_eq(ta, tb), "emitted tower differs at t={t}");
            }
        }
        assert!(
            differing > 0,
            "a different seed produced the identical siege spread"
        );
    }

    /// A settings grid change rebuilds the wave with the city: a stale onset table would index
    /// out of bounds for every tower of the bigger grid, and the rebuilt wave must be the one a
    /// fresh scene of that grid and seed builds.
    #[test]
    fn grid_change_rebuilds_the_siege_wave() {
        let mut settings = Settings {
            palette: PaletteMode::Siege,
            grid: 8,
            ..Settings::default()
        };
        let mut scene = Scene::new(&settings, 3);
        for grid in [60u32, 8, 60] {
            settings.grid = grid;
            scene.update(10.0, &settings);
            let frame = scene.frame(&settings);
            assert!(!frame.towers.is_empty(), "nothing visible at grid {grid}");
            for tower in frame.towers {
                assert!((0.0..=1.0).contains(&tower.siege_t), "siege_t out of range");
            }
        }

        // The wave carried through the rebuilds equals a fresh grid-60 scene's (same seed).
        let fresh = Scene::new(&settings, 3);
        assert_eq!(scene.siege.span(), fresh.siege.span());
        let wave = scene
            .siege
            .wave(PaletteMode::Siege, 10.0, settings.palette_cycle_seconds);
        for idx in 0..(settings.grid * settings.grid) {
            assert_eq!(
                scene.siege.siege_t(idx, wave),
                fresh.siege.siege_t(idx, wave),
                "tower {idx} of the rebuilt wave differs from a fresh one"
            );
            assert!((0.0..=1.0).contains(&fresh.siege.siege_t(idx, wave)));
        }
    }

    /// Regression: `FlightPath::position` must not panic on the tiny-negative inputs the camera
    /// feeds it around the wrap (`s - TANGENT_EPS` and `s - 2*TANGENT_EPS` go negative whenever
    /// the wrapped `s` sits below 0.14 / 0.28). `f32::rem_euclid` maps a tiny negative such as
    /// -1.49e-8 to exactly 39.0 (the modulus) instead of 38.99999998; the old floored index 39
    /// indexed past the 39-element waypoint array.
    #[test]
    fn flight_path_position_handles_tiny_negative_s() {
        let path = FlightPath::default_loop();
        // Values that previously produced s = span after rem_euclid and panicked on pts[39].
        for s in [
            -1.49e-8f32,
            -0.0f32,
            -f32::MIN_POSITIVE,
            -1.0e-7,
            -1.0e-5,
            -1.0e-3,
            -0.13,
            -0.13999999,
        ] {
            let _ = path.position(s); // must not panic
        }
        // Exact camera-style inputs: wrapped s slightly under TANGENT_EPS (0.14) and under
        // 2 * TANGENT_EPS (0.28) make `s - eps` a tiny negative at the wrap.
        for s in [1e-7f32, 0.14 - 1e-6, 0.28 - 1e-6, 0.14, 0.28] {
            let _ = path.position(s - 0.14);
            let _ = path.position(s - 0.28);
        }
        // Continuity with the wrapped equivalents: position(-1.49e-8) lands on the seam, within
        // a whisker of both position(0) and the negative-s wrap position(39.0 - 1.49e-8).
        let tiny = path.position(-1.49e-8f32);
        let at_zero = path.position(0.0);
        let wrapped = path.position(39.0 - 1.49e-8);
        for k in 0..3 {
            assert!((tiny[k] - at_zero[k]).abs() < 1e-3);
            assert!((tiny[k] - wrapped[k]).abs() < 1e-3);
        }
        // And -0.0 behaves exactly like +0.0.
        assert_eq!(path.position(-0.0f32), path.position(0.0f32));
    }

    /// Performance probe: peak `update()` cost at grid 60, in Siege mode — the busiest palette
    /// path, since every visible tower resolves its own siege blend. Real numbers come from
    /// `cargo test -p gibson-scene --release` (asserts are release-only).
    #[test]
    fn scene_update_peak_time_grid_60() {
        let settings = Settings {
            palette: PaletteMode::Siege,
            ..Settings::default()
        };
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
