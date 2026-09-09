# Film reference — the Gibson

This document is the **visual acceptance standard** for the Gibson tower-city
flythrough. Every renderer decision — tower geometry and glass, text block
layout, palettes, fog, floor traces, pulses, bloom, grain — is graded against
what is described and linked here. It is not a cleanup or progress document;
it is the project's record of what the 1995 film actually shows, gathered from
directly observed frames and from interviews with the crew who built the
sequences.

Sources: Peter Chiang (VFX supervisor) and Tim Field (VFX producer)
interviews at hackerscurator.com; frame captures at scifiinterfaces.com; and
photographs of Chiang's surviving perspex tower on hackerscurator.com. The
film frames themselves are copyrighted and are **not** committed to this
repository — see [How to compare](#how-to-compare).

## Production history

- The "Gibson" (the internal network the heroes fly through, named after
  William Gibson) was produced at Pinewood. Towers were designed after Muriel
  Cooper's MIT "Information Landscapes" visualizations.
- Two physical forms were made for the shoot: full **CG** towers, and clear
  **perspex (acrylic) square-section prisms** with printed text cels,
  photographed with motion control. "Only the blue pulses were CG." (Chiang)
- Neville Brody's studio designed all of the on-screen type.
- Interviews: Peter Chiang was the VFX supervisor; Tim Field the VFX producer
  (scheduling/budgeting of the visual effects).

## Observed towers

- Tall square prisms, roughly **3–5 : 1 height:width**, **translucent tinted
  glass**. The working model uses a 12 × 38 × 12-unit box (≈3.2 : 1).
- Text appears on **all faces and is visible through the body** — back-face
  text bleeds through the translucent glass, so faces must render double-sided
  with the body at partial opacity.
- Face text is a **mosaic of variable-size rectangular blocks** of dense tiny
  characters: hex dumps, numeric columns, short uppercase words, small
  bar-chart glyphs, some blocks framed with 1-px borders, some inverse-video.
- On "hero" towers, large **right-aligned uppercase directory names** in a
  wide **Eurostile-Extended-like** face (rendered with Michroma Regular), each
  followed by a **triangle marker** ("▸"), with a small hex block below the
  list.
- Lists clear and redraw **top-down**, alternate between two text sets, and
  get sweeping highlight bars. Selected blocks light up (magenta in NORMAL
  mode).

### Directory names observed on hero towers

`SHIPPING FORCASTS`, `ACCOUNTANTS`, `GARBAGE`, `KINEMATICS`, `SEA-BOARD LAWS`,
`COMPANY STATUS`, `COMPOSITE PLANTS`, `TPGC. REPORTS`, `WRHSE. EXPEND.`,
`ANNUAL BUDGETS`, `COMP. OPERATIONS` (spelling "FORCASTS" is as filmed).

The atlas generator draws from a longer in-universe list for variety, adding:
`SHIPPING ROUTINGS`, `TPGC. EXPEND.`, `WRHSE. LOCATION`, `PAYROLL`,
`PERSONNEL`, `TANKER ROUTES`, `OIL PLATFORMS`, `BALLAST CTRL`, `HULL STRESS`,
`CARGO MANIFEST`, `LEGAL`, `SECURITY`, `SYS ADMIN`, `BACKUP`, `ROOT`,
`.WORKSPACE`, `.GARBAGE`.

## Palette NORMAL

- Tower body: deep blue **#0E2A6A** at ~35 % opacity, translucent.
- Text: cyan **#48E8FF**, HDR — blooms to white where dense.
- Selected/highlighted block: **magenta #C060FF**.
- Floor: black with violet PCB traces **#5B3FE8**; pads/vias brighter
  **#A48CFF**. Traces are thick, rounded-corner Manhattan routes with pads and
  IC-like rectangles.
- Overhead shots: tower tops read **teal-green #3EE8C8**.
- Distance: towers dissolve into blue haze **#1030A0**, then pure black.
- Background is pure black; everything is emissive — no shadows or lighting.
- Pulses read white-cyan (blooming).

Shader-side HDR constants (base color × multiplier) that back these looks:
tower body `(0.055, 0.165, 0.415, 0.35)`, text `(0.28, 0.91, 1.0) × 1.8`,
highlight `(0.75, 0.38, 1.0) × 2.5`, trace `(0.36, 0.25, 0.91) × 1.6`, pad
`(0.64, 0.55, 1.0) × 1.6`, pulse `(0.85, 1.0, 1.0) × 3.0`, haze
`(0.06, 0.19, 0.63)`.

## Palette SIEGE ("under attack")

- Text blocks: **orange-red #FF6030**.
- Tower body: magenta-pink **#6A1040** (~40 % opacity).
- Floor traces: ice-blue/white **#9FC0FF**; pads/white variant of the same.
- Distant towers: brownish red (haze ≈ `(0.25, 0.06, 0.12)` ≈ **#400F1F**).
- Highlight: warm `(1.0, 0.85, 0.4) ≈ #FFD966`.

Shader-side HDR constants: text `(1.0, 0.38, 0.19) × 1.8`, trace
`(0.62, 0.75, 1.0) × 1.2`, pad `(0.85, 0.9, 1.0) × 1.2`, pulse
`(1.0, 0.9, 0.8) × 3.0`.

## Camera and effects

- Low flights down the lanes between towers with **hard banking (~25–35°)**
  and dives, plus **high overhead sweeps**.
- White-cyan **pulse streaks** run along the ground lanes and between towers.
- Strong **bloom/halation** around text and tower edges; visible **film
  grain**; heavy **motion blur** on fast banked passes (slow shutter on 35 mm
  film).
- Text and towers are emissive; the background is pure black at all times.

## Sources

Interviews and photography:

- HackersCurator crew-interview index:
  <https://hackerscurator.com/pages/interviews.html>
- Peter Chiang (VFX supervisor) interview:
  <https://hackerscurator.com/pages/interviews/peterChiangFinal.html>
- Tim Field (VFX producer) interview:
  <https://hackerscurator.com/pages/interviews/timFieldFinal.html>
- Chiang's surviving perspex tower (photograph):
  <https://hackerscurator.com/gfx/forInterviews/PeterChiang/PC_tower.gif>

Frame captures (scifiinterfaces.com, "Hackers 3D browsing"):

- Article page: <https://scifiinterfaces.com/2023/12/11/hackers/>
- Captures `Hackers_3D_browsing_0200.png`, `_0400.png`, `_0700.png`,
  `_0800.png`, `_1000.png`, `_1100-1.png` under the image CDN base URL:
  `https://i0.wp.com/scifiinterfaces.com/wp-content/uploads/2023/11/`
  (e.g.
  <https://i0.wp.com/scifiinterfaces.com/wp-content/uploads/2023/11/Hackers_3D_browsing_0200.png>)

Observation mapping used by the visuals work: NORMAL palette and text-mosaic
details are graded against frames 0200/0400/0700; the overhead teal tops
against 0800/1000; SIEGE ("under attack") against 1100. Tower material,
back-face text bleed, and block layout against the perspex-tower photograph
and the Chiang/Field interviews.

## How to compare

Generate stills from the running engine, then eyeball them side by side with
the references above:

```sh
cargo run --release -p gibson-app \
  -- --snapshot docs/screenshots/lane.png --size 1920x1080 --time 12
cargo run --release -p gibson-app \
  -- --snapshot docs/screenshots/overhead.png --size 1920x1080 --time 41
cargo run --release -p gibson-app \
  -- --snapshot docs/screenshots/siege.png --size 1920x1080 --palette siege --time 12
```

`--palette normal|siege|cycle` selects the palette mode; the frame captures
were graded at 1080p, so grade at the same resolution. Checks:

- text blooms without washing out and stays legible;
- floor traces are violet (NORMAL) on black with visible rounded elbows, pads
  and vias, and no hard grid edge at the fog line;
- distant towers fade blue → black (NORMAL), red-brown in SIEGE;
- SIEGE stills show orange-red text with ice-blue traces;
- at least one visible directory-list hero tower with ▸ markers;
- at least one pulse streak in a lane.

The reference film frames and photographs are copyrighted and are therefore
**not committed to the repo**. If you want local copies for comparison, fetch
them into the gitignored `docs/scratch/` directory (add it to `.gitignore`
with `docs/scratch/` if it is not already there) and open them next to the
generated PNGs.
