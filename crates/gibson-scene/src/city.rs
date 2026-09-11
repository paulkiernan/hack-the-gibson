//! The tower city: deterministic per-tower generation (face panels, top layer, animation phase)
//! plus per-frame frustum and fog culling with a back-to-front distance sort.

use gibson_types::{CameraPose, FOG_END, TOWER_HEIGHT, TOWER_HEIGHT_MIN, TOWER_WIDTH};
use glam::Vec3;
use rand::{rngs::StdRng, Rng, SeedableRng};

use crate::FlightPath;

/// Stable salt separating the city's RNG stream from the other subsystems.
const CITY_SALT: u64 = 0x6A09_E667_F3BC_C909;
/// Second mixing constant for the grid so two grids never share a stream prefix.
const GRID_MIX: u64 = 0x9E37_79B9_7F4A_7C15;
/// Bias exponent of the per-tower height draw: `44 + 66*u^0.35`. u^0.35 concentrates mass
/// near 1, so the skyline is predominantly tall (about 82 % of towers exceed 80 units,
/// median ≈ 96) with a minority stepping down toward `TOWER_HEIGHT_MIN` (44) — the film's
/// canyon profile.
const HEIGHT_BIAS: f32 = 0.35;
/// Horizontal half-extent (world units) of the flight-path overflight keep-out: the true
/// 12-unit footprint (`TOWER_WIDTH / 2` = 6) expanded by a clean 2.0-unit margin, so a path
/// sample grazing a tower's shoulder still caps that tower. The emitted/tested AABB keeps the
/// true ±6 footprint; the wider scan makes the 1.0-unit clearance invariant hold robustly.
const CAP_FOOTPRINT_HALF: f32 = TOWER_WIDTH * 0.5 + 2.0;
/// Vertical clearance left under the lowest overflying flight-path sample.
const PATH_CLEARANCE: f32 = 1.5;
/// Sampling step (flight-path segments) for the overflight scan: 0.01 is five times finer than
/// the 0.05 lattice of the clearance test, so a sampled minimum never misses a valley the test
/// would observe. Built once per city (Scene::new / grid change), never per frame.
const PATH_SAMPLE_STEP: f32 = 0.01;
/// Absolute shortest tower before a full-height slot degrades to a stub.
const HEIGHT_FLOOR: f32 = 8.0;
/// Smallest height ever emitted (a stub below an overflight that skims under `HEIGHT_FLOOR`).
const STUB_FLOOR: f32 = 0.5;
/// Generous horizontal half-angle multiplier (aspect ~2) used for the cull frustum.
const HORIZONTAL_FOV_MULT: f32 = 2.0;

/// The tower city state.
pub(crate) struct City {
    grid: u32,
    half: i32,
    towers: Vec<Tower>,
}

/// Per-tower fixed identity, drawn once at build time from the seed.
pub(crate) struct Tower {
    /// Atlas panel id per side face, order +x, -x, +z, -z.
    pub(crate) face_layers: [u32; 4],
    /// Atlas layer for the top face.
    pub(crate) top_layer: u32,
    /// Random per-tower animation phase.
    pub(crate) anim_phase: f32,
    /// Actual world height after the clearance cap (see [`City::cap_heights`]).
    pub(crate) height: f32,
}

/// One tower that survived this frame's culling, with its squared distance for sorting.
pub(crate) struct Visible {
    pub(crate) index: u32,
    pub(crate) dsq: f32,
}

impl City {
    /// Build the `grid x grid` tower city. Deterministic: the same `(seed, grid)` pair always
    /// yields the identical city, and a later rebuild (settings changed the grid) reproduces it.
    /// Each tower's base height is drawn here (biased tall); [`City::cap_heights`] then caps
    /// overflown towers below the flight path.
    pub(crate) fn new(grid: u32, seed: u64) -> City {
        let half = (grid / 2) as i32;
        let mut rng =
            StdRng::seed_from_u64(seed ^ CITY_SALT ^ (grid as u64).wrapping_mul(GRID_MIX));
        let mut towers = Vec::with_capacity((grid as usize) * (grid as usize));
        for _ in 0..(grid * grid) {
            towers.push(Tower {
                face_layers: [
                    pick_panel(&mut rng),
                    pick_panel(&mut rng),
                    pick_panel(&mut rng),
                    pick_panel(&mut rng),
                ],
                top_layer: rng.random_range(0..32u32),
                anim_phase: rng.random::<f32>(),
                // Base skyline height: `TOWER_HEIGHT_MIN + (TOWER_HEIGHT - TOWER_HEIGHT_MIN)
                // * u^0.35` with u uniform in [0, 1): a power draw biased toward the film's
                // tall canyon (~82 % above 80 units, median ~96, minimum 44). Capped below by
                // the flight path in `cap_heights`.
                height: TOWER_HEIGHT_MIN
                    + (TOWER_HEIGHT - TOWER_HEIGHT_MIN) * rng.random::<f32>().powf(HEIGHT_BIAS),
            });
        }
        City { grid, half, towers }
    }

    pub(crate) fn grid(&self) -> u32 {
        self.grid
    }

    /// Row-major index -> tower column/row and XZ base center.
    fn xz(&self, index: u32) -> (f32, f32) {
        let i = (index / self.grid) as i32 - self.half;
        let j = (index % self.grid) as i32 - self.half;
        (i as f32 * 30.0 + 15.0, j as f32 * 30.0)
    }

    /// Tower base-center position (y = 0).
    pub(crate) fn center(&self, index: u32) -> [f32; 3] {
        let (x, z) = self.xz(index);
        [x, 0.0, z]
    }

    /// Tower identity (faces, top layer, animation phase, capped height).
    pub(crate) fn tower(&self, index: u32) -> &Tower {
        &self.towers[index as usize]
    }

    /// Cap every overflown tower below the lowest flight-path point over its footprint.
    ///
    /// The flight path is sampled at `PATH_SAMPLE_STEP` (0.01 segments) across the whole loop;
    /// a sample counts as overflying a tower when its xz falls inside that tower's footprint
    /// expanded by the 2.0-unit `CAP_FOOTPRINT_HALF` margin. Each such tower records the
    /// minimum sample y, then its height becomes `min(random_height, min_y - PATH_CLEARANCE)`
    /// with a floor of `HEIGHT_FLOOR` (8); a tower whose minimum overflight y cannot even fit an
    /// 8-unit tower becomes a stub at `min_y - PATH_CLEARANCE` (never zero). Towers whose
    /// footprints the path never crosses keep their full random height. Opaque to `Scene`: the
    /// caller rebuilds the caps once per city (see `Scene::new` / grid-change in `update`).
    pub(crate) fn cap_heights(&mut self, path: &FlightPath) {
        let n = self.towers.len();
        let grid = self.grid as i32;
        let lo = -self.half;
        let hi = grid - self.half; // exclusive; tower offsets live in [lo, hi)
        let mut min_y = vec![f32::INFINITY; n];
        let mut overflown = vec![false; n];

        let span = path.len_segments();
        let steps = (span / PATH_SAMPLE_STEP).ceil() as usize;
        for k in 0..steps {
            let s = k as f32 * PATH_SAMPLE_STEP;
            if s >= span {
                break;
            }
            let p = path.position(s);
            // Nearest tower xz offset (ncol/nrow are lattice coordinates relative to `half`).
            let ncol = ((p[0] - 15.0) / 30.0).round() as i32;
            let nrow = (p[2] / 30.0).round() as i32;
            for nci in (ncol - 1)..=(ncol + 1) {
                for nrj in (nrow - 1)..=(nrow + 1) {
                    if nci < lo || nci >= hi || nrj < lo || nrj >= hi {
                        continue;
                    }
                    let cx = nci as f32 * 30.0 + 15.0;
                    let cz = nrj as f32 * 30.0;
                    if (p[0] - cx).abs() <= CAP_FOOTPRINT_HALF
                        && (p[2] - cz).abs() <= CAP_FOOTPRINT_HALF
                    {
                        let idx = ((nci + self.half) as usize) * (self.grid as usize)
                            + ((nrj + self.half) as usize);
                        if p[1] < min_y[idx] {
                            min_y[idx] = p[1];
                        }
                        overflown[idx] = true;
                    }
                }
            }
        }

        let mut stubs = 0u32;
        for (idx, t) in self.towers.iter_mut().enumerate() {
            if !overflown[idx] {
                continue;
            }
            let cap = min_y[idx] - PATH_CLEARANCE;
            if cap >= HEIGHT_FLOOR {
                t.height = t.height.min(cap);
            } else {
                // The path skims this footprint so low that even an 8-unit tower cannot fit:
                // leave a stub at the largest height that still clears the path.
                stubs += 1;
                t.height = cap.clamp(STUB_FLOOR, HEIGHT_FLOOR);
            }
        }
        if stubs > 0 {
            eprintln!(
                "gibson-scene: {stubs} tower(s) capped below {HEIGHT_FLOOR} by the flight path \
                 (stub heights at min_overflight_y - {PATH_CLEARANCE})"
            );
        }
    }

    /// Cull towers against the camera frustum and fog distance, then sort the survivors
    /// back-to-front (far to near) into the reused `out` buffer.
    pub(crate) fn cull(&self, cam: &CameraPose, out: &mut Vec<Visible>) {
        out.clear();
        let eye = Vec3::from_array(cam.position);
        let fwd = Vec3::from_array(cam.forward);
        let up = Vec3::from_array(cam.up);
        let right = fwd.cross(up);
        let right = if right.length_squared() < 1e-10 {
            Vec3::X
        } else {
            right.normalize()
        };
        let vtan = (cam.fov_y_radians * 0.5).tan();
        let htan = vtan * HORIZONTAL_FOV_MULT;
        let fog_sq = FOG_END * FOG_END;

        let n = self.towers.len() as u32;
        for index in 0..n {
            let (x, z) = self.xz(index);
            // Sphere about the tower's own mid-height: radius covers the top corners of the
            // `height`-tall 12x12 box (vertical half-height plus the footprint half-diagonal
            // of about 8.5 units, rounded up to 9).
            let h = self.towers[index as usize].height;
            let center = Vec3::new(x, h * 0.5, z);
            let r = h * 0.5 + 9.0;
            let d = center - eye;
            let df = d.dot(fwd);
            if df + r <= 0.0 {
                continue; // fully behind the camera
            }
            if d.dot(up).abs() > df * vtan + r {
                continue; // outside vertically
            }
            if d.dot(right).abs() > df * htan + r {
                continue; // outside horizontally
            }
            let dsq = d.length_squared();
            if dsq > fog_sq {
                continue; // past the fog end: not worth drawing
            }
            out.push(Visible { index, dsq });
        }
        out.sort_unstable_by(|a, b| b.dsq.total_cmp(&a.dsq));
    }
}

/// Weighted panel draw: panels 0..28 each weight 1.0, hero directory panels 28..32 weight 0.5
/// (~6.7 % of faces) so every corridor shot shows at least one readable directory tower,
/// matching the film's hero towers without drowning the mosaic field.
fn pick_panel(rng: &mut StdRng) -> u32 {
    let r = rng.random::<f32>() * 30.0;
    if r < 28.0 {
        r as u32
    } else {
        (28 + (((r - 28.0) / 0.5) as u32)).min(31)
    }
}
