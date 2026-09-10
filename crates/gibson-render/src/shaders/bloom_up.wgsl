// gibson-render shaders/bloom_up.wgsl
// 3x3 tent upsample by 2, accumulated additively onto the destination level (blend One/One).
// Nine explicit samples around the bilinear-magnified coordinate; the tent weights follow
// w(d) = 1 - |d| with d measured in destination texels.

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
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var src_smp: sampler;

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
    let src = vec2<f32>(textureDimensions(src_tex, 0));
    // Each destination texel advances half a source texel; tap (dx, dy) sits at
    // (pos + vec2(dx, dy)) / (2 * srcSize) in source UV space.
    let scale = 0.5 / src;
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let d = vec2<f32>(f32(dx), f32(dy));
            let w = (1.0 - abs(d.x) * 0.5) * (1.0 - abs(d.y) * 0.5);
            let uv = (in.pos.xy + d) * scale;
            sum = sum + textureSampleLevel(src_tex, src_smp, uv, 0.0).rgb * w;
            wsum = wsum + w;
        }
    }
    return vec4<f32>(sum / wsum, 1.0);
}
