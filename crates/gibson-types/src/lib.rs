//! Frozen shared type contracts for Hack the Gibson.
//!
//! This crate is the single contract every other crate compiles against. After the Scaffold
//! batch the public signatures, constant values, and fields in this file MUST NOT change;
//! later agents may only add doc comments.
//!
//! World conventions the rest of the project depends on:
//! - Right-handed Y-up world (legacy Irrlicht was left-handed; z was negated on import).
//! - Grid pitch `TOWER_PITCH` = 30 units; tower base centers sit at `x ≡ 15 (mod 30)`,
//!   `z ≡ 0 (mod 30)`; the lanes between them run along `x ≡ 0` and `z ≡ 15`.
//! - Tower footprint: `TOWER_WIDTH × TOWER_HEIGHT × TOWER_WIDTH` (12 × 110 × 12) at most; per-
//!   tower heights range `TOWER_HEIGHT_MIN..=TOWER_HEIGHT` (44..=110), base on y = 0.
//! - Colors are HDR linear values, pre-multiplied by intensity where noted.

/// Grid pitch in world units; towers are placed every 30 units.
pub const TOWER_PITCH: f32 = 30.0;
/// Tower box width and depth in world units.
pub const TOWER_WIDTH: f32 = 12.0;
/// Nominal (tallest) tower box height in world units (base sits on y = 0). Individual towers
/// carry their own height in [`TowerInstance::height`], drawn from `TOWER_HEIGHT_MIN..=TOWER_HEIGHT`.
pub const TOWER_HEIGHT: f32 = 110.0;
/// Shortest tower in the skyline (world units); the film's towers form a ~8-10:1 canyon.
pub const TOWER_HEIGHT_MIN: f32 = 44.0;

/// Floor tile edge length in world units (one tile = 96 × 96 cells).
pub const FLOOR_TILE_UNITS: f32 = 240.0;
/// Floor cells per tile edge; one cell = 2.5 world units.
pub const FLOOR_TILE_CELLS: u32 = 96;

/// Text atlas pixel dimensions, panel count, and layer count.
/// Layer `p` is panel `p` variant A; layer `p + 32` is the same panel's variant B.
pub const ATLAS_WIDTH: u32 = 256;
pub const ATLAS_HEIGHT: u32 = 768;
pub const ATLAS_PANELS: u32 = 32;
pub const ATLAS_LAYERS: u32 = 64;

/// Fog start/end distances in world units (beyond `FOG_END` towers are fully fogged out).
pub const FOG_START: f32 = 220.0;
pub const FOG_END: f32 = 900.0;

/// User-adjustable settings. Serialized by hosts (`.saver` defaults, `gibson.toml`, web query
/// params); `clamped()` normalizes any input (config files, CLI, query strings) to valid ranges.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Fly speed in flight-path segments per second (default 0.55).
    pub fly_speed: f32,
    /// Camera banking gain applied to yaw rate (default 0.45).
    pub bank_strength: f32,
    /// Maximum camera bank angle in degrees (default 32.0).
    pub bank_max_degrees: f32,
    /// Bank smoothing low-pass time constant in seconds (default 0.55).
    pub bank_smoothing: f32,
    /// Color treatment: normal Gibson blues, siege attack oranges, or timed cycling.
    pub palette: PaletteMode,
    /// Seconds between palette switches when `palette == Cycle` (default 240.0).
    pub palette_cycle_seconds: f32,
    /// Bloom intensity 0..=2 (default 0.45).
    pub bloom: f32,
    /// Motion blur strength 0..=1 (default 0.5).
    pub motion_blur: f32,
    /// Film grain amount 0..=0.2 (default 0.03).
    pub grain: f32,
    /// CRT overlay strength: 0 disables it, 1 is the full effect (scanlines, aperture grille,
    /// screen curvature, phosphor bloom, edge vignette) (default 0.35).
    pub crt: f32,
    /// Internal render resolution multiplier 0.25..=1 (default 1.0).
    pub render_scale: f32,
    /// Tower city grid: `grid × grid` towers, 8..=120 (default 60).
    pub grid: u32,
    /// Number of lane pulse streaks, 0..=2000 (default 700).
    pub pulses: u32,
    /// City/atlas/floor random seed; 0 means the host derives one from time (default 0).
    pub seed: u64,
    /// True while running as a screen-saver preview tile (fewer pulses, etc.).
    pub preview: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            fly_speed: 0.55,
            bank_strength: 0.45,
            bank_max_degrees: 32.0,
            bank_smoothing: 0.55,
            palette: PaletteMode::Normal,
            palette_cycle_seconds: 240.0,
            bloom: 0.45,
            motion_blur: 0.5,
            grain: 0.03,
            crt: 0.35,
            render_scale: 1.0,
            grid: 60,
            pulses: 700,
            seed: 0,
            preview: false,
        }
    }
}

impl Settings {
    /// Clamp every field to its legal range. `NaN` inputs are replaced by the field's default.
    /// - `fly_speed` 0.05..=3, `bank_strength` -3..=3, `bank_max_degrees` 0..=60,
    ///   `bank_smoothing` 0.05..=2, `bloom` 0..=2, `motion_blur` 0..=1, `grain` 0..=0.2,
    ///   `crt` 0..=1, `render_scale` 0.25..=1, `grid` 8..=120, `pulses` 0..=2000,
    ///   `palette_cycle_seconds` 10..=3600.
    pub fn clamped(self) -> Settings {
        Settings {
            fly_speed: clamp_or_default(self.fly_speed, 0.05, 3.0, 0.55),
            bank_strength: clamp_or_default(self.bank_strength, -3.0, 3.0, 0.45),
            bank_max_degrees: clamp_or_default(self.bank_max_degrees, 0.0, 60.0, 32.0),
            bank_smoothing: clamp_or_default(self.bank_smoothing, 0.05, 2.0, 0.55),
            palette: self.palette,
            palette_cycle_seconds: clamp_or_default(self.palette_cycle_seconds, 10.0, 3600.0, 240.0),
            bloom: clamp_or_default(self.bloom, 0.0, 2.0, 0.45),
            motion_blur: clamp_or_default(self.motion_blur, 0.0, 1.0, 0.5),
            grain: clamp_or_default(self.grain, 0.0, 0.2, 0.03),
            crt: clamp_or_default(self.crt, 0.0, 1.0, 0.35),
            render_scale: clamp_or_default(self.render_scale, 0.25, 1.0, 1.0),
            grid: self.grid.clamp(8, 120),
            pulses: self.pulses.clamp(0, 2000),
            seed: self.seed,
            preview: self.preview,
        }
    }
}

/// Clamp `v` to `[lo, hi]`, falling back to `default` when `v` is `NaN`.
fn clamp_or_default(v: f32, lo: f32, hi: f32, default: f32) -> f32 {
    if v.is_nan() {
        default
    } else {
        v.clamp(lo, hi)
    }
}

/// Color treatment for the whole scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaletteMode {
    /// Normal deep-blue Gibson palette.
    Normal,
    /// Siege / under-attack orange-red palette.
    Siege,
    /// Alternate between Normal and Siege every `palette_cycle_seconds`.
    Cycle,
}

/// HDR color palette. All values are linear and pre-multiplied by their intensity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Translucent tower glass body: RGB linear HDR + alpha (opacity).
    pub tower_body: [f32; 4],
    /// Tower-face text color (RGB).
    pub tower_text: [f32; 3],
    /// Selected-block highlight color (RGB).
    pub highlight: [f32; 3],
    /// Floor trace color (RGB).
    pub floor_trace: [f32; 3],
    /// Floor pad / via color (RGB).
    pub floor_pad: [f32; 3],
    /// Lane pulse streak color (RGB).
    pub pulse: [f32; 3],
    /// HDR linear glow colors the scene samples per beam; index 0 is the most common.
    pub pulse_hues: [[f32; 3]; 4],
    /// Distance haze color (RGB).
    pub haze: [f32; 3],
}

impl Palette {
    /// Deep-blue Gibson: body `#0E2A6A` at ~35 % opacity, cyan text, magenta highlights,
    /// violet PCB traces on black, near-white pulses, blue haze.
    pub const NORMAL: Palette = Palette {
        tower_body: [0.055, 0.165, 0.415, 0.35],
        // Text #48E8FF converted sRGB->linear (~0.066, 0.81, 1.0); red kept a touch above the
        // film value so glyphs stay bright, green/blue carry the hue.
        tower_text: [0.10, 0.84, 0.92],
        highlight: [0.75 * 2.5, 0.38 * 2.5, 1.0 * 2.5],
        floor_trace: [0.36 * 1.6, 0.25 * 1.6, 0.91 * 1.6],
        floor_pad: [0.64 * 1.6, 0.55 * 1.6, 1.0 * 1.6],
        pulse: [0.85 * 3.0, 1.0 * 3.0, 1.0 * 3.0],
        // Beam glows: mostly the white-cyan above, then bright green "zip" lasers, electric
        // blue, and magenta accents.
        pulse_hues: [
            [0.85 * 3.0, 1.0 * 3.0, 1.0 * 3.0],
            [0.20 * 3.0, 1.0 * 3.0, 0.35 * 3.0],
            [0.30 * 3.0, 0.55 * 3.0, 1.0 * 3.0],
            [1.0 * 3.0, 0.35 * 3.0, 0.95 * 3.0],
        ],
        haze: [0.06, 0.19, 0.63],
    };

    /// Siege palette: magenta-pink body, orange-red text, ice-blue floor, warm haze.
    pub const SIEGE: Palette = Palette {
        tower_body: [0.42, 0.06, 0.25, 0.40],
        // Text #FF6030 converted sRGB->linear is (1.0, 0.029, 0.015); storing the sRGB values
        // in this linear buffer washed the orange toward cream. x1.25 lifts the hot cores.
        tower_text: [1.0 * 1.25, 0.029 * 1.25, 0.0146 * 1.25],
        highlight: [1.0 * 2.5, 0.85 * 2.5, 0.4 * 2.5],
        // Trace #9FC0FF in linear is ~(0.34, 0.52, 1.0); x1.5 keeps them icy without clipping.
        floor_trace: [0.34 * 1.5, 0.52 * 1.5, 1.0 * 1.5],
        floor_pad: [0.69 * 1.2, 0.78 * 1.2, 1.0 * 1.2],
        pulse: [1.0 * 3.0, 0.9 * 3.0, 0.8 * 3.0],
        // Siege beam glows: warm white most often, amber, hot red, and pale green zips.
        pulse_hues: [
            [1.0 * 3.0, 0.9 * 3.0, 0.8 * 3.0],
            [1.0 * 3.0, 0.62 * 3.0, 0.18 * 3.0],
            [1.0 * 3.0, 0.25 * 3.0, 0.15 * 3.0],
            [0.55 * 3.0, 1.0 * 3.0, 0.45 * 3.0],
        ],
        haze: [0.25, 0.06, 0.12],
    };

    /// Linearly interpolate every component (including `tower_body` alpha). `t` is clamped to
    /// 0..=1; `t = 0` returns `self`, `t = 1` returns `other` (bit-exact endpoints).
    pub fn lerp(&self, other: &Palette, t: f32) -> Palette {
        let t = t.clamp(0.0, 1.0);
        let l3 = |a: [f32; 3], b: [f32; 3]| {
            [
                a[0] * (1.0 - t) + b[0] * t,
                a[1] * (1.0 - t) + b[1] * t,
                a[2] * (1.0 - t) + b[2] * t,
            ]
        };
        let l4 = |a: [f32; 4], b: [f32; 4]| {
            [
                a[0] * (1.0 - t) + b[0] * t,
                a[1] * (1.0 - t) + b[1] * t,
                a[2] * (1.0 - t) + b[2] * t,
                a[3] * (1.0 - t) + b[3] * t,
            ]
        };
        let lh = |a: &[[f32; 3]; 4], b: &[[f32; 3]; 4]| {
            let mut out = [[0.0f32; 3]; 4];
            for k in 0..4 {
                out[k] = l3(a[k], b[k]);
            }
            out
        };
        Palette {
            tower_body: l4(self.tower_body, other.tower_body),
            tower_text: l3(self.tower_text, other.tower_text),
            highlight: l3(self.highlight, other.highlight),
            floor_trace: l3(self.floor_trace, other.floor_trace),
            floor_pad: l3(self.floor_pad, other.floor_pad),
            pulse: l3(self.pulse, other.pulse),
            pulse_hues: lh(&self.pulse_hues, &other.pulse_hues),
            haze: l3(self.haze, other.haze),
        }
    }
}

/// One tower drawn this frame. GPU-ready layout; sorted back-to-front by the scene.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TowerInstance {
    /// Tower base center on the floor (y = 0).
    pub position: [f32; 3],
    /// Random per-tower animation phase (drives block refresh/wipe timing).
    pub anim_phase: f32,
    /// Atlas panel id per side face, order +x, -x, +z, -z.
    pub face_layers: [u32; 4],
    /// Atlas layer for the top face.
    pub top_layer: u32,
    /// Highlighted block id; 0 = none, else a block id 1..=255.
    pub highlight_block: u32,
    /// Highlight animation 0..1 (ramp up, hold, fade).
    pub highlight_t: f32,
    /// This tower's actual height in world units; the renderer scales the unit box by it
    /// (`TOWER_HEIGHT_MIN..=TOWER_HEIGHT`).
    pub height: f32,
}

/// One lane pulse streak this frame. GPU-ready layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PulseInstance {
    /// Pulse head position (world units).
    pub position: [f32; 3],
    /// Ribbon length in world units.
    pub length: f32,
    /// Unit travel direction.
    pub direction: [f32; 3],
    /// Brightness 0.7..=1.0.
    pub intensity: f32,
    /// HDR linear glow color for this beam; the scene randomizes it.
    pub color: [f32; 3],
    pub _pad: f32,
}

/// Camera pose handed to the renderer (vertical FOV is 58 degrees).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    /// Eye position.
    pub position: [f32; 3],
    /// Unit view direction.
    pub forward: [f32; 3],
    /// Unit up vector (may be rolled for banking).
    pub up: [f32; 3],
    /// Vertical field of view in radians (58 degrees).
    pub fov_y_radians: f32,
}

/// Generated tower-face text atlas.
///
/// Pixel layout: `layers` layers of `width × height`; rows are top-first, and the buffer is
/// layer-major (`rgba[layer][y][x][c]`). Channel semantics per pixel:
/// - R: glyph coverage (0 = empty, 255 = solid glyph).
/// - G: block-local vertical position, 0 at the top of the block ..= 255 at the bottom.
/// - B: block id (0 = no block, 1..=255).
/// - A: 255 inside a block, 0 outside.
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasImage {
    /// Pixel width of one layer (`ATLAS_WIDTH`).
    pub width: u32,
    /// Pixel height of one layer (`ATLAS_HEIGHT`).
    pub height: u32,
    /// Number of layers (`ATLAS_LAYERS`; panel p at p and p + 32).
    pub layers: u32,
    /// Layer-major RGBA bytes, `width * height * layers * 4` long.
    pub rgba: Vec<u8>,
    /// Per panel (len `ATLAS_PANELS`): the block ids that panel contains, in id order.
    pub blocks_per_panel: Vec<Vec<u8>>,
}

/// Generated PCB floor map for one toroidal tile (`FLOOR_TILE_CELLS × FLOOR_TILE_CELLS` cells).
#[derive(Clone, Debug, PartialEq)]
pub struct FloorMap {
    /// Cells per tile edge (`FLOOR_TILE_CELLS`).
    pub cells: u32,
    /// Row-major cells, z then x (`data[z * cells + x]`), 4 bytes per cell:
    /// - R: trace half-segment bits from the cell center — bit 1 = +x, 2 = -x, 4 = +z, 8 = -z.
    /// - G: ground feature — 0 = none, 1 = pad, 2 = via, 3 = chip body.
    /// - B: unused (0).
    /// - A: brightness, 128..=255.
    pub data: Vec<[u8; 4]>,
}

/// Everything the renderer needs to draw one frame. Borrowed from the scene.
#[derive(Clone, Copy, Debug)]
pub struct FrameData<'a> {
    /// Scene time in seconds (t = 0 at the host's first frame call).
    pub time: f64,
    /// Current camera pose.
    pub camera: CameraPose,
    /// Previous frame's camera pose (motion blur reprojection).
    pub prev_camera: CameraPose,
    /// Active color palette.
    pub palette: Palette,
    /// Visible towers, sorted back-to-front, frustum + fog culled.
    pub towers: &'a [TowerInstance],
    /// Visible pulses.
    pub pulses: &'a [PulseInstance],
    /// Settings the frame was produced with.
    pub settings: &'a Settings,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamped_honors_bounds() {
        let s = Settings {
            fly_speed: 99.0,
            bank_strength: -99.0,
            bank_max_degrees: 99.0,
            bank_smoothing: 0.0,
            palette: PaletteMode::Siege,
            palette_cycle_seconds: 1.0,
            bloom: -1.0,
            motion_blur: 5.0,
            grain: 1.0,
            crt: 5.0,
            render_scale: 0.0,
            grid: 2,
            pulses: 99999,
            seed: 7,
            preview: true,
        };
        let c = s.clamped();
        assert_eq!(c.fly_speed, 3.0);
        assert_eq!(c.bank_strength, -3.0);
        assert_eq!(c.bank_max_degrees, 60.0);
        assert_eq!(c.bank_smoothing, 0.05);
        assert_eq!(c.palette, PaletteMode::Siege);
        assert_eq!(c.palette_cycle_seconds, 10.0);
        assert_eq!(c.bloom, 0.0);
        assert_eq!(c.motion_blur, 1.0);
        assert_eq!(c.grain, 0.2);
        assert_eq!(c.crt, 1.0);
        assert_eq!(c.render_scale, 0.25);
        assert_eq!(c.grid, 8);
        assert_eq!(c.pulses, 2000);
        assert_eq!(c.seed, 7);
        assert!(c.preview);

        let low = Settings {
            fly_speed: -5.0,
            bank_strength: -5.0,
            bank_max_degrees: -3.0,
            bank_smoothing: 9.0,
            palette: PaletteMode::Normal,
            palette_cycle_seconds: 99999.0,
            bloom: 4.0,
            motion_blur: -4.0,
            grain: 0.0,
            crt: -1.0,
            render_scale: 5.0,
            grid: 0,
            pulses: 0,
            seed: 1,
            preview: false,
        }
        .clamped();
        assert_eq!(low.fly_speed, 0.05);
        assert_eq!(low.bank_strength, -3.0);
        assert_eq!(low.bank_max_degrees, 0.0);
        assert_eq!(low.bank_smoothing, 2.0);
        assert_eq!(low.palette_cycle_seconds, 3600.0);
        assert_eq!(low.bloom, 2.0);
        assert_eq!(low.motion_blur, 0.0);
        assert_eq!(low.grain, 0.0);
        assert_eq!(low.crt, 0.0);
        assert_eq!(low.render_scale, 1.0);
        assert_eq!(low.grid, 8);
        assert_eq!(low.pulses, 0);
    }

    #[test]
    fn clamped_replaces_nan_with_defaults() {
        let d = Settings::default();
        let s = Settings {
            fly_speed: f32::NAN,
            bank_strength: f32::NAN,
            bank_max_degrees: f32::NAN,
            bank_smoothing: f32::NAN,
            palette: PaletteMode::Cycle,
            palette_cycle_seconds: f32::NAN,
            bloom: f32::NAN,
            motion_blur: f32::NAN,
            grain: f32::NAN,
            crt: f32::NAN,
            render_scale: f32::NAN,
            grid: 30,
            pulses: 100,
            seed: 3,
            preview: false,
        }
        .clamped();
        assert_eq!(s.fly_speed, d.fly_speed);
        assert_eq!(s.bank_strength, d.bank_strength);
        assert_eq!(s.bank_max_degrees, d.bank_max_degrees);
        assert_eq!(s.bank_smoothing, d.bank_smoothing);
        assert_eq!(s.palette_cycle_seconds, d.palette_cycle_seconds);
        assert_eq!(s.bloom, d.bloom);
        assert_eq!(s.motion_blur, d.motion_blur);
        assert_eq!(s.grain, d.grain);
        assert_eq!(s.crt, d.crt);
        assert_eq!(s.render_scale, d.render_scale);
        // Non-float fields pass through untouched.
        assert_eq!(s.palette, PaletteMode::Cycle);
        assert_eq!(s.grid, 30);
        assert_eq!(s.pulses, 100);
        assert_eq!(s.seed, 3);
    }

    #[test]
    fn lerp_endpoints_and_clamp() {
        let n = Palette::NORMAL;
        let s = Palette::SIEGE;
        assert_eq!(n.lerp(&s, 0.0), n);
        assert_eq!(n.lerp(&s, 1.0), s);
        // Out-of-range t clamps.
        assert_eq!(n.lerp(&s, -2.0), n);
        assert_eq!(n.lerp(&s, 7.5), s);
    }

    #[test]
    fn lerp_interpolates_components_including_alpha() {
        let n = Palette::NORMAL;
        let s = Palette::SIEGE;
        let m = n.lerp(&s, 0.5);
        for i in 0..4 {
            let expect = (n.tower_body[i] + s.tower_body[i]) * 0.5;
            assert!((m.tower_body[i] - expect).abs() < 1e-6, "tower_body[{i}]");
        }
        for k in 0..4 {
            for i in 0..3 {
                let expect = (n.pulse_hues[k][i] + s.pulse_hues[k][i]) * 0.5;
                assert!(
                    (m.pulse_hues[k][i] - expect).abs() < 1e-6,
                    "pulse_hues[{k}][{i}]"
                );
            }
        }
        for i in 0..3 {
            let expect = (n.tower_text[i] + s.tower_text[i]) * 0.5;
            assert!((m.tower_text[i] - expect).abs() < 1e-6, "tower_text[{i}]");
            let expect = (n.floor_trace[i] + s.floor_trace[i]) * 0.5;
            assert!((m.floor_trace[i] - expect).abs() < 1e-6, "floor_trace[{i}]");
        }
        // Alpha really is interpolated (NORMAL 0.35 → SIEGE 0.40).
        assert!((m.tower_body[3] - 0.375).abs() < 1e-6);
    }

    #[test]
    fn instance_sizes_are_stable() {
        // GPU instance buffers are sized from these; the layout is part of the frozen contract.
        // TowerInstance keeps its historical 48 bytes with `height` replacing the old `_pad`.
        assert_eq!(std::mem::size_of::<TowerInstance>(), 48);
        // PulseInstance grows from 32 to 48 bytes with the added glow color.
        assert_eq!(std::mem::size_of::<PulseInstance>(), 48);
        assert_eq!(std::mem::align_of::<TowerInstance>(), 4);
        assert_eq!(std::mem::align_of::<PulseInstance>(), 4);
    }

    #[test]
    fn settings_serde_round_trip_with_defaults_for_missing() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        // Missing fields fall back to Default via #[serde(default)].
        let parsed: Settings = serde_json::from_str("{\"fly_speed\": 1.5}").unwrap();
        assert_eq!(parsed.fly_speed, 1.5);
        assert_eq!(parsed.grid, s.grid);
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        // PaletteMode lowercase round trip.
        let p: PaletteMode = serde_json::from_str("\"siege\"").unwrap();
        assert_eq!(p, PaletteMode::Siege);
        assert_eq!(serde_json::to_string(&PaletteMode::Cycle).unwrap(), "\"cycle\"");
    }
}
