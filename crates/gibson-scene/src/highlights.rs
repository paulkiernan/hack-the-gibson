//! Face-block highlights: every 6-10 seconds the city lights one rectangular text block on the
//! camera-facing face of a nearby tower with a magenta sweep (ramp 0.3 s, hold 3 s, fade 0.5 s).
//! At most four towers are lit at once; highlight state is keyed to the tower's grid index so it
//! survives per-frame culling.

use gibson_types::CameraPose;
use glam::Vec3;
use rand::{Rng, SeedableRng, rngs::StdRng};

use crate::city::{City, Visible};

/// Stable salt separating the highlight RNG stream from the other subsystems.
const HL_SALT: u64 = 0x243F_6A88_85A3_08D3;
/// Maximum concurrent highlights.
const MAX_ACTIVE: usize = 4;
/// Ramp-up duration, seconds.
const RAMP: f64 = 0.3;
/// End of the full-bright hold (`RAMP + 3.0`), seconds.
const HOLD: f64 = 3.3;
/// End of the fade-out (`HOLD + 0.5`), seconds.
const FADE_END: f64 = 3.8;
/// Towers closer than this (units) may be picked.
const PICK_RADIUS: f32 = 120.0;
/// Minimum `dot(forward, normalize(toTower))` for a pickable tower.
const PICK_DOT: f32 = 0.3;
/// Pick cadence: every 6..10 seconds.
const CADENCE_MIN: f64 = 6.0;
const CADENCE_SPAN: f64 = 4.0;

pub(crate) struct Highlights {
    rng: StdRng,
    /// Atlas `blocks_per_panel` (len 32). Empty until `set_block_ids` runs -> highlights off.
    blocks_per_panel: Vec<Vec<u8>>,
    /// Absolute scene time of the next pick attempt.
    next_pick_at: f64,
    active: Vec<Active>,
}

/// One lit block.
struct Active {
    /// Tower grid index (survives culling churn).
    index: u32,
    /// Atlas block id on the camera-facing panel, 1..=255.
    block: u8,
    /// Absolute scene time the highlight started.
    start: f64,
}

impl Highlights {
    pub(crate) fn new(seed: u64) -> Highlights {
        let mut rng = StdRng::seed_from_u64(seed ^ HL_SALT);
        let next_pick_at = CADENCE_MIN + rng.random_range(0.0..CADENCE_SPAN);
        Highlights {
            rng,
            blocks_per_panel: Vec::new(),
            next_pick_at,
            active: Vec::with_capacity(MAX_ACTIVE),
        }
    }

    /// Provide the atlas's per-panel block ids (see `Scene::set_block_ids`).
    pub(crate) fn set_block_ids(&mut self, blocks_per_panel: Vec<Vec<u8>>) {
        self.blocks_per_panel = blocks_per_panel;
    }

    /// Advance the highlight state to absolute `time`: expire finished sweeps, then attempt picks
    /// whenever the cadence timer has elapsed.
    pub(crate) fn update(&mut self, time: f64, cam: &CameraPose, city: &City, visible: &[Visible]) {
        self.active.retain(|a| time - a.start < FADE_END);
        while time >= self.next_pick_at {
            self.try_pick(time, cam, city, visible);
            self.next_pick_at += CADENCE_MIN + self.rng.random_range(0.0..CADENCE_SPAN);
        }
    }

    /// Attempt to light one tower from the current visible set.
    fn try_pick(&mut self, time: f64, cam: &CameraPose, city: &City, visible: &[Visible]) {
        if self.blocks_per_panel.is_empty() || self.active.len() >= MAX_ACTIVE {
            return;
        }
        let eye = Vec3::from_array(cam.position);
        let fwd = Vec3::from_array(cam.forward);

        // Candidates: visible towers close ahead, not already lit.
        let mut candidates: Vec<u32> = Vec::new();
        for v in visible {
            if v.dsq > PICK_RADIUS * PICK_RADIUS {
                continue;
            }
            if self.active.iter().any(|a| a.index == v.index) {
                continue;
            }
            let to = Vec3::from_array(city.center(v.index)) - eye;
            let d = to.length();
            if d < 1e-3 {
                continue;
            }
            if fwd.dot(to / d) <= PICK_DOT {
                continue;
            }
            candidates.push(v.index);
        }
        if candidates.is_empty() {
            return;
        }
        let tower = candidates[self.rng.random_range(0..candidates.len())];
        let face = face_most_facing(&eye, city, tower);
        let panel = city.tower(tower).face_layers[face] as usize;
        if let Some(list) = self.blocks_per_panel.get(panel) {
            if list.is_empty() {
                return;
            }
            let block = list[self.rng.random_range(0..list.len())];
            self.active.push(Active {
                index: tower,
                block,
                start: time,
            });
        }
    }

    /// Highlight value for a tower at absolute `time`: `(block id, 0..=1 envelope)` or
    /// `(0, 0.0)` when the tower is not lit.
    pub(crate) fn envelope(&self, index: u32, time: f64) -> (u32, f32) {
        for a in &self.active {
            if a.index == index {
                let phase = time - a.start;
                let t = if phase < 0.0 {
                    0.0
                } else if phase < RAMP {
                    phase / RAMP
                } else if phase < HOLD {
                    1.0
                } else if phase < FADE_END {
                    1.0 - (phase - HOLD) / (FADE_END - HOLD)
                } else {
                    0.0
                };
                return (a.block as u32, t as f32);
            }
        }
        (0, 0.0)
    }
}

/// Index (in `face_layers` order +x, -x, +z, -z) of the face most facing the camera.
fn face_most_facing(eye: &Vec3, city: &City, index: u32) -> usize {
    let to_cam = *eye - Vec3::from_array(city.center(index));
    if to_cam.x.abs() >= to_cam.z.abs() {
        if to_cam.x > 0.0 {
            0
        } else {
            1
        }
    } else if to_cam.z > 0.0 {
        2
    } else {
        3
    }
}
