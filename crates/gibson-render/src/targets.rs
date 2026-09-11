//! Full-resolution HDR render targets shared by the whole frame chain.
//!
//! The scene (floor, towers, pulses) draws into `color_a`; the motion-blur pass reads
//! `color_a` and `depth` and writes `color_b`; the composite pass samples `color_b`
//! (or `color_a` when motion blur is off) plus the bloom chain and writes the final
//! presentable / offscreen image. `depth` doubles as the motion-blur reprojection
//! source, so it needs `TEXTURE_BINDING` as well as `RENDER_ATTACHMENT`.

use crate::RenderError;
use wgpu::TextureFormat;

/// The render-target formats the whole chain is built around.
pub const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
pub const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;

/// Format of the depth *carry* (see [`SceneTargets::depth_color`]).
///
/// `R32Float` rather than one of the `Rgba16Float` targets the rest of the chain uses: the
/// value is an NDC depth in [0, 1] and half precision is nowhere near enough to reconstruct a
/// world position from. An f16 ulp just below 1.0 is ~4.9e-4, which with this camera's 0.5/2200
/// near/far planes is ~60 world units of error at 500 units out - the motion-blur reprojection
/// would smear in essentially random directions.
pub const DEPTH_COLOR_FORMAT: TextureFormat = TextureFormat::R32Float;

/// Ownership holder: the views are what the passes use; the textures must stay alive with
/// them, and the stored dimensions mirror the configured target.
#[allow(dead_code)]
pub struct SceneTargets {
    pub color_a: wgpu::Texture,
    pub view_a: wgpu::TextureView,
    pub color_b: wgpu::Texture,
    pub view_b: wgpu::TextureView,
    pub depth: wgpu::Texture,
    pub depth_view: wgpu::TextureView,
    /// The depth of the last fragment that wrote depth, as a colour value, for the motion-blur
    /// pass to reproject from.
    ///
    /// The natural source for that is the depth texture, but `textureLoad` on
    /// `texture_depth_2d` has no GLSL equivalent - naga rejects it outright ("WGSL `textureLoad`
    /// from depth textures is not supported in GLSL") - so the depth texel fetch compiles only
    /// on Metal/Vulkan/DX12/WebGPU and takes the whole renderer down on the WebGL2 fallback.
    /// Carrying the identical value through a colour attachment is expressible on every backend.
    pub depth_color: wgpu::Texture,
    pub depth_color_view: wgpu::TextureView,
    /// Composite output for the CRT pass to reconstruct. Present only when the CRT pass is
    /// active (the scene then renders at the signal resolution and this texture matches it).
    pub signal: Option<wgpu::Texture>,
    pub signal_view: Option<wgpu::TextureView>,
    pub width: u32,
    pub height: u32,
}

impl SceneTargets {
    /// Create (or recreate) the HDR color pair plus depth at `width x height`, and -- when
    /// `signal` is set -- the composite's signal buffer for the CRT pass at the same size.
    pub fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        signal: bool,
    ) -> Result<SceneTargets, RenderError> {
        let color_usage =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let mk = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: color_usage,
                view_formats: &[],
            })
        };
        let color_a = mk("gibson-hdr-a");
        let color_b = mk("gibson-hdr-b");
        let (signal_tex, signal_view) = if signal {
            let tex = mk("gibson-hdr-signal");
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            (Some(tex), Some(view))
        } else {
            (None, None)
        };
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gibson-depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            // Never sampled: the motion-blur pass reprojects from `depth_color` instead.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gibson-depth-color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        if width == 0 || height == 0 {
            return Err(RenderError::Other("zero render target size".into()));
        }
        Ok(SceneTargets {
            view_a: color_a.create_view(&wgpu::TextureViewDescriptor::default()),
            view_b: color_b.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_color_view: depth_color.create_view(&wgpu::TextureViewDescriptor::default()),
            color_a,
            color_b,
            depth,
            depth_color,
            signal: signal_tex,
            signal_view,
            width,
            height,
        })
    }
}

/// The second colour attachment of the floor pipeline.
///
/// Only the floor pass has it: a pipeline whose colour targets disagree on blend or write mask
/// requires `INDEPENDENT_BLEND`, which WebGL2 does not have, and the floor is the one scene
/// pass that blends nothing. Towers and pulses do blend, so they draw over the floor in the
/// next pass, where this attachment is not bound and the carry survives untouched.
pub fn depth_color_target() -> Option<wgpu::ColorTargetState> {
    Some(wgpu::ColorTargetState {
        format: DEPTH_COLOR_FORMAT,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    })
}
