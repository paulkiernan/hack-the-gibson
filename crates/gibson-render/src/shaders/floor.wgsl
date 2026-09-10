// gibson-render shaders/floor.wgsl
// One enormous quad at y = 0 spanning +/- (grid * 15 + 600). The fragment shader decodes one PCB
// tile: each 2.5-unit cell is fetched (nearest, never filtered) from the 96x96 floor map and an
// analytic SDF over that cell's features is evaluated, so the copper comes out as even-gauge
// rounded-corner routes rather than a grid of stubs.
//
// Cell to world: cell index `k` is *centred* on world `2.5k` and covers `[2.5k - 1.25, 2.5k +
// 1.25)`, which is what the tower lattice is expressed in: a tower stands at world `x = 15 + 30k`,
// `z = 30k`, i.e. cell `6 + 12k`, and its 12-unit footprint then sits inside the 5x5 package body
// with the 7x7 pin ring just outside it, on every side.
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
// The trace SDF is the union of the half-segments in the cell *and* the ones the three
// neighbouring cells that meet at each of its corners own, because a 45-degree run passes through
// the point where four cells meet: the two cells flanking that point own no half-segment at all,
// yet the conductor's band runs straight through them. Without those extra segments the copper is
// pinched to zero width at every corner - a chain of lozenges - instead of one continuous line.
// Only the corner read matters: an edge neighbour's collinear continuation is already drawn by
// this cell's own mirrored half-segment.
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
// Printed silkscreen legend: half-width of a courtyard line, its clearance to the pads it runs
// between, the pin-1 marker's dot radius, and how far a bus bundle's members stand apart.
const SILK_HW: f32 = 0.075;
const SILK_INSET: f32 = 0.14;

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

// Trace gauge class of a cell's B byte, and the half-width that class draws.
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

// Signed distance to a rounded box: a land whose corners have a solder-ball radius.
fn rbox_dist(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    return box_dist(p, half - vec2<f32>(r)) - r;
}

// Coverage of a stroked band: 1 deep inside the shape, 0 outside, `aa` wide edge.
fn cover(d: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(-aa, aa, d);
}

// A thin outline of the box `half` inset by `w`, `w` wide, antialiased by `aa`.
fn outline(d: f32, w: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(-aa, aa, abs(d) - w);
}

// Distance from `q` to the copper the *other* three cells meeting at a cell corner put into this
// cell, and the brightness of whichever of them owns it: each owns one half-segment that ends
// exactly on that corner, and together with this cell's own segment to the corner they are the
// four spokes a trace turns or passes through there. `q` is the fragment's local position
// mirrored into the corner's own quadrant (both components positive, corner at (HALF, HALF)),
// `m_*`/`b_*` are the three neighbours' direction masks and the bit each of them points at the
// corner with, `hw_*` their gauges and `br_*` their brightness bytes (the fragment's own cell
// drew none of this copper, so it must not tint it). A segment's gauge is the wider of its own
// cell and the cell it runs into; in the mirrored frame the x-side neighbour's segment runs into
// the z-side neighbour and vice versa, and the diagonal neighbour's runs into this cell.
fn corner_trace(
    q: vec2<f32>,
    m_ex: u32,
    b_ex: u32,
    m_ez: u32,
    b_ez: u32,
    m_d: u32,
    b_d: u32,
    hw_c: f32,
    hw_ex: f32,
    hw_ez: f32,
    hw_d: f32,
    br_ex: f32,
    br_ez: f32,
    br_d: f32,
) -> vec2<f32> {
    let cc = vec2<f32>(HALF, HALF);
    let d_ex = select(FAR_SEG, seg_dist(q, vec2<f32>(2.0 * HALF, 0.0), cc) - max(hw_ex, hw_ez), (m_ex & b_ex) != 0u);
    let d_ez = select(FAR_SEG, seg_dist(q, vec2<f32>(0.0, 2.0 * HALF), cc) - max(hw_ez, hw_ex), (m_ez & b_ez) != 0u);
    let d_d = select(FAR_SEG, seg_dist(q, vec2<f32>(2.0 * HALF, 2.0 * HALF), cc) - max(hw_d, hw_c), (m_d & b_d) != 0u);
    var best = d_ex;
    var br = br_ex;
    let t_ez = d_ez < best;
    best = select(best, d_ez, t_ez);
    br = select(br, br_ez, t_ez);
    let t_d = d_d < best;
    return vec2<f32>(select(best, d_d, t_d), select(br, br_d, t_d));
}

// True when a neighbour cell continues the printed legend this silk cell belongs to: a courtyard
// line runs between the package's pins and stops at them, and a pin-1 marker is a lone silk cell
// with no such neighbour.
fn is_legend(g: u32) -> bool {
    return g == 5u || g == 7u;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let wp = in.world;
    let cam = u.camera_pos.xyz;
    let dist = distance(wp, cam);
    let fog = 1.0 - smoothstep(u.time_fog_grid.y, u.time_fog_grid.z, dist);

    // Cell-space position of this fragment; cell `k` is centred on world `2.5k`.
    let pc = wp.xz / CELL + vec2<f32>(0.5);
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

    // The eight neighbours, for gauge continuity, chip rims, pour boundaries, the corner segments
    // and the silkscreen legend's connectivity.
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
    var dseg = min(min(min(s1, s2), min(s4, s8)), min(min(s16, s32), min(s64, s128)));

    // The corner this fragment sits nearest, and the three half-segments the other cells meeting
    // there own. Mirrored into that corner's own quadrant the geometry is always the same, so the
    // neighbour masks, gauges and pointing bits are picked by the fragment's quadrant and one
    // evaluation covers all four corners.
    let pos_x = lp.x >= 0.0;
    let pos_z = lp.y >= 0.0;
    let q = vec2<f32>(abs(lp.x), abs(lp.y));
    let m_ex = select(byte_of(n_nx.r), byte_of(n_px.r), pos_x);
    let m_ez = select(byte_of(n_nz.r), byte_of(n_pz.r), pos_z);
    let m_d = select(
        select(byte_of(n_nn.r), byte_of(n_pn.r), pos_x),
        select(byte_of(n_np.r), byte_of(n_pp.r), pos_x),
        pos_z,
    );
    // The bit each neighbour points at the corner with. In the mirrored frame the three segments
    // always run (-1,+1), (+1,-1) and (-1,-1), so each neighbour's test bit is that direction
    // mapped back onto the world axes by the fragment's quadrant.
    let b_ex = select(select(64u, 128u, pos_x), select(16u, 32u, pos_x), pos_z);
    let b_ez = select(select(32u, 16u, pos_x), select(128u, 64u, pos_x), pos_z);
    let b_d = select(select(16u, 32u, pos_x), select(64u, 128u, pos_x), pos_z);
    let hw_ex = select(hn, hx, pos_x);
    let hw_ez = select(hzz, hz, pos_z);
    let hw_d = select(select(hnn, hpn, pos_x), select(hnp, hpp, pos_x), pos_z);
    let br_ex = select(n_nx.a, n_px.a, pos_x);
    let br_ez = select(n_nz.a, n_pz.a, pos_z);
    let br_d = select(select(n_nn.a, n_pn.a, pos_x), select(n_np.a, n_pp.a, pos_x), pos_z);
    let corner = corner_trace(q, m_ex, b_ex, m_ez, b_ez, m_d, b_d, hw_c, hw_ex, hw_ez, hw_d, br_ex, br_ez, br_d);
    // Copper at the corner belongs to a neighbour, so it carries that cell's brightness: this
    // cell's own byte says nothing about a trace it does not own (a free lane cell's is zero).
    let corner_nearest = corner.x < dseg;
    dseg = min(dseg, corner.x);
    let trace_brightness = select(brightness, corner.y, corner_nearest);

    // --- Derivatives, all at the top level: the trace edge, the round features, the rectangular
    // features and the pour hatch each get the screen-space footprint of the coordinate they
    // vary with. ---
    let dp = length(lp);
    let aap = fwidth(dp);
    let aaxy = fwidth(lp.x) + fwidth(lp.y);
    let aah = fwidth(lp.x + lp.y);
    let aad = fwidth(lp.x - lp.y);
    // The distance field substitutes a far sentinel for an absent half-segment, so it steps by
    // that sentinel where a cell boundary separates a cell that owns a segment from one that does
    // not. `aaxy` is the distance field's own Lipschitz bound (it cannot vary faster than the
    // fragment's footprint), so clamping to it caps that step at a real edge width and stops a
    // hairline of trace colour appearing along the boundary.
    let aaseg = min(fwidth(dseg), aaxy);

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
    var col = (trace * 1.05 + halo * (1.0 - trace)) * trace_col * trace_brightness;

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
    // IC pin: the visible junction between a tower and the board, so it is a proper solder land --
    // a rounded rectangle elongated along the pin's outward direction (its R bit), wider than the
    // trace that leaves it, with a brighter solder fillet at its centre and a soft skirt of
    // reflected light where the mask opening ends. The trace starts at the pin's centre, so the
    // copper is attached to the land rather than stopping short of it.
    let axis_x = (rbyte & 3u) != 0u;
    let pin_half = select(vec2<f32>(0.40, 0.62), vec2<f32>(0.62, 0.40), axis_x);
    let pin_land = cover(rbox_dist(lp, pin_half, 0.11), aaxy);
    let pin_fillet = cover(rbox_dist(lp, pin_half * 0.55, 0.06), aaxy);
    let pin_skirt = 0.3 * exp(-2.6 * max(rbox_dist(lp, pin_half, 0.11), 0.0));
    col += (pin_land * 0.95 + pin_fillet * 0.5 + pin_skirt * (1.0 - pin_land)) * pad_col * brightness * is_pin;

    // --- Chip body (G=3): the epoxy package the tower stands on. Nearly black, so the sliver of
    // it that shows outside the tower base reads as the chip's own edge rather than as a bright
    // rim: a soft chamfer highlight that ramps in over the last half-unit and stops at the
    // boundary, with no hard line where the tower base meets the board. ---
    let is_chip = select(0.0, 1.0, gb == 3u);
    let chip_cov = cover(box_dist(lp, vec2<f32>(HALF, HALF)), aaxy);
    let bevel_w = 0.55;
    let bevel_px = select(0.0, 1.0, byte_of(n_px.g) != 3u) * smoothstep(HALF - bevel_w, HALF, lp.x);
    let bevel_nx = select(0.0, 1.0, byte_of(n_nx.g) != 3u) * smoothstep(HALF - bevel_w, HALF, -lp.x);
    let bevel_pz = select(0.0, 1.0, byte_of(n_pz.g) != 3u) * smoothstep(HALF - bevel_w, HALF, lp.y);
    let bevel_nz = select(0.0, 1.0, byte_of(n_nz.g) != 3u) * smoothstep(HALF - bevel_w, HALF, -lp.y);
    let bevel = max(max(bevel_px, bevel_nx), max(bevel_pz, bevel_nz));
    col += (trace_col * 0.10 + pad_col * bevel * 0.30) * chip_cov * brightness * is_chip;

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

    // --- Silkscreen (G=7): the printed legend. A courtyard cell sits on the package's pin ring,
    // so it draws a printed line out along every axis where the ring continues (the pads and the
    // other courtyard cells), stopping just short of the pads it runs between; a cell with no
    // neighbour on the ring is the pin-1 marker, a printed dot and ring outside the corner. ---
    let is_silk = select(0.0, 1.0, gb == 7u);
    let silk_px = select(0.0, 1.0, is_legend(byte_of(n_px.g)));
    let silk_nx = select(0.0, 1.0, is_legend(byte_of(n_nx.g)));
    let silk_pz = select(0.0, 1.0, is_legend(byte_of(n_pz.g)));
    let silk_nz = select(0.0, 1.0, is_legend(byte_of(n_nz.g)));
    let silk_reach = HALF * 0.5 - SILK_INSET * 0.5;
    let line_px = silk_px * cover(box_dist(lp - vec2<f32>(HALF * 0.5, 0.0), vec2<f32>(silk_reach, SILK_HW)), aaxy);
    let line_nx = silk_nx * cover(box_dist(lp + vec2<f32>(HALF * 0.5, 0.0), vec2<f32>(silk_reach, SILK_HW)), aaxy);
    let line_pz = silk_pz * cover(box_dist(lp - vec2<f32>(0.0, HALF * 0.5), vec2<f32>(SILK_HW, silk_reach)), aaxy);
    let line_nz = silk_nz * cover(box_dist(lp + vec2<f32>(0.0, HALF * 0.5), vec2<f32>(SILK_HW, silk_reach)), aaxy);
    let legend = max(max(line_px, line_nx), max(line_pz, line_nz));
    let lone = 1.0 - max(max(silk_px, silk_nx), max(silk_pz, silk_nz));
    let marker = cover(dp - 0.32, aap) * lone;
    let marker_ring = outline(dp - 0.5, 0.055, aap) * lone;
    col += (legend * 0.85 + marker * 0.9 + marker_ring * 0.7) * silk_col * brightness * is_silk;

    col = col * cell_ok * fog;
    return vec4<f32>(col, 1.0);
}
