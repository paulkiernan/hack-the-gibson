// gibson-render shaders/crt.wgsl
// CRT emulation, the final pass: reads the composite's output at *signal* resolution and writes
// the display at full *output* resolution.
//
// Ported from Timothy Lottes' "PUBLIC DOMAIN CRT STYLED SCAN-LINE SHADER", the shader the
// emulation community knows as `crt-lottes`. It is explicitly public domain -- the original
// header says "Please take and use, change, or whatever" -- so it is compatible with this
// project's GPL-3.0-or-later. Credit: Timothy Lottes, public domain.
// Reference GLSL: https://github.com/libretro/glsl-shaders/blob/master/crt/shaders/crt-lottes.glsl
//
// Why a separate pass: a CRT is a raster display. The picture exists as a *signal* with a fixed
// number of lines and the tube reconstructs it -- nearest fetch at signal texel centres, a
// Gaussian reconstruction filter across neighbouring texels, a Gaussian scanline weight, a
// three-line blend, a phosphor mask at *output* pixel scale and barrel warp. Scanlines only read
// as scanlines when the signal has far fewer lines than the display, which is why the whole scene
// chain runs at the smaller signal resolution (see `scene_size` in lib.rs) and this pass expands
// it. Stamping a scanline pattern over a native-resolution image cannot work: there is no signal
// to reconstruct, so the "scanlines" land on whatever pixel grid happens to be there.
//
// Ported pieces, keeping Lottes' names: `Fetch` (nearest at texel centres, scaled by
// `brightBoost` and linearised), `Dist`, `Gaus` (with the `shape` exponent), `Horz3`/`Horz5`
// horizontal reconstruction, `Scan` scanline weighting, the three-line `Tri` blend, `Warp`
// barrel distortion and `Mask` (his stretched-VGA shadow mask, `shadowMask == 3`).
// Not ported: his optional bloom (`DO_BLOOM`) -- the scene already runs a bloom chain upstream
// and a second one here would smear the phosphor mask, which is the point of this pass.
//
// Defaults are his: hardScan -8, hardPix -3, warpX 0.031, warpY 0.041, maskDark 0.5,
// maskLight 1.5, shadowMask 3, brightBoost 1, shape 2. The `crt` amount interpolates scanline
// depth (`hardScan`), mask strength (`maskDark`/`maskLight`) and warp together, from a flat tube
// at 0 to the full emulation at 1. `hardPix` is deliberately *not* scaled: the horizontal
// reconstruction filter is what turns a low-resolution signal into a continuous picture rather
// than a mosaic of blocks, so weakening it with the amount would just look broken.
//
// WebGL2 notes: no derivatives and no implicit-LOD samples here at all -- every signal fetch is
// an explicit `textureLoad` at an integer texel coordinate, which is also exactly the nearest
// fetch the algorithm asks for.

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
@group(0) @binding(1) var signal_tex: texture_2d<f32>;

const HARD_PIX: f32 = -3.0; // horizontal reconstruction sharpness (Lottes' hardPix)
const HARD_SCAN: f32 = -8.0; // scanline sharpness (Lottes' hardScan)
const WARP_X: f32 = 0.031; // barrel warp, x scaled by y^2 (Lottes' warpX)
const WARP_Y: f32 = 0.041; // barrel warp, y scaled by x^2 (Lottes' warpY)
const MASK_DARK: f32 = 0.5; // phosphor mask dark stripe (Lottes' maskDark)
const MASK_LIGHT: f32 = 1.5; // phosphor mask lit stripe (Lottes' maskLight)
const BRIGHT_BOOST: f32 = 1.0; // signal gain before reconstruction (Lottes' brightBoost)
const SHAPE: f32 = 2.0; // filter kernel shape exponent (Lottes' shape)
const GLASS_FRAC: f32 = 0.99; // tube glass fills this fraction of the raster
const CORNER_R: f32 = 0.06; // bezel corner radius (fraction of the screen height)
const EDGE_DARK: f32 = 0.35; // extra corner falloff depth at crt = 1

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

// sRGB -> linear (the composite writes display-gamma values into the signal buffer, whether the
// final target is an sRGB surface or not -- see the composite's `fx.w` handling).
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(lo, hi, c > vec3<f32>(0.04045));
}

// Linear -> sRGB, for the case where the final target is not sRGB and the hardware will not
// encode for us.
fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(lo, hi, c > vec3<f32>(0.0031308));
}

// Nearest signal fetch at a texel centre, scaled and linearised (Lottes' Fetch). Out-of-range
// texels clamp to the edge; the bezel below is what blacks out the area off the glass, so a
// clamped fetch never reaches the screen.
fn fetch(pos: vec2<f32>, off: vec2<f32>) -> vec3<f32> {
    let size = vec2<i32>(i32(u.signal.x), i32(u.signal.y));
    let idx = clamp(vec2<i32>(floor(pos * u.signal.xy + off)), vec2<i32>(0), size - vec2<i32>(1));
    return to_linear(textureLoad(signal_tex, idx, 0).rgb * BRIGHT_BOOST);
}

// Distance in signal texels to the nearest texel centre (Lottes' Dist).
fn dist_to_texel(pos: vec2<f32>) -> vec2<f32> {
    let p = pos * u.signal.xy;
    return -((p - floor(p)) - vec2<f32>(0.5));
}

// 1D Gaussian (Lottes' Gaus).
fn gaus(pos: f32, scale: f32) -> f32 {
    return exp2(scale * pow(abs(pos), SHAPE));
}

// 3-tap horizontal reconstruction (Lottes' Horz3).
fn horz3(pos: vec2<f32>, off: f32) -> vec3<f32> {
    let b = fetch(pos, vec2<f32>(-1.0, off));
    let c = fetch(pos, vec2<f32>(0.0, off));
    let d = fetch(pos, vec2<f32>(1.0, off));
    let dst = dist_to_texel(pos).x;
    let wb = gaus(dst - 1.0, HARD_PIX);
    let wc = gaus(dst, HARD_PIX);
    let wd = gaus(dst + 1.0, HARD_PIX);
    return (b * wb + c * wc + d * wd) / (wb + wc + wd);
}

// 5-tap horizontal reconstruction (Lottes' Horz5).
fn horz5(pos: vec2<f32>, off: f32) -> vec3<f32> {
    let a = fetch(pos, vec2<f32>(-2.0, off));
    let b = fetch(pos, vec2<f32>(-1.0, off));
    let c = fetch(pos, vec2<f32>(0.0, off));
    let d = fetch(pos, vec2<f32>(1.0, off));
    let e = fetch(pos, vec2<f32>(2.0, off));
    let dst = dist_to_texel(pos).x;
    let wa = gaus(dst - 2.0, HARD_PIX);
    let wb = gaus(dst - 1.0, HARD_PIX);
    let wc = gaus(dst, HARD_PIX);
    let wd = gaus(dst + 1.0, HARD_PIX);
    let we = gaus(dst + 2.0, HARD_PIX);
    return (a * wa + b * wb + c * wc + d * wd + e * we) / (wa + wb + wc + wd + we);
}

// Scanline weight for a signal line (Lottes' Scan). His `hardScan` fixes the *sharpness* of the
// line, so the `crt` amount scales how far the weight departs from flat instead: at 1 the weight
// is exactly his, at 0 the three lines are averaged equally and the vertical filter is a plain
// box. The flat end is 1/3 rather than 1 because `Tri` sums three weights -- interpolating from
// 1 would triple the picture's brightness at low amounts instead of leaving it alone.
fn scan(pos: vec2<f32>, off: f32, amount: f32) -> f32 {
    return mix(1.0 / 3.0, gaus(dist_to_texel(pos).y + off, HARD_SCAN), amount);
}

// Three-line blend of the signal (Lottes' Tri).
fn tri(pos: vec2<f32>, amount: f32) -> vec3<f32> {
    let a = horz3(pos, -1.0);
    let b = horz5(pos, 0.0);
    let c = horz3(pos, 1.0);
    let wa = scan(pos, -1.0, amount);
    let wb = scan(pos, 0.0, amount);
    let wc = scan(pos, 1.0, amount);
    return a * wa + b * wb + c * wc;
}

// Barrel distortion (Lottes' Warp); `amount` scales it so crt = 0 is a flat raster.
fn warp(pos_in: vec2<f32>, amount: f32) -> vec2<f32> {
    var pos = pos_in * 2.0 - 1.0;
    pos = pos * vec2<f32>(
        1.0 + (pos.y * pos.y) * WARP_X * amount,
        1.0 + (pos.x * pos.x) * WARP_Y * amount,
    );
    return pos * 0.5 + 0.5;
}

// Phosphor mask (Lottes' Mask, `shadowMask == 3`: the stretched VGA style). `pos` is in output
// pixels, so the stripes land on the display grid rather than the signal grid.
fn mask(pos_in: vec2<f32>, dark: f32, light: f32) -> vec3<f32> {
    var m = vec3<f32>(dark, dark, dark);
    var pos = pos_in;
    pos.x = fract((pos.x + pos.y * 3.0) / 6.0);
    if (pos.x < 1.0 / 3.0) {
        m.r = light;
    } else if (pos.x < 2.0 / 3.0) {
        m.g = light;
    } else {
        m.b = light;
    }
    return m;
}

// Soft rounded bezel plus tube corner falloff; 1 on the glass, 0 off it.
fn bezel(uv: vec2<f32>) -> f32 {
    let aspect = u.resolution.x / max(u.resolution.y, 1.0);
    let n = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);
    let half = vec2<f32>(0.5 * aspect * GLASS_FRAC, 0.5 * GLASS_FRAC);
    let q = abs(n) - (half - vec2<f32>(CORNER_R));
    let d = length(max(q, vec2<f32>(0.0))) - CORNER_R;
    let aa = 2.0 / max(u.resolution.y, 1.0);
    let glass = 1.0 - smoothstep(-aa, aa, d);
    let rn = length(n / half);
    return glass * (1.0 - EDGE_DARK * smoothstep(0.55, 1.0, rn));
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let crt = clamp(u.post.x, 0.0, 1.0);

    // Signal position: the warped raster coordinate, in signal texel space.
    let pos = warp(in.uv, crt);
    var col = tri(pos, crt);

    // Phosphor mask on the *output* pixel grid (Lottes does this on gl_FragCoord too), scaled in
    // from neutral as the amount rises.
    col = col * mask(in.pos.xy, mix(1.0, MASK_DARK, crt), mix(1.0, MASK_LIGHT, crt));

    // Tube shape: soft rounded bezel plus corner falloff, on top of everything else.
    col = col * mix(1.0, bezel(in.uv), crt);

    // `post.y` is 1 when the final target cannot sRGB-encode on store, exactly as in the
    // composite: with an sRGB target the linear value is what the hardware wants.
    if (u.post.y > 0.5) {
        return vec4<f32>(to_srgb(col), 1.0);
    }
    return vec4<f32>(col, 1.0);
}
