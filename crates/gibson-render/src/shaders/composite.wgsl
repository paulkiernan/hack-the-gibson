// gibson-render shaders/composite.wgsl
// Final composite to the presentable / offscreen target: HDR color + bloom, ACES fitted
// tonemap, radial chromatic aberration, film grain, vignette -- and, when `settings.crt > 0`,
// a CRT overlay as the very last step.
//
// The CRT treatment is the tail of this single fullscreen pass rather than a dedicated final
// pass: the composite is already the last writer of the final target, so a separate pass would
// need an extra intermediate texture plus a second fullscreen draw just to read back what this
// pass wrote. Folding it in costs nothing when it is off and keeps the whole effect on the
// already-allocated color/bloom reads. It runs on the tonemapped values in display-gamma
// space:
//   - subtle barrel curvature of the sampled UV; the area outside the curved glass goes black
//     through a soft rounded bezel edge,
//   - scanlines on a fixed ~480-logical-line pitch (resolution independent) with a slow drift,
//   - a gentle RGB aperture-grille triad on a 3-pixel horizontal cycle,
//   - a horizontal phosphor smear of bright areas plus a small gamma/contrast lift so the
//     picture reads as emissive phosphor,
//   - extra darkening toward the tube corners on top of the vignette.
// Every term is scaled by `u.post.x` (the crt amount); when it is zero the whole block is
// skipped so the output is byte-identical to the plain composite.
//
// The target is an *UnormSrgb format so linear values are written and the hardware encodes --
// unless the surface offered no sRGB format (fx.w = 1), in which case we encode manually here.
// Never double-encode: the CRT block works in gamma space and decodes back to linear when the
// target will encode on store.

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
    post: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: FrameUniform;
@group(0) @binding(1) var color_tex: texture_2d<f32>;
@group(0) @binding(2) var color_smp: sampler;
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;

const SCAN_LINES: f32 = 480.0; // logical line count; pitch is resolution independent
const GR_AMP: f32 = 0.12; // aperture-grille modulation depth at crt = 1
const SCAN_AMP: f32 = 0.30; // scanline darkening depth at crt = 1
const CURVE: f32 = 0.085; // barrel curvature gain (max uv offset ~3% of the frame edge)
const GLASS_FRAC: f32 = 0.985; // tube glass fills this fraction of the raster
const CORNER_R: f32 = 0.10; // bezel corner radius (fraction of the screen height)
const EDGE_DARK: f32 = 0.5; // extra corner falloff depth at crt = 1

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

fn srgb_decode(x: vec3<f32>) -> vec3<f32> {
    let lo = x / 12.92;
    let hi = pow((x + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(lo, hi, x > vec3<f32>(0.04045));
}

// Grille stripe pass-through: 1 exactly on a phosphor stripe center, dropping to 0 between
// stripes. `dc` is the wrapped distance to the stripe center in triad cycles (0..=0.5).
fn stripe_att(dc: f32) -> f32 {
    return 1.0 - smoothstep(0.02, 0.30, dc * 3.0);
}

// Signed distance to a rounded rectangle centered at the origin. Negative inside.
fn rrect_dist(p: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - (half - vec2<f32>(radius));
    return length(max(q, vec2<f32>(0.0))) - radius;
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let crt = u.post.x;

    // Curved-screen sampling coordinate. The push is exactly zero when crt is 0 so the plain
    // composite path below is untouched.
    var suv = uv;
    if (crt > 0.0) {
        let cc = uv - vec2<f32>(0.5);
        suv = uv + cc * dot(cc, cc) * CURVE * crt;
    }

    // Radial chromatic aberration: 1.5 px at unit radius, scaled by r^2, applied on the
    // (possibly curved) sampling coordinate.
    let p = suv - vec2<f32>(0.5);
    let r = length(p) * 2.0;
    let dir = p / max(r, 1e-4);
    let off = dir * (1.5 * r * r) / min(u.resolution.x, u.resolution.y);
    let cr = textureSampleLevel(color_tex, color_smp, suv + off, 0.0).r;
    let cg = textureSampleLevel(color_tex, color_smp, suv, 0.0).g;
    let cb = textureSampleLevel(color_tex, color_smp, suv - off, 0.0).b;
    var hdr = vec3<f32>(cr, cg, cb);

    // Phosphor halation: bright areas bleed a couple of pixels horizontally on the tube.
    // Additive and gated on the center luminance so the black substrate stays black.
    if (crt > 0.0) {
        let bleed = 0.5 * (
            textureSampleLevel(color_tex, color_smp, suv + vec2<f32>(2.2 / u.resolution.x, 0.0), 0.0).rgb
            + textureSampleLevel(color_tex, color_smp, suv - vec2<f32>(2.2 / u.resolution.x, 0.0), 0.0).rgb);
        let lumc = dot(hdr, vec3<f32>(0.2126, 0.7152, 0.0722));
        hdr += bleed * (0.20 * crt) * smoothstep(0.9, 3.0, lumc);
    }
    hdr = hdr + textureSampleLevel(bloom_tex, color_smp, suv, 0.0).rgb * u.fx.x;

    // Vignette on the linear sum, then tonemap.
    let vig = 1.0 - 0.35 * smoothstep(0.4, 1.4, r);
    var out_c = aces_fitted(hdr * vig);

    // Film grain in gamma-ish space, weighted by local luminance so pure blacks stay clean
    // (35 mm grain lives in the midtones, not the voids). Disabled when the amount is zero.
    if (u.fx.z > 0.0) {
        let g = hash2(uv * u.resolution.xy + vec2<f32>(u.time_fog_grid.x * 0.7, u.time_fog_grid.x * 1.3));
        let lum = dot(out_c, vec3<f32>(0.2126, 0.7152, 0.0722));
        let weight = 0.10 + 1.5 * smoothstep(0.015, 0.35, lum);
        out_c = out_c + (g - 0.5) * 2.0 * u.fx.z * weight;
    }

    // ----- CRT overlay (display-gamma space), skipped entirely when crt == 0. -----
    var final_c = out_c;
    if (crt > 0.0) {
        var disp = srgb_encode(out_c);

        // Aperture grille: gentle RGB triad, 3-pixel horizontal cycle, per-channel stripe mask.
        let triad = fract(suv.x * u.resolution.x / 3.0);
        // Wrapped distance (in triad cycles, 0..=0.5) from each channel's stripe center.
        let d_r = min(triad, 1.0 - triad);
        let d_g = min(abs(triad - 0.33333), 1.0 - abs(triad - 0.33333));
        let d_b = min(abs(triad - 0.66667), 1.0 - abs(triad - 0.66667));
        let mask = vec3<f32>(stripe_att(d_r), stripe_att(d_g), stripe_att(d_b));
        disp *= vec3<f32>(1.0) - GR_AMP * crt * (vec3<f32>(1.0) - mask);

        // Scanlines: 480 logical lines, darkening between lines, drifting very slowly downward.
        let sfrac = fract(suv.y * SCAN_LINES - u.time_fog_grid.x * 0.12);
        let dline = min(sfrac, 1.0 - sfrac); // 0 on a line center, 0.5 between lines
        disp *= vec3<f32>(1.0) - SCAN_AMP * crt * smoothstep(0.04, 0.5, dline);

        // Tube shape: soft rounded bezel blacking the area outside the curved glass, plus
        // extra corner falloff over the whole glass (on top of the linear vignette).
        let aspect = u.resolution.x / max(u.resolution.y, 1.0);
        let n = vec2<f32>((suv.x - 0.5) * aspect, suv.y - 0.5);
        let half = vec2<f32>(0.5 * aspect * GLASS_FRAC, 0.5 * GLASS_FRAC);
        let aa = 2.0 / max(u.resolution.y, 1.0);
        let dglass = rrect_dist(n, half, CORNER_R);
        let bezel = 1.0 - smoothstep(-aa, aa, dglass);
        let rn = length(n / half);
        let corner = 1.0 - EDGE_DARK * smoothstep(0.55, 1.0, rn);
        disp *= mix(vec3<f32>(1.0), vec3<f32>(bezel * corner), crt);

        // Emissive-phosphor gamma/contrast lift, then clamp.
        disp = pow(disp, vec3<f32>(1.0 / (1.0 + 0.16 * crt)));
        disp = clamp(disp, vec3<f32>(0.0), vec3<f32>(1.0));

        // Back to linear when the target sRGB-encodes on store (fx.w = 0); when the target is
        // linear (fx.w = 1) the gamma bytes are written directly.
        if (u.fx.w <= 0.5) {
            final_c = srgb_decode(disp);
        } else {
            final_c = disp;
        }
    } else if (u.fx.w > 0.5) {
        final_c = srgb_encode(out_c);
    }
    return vec4<f32>(final_c, 1.0);
}
