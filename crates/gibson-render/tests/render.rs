//! Acceptance tests for the gibson-render frame chain.
//!
//! Each test creates a real `Renderer` on the default adapter with generated atlas + floor
//! content and renders offscreen frames, then inspects the returned sRGB8 pixels. Every test
//! prints a skip message (and passes) when no adapter/device is available, e.g. on CI runners
//! without a GPU.

use gibson_render::{RenderError, Renderer, Viewport};
use gibson_types::*;
use std::f32::consts::{FRAC_1_SQRT_2, PI};

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
fn renderer_at(width: u32, height: u32, scale: f32, settings: &Settings) -> Option<Renderer> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let a = atlas();
    let f = floor();
    match pollster::block_on(Renderer::new(
        &instance,
        None,
        Viewport {
            width,
            height,
            scale,
        },
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

/// An sRGB8 buffer as whole pixels: every buffer the renderer returns is `w * h * 4` bytes, so
/// `as_chunks` never has a remainder to report.
fn pixels(rgba: &[u8]) -> &[[u8; 4]] {
    rgba.as_chunks::<4>().0
}

fn luminance(rgba: &[u8]) -> f64 {
    let n = rgba.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let mut sum = 0u64;
    for px in pixels(rgba) {
        sum += (px[0] as u64 * 77 + px[1] as u64 * 150 + px[2] as u64 * 29) >> 8;
    }
    sum as f64 / n as f64
}

fn nonzero_count(rgba: &[u8]) -> usize {
    pixels(rgba)
        .iter()
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
    // No adapter (a headless runner): `renderer_at` has printed its skip note and there is no
    // pipeline set here to compile. Creating the renderer is the whole test, so the value is
    // only kept to prove it was created.
    let _ = renderer_at(512, 384, 1.0, &s);
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
        pixels(&rgba)
            .iter()
            .all(|px| px[0] == 0 && px[1] == 0 && px[2] == 0),
        "empty frame must be pure black (all effects disabled)"
    );
    // Composite is opaque: alpha must be saturated.
    assert!(pixels(&rgba).iter().all(|px| px[3] == 255));
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
        siege_t: 0.0,
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

/// Atlas pixel rect of `block` inside `panel` (variant A): (x0, y0, x1, y1), all inclusive.
fn block_rect(a: &AtlasImage, panel: usize, block: u8) -> Option<(u32, u32, u32, u32)> {
    let w = ATLAS_WIDTH as usize;
    let h = ATLAS_HEIGHT as usize;
    let base = panel * w * h;
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for row in 0..h {
        for col in 0..w {
            if a.rgba[(base + row * w + col) * 4 + 2] == block {
                x0 = x0.min(col as u32);
                y0 = y0.min(row as u32);
                x1 = x1.max(col as u32);
                y1 = y1.max(row as u32);
            }
        }
    }
    if x1 < x0 {
        None
    } else {
        Some((x0, y0, x1, y1))
    }
}

/// Regression test for the panel metadata `textureLoad`: the atlas-array fetch must pass the
/// *panel* as the array index and 0 as the mip level. With the arguments swapped the metadata
/// (block id B, block-local v G) was read from panel 0 for every panel, so a `highlight_block`
/// on any other panel either never fired or lit panel 0's geometry. The test highlights two
/// disjoint blocks of a non-zero panel whose ids do not exist in panel 0: each highlight must
/// change the image, and the two changed-pixel sets must not overlap (a highlight that fired on
/// the wrong panel or wrong block would overlap or vanish).
#[test]
fn highlight_block_lights_only_its_own_panel_block() {
    use std::collections::HashSet;

    let s = settings();
    let a = atlas();
    let panel0: HashSet<u8> = a.blocks_per_panel[0].iter().copied().collect();
    let (panel, id_x, id_y) = 'pick: {
        for p in 1..ATLAS_PANELS as usize {
            let ids: Vec<u8> = a.blocks_per_panel[p]
                .iter()
                .copied()
                .filter(|id| !panel0.contains(id))
                .collect();
            for (i, &x) in ids.iter().enumerate() {
                for &y in &ids[i + 1..] {
                    let rx = block_rect(&a, p, x).unwrap();
                    let ry = block_rect(&a, p, y).unwrap();
                    // Well separated (>24 atlas px) so linear edge sampling cannot bridge them.
                    let separated = rx.2 + 24 < ry.0
                        || ry.2 + 24 < rx.0
                        || rx.3 + 24 < ry.1
                        || ry.3 + 24 < rx.1;
                    let big_enough = rx.2 - rx.0 >= 8 && ry.2 - ry.0 >= 8;
                    if separated && big_enough {
                        break 'pick (p, x, y);
                    }
                }
            }
        }
        eprintln!("skipped: no panel with two panel-0-free disjoint blocks");
        return;
    };
    let mut r = match renderer_at(320, 240, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let pose = camera([0.0, 20.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let tower = |hl: u32| TowerInstance {
        position: [0.0, 0.0, -40.0],
        anim_phase: 0.25,
        face_layers: [panel as u32; 4],
        top_layer: 4,
        highlight_block: hl,
        highlight_t: 1.0,
        height: 38.0, // one band: the +z face maps the whole panel
        siege_t: 0.0,
    };

    let diff = |rgba_a: &[u8], rgba_b: &[u8]| -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (i, (pa, pb)) in pixels(rgba_a).iter().zip(pixels(rgba_b)).enumerate() {
            if (pa[0] as i32 - pb[0] as i32).abs() > 4
                || (pa[1] as i32 - pb[1] as i32).abs() > 4
                || (pa[2] as i32 - pb[2] as i32).abs() > 4
            {
                out.push((i % 320, i / 320));
            }
        }
        out
    };

    let tx = tower(id_x as u32);
    let ty = tower(id_y as u32);
    let t0 = tower(0);
    let base = empty_frame(5.0, pose, &s, std::slice::from_ref(&t0), &[]);
    let hx = empty_frame(5.0, pose, &s, std::slice::from_ref(&tx), &[]);
    let hy = empty_frame(5.0, pose, &s, std::slice::from_ref(&ty), &[]);
    let (_, _, base_rgba) = r.render_to_rgba(&base).expect("render base");
    let (_, _, x_rgba) = r.render_to_rgba(&hx).expect("render highlight x");
    let (_, _, y_rgba) = r.render_to_rgba(&hy).expect("render highlight y");

    let dx = diff(&x_rgba, &base_rgba);
    let dy = diff(&y_rgba, &base_rgba);
    assert!(
        dx.len() >= 30,
        "highlighting block {id_x} of panel {panel} must change the image ({} px changed)",
        dx.len()
    );
    assert!(
        dy.len() >= 30,
        "highlighting block {id_y} of panel {panel} must change the image ({} px changed)",
        dy.len()
    );
    // A highlight must light its own block only: the two changed sets must be disjoint.
    let xs: HashSet<(usize, usize)> = dx.iter().copied().collect();
    let overlap = dy.iter().filter(|p| xs.contains(p)).count();
    assert!(
        overlap == 0,
        "block {id_x} and {id_y} highlights overlap on {} pixels (metadata read from the wrong panel?)",
        overlap
    );
}

#[test]
fn offscreen_renders_do_not_touch_present_counters() {
    // present_stats() counts surface presents and occlusion/busy skips, both of which only
    // happen in render() -- a real windowed surface. Offscreen render_to_rgba frames must
    // leave the counters untouched. (The Occluded/Timeout skip path itself cannot be driven
    // without a real surface, the same limitation as the Outdated/Lost recovery.)
    let s = settings();
    let mut r = match renderer_at(320, 240, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(r.present_stats(), (0, 0));
    let pose = camera([0.0, 400.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
    let frame = empty_frame(1.0, pose, &s, &[], &[]);
    for _ in 0..2 {
        let (_, _, rgba) = r.render_to_rgba(&frame).expect("offscreen render");
        assert_eq!(rgba.len(), 320 * 240 * 4);
    }
    assert_eq!(
        r.present_stats(),
        (0, 0),
        "offscreen renders must neither present nor skip"
    );
}

/// Offscreen frames reuse one target and one staging buffer: the steady state allocates no GPU
/// resources at all. The snapshot host renders one offscreen frame per simulated 1/60 s step,
/// so this is the difference between ~11 MiB allocated (and freed) per frame and none -- and
/// the difference between a flat and a runaway GPU resource census in a long run.
#[test]
fn offscreen_frames_allocate_nothing_after_the_first() {
    let s = settings();
    let mut r = match renderer_at(320, 240, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let pose = camera([0.0, 20.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let frame = empty_frame(1.0, pose, &s, &[], &[]);

    let before_warmup = r.gpu_census();
    let _ = r.render_to_rgba(&frame).expect("first offscreen render");
    let warm = r.gpu_census();
    assert!(
        warm.textures > before_warmup.textures && warm.buffers > before_warmup.buffers,
        "the first offscreen frame must build its own target and staging buffer \
         ({before_warmup} -> {warm})"
    );

    for i in 0..8 {
        let (w, h, rgba) = r
            .render_to_rgba(&frame)
            .expect("steady-state offscreen render");
        assert_eq!((w, h, rgba.len()), (320, 240, 320 * 240 * 4));
        assert_eq!(
            r.gpu_census(),
            warm,
            "offscreen frame {i} changed the GPU resource census ({warm} -> {})",
            r.gpu_census()
        );
    }
}

/// `resize` is idempotent: re-asserting the size a renderer is already at must not rebuild the
/// target set (the HDR targets, the bloom chain, the view bind groups). Hosts assert their size
/// on layout and backing-scale notifications, not only when it changes, and a rebuild there is
/// ~100 MiB of texture churn per call -- indistinguishable in a census from a real leak.
#[test]
fn repeated_identical_resizes_allocate_nothing() {
    let s = settings();
    let mut r = match renderer_at(320, 240, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let before = r.gpu_census();
    for _ in 0..8 {
        r.resize(Viewport {
            width: 320,
            height: 240,
            scale: 1.0,
        });
    }
    assert_eq!(
        r.gpu_census(),
        before,
        "a resize to the current size must not allocate"
    );

    // ...but a real size change must rebuild, and the offscreen output must follow it.
    r.resize(Viewport {
        width: 400,
        height: 300,
        scale: 1.0,
    });
    assert!(
        r.gpu_census().textures > before.textures,
        "a real resize must rebuild the HDR targets"
    );
    let pose = camera([0.0, 20.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let frame = empty_frame(1.0, pose, &s, &[], &[]);
    let (w, h, _) = r.render_to_rgba(&frame).expect("render after resize");
    assert_eq!((w, h), (400, 300));
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
        siege_t: 0.0,
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
        siege_t: 0.0,
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
    assert!(
        luminance(&on) >= off_mean,
        "effects must not darken the frame"
    );
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
            if x.abs() > 260.0 || !(-820.0..140.0).contains(&z) {
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
                siege_t: 0.0,
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
                direction: if k % 2 == 0 {
                    [1.0, 0.0, 0.0]
                } else {
                    [-1.0, 0.0, 0.0]
                },
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
    for (pa, pb) in pixels(a).iter().zip(pixels(b)) {
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
        siege_t: 0.0,
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
        siege_t: 0.0,
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
        let frame = empty_frame(
            1.0,
            pose,
            &s,
            std::slice::from_ref(&tower),
            std::slice::from_ref(&pulse),
        );
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
    let (mr, mg, mb) = (
        sr as f64 / n as f64,
        sg as f64 / n as f64,
        sb as f64 / n as f64,
    );
    assert!(
        mg > 2.0 * mr && mg > 2.0 * mb,
        "green beam pixels must be green-dominant (mean r {mr:.1} g {mg:.1} b {mb:.1})"
    );
    eprintln!("beam color: mean over {n} lit pixels r {mr:.1} g {mg:.1} b {mb:.1}");
}

// ---------------------------------------------------------------------------------------------
// Feature acceptance: siege blend, tower height treatment, CRT pass, floor feature kinds.
// ---------------------------------------------------------------------------------------------

/// A floor map built from a per-cell closure, so a test can lay out exactly the features it
/// wants (row-major, `data[z * cells + x]` as the contract specifies). Using a synthetic tile
/// keeps these tests independent of whatever the generator is emitting this week.
fn synthetic_floor(f: impl Fn(usize, usize) -> [u8; 4]) -> FloorMap {
    let n = FLOOR_TILE_CELLS as usize;
    let mut data = vec![[0u8; 4]; n * n];
    for z in 0..n {
        for x in 0..n {
            data[z * n + x] = f(x, z);
        }
    }
    FloorMap {
        cells: FLOOR_TILE_CELLS,
        data,
    }
}

/// Build a renderer offscreen against a specific floor map.
fn renderer_with_floor(
    width: u32,
    height: u32,
    settings: &Settings,
    floor: &FloorMap,
) -> Option<Renderer> {
    renderer_with_assets(width, height, settings, &atlas(), floor)
}

/// Build a renderer offscreen against explicit atlas + floor content.
fn renderer_with_assets(
    width: u32,
    height: u32,
    settings: &Settings,
    atlas: &AtlasImage,
    floor: &FloorMap,
) -> Option<Renderer> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    match pollster::block_on(Renderer::new(
        &instance,
        None,
        Viewport {
            width,
            height,
            scale: 1.0,
        },
        atlas,
        floor,
        settings,
    )) {
        Ok(r) => Some(r),
        Err(RenderError::NoAdapter | RenderError::NoDevice(_)) => {
            eprintln!("skipped: no graphics adapter/device available");
            None
        }
        Err(e) => panic!("renderer creation failed: {e}"),
    }
}

/// An atlas with no glyphs at all: towers drawn with it show their glass body and rim and nothing
/// else, which is what a measurement of the glass itself needs. (At normal viewing distances the
/// text layer covers most of a face -- a deliberate, height-independent wash -- so it swamps any
/// attempt to measure the body's height gradient against it.)
fn blank_atlas() -> AtlasImage {
    AtlasImage {
        width: ATLAS_WIDTH,
        height: ATLAS_HEIGHT,
        layers: ATLAS_LAYERS,
        rgba: vec![0; (ATLAS_WIDTH * ATLAS_HEIGHT * ATLAS_LAYERS * 4) as usize],
        blocks_per_panel: vec![Vec::new(); ATLAS_PANELS as usize],
    }
}

/// The screen box one tower covers: the columns and rows inside the search window where the
/// frame with the tower differs from the frame without it. Returns `(x0, x1, y0, y1)` inclusive.
fn silhouette_box(
    with: &[u8],
    without: &[u8],
    w: usize,
    h: usize,
    search_x0: usize,
    search_x1: usize,
) -> (usize, usize, usize, usize) {
    let (mut x0, mut x1) = (usize::MAX, 0usize);
    let (mut y0, mut y1) = (usize::MAX, 0usize);
    for y in 0..h {
        for x in search_x0..search_x1 {
            let i = (y * w + x) * 4;
            if with[i..i + 3] != without[i..i + 3] {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    assert!(
        x0 <= x1 && y0 <= y1,
        "no tower silhouette found in the search window"
    );
    (x0, x1, y0, y1)
}

/// `(mean, 25th percentile, count)` of the luminance of the *lit* pixels of a rectangle. Pixels at
/// or below `LIT` are background and are excluded, so a measurement never picks up the black
/// substrate or the sky next to the tower -- but the threshold has to sit just above true black:
/// a tower's glass is dim enough at the top that anything higher would silently measure only the
/// glyphs and the edge rim and call the result "the glass".
fn rect_lum(
    rgba: &[u8],
    w: usize,
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
) -> (f64, f64, usize) {
    const LIT: f64 = 0.0015;
    let mut lum: Vec<f64> = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 4;
            let l = (rgba[i] as f64 * 0.2126
                + rgba[i + 1] as f64 * 0.7152
                + rgba[i + 2] as f64 * 0.0722)
                / 255.0;
            if l > LIT {
                lum.push(l);
            }
        }
    }
    assert!(!lum.is_empty(), "no lit pixels in the measurement rect");
    let mean = lum.iter().sum::<f64>() / lum.len() as f64;
    lum.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (mean, lum[lum.len() / 4], lum.len())
}

/// `(mean r, mean g, mean b)` over the lit pixels of a rectangle.
fn rect_rgb(rgba: &[u8], w: usize, x0: usize, x1: usize, y0: usize, y1: usize) -> (f64, f64, f64) {
    let (mut r, mut g, mut b, mut n) = (0f64, 0f64, 0f64, 0usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 4;
            if rgba[i] as u32 + rgba[i + 1] as u32 + rgba[i + 2] as u32 > 12 {
                r += rgba[i] as f64;
                g += rgba[i + 1] as f64;
                b += rgba[i + 2] as f64;
                n += 1;
            }
        }
    }
    assert!(n > 50, "rectangle too dark to measure ({n} lit pixels)");
    (r / n as f64, g / n as f64, b / n as f64)
}

fn srgb_to_linear(b: u8) -> f64 {
    let c = b as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear luminance of a pixel.
fn pixel_lum_linear(rgba: &[u8], i: usize) -> f64 {
    srgb_to_linear(rgba[i]) * 0.2126
        + srgb_to_linear(rgba[i + 1]) * 0.7152
        + srgb_to_linear(rgba[i + 2]) * 0.0722
}

/// Raster strength: the median, over narrow column bands, of the RMS of the row-mean profile
/// after detrending, relative to the band's mean level.
///
/// Scaled to the band's own mean level so bands of different brightness are comparable. The unit
/// is the *fixed pattern* of the tube -- scanlines and the phosphor mask are phase-locked to the
/// output grid, so they survive in a column band, while picture detail does not (and a whole-row
/// average is useless here: the barrel warp walks the scanline phase across the row and cancels
/// it in the row mean, exactly as on a real curved tube). Measured against a smooth backdrop,
/// where the picture itself contributes nothing vertical.
fn raster_strength(rgba: &[u8], w: usize, h: usize) -> f64 {
    let lum = |i: usize| {
        (rgba[i] as f64 * 0.2126 + rgba[i + 1] as f64 * 0.7152 + rgba[i + 2] as f64 * 0.0722)
            / 255.0
    };
    const BAND: usize = 32;
    let mut strengths: Vec<f64> = Vec::new();
    for x0 in (w / 8..w * 7 / 8).step_by(BAND) {
        let mut rows: Vec<f64> = Vec::with_capacity(h);
        for y in 0..h {
            let mut sum = 0f64;
            for x in x0..(x0 + BAND).min(w) {
                sum += lum((y * w + x) * 4);
            }
            rows.push(sum / BAND as f64);
        }
        let mean = rows.iter().sum::<f64>() / rows.len() as f64;
        if !(0.02..=0.9).contains(&mean) {
            continue; // black or blown out: no room for a pattern either way
        }
        // Detrend with a 9-row moving average and keep the residual.
        let mut sq = 0f64;
        let mut n = 0usize;
        for y in 4..h - 4 {
            let local = rows[y - 4..y + 5].iter().sum::<f64>() / 9.0;
            sq += (rows[y] - local) * (rows[y] - local);
            n += 1;
        }
        strengths.push((sq / n as f64).sqrt() / mean);
    }
    assert!(!strengths.is_empty(), "no usable column bands");
    strengths.sort_by(|a, b| a.partial_cmp(b).unwrap());
    strengths[strengths.len() / 2]
}

/// Towers for the siege tests: three in one row, identical but for `siege_t`.
fn siege_row(ts: [f32; 3]) -> Vec<TowerInstance> {
    [-45.0f32, 0.0, 45.0]
        .iter()
        .zip(ts)
        .map(|(&x, t)| TowerInstance {
            position: [x, 0.0, -90.0],
            anim_phase: 0.25,
            face_layers: [0, 1, 2, 3],
            top_layer: 4,
            highlight_block: 0,
            highlight_t: 0.0,
            height: TOWER_HEIGHT,
            siege_t: t,
        })
        .collect()
}

/// Column band covering tower `k` of [`siege_row`] on a 480x270 frame.
fn siege_band(k: usize) -> (usize, usize) {
    let xs = [-45.0f32, 0.0, 45.0];
    let ndc = (xs[k] / 90.0) / (29.0f32.to_radians().tan() * (480.0 / 270.0));
    let cx = (240.0 * (1.0 + ndc)).round() as usize;
    (cx - 9, cx + 9)
}

/// Acceptance: `siege_t` blends each tower on its own, between the NORMAL and SIEGE ends of the
/// palette, and one tower's value never touches another tower's pixels.
#[test]
fn siege_t_blends_each_tower_toward_the_siege_palette() {
    let s = settings();
    // A black floor keeps the measurement to the tower itself.
    let f = synthetic_floor(|_, _| [0, 0, 0, 0]);
    let mut r = match renderer_with_floor(480, 270, &s, &f) {
        Some(r) => r,
        None => return,
    };
    let pose = camera([0.0, 55.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let render = |r: &mut Renderer, ts: [f32; 3]| -> Vec<u8> {
        let towers = siege_row(ts);
        let frame = empty_frame(1.0, pose, &s, &towers, &[]);
        r.render_to_rgba(&frame).expect("siege render").2
    };
    let normal = render(&mut r, [0.0, 0.0, 0.0]);
    let half = render(&mut r, [0.5, 0.5, 0.5]);
    let siege = render(&mut r, [1.0, 1.0, 1.0]);
    let only_mid = render(&mut r, [0.0, 1.0, 0.0]);
    let (w, h) = (480usize, 270usize);

    // Red/blue balance per tower: NORMAL is cyan-on-blue, SIEGE is orange-on-magenta.
    let rb = |px: &[u8], k: usize| -> f64 {
        let (x0, x1) = siege_band(k);
        let (mr, _, mb) = rect_rgb(px, w, x0, x1, 60, h - 40);
        mr / mb.max(1e-6)
    };
    let r0 = rb(&normal, 1);
    let r5 = rb(&half, 1);
    let r1 = rb(&siege, 1);
    eprintln!("siege: middle tower r/b  normal {r0:.3}  half {r5:.3}  siege {r1:.3}");
    assert!(
        r1 > 2.0 * r0,
        "siege_t = 1 must swing the tower from blue to red dominant (r/b {r0:.3} -> {r1:.3})"
    );
    assert!(
        r0 < r5 && r5 < r1,
        "siege_t = 0.5 must land between the endpoints (r/b {r0:.3} / {r5:.3} / {r1:.3})"
    );
    let m01 = mad(&normal, &siege);
    assert!(
        m01 > 3.0,
        "a full siege frame must differ measurably from the normal one (mad {m01:.3})"
    );

    // Per-tower independence: sieging only the middle tower must leave the outer ones' pixels
    // untouched, byte for byte.
    let left_band = siege_band(0);
    let mid_band = siege_band(1);
    let untouched = (60..h - 40).all(|y| {
        (left_band.0..left_band.1).all(|x| {
            let i = (y * w + x) * 4;
            normal[i..i + 3] == only_mid[i..i + 3]
        })
    });
    assert!(
        untouched,
        "sieging one tower must not change another tower's pixels"
    );
    let mid_changed = (60..h - 40).any(|y| {
        (mid_band.0..mid_band.1).any(|x| {
            let i = (y * w + x) * 4;
            normal[i..i + 3] != only_mid[i..i + 3]
        })
    });
    assert!(mid_changed, "the sieged tower's own pixels must change");
}

/// `FrameData::palette` is the NORMAL end of the blend, so a tower at `siege_t = 1` under the
/// NORMAL palette and a tower at `siege_t = 0` under the SIEGE palette are asking for exactly the
/// same thing. Rendering both must be byte-identical: that is the pre-feature behaviour (one
/// palette, no per-tower state) still reachable at `siege_t = 0`, and it pins the NORMAL end to
/// the frame's palette rather than to the `Palette::NORMAL` constant.
#[test]
fn siege_zero_leaves_the_frames_palette_alone() {
    let s = settings();
    // Black floor (an empty cell draws nothing in any palette), and the camera looks up: the only
    // thing that can change a pixel is the tower's own palette entries.
    let f = synthetic_floor(|_, _| [0, 0, 0, 0]);
    let mut r = match renderer_with_floor(320, 240, &s, &f) {
        Some(r) => r,
        None => return,
    };
    let pose = camera([0.0, 55.0, 0.0], [0.0, 0.766, -0.643], [0.0, 0.643, 0.766]);
    let render = |r: &mut Renderer, palette: Palette, siege_t: f32| -> Vec<u8> {
        let tower = TowerInstance {
            position: [0.0, 0.0, -90.0],
            anim_phase: 0.25,
            face_layers: [0, 1, 2, 3],
            top_layer: 4,
            highlight_block: 0,
            highlight_t: 0.0,
            height: TOWER_HEIGHT,
            siege_t,
        };
        let mut frame = empty_frame(1.0, pose, &s, std::slice::from_ref(&tower), &[]);
        frame.palette = palette;
        r.render_to_rgba(&frame).expect("palette render").2
    };
    let sieged = render(&mut r, Palette::SIEGE, 0.0);
    let normal = render(&mut r, Palette::NORMAL, 1.0);
    assert_eq!(
        sieged, normal,
        "siege_t = 0 must render the frame's own palette untouched, and siege_t = 1 the SIEGE end"
    );
    // The scene must actually have been visible (a black frame would pass trivially).
    assert!(nonzero_count(&sieged) > 500, "tower must cover the frame");
}

/// Acceptance: for one tower, the base region is brighter *and* more opaque than the top region.
///
/// Brightness is measured against a black floor, where a pixel is the tower's own emission and
/// nothing else. Opacity is measured by differencing two renders of the same scene whose only
/// difference is the floor's brightness (the floor palette entries feed no other shader): with
/// `obs = src + backdrop * (1 - alpha)`, the backdrop difference passes through in proportion to
/// `1 - alpha` at each height, whatever `src` happens to be.
#[test]
fn tower_base_glows_and_is_more_opaque_than_the_top() {
    let s = settings();
    let (w, h) = (480usize, 270usize);
    // A short tower 90 units down the lane: the whole shaft is in frame from a level camera, the
    // roof face is a thin sliver at the very top, and the distance haze -- which is *not*
    // height-dependent -- stays out of it entirely (haze starts at 120 units).
    let short = TowerInstance {
        position: [0.0, 0.0, -90.0],
        anim_phase: 0.25,
        face_layers: [0, 1, 2, 3],
        top_layer: 4,
        highlight_block: 0,
        highlight_t: 0.0,
        height: TOWER_HEIGHT_MIN,
        siege_t: 0.0,
    };
    let level = camera([0.0, 22.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    // Bands: the lowest fifth of the shaft and the 20..40% band under the top. Both stay clear of
    // the tower's silhouette edges, of the roof face (which is a sliver at the very top from any
    // camera above the tower) and of the top rim, all of which carry bright rim light that has
    // nothing to do with the body's height gradient. Returned as (base, top) row ranges.
    let bands = |box_: (usize, usize, usize, usize)| -> ((usize, usize), (usize, usize)) {
        let (_, _, y0, y1) = box_;
        let span = y1 - y0;
        (
            (y1 - span / 4, y1 - span / 20),
            (y0 + span / 5, y0 + span * 2 / 5),
        )
    };

    // --- Brightness over the black floor: with a glyph-free atlas a pixel there is the tower's
    // own glass emission and the edge rim, nothing else. ---
    let black = synthetic_floor(|_, _| [0, 0, 0, 0]);
    let mut rb = match renderer_with_assets(480, 270, &s, &blank_atlas(), &black) {
        Some(r) => r,
        None => return,
    };
    let with_tower = empty_frame(1.0, level, &s, std::slice::from_ref(&short), &[]);
    let without_tower = empty_frame(1.0, level, &s, &[], &[]);
    let (_, _, lit) = rb.render_to_rgba(&with_tower).expect("tower render");
    let (_, _, empty) = rb.render_to_rgba(&without_tower).expect("empty render");
    assert_eq!(
        nonzero_count(&empty),
        0,
        "the black floor map must draw nothing"
    );
    let box_ = silhouette_box(&lit, &empty, w, h, 200, 280);
    let (x0, x1, top_row, bottom_row) = box_;
    // Two pixels inside the face's vertical rim on each side: the rim is bright at every height
    // and would otherwise dominate a band whose glass is dim.
    let (bx0, bx1) = (x0 + 2, x1 - 1);
    let (base, top) = bands(box_);
    let (base_mean, base_glass, base_n) = rect_lum(&lit, w, bx0, bx1, base.0, base.1);
    let (top_mean, top_glass, top_n) = rect_lum(&lit, w, bx0, bx1, top.0, top.1);
    eprintln!(
        "tower height: box {x0}..{x1} x rows {top_row}..{bottom_row}; \
         base mean {base_mean:.4} glass {base_glass:.4} ({base_n} px) | \
         top mean {top_mean:.4} glass {top_glass:.4} ({top_n} px)"
    );
    assert!(
        base_glass > top_glass * 2.0,
        "the glass must glow at the base (p25 {base_glass:.4} vs {top_glass:.4})"
    );
    assert!(
        base_mean > top_mean * 1.05,
        "the base band must be brighter than the top band (mean {base_mean:.4} vs {top_mean:.4})"
    );
    assert!(top_mean > 0.0, "the top band must still be visible");

    // --- Opacity: look down at the same tower standing on a bright copper plane, so every band
    // of its silhouette has the floor behind it. With `obs = src + backdrop * (1 - alpha)`, the
    // fraction of the backdrop the tower hides is `alpha` (plus the tower's own, much dimmer,
    // emission), which is exactly the height-dependent quantity under test. ---
    let plane = synthetic_floor(|_, _| [0, 6, 0, 255]);
    let mut rp = match renderer_with_assets(480, 270, &s, &blank_atlas(), &plane) {
        Some(r) => r,
        None => return,
    };
    let standing = TowerInstance {
        position: [0.0, 0.0, 0.0],
        ..short
    };
    // 45 degrees down from 120 units above and 120 behind: every ray in the frame lands on the
    // floor (the horizon is far outside it), so the whole silhouette has a backdrop, and the
    // floor behind it is close enough that fog leaves most of its brightness.
    let down = camera(
        [0.0, 120.0, 120.0],
        [0.0, -FRAC_1_SQRT_2, -FRAC_1_SQRT_2],
        [0.0, FRAC_1_SQRT_2, -FRAC_1_SQRT_2],
    );
    let render_plane = |r: &mut Renderer, towers: &[TowerInstance]| -> Vec<u8> {
        let mut pal = Palette::NORMAL;
        pal.floor_trace = [
            pal.floor_trace[0] * 2.0,
            pal.floor_trace[1] * 2.0,
            pal.floor_trace[2] * 2.0,
        ];
        let mut frame = empty_frame(1.0, down, &s, towers, &[]);
        frame.palette = pal;
        r.render_to_rgba(&frame).expect("plane render").2
    };
    let lo = render_plane(&mut rp, std::slice::from_ref(&standing));
    let back = render_plane(&mut rp, &[]);
    let box_ = silhouette_box(&lo, &back, w, h, 200, 280);
    let (x0, x1, _, _) = box_;
    let (bx0, bx1) = (x0 + 2, x1 - 1);
    let (base, top) = bands(box_);
    let occluded = |y0: usize, y1: usize| -> f64 {
        let (mut sum, mut n) = (0f64, 0usize);
        for y in y0..y1 {
            for x in bx0..bx1 {
                let i = (y * w + x) * 4;
                let backdrop = pixel_lum_linear(&back, i);
                // Only pixels with a real backdrop behind them say anything about opacity.
                if backdrop > 0.02 {
                    sum += 1.0 - pixel_lum_linear(&lo, i) / backdrop;
                    n += 1;
                }
            }
        }
        assert!(n > 100, "too few backed pixels behind the tower ({n})");
        sum / n as f64
    };
    // The tonemap is monotone but compressive, so this understates the true occlusion; what it
    // cannot do is invert its ordering, and the two bands differ by a factor of six in the
    // shader's alpha.
    let occ_base = occluded(base.0, base.1);
    let occ_top = occluded(top.0, top.1);
    eprintln!(
        "tower height: backdrop hidden base {occ_base:.3}, top {occ_top:.3} (0 = clear glass)"
    );
    assert!(
        occ_base > 0.08 && occ_top > 0.0,
        "both bands must hide a real fraction of the backdrop (base {occ_base:.3}, top {occ_top:.3})"
    );
    assert!(
        occ_base > occ_top * 1.8,
        "the base must hide far more of the backdrop than the top (hidden {occ_base:.3} vs {occ_top:.3})"
    );
}

/// Acceptance: the CRT pass reconstructs a signal buffer, so it must handle a signal and an
/// output size that are both odd, and it must leave visible raster structure.
///
/// The scene is a solid copper plane filling the frame: a smooth backdrop is what makes the
/// raster measurable, because every bit of vertical structure in the picture is structure the
/// fold cannot tell apart from the tube's.
#[test]
fn crt_pass_handles_odd_output_size() {
    let mut s = settings();
    let plane = synthetic_floor(|_, _| [0, 6, 0, 255]);
    let mut r = match renderer_with_floor(1017, 613, &s, &plane) {
        Some(r) => r,
        None => return,
    };
    let pose = camera([120.0, 46.0, 120.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
    let mut render = |r: &mut Renderer, crt: f32| -> Vec<u8> {
        s.crt = crt;
        let frame = empty_frame(1.0, pose, &s, &[], &[]);
        let (w, h, px) = r.render_to_rgba(&frame).expect("odd-size crt render");
        assert_eq!((w, h), (1017, 613));
        px
    };
    let off = render(&mut r, 0.0);
    let on = render(&mut r, 1.0);
    let m = mad(&off, &on);
    eprintln!("crt odd size 1017x613: mad(0, 1) = {m:.3}");
    assert!(m > 0.4, "crt = 1 must differ measurably (mad {m:.3})");
    let d_on = raster_strength(&on, 1017, 613);
    let d_off = raster_strength(&off, 1017, 613);
    let d_mid = raster_strength(&render(&mut r, 0.35), 1017, 613);
    eprintln!(
        "crt odd size: frame raster strength crt=0 {d_off:.4}, crt=0.35 {d_mid:.4}, crt=1 {d_on:.4}"
    );
    assert!(
        d_on > 5.0 * d_off,
        "the CRT must add a grid-locked raster: {d_on:.4} against the plain composite's {d_off:.4}"
    );
    assert!(
        d_mid > 2.0 * d_off && d_mid < d_on,
        "the default amount must show a real but shallower raster than full ({d_mid:.4})"
    );
}

/// Acceptance: a floor cell whose feature kind is reserved (`G > 8`) draws nothing -- not its
/// pads, not its pour and not the traces its direction mask asks for -- while known kinds on the
/// same tile do draw.
#[test]
fn floor_reserved_feature_kinds_draw_nothing() {
    let s = settings();
    // 200 units up, straight down: a wide patch of the tile, with the tile repeating around it.
    let pose = camera([120.0, 200.0, 120.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
    for g in [9u8, 12, 200, 255] {
        // Every direction bit set, the thickest gauge, hatched, at full brightness: if a reserved
        // kind were misread as a known one, this cell would light up.
        let f = synthetic_floor(|_, _| [0xFF, g, 0x0F, 255]);
        let mut r = match renderer_with_floor(320, 240, &s, &f) {
            Some(r) => r,
            None => return,
        };
        let frame = empty_frame(1.0, pose, &s, &[], &[]);
        let (_, _, px) = r.render_to_rgba(&frame).expect("reserved-kind render");
        let lit = nonzero_count(&px);
        assert_eq!(
            lit, 0,
            "G = {g} is reserved and must draw nothing ({lit} lit)"
        );
    }
    // Controls: known kinds on the same tile do light up.
    for (g, label) in [(6u8, "copper pour"), (7, "silkscreen"), (0, "traces")] {
        let f = synthetic_floor(move |_, _| [0xFF, g, 0x02, 255]);
        let mut r = match renderer_with_floor(320, 240, &s, &f) {
            Some(r) => r,
            None => return,
        };
        let frame = empty_frame(1.0, pose, &s, &[], &[]);
        let (_, _, px) = r.render_to_rgba(&frame).expect("control render");
        let lit = nonzero_count(&px);
        eprintln!("floor control {label}: {lit} lit pixels");
        assert!(lit > 500, "{label} must draw ({lit} lit)");
    }
}

// ---------------------------------------------------------------------------------------------
// Floor trace continuity. The trace SDF is built from half-segments that leave a cell centre, so
// a 45-degree run passes through the point where four cells meet. Only the two cells along the
// run own the collinear halves there; the two cells flanking the corner own nothing, and the
// conductor must still be continuous and even through them.
//
// Measuring that needs a perpendicular cut through the run, so the diagonal camera is rolled 45
// degrees: the world diagonal lands exactly on the image's horizontal axis and an image column is
// a clean cross-section of the conductor.
// ---------------------------------------------------------------------------------------------

/// Cell the continuity runs start at, their length in cells, and the world coordinate of their
/// midpoint (`2.5 * RUN_CELL + 1.25 * (RUN_LEN - 1)` = 60: cell `k` is centred on world `2.5k`
/// and the run ends one half-cell past each end cell's centre), which is where each camera sits.
const RUN_CELL: usize = 20;
const RUN_LEN: usize = 9;
const RUN_MID: f32 = 2.5 * (RUN_CELL as f32) + 1.25 * ((RUN_LEN - 1) as f32);

/// Camera height for the continuity frames: 800x800 then spans 79.8 world units, i.e. 10.0 px per
/// unit at the image centre, and the 22.5-unit run crosses nine cells with every gauge several
/// pixels wide.
const RUN_CAM_H: f32 = 72.0;
/// Columns to drop at each end of a measured run before judging it, so the round end cap is not
/// mistaken for a pinch: 30 px is three cells at the test's scale.
const RUN_END_SKIP: usize = 30;

/// One straight 45-degree run: cell `(k, k)` for `k` in `RUN_CELL..RUN_CELL + RUN_LEN` carries
/// both diagonal bits, so the chain is a single conductor through every cell centre on the world
/// diagonal, ending in a round cap at each end cell's outer corner.
fn diagonal_run_floor(gauge: u8) -> FloorMap {
    synthetic_floor(move |x, z| {
        if x == z && (RUN_CELL..RUN_CELL + RUN_LEN).contains(&x) {
            [16 | 128, 0, gauge, 255]
        } else {
            [0, 0, 0, 0]
        }
    })
}

/// The orthogonal control: the same run laid along +x through cell row 24, whose centre line is
/// the camera axis. Bits 1|2 make it one collinear conductor of the same length.
fn orthogonal_run_floor(gauge: u8) -> FloorMap {
    synthetic_floor(move |x, z| {
        if z == 24 && (RUN_CELL..RUN_CELL + RUN_LEN).contains(&x) {
            [1 | 2, 0, gauge, 255]
        } else {
            [0, 0, 0, 0]
        }
    })
}

/// Top-down with the screen axes rolled 45 degrees, so the world diagonal is the image's
/// horizontal axis (`right` = the diagonal, `up` = the anti-diagonal).
fn diagonal_camera() -> CameraPose {
    let r = 1.0 / 2.0f32.sqrt();
    camera(
        [RUN_MID, RUN_CAM_H, RUN_MID],
        [0.0, -1.0, 0.0],
        [r, 0.0, -r],
    )
}

/// Top-down with the screen axes on the world axes, so a +x run is horizontal in the image.
fn orthogonal_camera() -> CameraPose {
    camera(
        [RUN_MID, RUN_CAM_H, RUN_MID],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
    )
}

/// Luma of one pixel.
fn pixel_luma(rgba: &[u8], i: usize) -> f64 {
    0.299 * rgba[i] as f64 + 0.587 * rgba[i + 1] as f64 + 0.114 * rgba[i + 2] as f64
}

/// Peak luma of a frame: the conductor's own core brightness, which is the full-coverage
/// reference the cross-sections are measured against.
fn peak_luma(rgba: &[u8]) -> f64 {
    (0..rgba.len() / 4)
        .map(|i| pixel_luma(rgba, i * 4))
        .fold(0.0, f64::max)
}

/// Per-column width of a horizontal conductor band, in pixels: the integral of the coverage
/// above half the core brightness down each column. A column with no conductor reads zero; the
/// emissive halo just off the copper sits around a fifth of the core and is excluded; and the
/// half-core crossing of the edge ramp is the true edge, which makes the figure sub-pixel.
fn column_widths(rgba: &[u8], w: usize, h: usize, peak: f64) -> Vec<f64> {
    let lo = 0.5 * peak;
    (0..w)
        .map(|x| {
            (0..h)
                .map(|y| {
                    let l = pixel_luma(rgba, (y * w + x) * 4);
                    ((l - lo) / (peak - lo)).clamp(0.0, 1.0)
                })
                .sum::<f64>()
        })
        .collect()
}

/// `(min, median, max)` over the interior of a measured band: the columns between the first and
/// last that carry conductor, `skip` columns in from each end.
fn band_stats(widths: &[f64], skip: usize) -> (f64, f64, f64) {
    let first = widths.iter().position(|w| *w > 0.0).expect("band present");
    let last = widths.iter().rposition(|w| *w > 0.0).expect("band present");
    let window = &widths[first + skip..=last - skip];
    let mut sorted = window.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (
        sorted[0],
        sorted[sorted.len() / 2],
        sorted[sorted.len() - 1],
    )
}

/// The conductor's width at the image centre for a top-down 800x800 frame, in pixels: the frame
/// spans `2 * H * tan(fov/2)` world units, so the scale is uniform across both axes.
fn run_scale_px() -> f64 {
    let fov = 58.0f64 * std::f64::consts::PI / 180.0;
    800.0 / (2.0 * RUN_CAM_H as f64 * (fov / 2.0).tan())
}

/// Measure one straight run: render it from `pose` and return the conductor's cross-section
/// statistics in the image -- `(min, median, max, span)` in pixels, the span being how many
/// columns carry any conductor at all.
fn measure_run(floor: &FloorMap, s: &Settings, pose: CameraPose) -> Option<(f64, f64, f64, usize)> {
    let mut r = renderer_with_floor(800, 800, s, floor)?;
    let frame = empty_frame(1.0, pose, s, &[], &[]);
    let (w, h, px) = r.render_to_rgba(&frame).expect("run render");
    let widths = column_widths(&px, w as usize, h as usize, peak_luma(&px));
    let (min, med, max) = band_stats(&widths, RUN_END_SKIP);
    let first = widths.iter().position(|v| *v > 0.0).expect("band present");
    let last = widths.iter().rposition(|v| *v > 0.0).expect("band present");
    Some((min, med, max, last - first + 1))
}

/// The three gauge classes and their data half-widths.
const GAUGES: [(u8, f64); 3] = [(0, 0.2), (1, 0.3), (2, 0.44)];

/// Acceptance: a long 45-degree run renders as one continuous conductor of even width, matching
/// the same-gauge orthogonal run. The half-segment SDF used to pinch the conductor to zero width
/// at every cell corner -- a chain of lozenges -- which is the regression the owner reported.
#[test]
fn floor_diagonal_runs_are_continuous() {
    let s = settings();
    let scale = run_scale_px();
    for (gauge, hw) in GAUGES {
        let (min, med, max, span) =
            match measure_run(&diagonal_run_floor(gauge), &s, diagonal_camera()) {
                Some(m) => m,
                None => return,
            };
        let (_, straight, _, straight_span) =
            match measure_run(&orthogonal_run_floor(gauge), &s, orthogonal_camera()) {
                Some(m) => m,
                None => return,
            };
        let expected = 2.0 * hw * scale;
        eprintln!(
            "diagonal gauge {gauge}: span {span} min {min:.2} median {med:.2} max {max:.2} px; \
             straight run {straight:.2} px (data width {expected:.2})"
        );
        assert!(
            min > 0.25 * med,
            "gauge {gauge}: the conductor pinches to {min:.2} px against a {med:.2} px run \
             (substrate between cells)"
        );
        assert!(
            min >= 0.9 * med,
            "gauge {gauge}: the run waists from {min:.2} to {max:.2} px (median {med:.2})"
        );
        // The measured width runs about a third of a pixel under the data width: the output is
        // sRGB-encoded, so a half-covered pixel reads a little brighter than half the core and the
        // half-core crossing sits just inside the true edge.
        assert!(
            (med - expected).abs() <= 0.5,
            "gauge {gauge}: the run renders {med:.2} px wide, the data asks for {expected:.2}"
        );
        assert!(
            (med - straight).abs() <= 0.05 * straight,
            "gauge {gauge}: the 45-degree run is {med:.2} px wide against {straight:.2} px for the \
             straight run of the same gauge"
        );
        // The same nine cells make a run that is sqrt(2) times longer along the world diagonal, so
        // the rendered spans say whether the corner copper stays exactly on the route: a phantom
        // half-segment past an end (or a missing one) shows up here first.
        let straight_diag_span = straight_span as f64 * 2.0f64.sqrt();
        assert!(
            (span as f64 - straight_diag_span).abs() <= 12.0,
            "gauge {gauge}: the 45-degree run spans {span} px against {straight_diag_span:.0} for a \
             run of the same cell length"
        );
    }
}

/// Acceptance: the orthogonal run is unchanged -- one conductor of exactly its data width, with no
/// waisting along it. The half-segment construction was already exact for orthogonal traces (each
/// cell's mirror bit continues the run collinearly), so this pins that the corner union did not
/// disturb it.
#[test]
fn floor_orthogonal_runs_are_unchanged() {
    let s = settings();
    let scale = run_scale_px();
    for (gauge, hw) in GAUGES {
        let (min, med, max, span) =
            match measure_run(&orthogonal_run_floor(gauge), &s, orthogonal_camera()) {
                Some(m) => m,
                None => return,
            };
        let expected = 2.0 * hw * scale;
        eprintln!(
            "orthogonal gauge {gauge}: span {span} min {min:.2} median {med:.2} max {max:.2} px \
             (data width {expected:.2})"
        );
        assert!(
            min >= 0.95 * med,
            "gauge {gauge}: the orthogonal run waists from {min:.2} to {max:.2} px \
             (median {med:.2})"
        );
        assert!(
            (med - expected).abs() <= 0.5,
            "gauge {gauge}: the run renders {med:.2} px wide, the data asks for {expected:.2}"
        );
    }
}

/// Write `docs/scratch/floor-overhead.png` (1920x1080, straight-down floor view, CRT off) to
/// inspect the circuit-board look. `GIBSON_RENDER_PROBE_OUT` redirects the path. Gated behind
/// `GIBSON_RENDER_PROBE=1`.
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
    let mut r = match renderer_at(1920, 1080, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    // 60 units up, straight down: a close look at the generated tile -- diagonals, gauges, pours
    // and footprints all at a readable scale.
    let pose = camera([120.0, 60.0, 120.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
    let frame = empty_frame(1.0, pose, &s, &[], &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("floor probe render");
    let out = probe_out("floor-overhead.png");
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
            siege_t: 0.0,
        });
    }
    let pose = camera([105.0, 55.0, 100.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let frame = empty_frame(1.0, pose, &s, &towers, &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("face study render");
    let out = probe_out("face-study.png");
    write_png(&out, w, h, &rgba);
    eprintln!("wrote {}", out.display());
}

/// Per-pass GPU timestamp profile plus a wall-clock frame time over the populated frame.
///
/// Gated behind `GIBSON_PROFILE=1` (the renderer only requests `TIMESTAMP_QUERY` when that
/// variable is set at construction) and `GIBSON_PROFILE_SIZE=WxH` (default 2940x1912, the
/// screensaver drawable). Prints medians of the per-pass timestamps plus the SUM, and the wall
/// frame time measured with the same method as the on-screen numbers (incl. readback).
#[test]
fn probe_frame_profile() {
    if std::env::var_os("GIBSON_PROFILE").is_none() {
        return;
    }
    let (pw, ph) = std::env::var("GIBSON_PROFILE_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))
        })
        .unwrap_or((2940, 1912));
    // Report which timestamp capabilities this adapter actually exposes (the renderer refuses
    // to enable profiling without both, so a silent "profiling disabled" needs an explanation).
    {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        if let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            }))
        {
            let f = adapter.features();
            println!(
                "timestamp features: query={} inside_encoders={} inside_passes={}",
                f.contains(wgpu::Features::TIMESTAMP_QUERY),
                f.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
                f.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
            );
        }
    }
    let crt: f32 = std::env::var("GIBSON_RENDER_CRT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.35);
    // Motion-blur amount, so the depth-carry store can be measured in both states (the carry is
    // only read by the motion-blur pass, so its store is dead work when this is 0).
    let motion_blur: f32 = std::env::var("GIBSON_RENDER_MOTION_BLUR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.5);
    let s = Settings {
        bloom: 0.35,
        motion_blur,
        grain: 0.03,
        crt,
        ..settings()
    };
    let mut r = match renderer_at(pw, ph, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let (towers, pulses, pose) = populated_frame();
    let frame = empty_frame(3.0, pose, &s, &towers, &pulses);
    println!(
        "probe_frame_profile: {pw}x{ph} towers={} pulses={}",
        towers.len(),
        pulses.len()
    );
    // Warm up (first-use allocations, pipeline caches).
    let _ = r.render_to_rgba(&frame).expect("warmup frame");

    const N: u32 = 3;
    let t0 = std::time::Instant::now();
    for _ in 0..N {
        let _ = r.render_to_rgba(&frame).expect("timed frame");
    }
    let wall = t0.elapsed().as_secs_f64() * 1000.0 / N as f64;
    println!("wall: {wall:.1} ms/frame (incl. readback, {N} frames)");

    let mut values: Vec<(&'static str, Vec<f64>)> = Vec::new();
    for _ in 0..5 {
        for (name, ms) in r.profile_frame(&frame).expect("profile frame") {
            match values.iter_mut().find(|(n, _)| *n == name) {
                Some((_, v)) => v.push(ms),
                None => values.push((name, vec![ms])),
            }
        }
    }
    println!("--- GPU pass times (median of 5, ms) ---");
    let mut total = 0.0;
    for (name, mut v) in values {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = v[v.len() / 2];
        total += med;
        println!("{name:<22} {med:7.3}");
    }
    println!("{:<22} {total:7.3}", "SUM");
}

/// Where a probe writes: `docs/scratch/<name>` under the repo root, or exactly
/// `GIBSON_RENDER_PROBE_OUT` when that is set.
fn probe_out(name: &str) -> std::path::PathBuf {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = std::env::var("GIBSON_RENDER_PROBE_OUT")
        .ok()
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../docs/scratch").join(name));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create probe dir");
    }
    out
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

    let out = probe_out("render-tall.png");
    write_png(&out, w, h, &rgba);
    eprintln!("wrote {}", out.display());
}

/// Write `docs/scratch/siege-wave.png` (1920x1080): the populated lane with a siege rolling
/// through it -- `siege_t` rises with distance from a seed tower, so the shot shows normal, half
/// and fully sieged towers side by side. `GIBSON_RENDER_CRT` sets the CRT amount. Gated like the
/// other probes.
#[test]
fn probe_siege_wave() {
    if std::env::var("GIBSON_RENDER_PROBE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set GIBSON_RENDER_PROBE=1 to render the siege probe image");
        return;
    }
    let crt: f32 = std::env::var("GIBSON_RENDER_CRT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let s = Settings {
        bloom: 0.45,
        motion_blur: 0.0,
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
    let (mut towers, pulses, pose) = populated_frame();
    // A siege front rolling up the canyon towards the camera, with each tower on its own clock.
    // The wave is radial from the camera and spends its whole ramp inside the near field: past
    // 220 units the distance fog has taken over and a tower reads as atmosphere (a blue haze)
    // rather than as a building, so a front pushed further out would show no colour at all. Each
    // tower also carries a per-tower jitter, so neighbours sit at visibly different points of the
    // flip instead of in tidy rings.
    let eye = (pose.position[0], pose.position[2]);
    for t in towers.iter_mut() {
        let d = ((t.position[0] - eye.0).powi(2) + (t.position[2] - eye.1).powi(2)).sqrt();
        let front = (d - 60.0) / 70.0;
        let jitter = hash01((t.position[0] / 30.0) as i32, (t.position[2] / 30.0) as i32) - 0.5;
        t.siege_t = (front + jitter * 0.6).clamp(0.0, 1.0);
    }
    let frame = empty_frame(1.5, pose, &s, &towers, &pulses);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("siege wave render");
    let out = probe_out("siege-wave.png");
    write_png(&out, w, h, &rgba);
    eprintln!("wrote {}", out.display());
}

/// Write `docs/scratch/floor-features.png` (1920x1080): a synthetic tile that lays out every
/// feature the widened encoding can ask for -- orthogonal and diagonal traces in all three gauge
/// classes, through-hole pads, vias, SMD pads, IC bodies with pins, solid and hatched pours,
/// silkscreen and mounting holes -- so the shader can be graded without depending on what the
/// generator emits. Gated like the other probes.
#[test]
fn probe_floor_features() {
    if std::env::var("GIBSON_RENDER_PROBE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set GIBSON_RENDER_PROBE=1 to render the floor feature probe");
        return;
    }
    let s = Settings {
        bloom: 0.35,
        crt: 0.0,
        seed: 7,
        grid: 60,
        ..Settings::default()
    };
    // 4x4-cell feature blocks (10 world units each) tiled across the whole map, so the frame
    // shows a populated board rather than a handful of shapes on black: one block per feature kind
    // in the tile, and a routed mesh everywhere else.
    let f = synthetic_floor(|x, z| {
        let (bx, bz) = (x / 4 % 4, z / 4 % 4);
        let (lx, lz) = (x % 4, z % 4);
        match (bx, bz) {
            // Orthogonal run with a 90-degree corner.
            (0, 0) => [if lz == 2 { 3 } else { 0 }, 0, 0, 255],
            // Diagonal run: a 45-degree jog cornering into an orthogonal run.
            (1, 0) => [
                if lz == 0 && lx == 0 {
                    16
                } else if lz == 3 {
                    0
                } else {
                    5
                },
                0,
                0,
                255,
            ],
            // Gauge: thin, medium and thick runs side by side.
            (2, 0) => [1, 0, ((lz as u8) & 3).min(2), 255],
            // Through-hole pads (G=1) and vias (G=2) on a signal run.
            (3, 0) => {
                let g = if lx == 2 && lz == 2 {
                    1
                } else if lz == 2 {
                    2
                } else {
                    0
                };
                [if lz == 2 { 1 } else { 0 }, g, 0, 255]
            }
            // SMD pads (G=4) tiling a chip footprint.
            (0, 1) => [0, if lx % 2 == 0 && lz % 2 == 0 { 4 } else { 0 }, 0, 255],
            // IC body (G=3) with pins (G=5) around it.
            (1, 1) => {
                let body = (1..3).contains(&lx) && (1..3).contains(&lz);
                let pin = !body && (lx == 0 || lz == 0);
                [
                    0,
                    if body {
                        3
                    } else if pin {
                        5
                    } else {
                        0
                    },
                    0,
                    255,
                ]
            }
            // Solid pour (G=6) and hatched pour (B bit 3).
            (2, 1) => [0, 6, 0, 255],
            (3, 1) => [0, 6, 8, 255],
            // Silkscreen (G=7) outline, and a mounting hole (G=8) ringed by power traces.
            (0, 2) => [
                0,
                if lx == 0 || lz == 0 || lx == 3 || lz == 3 {
                    7
                } else {
                    0
                },
                0,
                255,
            ],
            (1, 2) => {
                let g = if lx == 2 && lz == 2 { 8 } else { 0 };
                [if lx == 2 || lz == 2 { 2 } else { 0 }, g, 2, 255]
            }
            // A three-wide bus bundle (B bit 2).
            (2, 2) => [if lz == 1 { 1 } else { 0 }, 0, 4, 255],
            // A trace landing on a pad, and a mixed corner.
            (3, 2) => {
                let g = if lx == 2 && lz == 2 { 1 } else { 0 };
                [
                    if lz == 2 {
                        3
                    } else if lx == 2 {
                        8
                    } else {
                        0
                    },
                    g,
                    0,
                    255,
                ]
            }
            // Routed mesh everywhere else: through-routing with diagonal jogs, so the board reads
            // as a board between the feature blocks instead of as isolated shapes on black.
            _ => {
                let h = (x * 7 + z * 13) % 11;
                match h {
                    0 => [1 | 4, 0, 1, 255],
                    1 => [2 | 8, 0, 1, 255],
                    2 => [1 | 8, 0, 0, 255],
                    3 => [16, 0, 0, 255],
                    4 => [2 | 4, 0, 2, 255],
                    5 => [1 | 2 | 4 | 8, 0, 1, 255],
                    6 => [0, 2, 0, 255],
                    7 => [3, 0, 1, 255],
                    8 => [1 | 16, 0, 0, 255],
                    9 => [0, 0, 0, 255],
                    _ => [4 | 8, 0, 0, 255],
                }
            }
        }
    });
    let mut r = match renderer_with_floor(1920, 1080, &s, &f) {
        Some(r) => r,
        None => return,
    };
    // Straight down from 58 units: the frame spans ~64 world units, i.e. about six feature blocks
    // with every cell big enough to tell a via from a pad.
    let pose = camera([18.0, 58.0, 30.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
    let frame = empty_frame(1.0, pose, &s, &[], &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("floor feature render");
    let out = probe_out("floor-features.png");
    write_png(&out, w, h, &rgba);
    eprintln!("wrote {}", out.display());
}

/// Panel band height: the renderer slices a face into `round(height / 38)` bands, so a 38-unit
/// tower is exactly one band -- the whole panel over the whole face.
const BAND_UNITS_TEST: f32 = 38.0;

/// Write `docs/scratch/panel-legibility.png` (1920x1080): one tower face showing atlas panel 28 --
/// the hero directory list ("SHIPPING FORCASTS", ...) -- filling the frame at roughly its native
/// mosaic scale, so the tube's cost to name legibility can be judged. `GIBSON_RENDER_CRT` sets the
/// CRT amount. Gated like the other probes.
#[test]
fn probe_panel_legibility() {
    if std::env::var("GIBSON_RENDER_PROBE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set GIBSON_RENDER_PROBE=1 to render the panel legibility probe");
        return;
    }
    let crt: f32 = std::env::var("GIBSON_RENDER_CRT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.35);
    let s = Settings {
        bloom: 0.25,
        motion_blur: 0.0,
        grain: 0.0,
        crt,
        seed: 7,
        grid: 60,
        ..Settings::default()
    };
    let mut r = match renderer_at(1920, 1080, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    // A one-band tower (the band height) whose every face shows the hero panel, 34 units away so
    // the 12x38-unit face covers the frame at about the panel's own 256x768 mosaic scale.
    let tower = TowerInstance {
        position: [0.0, 0.0, 0.0],
        anim_phase: 0.25,
        face_layers: [28, 28, 28, 28],
        top_layer: 28,
        highlight_block: 0,
        highlight_t: 0.0,
        height: BAND_UNITS_TEST,
        siege_t: 0.0,
    };
    let pose = camera(
        [0.0, BAND_UNITS_TEST * 0.5, 34.0],
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
    );
    let frame = empty_frame(1.0, pose, &s, std::slice::from_ref(&tower), &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("panel legibility render");
    let out = probe_out("panel-legibility.png");
    write_png(&out, w, h, &rgba);
    eprintln!("wrote {}", out.display());
}

/// Write `docs/scratch/siege-towers.png` (1280x720): three identical towers side by side at
/// `siege_t` 0, 0.5 and 1, close enough to read as separate buildings. The lane view shows the
/// wave rolling through the city, but towers there stack through the haze a dozen deep, so this is
/// the shot that answers "does one building turn red on its own?". Gated like the other probes.
#[test]
fn probe_siege_towers() {
    if std::env::var("GIBSON_RENDER_PROBE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set GIBSON_RENDER_PROBE=1 to render the siege tower probe");
        return;
    }
    let crt: f32 = std::env::var("GIBSON_RENDER_CRT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let s = Settings {
        bloom: 0.45,
        motion_blur: 0.0,
        grain: 0.03,
        crt,
        seed: 7,
        grid: 60,
        ..Settings::default()
    };
    let mut r = match renderer_at(1280, 720, 1.0, &s) {
        Some(r) => r,
        None => return,
    };
    let towers = siege_row([0.0, 0.5, 1.0]);
    let pose = camera([0.0, 55.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
    let frame = empty_frame(1.5, pose, &s, &towers, &[]);
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("siege tower render");
    let out = probe_out("siege-towers.png");
    write_png(&out, w, h, &rgba);
    eprintln!("wrote {}", out.display());
}
