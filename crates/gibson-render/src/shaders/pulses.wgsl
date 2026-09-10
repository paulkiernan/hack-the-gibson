// gibson-render shaders/pulses.wgsl
// Instanced laser pulses: camera-facing ribbons extruded along the travel direction, additive
// blending (One/One), depth test on, depth write off. Each beam carries its own HDR glow color;
// a hot near-white core rides a wider soft colored halo (so a green beam reads unmistakably
// green), and the brightness ramps head-to-tail so fast beams read as zipping streaks with a
// trailing tail. Beams fly anywhere from the ground lanes up to ~90 units, so nothing here
// assumes a floor height. Ribbon length/direction/position come per instance; the head is
// `inst_pos` and the tail trails `inst_len` behind it along `-dir`.

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

const RIBBON_HALF: f32 = 1.5; // ribbon half width in world units (whole beam ~3 u wide)
const CORE_SIGMA: f32 = 0.14; // hot core radius (world units)
const GLOW_SIGMA: f32 = 0.55; // colored halo radius (world units)

struct VsIn {
    @location(0) along: f32, // 0 at tail ..= 1 at head
    @location(1) across: f32, // -1 ..= 1 across the ribbon
    @location(2) inst_pos: vec3<f32>,
    @location(3) inst_len: f32,
    @location(4) inst_dir: vec3<f32>,
    @location(5) inst_intensity: f32,
    @location(6) inst_color: vec3<f32>,
}
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) along: f32,
    @location(1) across: f32,
    @location(2) world: vec3<f32>,
    @location(3) color: vec3<f32>,
    @location(4) intensity: f32,
    @location(5) len: f32,
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let head = in.inst_pos;
    let dir = normalize(in.inst_dir);
    // Side axis that keeps the ribbon normal facing the camera.
    var side = cross(dir, u.camera_pos.xyz - head);
    if (dot(side, side) < 1e-6) {
        side = cross(dir, vec3<f32>(0.0, 1.0, 0.0));
    }
    side = normalize(side);
    let len = max(in.inst_len, 0.001);
    let tail = head - dir * len;
    let base = mix(tail, head, in.along);
    let p = base + side * (in.across * RIBBON_HALF);
    var out: VsOut;
    out.clip = u.view_proj * vec4<f32>(p, 1.0);
    out.along = in.along;
    out.across = in.across;
    out.world = p;
    out.color = in.inst_color;
    out.intensity = in.inst_intensity;
    out.len = len;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let cam = u.camera_pos.xyz;
    let dist = distance(in.world, cam);
    let fog = 1.0 - smoothstep(u.time_fog_grid.y, u.time_fog_grid.z, dist);

    // Cross-beam profile: a tight hot core plus a wide colored halo, both scaled in world
    // units so the beam looks the same thickness from any distance.
    let w = in.across * RIBBON_HALF;
    let core = exp(-(w * w) / (2.0 * CORE_SIGMA * CORE_SIGMA));
    let halo = exp(-(w * w) / (2.0 * GLOW_SIGMA * GLOW_SIGMA));

    // Head-to-tail envelope: brightness concentrates at the head and decays along the tail.
    // The decay is exponential in world distance from the head with a length-relative scale,
    // so short and long beams both read as a bright head with a streaking tail.
    let len = max(in.len, 1.0);
    let head_dist = (1.0 - in.along) * len;
    let ramp = exp(-head_dist / max(0.35 * len, 2.0));
    // Soft tip: fade the last ~1.5 world units so the head never ends in a hard cap.
    let tip = 1.0 - exp(-head_dist * 0.9);
    let env = ramp * tip;

    // Luminance of the beam color; the hot core lifts toward near-white while the halo keeps
    // the saturated hue so the beam's color stays unmistakable (green dominates red/blue).
    let lum = dot(max(in.color, vec3<f32>(0.0)), vec3<f32>(0.2126, 0.7152, 0.0722));
    let hot = in.color * 1.6 + vec3<f32>(0.35 * lum); // near-white head core
    let body = mix(in.color * 0.55, hot, core); // core replaces the halo at the axis
    let c = body * (0.25 * halo + 0.95 * core) * env * in.intensity * fog;
    return vec4<f32>(c, 1.0);
}
