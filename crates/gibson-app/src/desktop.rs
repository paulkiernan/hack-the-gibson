//! Windowed desktop host (macOS / Windows / Linux) on winit 0.30.
//!
//! Renders continuously with `ControlFlow::Poll`; honors HiDPI by handing the
//! renderer logical dimensions and `scale = device_pixel_ratio ×
//! settings.render_scale` (so the internal target lands at the window's
//! physical pixel size when `render_scale == 1`). Esc / Q / CloseRequested
//! quits. Recoverable frame errors are logged; only a failure of the very
//! first frame exits non-zero.

use std::sync::Arc;
use std::time::Instant;

use gibson_core::{Gibson, SurfaceTarget};
use gibson_types::Settings;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::cli::{self, Cli};

/// Run the windowed app. Returns `Err` only when startup or the very first
/// frame fails (callers turn that into a non-zero exit).
pub fn run(cli: &Cli) -> Result<(), String> {
    let settings = cli::resolve(cli)?;
    let event_loop = EventLoop::new().map_err(|e| format!("cannot create event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        window: None,
        gibson: None,
        settings,
        fullscreen: cli.fullscreen,
        start: None,
        frames: 0,
        fps_start: Instant::now(),
        fps_frames: 0,
        last_extent: None,
        fatal: None,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop error: {e}"))?;
    if let Some(err) = app.fatal {
        return Err(err);
    }
    Ok(())
}

struct App {
    window: Option<Arc<Window>>,
    gibson: Option<Gibson>,
    settings: Settings,
    fullscreen: bool,
    /// Clock whose origin is the moment the Gibson instance was created
    /// (host monotonic seconds are what `Gibson::frame` expects).
    start: Option<Instant>,
    /// Total frames presented (decides whether a frame error is fatal).
    frames: u64,
    /// FPS reporting window.
    fps_start: Instant,
    fps_frames: u64,
    /// Last size+scale handed to the renderer (avoids reconfiguring the
    /// surface on every frame).
    last_extent: Option<(u32, u32, u32)>,
    /// Set when startup or the first frame fails; the loop exits and `run`
    /// returns it as the error.
    fatal: Option<String>,
}

impl App {
    /// Logical window size for the renderer plus the matching scale factor,
    /// derived from the window's current physical size.
    fn render_extent(&self, window: &Window) -> (u32, u32, f32) {
        let sf = window.scale_factor();
        let physical = window.inner_size();
        let logical_w = (physical.width as f64 / sf).round() as u32;
        let logical_h = (physical.height as f64 / sf).round() as u32;
        let scale = (sf as f32) * self.settings.render_scale;
        (logical_w.max(1), logical_h.max(1), scale)
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        // Compute the extent first (borrows all of self), then take the
        // disjoint field borrows it needs to render.
        let Some(window) = self.window.as_ref() else { return };
        let Some(start) = self.start else { return };
        if window.inner_size().width == 0 || window.inner_size().height == 0 {
            // Minimized / not yet laid out: skip the frame, keep polling.
            window.request_redraw();
            return;
        }
        let (w, h, scale) = self.render_extent(window);
        let Some(gibson) = self.gibson.as_mut() else { return };
        let extent = (w, h, scale.to_bits());
        if self.last_extent != Some(extent) {
            gibson.resize(w, h, scale);
            self.last_extent = Some(extent);
        }

        let t = start.elapsed().as_secs_f64();
        match gibson.frame(t) {
            Ok(()) => {}
            Err(e) if self.frames == 0 => {
                self.fatal = Some(format!("first frame failed: {e}"));
                event_loop.exit();
                return;
            }
            Err(e) => log::error!("frame error: {e}"),
        }
        self.frames += 1;
        self.fps_frames += 1;
        let elapsed = self.fps_start.elapsed().as_secs_f64();
        if elapsed >= 5.0 {
            let fps = self.fps_frames as f64 / elapsed;
            log::info!("fps: {fps:.1}");
            self.fps_start = Instant::now();
            self.fps_frames = 0;
        }
        window.request_redraw();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title("Hack the Gibson")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        if self.fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.fatal = Some(format!("cannot create window: {e}"));
                event_loop.exit();
                return;
            }
        };
        let (w, h, scale) = self.render_extent(&window);
        let target = SurfaceTarget::Window(window.clone().into());
        match pollster::block_on(Gibson::new(
            target,
            w,
            h,
            scale,
            self.settings.clone(),
        )) {
            Ok(gibson) => self.gibson = Some(gibson),
            Err(e) => {
                self.fatal = Some(format!("cannot initialize renderer: {e}"));
                event_loop.exit();
                return;
            }
        }
        self.start = Some(Instant::now());
        self.fps_start = Instant::now();
        self.window = Some(window.clone());
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            // The next RedrawRequested re-derives size + scale from the window,
            // so Resized and ScaleFactorChanged just ask for a redraw.
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                if matches!(code, KeyCode::Escape | KeyCode::KeyQ) {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
}
