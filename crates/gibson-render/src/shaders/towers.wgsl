// gibson-render shaders/towers.wgsl
// Instanced translucent tower boxes. Five faces per tower (four sides + top); depth write off,
// cull off so back faces bleed text through the glass; premultiplied-alpha blending. The unit
// box is mapped to [0, instance height] in y so one geometry serves towers of every height
// (TOWER_HEIGHT_MIN..=TOWER_HEIGHT). Side faces are split into vertical bands of constant texel
// density: bands = round(height / 38), each band covering an equal slice of the face and
// sampling a different panel layer `(face_layer + band * 7) % 32`, so text stays the same size
// on short and tall towers and nothing repeats vertically.
//
// All block animation (variant cycling, top-down redraw wipes, sweeping highlight bars) is
// computed here from the atlas channels: R = glyph coverage, G = block-local v (0 top ..= 1
// bottom), B = block id, A = block coverage. Layer p is panel p variant A, layer p+32 variant B;
// a block flips variants every time its cycle counter increments.
//
// WebGL2 notes: no fwidth / implicit-LOD texture samples inside non-uniform control flow; the
// resolved layer index is sampled with one explicit-LOD textureSampleLevel call.

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
@group(0) @binding(1) var atlas_tex: texture_2d_array<f32>;
@group(0) @binding(2) var atlas_smp: sampler;

const ATLAS_W: i32 = 256;
const ATLAS_H: i32 = 768;
const TOWER_W: f32 = 6.0; // half width / half depth of a tower
// Panel height in world units one band covers on a tower of the reference height.
const BAND_UNITS: f32 = 38.0;
const PANELS: u32 = 32u;
// Band-to-panel stride: consecutive bands read panels (face_layer + band*7) % 32, which walks
// the whole panel ring in 32/7 steps and keeps adjacent bands on unrelated panels.
const BAND_STEP: u32 = 7u;

// Deterministic scalar hashes (no sin; WebGL2-safe mediump-friendly).
fn hash1(p: f32) -> f32 {
    var q = fract(p * 0.1031);
    q = q * (q + 33.33);
    return fract(q * (q + 91.77));
}
fn hash2(a: f32, b: f32) -> f32 {
    var p3 = vec3<f32>(a, b, a) * 0.1031;
    p3 = fract(p3);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

struct VsIn {
    @location(0) pos: vec3<f32>, // local unit box, -1..1
    @location(1) face: u32,      // 0..3 = +x,-x,+z,-z; 4 = top
    @location(2) uv: vec2<f32>,
    @location(3) inst_pos: vec3<f32>,
    @location(4) anim_phase: f32,
    @location(5) face_layers: vec4<u32>,
    @location(6) top_layer: u32,
    @location(7) highlight_block: u32,
    @location(8) highlight_t: f32,
    @location(9) height: f32,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) face: u32,
    @location(3) @interpolate(flat) face_layers: vec4<u32>,
    @location(4) @interpolate(flat) top_layer: u32,
    @location(5) @interpolate(flat) highlight_block: u32,
    @location(6) @interpolate(flat) highlight_t: f32,
    @location(7) @interpolate(flat) anim_phase: f32,
    @location(8) @interpolate(flat) height: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    // Map the unit box onto [base_y, base_y + height]: local y = -1 at the floor, +1 at the top.
    let h = max(in.height, 1.0);
    let wy = (in.pos.y * 0.5 + 0.5) * h;
    let world = vec3<f32>(in.pos.x * TOWER_W, wy, in.pos.z * TOWER_W) + in.inst_pos;
    var out: VsOut;
    out.clip = u.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.uv = in.uv;
    out.face = in.face;
    out.face_layers = in.face_layers;
    out.top_layer = in.top_layer;
    out.highlight_block = in.highlight_block;
    out.highlight_t = in.highlight_t;
    out.anim_phase = in.anim_phase;
    out.height = h;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = u.time_fog_grid.x;
    let cam = u.camera_pos.xyz;
    let dist = distance(in.world, cam);

    // Vertical band count for this face: constant-texel-density slices. Only the sides get
    // banded; the top face is one fixed-size band of its own panel.
    let bands = select(1u, max(1u, u32(round(in.height / BAND_UNITS))), in.face != 4u);
    // Band index: 0 at the tower top .. bands-1 at the floor (uv.y = 0 top ..= 1 bottom).
    let bf = in.uv.y * f32(bands);
    let band = u32(min(floor(bf), f32(bands - 1u)));

    // Glass body: opacity with a subtle vertical gradient (brighter toward the top).
    let vgrad = clamp(in.world.y / in.height, 0.0, 1.0);
    var col = u.tower_body.rgb * u.tower_body.a * (0.8 + 0.4 * vgrad);
    if (in.face == 4u) {
        // Overhead the glass roof reads teal-green (film #3EE8C8) instead of the deep blue body.
        col = mix(col, vec3<f32>(0.20, 0.92, 0.80) * u.tower_body.a * (0.8 + 0.4 * vgrad), 0.55);
    }

    // Sampling uv. A side band maps the full panel height over its slice of the face, so the
    // texel density (and therefore the text size) is the same on every tower. The top face
    // reads only the top third of its layer (256 of 768 rows).
    var suv: vec2<f32>;
    var rim = 1.0 - smoothstep(0.0, 0.04, min(in.uv.x, 1.0 - in.uv.x));
    if (in.face == 4u) {
        suv = vec2<f32>(in.uv.x, in.uv.y / 3.0);
        let rim_v = 1.0 - smoothstep(0.0, 0.04, min(in.uv.y * 3.0, 1.0 - in.uv.y * 3.0));
        rim = max(rim, rim_v);
    } else {
        suv = vec2<f32>(in.uv.x, bf - f32(band));
        // Bright glass edge along the top rim of the box (world-space constant, scales with the
        // face, so a taller tower shows the same world-space edge thickness).
        let top_rim = 1.0 - smoothstep(0.0, 0.02, in.uv.y);
        rim = max(rim, top_rim);
    }
    col += u.tower_text.rgb * 0.5 * rim;

    // Which panel does this face show? Sides offset by the band so every band of a tall face
    // carries different text (no vertical repetition); the top face keeps its own layer.
    var panel: u32;
    if (in.face == 4u) {
        panel = in.top_layer;
    } else {
        panel = (in.face_layers[in.face] + band * BAND_STEP) % PANELS;
    }

    // Read the block metadata from the band's base layer (variant A): the G/B/A planes are
    // identical across variants, so this texel tells us the animation state at this pixel.
    let icoord = vec2<i32>(
        min(i32(floor(suv.x * 256.0)), ATLAS_W - 1),
        min(i32(floor(suv.y * 768.0)), ATLAS_H - 1),
    );
    let base = textureLoad(atlas_tex, icoord, 0, i32(panel));
    let b255 = base.b * 255.0;
    let bid = u32(min(b255 + 0.5, 255.0));
    let ph = hash1(b255 + in.anim_phase);
    let period = 6.0 + 14.0 * hash2(b255 + 1.7, in.anim_phase + 3.1);
    let cyc = floor(t / period + ph);
    let cyc_start = (cyc - ph) * period;

    // Top-down redraw wipe: block becomes visible from its top edge downward. Blocks in
    // different bands use different panel ids, so their phases are unrelated: a tall tower's
    // text never redraws in lockstep.
    var visible = step(base.g, (t - cyc_start) / 0.35);
    // Sweeping highlight bar on some blocks, one pass every 2 s per cycle.
    if (hash2(b255 + 5.9, cyc + 11.3) < 0.35) {
        let sweep = fract((t - cyc_start) / 2.0);
        if (abs(base.g - sweep) < 0.04) {
            visible = visible * 2.5;
        }
    }

    // Effective layer flips A/B every cycle (parity of the cycle counter).
    let parity = u32(cyc) & 1u;
    let layer = panel + 32u * parity;

    // One explicit-LOD sample after the layer is fully resolved (non-uniform layer index).
    let samp = textureSampleLevel(atlas_tex, atlas_smp, suv, i32(layer), 0.0);
    let glyph = samp.r * visible;
    col += u.tower_text.rgb * glyph;

    // Selected-block highlight: glyph pixels blend toward the highlight color and the whole
    // block gets a translucent fill.
    if (bid == in.highlight_block && bid > 0u) {
        let glyph_mask = smoothstep(0.001, 0.06, samp.r);
        col = mix(col, u.highlight.rgb, in.highlight_t * glyph_mask);
        col += u.highlight.rgb * samp.a * 0.5 * in.highlight_t;
    }

    // Blue haze toward the distance, then fog out (fog multiplies color AND alpha).
    col = mix(col, u.haze.rgb * 0.6, smoothstep(60.0, 500.0, dist));
    let fog = 1.0 - smoothstep(u.time_fog_grid.y, u.time_fog_grid.z, dist);
    col = col * fog;
    return vec4<f32>(col, u.tower_body.a * fog);
}
