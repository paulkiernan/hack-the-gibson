// gibson-render shaders/bloom_prefilter.wgsl
// Soft-threshold pass at half resolution: extracts the HDR brights that will bloom. The bilinear
// sample lands on the shared corner of the 2x2 source texel block this destination pixel covers
// (destination pixel i spans source texels 2i and 2i+1), so the sample is exactly the block
// average.

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
@group(0) @binding(1) var hdr_tex: texture_2d<f32>;
@group(0) @binding(2) var hdr_smp: sampler;

const THRESHOLD: f32 = 0.8;
const KNEE: f32 = 0.5;

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
    let src = vec2<f32>(textureDimensions(hdr_tex, 0));
    // Destination pixel i has center pos = i + 0.5; the 2x2 source block it covers has its
    // shared corner at source texel 2i+1 = 2 * pos.
    let uv = (in.pos.xy * 2.0) / src;
    let c = textureSampleLevel(hdr_tex, hdr_smp, uv, 0.0).rgb;
    let brightness = max(c.r, max(c.g, c.b));
    let soft = clamp(brightness - THRESHOLD + KNEE, 0.0, 2.0 * KNEE);
    let soft_c = soft * soft / (4.0 * KNEE + 1e-4);
    let contribution = max(soft_c, brightness - THRESHOLD) / max(brightness, 1e-4);
    return vec4<f32>(c * contribution, 1.0);
}
