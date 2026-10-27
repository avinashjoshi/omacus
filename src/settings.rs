//! One place for the choices that used to be a file you edited by hand:
//! update checks, the theme, and whether answers sit in a column.
//!
//! The file is `settings.toml` next to `theme.toml`. A file that only sets
//! `check_for_updates` still loads; the other choices stay at their defaults.
//! The window keeps a copy in memory so a check on every frame does not read
//! the disk.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use eframe::egui;
use serde::Deserialize;

use crate::design;
use crate::theme::{self, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    Auto,
    Light,
    Dark,
    Paper,
    Dusk,
    Omarchy,
    Custom,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 7] = [
        ThemeChoice::Auto,
        ThemeChoice::Light,
        ThemeChoice::Dark,
        ThemeChoice::Paper,
        ThemeChoice::Dusk,
        ThemeChoice::Omarchy,
        ThemeChoice::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Auto => "Auto",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Paper => "Paper",
            ThemeChoice::Dusk => "Dusk",
            ThemeChoice::Omarchy => "Omarchy",
            ThemeChoice::Custom => "Custom",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ThemeChoice::Auto => "auto",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Paper => "paper",
            ThemeChoice::Dusk => "dusk",
            ThemeChoice::Omarchy => "omarchy",
            ThemeChoice::Custom => "custom",
        }
    }

    pub fn parse(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "light" => ThemeChoice::Light,
            "dark" => ThemeChoice::Dark,
            "paper" => ThemeChoice::Paper,
            "dusk" => ThemeChoice::Dusk,
            "omarchy" => ThemeChoice::Omarchy,
            "custom" => ThemeChoice::Custom,
            _ => ThemeChoice::Auto,
        }
    }

    /// One sentence under the picker.
    pub fn blurb(self) -> &'static str {
        match self {
            ThemeChoice::Auto => {
                "Your theme.toml if you have one, then the Omarchy theme, then light or dark to match the system."
            }
            ThemeChoice::Light => "A plain light theme.",
            ThemeChoice::Dark => "A dark theme.",
            ThemeChoice::Paper => "Warm paper, for reading in daylight.",
            ThemeChoice::Dusk => "A blue night theme.",
            ThemeChoice::Omarchy => {
                "Only the active Omarchy theme. It switches when you change that theme."
            }
            ThemeChoice::Custom => "Only your theme.toml. The Omarchy theme is ignored.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub check_for_updates: bool,
    pub theme: ThemeChoice,
    pub hide_answers: bool,
    /// `None` is the OS Downloads folder.
    pub export_dir: Option<PathBuf>,
    /// Overwrite an existing PDF without asking.
    pub replace_exports: bool,
    /// Minutes of no typing before unlocked pages lock again. `0` is never.
    pub lock_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            check_for_updates: true,
            theme: ThemeChoice::Auto,
            hide_answers: false,
            export_dir: None,
            replace_exports: false,
            lock_minutes: 5,
        }
    }
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default = "yes")]
    check_for_updates: bool,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    hide_answers: bool,
    #[serde(default)]
    export_dir: Option<String>,
    #[serde(default)]
    replace_exports: bool,
    #[serde(default = "five_minutes")]
    lock_minutes: u32,
}

fn five_minutes() -> u32 {
    5
}

fn yes() -> bool {
    true
}

impl Settings {
    pub fn parse(src: &str) -> Self {
        let Ok(raw) = toml::from_str::<Raw>(src) else {
            return Self::default();
        };
        Self {
            check_for_updates: raw.check_for_updates,
            theme: raw
                .theme
                .as_deref()
                .map(ThemeChoice::parse)
                .unwrap_or_default(),
            hide_answers: raw.hide_answers,
            export_dir: raw.export_dir.and_then(|s| {
                let s = s.trim();
                if s.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(s))
                }
            }),
            replace_exports: raw.replace_exports,
            lock_minutes: raw.lock_minutes.min(240),
        }
    }

    pub fn to_toml(&self) -> String {
        let mut out = format!(
            "# Omacus settings. theme is auto, light, dark, paper, dusk, omarchy, or custom.\n\
             check_for_updates = {}\n\
             theme = \"{}\"\n\
             hide_answers = {}\n\
             replace_exports = {}\n\
             lock_minutes = {}\n",
            self.check_for_updates,
            self.theme.as_str(),
            self.hide_answers,
            self.replace_exports,
            self.lock_minutes,
        );
        if let Some(dir) = &self.export_dir {
            out.push_str(&format!(
                "export_dir = {}\n",
                toml_string(&dir.display().to_string())
            ));
        }
        out
    }

    /// Choices that follow the person. The export folder stays on this computer.
    pub fn shared_prefs(&self) -> serde_json::Value {
        serde_json::json!({
            "theme": self.theme.as_str(),
            "hideAnswers": self.hide_answers,
            "lockMinutes": self.lock_minutes,
            "replaceExports": self.replace_exports,
            "checkForUpdates": self.check_for_updates,
        })
    }

    /// Apply a shared settings object. Unknown keys, including a path, are ignored.
    pub fn apply_shared_prefs(&mut self, prefs: &serde_json::Value) -> bool {
        let Some(prefs) = prefs.as_object() else {
            return false;
        };
        let mut changed = false;
        if let Some(theme) = prefs.get("theme").and_then(|value| value.as_str()) {
            let next = ThemeChoice::parse(theme);
            if next != self.theme {
                self.theme = next;
                changed = true;
            }
        }
        if let Some(hide) = prefs.get("hideAnswers").and_then(|value| value.as_bool())
            && hide != self.hide_answers
        {
            self.hide_answers = hide;
            changed = true;
        }
        if let Some(minutes) = prefs.get("lockMinutes").and_then(|value| value.as_u64()) {
            let minutes = u32::try_from(minutes).unwrap_or(240).min(240);
            if minutes != self.lock_minutes {
                self.lock_minutes = minutes;
                changed = true;
            }
        }
        if let Some(replace) = prefs
            .get("replaceExports")
            .and_then(|value| value.as_bool())
            && replace != self.replace_exports
        {
            self.replace_exports = replace;
            changed = true;
        }
        if let Some(check) = prefs
            .get("checkForUpdates")
            .and_then(|value| value.as_bool())
            && check != self.check_for_updates
        {
            self.check_for_updates = check;
            changed = true;
        }
        changed
    }
}

fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn memory() -> &'static Mutex<Option<Settings>> {
    static CURRENT: Mutex<Option<Settings>> = Mutex::new(None);
    &CURRENT
}

/// The settings file, next to `theme.toml`.
pub fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("omacus").join("settings.toml"))
}

pub fn load() -> Settings {
    path().map(|p| load_from(&p)).unwrap_or_default()
}

pub fn load_from(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| Settings::parse(&s))
        .unwrap_or_default()
}

/// Remember `settings` for this process. Update checks read this, not the file.
pub fn remember(settings: &Settings) {
    *memory().lock().unwrap_or_else(|e| e.into_inner()) = Some(settings.clone());
}

pub fn current() -> Settings {
    let mut slot = memory().lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(load());
    }
    slot.clone().unwrap_or_default()
}

/// Whether the version check runs. `OMACUS_NO_UPDATE_CHECK` forces it off.
pub fn updates_enabled() -> bool {
    if std::env::var_os("OMACUS_NO_UPDATE_CHECK").is_some() {
        return false;
    }
    current().check_for_updates
}

pub fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, settings.to_toml()).map_err(|e| e.to_string())
}

/// Write `settings.toml` and remember it. The window calls this when a choice changes.
pub fn save(settings: &Settings) -> Result<(), String> {
    remember(settings);
    let Some(path) = path() else {
        return Err("no config folder for settings.toml".into());
    };
    save_to(&path, settings)
}

/// The Settings tab. `true` means a choice moved this frame.
/// `export_everything` is set when that button is pressed.
pub fn controls(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    theme: &Theme,
    export_everything: &mut bool,
) -> bool {
    let before = settings.clone();
    ui.spacing_mut().item_spacing.y = theme::token::SPACE_3;

    group(ui, theme, "Appearance", |ui| {
        let blurb = settings.theme.blurb().to_string();
        labeled(ui, theme, "Theme", &blurb, |ui| {
            egui::ComboBox::from_id_salt("omacus-theme")
                .width(148.0)
                .selected_text(settings.theme.label())
                .show_ui(ui, |ui| {
                    for choice in ThemeChoice::ALL {
                        ui.selectable_value(&mut settings.theme, choice, choice.label());
                    }
                });
        });
        if let Some(missing) = theme::missing_note(settings.theme) {
            caption(ui, theme, missing);
        }
    });

    group(ui, theme, "Answers", |ui| {
        switch_row(
            ui,
            theme,
            "Hide the answers column",
            "Answers sit after each line. Ctrl+\\ toggles this. The preview and PDF follow this too.",
            &mut settings.hide_answers,
        );
    });

    group(ui, theme, "Lock", |ui| {
        let lock_label = match settings.lock_minutes {
            0 => "Never".to_string(),
            1 => "1 minute".to_string(),
            n => format!("{n} minutes"),
        };
        labeled(
            ui,
            theme,
            "Lock again after",
            "Unlocked pages and folders lock again after this long with nothing typed. Quitting locks them too. A forgotten password can't be recovered.",
            |ui| {
                egui::ComboBox::from_id_salt("omacus-lock-minutes")
                    .width(148.0)
                    .selected_text(lock_label)
                    .show_ui(ui, |ui| {
                        for (minutes, label) in [
                            (1, "1 minute"),
                            (5, "5 minutes"),
                            (15, "15 minutes"),
                            (30, "30 minutes"),
                            (60, "60 minutes"),
                            (0, "Never"),
                        ] {
                            ui.selectable_value(&mut settings.lock_minutes, minutes, label);
                        }
                    });
            },
        );
    });

    group(ui, theme, "Export", |ui| {
        let folder = settings
            .export_dir
            .clone()
            .unwrap_or_else(crate::pdf::default_export_dir);
        let where_saved = match &settings.export_dir {
            Some(dir) => dir.display().to_string(),
            None => "Downloads".to_string(),
        };
        labeled(
            ui,
            theme,
            "PDF folder",
            &format!("Each PDF is named after the page and saved in {where_saved}."),
            |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = theme::token::SPACE_2;
                    if design::quiet(ui, theme, "Choose folder…")
                        .on_hover_text("Where Export PDF writes")
                        .clicked()
                    {
                        let mut dialog = rfd::FileDialog::new().set_title("Export PDF folder");
                        if folder.is_dir() {
                            dialog = dialog.set_directory(&folder);
                        }
                        if let Some(dir) = dialog.pick_folder() {
                            settings.export_dir = Some(dir);
                        }
                    }
                    if settings.export_dir.is_some()
                        && design::quiet(ui, theme, "Use Downloads")
                            .on_hover_text("The usual Downloads folder")
                            .clicked()
                    {
                        settings.export_dir = None;
                    }
                });
            },
        );
        ui.add_space(theme::token::SPACE_2);
        if design::quiet(ui, theme, "Export everything")
            .on_hover_text(
                "Pages, folders, settings, and Recently deleted, in one file. Locked pages stay locked.",
            )
            .clicked()
        {
            *export_everything = true;
        }
        ui.add_space(theme::token::SPACE_2);
        switch_row(
            ui,
            theme,
            "Always replace without asking",
            "When that PDF is already there, Omacus asks before replacing it.",
            &mut settings.replace_exports,
        );
    });

    group(ui, theme, "Updates", |ui| {
        switch_row(
            ui,
            theme,
            "Check for updates",
            "Asks GitHub for the latest version number. Nothing about your pages is sent.",
            &mut settings.check_for_updates,
        );
    });

    if let Some(path) = path() {
        caption(ui, theme, &format!("Saved in {}", path.display()));
    }
    settings != &before
}

fn group(ui: &mut egui::Ui, theme: &Theme, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.label(
        egui::RichText::new(title)
            .size(theme::token::TYPE_CAPTION)
            .color(theme.text_muted()),
    );
    egui::Frame::NONE
        .fill(theme.surface())
        .stroke(egui::Stroke::new(1.0, theme.border()))
        .corner_radius(egui::CornerRadius::same(theme::token::RADIUS_LG))
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = theme::token::SPACE_2;
            body(ui);
        });
}

fn labeled(
    ui: &mut egui::Ui,
    theme: &Theme,
    title: &str,
    help: &str,
    control: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal_top(|ui| {
        let width = ui.available_width();
        ui.vertical(|ui| {
            ui.set_max_width((width - 180.0).max(140.0));
            ui.label(
                egui::RichText::new(title)
                    .size(theme::token::TYPE_LABEL)
                    .color(theme.text()),
            );
            caption(ui, theme, help);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), control);
    });
}

fn switch_row(ui: &mut egui::Ui, theme: &Theme, title: &str, help: &str, on: &mut bool) {
    labeled(ui, theme, title, help, |ui| {
        design::toggle(ui, theme, on);
    });
}

fn caption(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(theme::token::TYPE_CAPTION)
            .color(theme.text_muted()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_with_only_the_update_switch_keeps_the_other_defaults() {
        let settings = Settings::parse("check_for_updates = false\n");
        assert!(!settings.check_for_updates);
        assert_eq!(settings.theme, ThemeChoice::Auto);
        assert!(!settings.hide_answers);
        assert!(settings.export_dir.is_none());
        assert!(!settings.replace_exports);
        assert_eq!(settings.lock_minutes, 5);
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert!(Settings::default().check_for_updates);
        assert!(Settings::parse("not toml = [").check_for_updates);
    }

    #[test]
    fn an_unknown_theme_name_is_auto_and_the_rest_survives() {
        let settings =
            Settings::parse("theme = \"neon\"\ncheck_for_updates = false\nhide_answers = true\n");
        assert_eq!(settings.theme, ThemeChoice::Auto);
        assert!(!settings.check_for_updates);
        assert!(settings.hide_answers);
    }

    #[test]
    fn settings_round_trip_through_a_file() {
        let dir = std::env::temp_dir().join(format!(
            "omacus-settings-{}-{}",
            std::process::id(),
            "paper"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("omacus").join("settings.toml");
        let export = dir.join("Northwind");
        let settings = Settings {
            check_for_updates: false,
            theme: ThemeChoice::Paper,
            hide_answers: true,
            export_dir: Some(export.clone()),
            replace_exports: true,
            lock_minutes: 15,
        };
        save_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path), settings);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("theme = \"paper\""));
        assert!(text.contains("hide_answers = true"));
        assert!(text.contains("replace_exports = true"));
        assert!(text.contains("lock_minutes = 15"));
        assert!(text.contains("export_dir = "));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shared_settings_leave_the_export_folder_and_keep_omarchy() {
        let export = PathBuf::from("/tmp/harbor");
        let mut settings = Settings {
            theme: ThemeChoice::Omarchy,
            export_dir: Some(export.clone()),
            ..Settings::default()
        };
        let shared = settings.shared_prefs();
        assert!(shared.get("export_dir").is_none());
        assert!(shared.get("exportDir").is_none());
        assert_eq!(
            shared.get("theme").and_then(|value| value.as_str()),
            Some("omarchy")
        );
        let incoming = serde_json::json!({
            "hideAnswers": true,
            "lockMinutes": 15
        });
        assert!(settings.apply_shared_prefs(&incoming));
        assert_eq!(settings.theme, ThemeChoice::Omarchy);
        assert_eq!(settings.export_dir.as_deref(), Some(export.as_path()));
        assert!(settings.hide_answers);
        assert_eq!(settings.lock_minutes, 15);
        assert!(!settings.apply_shared_prefs(&serde_json::json!({"export_dir": "/tmp/ferry"})));
        assert_eq!(settings.export_dir.as_deref(), Some(export.as_path()));
    }
}
