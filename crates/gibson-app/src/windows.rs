//! Windows `.scr` screensaver host.
//!
//! Implements the classic Windows screensaver protocol:
//! - `/s` (or `-s`): borderless fullscreen on every monitor, one window + one
//!   `Gibson` per monitor; any key, mouse button, wheel, or mouse move > 5 px
//!   from the starting position exits.
//! - `/p <hwnd>` (or `/p:<hwnd>`): the control-panel preview — render into the
//!   given host window until `IsWindow` reports it gone. `settings.preview` is
//!   forced on so the preview shows the reduced scene.
//! - `/c` (or `/c:<hwnd>`): ensure `gibson.toml` exists and open it in the
//!   default editor.
//!
//! The desktop windowed app remains the default when none of those arguments
//! are present. This host is **compile-verified in CI but not runtime-tested**
//! (no Windows machine in development); see `platform/windows/README.md`.

use std::num::NonZeroIsize;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gibson_core::{Gibson, SurfaceTarget};
use gibson_types::Settings;
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
    IsWindow, PeekMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow, ShowCursor,
    ShowWindow, TranslateMessage, MSG, PM_REMOVE, SW_SHOW, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN,
    WM_SYSKEYDOWN, WM_XBUTTONDOWN, WNDCLASSW, WS_POPUP, WS_VISIBLE,
};

use crate::config;
use crate::saver_args::WindowsSaverMode;

/// Set by the window procedure when the user provides any input.
static EXIT: AtomicBool = AtomicBool::new(false);
/// Cursor position when the fullscreen run started (any later move > 5 px exits).
static CURSOR_START: OnceLock<POINT> = OnceLock::new();

const WINDOW_CLASS: &str = "GibsonScreensaverWindow";

pub fn run(mode: WindowsSaverMode) -> Result<(), String> {
    // Windows screensaver hosts do not go through clap; the config file is the
    // only settings source.
    let settings = config::load_or_create(&config::default_path())?;
    match mode {
        WindowsSaverMode::Fullscreen => run_fullscreen(settings),
        WindowsSaverMode::Preview(hwnd) => run_preview(hwnd, settings),
        WindowsSaverMode::Configure => run_configure(),
    }
}

/// Reattach to the console of the process that launched us, when there is one.
///
/// `gibson-app` is built as a Windows GUI-subsystem binary so neither the
/// screensaver nor the preview ever flashes a console window. The same binary
/// is also a CLI, so when it is launched from cmd or PowerShell we adopt the
/// parent's console to keep `--help`, `--version`, `--snapshot` and
/// `RUST_LOG` output visible. `AttachConsole` fails when the process already
/// has a console (the argument is ignored) and when the parent has none (the
/// screensaver host and double-click cases); both are silent no-ops, which is
/// exactly the screensaver behaviour we want.
///
/// Rust's std re-queries `GetStdHandle` on every read/write (see rust-src
/// `library/std/src/sys/stdio/windows.rs`: "Don't cache handles but get them
/// fresh for every read/write"), so attaching before any output is printed is
/// sufficient for `println!`/`eprintln!`; the CONOUT$/CONIN$ fallback below
/// only covers the case where the console attach did not set the process's
/// standard handles.
pub fn attach_parent_console() {
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        // No parent console: the normal screensaver path. Nothing to do, and
        // nothing may be printed here (there would be nowhere to print it).
        return;
    }

    for (id, device, for_writing) in [
        (STD_OUTPUT_HANDLE, "CONOUT$", true),
        (STD_ERROR_HANDLE, "CONOUT$", true),
        (STD_INPUT_HANDLE, "CONIN$", false),
    ] {
        let current: HANDLE = unsafe { GetStdHandle(id) };
        if !current.is_null() && current != INVALID_HANDLE_VALUE {
            continue; // the attach already wired this one up
        }
        let opened = if for_writing {
            std::fs::OpenOptions::new().write(true).open(device)
        } else {
            std::fs::OpenOptions::new().read(true).open(device)
        };
        if let Ok(file) = opened {
            use std::os::windows::io::AsRawHandle;
            unsafe { SetStdHandle(id, file.as_raw_handle() as HANDLE) };
            // The standard handle must stay valid for the process lifetime;
            // leak the File instead of closing it here.
            std::mem::forget(file);
        }
    }
}

/// Render into the host-owned preview window until it is destroyed.
fn run_preview(hwnd_value: isize, mut settings: Settings) -> Result<(), String> {
    let hwnd = hwnd_value as HWND;
    if hwnd.is_null() {
        return Err("preview mode: null window handle".to_string());
    }
    settings.preview = true;
    let scale = settings.render_scale;

    let start = Instant::now();
    let mut gibson: Option<Gibson> = None;
    let mut last_size = (0u32, 0u32);
    let mut tick = Instant::now();

    loop {
        pump_messages();
        if EXIT.load(Ordering::SeqCst) {
            break;
        }
        // The host may destroy the preview window at any time.
        if unsafe { IsWindow(hwnd) } == 0 {
            break;
        }

        let mut client: RECT = unsafe { std::mem::zeroed() };
        let size = if unsafe { GetClientRect(hwnd, &mut client) } != 0 {
            (
                (client.right - client.left).max(0) as u32,
                (client.bottom - client.top).max(0) as u32,
            )
        } else {
            (0, 0)
        };
        if size != last_size && size.0 > 0 && size.1 > 0 {
            match gibson.as_mut() {
                None => {
                    let target = raw_surface(hwnd)?;
                    let g = pollster::block_on(Gibson::new(
                        SurfaceTarget::Raw(target),
                        size.0,
                        size.1,
                        scale,
                        settings.clone(),
                    ))
                    .map_err(|e| format!("cannot initialize preview renderer: {e}"))?;
                    gibson = Some(g);
                }
                Some(g) => g.resize(size.0, size.1, scale),
            }
            last_size = size;
        }

        if let Some(g) = gibson.as_mut() {
            let t = start.elapsed().as_secs_f64();
            let (presented_before, _) = g.present_stats();
            if let Err(e) = g.frame(t) {
                log::error!("preview frame error: {e}");
            }
            let (presented_after, _) = g.present_stats();
            pace_frame(presented_after > presented_before, &mut tick);
        } else {
            std::thread::sleep(Duration::from_millis(16));
        }
    }
    Ok(())
}

/// `/s`: one borderless fullscreen window per monitor, each with its own
/// `Gibson`. Exits on any key / mouse button / wheel, or a mouse move greater
/// than 5 px from the position at startup.
fn run_fullscreen(settings: Settings) -> Result<(), String> {
    let hinstance = unsafe { GetModuleHandleW(null_mut()) };
    if hinstance.is_null() {
        return Err("cannot get the module handle".to_string());
    }

    let class_name = wide(WINDOW_CLASS);
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: null_mut(),
        hCursor: null_mut(),
        hbrBackground: null_mut(),
        lpszMenuName: null_mut(),
        lpszClassName: class_name.as_ptr(),
    };
    if unsafe { RegisterClassW(&wc) } == 0 {
        return Err("cannot register the screensaver window class".to_string());
    }

    let monitors = monitor_rects()?;
    if monitors.is_empty() {
        return Err("no monitors found".to_string());
    }

    // Hide the cursor and remember where it was (any later move > 5 px exits).
    unsafe { ShowCursor(0) };
    let mut cursor = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut cursor) };
    let _ = CURSOR_START.set(cursor);

    let mut windows: Vec<(HWND, Gibson)> = Vec::new();
    for rc in &monitors {
        let w = (rc.right - rc.left).max(0);
        let h = (rc.bottom - rc.top).max(0);
        if w == 0 || h == 0 {
            continue;
        }
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                null_mut(),
                WS_POPUP | WS_VISIBLE,
                rc.left,
                rc.top,
                w,
                h,
                null_mut(),
                null_mut(),
                hinstance,
                null_mut(),
            )
        };
        if hwnd.is_null() {
            log::error!(
                "cannot create a screensaver window for ({}, {})",
                rc.left,
                rc.top
            );
            continue;
        }
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
        }
        match create_gibson(hwnd, w as u32, h as u32, &settings) {
            Ok(g) => windows.push((hwnd, g)),
            Err(e) => {
                log::error!("monitor window renderer failed: {e}");
                unsafe { DestroyWindow(hwnd) };
            }
        }
    }
    if windows.is_empty() {
        unsafe { ShowCursor(1) };
        return Err("could not create any fullscreen window".to_string());
    }

    let start = Instant::now();
    let mut tick = Instant::now();
    while !EXIT.load(Ordering::SeqCst) {
        pump_messages();
        if EXIT.load(Ordering::SeqCst) {
            break;
        }
        let t = start.elapsed().as_secs_f64();
        let mut any_presented = false;
        for (hwnd, g) in windows.iter_mut() {
            if unsafe { IsWindow(*hwnd) } == 0 {
                EXIT.store(true, Ordering::SeqCst);
                break;
            }
            let (presented_before, _) = g.present_stats();
            if let Err(e) = g.frame(t) {
                log::error!("frame error: {e}");
            }
            let (presented_after, _) = g.present_stats();
            any_presented |= presented_after > presented_before;
        }
        pace_frame(any_presented, &mut tick);
    }

    for (hwnd, _) in &windows {
        unsafe { DestroyWindow(*hwnd) };
    }
    unsafe { ShowCursor(1) };
    Ok(())
}

/// `/c`: make sure the config exists and open it with the default editor.
fn run_configure() -> Result<(), String> {
    let path = config::default_path();
    config::ensure_file(&path)?;
    let file = path.to_string_lossy().into_owned();
    let launched = std::process::Command::new("cmd.exe")
        .args(["/C", "start", "", &format!("\"{file}\"")])
        .spawn();
    match launched {
        Ok(_) => Ok(()),
        Err(_) => std::process::Command::new("notepad.exe")
            .arg(&file)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("cannot open {}: {e}", path.display())),
    }
}

/// Pump all queued messages for this thread's windows.
fn pump_messages() {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            if msg.message == WM_QUIT {
                EXIT.store(true, Ordering::SeqCst);
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Any key, mouse button, or wheel ends the screensaver.
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
        | WM_XBUTTONDOWN | WM_MOUSEWHEEL => {
            EXIT.store(true, Ordering::SeqCst);
            0
        }
        WM_MOUSEMOVE => {
            let start = CURSOR_START.get().copied();
            let mut now = POINT { x: 0, y: 0 };
            GetCursorPos(&mut now);
            if let Some(s) = start {
                if (now.x - s.x).abs() > 5 || (now.y - s.y).abs() > 5 {
                    EXIT.store(true, Ordering::SeqCst);
                }
            }
            0
        }
        WM_ERASEBKGND => 1, // the swapchain paints everything; skip the erase flash
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Build the raw Win32 handles for a wgpu surface on `hwnd`.
fn raw_surface(hwnd: HWND) -> Result<wgpu::SurfaceTargetUnsafe, String> {
    let hinstance = unsafe { GetModuleHandleW(null_mut()) };
    let hwnd_nz = NonZeroIsize::new(hwnd as isize).ok_or("null window handle")?;
    let hinstance_nz = NonZeroIsize::new(hinstance as isize);
    let mut win32 = wgpu::rwh::Win32WindowHandle::new(hwnd_nz);
    win32.hinstance = hinstance_nz;
    // SAFETY: the handles point at a live window created by this process (or a
    // host window we were told to adopt) and stay alive for the Gibson lifetime.
    let display_handle = unsafe {
        wgpu::rwh::DisplayHandle::borrow_raw(wgpu::rwh::RawDisplayHandle::Windows(
            wgpu::rwh::WindowsDisplayHandle::new(),
        ))
    };
    let window_handle =
        unsafe { wgpu::rwh::WindowHandle::borrow_raw(wgpu::rwh::RawWindowHandle::Win32(win32)) };
    unsafe {
        wgpu::SurfaceTargetUnsafe::from_display_and_window(&display_handle, &window_handle)
            .map_err(|e| format!("invalid Win32 surface handles: {e}"))
    }
}

fn create_gibson(
    hwnd: HWND,
    width: u32,
    height: u32,
    settings: &Settings,
) -> Result<Gibson, String> {
    let target = raw_surface(hwnd)?;
    pollster::block_on(Gibson::new(
        SurfaceTarget::Raw(target),
        width.max(1),
        height.max(1),
        settings.render_scale,
        settings.clone(),
    ))
    .map_err(|e| format!("cannot initialize renderer: {e}"))
}

/// Collect the work-area rectangle of every monitor on the virtual desktop.
fn monitor_rects() -> Result<Vec<RECT>, String> {
    let mut rects: Vec<RECT> = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null_mut(),
            Some(monitor_enum_proc),
            &mut rects as *mut Vec<RECT> as LPARAM,
        )
    };
    if ok == 0 {
        return Err("EnumDisplayMonitors failed".to_string());
    }
    Ok(rects)
}

unsafe extern "system" fn monitor_enum_proc(
    monitor: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let rects = unsafe { &mut *(data as *mut Vec<RECT>) };
    let mut info: MONITORINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(monitor, &mut info) != 0 {
        rects.push(info.rcMonitor);
    }
    1 // continue enumerating
}

/// Sleep until the next ~60 Hz tick.
fn pace(tick: &mut Instant) {
    *tick += Duration::from_millis(16);
    let now = Instant::now();
    if *tick > now {
        std::thread::sleep(*tick - now);
    } else {
        *tick = now + Duration::from_millis(16);
    }
}

/// Pace a frame: 60 Hz when it actually presented, a slow poll when the surface skipped it
/// (window occluded/covered), so an invisible screensaver cannot spin a core.
fn pace_frame(presented: bool, tick: &mut Instant) {
    if presented {
        pace(tick);
    } else {
        std::thread::sleep(Duration::from_millis(250));
        *tick = Instant::now();
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
