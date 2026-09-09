//! Dev-only ASCII dump of a generated floor tile, for eyeballing the PCB look.
//!
//! Usage: `cargo run -p gibson-floor --example floor_dump -- [seed] [span]`
//!
//! Prints a coverage summary plus the top-left `span x span` corner. Legend:
//! `.` empty, `-` horizontal segment, `|` vertical segment, `+` corner or crossing,
//! `O` pad, `o` via, `#` chip body.

use gibson_floor::generate;

fn glyph(c: &[u8; 4]) -> char {
    match c[1] {
        3 => '#',
        1 => 'O',
        2 => 'o',
        _ => {
            let horizontal = c[0] & 0b0011 != 0;
            let vertical = c[0] & 0b1100 != 0;
            match (horizontal, vertical) {
                (true, true) => '+',
                (true, false) => '-',
                (false, true) => '|',
                (false, false) => '.',
            }
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let seed: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let span: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(48);
    let f = generate(seed);
    let n = f.cells as usize;
    assert!(span <= n, "span must be <= {n}");
    let cells = f.data.len();
    let traced = f.data.iter().filter(|c| c[0] != 0).count();
    let pads = f.data.iter().filter(|c| c[1] == 1).count();
    let vias = f.data.iter().filter(|c| c[1] == 2).count();
    let chips = f.data.iter().filter(|c| c[1] == 3).count();
    println!(
        "seed {seed}: {n}x{n} toroidal tile, {:.1}% of cells carry a trace segment",
        100.0 * traced as f64 / cells as f64
    );
    println!("pads: {pads}, vias: {vias}, chip-body cells: {chips}");
    println!("{span}x{span} top-left corner:");
    for z in 0..span {
        let mut line = String::with_capacity(span);
        for x in 0..span {
            line.push(glyph(&f.data[z * n + x]));
        }
        println!("{line}");
    }
}
