//! Hack the Gibson — desktop hosts.
//!
//! One binary, four hosts:
//! - a windowed desktop app (winit, all platforms),
//! - `--snapshot <png>`: a deterministic offscreen still for CI/screenshots,
//! - a Linux xscreensaver hack rendering into an existing X11 window,
//! - a Windows `.scr` implementing the classic `/s`, `/p <hwnd>`, `/c` protocol.
//!
//! Settings precedence everywhere: `defaults < gibson.toml < CLI overrides`.

// The shipped `Gibson.scr` must be a GUI-subsystem binary: a console-subsystem
// screensaver makes Windows flash a cmd window over the lock screen on every
// activation. The console CLI surface (`--help`, `--snapshot`, RUST_LOG output)
// is preserved by reattaching to the launching console in `main` — see
// `windows::attach_parent_console`. Kept unconditional rather than gated on
// `not(debug_assertions)` so the configuration that ships is the one developers
// run every day (attach regressions show up in debug), and because
// `cargo test` output is unaffected: cargo captures test output through
// inherited pipe handles, while the subsystem only controls whether Windows
// allocates a console.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod cli;
mod config;
mod desktop;
mod error_policy;
mod saver_args;
mod snapshot;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
mod windows;

use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    // First thing: on Windows reattach to the launching console so the CLI
    // surface (--help/--version/errors/RUST_LOG) still prints. No-op when there
    // is no parent console (screensaver host, double-click) — the normal path.
    #[cfg(windows)]
    windows::attach_parent_console();

    env_logger::init();

    // The Windows screensaver protocol hands the .scr arguments clap cannot
    // parse (`/s`, `/p <hwnd>`, `/c[:hwnd]`, with assorted separators), so
    // detect those before clap sees the command line. The detector itself is
    // platform-independent and unit-tested on every host.
    #[cfg(windows)]
    {
        let argv: Vec<String> = std::env::args().collect();
        if let Some(mode) = saver_args::detect(&argv[1..]) {
            return finish(windows::run(mode));
        }
    }

    let cli = cli::Cli::parse();

    // Linux xscreensaver host: xscreensaver launches the hack with
    // `--window-id <xid>` (or sets XSCREENSAVER_WINDOW); adopt that window.
    #[cfg(target_os = "linux")]
    if cli.x11_window_arg().is_some() {
        return finish(linux::run(&cli));
    }

    if cli.snapshot.is_some() {
        finish(snapshot::run(&cli))
    } else {
        finish(desktop::run(&cli))
    }
}

fn finish(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gibson-app: {e}");
            ExitCode::FAILURE
        }
    }
}
