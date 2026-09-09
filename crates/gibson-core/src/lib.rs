//! Host-facing Gibson facade.
//!
//! Owns the `wgpu::Instance`, the generated content (atlas + floor), the [`Scene`], and the
//! [`Renderer`], and exposes the single entry points hosts drive: `frame()` for continuous
//! rendering and `snapshot()` for one offscreen still. Time is host monotonic seconds; the first
//! call to `frame`/`snapshot` defines `t = 0`.
//!
//! Batch 0 scaffold: this wiring is already real. With the Batch 0 stub content generators and
//! scene it produces a haze-colored frame; Batch 2 verifies it end to end.

use gibson_render::{RenderError, Renderer};
use gibson_scene::Scene;
use gibson_types::Settings;

/// Where the renderer should present.
pub enum SurfaceTarget {
    /// A wgpu surface target (winit window, web canvas, …).
    Window(wgpu::SurfaceTarget<'static>),
    /// A raw platform surface handle (host-owned NSView/HWND/X11 window).
    Raw(wgpu::SurfaceTargetUnsafe),
    /// No surface: offscreen rendering / snapshots only.
    Offscreen,
}

/// Errors surfaced by [`Gibson`].
#[derive(Debug)]
pub enum GibsonError {
    /// The renderer failed (adapter/device/surface/render).
    Render(RenderError),
    /// A surface could not be created from the requested target.
    Surface(String),
}

impl std::fmt::Display for GibsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GibsonError::Render(e) => write!(f, "render error: {e}"),
            GibsonError::Surface(e) => write!(f, "surface error: {e}"),
        }
    }
}

impl std::error::Error for GibsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GibsonError::Render(e) => Some(e),
            GibsonError::Surface(_) => None,
        }
    }
}

impl From<RenderError> for GibsonError {
    fn from(e: RenderError) -> Self {
        GibsonError::Render(e)
    }
}

/// A running Gibson instance.
pub struct Gibson {
    scene: Scene,
    renderer: Renderer,
    settings: Settings,
    /// Host time of the first `frame`/`snapshot` call; defines the scene's `t = 0`.
    t0: Option<f64>,
}

impl Gibson {
    /// Build a full Gibson instance for `target` at `width × height` physical pixels.
    ///
    /// `scale` multiplies the render resolution (device pixel ratio × `settings.render_scale`
    /// on the host side). `settings` is clamped. A `seed` of 0 resolves to a time-derived seed.
    pub async fn new(
        target: SurfaceTarget,
        width: u32,
        height: u32,
        scale: f32,
        settings: Settings,
    ) -> Result<Gibson, GibsonError> {
        let settings = settings.clamped();
        let seed = if settings.seed != 0 {
            settings.seed
        } else {
            time_derived_seed()
        };
        log::info!("gibson-core: instance (seed {seed})");

        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = backends();
        let instance = wgpu::Instance::new(desc);

        let surface = match target {
            SurfaceTarget::Window(t) => Some(
                instance
                    .create_surface(t)
                    .map_err(|e| GibsonError::Surface(e.to_string()))?,
            ),
            SurfaceTarget::Raw(t) => Some(
                unsafe { instance.create_surface_unsafe(t) }
                    .map_err(|e| GibsonError::Surface(e.to_string()))?,
            ),
            SurfaceTarget::Offscreen => None,
        };

        let atlas = gibson_atlas::generate(seed);
        let floor = gibson_floor::generate(seed);
        let mut scene = Scene::new(&settings, seed);
        scene.set_block_ids(atlas.blocks_per_panel.clone());

        let renderer = Renderer::new(&instance, surface, width, height, scale, &atlas, &floor, &settings)
            .await
            .map_err(GibsonError::Render)?;

        Ok(Gibson {
            scene,
            renderer,
            settings,
            t0: None,
        })
    }

    /// Resize the presentation surface (and render target) in physical pixels.
    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        self.renderer.resize(width, height, scale);
    }

    /// Advance the scene to `time_seconds` (host monotonic; the first call defines `t = 0`) and
    /// present one frame on the surface.
    pub fn frame(&mut self, time_seconds: f64) -> Result<(), GibsonError> {
        let t = self.relative_time(time_seconds);
        self.scene.update(t, &self.settings);
        let frame = self.scene.frame(&self.settings);
        self.renderer.render(&frame)?;
        Ok(())
    }

    /// Advance the scene to `time_seconds` and return one offscreen still as tightly packed
    /// sRGB8 rows, top row first: `(width, height, rgba)`.
    pub fn snapshot(&mut self, time_seconds: f64) -> Result<(u32, u32, Vec<u8>), GibsonError> {
        let t = self.relative_time(time_seconds);
        self.scene.update(t, &self.settings);
        let frame = self.scene.frame(&self.settings);
        self.renderer.render_to_rgba(&frame).map_err(GibsonError::Render)
    }

    /// Current (clamped) settings.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Replace the settings (clamped). Batch 2 may also want to push them into the scene.
    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings.clamped();
    }

    fn relative_time(&mut self, time_seconds: f64) -> f64 {
        let t0 = *self.t0.get_or_insert(time_seconds);
        (time_seconds - t0).max(0.0)
    }
}

/// Which wgpu backends to request.
#[cfg(not(target_arch = "wasm32"))]
fn backends() -> wgpu::Backends {
    wgpu::Backends::PRIMARY
}

#[cfg(target_arch = "wasm32")]
fn backends() -> wgpu::Backends {
    wgpu::Backends::BROWSER_WEBGPU.union(wgpu::Backends::GL)
}

/// Time-derived seed used when `Settings::seed == 0`.
///
/// Native: nanoseconds since the Unix epoch. Wasm: we deliberately avoid a `js_sys` dependency
/// in this crate, so the fallback is a fixed constant; web hosts should pass an explicit seed
/// (e.g. from a query parameter) when they want per-visit variation.
#[cfg(not(target_arch = "wasm32"))]
fn time_derived_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x6A63_6F72_655F_3030)
}

#[cfg(target_arch = "wasm32")]
fn time_derived_seed() -> u64 {
    0x6A63_6F72_655F_3030
}
