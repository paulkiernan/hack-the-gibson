//! `gibson.toml` handling: the platform config path, a commented default template, and
//! load-or-create. Settings precedence is `defaults < config file < CLI overrides` — this module
//! only handles the file; `crate::cli::resolve` applies the overrides on top.

use std::fs;
use std::path::{Path, PathBuf};

use gibson_types::Settings;

/// Default config path: `<config dir>/hack-the-gibson/gibson.toml`
/// (macOS `~/Library/Application Support`, Linux `~/.config`, Windows `%APPDATA%`).
pub fn default_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("hack-the-gibson").join("gibson.toml")
}

/// The commented default config file. Values must stay byte-for-byte equal to
/// [`Settings::default`] so a freshly created file parses back to the defaults
/// (the config round-trip test enforces this).
pub const TEMPLATE: &str = r#"# Hack the Gibson settings.
#
# Written automatically the first time a host runs; edit freely. The desktop app,
# the screensaver hosts, and the --snapshot command all read this file, and any
# value outside its documented range is clamped to that range on load.
#
# The tower city (layout, text, floor traces) is generated from `seed`; fly path,
# banking and all effects are driven by the clock, so the same settings always
# produce the same flight.

# City / atlas / floor random seed. 0 derives a seed from the current time, so
# every launch shows a different city. Set a nonzero value to reproduce a
# particular city and flight exactly.
seed = 0

# ---- camera ---------------------------------------------------------------
# Fly speed along the flight path, in path segments per second (0.05 .. 3).
fly_speed = 0.55
# How strongly the camera banks into turns (0 .. 3; higher = steeper roll).
bank_strength = 0.45
# Maximum bank angle in degrees (0 .. 60).
bank_max_degrees = 32
# Banking smoothness: higher values smooth the roll harder (0.05 .. 2).
bank_smoothing = 0.55

# ---- color -----------------------------------------------------------------
# Palette: "normal" (deep Gibson blues), "siege" (attack oranges), or "cycle"
# (alternate between them).
palette = "normal"
# Seconds between palette switches when palette = "cycle" (10 .. 3600).
palette_cycle_seconds = 240

# ---- effects ---------------------------------------------------------------
# Bloom intensity (0 disables bloom .. 2).
bloom = 0.45
# Motion blur strength (0 disables .. 1).
motion_blur = 0.5
# Film grain amount (0 disables .. 0.2).
grain = 0.03
# CRT overlay strength (0 disables .. 1): scanlines, aperture grille, screen
# curvature, phosphor bloom, and edge vignette over the whole render.
crt = 0.35
# Internal render resolution multiplier (0.25 .. 1): 0.5 renders at half
# resolution for slower GPUs.
render_scale = 1

# ---- world ------------------------------------------------------------------
# Tower city grid: grid x grid towers (8 .. 120).
grid = 60
# Number of blue pulse streaks flying down the lanes (0 .. 2000).
pulses = 700

# True while running as a screensaver preview tile (fewer pulses etc.).
# Screensaver hosts set this themselves; leave it false for the desktop app.
preview = false
"#;

/// Ensure the config file exists (creating it from [`TEMPLATE`] when missing,
/// along with its parent directory) and parse it into clamped settings.
///
/// Unknown keys are ignored (serde ignores unrecognized fields); missing keys
/// fall back to defaults via `#[serde(default)]` on `Settings`; then
/// [`Settings::clamped`] normalizes anything out of range.
pub fn load_or_create(path: &Path) -> Result<Settings, String> {
    ensure_file(path)?;
    let text = fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let settings: Settings = toml::from_str(&text)
        .map_err(|e| format!("invalid config {}: {e}", path.display()))?;
    Ok(settings.clamped())
}

/// Create the config file (and its parent directory) if it does not exist yet.
pub fn ensure_file(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
    }
    fs::write(path, TEMPLATE).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(tag: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("gibson-app-{tag}-{nonce}.toml"))
    }

    #[test]
    fn fresh_file_parses_to_defaults() {
        let path = temp_path("fresh");
        let settings = load_or_create(&path).expect("create + parse fresh config");
        assert!(path.exists(), "config file should have been created");
        assert_eq!(settings, Settings::default());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn default_template_parses_to_defaults() {
        // The hand-written template must stay in sync with Settings::default().
        let settings: Settings = toml::from_str(TEMPLATE).expect("template parses");
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn round_trip_defaults_through_toml() {
        // Serialize defaults, parse them back: identical.
        let text = toml::to_string(&Settings::default()).expect("serialize defaults");
        let back: Settings = toml::from_str(&text).expect("parse serialized defaults");
        assert_eq!(back, Settings::default());
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let text = r#"
            fly_speed = 0.7
            grid = 40
            totally_unknown_key = "x"
            another_one = 12345
            [nested.unknown]
            deep = true
        "#;
        let settings: Settings = toml::from_str(text).expect("parse with unknown keys");
        assert_eq!(settings.fly_speed, 0.7);
        assert_eq!(settings.grid, 40);
        // Everything else untouched (defaults).
        let d = Settings::default();
        assert_eq!(settings.palette, d.palette);
        assert_eq!(settings.bloom, d.bloom);
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        // Only two keys present: the rest come from Settings::default().
        let text = "fly_speed = 0.3\npulses = 5\n";
        let settings: Settings = toml::from_str(text).expect("partial config parses");
        assert_eq!(settings.fly_speed, 0.3);
        assert_eq!(settings.pulses, 5);
        assert_eq!(settings.palette, Settings::default().palette);
        assert_eq!(settings.grid, Settings::default().grid);
    }

    #[test]
    fn empty_file_is_defaults() {
        let settings: Settings = toml::from_str("").expect("empty config parses");
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn out_of_range_values_are_clamped_on_load() {
        let path = temp_path("clamped");
        fs::write(
            &path,
            "fly_speed = 99\ngrid = 2\nbloom = -3\nmotion_blur = 7\npulses = 99999\ncrt = 9\n",
        )
        .expect("write config");
        let settings = load_or_create(&path).expect("load clamps");
        assert_eq!(settings.fly_speed, 3.0);
        assert_eq!(settings.grid, 8);
        assert_eq!(settings.bloom, 0.0);
        assert_eq!(settings.motion_blur, 1.0);
        assert_eq!(settings.pulses, 2000);
        assert_eq!(settings.crt, 1.0);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn crt_value_loaded_from_config() {
        let path = temp_path("crt");
        fs::write(&path, "crt = 0.6\n").expect("write config");
        let settings = load_or_create(&path).expect("load");
        assert_eq!(settings.crt, 0.6);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn invalid_toml_is_an_error() {
        let path = temp_path("bad");
        fs::write(&path, "not [ valid toml").expect("write bad config");
        assert!(load_or_create(&path).is_err());
        let _ = fs::remove_file(&path);
    }
}
