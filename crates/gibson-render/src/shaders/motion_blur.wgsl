// gibson-render shaders/motion_blur.wgsl
// Full-resolution motion blur by depth reprojection: reconstruct the world position of each
// pixel from the depth buffer, reproject it with the previous view-projection, and smear the
// HDR color along the resulting screen-space velocity (8 taps, clamped to 20 px). Background
// pixels (depth >= 1.0) pass through unchanged.

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
@group(0) @binding(1) var color_tex: texture_2d<f32>;
@group(0) @binding(2) var color_smp: sampler;
@group(0) @binding(3) var depth_tex: texture_depth_2d;

const MAX_VEL_PX: f32 = 20.0;
const TAPS: i32 = 8;

struct VsIn {
    @location(0) pos: vec2<f32>,
}
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
struct FsIn {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(in.pos, 0.0, 1.0);
    out.uv = vec2<f32>(in.pos.x * 0.5 + 0.5, 0.5 - in.pos.y * 0.5);
    return out;
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let depth = textureLoad(depth_tex, vec2<i32>(in.pos.xy), 0);
    if (depth >= 1.0) {
        return textureSampleLevel(color_tex, color_smp, uv, 0.0);
    }
    // Reconstruct the world position (zero-to-one depth range).
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - 2.0 * uv.y, depth);
    let clip_world = u.inv_view_proj * vec4<f32>(ndc, 1.0);
    let world = clip_world.xyz / clip_world.w;

    // Where was that world point last frame?
    let prev_clip = u.prev_view_proj * vec4<f32>(world, 1.0);
    if (prev_clip.w <= 0.0) {
        return textureSampleLevel(color_tex, color_smp, uv, 0.0);
    }
    let prev_ndc = prev_clip.xy / prev_clip.w;

    // Screen-space velocity (current - previous), scaled by the user amount, clamped to 20 px.
    let ndc_now = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - 2.0 * uv.y);
    var vel_px = (ndc_now - prev_ndc) * 0.5 * u.resolution.xy * u.fx.y;
    let vel_len = length(vel_px);
    if (vel_len > MAX_VEL_PX) {
        vel_px = vel_px * (MAX_VEL_PX / vel_len);
    }
    let vel_uv = vel_px / u.resolution.xy;

    var sum = vec3<f32>(0.0);
    for (var i = 0; i < TAPS; i = i + 1) {
        let off = vel_uv * (f32(i) / f32(TAPS - 1) - 0.5);
        sum = sum + textureSampleLevel(color_tex, color_smp, uv + off, 0.0).rgb;
    }
    return vec4<f32>(sum / f32(TAPS), 1.0);
}
