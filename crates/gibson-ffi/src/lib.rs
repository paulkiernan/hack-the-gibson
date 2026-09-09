//! C ABI bridge for the macOS screen saver (Swift links this staticlib).
//!
//! The four exported functions create/drive/destroy a [`Gibson`] instance whose
//! wgpu surface is hosted inside the caller's `NSView` (via
//! [`wgpu::SurfaceTargetUnsafe::RawHandle`] with an AppKit display handle and an
//! [`wgpu::rwh::AppKitWindowHandle`] pointing at the view). Everything is opaque
//! to the host: the functions take and return raw `void*` / integer handles and
//! never unwind across the boundary.
//!
//! # Ownership and threading contract
//!
//! - The returned handle owns the [`Gibson`] and, through it, the wgpu surface
//!   that references the host `NSView` and its `CAMetalLayer`. The view MUST
//!   outlive the handle: call [`gibson_destroy`] before the view goes away.
//! - All four functions MUST be called from the main thread. AppKit views are
//!   main-thread-only, and wgpu's metal surface configures the view's
//!   `CAMetalLayer` on the calling thread.
//! - A handle is single-owner. `gibson_destroy` frees it; the Swift side must
//!   nil its pointer afterwards (the functions tolerate a null handle as a
//!   no-op / error return, but never dereference freed memory).
//! - Panics never cross the boundary: every exported body runs inside
//!   `std::panic::catch_unwind`, logs the payload to stderr (visible to
//!   `log stream` for `legacyScreenSaver` and to any harness that inherits
//!   stderr), and returns null / nonzero.

use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use gibson_core::{Gibson, SurfaceTarget};
use gibson_types::Settings;

/// `gibson_frame` return code: frame rendered and presented.
pub const FRAME_OK: i32 = 0;
/// `gibson_frame` return code: null or invalid handle.
pub const FRAME_ERR_NULL_HANDLE: i32 = 1;
/// `gibson_frame` return code: the renderer reported an error.
pub const FRAME_ERR_RENDER: i32 = 2;
/// `gibson_frame` return code: a Rust panic was caught at the boundary.
pub const FRAME_ERR_PANIC: i32 = 3;

/// Log a caught panic payload (stderr is the channel `log stream` shows).
fn panic_payload(payload: &Box<dyn std::any::Any + Send>, context: &str) {
    if let Some(message) = payload.downcast_ref::<&str>() {
        eprintln!("gibson-ffi: panic in {context}: {message}");
    } else if let Some(message) = payload.downcast_ref::<String>() {
        eprintln!("gibson-ffi: panic in {context}: {message}");
    } else {
        eprintln!("gibson-ffi: panic in {context}: (non-string payload)");
    }
}

/// Parse `settings_json` (a null-terminated UTF-8 C string, or null) into
/// [`Settings`]. Invalid or missing JSON falls back to defaults, logged.
fn parse_settings(settings_json: *const c_char) -> Settings {
    let json = if settings_json.is_null() {
        eprintln!("gibson-ffi: settings_json is null; using default settings");
        None
    } else {
        // SAFETY: the Swift side promises a null-terminated C string (or null,
        // handled above) that stays valid for the call.
        Some(unsafe { CStr::from_ptr(settings_json) }.to_string_lossy().into_owned())
    };
    match json {
        None => Settings::default(),
        Some(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(settings) => settings,
            Err(e) => {
                eprintln!("gibson-ffi: invalid settings json ({e}); using default settings");
                Settings::default()
            }
        },
    }
}

/// Create a Gibson instance hosted in the caller's NSView.
///
/// `ns_view` must be a valid `NSView*` (null yields null). `width`/`height` are
/// the view's size in physical pixels, `scale` the window's backing scale
/// factor (renderer resolution scales from these, see `gibson-render`).
/// `settings_json` is a JSON object matching `gibson_types::Settings`
/// (snake_case fields) or null for defaults. Returns an opaque handle, null on
/// any failure. Errors are logged.
///
/// # Safety
/// `ns_view` must be a valid `NSView*` or null; `settings_json` a
/// null-terminated UTF-8 C string or null. Both must remain valid for the
/// duration of the call. Must run on the main thread.
#[no_mangle]
pub extern "C" fn gibson_create(
    ns_view: *mut c_void,
    width: u32,
    height: u32,
    scale: f32,
    settings_json: *const c_char,
) -> *mut c_void {
    match catch_unwind(AssertUnwindSafe(|| {
        create_inner(ns_view, width, height, scale, settings_json)
    })) {
        Ok(handle) => handle,
        Err(payload) => {
            panic_payload(&payload, "gibson_create");
            ptr::null_mut()
        }
    }
}

fn create_inner(
    ns_view: *mut c_void,
    width: u32,
    height: u32,
    scale: f32,
    settings_json: *const c_char,
) -> *mut c_void {
    let ns_view = match std::ptr::NonNull::new(ns_view) {
        Some(v) => v,
        None => {
            eprintln!("gibson-ffi: gibson_create: ns_view is null");
            return ptr::null_mut();
        }
    };
    let settings = parse_settings(settings_json).clamped();

    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(wgpu::rwh::RawDisplayHandle::AppKit(
            wgpu::rwh::AppKitDisplayHandle::new(),
        )),
        raw_window_handle: wgpu::rwh::RawWindowHandle::AppKit(
            wgpu::rwh::AppKitWindowHandle::new(ns_view),
        ),
    };

    match pollster::block_on(Gibson::new(
        SurfaceTarget::Raw(target),
        width,
        height,
        scale,
        settings,
    )) {
        Ok(gibson) => {
            eprintln!(
                "gibson-ffi: gibson_create ok: {}x{} scale {scale}",
                width, height
            );
            Box::into_raw(Box::new(gibson)).cast()
        }
        Err(e) => {
            eprintln!("gibson-ffi: gibson_create failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Resize the instance's render target (call from the main thread when the
/// view's size or backing scale changes). Null handle is a no-op.
///
/// # Safety
/// `handle` must come from [`gibson_create`] and not yet be destroyed, or be
/// null. Must run on the main thread.
#[no_mangle]
pub extern "C" fn gibson_resize(handle: *mut c_void, width: u32, height: u32, scale: f32) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            eprintln!("gibson-ffi: gibson_resize: null handle ignored");
            return;
        }
        // SAFETY: `handle` came from `gibson_create` (a Box<Gibson>) and the
        // Swift side guarantees it is not destroyed concurrently.
        let gibson = unsafe { &mut *(handle.cast::<Gibson>()) };
        gibson.resize(width, height, scale);
    }));
}

/// Advance the instance to `time_seconds` (host monotonic seconds) and present
/// one frame. Returns 0 on success; see the `FRAME_*` constants for failures.
///
/// # Safety
/// `handle` must come from [`gibson_create`] and not yet be destroyed, or be
/// null (returns [`FRAME_ERR_NULL_HANDLE`]). Must run on the main thread.
#[no_mangle]
pub extern "C" fn gibson_frame(handle: *mut c_void, time_seconds: f64) -> i32 {
    match catch_unwind(AssertUnwindSafe(|| frame_inner(handle, time_seconds))) {
        Ok(code) => code,
        Err(payload) => {
            panic_payload(&payload, "gibson_frame");
            FRAME_ERR_PANIC
        }
    }
}

fn frame_inner(handle: *mut c_void, time_seconds: f64) -> i32 {
    if handle.is_null() {
        eprintln!("gibson-ffi: gibson_frame: null handle");
        return FRAME_ERR_NULL_HANDLE;
    }
    // SAFETY: `handle` came from `gibson_create` (a Box<Gibson>) and the Swift
    // side guarantees it is not destroyed concurrently.
    let gibson = unsafe { &mut *(handle.cast::<Gibson>()) };
    match gibson.frame(time_seconds) {
        Ok(()) => FRAME_OK,
        Err(e) => {
            eprintln!("gibson-ffi: gibson_frame failed: {e}");
            FRAME_ERR_RENDER
        }
    }
}

/// Destroy the instance and free the handle. Null handle is a no-op. The
/// caller must not use the handle afterwards (nil its pointer).
///
/// # Safety
/// `handle` must come from [`gibson_create`] (or be null) and be used at most
/// once here; the Swift side nils its pointer after the call. Must run on the
/// main thread.
#[no_mangle]
pub extern "C" fn gibson_destroy(handle: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return;
        }
        // SAFETY: reclaim the Box handed out by `gibson_create`.
        drop(unsafe { Box::from_raw(handle.cast::<Gibson>()) });
        eprintln!("gibson-ffi: gibson_destroy ok");
    }));
}

