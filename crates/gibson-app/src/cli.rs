//! Command-line interface shared by every host: the desktop windowed app, the
//! `--snapshot` offscreen renderer, and (on Linux) the xscreensaver host.
//!
//! Precedence for every tunable: `defaults < config file < CLI override`.

use std::path::PathBuf;

use clap::Parser;
use gibson_types::{PaletteMode, Settings};

use crate::config;

/// Parse a `WxH` image size (`x` or `X` as the separator). Both dimensions must
/// be nonzero.
pub fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let err = || format!("invalid size {s:?}: expected WxH (e.g. 1920x1080)");
    let (w, h) = s.split_once(['x', 'X']).ok_or_else(err)?;
    if w.contains(['x', 'X']) || h.contains(['x', 'X']) {
        return Err(err());
    }
    let w: u32 = w.trim().parse().map_err(|_| err())?;
    let h: u32 = h.trim().parse().map_err(|_| err())?;
    if w == 0 || h == 0 {
        return Err(format!("invalid size {s:?}: dimensions must be nonzero"));
    }
    Ok((w, h))
}

/// `--palette` value parser: accept `normal`, `siege`, or `cycle`
/// (case-insensitive).
pub fn parse_palette(s: &str) -> Result<PaletteMode, String> {
    match s.to_ascii_lowercase().as_str() {
        "normal" => Ok(PaletteMode::Normal),
        "siege" => Ok(PaletteMode::Siege),
        "cycle" => Ok(PaletteMode::Cycle),
        other => Err(format!(
            "invalid palette {other:?}: expected normal, siege, or cycle"
        )),
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "gibson-app",
    about = "Hack the Gibson: a film-accurate flythrough of the 1995 tower city",
    version,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Start borderless fullscreen on the current monitor.
    #[arg(long)]
    pub fullscreen: bool,

    /// Render one offscreen still to PATH and exit (deterministic for a fixed
    /// --seed; the scene is stepped at a fixed 60 Hz).
    #[arg(long, value_name = "PATH")]
    pub snapshot: Option<PathBuf>,

    /// Snapshot image size.
    #[arg(long, value_name = "WxH", default_value = "1920x1080")]
    pub size: String,

    /// Snapshot simulation time in seconds.
    #[arg(long, value_name = "SECS", default_value_t = 12.0)]
    pub time: f64,

    /// Config file path (default: <config dir>/hack-the-gibson/gibson.toml).
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Override the configured fly speed.
    #[arg(long, value_name = "SPEED")]
    pub speed: Option<f32>,

    /// Override the configured bank strength.
    #[arg(long, value_name = "STRENGTH")]
    pub bank: Option<f32>,

    /// Override the configured palette.
    #[arg(long, value_name = "MODE", value_parser = parse_palette)]
    pub palette: Option<PaletteMode>,

    /// Override the configured tower grid.
    #[arg(long, value_name = "N")]
    pub grid: Option<u32>,

    /// Override the configured pulse count.
    #[arg(long, value_name = "N")]
    pub pulses: Option<u32>,

    /// Override the configured film grain amount.
    #[arg(long, value_name = "AMOUNT")]
    pub grain: Option<f32>,

    /// Override the configured CRT overlay strength.
    #[arg(long, value_name = "STRENGTH", conflicts_with = "no_crt")]
    pub crt: Option<f32>,

    /// Override the configured random seed.
    #[arg(long, value_name = "SEED")]
    pub seed: Option<u64>,

    /// Override the configured render resolution multiplier.
    #[arg(long, value_name = "SCALE")]
    pub render_scale: Option<f32>,

    /// Disable bloom.
    #[arg(long)]
    pub no_bloom: bool,

    /// Disable motion blur.
    #[arg(long)]
    pub no_motion_blur: bool,

    /// Disable the CRT overlay.
    #[arg(long, conflicts_with = "crt")]
    pub no_crt: bool,

    /// Linux xscreensaver: X11 window id to render into (decimal or 0x hex).
    /// Defaults to the XSCREENSAVER_WINDOW environment variable when set.
    #[cfg(target_os = "linux")]
    #[arg(long, value_name = "XID", hide = true)]
    pub window_id: Option<String>,
}

impl Cli {
    /// The X11 window id for the xscreensaver host: the explicit `--window-id`
    /// argument, or else the `XSCREENSAVER_WINDOW` environment variable
    /// xscreensaver sets for external-window hacks.
    #[cfg(target_os = "linux")]
    pub fn x11_window_arg(&self) -> Option<String> {
        self.window_id
            .clone()
            .or_else(|| std::env::var("XSCREENSAVER_WINDOW").ok())
    }
}

/// Resolve the effective settings for a run: load (or create) the config file,
/// then apply CLI overrides. Final result is always clamped.
pub fn resolve(cli: &Cli) -> Result<Settings, String> {
    let path = cli.config.clone().unwrap_or_else(config::default_path);
    let mut settings = config::load_or_create(&path)?;
    if let Some(v) = cli.speed {
        settings.fly_speed = v;
    }
    if let Some(v) = cli.bank {
        settings.bank_strength = v;
    }
    if let Some(v) = cli.palette {
        settings.palette = v;
    }
    if let Some(v) = cli.grid {
        settings.grid = v;
    }
    if let Some(v) = cli.pulses {
        settings.pulses = v;
    }
    if let Some(v) = cli.grain {
        settings.grain = v;
    }
    if let Some(v) = cli.crt {
        settings.crt = v;
    }
    if let Some(v) = cli.seed {
        settings.seed = v;
    }
    if let Some(v) = cli.render_scale {
        settings.render_scale = v;
    }
    if cli.no_bloom {
        settings.bloom = 0.0;
    }
    if cli.no_motion_blur {
        settings.motion_blur = 0.0;
    }
    if cli.no_crt {
        settings.crt = 0.0;
    }
    Ok(settings.clamped())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_from(std::iter::once("gibson-app").chain(args.iter().copied()))
    }

    fn temp_config(body: &str) -> PathBuf {
        // Nanosecond timestamps alone can collide when sibling tests on other threads
        // start within the same clock tick, so add a process-wide counter to keep every
        // temp file unique.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("gibson-app-cli-{nonce}-{seq}.toml"));
        fs::write(&path, body).expect("write temp config");
        path
    }

    #[test]
    fn size_parsing_accepts_valid_sizes() {
        assert_eq!(parse_size("1920x1080"), Ok((1920, 1080)));
        assert_eq!(parse_size("800X600"), Ok((800, 600))); // uppercase X
        assert_eq!(parse_size(" 640 x 480 "), Ok((640, 480))); // stray spaces
        assert_eq!(parse_size("1x1"), Ok((1, 1)));
    }

    #[test]
    fn size_parsing_rejects_invalid_sizes() {
        for bad in ["abc", "1920", "x1080", "1920x", "a x b", "0x0", "1920x1080x32", "", "-5x10"] {
            assert!(parse_size(bad).is_err(), "expected {bad:?} to be rejected");
        }
    }

    #[test]
    fn palette_parsing() {
        assert_eq!(parse_palette("normal"), Ok(PaletteMode::Normal));
        assert_eq!(parse_palette("SIEGE"), Ok(PaletteMode::Siege));
        assert_eq!(parse_palette("Cycle"), Ok(PaletteMode::Cycle));
        assert!(parse_palette("sepia").is_err());
    }

    #[test]
    fn cli_overrides_config_file() {
        let path = temp_config("fly_speed = 1.5\ngrid = 50\npulses = 1000\n");
        let cli = parse(&["--config", path.to_str().unwrap(), "--speed", "0.8"]);
        let settings = resolve(&cli).expect("resolve");
        assert_eq!(settings.fly_speed, 0.8, "CLI --speed beats the config file");
        assert_eq!(settings.grid, 50, "unoverridden config value survives");
        assert_eq!(settings.pulses, 1000, "unoverridden config value survives");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn cli_booleans_and_palette_override() {
        let path = temp_config("bloom = 0.9\nmotion_blur = 0.7\npalette = \"siege\"\n");
        let cli = parse(&[
            "--config",
            path.to_str().unwrap(),
            "--palette",
            "cycle",
            "--no-bloom",
            "--no-motion-blur",
        ]);
        let settings = resolve(&cli).expect("resolve");
        assert_eq!(settings.palette, PaletteMode::Cycle);
        assert_eq!(settings.bloom, 0.0);
        assert_eq!(settings.motion_blur, 0.0);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn missing_config_is_created_then_overridden() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("gibson-app-new-{nonce}.toml"));
        let cli = parse(&["--config", path.to_str().unwrap(), "--grid", "33"]);
        let settings = resolve(&cli).expect("resolve creates config");
        assert!(path.exists(), "--config path should be created");
        assert_eq!(settings.grid, 33);
        assert_eq!(settings.fly_speed, Settings::default().fly_speed);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn crt_override_and_no_crt() {
        let path = temp_config("crt = 0.8\n");
        // --crt overrides the config file.
        let cli = parse(&["--config", path.to_str().unwrap(), "--crt", "0.2"]);
        let settings = resolve(&cli).expect("resolve");
        assert_eq!(settings.crt, 0.2);
        // --no-crt forces 0 regardless of the config file.
        let cli = parse(&["--config", path.to_str().unwrap(), "--no-crt"]);
        let settings = resolve(&cli).expect("resolve");
        assert_eq!(settings.crt, 0.0);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn crt_and_no_crt_conflict() {
        use clap::error::ErrorKind;
        let err = Cli::try_parse_from(["gibson-app", "--crt", "0.5", "--no-crt"])
            .expect_err("--crt and --no-crt must conflict");
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn cli_crt_out_of_range_is_clamped() {
        let path = temp_config("");
        let cli = parse(&["--config", path.to_str().unwrap(), "--crt", "3"]);
        let settings = resolve(&cli).expect("resolve");
        assert_eq!(settings.crt, 1.0);
        let cli = parse(&["--config", path.to_str().unwrap(), "--crt=-1"]);
        let settings = resolve(&cli).expect("resolve");
        assert_eq!(settings.crt, 0.0);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn out_of_range_cli_values_are_clamped() {
        // Config default path is the real user dir; point at a temp file so the
        // test never touches the user's config.
        let path = temp_config("");
        let cli = parse(&["--config", path.to_str().unwrap(), "--speed", "50", "--grid", "100000"]);
        let settings = resolve(&cli).expect("resolve");
        assert_eq!(settings.fly_speed, 3.0);
        assert_eq!(settings.grid, 120);
        assert_eq!(settings.render_scale, Settings::default().render_scale);
        let _ = fs::remove_file(&path);
    }
}
