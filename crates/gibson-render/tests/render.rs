//! Acceptance tests for the gibson-render frame chain.
//!
//! Each test creates a real `Renderer` on the default adapter with generated atlas + floor
//! content and renders offscreen frames, then inspects the returned sRGB8 pixels. Every test
//! prints a skip message (and passes) when no adapter/device is available, e.g. on CI runners
//! without a GPU.

use gibson_types::*;
use gibson_render::{RenderError, Renderer};
use std::f32::consts::PI;

fn settings() -> Settings {
    Settings {
        // Deterministic, effects off for the pixel-exact tests; individual tests re-enable.
        bloom: 0.0,
        motion_blur: 0.0,
        grain: 0.0,
        crt: 0.0,
        seed: 7,
        grid: 60,
        ..Settings::default()
    }
}

fn atlas() -> AtlasImage {
    gibson_atlas::generate(7)
}

fn floor() -> FloorMap {
    gibson_floor::generate(7)
}

/// Build a renderer offscreen; `None` (with a printed note) when the machine has no adapter.
fn renderer_at(
    width: u32,
    height: u32,
    scale: f32,
    settings: &Settings,
) -> Option<Renderer> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let a = atlas();
    let f = floor();
    match pollster::block_on(Renderer::new(
        &instance,
        None,
        width,
        height,
        scale,
        &a,
        &f,
        settings,
    )) {
        Ok(r) => Some(r),
        Err(RenderError::NoAdapter | RenderError::NoDevice(_)) => {
            eprintln!("skipped: no graphics adapter/device available");
            None
        }
        Err(e) => {
            panic!("renderer creation failed: {e}");
        }
    }
}

fn camera(pos: [f32; 3], forward: [f32; 3], up: [f32; 3]) -> CameraPose {
    CameraPose {
        position: pos,
        forward,
        up,
        fov_y_radians: 58.0 * PI / 180.0,
    }
}

fn empty_frame<'a>(
    time: f64,
    pose: CameraPose,
    s: &'a Settings,
    towers: &'a [TowerInstance],
    pulses: &'a [PulseInstance],
) -> FrameData<'a> {
    FrameData {
        time,
        camera: pose,
        prev_camera: pose,
        palette: Palette::NORMAL,
        towers,
        pulses,
        settings: s,
    }
}

fn luminance(rgba: &[u8]) -> f64 {
    let n = rgba.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let mut sum = 0u64;
    for px in rgba.chunks_exact(4) {
        sum += (px[0] as u64 * 77 + px[1] as u64 * 150 + px[2] as u64 * 29) >> 8;
    }
    sum as f64 / n as f64
}

fn nonzero_count(rgba: &[u8]) -> usize {
    rgba.chunks_exact(4)
        .filter(|px| px[0] != 0 || px[1] != 0 || px[2] != 0)
        .count()
}

/// Central 25% region statistics.
fn central_25(rgba: &[u8], w: usize, h: usize) -> (usize, usize) {
    let x0 = w / 4;
    let x1 = 3 * w / 4;
    let y0 = h / 4;
    let y1 = 3 * h / 4;
    let mut nz = 0;
    let mut total = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 4;
            if rgba[i] != 0 || rgba[i + 1] != 0 || rgba[i + 2] != 0 {
                nz += 1;
            }
            total += 1;
        }
    }
    (nz, total)
}

#[test]
fn pipelines_compile_offscreen() {
    // The pipeline-creation gate: any WGSL validation error fails Renderer::new.
    let s = settings();
    let r = renderer_at(512, 384, 1.0, &s);
    if r.is_none() {
        return;
    }
}

#[test]
fn empty_frame_is_pure_black() {
    let s = settings();
    let mut r = match renderer_at(320, 240, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    // Camera high above the floor, looking at the sky: no floor, no towers, no pulses.
    let pose = camera([0.0, 400.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
    let frame = empty_frame(1.0, pose, &s, &[], &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("offscreen render");
    assert_eq!((w, h), (320, 240));
    assert_eq!(rgba.len(), 320 * 240 * 4);
    assert!(
        rgba.chunks_exact(4).all(|px| px[0] == 0 && px[1] == 0 && px[2] == 0),
        "empty frame must be pure black (all effects disabled)"
    );
    // Composite is opaque: alpha must be saturated.
    assert!(rgba.chunks_exact(4).all(|px| px[3] == 255));
}

#[test]
fn tower_in_front_produces_pixels() {
    let s = settings();
    let a = atlas();
    if a.blocks_per_panel[0].is_empty() {
        eprintln!("skipped: atlas has no blocks in panel 0");
        return;
    }
    let mut r = match renderer_at(320, 240, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    // Empty reference frame.
    let pose = camera([0.0, 20.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let empty = empty_frame(1.0, pose, &s, &[], &[]);
    let (_, _, empty_rgba) = r.render_to_rgba(&empty).expect("empty render");
    let empty_mean = luminance(&empty_rgba);

    // One tower 40 units straight ahead.
    let tower = TowerInstance {
        position: [0.0, 0.0, -40.0],
        anim_phase: 0.25,
        face_layers: [0, 1, 2, 3],
        top_layer: 4,
        highlight_block: 0,
        highlight_t: 0.0,
        height: TOWER_HEIGHT,
    };
    let frame = empty_frame(1.0, pose, &s, std::slice::from_ref(&tower), &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("tower render");
    let (nz, total) = central_25(&rgba, w as usize, h as usize);
    assert!(
        nz > total / 10,
        "tower at 40 units must cover the central 25% (nonzero {nz}/{total})"
    );
    assert!(
        luminance(&rgba) > empty_mean + 1.0,
        "tower frame mean ({}) must exceed empty frame mean ({})",
        luminance(&rgba),
        empty_mean
    );
}

#[test]
fn floor_from_above_draws_traces() {
    let s = settings();
    let f = floor();
    let trace_cells = f.data.iter().filter(|c| c[0] != 0).count();
    assert!(trace_cells > 0, "generated floor map has no trace cells");
    let mut r = match renderer_at(320, 240, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    // 200 units above the floor looking straight down: a ~200-tan(29 deg) radius of traces.
    let pose = camera([120.0, 200.0, 120.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
    let frame = empty_frame(1.0, pose, &s, &[], &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("floor render");
    let nz = nonzero_count(&rgba);
    assert!(
        nz > 100,
        "floor traces must be visible from above (nonzero {nz} in {}x{h})",
        w
    );
}

#[test]
fn effect_toggles_and_odd_size_render() {
    // Non-power-of-two target with every effect disabled: the chain still runs and the
    // readback buffer is tightly packed.
    let s = settings();
    let mut r = match renderer_at(1017, 613, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let pose = camera([0.0, 20.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let tower = TowerInstance {
        position: [0.0, 0.0, -60.0],
        anim_phase: 0.5,
        face_layers: [0, 1, 2, 3],
        top_layer: 4,
        highlight_block: 0,
        highlight_t: 0.0,
        height: TOWER_HEIGHT,
    };
    let frame = empty_frame(1.0, pose, &s, std::slice::from_ref(&tower), &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("odd-size render");
    assert_eq!((w, h), (1017, 613));
    assert_eq!(rgba.len(), 1017 * 613 * 4);
}

#[test]
fn bloom_motion_grain_on_render_succeeds() {
    // The full-effects path (bloom + motion blur + grain) renders and is brighter than the
    // effects-off frame of the same scene.
    let mut s = settings();
    let mut r = match renderer_at(320, 240, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let pose = camera([0.0, 20.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let tower = TowerInstance {
        position: [0.0, 0.0, -60.0],
        anim_phase: 0.5,
        face_layers: [0, 1, 2, 3],
        top_layer: 4,
        highlight_block: 0,
        highlight_t: 0.0,
        height: TOWER_HEIGHT,
    };
    let off_frame = empty_frame(1.0, pose, &s, std::slice::from_ref(&tower), &[]);
    let (_, _, off) = r.render_to_rgba(&off_frame).expect("effects off");
    let off_mean = luminance(&off);

    s.bloom = 0.45;
    s.motion_blur = 0.5;
    s.grain = 0.04;
    let pose2 = camera([0.0, 20.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let on_frame = empty_frame(1.0, pose2, &s, std::slice::from_ref(&tower), &[]);
    let (_, _, on) = r.render_to_rgba(&on_frame).expect("effects on");
    assert!(luminance(&on) >= off_mean, "effects must not darken the frame");
}

/// Deterministic 0..1 hash over grid indices.
fn hash01(a: i32, b: i32) -> f32 {
    let mut h = (a as u32).wrapping_mul(0x9E37_79B1) ^ (b as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 16;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Film canyon height: most towers reach near TOWER_HEIGHT, a quarter are visibly shorter for
/// skyline variety (band counts 1..3 exercised side by side).
fn tower_height(i: i32, j: i32) -> f32 {
    let u = hash01(i, j);
    if u < 0.75 {
        TOWER_HEIGHT * (0.96 + 0.04 * hash01(i + 13, j))
    } else {
        TOWER_HEIGHT_MIN + 44.0 * hash01(i, j + 7)
    }
}

/// One of a handful of HDR beam hues; every 5th beam is bright green.
fn beam_color(n: i32) -> [f32; 3] {
    const HUES: [[f32; 3]; 5] = [
        [0.55 * 3.0, 1.0 * 3.0, 1.0 * 3.0], // cool white-cyan (film pulses)
        [0.30 * 3.0, 1.0 * 3.0, 0.32 * 3.0], // bright green
        [1.0 * 3.0, 0.40 * 3.0, 0.35 * 3.0], // amber
        [0.60 * 3.0, 1.6 * 3.0, 1.0 * 3.0], // pale cyan
        [1.0 * 3.0, 0.45 * 3.0, 1.0 * 3.0], // magenta
    ];
    HUES[(n.rem_euclid(5)) as usize]
}

/// Build a populated city frame (towers on the documented grid + lane pulses).
fn populated_frame() -> (Vec<TowerInstance>, Vec<PulseInstance>, CameraPose) {
    let grid = 60i32;
    let half = grid / 2;
    // Tower rows along the visible corridor: all towers within 260 units of the lane.
    let mut towers = Vec::new();
    for i in -half..half {
        for j in -half..half {
            let x = (i as f32) * 30.0 + 15.0;
            let z = (j as f32) * 30.0;
            // Corridor along the lane the camera flies: towers ahead of it (z < 140) within
            // +/-260 laterally, out to the fog end. Near towers flank the camera like canyon
            // walls, faces toward the lane.
            if x.abs() > 260.0 || z >= 140.0 || z < -820.0 {
                continue;
            }
            let panel = |k: i32| {
                // Bias toward directory hero panels for readability on the visible face.
                if k % 5 == 0 {
                    (28 + ((i * 3 + j) & 3)) as u32
                } else {
                    ((k * 7 + i * 13 + j * 5) & 31) as u32
                }
            };
            towers.push(TowerInstance {
                position: [x, 0.0, z],
                anim_phase: ((i * 31 + j * 17) as f32).abs() * 0.001,
                face_layers: [
                    panel(i * 4 + j),
                    panel(i * 4 + j + 1),
                    panel(i * 4 + j + 2),
                    panel(i * 4 + j + 3),
                ],
                top_layer: panel(i * 9 + j * 3),
                highlight_block: 0,
                highlight_t: 0.0,
                height: tower_height(i, j),
            });
        }
    }
    // Laser beams running along the x-lanes at z = 15 + 30k, at ground or altitude, each with
    // its own HDR color (bright green among them) and a head-heavy profile.
    let mut pulses = Vec::new();
    let mut n = 0i32;
    for k in 0..10 {
        let z = 15.0 + 30.0 * k as f32;
        for x in (0..900).step_by(150) {
            // Ground pulses low over the lane; a quarter of them fly high (up to ~90).
            let y = if k % 4 == 0 {
                30.0 + 45.0 * hash01(k, x)
            } else {
                1.2 + 0.4 * (k as f32 * 0.37).fract()
            };
            let len = 14.0 + 30.0 * hash01(k, x);
            pulses.push(PulseInstance {
                position: [x as f32, y, z],
                length: len,
                direction: if k % 2 == 0 { [1.0, 0.0, 0.0] } else { [-1.0, 0.0, 0.0] },
                intensity: 0.85 + 0.15 * hash01(k, x),
                color: beam_color(n),
                _pad: 0.0,
            });
            n += 1;
        }
    }
    // Back-to-front along -z for correct translucency ordering.
    towers.sort_by(|a, b| b.position[2].partial_cmp(&a.position[2]).unwrap());
    // Low along the lane: canyon walls on both sides rise past the top of the frame, floor
    // traces and beams fill the bottom.
    let pose = camera([0.0, 6.0, 140.0], [0.0, -0.03, -1.0], [0.0, 1.0, 0.0]);
    (towers, pulses, pose)
}

/// Tallest lit row (smallest y) inside a horizontal pixel band, else `h`.
fn topmost_row(rgba: &[u8], w: usize, h: usize, x0: usize, x1: usize) -> usize {
    for y in 0..h {
        for x in x0..x1 {
            let i = (y * w + x) * 4;
            if rgba[i] != 0 || rgba[i + 1] != 0 || rgba[i + 2] != 0 {
                return y;
            }
        }
    }
    h
}

/// Count of nonzero pixels inside a band.
fn lit_in_band(rgba: &[u8], w: usize, h: usize, x0: usize, x1: usize) -> usize {
    let mut n = 0;
    for y in 0..h {
        for x in x0..x1 {
            let i = (y * w + x) * 4;
            if rgba[i] != 0 || rgba[i + 1] != 0 || rgba[i + 2] != 0 {
                n += 1;
            }
        }
    }
    n
}

/// Mean absolute difference of rgb bytes between two frames.
fn mad(a: &[u8], b: &[u8]) -> f64 {
    let n = a.len() / 4;
    let mut sum = 0u64;
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        sum += pa[0].abs_diff(pb[0]) as u64;
        sum += pa[1].abs_diff(pb[1]) as u64;
        sum += pa[2].abs_diff(pb[2]) as u64;
    }
    sum as f64 / (n as f64 * 3.0)
}

/// Acceptance 1: per-instance height drives the drawn tower. A 44-unit and a 110-unit tower in
/// one frame both produce pixels, and the taller tower's top reaches higher on screen.
#[test]
fn tower_height_controls_extent() {
    let s = settings();
    let mut r = match renderer_at(640, 480, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let pose = camera([0.0, 20.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let mk = |x: f32, height: f32| TowerInstance {
        position: [x, 0.0, -60.0],
        anim_phase: 0.25,
        face_layers: [0, 1, 2, 3],
        top_layer: 4,
        highlight_block: 0,
        highlight_t: 0.0,
        height,
    };
    let towers = [mk(-24.0, TOWER_HEIGHT_MIN), mk(24.0, TOWER_HEIGHT)];
    let frame = empty_frame(1.0, pose, &s, &towers, &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("height render");
    let wu = w as usize;
    let hu = h as usize;
    // Left half holds the short tower (center x = 320 - 24*scale), right half the tall one.
    let short_x0 = (wu as f32 * 0.22) as usize;
    let short_x1 = (wu as f32 * 0.38) as usize;
    let tall_x0 = (wu as f32 * 0.62) as usize;
    let tall_x1 = (wu as f32 * 0.78) as usize;
    let short_lit = lit_in_band(&rgba, wu, hu, short_x0, short_x1);
    let tall_lit = lit_in_band(&rgba, wu, hu, tall_x0, tall_x1);
    assert!(
        short_lit > (hu * (tall_x1 - tall_x0)) / 40,
        "short tower must produce pixels (lit {short_lit})"
    );
    assert!(
        tall_lit > (hu * (tall_x1 - tall_x0)) / 40,
        "tall tower must produce pixels (lit {tall_lit})"
    );
    let short_top = topmost_row(&rgba, wu, hu, short_x0, short_x1);
    let tall_top = topmost_row(&rgba, wu, hu, tall_x0, tall_x1);
    assert!(
        tall_top < short_top,
        "taller tower must cover more vertical extent (tall top row {tall_top} >= short top row {short_top})"
    );
    assert!(
        short_top > 20,
        "short tower top must not reach the frame edge (top row {short_top})"
    );
    eprintln!("height test: short 44u top row {short_top}, tall 110u top row {tall_top}");
}

/// Acceptance 2: the CRT overlay renders at 0 / 0.35 / 1, crt = 0 is byte-identical to a
/// second crt = 0 render (the composite code path is skipped entirely), and crt = 1 differs
/// measurably with crt = 0.35 in between.
#[test]
fn crt_overlay_scales_with_amount() {
    let mut s = settings();
    let mut r = match renderer_at(640, 360, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let pose = camera([0.0, 14.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let tower = TowerInstance {
        position: [0.0, 0.0, -50.0],
        anim_phase: 0.5,
        face_layers: [0, 1, 2, 3],
        top_layer: 4,
        highlight_block: 0,
        highlight_t: 0.0,
        height: TOWER_HEIGHT,
    };
    let pulse = PulseInstance {
        position: [12.0, 6.0, -60.0],
        length: 30.0,
        direction: [1.0, 0.0, 0.0],
        intensity: 1.0,
        color: [0.6, 2.4, 2.6],
        _pad: 0.0,
    };
    let mut render_at = |r: &mut Renderer, crt: f32| -> Vec<u8> {
        s.crt = crt;
        let frame = empty_frame(1.0, pose, &s, std::slice::from_ref(&tower), std::slice::from_ref(&pulse));
        let (_, _, px) = r.render_to_rgba(&frame).expect("crt render");
        px
    };
    let b0 = render_at(&mut r, 0.0);
    let b0b = render_at(&mut r, 0.0);
    assert_eq!(b0, b0b, "crt = 0 renders must be byte-identical");
    let b35 = render_at(&mut r, 0.35);
    let b1 = render_at(&mut r, 1.0);
    let m01 = mad(&b0, &b1);
    let m035 = mad(&b0, &b35);
    assert!(
        m01 > 0.4,
        "crt = 1 must differ measurably from crt = 0 (mean abs diff {m01:.3})"
    );
    assert!(
        m035 > 0.0 && m035 < m01,
        "crt = 0.35 must sit between 0 and 1 (mad 0.35 = {m035:.3}, mad 1.0 = {m01:.3})"
    );
    eprintln!("crt overlay: mad(0, 0.35) = {m035:.3}, mad(0, 1.0) = {m01:.3} (rgb bytes 0..255)");
}

/// Acceptance 3: a beam's per-instance color reaches the screen - a bright-green beam's pixels
/// have a green channel that clearly dominates red and blue.
#[test]
fn beam_color_reaches_the_screen() {
    let s = settings();
    let mut r = match renderer_at(320, 240, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    // Camera level down a lane; one bright green beam crosses the view ahead, above the camera
    // so it renders against pure sky (the floor only starts below the horizon row).
    let pose = camera([0.0, 6.0, 140.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let pulse = PulseInstance {
        position: [-8.0, 12.0, 60.0],
        length: 36.0,
        direction: [1.0, 0.0, 0.0],
        intensity: 1.0,
        color: [0.15 * 3.0, 1.0 * 3.0, 0.15 * 3.0],
        _pad: 0.0,
    };
    let frame = empty_frame(1.0, pose, &s, &[], std::slice::from_ref(&pulse));
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("beam render");
    let wu = w as usize;
    let hu = h as usize;
    // Region the beam crosses: sky band above the horizon (rows < 50%), mid-left.
    let (x0, x1, y0, y1) = (wu / 12, wu * 11 / 24, hu * 2 / 8, hu * 15 / 32);
    let (mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * wu + x) * 4;
            // Only pixels the beam actually lit (exclude the black substrate).
            if rgba[i + 1] > 20 {
                sr += rgba[i] as u64;
                sg += rgba[i + 1] as u64;
                sb += rgba[i + 2] as u64;
                n += 1;
            }
        }
    }
    assert!(n > 40, "green beam must produce lit pixels (found {n})");
    let (mr, mg, mb) = (sr as f64 / n as f64, sg as f64 / n as f64, sb as f64 / n as f64);
    assert!(
        mg > 2.0 * mr && mg > 2.0 * mb,
        "green beam pixels must be green-dominant (mean r {mr:.1} g {mg:.1} b {mb:.1})"
    );
    eprintln!("beam color: mean over {n} lit pixels r {mr:.1} g {mg:.1} b {mb:.1}");
}

/// Write `docs/scratch/floor-overhead.png` (1280x720, straight-down floor view, CRT off) to
/// inspect the circuit-board trace look. Gated behind `GIBSON_RENDER_PROBE=1`.
#[test]
fn probe_floor_overhead() {
    if std::env::var("GIBSON_RENDER_PROBE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set GIBSON_RENDER_PROBE=1 to render the floor probe image");
        return;
    }
    let s = Settings {
        bloom: 0.35,
        crt: 0.0,
        seed: 7,
        grid: 60,
        ..Settings::default()
    };
    let mut r = match renderer_at(1280, 720, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    // 90 units up, straight down: a wide patch of floor with tower keep-out zones.
    let pose = camera([120.0, 90.0, 120.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
    let frame = empty_frame(1.0, pose, &s, &[], &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("floor probe render");
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = manifest.join("../../docs/scratch/floor-overhead.png");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create docs/scratch");
    }
    write_png(&out, w, h, &rgba);
    eprintln!("wrote {}", out.display());
}

/// Write `docs/scratch/face-study.png` (1920x1080): three towers of heights 44 / 110 / 76
/// (1 / 3 / 2 bands) side by side, faces perpendicular to the camera, to inspect vertical-band
/// texturing (constant text size, seams, no vertical repetition). Gated like the other probes.
#[test]
fn probe_face_study() {
    if std::env::var("GIBSON_RENDER_PROBE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set GIBSON_RENDER_PROBE=1 to render the face-study image");
        return;
    }
    let s = Settings {
        bloom: 0.45,
        motion_blur: 0.0,
        grain: 0.02,
        crt: 0.0,
        seed: 7,
        grid: 60,
        ..Settings::default()
    };
    let mut r = match renderer_at(1920, 1080, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let heights = [44.0f32, TOWER_HEIGHT, 76.0];
    let xs = [45.0f32, 105.0, 165.0];
    let mut towers = Vec::new();
    for k in 0..3 {
        towers.push(TowerInstance {
            position: [xs[k], 0.0, 0.0],
            anim_phase: 0.1 + 0.2 * k as f32,
            face_layers: [2, 3, 0, 1],
            top_layer: 4,
            highlight_block: 0,
            highlight_t: 0.0,
            height: heights[k],
        });
    }
    let pose = camera([105.0, 55.0, 100.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let frame = empty_frame(1.0, pose, &s, &towers, &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("face study render");
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = manifest.join("../../docs/scratch/face-study.png");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create docs/scratch");
    }
    write_png(&out, w, h, &rgba);
    eprintln!("wrote {}", out.display());
}

fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create probe png");
    let buf = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(buf, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().expect("png header");
    writer.write_image_data(rgba).expect("png data");
}

/// Write `docs/scratch/render-tall.png` (1920x1080, populated tall city, CRT on) for visual
/// inspection, and print per-frame wall time (includes the readback; upper bound on GPU time).
/// Gated behind `GIBSON_RENDER_PROBE=1` so the default test run stays fast.
#[test]
fn probe_populated_frame() {
    if std::env::var("GIBSON_RENDER_PROBE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set GIBSON_RENDER_PROBE=1 to render the 1080p probe image");
        return;
    }
    let crt: f32 = std::env::var("GIBSON_RENDER_CRT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.35);
    let s = Settings {
        bloom: 0.45,
        motion_blur: 0.35,
        grain: 0.04,
        crt,
        seed: 7,
        grid: 60,
        pulses: 200,
        ..Settings::default()
    };
    let mut r = match renderer_at(1920, 1080, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let (towers, pulses, pose) = populated_frame();
    let frame = FrameData {
        time: 1.5,
        camera: pose,
        prev_camera: pose,
        palette: Palette::NORMAL,
        towers: &towers,
        pulses: &pulses,
        settings: &s,
    };
    // Warm up once (allocations, shader first-touch), then time a few full frames.
    let (w, h, _) = r.render_to_rgba(&frame).expect("probe render");
    assert_eq!((w, h), (1920, 1080));
    let t0 = std::time::Instant::now();
    let mut rgba = Vec::new();
    for _ in 0..4 {
        let (_, _, px) = r.render_to_rgba(&frame).expect("probe render");
        rgba = px;
    }
    let per_frame = t0.elapsed().as_secs_f64() / 4.0 * 1000.0;
    eprintln!("render-tall: {per_frame:.2} ms/frame at 1920x1080 (wall, incl. readback)");

    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = std::env::var("GIBSON_RENDER_PROBE_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../../docs/scratch/render-tall.png"));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create docs/scratch");
    }
    write_png(&out, w, h, &rgba);
    eprintln!("wrote {}", out.display());
}
