// gibson-render shaders/pulses.wgsl
// Instanced lane pulse streaks: camera-facing ribbons extruded along the travel direction.
// Additive blending (One/One), depth test on, depth write off. Brightness fades head -> tail.

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

const HALF_W: f32 = 0.6; // ribbon half width in world units

struct VsIn {
    @location(0) along: f32, // 0 at tail ..= 1 at head
    @location(1) across: f32, // -1 ..= 1 across the ribbon
    @location(2) inst_pos: vec3<f32>,
    @location(3) inst_len: f32,
    @location(4) inst_dir: vec3<f32>,
    @location(5) inst_intensity: f32,
}
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) fade: f32,
    @location(1) across: f32,
    @location(2) world: vec3<f32>,
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
    let tail = head - dir * max(in.inst_len, 0.001);
    let base = mix(tail, head, in.along);
    let p = base + side * (in.across * HALF_W);
    var out: VsOut;
    out.clip = u.view_proj * vec4<f32>(p, 1.0);
    out.fade = in.along * in.inst_intensity;
    out.across = in.across;
    out.world = p;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let cam = u.camera_pos.xyz;
    let dist = distance(in.world, cam);
    let fog = 1.0 - smoothstep(u.time_fog_grid.y, u.time_fog_grid.z, dist);
    let soft = exp(-1.5 * in.across * in.across);
    let c = u.pulse.rgb * in.fade * soft * fog;
    return vec4<f32>(c, 1.0);
}
