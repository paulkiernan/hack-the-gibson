//! Host-facing Gibson facade.
//!
//! Owns the `wgpu::Instance`, the generated content (atlas + floor), the [`Scene`], and the
//! [`Renderer`], and exposes the single entry points hosts drive: `frame()` for continuous
//! rendering and `snapshot()` for one offscreen still. Time is host monotonic seconds; the first
//! call to `frame`/`snapshot` defines `t = 0`.
//!
//! Batch 0 scaffold: this wiring is already real. With the Batch 0 stub content generators and
//! scene it produces a haze-colored frame; Batch 2 verifies it end to end.

use gibson_render::{RenderError, Renderer, Viewport};
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
    /// True when this instance presents to a surface (Window/Raw); offscreen instances are
    /// never subject to the on-screen pixel budget.
    on_screen: bool,
    /// Last effective scale handed to the renderer (so resize only logs real changes).
    last_scale: f32,
}

/// The scale actually handed to the renderer: the caller's scale times, for on-screen targets
/// only, an automatic cap that keeps the render target inside [`MAX_ON_SCREEN_PIXELS`].
///
/// The budget is applied to the *physical* pixel count the caller's scale would produce
/// (`width * scale` by `height * scale`), not to the logical size: a 1280x800 window on a 2x
/// display renders 2560x1600 physical pixels and must be capped like any other 4.1 Mpx target.
/// `auto = min(1.0, sqrt(MAX_ON_SCREEN_PIXELS / (w_phys * h_phys)))`. Offscreen targets use the
/// caller's scale unchanged, so `--snapshot --size WxH` always produces exactly that size.
fn effective_scale(on_screen: bool, width: u32, height: u32, scale: f32) -> f32 {
    if !on_screen {
        return scale;
    }
    let w_phys = (width.max(1) as f32) * scale;
    let h_phys = (height.max(1) as f32) * scale;
    let auto = (gibson_types::MAX_ON_SCREEN_PIXELS / (w_phys * h_phys))
        .sqrt()
        .min(1.0);
    scale * auto
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

        // Whether we present to a surface (Window/Raw) or render purely offscreen. This is the
        // switch that decides if the on-screen pixel budget applies.
        let on_screen = !matches!(target, SurfaceTarget::Offscreen);
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

        let renderer_scale = effective_scale(on_screen, width, height, scale);
        log::info!(
            "gibson-core: render target {}x{} (scale {renderer_scale:.3})",
            ((width as f32) * renderer_scale).round().max(1.0) as u32,
            ((height as f32) * renderer_scale).round().max(1.0) as u32
        );
        let renderer = Renderer::new(
            &instance,
            surface,
            Viewport {
                width,
                height,
                scale: renderer_scale,
            },
            &atlas,
            &floor,
            &settings,
        )
        .await
        .map_err(GibsonError::Render)?;

        Ok(Gibson {
            scene,
            renderer,
            settings,
            t0: None,
            on_screen,
            last_scale: renderer_scale,
        })
    }

    /// Resize the presentation surface (and render target) in physical pixels.
    ///
    /// The on-screen pixel budget is re-derived here, so a window dragged from a Retina display
    /// to a non-Retina one (or between differently sized displays) picks up the right cap.
    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        let renderer_scale = effective_scale(self.on_screen, width, height, scale);
        if renderer_scale != self.last_scale {
            log::info!(
                "gibson-core: render target {}x{} (scale {renderer_scale:.3})",
                ((width as f32) * renderer_scale).round().max(1.0) as u32,
                ((height as f32) * renderer_scale).round().max(1.0) as u32
            );
            self.last_scale = renderer_scale;
        }
        self.renderer.resize(Viewport {
            width,
            height,
            scale: renderer_scale,
        });
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
        self.renderer
            .render_to_rgba(&frame)
            .map_err(GibsonError::Render)
    }

    /// Current (clamped) settings.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// (presented, skipped) frame counters forwarded from the renderer: `presented` frames
    /// were actually shown on the surface; `skipped` frames were dropped because the surface
    /// was occluded or busy. A host can use this to report honest fps and to back off its
    /// render loop while the skip counter is advancing (see the desktop host).
    pub fn present_stats(&self) -> (u64, u64) {
        self.renderer.present_stats()
    }

    /// `(skipped_timeout, skipped_occluded)`: why frames the host asked for were
    /// not presented. `Timeout` = the drawable pool was starved (GPU/host
    /// behind); `Occluded` = the surface's layer/window was not displayable.
    pub fn skip_breakdown(&self) -> (u64, u64) {
        self.renderer.skip_breakdown()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// On-screen targets are capped to the pixel budget; the derived scale for the 2940x1912
    /// screensaver drawable must land near 2078x1352.
    #[test]
    fn on_screen_scale_is_capped_to_the_budget() {
        let scale = effective_scale(true, 2940, 1912, 1.0);
        let (w, h) = (
            (2940.0 * scale).round() as u32,
            (1912.0 * scale).round() as u32,
        );
        assert!(
            (2060..=2096).contains(&w) && (1334..=1370).contains(&h),
            "expected ~2078x1352, got {w}x{h} (scale {scale})"
        );
        assert!(scale < 1.0);
    }

    /// Offscreen renders are never capped: `--snapshot --size 2940x1912` must stay 2940x1912.
    #[test]
    fn offscreen_scale_is_never_capped() {
        assert_eq!(effective_scale(false, 2940, 1912, 1.0), 1.0);
        assert_eq!(effective_scale(false, 8000, 8000, 0.75), 0.75);
    }

    /// A window already inside the budget keeps its scale exactly.
    #[test]
    fn small_window_is_untouched() {
        assert_eq!(effective_scale(true, 1280, 800, 1.0), 1.0);
        assert_eq!(effective_scale(true, 1600, 900, 1.0), 1.0);
    }

    /// The user's multiplier stays a multiplier: it scales on top of the automatic cap, and a
    /// High-DPI window (1280x800 logical at 2x = 4.1 Mpx physical) is capped to the budget.
    #[test]
    fn render_scale_multiplies_on_top_of_the_cap() {
        let auto = effective_scale(true, 1280, 800, 2.0);
        assert!(
            (1280.0 * auto - 1280.0 * 2.0 * 0.827).abs() < 6.0,
            "auto {auto}"
        );
        let (w, h) = (1280.0 * auto, 800.0 * auto);
        let px = w * h;
        assert!(
            (px / gibson_types::MAX_ON_SCREEN_PIXELS - 1.0).abs() < 0.01,
            "capped physical pixels {px} should sit at the budget"
        );
        // Under budget, the user's scale passes through untouched.
        assert_eq!(effective_scale(true, 1280, 800, 1.0), 1.0);
    }

    /// End-to-end: an offscreen instance at the screensaver size still produces a snapshot of
    /// exactly that size (the cap must not leak into the offscreen path). Skips (and passes)
    /// when the machine has no usable GPU adapter.
    #[test]
    fn offscreen_snapshot_keeps_the_requested_size() {
        let settings = Settings {
            seed: 7,
            ..Settings::default()
        };
        let gibson = pollster::block_on(Gibson::new(
            SurfaceTarget::Offscreen,
            2940,
            1912,
            1.0,
            settings,
        ));
        let mut gibson = match gibson {
            Ok(g) => g,
            Err(GibsonError::Render(RenderError::NoAdapter | RenderError::NoDevice(_))) => {
                eprintln!("skipped: no graphics adapter/device available");
                return;
            }
            Err(e) => panic!("offscreen renderer creation failed: {e}"),
        };
        let (w, h, rgba) = gibson.snapshot(0.0).expect("offscreen snapshot");
        assert_eq!(
            (w, h),
            (2940, 1912),
            "offscreen snapshot must keep the requested size"
        );
        assert_eq!(rgba.len(), 2940 * 1912 * 4);
    }
}
