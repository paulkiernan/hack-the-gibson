// gibson-render shaders/floor.wgsl
// One enormous quad at y = 0 spanning +/- (grid * 15 + 600). The fragment shader decodes one PCB
// tile: each 2.5-unit cell is fetched (nearest, never filtered) from the 96x96 floor map and an
// analytic SDF over that cell's features is evaluated, so the copper comes out as even-gauge
// rounded-corner routes rather than a grid of stubs.
//
// Cell encoding (see `FloorMap` in gibson-types):
//   R = direction mask of half-segments leaving the cell centre. Bits 0-3 orthogonal
//       (+x, -x, +z, -z), bits 4-7 diagonals (+x+z, -x+z, +x-z, -x-z). The diagonals are what let
//       the board turn a corner at 45 degrees instead of stair-stepping.
//   G = feature kind: 0 none, 1 through-hole pad, 2 via, 3 IC body, 4 SMD pad, 5 IC pin,
//       6 copper pour, 7 silkscreen, 8 mounting hole. Above 8 is reserved and must draw nothing.
//   B = bits 0-1 trace gauge class (0 thin, 1 medium, 2 thick power/ground), bit 2 bus bundle,
//       bit 3 hatched pour, bits 4-7 reserved.
//   A = brightness.
//
// Everything is emissive over a true-black substrate: the pass has no blending, so what this
// shader returns *is* the pixel. Fog fades the whole floor to black with FOG_END.
//
// WGSL note: `fwidth` may only appear in uniform control flow, so every derivative call below
// sits at the top level of the fragment shader (never inside a data-dependent branch); branch
// effects are expressed with `select` instead.

struct FrameUniform {
    view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    tower_body_normal: vec4<f32>,
    tower_text_normal: vec4<f32>,
    highlight_normal: vec4<f32>,
    floor_trace: vec4<f32>,
    floor_pad: vec4<f32>,
    pulse: vec4<f32>,
    haze: vec4<f32>,
    time_fog_grid: vec4<f32>,
    resolution: vec4<f32>,
    fx: vec4<f32>,
    post: vec4<f32>,
    tower_body_siege: vec4<f32>,
    tower_text_siege: vec4<f32>,
    highlight_siege: vec4<f32>,
    signal: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: FrameUniform;
@group(0) @binding(3) var floor_tex: texture_2d<f32>;

const CELL: f32 = 2.5; // world units per cell
const CELLS: f32 = 96.0; // cells per tile edge
const HALF: f32 = 1.25; // half cell in world units
// Trace half-widths by gauge class: thin signal, medium, thick power/ground. A trace's gauge is
// the wider of the cell it leaves and the cell it runs into, so a joint between two gauges
// widens smoothly instead of stepping at the cell edge.
const HW_THIN: f32 = 0.2;
const HW_MED: f32 = 0.3;
const HW_THICK: f32 = 0.44;
const FAR_SEG: f32 = 1.0e5; // SDF substitute for a half-segment that is not present

struct VsIn {
    @location(0) corner: vec2<f32>, // world-space x and z of a quad corner (y = 0)
}
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let world = vec3<f32>(in.corner.x, 0.0, in.corner.y);
    var out: VsOut;
    out.clip = u.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    return out;
}

// Modulo that is well defined for negative inputs.
fn wrap_cell(v: f32) -> f32 {
    return v - floor(v / CELLS) * CELLS;
}

// The wrapped cell at (cx + dx, cz + dz), so the tile is toroidal in every direction.
fn cell_at(cx: i32, cz: i32, dx: i32, dz: i32) -> vec4<f32> {
    let nx = ((cx + dx) % 96 + 96) % 96;
    let nz = ((cz + dz) % 96 + 96) % 96;
    return textureLoad(floor_tex, vec2<i32>(nx, nz), 0);
}

fn byte_of(v: f32) -> u32 {
    return u32(min(v * 255.0 + 0.5, 255.0));
}

/// Trace half-width for a cell's gauge class (0 thin, 1 medium, 2 thick; 3 clamps to thick).
fn gauge_hw(b: u32) -> f32 {
    let k = b & 3u;
    return select(select(HW_THIN, HW_MED, k == 1u), HW_THICK, k >= 2u);
}

// Distance from p to the segment a..b (a, b in world units relative to the cell center).
fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let denom = dot(ab, ab);
    let tt = select(0.0, dot(p - a, ab) / denom, denom > 1e-8);
    let t = clamp(tt, 0.0, 1.0);
    return length(p - (a + ab * t));
}

// Signed box distance (negative inside); used for pads, pins and chip bodies.
fn box_dist(p: vec2<f32>, half: vec2<f32>) -> f32 {
    let q = abs(p) - half;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
}

// Coverage of a stroked band: 1 deep inside the shape, 0 outside, `aa` wide edge.
fn cover(d: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(-aa, aa, d);
}

// A thin outline of the box `half` inset by `w`, `w` wide, antialiased by `aa`.
fn outline(d: f32, w: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(-aa, aa, abs(d) - w);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let wp = in.world;
    let cam = u.camera_pos.xyz;
    let dist = distance(wp, cam);
    let fog = 1.0 - smoothstep(u.time_fog_grid.y, u.time_fog_grid.z, dist);

    // Cell-space position of this fragment.
    let pc = wp.xz / CELL;
    let fcell = floor(pc);
    let cx = i32(wrap_cell(fcell.x));
    let cz = i32(wrap_cell(fcell.y));
    // Local position within the cell, in world units, origin at the cell center (+x, +z).
    let lp = (fract(pc) - vec2<f32>(0.5)) * CELL;

    let cell = cell_at(cx, cz, 0, 0);
    let rbyte = byte_of(cell.r);
    let gb = byte_of(cell.g);
    let bbyte = byte_of(cell.b);
    let brightness = cell.a; // A byte 128..=255 normalized
    // Reserved feature kinds draw nothing at all -- not even the traces of a cell that claims to
    // be a kind we do not know, so a future generator can never half-draw on an old renderer.
    let cell_ok = select(0.0, 1.0, gb <= 8u);

    // The eight neighbours, for gauge continuity, chip rims and pour boundaries.
    let n_px = cell_at(cx, cz, 1, 0);
    let n_nx = cell_at(cx, cz, -1, 0);
    let n_pz = cell_at(cx, cz, 0, 1);
    let n_nz = cell_at(cx, cz, 0, -1);
    let n_pp = cell_at(cx, cz, 1, 1);
    let n_np = cell_at(cx, cz, -1, 1);
    let n_pn = cell_at(cx, cz, 1, -1);
    let n_nn = cell_at(cx, cz, -1, -1);

    // --- Traces: half-segments leaving this cell's centre. Each segment's SDF is shifted by its
    // own half-width, so the min over the present segments is the distance field of the whole
    // route: crossings, junctions and 45-degree jogs all come out as one continuous conductor
    // with a filleted join at the cell centre. ---
    let hw_c = gauge_hw(bbyte);
    let hx = gauge_hw(byte_of(n_px.b));
    let hn = gauge_hw(byte_of(n_nx.b));
    let hz = gauge_hw(byte_of(n_pz.b));
    let hzz = gauge_hw(byte_of(n_nz.b));
    let hpp = gauge_hw(byte_of(n_pp.b));
    let hnp = gauge_hw(byte_of(n_np.b));
    let hpn = gauge_hw(byte_of(n_pn.b));
    let hnn = gauge_hw(byte_of(n_nn.b));
    let s1 = select(FAR_SEG, seg_dist(lp, vec2<f32>(0.0), vec2<f32>(HALF, 0.0)) - max(hw_c, hx), (rbyte & 1u) != 0u);
    let s2 = select(FAR_SEG, seg_dist(lp, vec2<f32>(0.0), vec2<f32>(-HALF, 0.0)) - max(hw_c, hn), (rbyte & 2u) != 0u);
    let s4 = select(FAR_SEG, seg_dist(lp, vec2<f32>(0.0), vec2<f32>(0.0, HALF)) - max(hw_c, hz), (rbyte & 4u) != 0u);
    let s8 = select(FAR_SEG, seg_dist(lp, vec2<f32>(0.0), vec2<f32>(0.0, -HALF)) - max(hw_c, hzz), (rbyte & 8u) != 0u);
    let s16 = select(FAR_SEG, seg_dist(lp, vec2<f32>(0.0), vec2<f32>(HALF, HALF)) - max(hw_c, hpp), (rbyte & 16u) != 0u);
    let s32 = select(FAR_SEG, seg_dist(lp, vec2<f32>(0.0), vec2<f32>(-HALF, HALF)) - max(hw_c, hnp), (rbyte & 32u) != 0u);
    let s64 = select(FAR_SEG, seg_dist(lp, vec2<f32>(0.0), vec2<f32>(HALF, -HALF)) - max(hw_c, hpn), (rbyte & 64u) != 0u);
    let s128 = select(FAR_SEG, seg_dist(lp, vec2<f32>(0.0), vec2<f32>(-HALF, -HALF)) - max(hw_c, hnn), (rbyte & 128u) != 0u);
    let dseg = min(min(min(s1, s2), min(s4, s8)), min(min(s16, s32), min(s64, s128)));

    // --- Derivatives, all at the top level: the trace edge, the round features, the rectangular
    // features and the pour hatch each get the screen-space footprint of the coordinate they
    // vary with. ---
    let aaseg = fwidth(dseg);
    let dp = length(lp);
    let aap = fwidth(dp);
    let aaxy = fwidth(lp.x) + fwidth(lp.y);
    let aah = fwidth(lp.x + lp.y);
    let aad = fwidth(lp.x - lp.y);

    // Trace colour shaping: the frozen palette stores a hot wide-gamut value that ACES would wash
    // to pale lavender; the film trace is a deep violet (#5B3FE8), so rebalance toward the blue
    // and keep the core under the bloom threshold so traces read crisp on black substrate (only
    // pads and chip rims bloom). The halo term adds glow just off the conductor edge.
    let trace_col = u.floor_trace.rgb * vec3<f32>(0.30, 0.24, 0.50);
    let pad_col = u.floor_pad.rgb * vec3<f32>(0.30, 0.26, 0.50);
    // Silkscreen is a printed legend, not copper: pale and near-neutral.
    let silk_col = vec3<f32>(0.30, 0.30, 0.30) + pad_col * 0.35;

    let trace = cover(dseg, aaseg);
    let halo = 0.22 * exp(-3.5 * max(dseg, 0.0));
    var col = (trace * 1.05 + halo * (1.0 - trace)) * trace_col * brightness;

    // --- Round features. ---
    let is_th = select(0.0, 1.0, gb == 1u);
    let is_via = select(0.0, 1.0, gb == 2u);
    let is_mount = select(0.0, 1.0, gb == 8u);
    // Through-hole pad: copper annulus with a dark drilled centre the substrate shows through.
    let th = cover(dp - 0.92, aap) * smoothstep(0.3 - aap, 0.3 + aap, dp);
    let th_flare = cover(dp - 0.5, aap) * smoothstep(0.3 - aap, 0.3 + aap, dp);
    let th_halo = 0.4 * exp(-3.0 * max(dp - 0.92, 0.0));
    col += (th * (1.0 + 0.35 * th_flare) + th_halo) * pad_col * brightness * is_th;
    // Via: small drilled ring, dimmer than a pad.
    let via = cover(dp - 0.46, aap) * smoothstep(0.16 - aap, 0.16 + aap, dp);
    col += (via * 1.1 + 0.35 * exp(-3.0 * max(dp - 0.46, 0.0))) * pad_col * brightness * is_via;
    // Mounting hole: a wide bright flange around a big hole.
    let mount = cover(dp - 1.02, aap) * smoothstep(0.55 - aap, 0.55 + aap, dp);
    let mount_rim = outline(dp - 0.78, 0.11, aap);
    col += (mount * 0.8 + mount_rim * 1.2) * pad_col * brightness * is_mount;

    // --- Rectangular features. ---
    let is_smd = select(0.0, 1.0, gb == 4u);
    let is_pin = select(0.0, 1.0, gb == 5u);
    // SMD pad: a flat rectangular land.
    let smd = cover(box_dist(lp, vec2<f32>(0.62, 0.42)), aaxy);
    col += smd * pad_col * 0.95 * brightness * is_smd;
    // IC pin: a small land elongated along the axis it connects on.
    let horiz = (rbyte & 5u) != 0u;
    let pin_half = select(vec2<f32>(0.16, 0.32), vec2<f32>(0.32, 0.16), horiz);
    let pin = cover(box_dist(lp, pin_half), aaxy);
    col += pin * pad_col * 1.05 * brightness * is_pin;

    // --- Chip body (G=3): dim epoxy rectangle with a bright rim only where the chip ends. ---
    let is_chip = select(0.0, 1.0, gb == 3u);
    let chip_cov = cover(box_dist(lp, vec2<f32>(HALF, HALF)), aaxy);
    let band = 0.3;
    let edge_band = select(0.0, 1.0, byte_of(n_px.g) != 3u) * cover(lp.x - (HALF - band), aaxy)
        + select(0.0, 1.0, byte_of(n_nx.g) != 3u) * cover(-lp.x - (HALF - band), aaxy)
        + select(0.0, 1.0, byte_of(n_pz.g) != 3u) * cover(lp.y - (HALF - band), aaxy)
        + select(0.0, 1.0, byte_of(n_nz.g) != 3u) * cover(-lp.y - (HALF - band), aaxy);
    let dchip = max(min(abs(lp.x) - (HALF - band), abs(lp.y) - (HALF - band)), 0.0);
    col += trace_col * 0.55 * chip_cov * brightness * is_chip;
    col += pad_col * (edge_band * 1.2 + 0.4 * exp(-1.8 * min(dchip + 0.5, 8.0)) * 0.6)
        * chip_cov * brightness * is_chip;

    // --- Copper pour (G=6): a plane, not a fat trace. Solid or hatched per B bit 3, with a
    // brighter boundary strip only where the pour actually ends, so neighbouring pour cells
    // merge into one continuous ground plane. ---
    let is_pour = select(0.0, 1.0, gb == 6u);
    let hatched = (bbyte & 8u) != 0u;
    let t1 = abs(fract((lp.x + lp.y) * 1.2) - 0.5);
    let t2 = abs(fract((lp.x - lp.y) * 1.2) - 0.5);
    let hatch = max(
        1.0 - smoothstep(0.06, 0.06 + aah, t1),
        1.0 - smoothstep(0.06, 0.06 + aad, t2),
    );
    let fill = select(0.42, 0.42 + 0.75 * hatch, hatched);
    let pour_edge_w = 0.12;
    let pour_edge = select(0.0, 1.0, byte_of(n_px.g) != 6u) * cover(lp.x - (HALF - pour_edge_w), aaxy)
        + select(0.0, 1.0, byte_of(n_nx.g) != 6u) * cover(-lp.x - (HALF - pour_edge_w), aaxy)
        + select(0.0, 1.0, byte_of(n_pz.g) != 6u) * cover(lp.y - (HALF - pour_edge_w), aaxy)
        + select(0.0, 1.0, byte_of(n_nz.g) != 6u) * cover(-lp.y - (HALF - pour_edge_w), aaxy);
    col += (fill + pour_edge * 0.65) * trace_col * brightness * is_pour;

    // --- Silkscreen (G=7): a pale printed outline with a pin-1 dot, the legend on top of the
    // copper. Kept thin and dim so it never competes with a trace. ---
    let is_silk = select(0.0, 1.0, gb == 7u);
    let silk_rect = outline(box_dist(lp, vec2<f32>(0.86, 0.86)), 0.055, aaxy);
    let silk_dot = cover(length(lp - vec2<f32>(-0.86, -0.86)) - 0.13, aap);
    col += (silk_rect * 0.8 + silk_dot * 0.9) * silk_col * brightness * is_silk;

    col = col * cell_ok * fog;
    return vec4<f32>(col, 1.0);
}
