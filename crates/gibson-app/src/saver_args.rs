//! Lenient parser for the classic Windows screensaver command-line protocol.
//!
//! The Windows display applet launches a `.scr` with `-s` / `/s` (full screen),
//! `/p <hwnd>` / `/p:<hwnd>` (small preview inside a host window), and
//! `/c` / `/c:<hwnd>` (configure). The separators and switch prefixes vary, so
//! this parser accepts the common forms. It is deliberately platform
//! *independent* (unit-tested on every host); `main` only consults it on
//! Windows, before clap sees the command line.
#![cfg_attr(not(windows), allow(dead_code))]

/// One of the classic Windows screensaver modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsSaverMode {
    /// `/s` — run full-screen, one borderless window per monitor.
    Fullscreen,
    /// `/p <hwnd>` — render into the given host window until it is destroyed.
    Preview(isize),
    /// `/c` — open the settings (we open `gibson.toml` in the default editor).
    Configure,
}

/// Detect a Windows screensaver invocation in `args` (argv without the program
/// name). Returns the first recognized mode; unrelated arguments are skipped,
/// so a desktop launch that happens to contain a stray token still works.
pub fn detect(args: &[String]) -> Option<WindowsSaverMode> {
    let mut rest = args.iter();
    while let Some(raw) = rest.next() {
        let lower = raw.to_ascii_lowercase();
        let mode = match lower.as_str() {
            "/s" | "-s" => Some(WindowsSaverMode::Fullscreen),
            "/c" | "-c" => Some(WindowsSaverMode::Configure),
            "/p" | "-p" => {
                let value = attached_value(raw)
                    .or_else(|| rest.next().map(String::as_str))
                    .and_then(parse_hwnd);
                value.map(WindowsSaverMode::Preview)
            }
            _ => {
                if let Some(value) = strip_prefixes(&lower, &["/c:", "/c=", "-c:", "-c="]) {
                    let _ = value; // the hwnd (if any) is ignored for configure
                    Some(WindowsSaverMode::Configure)
                } else if let Some(value) = strip_prefixes(&lower, &["/p:", "/p=", "-p:", "-p="]) {
                    let value = raw.get(lower.len() - value.len()..);
                    value.and_then(parse_hwnd).map(WindowsSaverMode::Preview)
                } else {
                    None
                }
            }
        };
        if mode.is_some() {
            return mode;
        }
    }
    None
}

/// Value attached to a switch with `:` or `=` (`/p:4242`), when present.
fn attached_value(raw: &str) -> Option<&str> {
    for sep in [':', '='] {
        if let Some((_, v)) = raw.split_once(sep) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn strip_prefixes<'a>(s: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| s.strip_prefix(p))
}

/// Parse an HWND / XID given as a decimal or `0x`-hex integer.
fn parse_hwnd(s: &str) -> Option<isize> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        isize::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<isize>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fullscreen_forms() {
        assert_eq!(detect(&args(&["/s"])), Some(WindowsSaverMode::Fullscreen));
        assert_eq!(detect(&args(&["-s"])), Some(WindowsSaverMode::Fullscreen));
        assert_eq!(detect(&args(&["/S"])), Some(WindowsSaverMode::Fullscreen));
        assert_eq!(
            detect(&args(&["C:\\Windows\\System32\\Gibson.scr", "/s"])),
            Some(WindowsSaverMode::Fullscreen)
        );
    }

    #[test]
    fn preview_forms() {
        assert_eq!(
            detect(&args(&["/p", "4242"])),
            Some(WindowsSaverMode::Preview(4242))
        );
        assert_eq!(
            detect(&args(&["/p:4242"])),
            Some(WindowsSaverMode::Preview(4242))
        );
        assert_eq!(
            detect(&args(&["-p", "0x1092"])),
            Some(WindowsSaverMode::Preview(0x1092))
        );
        assert_eq!(
            detect(&args(&["/p=65552"])),
            Some(WindowsSaverMode::Preview(65552))
        );
    }

    #[test]
    fn configure_forms() {
        assert_eq!(detect(&args(&["/c"])), Some(WindowsSaverMode::Configure));
        assert_eq!(detect(&args(&["-c"])), Some(WindowsSaverMode::Configure));
        assert_eq!(
            detect(&args(&["/c:1234"])),
            Some(WindowsSaverMode::Configure)
        );
        assert_eq!(
            detect(&args(&["/c=43210"])),
            Some(WindowsSaverMode::Configure)
        );
    }

    #[test]
    fn desktop_args_are_not_screensaver_args() {
        assert_eq!(detect(&args(&["--fullscreen"])), None);
        assert_eq!(detect(&args(&["--snapshot", "x.png"])), None);
        assert_eq!(detect(&args(&[])), None);
        assert_eq!(detect(&args(&["--speed", "1.2"])), None);
    }

    #[test]
    fn malformed_preview_value_falls_through() {
        // "/p" with a garbage value is not a valid invocation; treat as absent.
        assert_eq!(detect(&args(&["/p", "not-a-hwnd"])), None);
    }
}
