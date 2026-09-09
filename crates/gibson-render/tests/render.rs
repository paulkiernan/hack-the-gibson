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
        _pad: 0.0,
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
        _pad: 0.0,
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
        _pad: 0.0,
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
            if x.abs() > 260.0 || z > 20.0 || z < -820.0 {
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
                _pad: 0.0,
            });
        }
    }
    // Pulses running along the x-lanes at z = 15 + 30k.
    let mut pulses = Vec::new();
    for k in 0..10 {
        let z = 15.0 + 30.0 * k as f32;
        for x in (0..900).step_by(150) {
            pulses.push(PulseInstance {
                position: [x as f32, 1.5 + 0.4 * (k as f32 * 0.37).fract(), z],
                length: 14.0,
                direction: if k % 2 == 0 { [1.0, 0.0, 0.0] } else { [-1.0, 0.0, 0.0] },
                intensity: 0.9,
            });
        }
    }
    // Back-to-front along -z for correct translucency ordering.
    towers.sort_by(|a, b| b.position[2].partial_cmp(&a.position[2]).unwrap());
    let pose = camera([0.0, 9.0, 140.0], [0.0, -0.06, -1.0], [0.0, 1.0, 0.0]);
    (towers, pulses, pose)
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

/// Write `docs/scratch/render-probe.png` (1920x1080, populated city) for visual inspection.
/// Gated behind `GIBSON_RENDER_PROBE=1` so the default test run stays fast.
#[test]
fn probe_populated_frame() {
    if std::env::var("GIBSON_RENDER_PROBE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set GIBSON_RENDER_PROBE=1 to render the 1080p probe image");
        return;
    }
    let s = Settings {
        bloom: 0.45,
        motion_blur: 0.35,
        grain: 0.04,
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
    let (w, h, rgba) = r.render_to_rgba(&frame).expect("probe render");
    assert_eq!((w, h), (1920, 1080));

    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = manifest.join("../../docs/scratch/render-probe.png");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create docs/scratch");
    }
    write_png(&out, w, h, &rgba);
    eprintln!("wrote {}", out.display());
}
