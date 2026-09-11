//! Dev-only ASCII dump of a generated floor tile, for eyeballing the PCB look.
//!
//! Usage: `cargo run -p gibson-floor --example floor_dump -- [seed] [span] [x0] [z0]`
//!
//! Prints a copper summary plus the top-left `span x span` corner.
//!
//! Legend (feature kinds win over trace shapes, top of the list first):
//! - `#` IC body, `"` IC pin, `=` SMD land, `O` through-hole pad, `o` via,
//!   `@` mounting hole, `%` solid copper pour, `:` hatched copper pour, `~` silkscreen
//! - `.` bare substrate
//! - `-` horizontal run, `|` vertical run, `+` orthogonal corner or crossing,
//!   `\` 45-degree run on the (+x+z) / (-x-z) diagonal, `/` 45-degree run on the
//!   (-x+z) / (+x-z) diagonal, `*` orthogonal and diagonal in the same cell,
//!   `X` both diagonal slopes in the same cell

use gibson_floor::generate;

fn glyph(c: &[u8; 4]) -> char {
    match c[1] {
        3 => '#',
        5 => '"',
        4 => '=',
        1 => 'O',
        2 => 'o',
        8 => '@',
        6 => {
            if c[2] & 0b1000 != 0 {
                ':'
            } else {
                '%'
            }
        }
        7 => '~',
        _ => {
            let orth = c[0] & 0b0000_1111 != 0;
            let back = c[0] & 0b1001_0000 != 0; // 16 = +x+z, 128 = -x-z
            let slash = c[0] & 0b0110_0000 != 0; // 32 = -x+z, 64 = +x-z
            if orth && (back || slash) {
                '*'
            } else if back && slash {
                'X'
            } else if orth {
                match (c[0] & 0b0011 != 0, c[0] & 0b1100 != 0) {
                    (true, true) => '+',
                    (true, false) => '-',
                    (false, true) => '|',
                    (false, false) => '.',
                }
            } else if back {
                '\\'
            } else if slash {
                '/'
            } else {
                '.'
            }
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let seed: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let span: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(48);
    let x0: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let z0: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let f = generate(seed);
    let n = f.cells as usize;
    assert!(span <= n, "span must be <= {n}");
    let cells = f.data.len() as f64;

    let mut traced = 0usize;
    let mut diagonal = 0usize;
    let mut kinds = [0usize; 9];
    let mut gauge = [0usize; 3];
    let mut bus = 0usize;
    let mut hatched = 0usize;
    for c in &f.data {
        if c[0] != 0 {
            traced += 1;
            if c[0] & 0b1111_0000 != 0 {
                diagonal += 1;
            }
        }
        kinds[c[1].min(8) as usize] += 1;
        if c[1] == 6 && c[2] & 0b1000 != 0 {
            hatched += 1;
        }
        if c[0] != 0 {
            gauge[(c[2] & 0b11) as usize] += 1;
            if c[2] & 0b100 != 0 {
                bus += 1;
            }
        }
    }
    println!(
        "seed {seed}: {n}x{n} toroidal tile ({} cells)",
        cells as usize
    );
    println!(
        "copper: {traced} cells ({:.1}%), of which {diagonal} carry a 45-degree half-segment \
         ({:.1}% of copper)",
        100.0 * traced as f64 / cells,
        100.0 * diagonal as f64 / traced.max(1) as f64
    );
    println!("bus-flagged copper cells: {bus}");
    println!(
        "gauge: thin {} ({}%), medium {} ({}%), power {} ({}%)",
        gauge[0],
        100 * gauge[0] / traced.max(1),
        gauge[1],
        100 * gauge[1] / traced.max(1),
        gauge[2],
        100 * gauge[2] / traced.max(1)
    );
    let names = [
        "none", "pad", "via", "ic-body", "smd", "pin", "pour", "silk", "hole",
    ];
    let summary: Vec<String> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let extra = if i == 6 {
                format!(" [solid {}, hatched {hatched}]", kinds[6] - hatched)
            } else {
                String::new()
            };
            format!(
                "{name} {} ({:.1}%){extra}",
                kinds[i],
                100.0 * kinds[i] as f64 / cells
            )
        })
        .collect();
    println!("{}", summary.join(", "));
    println!("{span}x{span} window at ({x0}, {z0}):");
    for zz in 0..span {
        let mut line = String::with_capacity(span);
        for xx in 0..span {
            let x = (x0 + xx) % n;
            let z = (z0 + zz) % n;
            line.push(glyph(&f.data[z * n + x]));
        }
        println!("{line}");
    }
}
