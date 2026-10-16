//! Colors, in order of preference: the user's own `theme.toml` in the Omacus config
//! directory, the active Omarchy theme, or a built-in light/dark theme that follows the OS.
//! Theme files use Omarchy's `colors.toml` format and are reloaded live.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eframe::egui::Color32;

#[derive(Clone, Debug)]
pub struct Theme {
    pub dark: bool,
    pub background: Color32,
    pub sidebar: Color32,
    pub foreground: Color32,
    pub dim: Color32,
    pub comment: Color32,
    pub accent: Color32,
    pub number: Color32,
    pub keyword: Color32,
    pub variable: Color32,
    pub selection: Color32,
    pub hover: Color32,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            dark: true,
            background: hex("#1a1b26"),
            sidebar: hex("#16161e"),
            foreground: hex("#c0caf5"),
            dim: hex("#787c99"),
            comment: hex("#565f89"),
            accent: hex("#7aa2f7"),
            number: hex("#e0e6ff"),
            keyword: hex("#7dcfff"),
            variable: hex("#bb9af7"),
            selection: hex("#33467c"),
            hover: hex("#24283b"),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            background: hex("#ffffff"),
            sidebar: hex("#f6f8fa"),
            foreground: hex("#24292f"),
            dim: hex("#6e7781"),
            comment: hex("#8c959f"),
            accent: hex("#0969da"),
            number: hex("#1f2328"),
            keyword: hex("#8250df"),
            variable: hex("#cf222e"),
            selection: hex("#ddf4ff"),
            hover: hex("#eaeef2"),
        }
    }
}

fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(cfg) = dirs::config_dir() {
        v.push(cfg.join("omacus/theme.toml"));
    }
    if let Some(state) = dirs::state_dir() {
        v.push(state.join("omarchy/current/theme/colors.toml"));
    }
    if let Some(cfg) = dirs::config_dir() {
        v.push(cfg.join("omarchy/current/theme/colors.toml"));
    }
    v
}

fn read_colors() -> Option<String> {
    candidates()
        .into_iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
}

impl Theme {
    /// `system_dark` picks the built-in theme when no theme file exists.
    pub fn load(system_dark: bool) -> Self {
        read_colors()
            .and_then(|s| Self::parse(&s))
            .unwrap_or_else(|| {
                if system_dark {
                    Self::dark()
                } else {
                    Self::light()
                }
            })
    }

    /// Where a custom theme can be placed.
    pub fn user_theme_path() -> Option<PathBuf> {
        dirs::config_dir().map(|c| c.join("omacus").join("theme.toml"))
    }

    fn parse(src: &str) -> Option<Self> {
        let table: toml::Table = src.parse().ok()?;
        let get = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| table.get(*k).and_then(|v| v.as_str()))
                .and_then(parse_hex)
        };
        let d = Self::default();
        let background = get(&["background"]).unwrap_or(d.background);
        let foreground = get(&["foreground"]).unwrap_or(d.foreground);
        let accent = get(&["accent", "blue", "color4"]).unwrap_or(d.accent);
        let dark = table
            .get("mode")
            .and_then(|v| v.as_str())
            .map_or_else(|| luminance(background) < 0.5, |m| m != "light");
        Some(Self {
            dark,
            background,
            sidebar: get(&["dark_background"]).unwrap_or(mix(background, Color32::BLACK, 0.8)),
            foreground,
            dim: get(&["light_foreground", "color8"]).unwrap_or(mix(foreground, background, 0.6)),
            comment: get(&["dark_foreground"]).unwrap_or(mix(foreground, background, 0.45)),
            accent,
            number: get(&["bright_foreground", "color15"]).unwrap_or(foreground),
            keyword: get(&["cyan", "color6"]).unwrap_or(accent),
            variable: get(&["magenta", "color5"]).unwrap_or(accent),
            selection: get(&["selection", "selection_background"])
                .unwrap_or(mix(accent, background, 0.3)),
            hover: get(&["lighter_background"]).unwrap_or(mix(foreground, background, 0.08)),
        })
    }
}

/// Polls the theme file and raises `changed` when its contents differ.
pub fn watch(on_change: impl Fn() + Send + 'static) -> Arc<AtomicBool> {
    let changed = Arc::new(AtomicBool::new(false));
    let flag = changed.clone();
    std::thread::spawn(move || {
        let mut last = read_colors();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let now = read_colors();
            if now != last {
                last = now;
                flag.store(true, Ordering::Relaxed);
                on_change();
            }
        }
    });
    changed
}

fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.trim().trim_start_matches('#').trim_start_matches("0x");
    let n = u32::from_str_radix(s.get(..6)?, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

fn hex(s: &str) -> Color32 {
    parse_hex(s).unwrap()
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (f32::from(x) * t + f32::from(y) * (1.0 - t)).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn luminance(c: Color32) -> f32 {
    (0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b())) / 255.0
}
