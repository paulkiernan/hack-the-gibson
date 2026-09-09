//! Gibson wgpu renderer.
//!
//! Batch 0 scaffold: `Renderer::new` performs the real wgpu initialization (adapter, device,
//! surface configuration) so hosts can validate their surface path today; `render` clears the
//! surface to `frame.palette.haze` and presents; `render_to_rgba` clears an offscreen sRGB
//! texture to the same haze and reads the pixels back. Batch 1 replaces the frame body with the
//! full pipeline chain (floor, instanced towers, pulses, bloom, motion blur, composite) without
//! changing any public signature.
//!
//! WebGL2 constraints every pipeline in this crate must respect (Batch 1): no storage buffers,
//! no compute shaders, uniform buffers ≤ 16 KiB, per-instance data via instance vertex buffers,
//! `texture_2d_array<f32>` allowed, depth sampled with `textureLoad` on `texture_depth_2d`,
//! render targets `Rgba16Float` + `Depth32Float`, no MSAA.
//!
//! The eight WGSL sources are wired as compile-time constants in [`shaders`] (a missing file is
//! a compile error). Batch 1 builds its pipelines from them.
//!
//! # wgpu 30 API notes (deviation from the frozen contract's assumptions)
//! wgpu 30 removed `wgpu::SurfaceError`. `Surface::get_current_texture` now returns a
//! [`wgpu::CurrentSurfaceTexture`] status enum, and presentation happens through
//! `Queue::present(surface_texture)` (there is no `SurfaceTexture::present()`). Consequently
//! [`RenderError::Surface`] carries a `String` instead of `wgpu::SurfaceError` — see the report
//! of the Scaffold batch.

pub mod shaders {
    //! Compile-time WGSL sources. A missing file fails the build.

    /// Instanced translucent tower boxes with animated text layers.
    pub const TOWERS: &str = include_str!("shaders/towers.wgsl");
    /// PCB floor quad fragment shader (analytic SDF over the floor map).
    pub const FLOOR: &str = include_str!("shaders/floor.wgsl");
    /// Instanced lane pulse ribbons.
    pub const PULSES: &str = include_str!("shaders/pulses.wgsl");
    /// Bloom prefilter at half resolution.
    pub const BLOOM_PREFILTER: &str = include_str!("shaders/bloom_prefilter.wgsl");
    /// Bloom downsample.
    pub const BLOOM_DOWN: &str = include_str!("shaders/bloom_down.wgsl");
    /// Bloom upsample.
    pub const BLOOM_UP: &str = include_str!("shaders/bloom_up.wgsl");
    /// Motion blur by depth reprojection.
    pub const MOTION_BLUR: &str = include_str!("shaders/motion_blur.wgsl");
    /// Final composite: tonemap, chromatic aberration, grain, vignette.
    pub const COMPOSITE: &str = include_str!("shaders/composite.wgsl");
}

use gibson_types::{AtlasImage, FloorMap, FrameData, Settings};
use glam::Mat4;
use std::fmt;

/// Errors surfaced by the renderer.
#[derive(Debug)]
pub enum RenderError {
    /// No adapter compatible with the requested surface/power preference.
    NoAdapter,
    /// The adapter refused to create a device/queue.
    NoDevice(String),
    /// A surface operation failed (acquire, present, surface lost, …).
    Surface(String),
    /// Anything else.
    Other(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::NoAdapter => write!(f, "no compatible graphics adapter found"),
            RenderError::NoDevice(e) => write!(f, "could not create a graphics device: {e}"),
            RenderError::Surface(e) => write!(f, "surface error: {e}"),
            RenderError::Other(e) => write!(f, "renderer error: {e}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// The Gibson renderer: device/queue plus an optional presentable surface.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: Option<wgpu::Surface<'static>>,
    surface_format: Option<wgpu::TextureFormat>,
    width: u32,
    height: u32,
    scale: f32,
    /// Previous-frame view-projection matrix for motion-blur reprojection (Batch 1).
    pub(crate) prev_view_proj: Mat4,
    /// Marker so the atlas/floor/settings the stub receives are part of the API even before
    /// Batch 1 builds textures from them.
    _content: (),
}

fn scaled_dimensions(width: u32, height: u32, scale: f32) -> (u32, u32) {
    (
        ((width as f32) * scale).round().max(1.0) as u32,
        ((height as f32) * scale).round().max(1.0) as u32,
    )
}

impl Renderer {
    /// Initialize the GPU: request an adapter (compatible with `surface` when given), a device +
    /// queue, and configure the surface at `width·scale × height·scale`.
    ///
    /// When `surface` is `None` the renderer is offscreen-only: `render` is unavailable but
    /// `render_to_rgba` still works. `atlas`, `floor`, and `settings` are consumed by Batch 1 to
    /// build the content pipelines; the scaffold only holds them.
    pub async fn new(
        instance: &wgpu::Instance,
        surface: Option<wgpu::Surface<'static>>,
        width: u32,
        height: u32,
        scale: f32,
        atlas: &AtlasImage,
        floor: &FloorMap,
        settings: &Settings,
    ) -> Result<Renderer, RenderError> {
        let (width, height) = scaled_dimensions(width, height, scale);
        log::info!(
            "gibson-render: requesting adapter (surface: {}, size {width}x{height})",
            surface.is_some()
        );
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: surface.as_ref(),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|_| RenderError::NoAdapter)?;
        log::info!("gibson-render: adapter {:?}", adapter.get_info());

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("gibson-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| RenderError::NoDevice(e.to_string()))?;
        log::info!("gibson-render: device + queue created");

        let mut surface_format = None;
        if let Some(surf) = &surface {
            let caps = surf.get_capabilities(&adapter);
            // Prefer an sRGB format; fall back to the surface's first offered format.
            let format = caps
                .formats
                .iter()
                .copied()
                .find(|f| f.is_srgb())
                .unwrap_or(caps.formats[0]);
            let alpha_mode = caps
                .alpha_modes
                .iter()
                .copied()
                .find(|a| {
                    matches!(
                        a,
                        wgpu::CompositeAlphaMode::Auto | wgpu::CompositeAlphaMode::Opaque
                    )
                })
                .unwrap_or(caps.alpha_modes[0]);
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                color_space: wgpu::SurfaceColorSpace::Auto,
                width,
                height,
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surf.configure(&device, &config);
            log::info!(
                "gibson-render: surface configured ({format:?}, {width}x{height}, {alpha_mode:?})"
            );
            surface_format = Some(format);
        }

        let _ = (atlas, floor, settings); // Batch 1: build atlas/floor textures and pipelines here.

        Ok(Renderer {
            device,
            queue,
            surface,
            surface_format,
            width,
            height,
            scale,
            prev_view_proj: Mat4::IDENTITY,
            _content: (),
        })
    }

    /// Reconfigure the surface (or just the offscreen size) at `width·scale × height·scale`.
    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        let (w, h) = scaled_dimensions(width, height, scale);
        self.width = w;
        self.height = h;
        self.scale = scale;
        if let (Some(surf), Some(format)) = (&self.surface, self.surface_format) {
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                color_space: wgpu::SurfaceColorSpace::Auto,
                width: w,
                height: h,
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surf.configure(&self.device, &config);
        }
        log::debug!("gibson-render: resized to {w}x{h} (scale {scale})");
    }

    /// Render one frame to the surface. Scaffold: clear to the frame's haze color and present.
    pub fn render(&mut self, frame: &FrameData) -> Result<(), RenderError> {
        let Some(surf) = &self.surface else {
            return Err(RenderError::Other(
                "render() requires a surface; use render_to_rgba for offscreen output".into(),
            ));
        };
        let texture = match surf.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            // Transient conditions: skip this frame and try again on the next one.
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                log::debug!("gibson-render: surface busy/occluded; frame skipped");
                return Ok(());
            }
            other => {
                return Err(RenderError::Surface(format!(
                    "surface acquire failed: {other:?}"
                )));
            }
        };
        // Batch 1: the motion-blur pass reprojects this frame against prev_view_proj; the
        // scaffold only carries the field forward from Renderer::new (IDENTITY).
        let _ = self.prev_view_proj;
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gibson-clear"),
            });
        {
            let haze = frame.palette.haze;
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-clear-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: haze[0] as f64,
                            g: haze[1] as f64,
                            b: haze[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(texture);
        Ok(())
    }

    /// Render one frame offscreen and read back tightly packed sRGB8 rows, top row first.
    ///
    /// Scaffold: clears an offscreen `Rgba8UnormSrgb` texture to the haze color. The returned
    /// buffer is `width * height * 4` bytes, tightly packed.
    pub fn render_to_rgba(&mut self, frame: &FrameData) -> Result<(u32, u32, Vec<u8>), RenderError> {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let size = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gibson-offscreen"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gibson-offscreen-clear"),
            });
        {
            let haze = frame.palette.haze;
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-offscreen-clear-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: haze[0] as f64,
                            g: haze[1] as f64,
                            b: haze[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        // Read back with the 256-byte row alignment wgpu requires for buffers, then strip it.
        let bytes_per_row = align_up(self.width as usize * 4, 256);
        let buffer_size = (bytes_per_row * self.height as usize) as u64;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gibson-offscreen-readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row as u32),
                    rows_per_image: Some(self.height),
                },
            },
            size,
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = slice.get_mapped_range().expect("map readback buffer");
        let mut rgba = Vec::with_capacity(self.width as usize * self.height as usize * 4);
        for row in mapped.chunks(bytes_per_row).take(self.height as usize) {
            rgba.extend_from_slice(&row[..self.width as usize * 4]);
        }
        drop(mapped);
        buffer.unmap();
        Ok((self.width, self.height, rgba))
    }
}

/// Round `v` up to the next multiple of `align`.
fn align_up(v: usize, align: usize) -> usize {
    (v + align - 1) & !(align - 1)
}
