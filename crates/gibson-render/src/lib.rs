//! Gibson wgpu renderer.
//!
//! One HDR frame chain draws the whole Gibson:
//!
//! 1. floor quad (analytic PCB-trace SDF over the 96x96 floor map) into `color_a` + depth, and
//!    the fragment's depth into a second attachment (the "depth carry", see
//!    [`targets::SceneTargets::depth_color`]),
//! 2. instanced translucent tower boxes (atlas text, per-block animation) over it,
//! 3. additive lane pulse ribbons,
//! 4. bloom (prefilter + 13-tap down + 3x3 tent up; skipped at `bloom == 0`),
//! 5. motion blur by depth reprojection (skipped at `motion_blur == 0`),
//! 6. composite (ACES, chromatic aberration, grain, vignette) into the final target -- or, when
//!    the CRT pass is active, into a signal-resolution HDR texture,
//! 7. the CRT pass (Lottes scanline emulation) reconstructs that signal onto the final target at
//!    full output resolution.
//!
//! Steps 1 and 2-3 are separate render passes: the depth carry rides in the floor's pass, which
//! blends nothing, so the blending towers and pulses can keep their single-target pipelines
//! (see [`targets::depth_color_target`]).
//!
//! Steps 1-6 run at the *scene* size: the output size, unless `settings.crt > 0`, in which case
//! they run at a smaller signal resolution and step 7 expands it. That split is what makes the
//! CRT real -- nearest-fetch signal reconstruction and an output-pixel-scale phosphor mask are
//! only both correct when the emulated raster is a different, lower resolution than the display.
//! `crt = 0` keeps the scene at the output size and skips step 7 entirely (byte-identical to the
//! plain composite).
//!
//! WebGL2 constraints every pipeline in this crate respects: no storage buffers, no compute
//! shaders, one uniform buffer <= 16 KiB per binding, per-instance data via instance-step vertex
//! buffers, `texture_2d_array<f32>` allowed, no depth-texture reads (`textureLoad` on
//! `texture_depth_2d` has no GLSL equivalent, so the depth the motion blur reprojects from is
//! carried in an `R32Float` colour attachment -- see
//! [`targets::SceneTargets::depth_color`]), no independent blend (so no pipeline mixes blended
//! and unblended colour targets), `Rgba16Float` + `Depth32Float` targets, no MSAA. Resource
//! shapes (bind groups per pipeline, samplers, vertex strides) fit
//! `Limits::downlevel_webgl2_defaults()`.

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
    /// CRT emulation (Timothy Lottes' public-domain scanline shader) from the signal buffer.
    pub const CRT: &str = include_str!("shaders/crt.wgsl");
}

mod bloom;
mod floor;
mod post;
mod profile;
mod pulses;
mod targets;
mod towers;
mod uniforms;
mod util;

use bloom::{Bloom, BloomInputs};
use floor::Floor;
use gibson_types::{AtlasImage, FloorMap, FrameData, Settings};
use glam::Mat4;
use post::Post;
use pulses::Pulses;
use std::fmt;
use targets::{SceneTargets, HDR_FORMAT};
use towers::Towers;
use uniforms::{projection_matrix, view_matrix, FrameTargets, FrameUniform};
use util::pipeline_layout;

/// GPU resource census: what this crate has allocated, and how much (see [`GpuCensus`]).
pub use util::GpuCensus;

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
///
/// Several fields (`scene_bgl`, `scene_layout`, the atlas/floor textures) exist only to keep
/// GPU resources alive for the lifetime of the bind groups that reference them.
#[allow(dead_code)]
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: Option<wgpu::Surface<'static>>,
    surface_format: Option<wgpu::TextureFormat>,
    width: u32,
    height: u32,
    scale: f32,
    /// Size the HDR chain renders at: `(width, height)` unless the CRT pass is active, in which
    /// case it is `scene_size(..)` -- the CRT signal resolution.
    render_width: u32,
    render_height: u32,
    /// CRT amount of the last frame, so a resize rebuilds the chain at the right signal size.
    crt: f32,

    // Shared scene bindings: uniform + atlas + floor + samplers.
    scene_bgl: wgpu::BindGroupLayout,
    scene_layout: wgpu::PipelineLayout,
    atlas_tex: wgpu::Texture,
    atlas_sampler: wgpu::Sampler,
    floor_tex: wgpu::Texture,
    uniform_buf: wgpu::Buffer,
    scene_bg: wgpu::BindGroup,

    floor: Floor,
    towers: Towers,
    pulses: Pulses,
    targets: SceneTargets,
    bloom: Bloom,
    post: Post,

    // Bind groups that reference per-size views (rebuilt on resize).
    motion_bg: wgpu::BindGroup,
    composite_a_bg: wgpu::BindGroup,
    composite_b_bg: wgpu::BindGroup,
    /// CRT pass bind group (uniform + signal buffer); `None` while the CRT pass is inactive.
    crt_bg: Option<wgpu::BindGroup>,

    /// Previous-frame view-projection matrix for motion-blur reprojection.
    prev_view_proj: Mat4,
    has_prev: bool,

    /// Frames actually presented to the surface.
    presented: u64,
    /// Frames dropped because the surface was busy/occluded (no present happened).
    skipped: u64,
    /// Sub-counters of `skipped`, by reason: a `Timeout` means the drawable pool
    /// was starved; an `Occluded` means the layer/window is not displayable.
    skipped_timeout: u64,
    skipped_occluded: u64,

    /// Frames rendered through [`Renderer::run_chain`], presented or offscreen.
    frames: u64,
    /// GPU resources this renderer has allocated, and the totals as of the last report (so a
    /// change is logged on the frame it happens).
    census: GpuCensus,
    last_census: GpuCensus,
    /// Cached offscreen composite target + readback buffer, keyed by size
    /// (`render_to_rgba`, `profile_frame`). Allocated on first offscreen frame and on resize,
    /// never per frame.
    offscreen: Option<targets::OffscreenTarget>,

    /// GPU timestamp profiling state; `Some` only when `GIBSON_PROFILE` was set in the
    /// environment and the adapter supports `TIMESTAMP_QUERY` (the default device requests
    /// no features, so the WebGL2 envelope is untouched).
    profile: Option<profile::Profile>,
}

/// A renderer's presentation request: the size in logical pixels the host asked for, plus the
/// super-sampling scale its pixel budget allows on top of it.
///
/// [`Viewport::scaled_dimensions`] turns the two into the pixel size every target in the chain is
/// allocated at, which is what `Renderer::width`/`height` hold. They are one value rather than
/// three loose arguments because they are only ever meaningful, chosen and changed together, and
/// because the clamped result is what both the constructor and [`Renderer::resize`] must agree
/// on.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    /// Requested size in logical pixels.
    pub width: u32,
    pub height: u32,
    /// Super-sampling scale: `render_scale` after the host's on-screen pixel budget.
    pub scale: f32,
}

impl Viewport {
    /// The pixel size the renderer allocates: rounded, and never zero (a zero-sized target is a
    /// wgpu validation error).
    pub fn scaled_dimensions(self) -> (u32, u32) {
        (
            ((self.width as f32) * self.scale).round().max(1.0) as u32,
            ((self.height as f32) * self.scale).round().max(1.0) as u32,
        )
    }
}

/// Signal resolution of the CRT path as a fraction of the output.
///
/// A CRT is a raster display: the picture exists as a signal with a fixed number of lines and the
/// tube reconstructs it. Scanlines only read as scanlines when that signal has far fewer lines
/// than the display, so the whole scene chain renders here and `crt.wgsl` expands it.
///
/// 4/5 -- 864 lines at 1080p -- was picked by rendering the lane view and grading it, and the
/// fraction matters as much as the size. The reconstruction samples the signal at the *output*
/// pixel centre, so the row phases cycle through `(k + 1/2)·p/q mod 1` for a ratio `p/q`: the
/// raster is deep and grid-locked only when some phase lands near 0 (a row exactly between two
/// signal lines). At 1/2 the two phases are 1/4 and 3/4 -- mirror images that cancel, so a
/// 540-line signal renders with *no* scanlines at all; at 5/6 (900 lines) the nearest phase is
/// 5/12 and the raster is a third as deep. 4/5 puts one row of every five exactly in a gap with
/// 3.8x contrast between its brightest and darkest rows, and the mosaic text stays legible:
/// rendered at 1920x1080 the hero directory list on a panel reads the same at this resolution as
/// at native, and the tube's raster is clearly visible at the default amount.
const CRT_SIGNAL_RATIO: f32 = 0.8;

/// The size the HDR chain renders at for a given output size and CRT amount.
///
/// `crt = 0` keeps the chain at the output size (and the CRT pass is skipped entirely); any
/// amount above zero runs the scene at the signal resolution. The amount deliberately does not
/// scale the resolution: 0.35 and 1 share one signal grid, so turning the effect up deepens the
/// scanlines and the mask instead of also changing the picture's sharpness.
fn scene_size(width: u32, height: u32, crt: f32) -> (u32, u32) {
    if crt <= 0.0 {
        return (width, height);
    }
    (
        ((width as f32) * CRT_SIGNAL_RATIO).round().max(1.0) as u32,
        ((height as f32) * CRT_SIGNAL_RATIO).round().max(1.0) as u32,
    )
}

/// How often (in frames) the GPU resource census is logged even when it has not changed: 600
/// frames is ~10 s at 60 fps and ~40 s at the 15 fps this renderer sustains at 2.8 Mpx.
const CENSUS_INTERVAL: u64 = 600;

/// Sampler for the tower text atlas: linear in x/y, clamped.
fn atlas_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("gibson-atlas-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}

fn scene_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gibson-scene-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

/// Mip levels for the atlas: 256x768 -> 128x384 -> 64x192.
///
/// Mipmapped sampling is the standard real-time fix for minified texture reads: without it
/// every distant text fragment fetches four texels from a 256x768 x 64-layer array, which
/// thrashes the texture cache (and aliases) for the many mid/far faces a canyon view stacks.
/// Only the R (glyph coverage) channel is meaningful when filtered; the block metadata fetch
/// uses `textureLoad`, which always reads level 0.
const ATLAS_MIPS: u32 = 3;

fn upload_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &AtlasImage,
    census: &mut GpuCensus,
) -> wgpu::Texture {
    let tex = census.create_texture(
        device,
        &wgpu::TextureDescriptor {
            label: Some("gibson-atlas"),
            size: wgpu::Extent3d {
                width: atlas.width,
                height: atlas.height,
                depth_or_array_layers: atlas.layers,
            },
            mip_level_count: ATLAS_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
    );
    let bpr = atlas.width * 4;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &atlas.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bpr),
            rows_per_image: Some(atlas.height),
        },
        wgpu::Extent3d {
            width: atlas.width,
            height: atlas.height,
            depth_or_array_layers: atlas.layers,
        },
    );

    // Box-filter the remaining levels on the CPU (once, at startup) and upload them
    // layer-major, exactly like level 0.
    let layers = atlas.layers as usize;
    let mut src = atlas.rgba.clone();
    let (mut w, mut h) = (atlas.width, atlas.height);
    for mip in 1..ATLAS_MIPS {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let src_layer = (w * h * 4) as usize;
        let dst_layer = (nw * nh * 4) as usize;
        let mut dst = vec![0u8; dst_layer * layers];
        for layer in 0..layers {
            let s = &src[layer * src_layer..(layer + 1) * src_layer];
            let d = &mut dst[layer * dst_layer..(layer + 1) * dst_layer];
            for y in 0..nh as usize {
                for x in 0..nw as usize {
                    let mut acc = [0u32; 4];
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let sx = (x * 2 + dx).min(w as usize - 1);
                            let sy = (y * 2 + dy).min(h as usize - 1);
                            let i = (sy * w as usize + sx) * 4;
                            for c in 0..4 {
                                acc[c] += s[i + c] as u32;
                            }
                        }
                    }
                    let o = (y * nw as usize + x) * 4;
                    for c in 0..4 {
                        d[o + c] = (acc[c] / 4) as u8;
                    }
                }
            }
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &dst,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(nw * 4),
                rows_per_image: Some(nh),
            },
            wgpu::Extent3d {
                width: nw,
                height: nh,
                depth_or_array_layers: atlas.layers,
            },
        );
        src = dst;
        w = nw;
        h = nh;
    }
    log::debug!("gibson-render: atlas uploaded with {ATLAS_MIPS} mips");
    tex
}

fn upload_floor(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    floor: &FloorMap,
    census: &mut GpuCensus,
) -> wgpu::Texture {
    let tex = census.create_texture(
        device,
        &wgpu::TextureDescriptor {
            label: Some("gibson-floor-map"),
            size: wgpu::Extent3d {
                width: floor.cells,
                height: floor.cells,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
    );
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&floor.data),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(floor.cells * 4),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: floor.cells,
            height: floor.cells,
            depth_or_array_layers: 1,
        },
    );
    tex
}

impl Renderer {
    /// Initialize the GPU: adapter/device/surface plus the full pipeline set.
    ///
    /// The surface is configured -- and every target allocated -- at
    /// `viewport.scaled_dimensions()`.
    pub async fn new(
        instance: &wgpu::Instance,
        surface: Option<wgpu::Surface<'static>>,
        viewport: Viewport,
        atlas: &AtlasImage,
        floor: &FloorMap,
        settings: &Settings,
    ) -> Result<Renderer, RenderError> {
        let (width, height) = viewport.scaled_dimensions();
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

        // Profiling is opt-in via the environment and only when the adapter can do GPU
        // timestamps inside command encoders; the default device still requests no features.
        let profile_features = wgpu::Features::TIMESTAMP_QUERY;
        let want_profile = cfg!(not(target_arch = "wasm32"))
            && std::env::var_os("GIBSON_PROFILE").is_some()
            && adapter.features().contains(profile_features);
        let required_features = if want_profile {
            log::info!("gibson-render: GPU timestamp profiling enabled (GIBSON_PROFILE)");
            profile_features
        } else {
            wgpu::Features::empty()
        };

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("gibson-device"),
                required_features,
                // Keep every resource shape inside the WebGL2 envelope so the same pipelines
                // work on the downlevel web target... but let the resolution limits come from
                // the adapter: hosts render at physical pixels (e.g. 2x Retina can exceed the
                // WebGL2-envelope's 2048 max texture dimension).
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| RenderError::NoDevice(e.to_string()))?;
        log::info!("gibson-render: device + queue created");

        // Any validation error raised *outside* an error scope (everything after startup: a
        // frame-time bind group, a lost device, a shader that only fails on one backend) is
        // otherwise completely invisible on the web - the canvas simply stops changing and no
        // host callback ever fires. Report it where a human will see it: the browser console.
        // Errors raised inside the scopes below still go to those scopes.
        device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
            log::error!("gibson-render: uncaptured device error: {error}");
            #[cfg(target_arch = "wasm32")]
            web_sys::console::error_1(
                &format!("gibson-render: uncaptured device error: {error}").into(),
            );
        }));

        let mut surface_format = None;
        if let Some(surf) = &surface {
            let caps = surf.get_capabilities(&adapter);
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
            surface_format = Some(format);
        }
        let _ = settings;
        // Every GPU resource this renderer owns is counted as it is built (see [`GpuCensus`]).
        let mut census = GpuCensus::default();
        let profile = want_profile.then(|| profile::Profile::new(&device, &queue, &mut census));
        // The scene chain runs at the CRT signal resolution when the tube is on, at the output
        // size when it is off; `settings` decides which for the first frame.
        let crt = settings.crt;
        let (render_width, render_height) = scene_size(width, height, crt);

        // Everything below creates pipelines/resources; capture any validation error (WGSL
        // compile failures included) and surface it as RenderError so hosts can report it.
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

        let scene_bgl = scene_bgl(&device);
        let scene_layout = pipeline_layout(&device, "gibson-scene-layout", &scene_bgl);
        let atlas_sampler = atlas_sampler(&device);

        let atlas_tex = upload_atlas(&device, &queue, atlas, &mut census);
        let floor_tex = upload_floor(&device, &queue, floor, &mut census);

        let uniform_buf = census.create_buffer(
            &device,
            &wgpu::BufferDescriptor {
                label: Some("gibson-uniform"),
                size: std::mem::size_of::<FrameUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );

        let atlas_view = atlas_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let floor_view = floor_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gibson-scene-bg"),
            layout: &scene_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&floor_view),
                },
            ],
        });

        let floor_pass = Floor::new(
            &device,
            &scene_layout,
            HDR_FORMAT,
            targets::DEPTH_FORMAT,
            &mut census,
        )?;
        let towers_pass = Towers::new(
            &device,
            &scene_layout,
            HDR_FORMAT,
            targets::DEPTH_FORMAT,
            &mut census,
        )?;
        let pulses_pass = Pulses::new(
            &device,
            &scene_layout,
            HDR_FORMAT,
            targets::DEPTH_FORMAT,
            &mut census,
        )?;

        let post = Post::new(&device, &mut census);
        let mut bloom = Bloom::new(&device, &mut census);

        let targets =
            SceneTargets::new(&device, render_width, render_height, crt > 0.0, &mut census)?;
        let post_sampler = post.sampler.clone();
        let bloom0 = build_bloom_and_groups(
            &device,
            &mut bloom,
            &uniform_buf,
            &post_sampler,
            &post,
            &targets,
            &mut census,
        );
        let crt_bg = targets
            .signal_view
            .as_ref()
            .map(|v| build_crt_bind_group(&device, &uniform_buf, &post, v));

        if let Some(err) = error_scope.pop().await {
            return Err(RenderError::Other(format!(
                "pipeline/resource validation failed: {err}"
            )));
        }

        Ok(Renderer {
            device,
            queue,
            surface,
            surface_format,
            width,
            height,
            scale: viewport.scale,
            render_width,
            render_height,
            crt,
            scene_bgl,
            scene_layout,
            atlas_tex,
            atlas_sampler,
            floor_tex,
            uniform_buf,
            scene_bg,
            floor: floor_pass,
            towers: towers_pass,
            pulses: pulses_pass,
            targets,
            bloom,
            post,
            motion_bg: bloom0.0,
            composite_a_bg: bloom0.1,
            composite_b_bg: bloom0.2,
            crt_bg,
            prev_view_proj: Mat4::IDENTITY,
            has_prev: false,
            presented: 0,
            skipped: 0,
            skipped_timeout: 0,
            skipped_occluded: 0,
            frames: 0,
            census,
            last_census: census,
            offscreen: None,
            profile,
        })
    }

    /// Reconfigure the surface (or just the offscreen size) to `viewport`'s scaled dimensions.
    ///
    /// Idempotent: a call that asks for the size and scale the renderer is already at does
    /// nothing. It has to be, because the work here is the whole size-dependent resource set --
    /// the HDR targets, the bloom chain and every view-dependent bind group -- and hosts
    /// re-assert their size far more often than it changes (a layout pass, a backing-scale
    /// notification, a display change). Rebuilding that set for a size that did not move would
    /// allocate and drop ten full-size textures plus several dozen bind groups per call, which
    /// is both a stall and the exact shape of "the renderer is leaking GPU memory" in a census.
    /// That is not hypothetical: the desktop host asserts its size on its very first frame, and
    /// the rebuild put its startup census at 22 textures / 181.5 MiB instead of 12 / 122.3 MiB.
    pub fn resize(&mut self, viewport: Viewport) {
        let scale = viewport.scale;
        let (w, h) = viewport.scaled_dimensions();
        if (w, h) == (self.width, self.height) && scale == self.scale {
            return;
        }
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
        self.rebuild_size_dependent();
        log::debug!("gibson-render: resized to {w}x{h} (scale {scale})");
    }

    /// Recreate the HDR targets, bloom chain and view-dependent bind groups at the current
    /// scene size (the output size, or the CRT signal resolution when the tube is on).
    fn rebuild_size_dependent(&mut self) {
        let (rw, rh) = scene_size(self.width, self.height, self.crt);
        let signal = self.crt > 0.0;
        let Ok(targets) = SceneTargets::new(&self.device, rw, rh, signal, &mut self.census) else {
            log::error!("gibson-render: failed to rebuild targets at {rw}x{rh}");
            return;
        };
        self.render_width = rw;
        self.render_height = rh;
        self.targets = targets;
        self.bloom.rebuild(
            &self.device,
            rw,
            rh,
            BloomInputs {
                uniform: &self.uniform_buf,
                sampler: &self.post.sampler,
                src_view: &self.targets.view_a,
            },
            &mut self.census,
        );
        let (mbg, cbg_a, cbg_b) = build_view_bind_groups(
            &self.device,
            &self.uniform_buf,
            &self.bloom,
            &self.post.sampler,
            &self.post,
            &self.targets,
        );
        self.motion_bg = mbg;
        self.composite_a_bg = cbg_a;
        self.composite_b_bg = cbg_b;
        self.crt_bg = self
            .targets
            .signal_view
            .as_ref()
            .map(|v| build_crt_bind_group(&self.device, &self.uniform_buf, &self.post, v));
    }

    /// Resize the HDR chain if this frame's CRT amount changes the signal resolution. Cheap and
    /// idempotent -- the sizes only move when the amount crosses zero (or the output resizes).
    fn ensure_scene_size(&mut self, crt: f32) {
        let want = scene_size(self.width, self.height, crt);
        if want == (self.render_width, self.render_height) && (crt > 0.0) == (self.crt > 0.0) {
            self.crt = crt;
            return;
        }
        self.crt = crt;
        self.rebuild_size_dependent();
    }

    /// Render one frame to the surface.
    pub fn render(&mut self, frame: &FrameData) -> Result<(), RenderError> {
        let Some(surf) = &self.surface else {
            return Err(RenderError::Other(
                "render() requires a surface; use render_to_rgba for offscreen output".into(),
            ));
        };
        let texture = match surf.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout => {
                log::debug!("gibson-render: surface timeout; frame skipped");
                self.skipped += 1;
                self.skipped_timeout += 1;
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                log::debug!("gibson-render: surface occluded; frame skipped");
                self.skipped += 1;
                self.skipped_occluded += 1;
                return Ok(());
            }
            other @ (wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost) => {
                // Display-mode change or GPU reset: reconfigure the surface at the current size
                // and try exactly once before surfacing an error. Without this a screensaver
                // that stops its loop after repeated failures would black-screen on a mode
                // change until its idle self-terminate kicks in.
                log::info!("gibson-render: surface {other:?}; reconfiguring and retrying once");
                if let (Some(surf), Some(format)) = (&self.surface, self.surface_format) {
                    let config = wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format,
                        color_space: wgpu::SurfaceColorSpace::Auto,
                        width: self.width,
                        height: self.height,
                        present_mode: wgpu::PresentMode::AutoVsync,
                        alpha_mode: wgpu::CompositeAlphaMode::Auto,
                        view_formats: vec![],
                        desired_maximum_frame_latency: 2,
                    };
                    surf.configure(&self.device, &config);
                    match surf.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(t)
                        | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                        wgpu::CurrentSurfaceTexture::Timeout => {
                            log::debug!(
                                "gibson-render: surface timeout after reconfigure; frame skipped"
                            );
                            self.skipped += 1;
                            self.skipped_timeout += 1;
                            return Ok(());
                        }
                        wgpu::CurrentSurfaceTexture::Occluded => {
                            log::debug!(
                                "gibson-render: surface occluded after reconfigure; frame skipped"
                            );
                            self.skipped += 1;
                            self.skipped_occluded += 1;
                            return Ok(());
                        }
                        other => {
                            return Err(RenderError::Surface(format!(
                                "surface acquire failed after reconfigure: {other:?}"
                            )));
                        }
                    }
                } else {
                    return Err(RenderError::Surface(
                        "surface lost but no surface to reconfigure".into(),
                    ));
                }
            }
            other => {
                return Err(RenderError::Surface(format!(
                    "surface acquire failed: {other:?}"
                )));
            }
        };
        let format = self
            .surface_format
            .ok_or_else(|| RenderError::Other("no surface format".into()))?;
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut prof = self.profile.take();
        let result = self.run_chain(frame, &view, format, prof.as_mut());
        self.profile = prof;
        result?;
        self.queue.present(texture);
        self.presented += 1;
        Ok(())
    }

    /// (presented, skipped) frame counters. `presented` counts frames actually presented to
    /// the surface; `skipped` counts frames dropped because the surface reported `Timeout` or
    /// `Occluded` (the host should back off when it sees the skip counter advance). Offscreen
    /// `render_to_rgba` frames never touch either counter.
    pub fn present_stats(&self) -> (u64, u64) {
        (self.presented, self.skipped)
    }

    /// `(skipped_timeout, skipped_occluded)`: why frames were dropped. A
    /// `Timeout` points at a starved drawable pool (the GPU/host is behind);
    /// an `Occluded` points at a layer or window the display cannot show.
    pub fn skip_breakdown(&self) -> (u64, u64) {
        (self.skipped_timeout, self.skipped_occluded)
    }

    /// GPU resources this renderer has allocated, with their byte sizes.
    ///
    /// The frame path allocates nothing in the steady state, so these totals stop moving after
    /// startup and stay put across resizes; a run whose totals keep climbing is creating (and,
    /// since nothing is freed, accumulating) GPU resources per frame. Measured on the desktop
    /// host: flat at `textures=22 buffers=10` from frame 1 to frame 34800 over 31 minutes. See
    /// [`GpuCensus`] for why this is counted in-process rather than read from Metal or `ioreg`.
    pub fn gpu_census(&self) -> GpuCensus {
        self.census
    }

    /// Render one frame offscreen and read back tightly packed sRGB8 rows, top row first.
    ///
    /// The target and its staging buffer are cached per render size (see
    /// [`targets::OffscreenTarget`]), so a host that renders offscreen frame after frame --
    /// the snapshot host renders one per simulated 1/60 s step -- allocates nothing per frame.
    pub fn render_to_rgba(
        &mut self,
        frame: &FrameData,
    ) -> Result<(u32, u32, Vec<u8>), RenderError> {
        let (width, height) = (self.width, self.height);
        self.ensure_offscreen();
        let target = self
            .offscreen
            .as_ref()
            .ok_or_else(|| RenderError::Other("offscreen target missing".into()))?;
        let texture = target.texture.clone();
        let view = target.view.clone();
        let readback = target.readback.clone();
        let bytes_per_row = target.bytes_per_row;
        let mut prof = self.profile.take();
        let result = self.run_chain(frame, &view, targets::OFFSCREEN_FORMAT, prof.as_mut());
        self.profile = prof;
        result?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gibson-readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = slice.get_mapped_range().expect("map readback buffer");
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for row in mapped.chunks(bytes_per_row as usize).take(height as usize) {
            rgba.extend_from_slice(&row[..width as usize * 4]);
        }
        drop(mapped);
        readback.unmap();
        Ok((width, height, rgba))
    }

    /// Build the cached offscreen target if it is missing or the render size moved.
    fn ensure_offscreen(&mut self) {
        let stale = match &self.offscreen {
            Some(o) => o.width != self.width || o.height != self.height,
            None => true,
        };
        if stale {
            self.offscreen = Some(targets::OffscreenTarget::new(
                &self.device,
                self.width,
                self.height,
                &mut self.census,
            ));
        }
    }

    /// Run the full HDR chain and composite into `final_view`.
    fn run_chain(
        &mut self,
        frame: &FrameData,
        final_view: &wgpu::TextureView,
        final_format: wgpu::TextureFormat,
        mut profile: Option<&mut profile::Profile>,
    ) -> Result<(), RenderError> {
        // The CRT amount decides the size the whole scene chain runs at; a change rebuilds the
        // targets/bloom/bind groups once, before anything is encoded this frame.
        let crt_on = frame.settings.crt > 0.0;
        self.ensure_scene_size(frame.settings.crt);

        // Camera matrices.
        let vp =
            projection_matrix(&frame.camera, self.width, self.height) * view_matrix(&frame.camera);
        let prev = if self.has_prev {
            self.prev_view_proj
        } else {
            // First frame: use the frame's own prev pose so nothing jumps.
            projection_matrix(&frame.prev_camera, self.width, self.height)
                * view_matrix(&frame.prev_camera)
        };
        // The composite writes the signal buffer when the CRT pass follows (an Rgba16Float
        // target, so it encodes sRGB itself) and the final target otherwise.
        let scene_srgb = if crt_on {
            false
        } else {
            final_format.is_srgb()
        };
        let uniform = FrameUniform::new(
            vp,
            prev,
            frame,
            FrameTargets {
                width: self.width,
                height: self.height,
                scene: (self.render_width, self.render_height),
                scene_srgb,
                final_srgb: final_format.is_srgb(),
            },
        );
        self.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniform));

        // Instance + geometry uploads.
        self.towers
            .upload(&self.device, &self.queue, frame.towers, &mut self.census);
        self.pulses
            .upload(&self.device, &self.queue, frame.pulses, &mut self.census);
        self.floor.ensure_grid(
            &self.device,
            &self.queue,
            frame.settings.grid as f32,
            &mut self.census,
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gibson-frame"),
            });

        // --- Scene: floor, towers, pulses into color_a + depth. ---
        // When profiling, the three scene stages run as separate passes (towers/pulses load the
        // floor's color + depth instead of clearing) so their cost is attributable; the
        // production path keeps the single pass.
        if profile.is_some() {
            let tw = match profile.as_deref_mut() {
                Some(p) => p.pass("scene floor"),
                None => None,
            };
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-floor-pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.targets.view_a,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.targets.depth_color_view,
                        depth_slice: None,
                        resolve_target: None,
                        // `r` is the only channel an `R32Float` attachment stores; 1.0 matches the
                        // depth clear below, so "no geometry here" reads the same from either
                        // source (the motion blur treats depth >= 1.0 as background).
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 1.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_bind_group(0, &self.scene_bg, &[]);
            rp.set_pipeline(&self.floor.pipeline);
            self.floor.draw(&mut rp);
            drop(rp);

            let tw = match profile.as_deref_mut() {
                Some(p) => p.pass("scene towers"),
                None => None,
            };
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-towers-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.view_a,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_bind_group(0, &self.scene_bg, &[]);
            rp.set_pipeline(&self.towers.pipeline);
            self.towers.draw(&mut rp, frame.towers.len() as u32);
            drop(rp);

            let tw = match profile.as_deref_mut() {
                Some(p) => p.pass("scene pulses"),
                None => None,
            };
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-pulses-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.view_a,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_bind_group(0, &self.scene_bg, &[]);
            rp.set_pipeline(&self.pulses.pipeline);
            self.pulses.draw(&mut rp, frame.pulses.len() as u32);
            drop(rp);
        } else {
            // Two passes rather than one, because the floor pass carries depth in a second
            // colour attachment and a pipeline whose colour targets disagree on blend or write
            // mask needs `INDEPENDENT_BLEND`, which WebGL2 does not have. The floor blends
            // nothing, so the carry rides there; the towers and pulses blend, so they draw over
            // the floor in the next pass, which does not bind the carry at all.
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-floor-pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.targets.view_a,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.targets.depth_color_view,
                        depth_slice: None,
                        resolve_target: None,
                        // `r` is the only channel an `R32Float` attachment stores; 1.0 matches the
                        // depth clear below, so "no geometry here" reads the same from either
                        // source (the motion blur treats depth >= 1.0 as background).
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 1.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_bind_group(0, &self.scene_bg, &[]);
            rp.set_pipeline(&self.floor.pipeline);
            self.floor.draw(&mut rp);
            drop(rp);

            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-towers-pulses-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.view_a,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_bind_group(0, &self.scene_bg, &[]);
            rp.set_pipeline(&self.towers.pipeline);
            self.towers.draw(&mut rp, frame.towers.len() as u32);
            rp.set_pipeline(&self.pulses.pipeline);
            self.pulses.draw(&mut rp, frame.pulses.len() as u32);
        }

        // --- Bloom (skipped when settings.bloom == 0). ---
        if frame.settings.bloom > 0.0 {
            self.bloom.run(&mut encoder, profile.as_deref_mut());
        }

        // --- Motion blur: color_a + depth -> color_b (skipped when motion_blur == 0). ---
        let motion_on = frame.settings.motion_blur > 0.0;
        if motion_on {
            let tw = match profile.as_deref_mut() {
                Some(p) => p.pass("motion blur"),
                None => None,
            };
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-motion-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.view_b,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(self.post.motion());
            rp.set_bind_group(0, &self.motion_bg, &[]);
            rp.set_vertex_buffer(0, self.post.triangle().slice(..));
            rp.draw(0..3, 0..1);
        }

        // --- Composite: tonemap + grain + vignette, into the final target or (when the CRT pass
        // follows) the signal buffer the tube reconstructs. ---
        {
            let src_bg = if motion_on {
                &self.composite_b_bg
            } else {
                &self.composite_a_bg
            };
            let (target_view, target_format) = if crt_on {
                (
                    self.targets
                        .signal_view
                        .as_ref()
                        .expect("signal buffer when the CRT pass is active"),
                    HDR_FORMAT,
                )
            } else {
                (final_view, final_format)
            };
            let pipeline = self
                .post
                .composite_pipeline(&self.device, target_format)
                .clone();
            let tw = match profile.as_deref_mut() {
                Some(p) => p.pass("composite"),
                None => None,
            };
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-composite-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&pipeline);
            rp.set_bind_group(0, src_bg, &[]);
            rp.set_vertex_buffer(0, self.post.triangle().slice(..));
            rp.draw(0..3, 0..1);
        }

        // --- CRT pass: reconstruct the signal onto the display at full output resolution. ---
        if crt_on {
            let bg = self
                .crt_bg
                .as_ref()
                .expect("CRT bind group when the CRT pass is active");
            let pipeline = self.post.crt_pipeline(&self.device, final_format).clone();
            let tw = match profile.as_deref_mut() {
                Some(p) => p.pass("crt"),
                None => None,
            };
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-crt-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: final_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&pipeline);
            rp.set_bind_group(0, bg, &[]);
            rp.set_vertex_buffer(0, self.post.triangle().slice(..));
            rp.draw(0..3, 0..1);
        }

        // Resolve the timestamp queries inside this submission so the caller can read them
        // back right after the frame without a second submit.
        if let Some(p) = profile.as_deref_mut() {
            // Metal only records an end-of-pass timestamp at the next pass boundary, so the
            // composite needs one trailing trivial pass to be measured at all.
            let view = p.tail_view().clone();
            let tw = p.pass("tail marker");
            let _rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gibson-profile-tail"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: tw,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        if let Some(p) = profile.as_deref() {
            p.resolve(&mut encoder);
        }

        self.queue.submit(Some(encoder.finish()));
        self.prev_view_proj = vp;
        self.has_prev = true;
        self.frames += 1;
        self.report_census();
        Ok(())
    }

    /// Log the GPU resource census periodically, and on the frame it changes.
    ///
    /// The steady state has one fixed set of targets and buffers, so the logs of a long run
    /// come out with identical totals; a frame that allocated anything would show up as an
    /// increase on that very line rather than hiding between samples. This is the in-process
    /// evidence behind "the frame path does not allocate per frame" (see [`util::GpuCensus`]).
    fn report_census(&mut self) {
        let now = self.census;
        if !self.frames.is_multiple_of(CENSUS_INTERVAL) && now == self.last_census {
            return;
        }
        log::info!("gibson-render: frames={} gpu {now}", self.frames);
        self.last_census = now;
    }

    /// Render one frame offscreen with GPU timestamps and return per-pass milliseconds.
    ///
    /// Only available when the renderer was created with `GIBSON_PROFILE` set and the adapter
    /// supports `TIMESTAMP_QUERY`. The composite writes into a full-size offscreen texture
    /// (same pixel count as a real frame) so its fill-rate cost is measured honestly; the
    /// pixels are never read back, so the only GPU sync is the timestamp buffer.
    pub fn profile_frame(
        &mut self,
        frame: &FrameData,
    ) -> Result<Vec<(&'static str, f64)>, RenderError> {
        let Some(mut profile) = self.profile.take() else {
            return Err(RenderError::Other(
                "profiling disabled: create the renderer with GIBSON_PROFILE=1".into(),
            ));
        };
        profile.reset();
        self.ensure_offscreen();
        let view = self
            .offscreen
            .as_ref()
            .ok_or_else(|| RenderError::Other("offscreen target missing".into()))?
            .view
            .clone();
        let result = self.run_chain(frame, &view, targets::OFFSCREEN_FORMAT, Some(&mut profile));
        let report = match result {
            Ok(()) => profile.read(&self.device),
            Err(e) => {
                self.profile = Some(profile);
                return Err(e);
            }
        };
        self.profile = Some(profile);
        Ok(report)
    }
}

/// Build the bloom chain and every bind group that depends on the current targets.
fn build_bloom_and_groups(
    device: &wgpu::Device,
    bloom: &mut Bloom,
    uniform: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    post: &Post,
    targets: &SceneTargets,
    census: &mut GpuCensus,
) -> (wgpu::BindGroup, wgpu::BindGroup, wgpu::BindGroup) {
    // The bloom chain shadows the scene size, and `targets` already carries it.
    bloom.rebuild(
        device,
        targets.width,
        targets.height,
        BloomInputs {
            uniform,
            sampler,
            src_view: &targets.view_a,
        },
        census,
    );
    build_view_bind_groups(device, uniform, bloom, sampler, post, targets)
}

/// (motion_bg, composite_bg_a, composite_bg_b)
fn build_view_bind_groups(
    device: &wgpu::Device,
    uniform: &wgpu::Buffer,
    bloom: &Bloom,
    sampler: &wgpu::Sampler,
    post: &Post,
    targets: &SceneTargets,
) -> (wgpu::BindGroup, wgpu::BindGroup, wgpu::BindGroup) {
    let bloom0 = bloom.level0_view();
    let mk = |layout: &wgpu::BindGroupLayout,
              color: &wgpu::TextureView,
              bloom_view: Option<&wgpu::TextureView>,
              depth_carry: bool| {
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(color),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ];
        if let Some(bv) = bloom_view {
            entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(bv),
            });
        } else if depth_carry {
            entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&targets.depth_color_view),
            });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gibson-view-bg"),
            layout,
            entries: &entries,
        })
    };
    let motion_bg = mk(&post.motion_bgl, &targets.view_a, None, true);
    let composite_a_bg = mk(&post.composite_bgl, &targets.view_a, bloom0, false);
    let composite_b_bg = mk(&post.composite_bgl, &targets.view_b, bloom0, false);
    (motion_bg, composite_a_bg, composite_b_bg)
}

/// The CRT pass's bind group: the frame uniform plus the composite's signal buffer.
/// `crt.wgsl` fetches that texture with explicit `textureLoad`s, so it binds no sampler.
fn build_crt_bind_group(
    device: &wgpu::Device,
    uniform: &wgpu::Buffer,
    post: &Post,
    signal: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gibson-crt-bg"),
        layout: &post.crt_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(signal),
            },
        ],
    })
}

/// Round `v` up to the next multiple of `align`.
fn align_up(v: usize, align: usize) -> usize {
    (v + align - 1) & !(align - 1)
}
