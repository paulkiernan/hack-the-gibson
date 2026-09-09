// gibson-render shaders/bloom_down.wgsl
// 13-tap downsample by 2. The centre tap is bilinear on the shared corner of the 2x2 source
// block under the destination pixel (i.e. it already averages four texels); the ring taps sit on
// the corners of the neighbouring blocks, widening the kernel. One shared explicit-LOD sample.

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

// Tap offsets in source texels relative to the 2x2-block corner under this destination pixel.
const TAP: array<vec2<f32>, 13> = array<vec2<f32>, 13>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(-1.0, 0.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(-1.0, 1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(2.0, 0.0),
    vec2<f32>(-2.0, 0.0),
    vec2<f32>(0.0, 2.0),
    vec2<f32>(0.0, -2.0),
);
const WEIGHT: array<f32, 13> = array<f32, 13>(
    4.0,
    2.0,
    2.0,
    2.0,
    2.0,
    1.5,
    1.5,
    1.5,
    1.5,
    1.0,
    1.0,
    1.0,
    1.0,
);

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
    let center_uv = (in.pos.xy * 2.0) / src;
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0; i < 13; i = i + 1) {
        let off = TAP[i] / src;
        sum = sum + textureSampleLevel(src_tex, src_smp, center_uv + off, 0.0).rgb * WEIGHT[i];
        wsum = wsum + WEIGHT[i];
    }
    return vec4<f32>(sum / wsum, 1.0);
}
