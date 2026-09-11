//! Offscreen snapshot host: render one deterministic still to a PNG and exit.
//!
//! The scene is stepped in fixed 1/60 s increments from `t = 0` to the
//! requested time (each step runs a full `snapshot()`), so the result is
//! reproducible regardless of machine speed: the same `--seed`, `--size`,
//! `--time`, and render settings always produce byte-identical pixels. The
//! final step's image is encoded as 8-bit RGB PNG, rows top-first (the
//! rendered image is opaque by construction, so the alpha plane is dropped
//! before encoding rather than stored as a constant 255).

use std::fs::File;
use std::io::BufWriter;

use gibson_core::{Gibson, SurfaceTarget};

use crate::cli::{self, Cli};

/// Fixed simulation step: 60 updates per simulated second.
const STEP_HZ: f64 = 60.0;

pub fn run(cli: &Cli) -> Result<(), String> {
    let out_path = cli
        .snapshot
        .clone()
        .ok_or("internal: snapshot mode without a path")?;
    let (width, height) = cli::parse_size(&cli.size)?;
    let settings = cli::resolve(cli)?;
    let time = cli.time.max(0.0);

    // Offscreen renderer: `scale` is just `render_scale` here (no display), so
    // the output is `size × render_scale` pixels.
    let scale = settings.render_scale;
    let mut gibson = pollster::block_on(Gibson::new(
        SurfaceTarget::Offscreen,
        width,
        height,
        scale,
        settings,
    ))
    .map_err(|e| format!("cannot initialize offscreen renderer: {e}"))?;

    // Step the scene at a fixed 60 Hz. The first call defines the scene's
    // `t = 0`; the final step is the image we keep.
    let steps = (time * STEP_HZ).round() as u64;
    let mut image = None;
    for step in 0..=steps {
        let t = step as f64 / STEP_HZ;
        let frame = gibson
            .snapshot(t)
            .map_err(|e| format!("snapshot frame {step} (t = {t:.3}s) failed: {e}"))?;
        image = Some(frame);
    }
    let (w, h, rgba) = image.expect("at least one snapshot step ran");

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
    }
    // The composite output is opaque (alpha 255 everywhere): store RGB and drop the wasted
    // 8-bit alpha plane (~25% of the file on this content). Best compression with adaptive
    // per-row filtering buys the rest; the deterministic pixel stream keeps two runs of the
    // same arguments byte-identical.
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|px| [px[0], px[1], px[2]])
        .collect();
    let file = File::create(&out_path)
        .map_err(|e| format!("cannot create {}: {e}", out_path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Best);
    encoder.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("png header: {e}"))?;
    writer
        .write_image_data(&rgb)
        .map_err(|e| format!("png write: {e}"))?;
    println!("wrote {} ({}x{})", out_path.display(), w, h);
    Ok(())
}
