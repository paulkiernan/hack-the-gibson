# Hack the Gibson — Linux xscreensaver host

`gibson-app` doubles as an xscreensaver "external window" hack: xscreensaver
creates a window and hands it to the process with `--window-id <xid>` (or the
`XSCREENSAVER_WINDOW` environment variable), and the process renders the
flythrough into that window at ~60 Hz until xscreensaver destroys it.

## Install

1. Build the binary:

       cargo build --release -p gibson-app
       cp target/release/gibson-app ~/.local/bin/gibson   # anywhere on PATH

2. Install the hack descriptor so the xscreensaver settings dialog knows the
   options:

       sudo cp platform/linux/gibson.xml /usr/share/xscreensaver/config/gibson.xml

3. Add a `programs:` line to `~/.xscreensaver` (create it with
   `xscreensaver-demo` first if it does not exist):

       programs: gibson -root

   `xscreensaver` resolves `gibson` on PATH; pass an absolute path if you
   installed it elsewhere, e.g.

       programs: /home/you/.local/bin/gibson -root

4. Pick "Hack the Gibson" in `xscreensaver-demo` and set a blanking mode /
   timer as usual. The demo's Settings dialog exposes "Fly speed" and "Pulse
   streaks"; everything else is configured in `~/.config/hack-the-gibson/gibson.toml`
   (created with defaults and comments on first run).

The X11 host renders through wgpu's Vulkan (or GL) backend into the window
xscreensaver provides, so it runs under a normal X11/XWayland session.

## Wayland

xscreensaver's architecture is X11-only. On a Wayland session use `swayidle`
(compatible with `sway`/`hyprland`/`wayfire`/...) plus the plain fullscreen
window mode instead:

    swayidle -w timeout 600 'gibson-app --fullscreen' resume 'pkill gibson-app'

or run `gibson-app --fullscreen` manually and quit with Esc or Q.

## Status

**Compile-verified in CI, not runtime-tested.** No X server is available on
the development/CI machines, so the X11 path has never been exercised live.
If it fails on your machine, report the `RUST_LOG=info gibson-app --window-id
<id>` log output.
