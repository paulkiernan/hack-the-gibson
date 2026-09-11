# Hack the Gibson — Linux xscreensaver host

`gibson-app` doubles as an xscreensaver "external window" hack: xscreensaver
creates a window, hands its XID to the process, and the process renders the
flythrough into that window at ~60 Hz until xscreensaver destroys it.

How the XID arrives differs between the two things that launch a hack, and
both paths matter:

- **The daemon** (`xscreensaver` itself) runs the `programs:` line from
  `~/.xscreensaver` verbatim through the shell and passes the window only in
  the `XSCREENSAVER_WINDOW` environment variable. It never appends a
  `--window-id` argument.
- **The settings dialog** (`xscreensaver-settings`) appends
  `--window-id 0x<id>` to the command line it launches for the embedded
  preview, and also sets `XSCREENSAVER_WINDOW` (it does the former so that a
  third-party saver which ignores the window would pop up its own full-screen
  window instead of failing).

`gibson-app` handles both: it selects the X11 host when `--window-id <xid>`
is given or when `XSCREENSAVER_WINDOW` is set. That is why the descriptor
carries no `<command>` element — anything listed there is added verbatim to
the `programs:` line and would break one path or the other. See the comment
at the top of [`hack-the-gibson.xml`](hack-the-gibson.xml).

## Why the name is `hack-the-gibson`, not `gibson`

Upstream xscreensaver has shipped its own, unrelated `gibson` hack since
version 5.44 (written by Jamie Zawinski in 2020, and also about the 1995
film). Installing anything of ours under that name collides twice over:

- Arch's `xscreensaver` package already owns its `gibson` executable, the
  matching `gibson.xml` in the xscreensaver config directory, and
  `/usr/share/man/man6/gibson.6.gz`, so installing our descriptor at that path
  is a pacman conflicting-files error (and a manual `cp` silently clobbers
  another package's file).
- xscreensaver prepends its own hacks directory to `$PATH` when resolving the
  `programs:` list, so a second `gibson` on `PATH` would lose to upstream's
  anyway.

The name is not cosmetic: `xscreensaver-settings` finds a hack's descriptor
by taking the basename of the program and looking for
`<hack-configuration-path>/<basename>.xml`. The installed executable and the
descriptor therefore have to agree, and both are `hack-the-gibson`.

## Install

1. Build the binary:

       cargo build --release -p gibson-app
       install -Dm755 target/release/gibson-app ~/.local/bin/hack-the-gibson

   The basename must be `hack-the-gibson`; `~/.local/bin` just has to be on
   `PATH`.

2. Install the hack descriptor so the xscreensaver settings dialog knows the
   options and can find the matching program:

       sudo install -Dm644 platform/linux/hack-the-gibson.xml \
            /usr/share/xscreensaver/config/hack-the-gibson.xml

3. Add a `programs:` line to `~/.xscreensaver` (create it with
   `xscreensaver-demo` first if it does not exist). No arguments:

       programs: hack-the-gibson

   `xscreensaver` resolves `hack-the-gibson` on `PATH`; pass an absolute path
   instead if you installed it elsewhere, e.g.

       programs: /home/you/.local/bin/hack-the-gibson

4. Pick "Hack the Gibson" in `xscreensaver-demo` and set a blanking mode /
   timer as usual. The demo's Settings dialog exposes "Fly speed" and "Pulse
   streaks"; everything else is configured in `~/.config/hack-the-gibson/gibson.toml`
   (created with defaults and comments on first run).

The X11 host renders through wgpu's Vulkan backend into the window
xscreensaver provides, so it runs under a normal X11/XWayland session.

## Wayland

xscreensaver's architecture is X11-only. On a Wayland session use `swayidle`
(compatible with `sway`/`hyprland`/`wayfire`/...) plus the plain fullscreen
window mode instead:

    swayidle -w timeout 600 'gibson-app --fullscreen' resume 'pkill gibson-app'

or run `gibson-app --fullscreen` manually and quit with Esc or Q.

## Status

**Smoke-tested in CI under Xvfb with a software rasteriser; not verified on
real hardware with a real GPU.** CI runs
[`smoke-test.sh`](smoke-test.sh), which covers both ways a hack is launched:
it creates a real X window and hands its id to the built binary through
`XSCREENSAVER_WINDOW` the way the daemon does, then does it again with
`--window-id <id>` the way the settings dialog's preview does. Each case must
adopt the window, present at least one frame (asserted from the renderer's own
presented-frame counter, not from "it did not crash"), and exit cleanly when
the window is destroyed. The script also checks the descriptor against the
binary: the name matches the installed basename, there is no `<command>`
element, every slider arg has its `%` value placeholder, and every switch the
descriptor can emit is accepted by `--help`. All of that is a software adapter
under a headless X server, so it says nothing about GPU driver behaviour, real
Xinerama/RANDR setups, or the xscreensaver daemon itself. If it fails on your
machine, report the `RUST_LOG=info XSCREENSAVER_WINDOW=<xid> hack-the-gibson`
log output.

## Testing it yourself

With any X server running (an Xvfb instance is enough):

    cargo build --release -p gibson-app
    bash platform/linux/smoke-test.sh target/release/gibson-app

The script compiles a tiny X client (`x11-test-window.c`), has it create and
map a window, prints the window id, runs the hack against that window twice
(once through the environment, once through `--window-id`), destroys the
window, and checks that the hack exited cleanly after presenting frames. It
prints the window id, the adapter line, and the presented/skipped counters
either way, and exits non-zero with the hack's whole log on any failure.
