//! C ABI bridge for the macOS screen saver (Swift links this staticlib).
//!
//! Functions are `extern "C"`, `#[no_mangle]`, and panic-safe: every body runs inside
//! `std::panic::catch_unwind`, errors are logged (and echoed to stderr for `log stream`), and a
//! null handle is never dereferenced.
//!
//! Batch 0 scaffold: stubs return null / nonzero / no-op. Batch 1 implements the real bridge:
//! `gibson_create` wraps a `Box<Gibson>` behind the opaque handle, rendering into the host's
//! `NSView` via `SurfaceTarget::Raw` (`SurfaceTargetUnsafe::RawHandle` with an AppKit display
//! handle + `AppKitWindowHandle` window handle).

use std::os::raw::{c_char, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Log a panic payload (if it carries a message) to stderr.
fn log_panic(context: &str) {
    eprintln!("gibson-ffi: panic in {context}");
    log::error!("gibson-ffi: panic in {context}");
}

/// Create a Gibson instance hosted in the caller's NSView.
///
/// Returns an opaque handle (null on failure). Scaffold: always null.
///
/// # Safety
/// `ns_view` must be a valid `NSView*` (may be null); `settings_json` must be a null-terminated
/// UTF-8 C string or null. The body never dereferences either in the scaffold.
#[no_mangle]
pub extern "C" fn gibson_create(
    ns_view: *mut c_void,
    width: u32,
    height: u32,
    scale: f32,
    settings_json: *const c_char,
) -> *mut c_void {
    let _ = (ns_view, width, height, scale, settings_json); // Batch 1 parses/uses these.
    match catch_unwind(AssertUnwindSafe(|| -> *mut c_void { std::ptr::null_mut() })) {
        Ok(handle) => handle,
        Err(_) => {
            log_panic("gibson_create");
            std::ptr::null_mut()
        }
    }
}

/// Resize the instance's render target.
///
/// # Safety
/// `handle` must be a handle from `gibson_create` or null; the scaffold never dereferences it.
#[no_mangle]
pub extern "C" fn gibson_resize(handle: *mut c_void, width: u32, height: u32, scale: f32) {
    let _ = (handle, width, height, scale); // Batch 1 forwards to Gibson::resize.
    let _ = catch_unwind(AssertUnwindSafe(|| {}));
}

/// Advance the instance to `time_seconds` and present a frame. Returns 0 on success, nonzero on
/// failure. Scaffold: always 1 (no instance to drive yet).
///
/// # Safety
/// `handle` must be a handle from `gibson_create` or null; the scaffold never dereferences it.
#[no_mangle]
pub extern "C" fn gibson_frame(handle: *mut c_void, time_seconds: f64) -> i32 {
    let _ = (handle, time_seconds); // Batch 1 forwards to Gibson::frame.
    match catch_unwind(AssertUnwindSafe(|| 1_i32)) {
        Ok(code) => code,
        Err(_) => {
            log_panic("gibson_frame");
            1
        }
    }
}

/// Destroy the instance and free the handle.
///
/// # Safety
/// `handle` must be a handle from `gibson_create` or null; a null handle is a no-op.
#[no_mangle]
pub extern "C" fn gibson_destroy(handle: *mut c_void) {
    let _ = handle; // Batch 1 drops the Box<Gibson> here.
    let _ = catch_unwind(AssertUnwindSafe(|| {}));
}
