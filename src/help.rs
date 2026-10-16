//! The help overlay: keyboard shortcuts, syntax by example, and tips.

use eframe::egui::{self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Sense, vec2};

use crate::engine::Engine;
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Keyboard,
    Syntax,
    Tips,
}

impl Tab {
    pub fn from_name(name: &str) -> Self {
        match name {
            "syntax" => Tab::Syntax,
            "tips" => Tab::Tips,
            _ => Tab::Keyboard,
        }
    }
}

pub enum Action {
    Close,
    /// Open the syntax examples as a new sheet to play with.
    OpenExamples,
}

const KEYS: &[(&str, &[(&str, &str)])] = &[
    (
        "Sheets",
        &[
            ("Ctrl+N", "New sheet"),
            ("Ctrl+Tab  /  Ctrl+Shift+Tab", "Next / previous sheet"),
            ("Ctrl+PgDn  /  Ctrl+PgUp", "Next / previous sheet"),
            ("Ctrl+1 … Ctrl+9", "Jump to a sheet"),
            ("Alt+↑  /  Alt+↓", "Move the sheet up / down"),
            ("Ctrl+Delete", "Delete the sheet (kept in trash)"),
            ("Ctrl+B", "Show / hide the sidebar"),
        ],
    ),
    (
        "Results",
        &[
            ("Click a result", "Copy it"),
            ("Ctrl+Shift+C", "Copy the last result"),
        ],
    ),
    (
        "Autocomplete",
        &[
            ("Tab", "Accept the suggestion"),
            ("↑  /  ↓", "Choose a suggestion"),
            ("Esc", "Dismiss suggestions"),
            ("Enter", "Always a new line, never accepts"),
        ],
    ),
    (
        "Editing",
        &[
            ("Ctrl+Z  /  Ctrl+Shift+Z", "Undo / redo (per sheet)"),
            ("Ctrl+A", "Select all"),
            ("Ctrl+Plus  /  Ctrl+Minus", "Zoom in / out"),
            ("Ctrl+0", "Reset zoom"),
        ],
    ),
    (
        "Window",
        &[
            ("F1  /  Ctrl+/", "This help"),
            ("Esc  /  Ctrl+W  /  Ctrl+Q", "Close (everything is saved)"),
            ("Super+Ctrl+Alt+=", "Open Omacus from anywhere (Omarchy)"),
        ],
    ),
];

/// Each group is evaluated as its own little sheet, so variables carry over.
const SYNTAX: &[(&str, &[&str])] = &[
    (
        "Arithmetic",
        &[
            "2^10",
            "sqrt(144) + 10 mod 3",
            "5 x 3",
            "1.5k * 4",
            "10 times 3 plus 2",
        ],
    ),
    (
        "Percentages",
        &[
            "20% of 340",
            "5% on $200",
            "15% off $1,299",
            "$120 as % of $480",
            "5% of what is 6",
        ],
    ),
    (
        "Units",
        &[
            "3.5 km in miles",
            "2 hours + 45 min in minutes",
            "72 fahrenheit in celsius",
            "10 GB / 4",
            "60 mph in km/h",
        ],
    ),
    (
        "Currency",
        &[
            "$30 in EUR",
            "€50 + £20 in USD",
            "₹5,000 in USD",
            "$100 in INR",
        ],
    ),
    (
        "Variables",
        &[
            "rent = $1,450",
            "rent * 12",
            "rate = $85/hour",
            "rate * 30 hours",
        ],
    ),
    (
        "Totals",
        &[
            "Coffee $4.50",
            "Lunch with Sam $18",
            "Taxi $12",
            "total",
            "average",
            "prev * 2",
        ],
    ),
    (
        "Time zones",
        &[
            "3pm PST in IST",
            "time in Tokyo",
            "10:30 London to New York",
            "now in Bangalore",
        ],
    ),
    (
        "Dates",
        &["today", "tomorrow + 2 weeks", "now - 90 minutes"],
    ),
    (
        "Number bases",
        &["255 in hex", "0xff in decimal", "10 in binary"],
    ),
    (
        "Notes",
        &[
            "# A header",
            "// a comment line",
            "Rent: $1,450",
            "\"quoted text\" 40 + 2",
        ],
    ),
];

const TIPS: &[(&str, &str)] = &[
    (
        "Write like notes",
        "Words Omacus doesn't know are ignored, so `Lunch with Sam $18` just works. Put labels before a colon (`Rent: $1,450`) to keep them out of the math entirely.",
    ),
    (
        "Blocks and totals",
        "`sum`, `total` and `average` cover the lines above, back to the last blank line or header. Leave a blank line to start a new block.",
    ),
    (
        "Chain with prev",
        "`prev` is the result of the line above: `prev * 2`, `prev in EUR`.",
    ),
    (
        "Name things",
        "`rent = $1,450` defines a variable for the rest of the sheet. Variables are highlighted, and autocomplete offers them as you type.",
    ),
    (
        "One sheet per thing",
        "Make a sheet per trip, budget or project with Ctrl+N. The first line is the title, so start with `# Goa trip`.",
    ),
    (
        "Nothing to save",
        "Sheets save as you type and reopen with your cursor where you left it. Close with Esc without a second thought.",
    ),
    (
        "Deleted isn't gone",
        "Deleted sheets go to `{data}/trash`. Sheets are plain text files in `{data}/sheets`, so search, sync or back them up.",
    ),
    (
        "Copy fast",
        "Click any result to copy it, or `Ctrl+Shift+C` for the last one, then paste anywhere.",
    ),
    (
        "Time zones",
        "Abbreviations follow daylight saving: `PST` in summer means Pacific time. Cities work too: `9am Bangalore in Berlin`.",
    ),
    (
        "From the terminal",
        "`omacus '$100 in INR'` prints one answer. `cat sheet.txt | omacus -` evaluates a whole sheet.",
    ),
    (
        "Theme",
        "On Omarchy, colors follow your theme and switch with it. Elsewhere Omacus follows the system's light or dark mode. For your own colors, put a `colors.toml`-style file at `{theme}`.",
    ),
];

/// Shortcut names as the platform writes them: `Ctrl+Shift+C` is `⌘⇧C` on macOS.
pub fn keys(s: &str) -> String {
    if cfg!(target_os = "macos") {
        s.replace("Ctrl+", "⌘")
            .replace("Shift+", "⇧")
            .replace("Alt+", "⌥")
    } else {
        s.to_string()
    }
}

pub fn examples_sheet() -> String {
    let mut s =
        String::from("# Omacus examples\n// Play around: edit anything, results update live.\n");
    for (group, lines) in SYNTAX {
        s.push_str(&format!("\n# {group}\n"));
        for line in *lines {
            s.push_str(line);
            s.push('\n');
        }
    }
    s
}

pub fn show(ctx: &egui::Context, tab: &mut Tab, theme: &Theme, engine: &Engine) -> Option<Action> {
    let mut action = None;
    let screen = ctx.content_rect();
    let size = vec2(
        (screen.width() - 80.0).clamp(320.0, 760.0),
        (screen.height() - 60.0).max(200.0),
    );

    let frame = Frame::NONE
        .fill(theme.sidebar)
        .stroke(egui::Stroke::new(1.0, theme.selection))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(24, 18));
    let modal = egui::Modal::new(egui::Id::unique("omacus-help"))
        .frame(frame)
        .backdrop_color(Color32::from_black_alpha(150))
        .show(ctx, |ui| {
            ui.set_width(size.x - 48.0);
            ui.set_max_height(size.y - 36.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("Omacus").size(20.0).color(theme.accent));
                ui.label(
                    RichText::new("notepad calculator")
                        .size(13.0)
                        .color(theme.comment),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close = egui::Button::new(RichText::new("✕").color(theme.dim)).frame(false);
                    if ui.add(close).on_hover_text("Close (Esc)").clicked() {
                        action = Some(Action::Close);
                    }
                });
            });
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                for (t, label) in [
                    (Tab::Keyboard, "Keyboard"),
                    (Tab::Syntax, "Syntax"),
                    (Tab::Tips, "Tips & tricks"),
                ] {
                    if tab_button(ui, theme, label, *tab == t) {
                        *tab = t;
                    }
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .show(ui, |ui| match tab {
                    Tab::Keyboard => keyboard(ui, theme),
                    Tab::Syntax => {
                        if syntax(ui, theme, engine) {
                            action = Some(Action::OpenExamples);
                        }
                    }
                    Tab::Tips => tips(ui, theme),
                });
        });

    if modal.should_close() {
        action.get_or_insert(Action::Close);
    }
    action
}

fn tab_button(ui: &mut egui::Ui, theme: &Theme, label: &str, active: bool) -> bool {
    let font = FontId::proportional(14.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, theme.foreground);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(20.0, 12.0), Sense::click());
    if active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), theme.selection);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), theme.hover);
    }
    let color = if active { theme.accent } else { theme.dim };
    ui.painter()
        .galley(rect.min + vec2(10.0, 6.0), galley, color);
    resp.clicked()
}

/// A fixed-width table cell with its content left-aligned (`add_sized` centers it).
fn left_cell(ui: &mut egui::Ui, size: egui::Vec2, label: egui::Label) {
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(size);
            ui.add(label);
        },
    );
}

fn heading(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.0)
            .color(theme.comment),
    );
    ui.add_space(2.0);
}

fn keyboard(ui: &mut egui::Ui, theme: &Theme) {
    for (section, shortcuts) in KEYS {
        heading(ui, theme, section);
        for (key, what) in *shortcuts {
            // The global hotkey is a Hyprland keybind.
            if key.starts_with("Super") && std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none()
            {
                continue;
            }
            let key = keys(key);
            ui.horizontal(|ui| {
                left_cell(
                    ui,
                    vec2(250.0, 22.0),
                    egui::Label::new(RichText::new(key).monospace().color(theme.keyword))
                        .truncate(),
                );
                ui.label(RichText::new(*what).color(theme.foreground));
            });
        }
    }
}

/// Returns true when "Open as a sheet" is clicked.
fn syntax(ui: &mut egui::Ui, theme: &Theme, engine: &Engine) -> bool {
    let mut open = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Every result below is calculated live.").color(theme.dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            open = ui
                .button(RichText::new("Open as a sheet").color(theme.accent))
                .on_hover_text("Create a new sheet with all these examples")
                .clicked();
        });
    });

    let width = ui.available_width();
    for (group, lines) in SYNTAX {
        heading(ui, theme, group);
        let results = engine.eval_document(&lines.join("\n"));
        for (line, result) in lines.iter().zip(results) {
            ui.horizontal(|ui| {
                left_cell(
                    ui,
                    vec2(width * 0.55, 20.0),
                    egui::Label::new(RichText::new(*line).monospace().color(theme.foreground))
                        .truncate(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(result.display)
                            .monospace()
                            .color(theme.accent),
                    );
                });
            });
        }
    }
    open
}

fn tips(ui: &mut egui::Ui, theme: &Theme) {
    for (title, body) in TIPS {
        ui.add_space(8.0);
        ui.label(RichText::new(*title).size(15.0).color(theme.accent));
        let mut job = egui::text::LayoutJob::default();
        // `code` spans are highlighted.
        let data =
            dirs::data_dir().map_or_else(String::new, |d| d.join("omacus").display().to_string());
        let theme_path =
            Theme::user_theme_path().map_or_else(String::new, |p| p.display().to_string());
        let body = keys(body)
            .replace("{data}", &data)
            .replace("{theme}", &theme_path);
        for (i, part) in body.split('`').enumerate() {
            let (color, family) = if i % 2 == 1 {
                (theme.keyword, egui::FontFamily::Monospace)
            } else {
                (theme.foreground, egui::FontFamily::Proportional)
            };
            job.append(
                part,
                0.0,
                egui::TextFormat::simple(FontId::new(14.0, family), color),
            );
        }
        job.wrap.max_width = ui.available_width();
        ui.label(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rates::Rates;

    #[test]
    fn every_example_has_a_result() {
        let engine = Engine::new(Rates::fixed(&[
            ("USD", 1.0),
            ("EUR", 0.9),
            ("INR", 83.0),
            ("GBP", 0.78),
        ]));
        for (group, lines) in SYNTAX {
            let results = engine.eval_document(&lines.join("\n"));
            for (line, r) in lines.iter().zip(results) {
                let is_note = line.starts_with('#') || line.starts_with("//");
                assert_eq!(
                    r.display.is_empty(),
                    is_note,
                    "{group}: {line:?} -> {:?}",
                    r.display
                );
            }
        }
    }
}
