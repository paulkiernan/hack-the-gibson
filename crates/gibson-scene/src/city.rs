//! The tower city: deterministic per-tower generation (face panels, top layer, animation phase)
//! plus per-frame frustum and fog culling with a back-to-front distance sort.

use gibson_types::{CameraPose, FOG_END, TOWER_HEIGHT};
use glam::Vec3;
use rand::{Rng, SeedableRng, rngs::StdRng};

/// Stable salt separating the city's RNG stream from the other subsystems.
const CITY_SALT: u64 = 0x6A09_E667_F3BC_C909;
/// Second mixing constant for the grid so two grids never share a stream prefix.
const GRID_MIX: u64 = 0x9E37_79B9_7F4A_7C15;
/// Tower bounding-sphere radius about the box center (y = TOWER_HEIGHT / 2).
const SPHERE_RADIUS: f32 = 22.0;
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
}

/// One tower that survived this frame's culling, with its squared distance for sorting.
pub(crate) struct Visible {
    pub(crate) index: u32,
    pub(crate) dsq: f32,
}

impl City {
    /// Build the `grid x grid` tower city. Deterministic: the same `(seed, grid)` pair always
    /// yields the identical city, and a later rebuild (settings changed the grid) reproduces it.
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

    /// Tower identity (faces, top layer, animation phase).
    pub(crate) fn tower(&self, index: u32) -> &Tower {
        &self.towers[index as usize]
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
        let r = SPHERE_RADIUS;
        let fog_sq = FOG_END * FOG_END;

        let n = self.towers.len() as u32;
        for index in 0..n {
            let (x, z) = self.xz(index);
            let center = Vec3::new(x, TOWER_HEIGHT * 0.5, z);
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

/// Weighted panel draw: panels 0..28 each weight 1.0, hero directory panels 28..32 weight 0.125.
fn pick_panel(rng: &mut StdRng) -> u32 {
    let r = rng.random::<f32>() * 28.5;
    if r < 28.0 {
        r as u32
    } else {
        (28 + (((r - 28.0) / 0.125) as u32)).min(31)
    }
}
