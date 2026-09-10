// gibson-render shaders/composite.wgsl
// Final composite: HDR color + bloom, ACES fitted tonemap, radial chromatic aberration, film
// grain and vignette.
//
// This runs at the *scene* size. When the CRT pass is active that is the signal resolution and
// the pass writes an Rgba16Float buffer for `crt.wgsl` to reconstruct onto the display; when it
// is off the scene runs at the output size and this pass writes the final target directly. The
// tube-shaped effects (barrel warp, scanlines, phosphor mask) are entirely the CRT pass's job --
// see `crt.wgsl` for why they cannot be stamped on a native-resolution image.
//
// Size-dependent terms use `u.signal` (the size this pass renders at), never `u.resolution` (the
// output size): the chromatic-aberration offset is measured in the pixels it is applied to, and
// the grain hash has to tile at the resolution being written.
//
// The target is an *UnormSrgb format so linear values are written and the hardware encodes --
// unless the target offered no sRGB format (fx.w = 1), in which case we encode manually here.

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

    // Radial chromatic aberration: 1.5 px at unit radius, scaled by r^2, in the pixels this pass
    // writes (the signal pixels when the CRT pass follows).
    let p = uv - vec2<f32>(0.5);
    let r = length(p) * 2.0;
    let dir = p / max(r, 1e-4);
    let off = dir * (1.5 * r * r) / min(u.signal.x, u.signal.y);
    let cr = textureSampleLevel(color_tex, color_smp, uv + off, 0.0).r;
    let cg = textureSampleLevel(color_tex, color_smp, uv, 0.0).g;
    let cb = textureSampleLevel(color_tex, color_smp, uv - off, 0.0).b;
    var hdr = vec3<f32>(cr, cg, cb);

    // Phosphor halation: bright areas bleed a couple of pixels horizontally on the tube.
    // Additive and gated on the center luminance so the black substrate stays black. Only runs
    // when the tube is on at all, so `crt = 0` is exactly the plain composite.
    if (u.post.x > 0.0) {
        let bleed = 0.5 * (
            textureSampleLevel(color_tex, color_smp, uv + vec2<f32>(2.2 * u.signal.z, 0.0), 0.0).rgb
            + textureSampleLevel(color_tex, color_smp, uv - vec2<f32>(2.2 * u.signal.z, 0.0), 0.0).rgb);
        let lumc = dot(hdr, vec3<f32>(0.2126, 0.7152, 0.0722));
        hdr += bleed * (0.10 * u.post.x) * smoothstep(0.9, 3.0, lumc);
    }
    hdr = hdr + textureSampleLevel(bloom_tex, color_smp, uv, 0.0).rgb * u.fx.x;

    // Vignette on the linear sum, then tonemap.
    let vig = 1.0 - 0.35 * smoothstep(0.4, 1.4, r);
    var out_c = aces_fitted(hdr * vig);

    // Film grain in gamma-ish space, weighted by local luminance so pure blacks stay clean
    // (35 mm grain lives in the midtones, not the voids). Disabled when the amount is zero.
    if (u.fx.z > 0.0) {
        let g = hash2(
            uv * u.signal.xy
                + vec2<f32>(u.time_fog_grid.x * 0.7, u.time_fog_grid.x * 1.3)
        );
        let lum = dot(out_c, vec3<f32>(0.2126, 0.7152, 0.0722));
        let weight = 0.10 + 1.5 * smoothstep(0.015, 0.35, lum);
        out_c = out_c + (g - 0.5) * 2.0 * u.fx.z * weight;
    }

    // Back to display-gamma bytes when the target will not encode them for us (fx.w = 1): the
    // surface is linear, or the CRT pass wants a gamma-space signal to linearise on fetch.
    if (u.fx.w > 0.5) {
        out_c = srgb_encode(out_c);
    }
    return vec4<f32>(out_c, 1.0);
}
