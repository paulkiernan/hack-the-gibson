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
        let color_usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
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
            color_a,
            color_b,
            depth,
            signal: signal_tex,
            signal_view,
            width,
            height,
        })
    }
}
