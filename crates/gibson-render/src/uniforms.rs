//! Per-frame uniform data shared by every pipeline.
//!
//! One `Rgba16Float` HDR chain is driven by a single uniform buffer bound to every
//! pipeline (WebGL2 allows at most four bind groups and a 16 KiB uniform buffer; this
//! struct is ~368 bytes). All colors are HDR linear values; the fragment shaders read
//! them straight out of this struct.
//!
//! The Rust layout mirrors the WGSL `FrameUniform` struct member-for-member so
//! `bytemuck` can shove it into the buffer verbatim. Every member is 16-byte aligned
//! (matrices are 64 bytes, all other members are `[f32; 4]`), which keeps the WGSL
//! uniform-layout rules satisfied without explicit padding fields.

use bytemuck::{Pod, Zeroable};
use gibson_types::FrameData;
use glam::Mat4;

/// WGSL-side `FrameUniform`; see the shaders (`u: FrameUniform`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FrameUniform {
    /// Current frame view-projection matrix (column-major).
    pub view_proj: [f32; 16],
    /// Previous frame view-projection matrix (motion-blur reprojection).
    pub prev_view_proj: [f32; 16],
    /// Inverse of `view_proj` (depth -> world reconstruction).
    pub inv_view_proj: [f32; 16],
    /// Camera eye position; w unused.
    pub camera_pos: [f32; 4],
    /// Tower glass body color + opacity.
    pub tower_body: [f32; 4],
    /// Tower text color; w unused.
    pub tower_text: [f32; 4],
    /// Block-highlight color; w unused.
    pub highlight: [f32; 4],
    /// Floor trace color; w unused.
    pub floor_trace: [f32; 4],
    /// Floor pad / via / chip-edge color; w unused.
    pub floor_pad: [f32; 4],
    /// Pulse streak color; w unused.
    pub pulse: [f32; 4],
    /// Distance haze color; w unused.
    pub haze: [f32; 4],
    /// x = time (s), y = fog start, z = fog end, w = city grid (as f32).
    pub time_fog_grid: [f32; 4],
    /// x = render width, y = render height, z = 1/width, w = 1/height.
    pub resolution: [f32; 4],
    /// x = bloom amount, y = motion blur amount, z = grain amount,
    /// w = 1.0 when the composite target is non-sRGB (manual encode).
    pub fx: [f32; 4],
}

impl FrameUniform {
    /// Fill the struct for one frame.
    pub fn new(
        view_proj: Mat4,
        prev_view_proj: Mat4,
        frame: &FrameData,
        width: u32,
        height: u32,
        srgb_target: bool,
    ) -> FrameUniform {
        let s = frame.settings;
        let palette = &frame.palette;
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        FrameUniform {
            view_proj: view_proj.to_cols_array(),
            prev_view_proj: prev_view_proj.to_cols_array(),
            inv_view_proj: view_proj.inverse().to_cols_array(),
            camera_pos: [frame.camera.position[0], frame.camera.position[1], frame.camera.position[2], 1.0],
            tower_body: palette.tower_body,
            tower_text: col4(palette.tower_text),
            highlight: col4(palette.highlight),
            floor_trace: col4(palette.floor_trace),
            floor_pad: col4(palette.floor_pad),
            pulse: col4(palette.pulse),
            haze: col4(palette.haze),
            time_fog_grid: [
                frame.time as f32,
                gibson_types::FOG_START,
                gibson_types::FOG_END,
                s.grid as f32,
            ],
            resolution: [w, h, 1.0 / w, 1.0 / h],
            fx: [
                s.bloom,
                s.motion_blur,
                s.grain,
                if srgb_target { 0.0 } else { 1.0 },
            ],
        }
    }
}

fn col4(rgb: [f32; 3]) -> [f32; 4] {
    [rgb[0], rgb[1], rgb[2], 1.0]
}

/// A 4x4 camera matrix derived from a pose.
pub fn view_matrix(pose: &gibson_types::CameraPose) -> Mat4 {
    let eye = glam::Vec3::from_array(pose.position);
    let dir = glam::Vec3::from_array(pose.forward).normalize();
    let up = glam::Vec3::from_array(pose.up);
    Mat4::look_to_rh(eye, dir, up)
}

/// Perspective projection matching the contract's 58-degree vertical FOV.
pub fn projection_matrix(pose: &gibson_types::CameraPose, width: u32, height: u32) -> Mat4 {
    let aspect = (width.max(1) as f32) / (height.max(1) as f32);
    Mat4::perspective_rh(pose.fov_y_radians, aspect, 0.5, 2200.0)
}
