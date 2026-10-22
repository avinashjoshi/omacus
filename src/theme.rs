//! Colors. Auto uses, in order: the user's `theme.toml`, the active Omarchy theme,
//! then a built-in light or dark theme that follows the OS. A built-in choice
//! (Light, Dark, Paper, Dusk) ignores those files. Theme files use Omarchy's
//! `colors.toml` format and are reloaded live.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eframe::egui::Color32;

use crate::settings::ThemeChoice;

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

    /// Warm paper. A light theme that is not the plain white one.
    pub fn paper() -> Self {
        Self {
            dark: false,
            background: hex("#f6f1e7"),
            sidebar: hex("#efe6d6"),
            foreground: hex("#3c3226"),
            dim: hex("#8a7b68"),
            comment: hex("#a89880"),
            accent: hex("#b45309"),
            number: hex("#2a2118"),
            keyword: hex("#0f766e"),
            variable: hex("#9f1239"),
            selection: hex("#eadcc4"),
            hover: hex("#e7dcc8"),
        }
    }

    /// Blue night. A dark theme that is not the built-in Tokyo Night dark.
    pub fn dusk() -> Self {
        Self {
            dark: true,
            background: hex("#1c2433"),
            sidebar: hex("#151b28"),
            foreground: hex("#d7e0f0"),
            dim: hex("#8b9bb3"),
            comment: hex("#64748b"),
            accent: hex("#8eb4ff"),
            number: hex("#f1f5ff"),
            keyword: hex("#7ddea0"),
            variable: hex("#f0a27a"),
            selection: hex("#2c3d5c"),
            hover: hex("#243044"),
        }
    }
}

fn user_path() -> Option<PathBuf> {
    dirs::config_dir().map(|c| c.join("omacus").join("theme.toml"))
}

fn omarchy_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(state) = dirs::state_dir() {
        v.push(state.join("omarchy/current/theme/colors.toml"));
    }
    if let Some(cfg) = dirs::config_dir() {
        v.push(cfg.join("omarchy/current/theme/colors.toml"));
    }
    v
}

fn read_file(path: Option<PathBuf>) -> Option<String> {
    std::fs::read_to_string(path?).ok()
}

fn user_colors() -> Option<String> {
    read_file(user_path())
}

fn omarchy_colors() -> Option<String> {
    omarchy_paths()
        .into_iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
}

fn candidates() -> Vec<PathBuf> {
    user_path().into_iter().chain(omarchy_paths()).collect()
}

fn read_colors() -> Option<String> {
    candidates()
        .into_iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
}

impl Theme {
    /// The theme for `choice`. Built-ins ignore theme files. Omarchy and Custom
    /// fall back to the system theme when their file is missing. Auto uses
    /// `theme.toml`, else the Omarchy theme, else the system light or dark theme.
    pub fn resolve(choice: ThemeChoice, system_dark: bool) -> Self {
        Self::from_sources(
            choice,
            system_dark,
            user_colors().as_deref(),
            omarchy_colors().as_deref(),
        )
    }

    pub(crate) fn from_sources(
        choice: ThemeChoice,
        system_dark: bool,
        user: Option<&str>,
        omarchy: Option<&str>,
    ) -> Self {
        let system = if system_dark {
            Self::dark()
        } else {
            Self::light()
        };
        let parsed = |src: Option<&str>| src.and_then(Self::parse);
        match choice {
            ThemeChoice::Light => Self::light(),
            ThemeChoice::Dark => Self::dark(),
            ThemeChoice::Paper => Self::paper(),
            ThemeChoice::Dusk => Self::dusk(),
            ThemeChoice::Auto => parsed(user).or_else(|| parsed(omarchy)).unwrap_or(system),
            ThemeChoice::Omarchy => parsed(omarchy).unwrap_or(system),
            ThemeChoice::Custom => parsed(user).unwrap_or(system),
        }
    }

    /// Where a custom theme can be placed.
    pub fn user_theme_path() -> Option<PathBuf> {
        user_path()
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

/// A sentence when Omarchy or Custom was chosen and that file is not there.
pub fn missing_note(choice: ThemeChoice) -> Option<&'static str> {
    match choice {
        ThemeChoice::Omarchy if omarchy_colors().and_then(|s| Theme::parse(&s)).is_none() => {
            Some("No Omarchy theme found. Using the system's light or dark theme until one is.")
        }
        ThemeChoice::Custom if user_colors().and_then(|s| Theme::parse(&s)).is_none() => {
            Some("No theme.toml found. Using the system's light or dark theme until you add one.")
        }
        _ => None,
    }
}

/// Polls the theme files and raises `changed` when their contents differ.
/// A built-in theme ignores the new colors; Auto, Omarchy and Custom pick them up.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_themes_differ_and_ignore_theme_files() {
        let themes = [Theme::light(), Theme::dark(), Theme::paper(), Theme::dusk()];
        let mut seen = Vec::new();
        for theme in &themes {
            assert!(
                !seen.contains(&theme.background),
                "two built-ins share a background"
            );
            seen.push(theme.background);
        }
        assert!(!Theme::paper().dark);
        assert!(Theme::dusk().dark);
        assert_eq!(Theme::paper().background, hex("#f6f1e7"));
        assert_eq!(Theme::dusk().background, hex("#1c2433"));

        let user = "background = \"#112233\"\nmode = \"dark\"\n";
        let omarchy = "background = \"#445566\"\nmode = \"dark\"\n";
        let light = Theme::from_sources(ThemeChoice::Light, true, Some(user), Some(omarchy));
        assert_eq!(light.background, Theme::light().background);
        let paper = Theme::from_sources(ThemeChoice::Paper, true, Some(user), Some(omarchy));
        assert_eq!(paper.background, Theme::paper().background);
        let dusk = Theme::from_sources(ThemeChoice::Dusk, false, Some(user), Some(omarchy));
        assert_eq!(dusk.background, Theme::dusk().background);
    }

    #[test]
    fn auto_prefers_a_custom_file_then_omarchy_then_the_system() {
        let user = "background = \"#112233\"\nmode = \"dark\"\n";
        let omarchy = "background = \"#445566\"\nmode = \"dark\"\n";
        let auto = Theme::from_sources(ThemeChoice::Auto, true, Some(user), Some(omarchy));
        assert_eq!(auto.background, Theme::parse(user).unwrap().background);
        let only_omarchy = Theme::from_sources(ThemeChoice::Auto, true, None, Some(omarchy));
        assert_eq!(
            only_omarchy.background,
            Theme::parse(omarchy).unwrap().background
        );
        let system = Theme::from_sources(ThemeChoice::Auto, false, None, None);
        assert_eq!(system.background, Theme::light().background);
        let dark = Theme::from_sources(ThemeChoice::Auto, true, None, None);
        assert_eq!(dark.background, Theme::dark().background);

        // Custom ignores the Omarchy file, and falls back when theme.toml is missing.
        let custom = Theme::from_sources(ThemeChoice::Custom, true, None, Some(omarchy));
        assert_eq!(custom.background, Theme::dark().background);
        let own = Theme::from_sources(ThemeChoice::Custom, false, Some(user), Some(omarchy));
        assert_eq!(own.background, Theme::parse(user).unwrap().background);

        // Omarchy ignores theme.toml, and falls back when no Omarchy theme is installed.
        let om = Theme::from_sources(ThemeChoice::Omarchy, false, Some(user), None);
        assert_eq!(om.background, Theme::light().background);
        let from_omarchy =
            Theme::from_sources(ThemeChoice::Omarchy, false, Some(user), Some(omarchy));
        assert_eq!(
            from_omarchy.background,
            Theme::parse(omarchy).unwrap().background
        );
    }
}
