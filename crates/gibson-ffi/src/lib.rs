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
//!   `CAMetalLayer` on the calling thread. Calls are not reentrant: a nested
//!   entry call while one is in flight is refused with `FRAME_ERR_BUSY`.
//! - A handle is single-owner. Every live handle is tracked in a process-wide
//!   registry: `gibson_destroy` removes the handle before freeing it, so a
//!   second `gibson_destroy` and any call after destruction are logged no-ops
//!   (error codes), never a dereference of freed memory. The Swift side should
//!   still nil its pointer after `gibson_destroy`.
//! - Panics never cross the boundary: every exported body runs inside
//!   `std::panic::catch_unwind`, reports the payload, and returns null/nonzero.
//!   The panic reporter itself is infallible (a failing stderr write cannot
//!   abort the host).
//!
//! # Logging
//!
//! A `log`-facade sink forwards `gibson-core`/`gibson-render` diagnostics to
//! `os_log` under the `org.hackthegibson.TheGibson` subsystem (installed once,
//! lazily), so Rust-side messages appear in `log stream` next to the Swift
//! `os.Logger` lines. Errors are additionally echoed to stderr, which is what
//! a standalone harness inherits.

use std::ffi::{c_char, c_void, CStr};
use std::io::Write as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};

use gibson_core::{Gibson, SurfaceTarget};
use gibson_types::Settings;

/// `gibson_frame` return code: frame rendered and presented.
pub const FRAME_OK: i32 = 0;
/// `gibson_frame` return code: null handle.
pub const FRAME_ERR_NULL_HANDLE: i32 = 1;
/// `gibson_frame` return code: the renderer reported an error.
pub const FRAME_ERR_RENDER: i32 = 2;
/// `gibson_frame` return code: a Rust panic was caught at the boundary.
pub const FRAME_ERR_PANIC: i32 = 3;
/// `gibson_frame` return code: unknown or already-destroyed handle.
pub const FRAME_ERR_STALE_HANDLE: i32 = 4;
/// `gibson_frame` return code: a nested call while another was in flight.
pub const FRAME_ERR_BUSY: i32 = 5;
/// `gibson_present_stats` return code: an output pointer was null.
pub const FRAME_ERR_NULL_OUT: i32 = 6;

/// Magic tag every live handle slot carries ("GIBS"); cheap corruption check
/// on top of registry membership.
const HANDLE_MAGIC: u32 = 0x4749_4253;

/// Opaque handle payload. The registry below is the authoritative validity
/// check (destroyed handles are removed before the memory is freed, so stale
/// pointers are never dereferenced); `magic` and `in_use` are defence in
/// depth against registry corruption and reentrancy.
#[repr(C)]
struct GibsonSlot {
    magic: u32,
    gibson: Gibson,
    in_use: AtomicBool,
}

/// All live handles, keyed by raw pointer. Every entry point takes this lock
/// to validate/register the handle AND to touch the slot header (magic, busy
/// flag); the body of a call runs after the lock is released, which `destroy`
/// (also lock-serialized) can only refuse while `busy` is set. A destroyed
/// handle is removed under the same lock before its memory is freed, so a
/// stale pointer is never dereferenced.
///
/// `Send + Sync` is sound: the `*mut` payload is never dereferenced outside
/// this lock's critical sections (see `acquire_slot`).
struct SlotRegistry(Mutex<Vec<*mut GibsonSlot>>);

// SAFETY: all access to the raw pointers is serialized by the inner mutex.
unsafe impl Send for SlotRegistry {}
// SAFETY: see above; validation and mutation of the pointed-to header bytes
// happen only inside the mutex.
unsafe impl Sync for SlotRegistry {}

static LIVE_SLOTS: SlotRegistry = SlotRegistry(Mutex::new(Vec::new()));

// ---------------------------------------------------------------------------
// Logging (os_log sink + stderr)
// ---------------------------------------------------------------------------

fn slots_lock() -> std::sync::MutexGuard<'static, Vec<*mut GibsonSlot>> {
    LIVE_SLOTS
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Forward `log`-facade records to os_log under the saver's subsystem.
struct OsLogSink {
    log: oslog::OsLog,
}

impl log::Log for OsLogSink {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }
    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            log::Level::Error => oslog::Level::Error,
            log::Level::Warn => oslog::Level::Default,
            log::Level::Info => oslog::Level::Info,
            log::Level::Debug | log::Level::Trace => oslog::Level::Debug,
        };
        self.log
            .with_level(level, &format!("[{}] {}", record.target(), record.args()));
    }
    fn flush(&self) {}
}

static LOGGER_INIT: Once = Once::new();

/// Install the os_log sink once. Infallible: a logger that is already set
/// (another host component) simply wins.
fn install_os_logger() {
    LOGGER_INIT.call_once(|| {
        let log = oslog::OsLog::new("org.hackthegibson.TheGibson", "rust");
        let sink = OsLogSink { log };
        if log::set_boxed_logger(Box::new(sink)).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
    });
}

/// Write to stderr (what a standalone harness inherits) and the log facade.
/// Infallible by construction: a write failure is discarded, never a panic.
fn emit(context: &str, message: &str) {
    let _ = writeln!(std::io::stderr(), "gibson-ffi: {context}: {message}");
}

fn report_info(context: &str, message: &str) {
    emit(context, message);
    log::info!("{context}: {message}");
}

fn report_warn(context: &str, message: &str) {
    emit(context, message);
    log::warn!("{context}: {message}");
}

fn report_error(context: &str, message: &str) {
    emit(context, message);
    log::error!("{context}: {message}");
}

/// Report a caught panic payload. Infallible (see module docs).
fn report_panic(context: &str, payload: &(dyn std::any::Any + Send)) {
    let message = if let Some(text) = payload.downcast_ref::<&str>() {
        format!("panic in {context}: {text}")
    } else if let Some(text) = payload.downcast_ref::<String>() {
        format!("panic in {context}: {text}")
    } else {
        format!("panic in {context}: (non-string payload)")
    };
    report_error(context, &message);
}

// ---------------------------------------------------------------------------
// Handle registry helpers
// ---------------------------------------------------------------------------

/// RAII busy guard returned by `acquire_slot`: clears the slot's `in_use`
/// flag on drop, so even a panic mid-body (caught at the FFI boundary) can
/// never leave a handle permanently busy.
struct BusyGuard(*mut GibsonSlot);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        // SAFETY: `acquire_slot` returned the guard only for a live registered
        // slot whose flag it set; destroy cannot free the slot while the flag
        // is set (it refuses under the registry lock).
        unsafe { (*self.0).in_use.store(false, Ordering::Release) };
    }
}

impl BusyGuard {
    fn slot_ptr(&self) -> *mut GibsonSlot {
        self.0
    }
}

/// Validate `handle` and mark its slot busy. Returns a [`BusyGuard`] on
/// success or the failure code. Membership, magic, and the busy handshake all
/// happen inside the registry lock (the same lock `destroy` frees under), so a
/// stale or concurrently-destroyed handle is never dereferenced.
///
/// The lock is released before the caller runs `Gibson` methods, which is
/// sound because the slot is already marked busy under the lock: `destroy`
/// (which requires `busy == false` to free, checked under the same lock)
/// refuses to run concurrently with a body that acquired the slot, and any
/// later `destroy` can only free the slot after the [`BusyGuard`] has cleared
/// the flag. A `destroy` that wins the lock first removes the slot before any
/// other caller can see it, so they fail the membership check without
/// touching the pointer.
fn acquire_slot(handle: *mut c_void, context: &str) -> Result<BusyGuard, i32> {
    if handle.is_null() {
        report_error(context, "null handle");
        return Err(FRAME_ERR_NULL_HANDLE);
    }
    let raw = handle.cast::<GibsonSlot>();
    let live = slots_lock();
    if !live.iter().copied().any(|p| p == raw) {
        report_error(context, "unknown or already-destroyed handle (stale)");
        return Err(FRAME_ERR_STALE_HANDLE);
    }
    // SAFETY: `raw` is a registered live slot and the registry lock (which
    // `destroy_inner` holds while freeing) is held across this whole block.
    let slot = unsafe { &*raw };
    if slot.magic != HANDLE_MAGIC {
        report_error(context, "handle failed magic validation");
        return Err(FRAME_ERR_STALE_HANDLE);
    }
    if slot.in_use.swap(true, Ordering::AcqRel) {
        report_error(context, "reentrant call refused (another call in flight)");
        return Err(FRAME_ERR_BUSY);
    }
    Ok(BusyGuard(raw))
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Parse `settings_json` (a null-terminated UTF-8 C string, or null) into
/// [`Settings`]. Invalid or missing JSON falls back to defaults, logged.
fn parse_settings(settings_json: *const c_char) -> Settings {
    let json = if settings_json.is_null() {
        report_info(
            "gibson_create",
            "settings_json is null; using default settings",
        );
        None
    } else {
        // SAFETY: the Swift side promises a null-terminated C string (or null,
        // handled above) that stays valid for the call.
        Some(
            unsafe { CStr::from_ptr(settings_json) }
                .to_string_lossy()
                .into_owned(),
        )
    };
    match json {
        None => Settings::default(),
        Some(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(settings) => settings,
            Err(e) => {
                report_warn(
                    "gibson_create",
                    &format!("invalid settings json ({e}); using default settings"),
                );
                Settings::default()
            }
        },
    }
}

/// Create a Gibson instance hosted in the caller's NSView.
///
/// `ns_view` must be a valid `NSView*` (null yields null). `width`/`height`
/// are the view's size in LOGICAL points, `scale` the window's backing scale
/// factor times the desired `render_scale` (the renderer's actual surface
/// extent becomes `width·scale × height·scale`, i.e. physical pixels at
/// `render_scale` 1). `settings_json` is a JSON object matching
/// `gibson_types::Settings` (snake_case fields) or null for defaults. Returns
/// an opaque handle, null on any failure.
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
    install_os_logger();
    match catch_unwind(AssertUnwindSafe(|| {
        create_inner(ns_view, width, height, scale, settings_json)
    })) {
        Ok(handle) => handle,
        Err(payload) => {
            report_panic("gibson_create", &*payload);
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
    let ns_view = match ptr::NonNull::new(ns_view) {
        Some(view) => view,
        None => {
            report_error("gibson_create", "ns_view is null");
            return ptr::null_mut();
        }
    };
    let settings = parse_settings(settings_json).clamped();

    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(wgpu::rwh::RawDisplayHandle::AppKit(
            wgpu::rwh::AppKitDisplayHandle::new(),
        )),
        raw_window_handle: wgpu::rwh::RawWindowHandle::AppKit(wgpu::rwh::AppKitWindowHandle::new(
            ns_view,
        )),
    };

    match pollster::block_on(Gibson::new(
        SurfaceTarget::Raw(target),
        width,
        height,
        scale,
        settings,
    )) {
        Ok(gibson) => {
            let slot = Box::new(GibsonSlot {
                magic: HANDLE_MAGIC,
                gibson,
                in_use: AtomicBool::new(false),
            });
            let raw = Box::into_raw(slot);
            slots_lock().push(raw);
            report_info(
                "gibson_create",
                &format!("ok: {width}x{height} (logical) scale {scale}"),
            );
            raw.cast::<c_void>()
        }
        Err(e) => {
            report_error("gibson_create", &format!("failed: {e}"));
            ptr::null_mut()
        }
    }
}

/// Resize the instance's render target (call from the main thread when the
/// view's size or backing scale changes). Null or stale handle: no-op.
///
/// # Safety
/// `handle` must come from [`gibson_create`] and not yet be destroyed, or be
/// null. Must run on the main thread, not reentrantly.
#[no_mangle]
pub extern "C" fn gibson_resize(handle: *mut c_void, width: u32, height: u32, scale: f32) {
    install_os_logger();
    let _ = catch_unwind(AssertUnwindSafe(|| {
        resize_inner(handle, width, height, scale)
    }));
}

fn resize_inner(handle: *mut c_void, width: u32, height: u32, scale: f32) {
    let guard = match acquire_slot(handle, "gibson_resize") {
        Ok(guard) => guard,
        Err(_) => return, // acquire_slot already logged.
    };
    // SAFETY: exclusive access is guaranteed by the busy handshake; the guard
    // releases it on drop.
    let gibson = unsafe { &mut (*guard.slot_ptr()).gibson };
    gibson.resize(width, height, scale);
}

/// Advance the instance to `time_seconds` (host monotonic seconds) and present
/// one frame. Returns 0 on success; see the `FRAME_*` constants for failures.
///
/// # Safety
/// `handle` must come from [`gibson_create`] and not yet be destroyed, or be
/// null. Must run on the main thread, not reentrantly.
#[no_mangle]
pub extern "C" fn gibson_frame(handle: *mut c_void, time_seconds: f64) -> i32 {
    install_os_logger();
    match catch_unwind(AssertUnwindSafe(|| frame_inner(handle, time_seconds))) {
        Ok(code) => code,
        Err(payload) => {
            report_panic("gibson_frame", &*payload);
            FRAME_ERR_PANIC
        }
    }
}

fn frame_inner(handle: *mut c_void, time_seconds: f64) -> i32 {
    let guard = match acquire_slot(handle, "gibson_frame") {
        Ok(guard) => guard,
        Err(code) => return code,
    };
    // SAFETY: exclusive access is guaranteed by the busy handshake; the guard
    // releases it on drop (also when the body panics).
    let gibson = unsafe { &mut (*guard.slot_ptr()).gibson };
    match gibson.frame(time_seconds) {
        Ok(()) => FRAME_OK,
        Err(e) => {
            report_error("gibson_frame", &format!("render failed: {e}"));
            FRAME_ERR_RENDER
        }
    }
}

/// Read the renderer's frame counters: `*presented` counts frames actually
/// presented to the surface, `*skipped` counts frames dropped because the
/// surface was occluded/busy (so a display-link callback that skipped a frame
/// is not counted as a rendered frame). Returns 0 on success; see the
/// `FRAME_*` constants (6 = null output pointer).
///
/// # Safety
/// `handle` must come from [`gibson_create`] and not yet be destroyed (or be
/// null); `presented` and `skipped` must be valid writable `u64*`. Must run on
/// the main thread, not reentrantly.
#[no_mangle]
pub extern "C" fn gibson_present_stats(
    handle: *mut c_void,
    presented: *mut u64,
    skipped: *mut u64,
) -> i32 {
    install_os_logger();
    match catch_unwind(AssertUnwindSafe(|| {
        present_stats_inner(handle, presented, skipped)
    })) {
        Ok(code) => code,
        Err(payload) => {
            report_panic("gibson_present_stats", &*payload);
            FRAME_ERR_PANIC
        }
    }
}

fn present_stats_inner(handle: *mut c_void, presented: *mut u64, skipped: *mut u64) -> i32 {
    if presented.is_null() || skipped.is_null() {
        report_error("gibson_present_stats", "null output pointer");
        return FRAME_ERR_NULL_OUT;
    }
    let guard = match acquire_slot(handle, "gibson_present_stats") {
        Ok(guard) => guard,
        Err(code) => return code,
    };
    // SAFETY: exclusive access is guaranteed by the busy handshake; the guard
    // releases it on drop.
    let gibson = unsafe { &(*guard.slot_ptr()).gibson };
    let (frames, dropped) = gibson.present_stats();
    // SAFETY: the caller promises valid writable pointers.
    unsafe {
        *presented = frames;
        *skipped = dropped;
    }
    FRAME_OK
}

/// Read why frames were dropped: `*timeout` counts skips from a starved
/// drawable pool, `*occluded` counts skips where the surface's layer/window was
/// not displayable. Returns 0 on success; 6 = null output pointer.
///
/// # Safety
/// `handle` must come from [`gibson_create`] and not yet be destroyed (or be
/// null); the output pointers must be valid writable `u64*`. Main thread, not
/// reentrantly.
#[no_mangle]
pub extern "C" fn gibson_skip_breakdown(
    handle: *mut c_void,
    timeout: *mut u64,
    occluded: *mut u64,
) -> i32 {
    install_os_logger();
    match catch_unwind(AssertUnwindSafe(|| {
        skip_breakdown_inner(handle, timeout, occluded)
    })) {
        Ok(code) => code,
        Err(payload) => {
            report_panic("gibson_skip_breakdown", &*payload);
            FRAME_ERR_PANIC
        }
    }
}

fn skip_breakdown_inner(handle: *mut c_void, timeout: *mut u64, occluded: *mut u64) -> i32 {
    if timeout.is_null() || occluded.is_null() {
        report_error("gibson_skip_breakdown", "null output pointer");
        return FRAME_ERR_NULL_OUT;
    }
    let guard = match acquire_slot(handle, "gibson_skip_breakdown") {
        Ok(guard) => guard,
        Err(code) => return code,
    };
    // SAFETY: exclusive access is guaranteed by the busy handshake.
    let gibson = unsafe { &(*guard.slot_ptr()).gibson };
    let (timeouts, occlusions) = gibson.skip_breakdown();
    // SAFETY: the caller promises valid writable pointers.
    unsafe {
        *timeout = timeouts;
        *occluded = occlusions;
    }
    FRAME_OK
}

/// Destroy the instance and free the handle. Null, unknown, or already
/// destroyed handle: logged no-op. The caller must not use the handle
/// afterwards (Swift nils its pointer).
///
/// # Safety
/// `handle` must come from [`gibson_create`] (or be null/unknown). Must run on
/// the main thread; refused (logged) if a call is currently in flight on the
/// slot to avoid freeing memory mid-use.
#[no_mangle]
pub extern "C" fn gibson_destroy(handle: *mut c_void) {
    install_os_logger();
    let _ = catch_unwind(AssertUnwindSafe(|| destroy_inner(handle)));
}

fn destroy_inner(handle: *mut c_void) {
    if handle.is_null() {
        report_error("gibson_destroy", "null handle (no-op)");
        return;
    }
    let raw = handle.cast::<GibsonSlot>();
    // Single critical section: membership check, busy check, and removal are
    // atomic with respect to `acquire_slot`, so a frame/resize that started
    // before us (busy set) makes us refuse, and one that starts after us can
    // never see a registered slot again.
    {
        let mut live = slots_lock();
        let Some(pos) = live.iter().position(|p| *p == raw) else {
            report_error(
                "gibson_destroy",
                "unknown or already-destroyed handle (no-op)",
            );
            return;
        };
        // SAFETY: the slot is registered in `live`, which we hold exclusively.
        let busy = unsafe { (*raw).in_use.load(Ordering::Acquire) };
        if busy {
            // A frame/resize is in flight on this slot (host lifecycle bug;
            // all calls must be main-thread and non-reentrant). Refuse rather
            // than free memory mid-use; the slot stays registered.
            report_error(
                "gibson_destroy",
                "destroy refused while a call is in flight (no-op)",
            );
            return;
        }
        live.swap_remove(pos);
    }
    // SAFETY: removed from the registry under the same lock every entry point
    // validates with; no other code can reach this slot now.
    unsafe { (*raw).magic = 0 };
    drop(unsafe { Box::from_raw(raw) });
    report_info("gibson_destroy", "ok");
}
