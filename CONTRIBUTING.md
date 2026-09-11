# Contributing to Hack the Gibson

Thanks for looking. This is a spare-time project, and it is arranged so that a
change can be built and verified on **one** platform without owning all five:
the renderer is shared, and the hosts on top of it are thin.

Everything below is a command or a path that exists in this repository. If
anything here turns out to be wrong, that is a bug in this file, and a pull
request against it is welcome.

## Fastest path to pixels

```sh
git clone https://github.com/paulkiernan/gibson-screensaver
cd gibson-screensaver
cargo run --release -p gibson-app
```

A window opens on the flythrough; Esc or Q quits. That is the whole first-run
story on macOS, Linux and Windows.

`--release` matters more here than in most projects: the renderer is a
fill-rate-bound frame chain, and a dev build leaves the workspace's own crates
unoptimized. (Dependencies are already at `opt-level = 2` in the dev profile -
see the root `Cargo.toml` - but that is what makes the wgpu stack tolerable to
build in debug, not a substitute for measuring in release.)

Then, before you push:

```sh
make check
```

`make help` lists every convenience target. CI gates four more things on a pull
request - formatting, Clippy on the native workspace and on the wasm crate, and
the commit messages - and each one is a target too: `make fmt-check`,
`make lint`, `make lint-wasm` and `make commits`. They are cheap; run them
before you push.

## Prerequisites

| To do this | You need |
| --- | --- |
| Build anything | A Rust toolchain from rustup. `rust-toolchain.toml` pins one **exact version** (`1.98.1` as this is written), so rustup installs and selects it inside the repository automatically - never invoke `cargo +nightly` here. See [Bumping Rust](#bumping-rust). |
| Format and lint the way CI does | `rustup component add rustfmt clippy` |
| Build the web bundle | `rustup target add wasm32-unknown-unknown`, then `cargo install wasm-pack --locked`. CI installs the prebuilt wasm-pack 0.15.0. |
| Build a universal macOS screen saver (arm64 + x86_64) | `rustup target add aarch64-apple-darwin x86_64-apple-darwin`, on macOS |
| Build the macOS screen saver at all | Command Line Tools: `xcode-select --install`. **Xcode is not required.** `platform/macos/Makefile` compiles the Rust staticlib, links the Swift host with `swiftc`, `lipo`s the slices together, wraps the `.saver` bundle and ad-hoc signs it. |
| Build on Linux | `libx11-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev` (winit's wayland stack needs the headers at build time; x11-dl dlopens libX11 at runtime), plus a Vulkan driver to run anything - wgpu's native backend set on Linux is Vulkan only. |
| Keep `target/` from eating your disk | `CARGO_PROFILE_DEV_DEBUG=line-tables-only`. Debug info, not the test binaries, is what costs gigabytes; line tables are all a backtrace needs. `make check` sets it for you. |

The saver defaults to `ARCHS="arm64 x86_64"`; if you have only one of the two
targets installed, build your own architecture:

```sh
make -C platform/macos ARCHS=arm64
```

## The crate map

| Crate | What lives there |
| --- | --- |
| `gibson-types` | The **frozen** shared contract: world constants (`TOWER_PITCH`, `FLOOR_TILE_UNITS`, `ATLAS_*`, `FOG_*`, `MAX_ON_SCREEN_PIXELS`), `Settings`, `Palette`, the `#[repr(C)]` + `Pod` GPU instance structs, `AtlasImage`, `FloorMap`, `FrameData`. Every other crate compiles against it. |
| `gibson-scene` | The simulation: the closed Catmull-Rom flight path (39 waypoints rescued from the 2015 C++ original), camera banking ported from the SceneKit fork, city placement with per-tower heights capped by real path clearance, lane pulse streaks, block highlights, and the siege spread. `Scene::update` / `Scene::frame` are the per-frame API. |
| `gibson-atlas` | The procedural text atlas: 64 layers of 256x768 RGBA, 32 tower-face panels with two text variants each, drawn with fontdue from the embedded IBM Plex Mono and Michroma fonts in `assets/fonts/`. |
| `gibson-floor` | The procedural 96x96 toroidal circuit board on which the towers are the integrated circuits: packages with pin rings, strictly planar octilinear routing with via-pair layer changes, power rails, bus bundles, ground pour, silkscreen. |
| `gibson-render` | The wgpu 30 frame graph and the WGSL shaders in `src/shaders/`: SDF floor, instanced translucent towers with in-shader per-block animation, additive beams, the bloom chain, depth-reprojected motion blur, the ACES composite, and the Lottes-style CRT pass. `crates/gibson-render/tests/render.rs` is the renderer's acceptance suite. |
| `gibson-core` | The host-facing `Gibson` facade over `SurfaceTarget::{Window, Raw, Offscreen}`: `frame()` for continuous rendering, `snapshot()` for one offscreen still, plus the present/skip counters. |
| `gibson-app` | The winit desktop host, `--snapshot`, the TOML config, and the two screensaver hosts: the Windows `.scr` protocol (`/s`, `/p <hwnd>`, `/c`) and the Linux xscreensaver host (`--window-id <xid>` or `$XSCREENSAVER_WINDOW`). |
| `gibson-web` | The wasm-bindgen entry point: canvas boot, URL query parameters, the `requestAnimationFrame` loop. wasm-only (`#![cfg(target_arch = "wasm32")]`). |
| `gibson-ffi` | The C ABI bridge the Swift screen saver links as a staticlib; header in `platform/macos/include/gibson_ffi.h`. |

Around the crates: `platform/macos` (Swift saver + its Makefile),
`platform/windows` (README only - the host is `gibson-app`), `platform/linux`
(the xscreensaver descriptor and the X11 smoke test), `web/` (the static site;
`web/pkg` is built by `make web` and gitignored), and `docs/`. Of `docs/`, two
files matter beyond reading: `docs/film-reference.md` is the visual acceptance
standard, and `docs/scratch/` is the gitignored place for probe output and local
reference stills.

## Running things

### Desktop app

```sh
cargo run --release -p gibson-app                    # windowed, Esc or Q quits
cargo run --release -p gibson-app -- --fullscreen
cargo run --release -p gibson-app -- --help          # the full flag list
RUST_LOG=info cargo run --release -p gibson-app      # logs to stderr
```

Settings are read from `<config-dir>/gibson-screensaver/gibson.toml`
(`~/Library/Application Support` on macOS, `~/.config` on Linux, `%APPDATA%` on
Windows), which is written with defaults and comments on first run. Command-line
flags override the file; every value is clamped on load, so a bad number cannot
put the renderer out of range.

### Deterministic stills

```sh
cargo run --release -p gibson-app -- \
  --snapshot out.png --size 1920x1080 --time 12 --seed 42
```

This renders one frame offscreen and exits: no window, no pixel cap, and
byte-identical output for a fixed `--seed`. **Always pass `--seed`.** The
default `seed = 0` means "derive one from the clock", so a still without it
cannot be compared against anything, including itself a minute later.

`--time` selects where along the loop the still is taken, and
`--palette normal|siege|cycle` selects the colour treatment, so a still of a
specific look needs both. The stills committed in `docs/screenshots/` were
graded at 1080p, and `docs/film-reference.md` ends with the checks a visual
change is graded against - read that list before posting a screenshot of a look
change.

### Web build

```sh
rustup target add wasm32-unknown-unknown   # once
cargo install wasm-pack --locked           # once
make web                                   # wasm-pack build ... --out-dir web/pkg
cd web && python3 -m http.server 8080      # then open http://localhost:8080/
```

The page must be served over HTTP: the wasm module needs response headers, so a
`file://` URL will not work. Settings come from query parameters, which is also
how you test a specific look without touching a config file:

```text
http://localhost:8080/?palette=cycle&grid=40&pulses=200
```

The README lists every supported parameter. `#status` on the page and the
browser console both report startup failures - by design, a black canvas with
an empty status line is a bug in the host, not an expected state.

### macOS screen saver

```sh
make saver             # -> platform/macos/build/Gibson.saver
make install-saver     # build, copy to ~/Library/Screen Savers, restart the saver process
make uninstall-saver
```

Then choose "The Gibson" in System Settings > Wallpaper > Screen Saver. A build
from source needs no quarantine step (nothing was downloaded). If you change
something the icon or preview shows, note that
`platform/macos/thumbnail.png` and `platform/macos/thumbnail@2x.png` are
committed files that the Makefile merely copies into the bundle - no target
regenerates them.

If the saver appears in the list but never draws, that is the classic symptom of
a quarantined bundle, not of a broken build; the README's
`xattr -dr com.apple.quarantine` section covers it.

### Tests

```sh
make check   # cargo test --workspace --exclude gibson-web, line-tables-only debuginfo
make test    # the same command without the debuginfo prefix
```

Two details are deliberate:

- **`--exclude gibson-web`**: that crate is wasm-only, so on a native host it
  compiles to an empty rlib and a native test run would prove nothing about it.
  To check a web change compiles, run `make web` (that is exactly what CI's
  `wasm` job does); to exercise it, serve the build and open it.
- **the debuginfo prefix**: `CARGO_PROFILE_DEV_DEBUG=line-tables-only`. A full
  workspace run with default debug info costs several gigabytes of `target/`;
  line tables keep backtraces useful at a fraction of that.

CI runs a narrower set on Windows and Linux
(`cargo test -p gibson-scene -p gibson-atlas -p gibson-floor -p gibson-app`)
because hosted runners have no GPU, and the full workspace on macOS. The
renderer's tests need a real adapter: each one prints
`skipped: no graphics adapter/device available` and passes when there is none,
so a run with no GPU is not evidence about the renderer.

The image-producing tests are gated behind an environment variable so a normal
run stays fast:

```sh
GIBSON_RENDER_PROBE=1 CARGO_PROFILE_DEV_DEBUG=line-tables-only \
  cargo test -p gibson-render --test render probe_ -- --nocapture
```

They write 1080p PNGs into `docs/scratch/` (gitignored) - the floor from above,
the floor's feature set, a face study, a populated frame, the siege wave and
siege towers, and panel legibility. `GIBSON_RENDER_PROBE_OUT` redirects the
path and `GIBSON_RENDER_CRT` sets the CRT amount. When the thing you are judging
is one layer, these are quicker to read than a full city render.

### Generators, without the renderer

```sh
cargo run -p gibson-floor --example floor_dump -- [seed] [span] [x0] [z0]
cargo run --release -p gibson-atlas --example atlas_dump [LAYER ...]
```

`floor_dump` prints a copper summary plus an ASCII view of the generated tile;
`atlas_dump` writes layer PNGs to `docs/scratch/atlas/`. Both are much faster
than rendering when what you are changing is a content generator.

## Conventions that will get a pull request sent back

Each of these has cost someone real debugging time. They are not style
preferences.

### `gibson-types` is a frozen contract

`gibson-types` holds the constants, `Settings`, `Palette`, the
`#[repr(C)]`/`Pod` GPU instance structs, `AtlasImage`, `FloorMap` and
`FrameData`. Every crate compiles against it, and the renderer reads those
structs out of GPU memory against a matching WGSL declaration in
`crates/gibson-render/src/shaders/`.

Changes to it are made in a dedicated amendment step, never alongside feature
work: the contract change, and every crate it forces to change, land in one
reviewable commit. In the commit convention that is a breaking change
(`feat(types)!: ...` plus a `BREAKING CHANGE:` footer), which is exactly what it
is. The crate's own module documentation records the three amendments made so
far (a darker normal tower body, per-tower siege blending, and the widened
`FloorMap::data` cell encoding) as the pattern to follow.

Two things inside it are load-bearing beyond the type signatures. The grid
conventions - pitch 30, towers at `x = 15 (mod 30)` / `z = 0 (mod 30)`, lanes at
`x = 0` and `z = 15`, and `FLOOR_TILE_UNITS = 240`, which is exactly eight
pitches - are what let the floor generator place grid-exact keep-out zones under
tower footprints. And the 39 flight-path waypoints are film-era artifacts:
never nudge them to accommodate a geometry change, cap the geometry instead (the
city does exactly that, sampling real path clearance per tower).

### Renderer work stays inside the WebGL2 envelope

The web build falls back to WebGL2 where a browser has no WebGPU, so this is
binding on all renderer work, not just the web crate:

- no storage buffers and no compute shaders;
- one uniform buffer per binding, 16 KiB or less;
- per-instance data through `VertexStepMode::Instance` vertex buffers;
- no MSAA, and no vertex-shader texture reads;
- no `textureLoad` on a depth texture - naga's GLSL backend rejects it outright,
  which is why the depth the motion blur reprojects from rides in an `R32Float`
  colour attachment;
- every derivative call (`fwidth`, implicit-LOD sampling) in uniform control
  flow;
- no pipeline whose colour targets differ in blend or write mask
  (`INDEPENDENT_BLEND` does not exist in WebGL2);
- resource shapes that fit `Limits::downlevel_webgl2_defaults()`.

The list is repeated at the top of `crates/gibson-render/src/lib.rs` with the
reasons attached. Neither `cargo test` on your machine nor the wasm build job
will catch a violation: it fails at run time, in a browser, on someone else's
machine.

### No wall-clock time in anything that compiles to wasm

On `wasm32-unknown-unknown`, `std::time::Instant::now()` and
`SystemTime::now()` do not return a wrong value - they
`panic!("time not implemented on this platform")`. One `Instant::now()` added
for a debug log line once blackened the entire web build.

The wasm-reachable set is `gibson-web` and everything it pulls in:
`gibson-core`, `gibson-render`, `gibson-scene`, `gibson-atlas`, `gibson-floor`,
`gibson-types`. `gibson-core::time_derived_seed` is the pattern to copy when a
clock is genuinely needed: the wasm arm returns a constant instead. Examples
under `crates/*/examples/` and the `gibson-app` hosts are native-only, so a
clock there is fine.

### Visual changes need visual proof

Render before and after with the same `--seed` and the same flags, and put both
images in the pull request. `docs/film-reference.md` is the standard, and its
closing checklist is what a look change is measured against - text blooms
without washing out, floor traces read as violet with visible elbows and pads,
distant towers fade blue to black (red-brown in SIEGE), at least one directory
hero tower and one visible pulse streak. Film frames are copyrighted and are not
committed; link or describe the reference instead of pasting one.

### A test has to be able to fail

A counter that can report success while nothing happened is worse than no
counter at all. Two real cases from this project's history:

- A frame-rate counter counted frames the renderer had *skipped*, and duly
  reported `41000 fps` over an occluded window that was doing no work at all.
  That hid both the real cost and a power bug, because the host was also
  busy-looping a core while frames were skipped. Counters now report presented
  and skipped frames separately, and the host backs off to a slow poll while
  frames are skipping.
- The macOS saver once shipped black, because "the process did not crash" was
  being read as "it drew". The Linux smoke test now asserts that a presented
  frame count actually increased before it calls a run good.

So: assert on what a consumer observes - pixels, presented frames, exit codes,
parsed values - not on the fact that a function was called, that a field was
copied, that a default is still the default, or that a string appears in a
source file. A test that passes when the feature is deleted is worse than no
test: it is a false signal other people will trust. `gibson-render`'s tests
render offscreen and inspect sRGB8 pixels, and the generators' tests assert
structural invariants (the floor's tests measure directly that two nets never
share a copper cell, for instance); follow whichever of those fits the change.

### Formatting and lints

`rustfmt.toml` is authoritative and deliberately almost empty: rustfmt's
defaults are the Rust Style Guide. Only `edition = "2021"` is set, to match the
workspace. No nightly-only key (`imports_granularity`, `group_imports`,
`wrap_comments`, ...) appears there, because on the stable toolchain that
`rust-toolchain.toml` pins, rustfmt warns about such a key and ignores it - it
would quietly do nothing here while reformatting everything for someone running
nightly rustfmt by hand.

```sh
make fmt         # cargo fmt --all
make fmt-check   # cargo fmt --all --check - what CI runs
make lint        # cargo clippy --workspace --exclude gibson-web --all-targets
make lint-wasm   # the same for gibson-web, on wasm32
```

The lint *levels* are neither CI flags nor `#![deny]` attributes in the source:
they are the root `Cargo.toml`'s `[workspace.lints]` table, which every member
opts into with `[lints] workspace = true`. `clippy::all` (correctness,
suspicious, style, complexity, perf) and rustc's `unused` group are `deny`
there, so clippy fails on a warning with no `-D warnings` anywhere, and a run on
your machine reports exactly what CI reports. No crate carries a blanket allow.
An `#[allow]` in the source is fine when the lint is genuinely wrong there, but
it has to say why in a comment, and it will be asked about in review.

CI runs all of that in its `lint` job. `make lint` excludes one crate because
`gibson-web` is `#![cfg(target_arch = "wasm32")]`: on a native host it compiles
to nothing, so a native lint would prove nothing about it. Lint its real code
with the target installed once:

```sh
rustup target add wasm32-unknown-unknown
make lint-wasm
```

The tree was formatted once, in a single dedicated commit listed in
`.git-blame-ignore-revs` - which is why that reformat did not move anyone's
blame. Two consequences for you. Run

```sh
git config blame.ignoreRevsFile .git-blame-ignore-revs
```

once per clone, and never fold a repo-wide reformat into a feature change: it
would move blame for every line of the tree again and collide with every branch
in flight. Format only the files you actually touch (`.editorconfig` is there so
your editor does not reformat them differently from rustfmt).

### Bumping Rust

`rust-toolchain.toml` pins the compiler by exact version (`1.98.1` as this is
written), and no workflow installs a toolchain of its own: rustup reads that file
in the checkout, so CI, the Makefile and your shell all get the same `rustc` and
`clippy`. The lint job prints them (`rustup show active-toolchain`, then
`cargo --version` and `cargo clippy --version`), so a divergence shows up in the
log instead of as a failure nobody can explain.

The pin is what makes `deny` in `[workspace.lints]` safe. Clippy grows new lints
every release, and on a floating `stable` the first one that fires lands on
whichever pull request happens to be open - failing a change that has nothing to
do with it, on a compiler neither the author nor CI chose. Pinned, the same lint
arrives once, in a commit whose whole subject is the bump.

So bumping is a deliberate change, and it is more than editing one line:

1. `rustup toolchain install <version> --component rustfmt,clippy`, then set
   `channel` in `rust-toolchain.toml` to that version.
2. Run the whole gate: `make fmt-check`, `make lint`, `make lint-wasm`,
   `make check`, and the snapshot command from "Deterministic stills" with the
   same `--seed`. A new compiler is a change to the program that renders the
   pixels, so the still is the evidence that the pixels did not change.
3. Deal with whatever the new Clippy found *in that commit* - fix it, or allow
   it narrowly with a comment saying why the lint is wrong here.
4. Check the new version is not ahead of what the packaging builds with. The AUR
   `PKGBUILD` is the only one of the three that compiles anything, and it uses
   Arch's distro `cargo`, which does not read `rust-toolchain.toml` at all
   (Arch ships this same version today); the Scoop and Homebrew manifests
   download prebuilt release assets. A pin ahead of the distro compiler is a
   broken build that CI cannot see.

Dependabot does not manage `rust-toolchain.toml`; it watches `Cargo.toml` and
the workflows.

### Commit messages: Conventional Commits, required

```text
type(scope): subject
```

- Imperative mood - `stop the leak`, not `stopped` or `stops`.
- Lower case, no trailing period, subject under about 72 columns.
- Body wrapped at 72 columns, saying *why*: the diff already says what.
- Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`,
  `ci`, `chore`, `revert`.
- Scopes are the crate or host: `types`, `scene`, `atlas`, `floor`, `render`,
  `core`, `app`, `web`, `ffi`, `macos`, `linux`, `windows`, plus `ci` and
  `docs`. Omit the scope when a change is genuinely repo-wide.
- A breaking change puts `!` after the scope - or straight after the type when
  there is no scope, `feat!: ...` - and explains itself in a `BREAKING CHANGE:`
  footer. An amendment to the frozen `gibson-types` contract is precisely that
  case, since every crate compiles against it.

Examples, adapted from real commits in this repository:

```text
fix(render): stop wgpu refusing every frame as occluded
fix(web): stop the canvas rendering black, and report startup failures
feat(floor): route nets planar with via-pair layer changes
```

CI checks the shape, in the `lint` job. Three things about how it does it are
worth knowing, because they are what make it usable rather than annoying:

- **It judges only the commits a pull request adds.** The range starts at the
  merge base, so the history from before this convention was adopted is never in
  it. A pull request cannot fail on a message it did not write.
- **The pull request title is checked too**, because a squash merge takes the
  title as the commit subject. Title it the way you would title the commit.
- **An unknown type or scope is rejected with the allowed set in the failure
  message**, so fixing it takes one try rather than a hunt.

The same gate runs locally:

```sh
make commits                  # the commits this branch adds on top of origin/main
make commits BASE=origin/release
```

That is `scripts/check-commit-messages.sh <rev-range>`; with no argument it reads
subjects from stdin, one per line, which is how you check a message before you
commit it:

```sh
echo 'fix(render): stop wgpu refusing every frame as occluded' \
  | scripts/check-commit-messages.sh
```

To get the type and scope lists in your editor:

```sh
git config commit.template .gitmessage
```

The history before this convention was adopted does not follow it, and it is
not being rewritten: older commits read `Batch 2: tuning pass...` or
`floor: no crossings - ...`. Leave them as they are.

### No emoji

In code, docs, commit messages and issue reports. This is a running house rule,
and it applies to the files in this scaffolding too.

## What makes a good pull request here

- **Small and single-purpose.** One idea per pull request, and no drive-by
  reformatting or refactoring mixed into it.
- **Says what was actually run.** The pull request template has a platform
  checklist; tick only what you exercised. "Built on Linux, ran the desktop app,
  did not run the saver" is a useful, reviewable statement - a fully ticked
  list is not, because almost nobody can test all five hosts.
- **Brings evidence with it.** For a visual change, two stills from the snapshot
  command with the same seed. For a bug fix, the reproduction that failed
  before and passes now. For a renderer change, the machine and GPU it was
  measured on.
- **Follows the conventions above**, including the commit messages.
- **Explains the trade-off** where there was one. This project has a lot of
  deliberate decisions (the on-screen pixel cap, the planar floor routing, the
  Lottes CRT over a cheaper fake) and a reviewer will want the reasoning, not
  only the result.

## Where the easy wins are

Honest gaps, roughly in order of how much a report or a fix would help:

- **The Linux xscreensaver host has had the least real-hardware exercise in the
  project.** CI runs it for real - it creates an X window under Xvfb, hands the
  id to the binary the way xscreensaver does, and asserts frames were presented
  - but that is a software Vulkan rasteriser on a headless server. Real drivers,
  multi-monitor and Xinerama/RANDR setups, and the xscreensaver daemon itself
  are untested. If you have an X11 machine, this is the highest-value testing
  area; the report we want includes the adapter line and the presented/skipped
  counters (`bash platform/linux/smoke-test.sh target/release/gibson-app`
  reproduces the CI run locally). On Wayland, xscreensaver is not an option -
  `gibson-app --fullscreen` under `swayidle` is, and the exact command is in
  `platform/linux/README.md`.
- **Safari is untested on the web build.** The WebGPU path wants Safari 26+, and
  the WebGL2 fallback is the path most other browsers without WebGPU take. Both
  are worth an issue that says which browser, which back end the page reported,
  and what the console said.
- **The Windows host's protocol paths have far less mileage than the renderer.**
  The renderer is exercised constantly by every host; the `/s`, `/p <hwnd>` and
  `/c` handling, per-monitor fullscreen window creation, and the input that
  dismisses the saver have been used far less. If something misbehaves there it
  is more likely to be host code than renderer code - include the arguments
  Windows passed.
- **Performance on non-Apple GPUs.** The frame budget was measured on an Apple
  M3 (roughly 5 ms per megapixel for the full chain, which is why on-screen
  targets are capped at 2.8 Mpx and offscreen renders are not). Numbers from
  other GPUs and drivers, especially anything that misses 60 Hz at the cap,
  would make that budget better grounded.
- **Known and accepted limitation, not a bug to fix:** the macOS bundle is
  ad-hoc signed and not notarized, because there is no paid developer account
  behind the project. The README's quarantine instructions are the supported
  answer; a pull request cannot fix it.

## Reporting bugs, and other ways to help

- **Bugs**: the bug report form asks for the host, OS, GPU, version and log
  output, and tells you where each host keeps its log (`RUST_LOG=info` for the
  desktop app,
  `log show --predicate 'subsystem == "org.hackthegibson.TheGibson"'` for the
  saver, the browser console for the web build).
- **"This does not look like the film"**: use the visual fidelity form. It is a
  first-class report type here, not a curiosity - fidelity to
  `docs/film-reference.md` is the project's standard. Film frames are
  copyrighted, so link or describe the reference rather than pasting a frame.
- **New ideas**: the feature request form.
- **Security issues**: do not open a public issue - see
  [SECURITY.md](SECURITY.md) for what is in scope and how to report privately.
- **Everything else**: there is no discussion forum or chat; issues are the
  channel, and the [live demo](https://paulkiernan.github.io/gibson-screensaver/)
  is the quickest way to see the current state without building anything.

All participation is covered by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

Licensed GPL-3.0-or-later; by contributing you agree your work is too. The
bundled fonts are SIL Open Font License (see `assets/fonts/OFL-*.txt`).
