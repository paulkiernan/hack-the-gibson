//! Hack the Gibson — web host (wasm-bindgen entry point).
//!
//! Bootstraps the full-viewport `<canvas id="gibson">`, reads settings from the URL query
//! string, drives `gibson-core` from a `requestAnimationFrame` loop, and mirrors viewport
//! changes into the renderer. Startup or persistent render failures are written into the
//! `#status` element (the canvas stays black) rather than panicking the module.
//!
//! The crate is `wasm32`-only: native workspace builds compile an empty rlib and this file is
//! never compiled on a native host.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gibson_core::{Gibson, SurfaceTarget};
use gibson_types::{PaletteMode, Settings};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlCanvasElement, Node, UrlSearchParams, Window};

/// Best-effort string for a JS error, for log lines.
fn js_err(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| "unknown JS error".to_string())
}

/// Canvas backing store is capped at this many physical pixels per CSS pixel; above it the
/// fill-rate cost outweighs any visible sharpness gain.
const MAX_DPR: f64 = 2.0;

/// The render loop stops scheduling after this many consecutive frame errors.
const MAX_CONSECUTIVE_FRAME_ERRORS: u32 = 10;

/// Element ids the host page (`web/index.html`) must provide.
const CANVAS_ID: &str = "gibson";
const STATUS_ID: &str = "status";

/// Human-readable failure shown in `#status` when a renderer cannot start (no WebGPU and no
/// WebGL2, or the browser declines a device).
const UNAVAILABLE_MESSAGE: &str =
    "WebGPU/WebGL2 unavailable - try Chrome 113+, Safari 26+, or Firefox with WebGPU enabled";

/// Long-lived per-page state shared by the animation loop and the resize listener.
struct State {
    gibson: RefCell<Gibson>,
    canvas: HtmlCanvasElement,
    /// Set by the `resize` listener; consumed on the next animation frame.
    resize_pending: Cell<bool>,
    consecutive_errors: Cell<u32>,
    /// The animation-frame callback, stored so it can re-schedule itself and outlive `start`.
    raf: RefCell<Option<Closure<dyn FnMut(f64)>>>,
    /// The resize listener handle, kept alive for the life of the page.
    resize_listener: RefCell<Option<Closure<dyn FnMut()>>>,
}

/// Called automatically when the module loads. Never panics; failures land in `#status`.
#[wasm_bindgen(start)]
pub async fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
    if let Err(message) = run().await {
        log::error!("gibson-web: startup failed: {message}");
        set_status(&format!("{UNAVAILABLE_MESSAGE} ({message})"));
    }
    Ok(())
}

async fn run() -> Result<(), String> {
    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let document = window.document().ok_or_else(|| "no document".to_string())?;

    let canvas = find_canvas(&document)?;
    let settings = settings_from_url(&window).clamped();
    log::info!(
        "gibson-web: settings: fly_speed={} bank_strength={} palette={:?} grid={} pulses={} seed={} \
         render_scale={} bloom={} motion_blur={} grain={} crt={}",
        settings.fly_speed,
        settings.bank_strength,
        settings.palette,
        settings.grid,
        settings.pulses,
        settings.seed,
        settings.render_scale,
        settings.bloom,
        settings.motion_blur,
        settings.grain,
        settings.crt,
    );

    // Logical (CSS) size goes to `Gibson`; the renderer derives the physical render size from
    // it via `scale`. The canvas backing store is set to that same physical size.
    let (width, height, scale) = layout(&window, &canvas, settings.render_scale);
    log::info!("gibson-web: viewport {width}x{height} css px, scale {scale} (render_scale {})", settings.render_scale);
    let target = wgpu::SurfaceTarget::Canvas(canvas.clone());
    let gibson = Gibson::new(SurfaceTarget::Window(target), width, height, scale, settings)
        .await
        .map_err(|e| e.to_string())?;

    clear_status();
    log::info!("gibson-web: running on {}", backend_name(&window));

    let state = Rc::new(State {
        gibson: RefCell::new(gibson),
        canvas,
        resize_pending: Cell::new(false),
        consecutive_errors: Cell::new(0),
        raf: RefCell::new(None),
        resize_listener: RefCell::new(None),
    });
    start_loop(&window, state)
}

fn start_loop(window: &Window, state: Rc<State>) -> Result<(), String> {
    // The resize listener only flags work; the actual resize happens on the next animation
    // frame so a drag that fires many events coalesces into one resize per frame.
    {
        let st = Rc::clone(&state);
        let listener: Closure<dyn FnMut()> =
            Closure::own_assert_unwind_safe(move || st.resize_pending.set(true));
        window
            .add_event_listener_with_callback("resize", listener.as_ref().unchecked_ref())
            .map_err(|e| format!("could not install resize listener: {}", js_err(e)))?;
        *state.resize_listener.borrow_mut() = Some(listener);
    }

    {
        let st = Rc::clone(&state);
        let raf: Closure<dyn FnMut(f64)> = Closure::own_assert_unwind_safe(move |ts: f64| {
            let window = web_sys::window().expect("window is open while animating");
            if st.resize_pending.replace(false) {
                // Mirror whatever settings are active (including any future set_settings call).
                let render_scale = st.gibson.borrow().settings().render_scale;
                let (w, h, scale) = layout(&window, &st.canvas, render_scale);
                log::info!("gibson-web: resize to {w}x{h} css px, scale {scale}");
                st.gibson.borrow_mut().resize(w, h, scale);
            }
            // `ts` is a DOMHighResTimeStamp in milliseconds; `Gibson::frame` expects host
            // monotonic seconds and defines t = 0 on its first call.
            match st.gibson.borrow_mut().frame(ts / 1000.0) {
                Ok(()) => st.consecutive_errors.set(0),
                Err(e) => {
                    log::error!("gibson-web: frame error: {e}");
                    let n = st.consecutive_errors.get() + 1;
                    st.consecutive_errors.set(n);
                    if n >= MAX_CONSECUTIVE_FRAME_ERRORS {
                        log::error!(
                            "gibson-web: stopping render loop after {n} consecutive frame errors"
                        );
                        set_status(&format!("Rendering failed: {e}"));
                        return;
                    }
                }
            }
            // Re-schedule from the stored handle so the closure stays alive.
            if let Some(raf) = st.raf.borrow().as_ref() {
                let _ = window.request_animation_frame(raf.as_ref().unchecked_ref());
            }
        });
        *state.raf.borrow_mut() = Some(raf);

        let raf_handle = state.raf.borrow();
        let raf = raf_handle.as_ref().expect("raf stored above");
        window
            .request_animation_frame(raf.as_ref().unchecked_ref())
            .map_err(|e| format!("could not schedule first frame: {}", js_err(e)))?;
    }
    Ok(())
}

fn find_canvas(document: &Document) -> Result<HtmlCanvasElement, String> {
    let element = document
        .get_element_by_id(CANVAS_ID)
        .ok_or_else(|| format!("missing element #{CANVAS_ID}"))?;
    element
        .dyn_into::<HtmlCanvasElement>()
        .map_err(|_| format!("element #{CANVAS_ID} is not a <canvas>"))
}

/// Measure the viewport and lay the canvas out to match.
///
/// `render_scale` is `Settings::render_scale` (the `?scale=` performance knob). Returns the
/// logical (CSS-pixel) size and the combined `device_pixel_ratio * render_scale`; the canvas
/// backing store is set to `logical * scale` so it exactly matches the (possibly downscaled)
/// resolution the renderer configures for the surface. The CSS box is untouched, so the canvas
/// still fills the viewport at full size while only the backing store shrinks.
fn layout(window: &Window, canvas: &HtmlCanvasElement, render_scale: f32) -> (u32, u32, f32) {
    let css_w = window
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let css_h = window
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let dpr = window.device_pixel_ratio().clamp(0.5, MAX_DPR);
    let logical_w = css_w.round().max(1.0) as u32;
    let logical_h = css_h.round().max(1.0) as u32;
    let scale = (dpr as f32) * render_scale;
    canvas.set_width(((logical_w as f32) * scale).round().max(1.0) as u32);
    canvas.set_height(((logical_h as f32) * scale).round().max(1.0) as u32);
    (logical_w, logical_h, scale)
}

/// Name the backend wgpu is using, for the startup log line.
///
/// `gibson-core` requests `Backends::BROWSER_WEBGPU | GL` on wasm. In wgpu 30 an instance with
/// the WebGPU backend flag can only produce WebGPU adapters while `navigator.gpu` exists;
/// without it the instance falls back to the WebGL2 adapter. So backend choice is exactly
/// `navigator.gpu` presence.
fn backend_name(window: &Window) -> &'static str {
    let navigator: JsValue = window.navigator().unchecked_into();
    match js_sys::Reflect::get(&navigator, &JsValue::from_str("gpu")) {
        Ok(gpu) if !gpu.is_undefined() => "WebGPU",
        _ => "WebGL2 (browser does not expose WebGPU)",
    }
}

/// Read settings from the URL query string. Unparseable or out-of-range values fall back to the
/// defaults; `clamped()` runs afterwards so unknown keys are simply ignored.
fn settings_from_url(window: &Window) -> Settings {
    let mut s = Settings::default();
    let search = window.location().search().unwrap_or_default();
    let Ok(params) = UrlSearchParams::new_with_str(&search) else {
        return s;
    };
    parse_f32(&params, "speed", &mut s.fly_speed);
    parse_f32(&params, "bank", &mut s.bank_strength);
    parse_palette(&params, &mut s.palette);
    parse_u32(&params, "grid", &mut s.grid);
    parse_u32(&params, "pulses", &mut s.pulses);
    parse_u64(&params, "seed", &mut s.seed);
    parse_f32(&params, "scale", &mut s.render_scale);
    parse_f32(&params, "bloom", &mut s.bloom);
    parse_f32(&params, "motionblur", &mut s.motion_blur);
    parse_f32(&params, "grain", &mut s.grain);
    parse_f32(&params, "crt", &mut s.crt);
    s
}

fn parse_f32(params: &UrlSearchParams, key: &str, slot: &mut f32) {
    let Some(raw) = params.get(key) else { return };
    match raw.trim().parse::<f32>() {
        Ok(v) => *slot = v,
        Err(_) => log::warn!("gibson-web: ignoring unparseable ?{key}={raw}"),
    }
}

fn parse_u32(params: &UrlSearchParams, key: &str, slot: &mut u32) {
    let Some(raw) = params.get(key) else { return };
    match raw.trim().parse::<u32>() {
        Ok(v) => *slot = v,
        Err(_) => log::warn!("gibson-web: ignoring unparseable ?{key}={raw}"),
    }
}

fn parse_u64(params: &UrlSearchParams, key: &str, slot: &mut u64) {
    let Some(raw) = params.get(key) else { return };
    match raw.trim().parse::<u64>() {
        Ok(v) => *slot = v,
        Err(_) => log::warn!("gibson-web: ignoring unparseable ?{key}={raw}"),
    }
}

fn parse_palette(params: &UrlSearchParams, slot: &mut PaletteMode) {
    let Some(raw) = params.get("palette") else { return };
    match raw.trim().to_ascii_lowercase().as_str() {
        "normal" => *slot = PaletteMode::Normal,
        "siege" => *slot = PaletteMode::Siege,
        "cycle" => *slot = PaletteMode::Cycle,
        other => log::warn!("gibson-web: ignoring unknown ?palette={other}"),
    }
}

fn status_node() -> Option<Node> {
    let window = web_sys::window()?;
    let document = window.document()?;
    let element: Element = document.get_element_by_id(STATUS_ID)?;
    Some(element.unchecked_into::<Node>())
}

fn set_status(message: &str) {
    if let Some(node) = status_node() {
        node.set_text_content(Some(message));
    }
}

fn clear_status() {
    if let Some(node) = status_node() {
        node.set_text_content(Some(""));
    }
}
