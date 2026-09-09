// gibson-render shaders/floor.wgsl
// One enormous quad at y = 0 spanning +/- (grid * 15 + 600). The fragment shader decodes the
// PCB tile: each 2.5-unit cell is fetched (nearest, never filtered) from the 96x96 floor map and
// an analytic SDF to that cell's half-segments / pads / vias / chip rectangle is evaluated, so
// the traces come out as even-gauge rounded-corner Manhattan routes. Traces stop with a pad at
// the tower keep-out boundaries; the substrate between features stays true black (no wash).
// Everything is emissive; fog fades the floor to black with FOG_END.
//
// WGSL note: `fwidth` may only appear in uniform control flow, so every derivative call below
// sits at the top level of the fragment shader (never inside a data-dependent branch); branch
// effects are expressed with `select` instead.

struct FrameUniform {
    view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    tower_body: vec4<f32>,
    tower_text: vec4<f32>,
    highlight: vec4<f32>,
    floor_trace: vec4<f32>,
    floor_pad: vec4<f32>,
    pulse: vec4<f32>,
    haze: vec4<f32>,
    time_fog_grid: vec4<f32>,
    resolution: vec4<f32>,
    fx: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: FrameUniform;
@group(0) @binding(3) var floor_tex: texture_2d<f32>;

const CELL: f32 = 2.5; // world units per cell
const CELLS: f32 = 96.0; // cells per tile edge
const HALF: f32 = 1.25; // half cell in world units
const TRACE_W: f32 = 0.6; // trace width in world units (even gauge, ~0.24 of a cell)

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

// Distance from p to the segment a..b (a, b in world units relative to the cell center).
fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let denom = dot(ab, ab);
    let tt = select(0.0, dot(p - a, ab) / denom, denom > 1e-8);
    let t = clamp(tt, 0.0, 1.0);
    return length(p - (a + ab * t));
}

// Signed box distance (negative inside); used for chip rectangles.
fn box_dist(p: vec2<f32>, half: vec2<f32>) -> f32 {
    let q = abs(p) - half;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
}

// Reads the G byte (feature type) of a wrapped neighbor cell.
fn neighbor_g(cx: i32, cz: i32, dx: i32, dz: i32) -> u32 {
    let nx = ((cx + dx) % 96 + 96) % 96;
    let nz = ((cz + dz) % 96 + 96) % 96;
    let n = textureLoad(floor_tex, vec2<i32>(nx, nz), 0);
    return u32(min(n.g * 255.0 + 0.5, 255.0));
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
    let cxm = wrap_cell(fcell.x);
    let czm = wrap_cell(fcell.y);
    let cx = i32(cxm);
    let cz = i32(czm);
    // Local position within the cell, in world units, origin at the cell center.
    let lp = (fract(pc) - vec2<f32>(0.5)) * CELL;

    let cell = textureLoad(floor_tex, vec2<i32>(cx, cz), 0);
    let rbyte = u32(min(cell.r * 255.0 + 0.5, 255.0));
    let gb = u32(min(cell.g * 255.0 + 0.5, 255.0));
    let brightness = cell.a; // A byte 128..=255 normalized

    // --- Traces: nearest half-segment leaving this cell's center (unset directions are
    // 1e9 = "not present"). All SDF work and all derivative calls happen unconditionally. ---
    let dseg = min(
        min(
            seg_dist(lp, vec2<f32>(0.0), vec2<f32>(HALF, 0.0)),
            seg_dist(lp, vec2<f32>(0.0), vec2<f32>(-HALF, 0.0)),
        ),
        min(
            seg_dist(lp, vec2<f32>(0.0), vec2<f32>(0.0, HALF)),
            seg_dist(lp, vec2<f32>(0.0), vec2<f32>(0.0, -HALF)),
        ),
    );
    let aaseg = fwidth(dseg);
    let hw = TRACE_W * 0.5;
    // Crisp anti-aliased conductor edge.
    let trace = 1.0 - smoothstep(hw - aaseg, hw + aaseg, dseg);
    let trace_present = select(0.0, 1.0, rbyte != 0u);
    // A tight halo that hugs the trace edge (falls to nothing within ~1 trace width) keeps the
    // traces luminous without washing the black substrate between the bus runs.
    let halo = 0.22 * exp(-3.5 * max(dseg - hw, 0.0));
    // Trace color shaping: the frozen palette stores a hot wide-gamut value that ACES would
    // wash to pale lavender; the film trace is a deep violet (#5B3FE8), so rebalance toward the
    // blue and keep the core under the bloom threshold so traces read crisp on black substrate
    // (only pads and chip rims bloom). The halo term adds glow just off the conductor edge.
    let trace_col = u.floor_trace.rgb * vec3<f32>(0.30, 0.24, 0.50);
    var col = (trace * 1.05 + halo * (1.0 - trace)) * trace_col * brightness * trace_present;

    // --- Pads (G=1) and vias (G=2): terminations read as distinct board features. ---
    let dp = length(lp);
    let aap = fwidth(dp);
    let is_pad = select(0.0, 1.0, gb == 1u);
    let is_via = select(0.0, 1.0, gb == 2u);
    // Pad: solid bright disc with a hotter centre and a crisp edge; a faint halo only just
    // past the rim so a trace ending at a keep-out boundary lands on a clean round pad.
    // Pads stay brighter than traces (they read as pale-lavender nodes in the film).
    let pad_col = u.floor_pad.rgb * vec3<f32>(0.30, 0.26, 0.50);
    let pad_disc = 1.0 - smoothstep(0.98 - aap, 0.98 + aap, dp);
    let pad_core = 1.0 - smoothstep(0.5 - aap, 0.5 + aap, dp);
    let pad_halo = 0.4 * exp(-3.0 * max(dp - 0.98, 0.0));
    col += (pad_disc * (1.0 + 0.4 * pad_core) + pad_halo) * pad_col * brightness * is_pad;
    // Via: bright copper ring around the dark drilled hole (substrate shows through).
    let ring = 1.0 - smoothstep(0.24 - aap, 0.24 + aap, abs(dp - 0.66));
    let via_halo = 0.5 * exp(-3.0 * max(dp - 0.9, 0.0));
    col += (ring * 1.4 + via_halo) * pad_col * brightness * is_via;

    // --- Chips (G=3): inset rectangle per cell, dark epoxy body with a bright rim only on the
    // chip's outer boundary (a cell whose neighbor is not part of the chip). ---
    let np_xp = neighbor_g(cx, cz, 1, 0);
    let np_xm = neighbor_g(cx, cz, -1, 0);
    let np_zp = neighbor_g(cx, cz, 0, 1);
    let np_zm = neighbor_g(cx, cz, 0, -1);
    let aachip = fwidth(lp.x) + fwidth(lp.y);
    let dbox = box_dist(lp, vec2<f32>(HALF, HALF));
    let chip_cov = 1.0 - smoothstep(-aachip, aachip, dbox);
    let band = 0.3;
    let edge_xp = 1.0 - smoothstep(band - aachip, band + aachip, HALF - lp.x);
    let edge_xm = 1.0 - smoothstep(band - aachip, band + aachip, lp.x + HALF);
    let edge_zp = 1.0 - smoothstep(band - aachip, band + aachip, HALF - lp.y);
    let edge_zm = 1.0 - smoothstep(band - aachip, band + aachip, lp.y + HALF);
    let is_edge = select(0.0, 1.0, np_xp != 3u) * edge_xp
        + select(0.0, 1.0, np_xm != 3u) * edge_xm
        + select(0.0, 1.0, np_zp != 3u) * edge_zp
        + select(0.0, 1.0, np_zm != 3u) * edge_zm;
    let is_chip = select(0.0, 1.0, gb == 3u);
    let dchip = max(min(abs(lp.x) - (HALF - 0.3), abs(lp.y) - (HALF - 0.3)), 0.0);
    let glow_c = 0.4 * exp(-1.8 * min(dchip + 0.5, 8.0));
    col += trace_col * 0.9 * chip_cov * brightness * is_chip;
    col += pad_col * (is_edge * 1.2 + glow_c * 0.6) * chip_cov * brightness * is_chip;

    col = col * fog;
    return vec4<f32>(col, 1.0);
}
