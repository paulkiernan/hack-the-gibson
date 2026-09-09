// gibson-render shaders/composite.wgsl
// Final composite to the presentable / offscreen target: HDR color + bloom, ACES fitted
// tonemap, radial chromatic aberration, film grain, vignette. The target is an *UnormSrgb
// format so linear values are written and the hardware encodes -- unless the surface offered no
// sRGB format (fx.w = 1), in which case we encode manually here. Never double-encode.

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
@group(0) @binding(1) var color_tex: texture_2d<f32>;
@group(0) @binding(2) var color_smp: sampler;
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;

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

fn hash2(p: vec2<f32>) -> f32 {
    var p3 = vec3<f32>(p.x, p.y, p.x) * 0.1031;
    p3 = fract(p3);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn aces_fitted(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn srgb_encode(x: vec3<f32>) -> vec3<f32> {
    let lo = x * 12.92;
    let hi = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(lo, hi, x > vec3<f32>(0.0031308));
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let uv = in.uv;
    // Radial chromatic aberration: 1.5 px at unit radius, scaled by r^2.
    let p = uv - vec2<f32>(0.5);
    let r = length(p) * 2.0;
    let dir = p / max(r, 1e-4);
    let off = dir * (1.5 * r * r) / min(u.resolution.x, u.resolution.y);
    let cr = textureSampleLevel(color_tex, color_smp, uv + off, 0.0).r;
    let cg = textureSampleLevel(color_tex, color_smp, uv, 0.0).g;
    let cb = textureSampleLevel(color_tex, color_smp, uv - off, 0.0).b;
    var hdr = vec3<f32>(cr, cg, cb);
    hdr = hdr + textureSampleLevel(bloom_tex, color_smp, uv, 0.0).rgb * u.fx.x;

    // Vignette on the linear sum, then tonemap.
    let vig = 1.0 - 0.35 * smoothstep(0.4, 1.4, r);
    var out_c = aces_fitted(hdr * vig);

    // Film grain in gamma-ish space; disabled when the amount is zero.
    if (u.fx.z > 0.0) {
        let g = hash2(uv * u.resolution.xy + vec2<f32>(u.time_fog_grid.x * 0.7, u.time_fog_grid.x * 1.3));
        out_c = out_c + (g - 0.5) * 2.0 * u.fx.z;
    }

    if (u.fx.w > 0.5) {
        out_c = srgb_encode(out_c);
    }
    return vec4<f32>(out_c, 1.0);
}
