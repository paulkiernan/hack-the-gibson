//! Per-frame uniform data shared by every pipeline.
//!
//! One `Rgba16Float` HDR chain is driven by a single uniform buffer bound to every
//! pipeline (WebGL2 allows at most four bind groups and a 16 KiB uniform buffer; this
//! struct is 448 bytes). All colors are HDR linear values; the fragment shaders read
//! them straight out of this struct.
//!
//! The Rust layout mirrors the WGSL `FrameUniform` struct member-for-member so
//! `bytemuck` can shove it into the buffer verbatim. Every member is 16-byte aligned
//! (matrices are 64 bytes, all other members are `[f32; 4]`), which keeps the WGSL
//! uniform-layout rules satisfied without explicit padding fields.

use bytemuck::{Pod, Zeroable};
use gibson_types::{FrameData, Palette};
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
    /// Tower glass body color + opacity at the NORMAL end of the siege blend.
    pub tower_body_normal: [f32; 4],
    /// Tower text color at the NORMAL end; w unused.
    pub tower_text_normal: [f32; 4],
    /// Block-highlight color at the NORMAL end; w unused.
    pub highlight_normal: [f32; 4],
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
    /// x = CRT amount (0 = off ..= 1 = full); y = 1.0 when the *final* target needs a manual
    /// sRGB encode (a linear surface), which is what the CRT pass branches on; zw unused.
    pub post: [f32; 4],
    /// Tower glass body at the SIEGE end of the blend (RGB linear HDR + opacity).
    pub tower_body_siege: [f32; 4],
    /// Tower text color at the SIEGE end; w unused.
    pub tower_text_siege: [f32; 4],
    /// Block-highlight color at the SIEGE end; w unused.
    pub highlight_siege: [f32; 4],
    /// x = scene-chain (CRT signal) width, y = height, z = 1/w, w = 1/h. The scene chain
    /// renders at this size; it equals `resolution` whenever the CRT pass is inactive.
    pub signal: [f32; 4],
}

impl FrameUniform {
    /// Fill the struct for one frame.
    ///
    /// `width`/`height` are the *output* size (what the composite's final target measures) and
    /// `scene` the size the HDR chain actually renders at: the CRT signal resolution when the
    /// CRT pass is active, the output size otherwise. The two differ only for `crt > 0`.
    ///
    /// `scene_srgb` describes the format the *composite* writes (the signal buffer when the CRT
    /// pass follows) and `final_srgb` the final output target; both are `true` when the hardware
    /// encodes sRGB on store, and each pass branches on its own flag.
    pub fn new(
        view_proj: Mat4,
        prev_view_proj: Mat4,
        frame: &FrameData,
        width: u32,
        height: u32,
        scene: (u32, u32),
        scene_srgb: bool,
        final_srgb: bool,
    ) -> FrameUniform {
        let s = frame.settings;
        let palette = &frame.palette;
        let siege = Palette::SIEGE;
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let (sw, sh) = (scene.0.max(1) as f32, scene.1.max(1) as f32);
        FrameUniform {
            view_proj: view_proj.to_cols_array(),
            prev_view_proj: prev_view_proj.to_cols_array(),
            inv_view_proj: view_proj.inverse().to_cols_array(),
            camera_pos: [frame.camera.position[0], frame.camera.position[1], frame.camera.position[2], 1.0],
            tower_body_normal: palette.tower_body,
            tower_text_normal: col4(palette.tower_text),
            highlight_normal: col4(palette.highlight),
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
                if scene_srgb { 0.0 } else { 1.0 },
            ],
            post: [s.crt, if final_srgb { 0.0 } else { 1.0 }, 0.0, 0.0],
            tower_body_siege: siege.tower_body,
            tower_text_siege: col4(siege.tower_text),
            highlight_siege: col4(siege.highlight),
            signal: [sw, sh, 1.0 / sw, 1.0 / sh],
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Member names of a `struct FrameUniform { .. }` block, in declaration order.
    fn struct_members(src: &str, rust_style: bool) -> Vec<String> {
        let marker = if rust_style {
            "pub struct FrameUniform {"
        } else {
            "struct FrameUniform {"
        };
        let body = src
            .split_once(marker)
            .unwrap_or_else(|| panic!("no FrameUniform in this source"))
            .1
            .split_once('}')
            .expect("unterminated FrameUniform")
            .0;
        body.lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with("//") {
                    return None;
                }
                let name = line.split(':').next()?.trim();
                Some(name.trim_start_matches("pub ").to_string())
            })
            .collect()
    }

    /// Every shader carries its own copy of the uniform layout, and the shaders read past `fx`,
    /// so a member added, removed or reordered on one side alone silently misreads another
    /// member's bytes (this is exactly how a stale struct once made the siege end read the CRT
    /// amount). Pin the copies to the Rust struct member for member.
    #[test]
    fn every_shader_frame_uniform_matches_the_rust_struct() {
        let rust = struct_members(include_str!("uniforms.rs"), true);
        assert!(rust.len() > 10, "parsed the wrong struct: {rust:?}");
        let sources: [(&str, &str); 9] = [
            ("towers", crate::shaders::TOWERS),
            ("floor", crate::shaders::FLOOR),
            ("pulses", crate::shaders::PULSES),
            ("bloom_prefilter", crate::shaders::BLOOM_PREFILTER),
            ("bloom_down", crate::shaders::BLOOM_DOWN),
            ("bloom_up", crate::shaders::BLOOM_UP),
            ("motion_blur", crate::shaders::MOTION_BLUR),
            ("composite", crate::shaders::COMPOSITE),
            ("crt", crate::shaders::CRT),
        ];
        for (name, src) in sources {
            assert_eq!(
                struct_members(src, false),
                rust,
                "{name}.wgsl's FrameUniform must match the Rust struct member for member"
            );
        }
        // The layout is only correct if every member is 16-byte aligned, which is why they are
        // all vec4/mat4.
        assert_eq!(std::mem::size_of::<FrameUniform>() % 16, 0);
        assert!(std::mem::size_of::<FrameUniform>() <= 16 * 1024);
    }
}
