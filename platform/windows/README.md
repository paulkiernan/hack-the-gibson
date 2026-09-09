# Hack the Gibson — Windows `.scr` screensaver host

`gibson-app.exe` doubles as a classic Windows screensaver. The same binary
understands the desktop flags (`--fullscreen`, `--snapshot`, ...) and the
screensaver protocol (`/s`, `/p <hwnd>`, `/c`).

## Install

1. Build on Windows (or take the `Gibson.scr` artifact from CI):

       cargo build --release -p gibson-app
       copy target\release\gibson-app.exe Gibson.scr

2. Right-click `Gibson.scr` and choose **Install** (Windows copies it to your
   system directory and registers it), or copy it to
   `C:\Windows\System32\Gibson.scr` yourself and pick "The Gibson" in
   Settings > Personalization > Lock screen > Screen saver settings.

That is all the OS needs. The screensaver modes work like any classic `.scr`:

- `/s` runs full-screen on every monitor (any key, mouse button, wheel, or a
  mouse move greater than 5 px exits),
- `/p <hwnd>` renders the small preview tile,
- `/c` opens the settings file.

## Settings

All tunables live in `%APPDATA%\hack-the-gibson\gibson.toml` (created with
defaults and explanatory comments on first run — including the first `/c`).
Editing it while the screensaver is not running takes effect on the next
activation; the values are the same ones documented in the project README.

## Status

**Compile-verified in CI, not runtime-tested.** No Windows machine is
available to the developers, so the `.scr` protocol paths (`/s`, `/p`, `/c`)
have never been exercised live — the CI job only proves the binary builds.
The windowed desktop app (`gibson-app.exe` with no arguments) is the
runtime-verified path on the platforms the developers actually use.
