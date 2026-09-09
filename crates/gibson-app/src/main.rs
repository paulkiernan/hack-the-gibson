//! Hack the Gibson desktop windowed app.
//!
//! Batch 0 scaffold: a minimal winit 0.30 `ApplicationHandler` that opens a 1280×800 window,
//! builds `Gibson` over a wgpu surface, and redraws continuously. Batch 1 replaces this file
//! wholesale with the full CLI/config/snapshot/screensaver host.

use std::sync::Arc;
use std::time::Instant;

use gibson_core::{Gibson, SurfaceTarget};
use gibson_types::Settings;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        window: None,
        gibson: None,
        start: None,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop error: {e}");
        std::process::exit(1);
    }
}

struct App {
    window: Option<Arc<Window>>,
    gibson: Option<Gibson>,
    start: Option<Instant>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Hack the Gibson")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attributes).expect("create window"));
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;

        // wgpu init is async; the scaffold blocks on it once at startup.
        let gibson = pollster::block_on(Gibson::new(
            SurfaceTarget::Window(window.clone().into()),
            size.width,
            size.height,
            scale,
            Settings::default(),
        ))
        .expect("initialize gibson");

        self.start = Some(Instant::now());
        self.gibson = Some(gibson);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(window) = &self.window {
                    let scale = window.scale_factor() as f32;
                    if let Some(gibson) = &mut self.gibson {
                        gibson.resize(size.width, size.height, scale);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let (Some(gibson), Some(start)) = (&mut self.gibson, self.start) {
                    let t = start.elapsed().as_secs_f64();
                    if let Err(e) = gibson.frame(t) {
                        log::error!("frame error: {e}");
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
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
