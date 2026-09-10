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

**Verified on real hardware.** The maintainer has run this host on Windows and
confirmed it works, and CI builds and tests the binary on every push.

One caveat worth keeping in mind: the renderer itself is shared with every
other host and is exercised constantly, but the Windows-specific plumbing —
the `/s`, `/p <hwnd>` and `/c` protocol paths, per-monitor fullscreen window
creation, and the input handling that dismisses the saver — has had far less
mileage than the macOS and web paths. If something misbehaves there, it is
more likely to be in this file's code than in the renderer, so please open an
issue with the argument the system passed.
