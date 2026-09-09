//! Dump atlas layers to PNG for eyeballing.
//!
//! Usage: `cargo run --release -p gibson-atlas --example atlas_dump [LAYER ...]`
//!
//! With no arguments a representative set of layers is dumped: mosaic panels, directory panels,
//! and their variant B counterparts. R is shown as the visible channel: block interiors are
//! tinted by block id (B) so rectangles are obvious, and glyph coverage brightens toward white.
//! Files land in `docs/scratch/atlas/layer_{n}.png` (gitignored).

use std::fs;
use std::path::Path;
use std::time::Instant;

use gibson_types::{ATLAS_HEIGHT, ATLAS_WIDTH};

const W: usize = ATLAS_WIDTH as usize;
const H: usize = ATLAS_HEIGHT as usize;

/// Dim tints per block id modulo 8 so neighboring blocks stay distinguishable.
const TINTS: [(f32, f32, f32); 8] = [
    (0.10, 0.16, 0.42),
    (0.16, 0.36, 0.30),
    (0.42, 0.14, 0.36),
    (0.34, 0.24, 0.10),
    (0.10, 0.30, 0.44),
    (0.30, 0.16, 0.30),
    (0.20, 0.24, 0.14),
    (0.12, 0.20, 0.20),
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Font metric probe (ascent/descent at our two sizes, plus sample advance widths).
    let mono = fontdue::Font::from_bytes(gibson_atlas::IBM_PLEX_MONO_TTF, Default::default())
        .expect("mono font");
    let michroma = fontdue::Font::from_bytes(gibson_atlas::MICHROMA_TTF, Default::default())
        .expect("michroma font");
    print_font_metrics("plex-mono 7px", &mono, 7.0);
    print_font_metrics("michroma 18px", &michroma, 18.0);
    let mw = |f: &fontdue::Font, ch| {
        let (m, _) = f.rasterize(ch, 18.0);
        m.advance_width
    };
    println!(
        "michroma 18px advances: 'M' {:.1} 'W' {:.1} 'I' {:.1} '.' {:.1} ' ' {:.1}",
        mw(&michroma, 'M'),
        mw(&michroma, 'W'),
        mw(&michroma, 'I'),
        mw(&michroma, '.'),
        mw(&michroma, ' ')
    );

    let t = Instant::now();
    let atlas = gibson_atlas::generate(1234);
    let gen = t.elapsed();

    println!(
        "generate(1234): {} x {} x {} layers, {:.1} MiB in {:.1?}",
        atlas.width,
        atlas.height,
        atlas.layers,
        atlas.rgba.len() as f64 / (1024.0 * 1024.0),
        gen
    );
    let counts: Vec<usize> = atlas.blocks_per_panel.iter().map(|b| b.len()).collect();
    println!(
        "blocks per panel: min {} max {} avg {:.1}",
        counts.iter().min().copied().unwrap_or(0),
        counts.iter().max().copied().unwrap_or(0),
        counts.iter().sum::<usize>() as f64 / counts.len() as f64
    );
    let mo: Vec<usize> = counts[..28].to_vec();
    let di: Vec<usize> = counts[28..].to_vec();
    let stats = |v: &[usize], name: &str| {
        let sum: usize = v.iter().sum();
        println!(
            "  {name}: min {} max {} avg {:.1}",
            v.iter().min().copied().unwrap_or(0),
            v.iter().max().copied().unwrap_or(0),
            sum as f64 / v.len() as f64
        );
    };
    stats(&mo, "mosaic panels 0..28");
    stats(&di, "directory panels 28..32");

    // Coverage summary per layer.
    let per = atlas.width as usize * atlas.height as usize;
    let mut min_cov = f64::MAX;
    let mut min_layer = 0usize;
    for l in 0..atlas.layers as usize {
        let lay = &atlas.rgba[l * per * 4..(l + 1) * per * 4];
        let nz = lay.chunks_exact(4).filter(|p| p[0] != 0).count();
        let cov = nz as f64 / per as f64;
        if cov < min_cov {
            min_cov = cov;
            min_layer = l;
        }
    }
    println!(
        "min R coverage: layer {min_layer} at {:.1}%",
        min_cov * 100.0
    );

    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/scratch/atlas");
    fs::create_dir_all(&out_dir).expect("create docs/scratch/atlas");

    let layers: Vec<u32> = if args.is_empty() {
        // Mosaic panel 0 (A/B), mid mosaic 13, directory panel 28 (A/B), directory panel 31 (A/B).
        vec![0, 13, 28, 31, 32, 45, 60, 63]
    } else {
        args.iter()
            .map(|a| a.parse().expect("layer index"))
            .collect()
    };
    for &l in &layers {
        if l >= atlas.layers {
            eprintln!("layer {l} out of range (0..{})", atlas.layers);
            continue;
        }
        let path = out_dir.join(format!("layer_{l}.png"));
        let lay = &atlas.rgba[l as usize * per * 4..(l as usize + 1) * per * 4];
        write_png(&path, lay);
        println!("wrote {}", path.display());
    }
}

fn print_font_metrics(name: &str, f: &fontdue::Font, px: f32) {
    if let Some(lm) = f.horizontal_line_metrics(px) {
        println!(
            "{name}: ascent {:.2} descent {:.2} line-gap {:.2} new-line {:.2}",
            lm.ascent, lm.descent, lm.line_gap, lm.new_line_size
        );
    }
}

/// Encode one layer: tint block interiors by id, glyphs brighten toward white.
fn write_png(path: &Path, lay: &[u8]) {
    let mut rgb = Vec::with_capacity(W * H * 3);
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) * 4;
            let (glyph, id, a) = (lay[i], lay[i + 2], lay[i + 3]);
            let (mut r, mut g, mut b) = (0.0f32, 0.0f32, 0.0f32);
            if a != 0 {
                let (tr, tg, tb) = TINTS[(id as usize) % TINTS.len()];
                r = tr * 255.0;
                g = tg * 255.0;
                b = tb * 255.0;
                if glyph != 0 {
                    let k = glyph as f32 / 255.0;
                    r = r * (1.0 - k) + 235.0 * k;
                    g = g * (1.0 - k) + 250.0 * k;
                    b = b * (1.0 - k) + 255.0 * k;
                }
            }
            rgb.push(r.min(255.0) as u8);
            rgb.push(g.min(255.0) as u8);
            rgb.push(b.min(255.0) as u8);
        }
    }
    let file = fs::File::create(path).expect("create png");
    let w = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(w, W as u32, H as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().expect("png header");
    writer.write_image_data(&rgb).expect("png data");
}
