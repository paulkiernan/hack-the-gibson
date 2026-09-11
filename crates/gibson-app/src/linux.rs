//! Linux xscreensaver host.
//!
//! Two things launch a hack, and they hand the window over differently: the
//! daemon runs the `programs:` line from `~/.xscreensaver` verbatim and passes
//! the window only in `$XSCREENSAVER_WINDOW`, while `xscreensaver-settings`
//! appends `--window-id 0x<id>` to the command line for its embedded preview
//! (and sets the environment too). `cli::x11_window_arg` accepts either, so
//! both work.
//!
//! We adopt the window xscreensaver made: open the X display, build a wgpu
//! surface on it via raw Xlib handles, then render at ~60 Hz while pumping
//! `StructureNotify` events. `ConfigureNotify` resizes the renderer;
//! `DestroyNotify` (xscreensaver's way of stopping the hack) exits.
//!
//! CI exercises this host for real — `platform/linux/smoke-test.sh` under Xvfb
//! with a software Vulkan driver adopts a window via `$XSCREENSAVER_WINDOW`
//! (and again via `--window-id`), asserts that frames were actually presented,
//! and asserts a clean exit when the window is destroyed. That is a software
//! adapter on a headless X server, not a real GPU; see
//! `platform/linux/README.md` for install notes and the exact scope of what has
//! been tested.

use std::ptr::NonNull;
use std::time::{Duration, Instant};

use gibson_core::{Gibson, SurfaceTarget};
use x11_dl::xlib::{
    ConfigureNotify, DestroyNotify, StructureNotifyMask, Window, XConfigureEvent,
    XDestroyWindowEvent, XErrorEvent, XEvent, XWindowAttributes, Xlib,
};

use crate::cli::{self, Cli};

/// Parse an X11 window id given as a decimal or `0x`-hex string.
///
/// Parsed straight into `Window` (the platform's `XID`) rather than a fixed-width `u64` so an id
/// too large for the platform is rejected instead of silently truncated, and so the call site
/// needs no cast.
fn parse_xid(s: &str) -> Result<Window, String> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Window::from_str_radix(hex, 16).map_err(|_| format!("invalid window id {s:?}"))
    } else {
        s.parse::<Window>()
            .map_err(|_| format!("invalid window id {s:?}"))
    }
}

/// Swallow X protocol errors: a DestroyNotify race or an already-gone window
/// must not terminate the process via the default X error handler.
unsafe extern "C" fn ignore_x_error(
    _display: *mut x11_dl::xlib::Display,
    _error: *mut XErrorEvent,
) -> std::os::raw::c_int {
    0
}

pub fn run(cli: &Cli) -> Result<(), String> {
    let xid_arg = cli
        .x11_window_arg()
        .ok_or_else(|| "no X11 window requested".to_string())?;
    let xid = parse_xid(&xid_arg)?;
    let settings = cli::resolve(cli)?;

    let xlib = Xlib::open().map_err(|e| format!("cannot load libX11: {e}"))?;

    unsafe {
        let display = (xlib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return Err("cannot open the X display (is DISPLAY set?)".to_string());
        }
        let result = drive(&xlib, display, xid, &settings);
        (xlib.XCloseDisplay)(display);
        result
    }
}

/// Render into `xid` until it is destroyed. Runs on the X11 connection's
/// thread; all calls are unsafe FFI.
#[allow(non_upper_case_globals)] // X11 event-type constants are lower-case (ConfigureNotify, …)
unsafe fn drive(
    xlib: &Xlib,
    display: *mut x11_dl::xlib::Display,
    xid: x11_dl::xlib::Window,
    settings: &gibson_types::Settings,
) -> Result<(), String> {
    (xlib.XSetErrorHandler)(Some(ignore_x_error));

    let mut attrs: XWindowAttributes = std::mem::zeroed();
    if (xlib.XGetWindowAttributes)(display, xid, &mut attrs) == 0 {
        return Err(format!("window {xid:#x} no longer exists"));
    }
    let width = attrs.width.max(1) as u32;
    let height = attrs.height.max(1) as u32;
    let screen = (xlib.XDefaultScreen)(display);
    let visual_id = if attrs.visual.is_null() {
        0
    } else {
        (xlib.XVisualIDFromVisual)(attrs.visual)
    };

    // Raw Xlib handles for the wgpu surface. `wgpu::rwh` is wgpu 30's own
    // raw-window-handle re-export, so the handle types match exactly what
    // `create_surface_unsafe` consumes.
    let display_handle = wgpu::rwh::DisplayHandle::borrow_raw(wgpu::rwh::RawDisplayHandle::Xlib(
        wgpu::rwh::XlibDisplayHandle::new(NonNull::new(display.cast()), screen),
    ));
    let mut xlib_window = wgpu::rwh::XlibWindowHandle::new(xid);
    xlib_window.visual_id = visual_id;
    let window_handle =
        wgpu::rwh::WindowHandle::borrow_raw(wgpu::rwh::RawWindowHandle::Xlib(xlib_window));
    let target =
        wgpu::SurfaceTargetUnsafe::from_display_and_window(&display_handle, &window_handle)
            .map_err(|e| format!("invalid X11 surface handles: {e}"))?;
    let mut gibson = pollster::block_on(Gibson::new(
        SurfaceTarget::Raw(target),
        width,
        height,
        settings.render_scale,
        settings.clone(),
    ))
    .map_err(|e| format!("cannot initialize renderer on window {xid:#x}: {e}"))?;
    log::info!("gibson-app: xscreensaver host running on window {xid:#x} ({width}x{height})");

    (xlib.XSelectInput)(display, xid, StructureNotifyMask);
    (xlib.XFlush)(display);

    let start = Instant::now();
    // If the window is destroyed between our adoption and XSelectInput above,
    // no DestroyNotify is ever queued for us — and if the owner crashes, none
    // arrives either. Re-query the window once a second as an event-stream-
    // independent liveness check, and cap consecutive frame errors so a dead
    // window cannot spin the loop at 60 Hz logging forever.
    let mut liveness_check = Instant::now() + Duration::from_secs(1);
    let mut error_policy = crate::error_policy::ConsecutiveErrorPolicy::new(10);
    // One exit point, so the frame counters are reported on every path —
    // including the failure paths. The CI smoke test reads this line to tell a
    // hack that drew from one that merely started.
    let outcome: Result<&'static str, String> = 'frames: loop {
        // Drain window events without blocking.
        let mut event: XEvent = std::mem::zeroed();
        while (xlib.XCheckWindowEvent)(display, xid, StructureNotifyMask, &mut event) != 0 {
            match event.get_type() {
                ConfigureNotify => {
                    let cfg: XConfigureEvent = event.configure;
                    let (w, h) = ((cfg.width.max(1)) as u32, (cfg.height.max(1)) as u32);
                    gibson.resize(w, h, settings.render_scale);
                }
                DestroyNotify => {
                    let gone: XDestroyWindowEvent = event.destroy_window;
                    if gone.window == xid {
                        break 'frames Ok("window destroyed");
                    }
                }
                _ => {}
            }
        }

        // Independent liveness check (see note above). X errors are swallowed
        // by the handler installed in drive(), so a vanished window simply
        // makes XGetWindowAttributes return 0.
        let now = Instant::now();
        if now >= liveness_check {
            liveness_check = now + Duration::from_secs(1);
            let mut live: XWindowAttributes = std::mem::zeroed();
            if (xlib.XGetWindowAttributes)(display, xid, &mut live) == 0 {
                break 'frames Ok("window is gone");
            }
        }

        // One frame, paced to ~60 Hz when presenting. If the surface reports the frame
        // skipped (window covered / surface busy), back off to a slow poll instead so an
        // invisible hack cannot spin.
        let t = start.elapsed().as_secs_f64();
        let (presented_before, _) = gibson.present_stats();
        match gibson.frame(t) {
            Ok(()) => error_policy.record_success(),
            Err(e) => {
                log::error!("frame error: {e}");
                if error_policy.record_error() {
                    break 'frames Err(format!(
                        "{} consecutive frame errors on window {xid:#x}; giving up",
                        error_policy.consecutive()
                    ));
                }
            }
        }
        let (presented_after, _) = gibson.present_stats();
        (xlib.XFlush)(display);
        let wait = if presented_after > presented_before {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(250)
        };
        std::thread::sleep(wait);
    };

    let (presented, skipped) = gibson.present_stats();
    match outcome {
        Ok(reason) => {
            log::info!(
                "gibson-app: exiting ({reason}); presented {presented} frames, skipped {skipped}, \
                 on window {xid:#x}"
            );
            Ok(())
        }
        Err(e) => {
            log::error!(
                "gibson-app: giving up on window {xid:#x} after presenting {presented} frames \
                 and skipping {skipped}: {e}"
            );
            Err(e)
        }
    }
}
