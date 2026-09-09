# Hack the Gibson

A screensaver and desktop toy that flies through the Gibson: the fictional
supercomputer from the 1995 film _Hackers_, visualized as a city of
translucent data towers standing in blackness on a circuit-board floor.

This is a Rust + wgpu rewrite of John Serafino's 2015 Irrlicht/OpenGL
original. One renderer core drives every platform — a macOS screen saver, a
Windows `.scr`, a Linux xscreensaver hack, a windowed desktop app, and a
WebGPU/WebGL2 web build — and the visuals are graded against real frames
from the film rather than guessed; see
[docs/film-reference.md](docs/film-reference.md) for the sources and the
production research.

![A low pass down the lanes of the Gibson: translucent blue towers, cyan mosaic text, and pulse streaks along the black floor](docs/screenshots/lane.png)

## Live demo

**<https://paulkiernan.github.io/hack-the-gibson/>**

The page uses WebGPU where the browser exposes it and falls back to WebGL2
otherwise — so Chrome 113+, Safari 26+, and Firefox with WebGPU enabled run
the WebGPU path, and any WebGL2-capable browser without WebGPU still gets
the scene through the fallback. The URL accepts query parameters — see the
Web entry under [Settings](#settings).

## Screenshots

![Overhead sweep: tower tops read teal-green, distant towers dissolve into blue haze](docs/screenshots/overhead.png)

![The siege palette from the film's "under attack" sequence: orange text, magenta-pink towers, ice-blue floor traces](docs/screenshots/siege.png)

## Install

### macOS screen saver

Requires macOS 14 or later. Either build it yourself (see [Building](#building)),
or download `Gibson.saver.zip` from the CI artifacts (or a release) on the
[GitHub Actions page](https://github.com/paulkiernan/hack-the-gibson/actions).

```bash
# If you downloaded the artifact, verify it first:
#   unzip the artifact into a folder, then, from inside that folder:
shasum -a 256 -c SHA256SUMS

# Install (or install a build with):
make install-saver
```

`make install-saver` copies `Gibson.saver` to `~/Library/Screen Savers/` and
restarts the screen-saver process; then pick **The Gibson** in System
Settings > Wallpaper > Screen Saver.

**Gatekeeper note:** the bundle is ad-hoc signed and not notarized. If macOS
refuses the downloaded copy the first time, right-click `Gibson.saver` and
choose Open, or approve it in System Settings > Privacy & Security, and copy
it into `~/Library/Screen Savers/` yourself. The `SHA256SUMS` file shipped
next to the zip lets you verify the download before you do any of that.

Remove it with `make uninstall-saver`.

### Windows `.scr`

Build on Windows (or take the `Gibson.scr` artifact from CI), rename the
binary, and install it the classic way:

```text
copy target\release\gibson-app.exe Gibson.scr
```

Right-click `Gibson.scr` and choose **Install**, or copy it to
`C:\Windows\System32\Gibson.scr` and pick "The Gibson" in Settings >
Personalization > Lock screen > Screen saver settings. The same binary
understands `/s` (full screen), `/p <hwnd>` (preview tile), and `/c` (opens
the settings file). Full steps are in
[platform/windows/README.md](platform/windows/README.md).

### Linux xscreensaver

`gibson-app` doubles as an xscreensaver "external window" hack. Install the
built binary somewhere on `PATH`, copy
[`platform/linux/gibson.xml`](platform/linux/gibson.xml) to
`/usr/share/xscreensaver/config/gibson.xml`, and add this line to
`~/.xscreensaver` (create it with `xscreensaver-demo` first if needed):

```text
programs: gibson -root
```

Full steps and the settings-dialog note are in
[platform/linux/README.md](platform/linux/README.md). xscreensaver is X11
only: on a Wayland session use `swayidle` plus `gibson-app --fullscreen`
instead (see the same file for the exact command).

### Desktop app (any platform)

```bash
cargo run --release -p gibson-app
```

A windowed flythrough opens; press Esc or Q to quit. Useful flags (see
`--help` for the full list):

```text
--fullscreen                          borderless fullscreen
--snapshot out.png --size 1920x1080   render one offscreen still and exit
        --time 12                     (seconds of simulated flight for the still;
                                       deterministic for a fixed --seed)
--speed 0.8                           fly speed
--bank 0.9                            banking strength
--palette normal|siege|cycle          color treatment
--grid 40                             city size (grid x grid towers)
--pulses 200                          lane pulse streaks
--seed 12345                          reproducible city and flight (0 = time-derived)
--render-scale 0.5                    internal resolution multiplier
--no-bloom --no-motion-blur --no-crt  disable individual effects
--config /path/to/gibson.toml         alternate settings file
```

**Platform status, stated plainly:** the macOS saver, the desktop app, and
the web build are the runtime-tested paths. The Windows `.scr` and Linux
xscreensaver hosts are **compile-verified in CI only — nobody has run them
yet**; they are documented as untested in their platform READMEs and should
be treated accordingly.

## Settings

One settings struct drives every host. All values are clamped to legal
ranges on load, so a bad config file, CLI value, or query parameter can
never put the renderer out of bounds.

| Setting | Default | Range | Effect |
| --- | --- | --- | --- |
| `fly_speed` | 0.55 | 0.05–3 | Speed along the flight path (segments per second) |
| `bank_strength` | 0.45 | −3–3 | How hard the camera banks into turns |
| `bank_max_degrees` | 32 | 0–60 | Maximum bank angle, degrees |
| `bank_smoothing` | 0.55 | 0.05–2 | Bank low-pass time constant, seconds |
| `palette` | `normal` | `normal`, `siege`, `cycle` | Color treatment (blues / attack oranges / timed cycling) |
| `palette_cycle_seconds` | 240 | 10–3600 | Seconds between switches in `cycle` mode |
| `bloom` | 0.35 | 0–2 | Bloom intensity; 0 disables bloom |
| `motion_blur` | 0.5 | 0–1 | Motion-blur strength; 0 disables |
| `grain` | 0.03 | 0–0.2 | Film-grain amount; 0 disables |
| `crt` | 0.35 | 0–1 | CRT-overlay strength (scanlines, aperture grille, curvature, phosphor bloom, edge vignette); 0 disables |
| `render_scale` | 1.0 | 0.25–1 | Internal resolution multiplier; lower is cheaper on slow GPUs |
| `grid` | 60 | 8–120 | City size: `grid x grid` towers |
| `pulses` | 700 | 0–2000 | Number of pulse streaks down the lanes |
| `seed` | 0 | any 64-bit integer | City, atlas, and floor seed; 0 derives one from the clock |
| `preview` | false | true / false | Screensaver-preview mode; hosts set this themselves |

Where each host stores or accepts them:

- **macOS Options sheet** (System Settings > Wallpaper > Screen Saver >
  Options…, or right-click the preview): sliders **Fly speed** (0.2–1.2),
  **Banking** (0–1), and **CRT overlay** (0–1); a **Palette** popup with
  Normal / Siege / Cycle; and **Bloom glow**, **Motion blur**, and
  **Film grain** checkboxes (unchecked disables the effect). Changes apply
  on the next activation.
- **`gibson.toml`**: created automatically — with every key, its default,
  and a comment — the first time a desktop host or `--snapshot` run starts,
  at `<config-dir>/hack-the-gibson/gibson.toml`, where `<config-dir>` is
  `~/Library/Application Support` on macOS, `~/.config` on Linux, and
  `%APPDATA%` on Windows. The Windows `/c` mode also ensures the file
  exists and opens it in your editor. Unknown keys are ignored; missing
  keys fall back to defaults.
- **Desktop CLI**: `--speed`, `--bank`, `--palette`, `--grid`, `--pulses`,
  `--grain`, `--crt`, `--seed`, `--render-scale`, and the
  `--no-bloom` / `--no-motion-blur` / `--no-crt` switches, applied on top of
  the config file.
- **Web**: the same names as query parameters on the demo URL, except
  `fly_speed` is `speed`, `bank_strength` is `bank`, and `render_scale` is
  `scale`:

  ```text
  https://paulkiernan.github.io/hack-the-gibson/?palette=cycle&grid=40&pulses=200
  ```

  Supported: `speed`, `bank`, `palette`, `grid`, `pulses`, `seed`, `scale`,
  `bloom`, `motionblur`, `grain`, `crt`.

Precedence everywhere is: built-in defaults < `gibson.toml` < command-line
or query overrides.

## Building

Prerequisites:

- Rust stable (the workspace pins `stable` in `rust-toolchain.toml`)
- For the web build: `rustup target add wasm32-unknown-unknown` and
  `cargo install wasm-pack --locked`
- For a universal macOS saver (arm64 + x86_64 slices):
  `rustup target add aarch64-apple-darwin x86_64-apple-darwin`
  (the saver Makefile defaults to both; build just your arch with
  `make -C platform/macos ARCHS=arm64`)

Root Makefile targets (each delegates to cargo or a platform Makefile):

| Target | What it does |
| --- | --- |
| `make app` | Run the windowed desktop app |
| `make snapshot` | Render `docs/screenshots/lane.png` (1920x1080, t=12 s) |
| `make web` | `wasm-pack build` of `crates/gibson-web` into `web/pkg` |
| `make saver` | Build `platform/macos/build/Gibson.saver` |
| `make install-saver` / `make uninstall-saver` | Install / remove the saver |
| `make test` | `cargo test --workspace --exclude gibson-web` |
| `make clean` | `cargo clean` |

Notable: the macOS saver builds with **Command Line Tools only — no Xcode
required**. It is a Makefile that compiles the Rust core to a staticlib,
links it into a Swift dylib with `swiftc`, `lipo`s the architectures
together, wraps the result in a `.saver` bundle, and ad-hoc codesigns it
(recipe proven by the PerfectoWeb/Gibson saver). CI builds every target —
macOS (tests + saver), Windows (tests + `.scr`), Linux (tests + xscreensaver
host), and wasm (web bundle) — and uploads the artifacts.

## How it works

The workspace is a set of small crates with one contract crate,
`gibson-types`, that everything compiles against:

- **Deterministic procedural content.** A 64-bit `seed` reproduces the same
  city, text, and floor exactly. The text atlas (`gibson-atlas`) generates
  64 layers of 256x768 RGBA: 32 tower-face panels, each with two text
  variants that share byte-identical block geometry. Panels 0–28 are dense
  mosaics of mono-text blocks — hex dumps, numeric columns, keyword rows,
  bar-chart glyphs, framed and inverse-video blocks — in IBM Plex Mono;
  panels 28–32 are hero directory lists (Michroma, the Eurostile-Extended-
  style face) whose entries double as individually highlightable blocks. The
  floor (`gibson-floor`) generates a toroidal 96x96-cell PCB tile: random-
  walk Manhattan traces with pads, vias, and chips that wrap seamlessly.
- **The city.** A `grid x grid` array of towers standing in the lanes of the
  2015 world grid. Towers are instanced translucent glass boxes 12 units
  wide and 44–110 tall, drawn double-sided so back-face text bleeds through
  the body. In the shader, each text block clears and redraws top-down on
  its own cycle, alternating between the two text variants; occasionally a
  block on a nearby tower lights up in the palette's highlight color, and
  pulse streaks run down the lanes between towers.
- **The camera.** The closed-loop flight path was rescued from the 2015 C++
  waypoints (z-negated for a right-handed Y-up world), and the banking
  algorithm — a low-passed yaw rate driving a smoothed roll — was ported
  from the SceneKit fork that this project grew out of.
- **The frame graph.** The scene renders at `render_scale` into an HDR
  (16-bit float) target: floor, towers, pulses, then a bloom prefilter with
  a downsample/upsample chain, motion blur by depth reprojection against the
  previous frame, and a final composite applying ACES tonemapping, chromatic
  aberration, film grain, and vignette — plus, when `crt > 0`, a CRT
  treatment (scanlines, aperture grille, screen curvature, phosphor smear)
  as the last step. Distant towers fade through a blue haze to black, and
  the whole look is graded against the film reference in
  [docs/film-reference.md](docs/film-reference.md).

## Credits

- **John Serafino** — the 2015 Irrlicht original this is a rewrite of.
- **dherberger** — the 2026 macOS SceneKit/Metal `.saver` fork that proved
  the screensaver path and whose banking code and grid conventions carried
  over; the fork was merged back upstream to become this project.
- The film crew whose work is being reproduced: **Peter Chiang** (VFX
  supervisor), **Tim Field** (VFX producer), and **Neville Brody** (type
  design). The film's towers were built as clear perspex prisms with printed
  text cels, shot on motion control at Pinewood, and designed after Muriel
  Cooper's MIT "Information Landscapes".

## License

GPL-3.0-or-later (see [GPL.txt](GPL.txt)). The bundled fonts — Michroma
Regular and IBM Plex Mono Medium — are SIL Open Font License; their license
text ships in [assets/fonts/OFL-Michroma.txt](assets/fonts/OFL-Michroma.txt)
and [assets/fonts/OFL-IBMPlexMono.txt](assets/fonts/OFL-IBMPlexMono.txt).
