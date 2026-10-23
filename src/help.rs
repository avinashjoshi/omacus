//! The help overlay: keyboard shortcuts, syntax by example, tips, and About
//! (version, updates and links).

use eframe::egui::{self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Sense, vec2};

use crate::settings::Settings;
use crate::theme::Theme;
use crate::update::{self, Check, Updates};
use omacus_engine::Engine;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Keyboard,
    Syntax,
    Tips,
    About,
    Settings,
}

impl Tab {
    pub fn from_name(name: &str) -> Self {
        match name {
            "syntax" => Tab::Syntax,
            "tips" => Tab::Tips,
            "about" => Tab::About,
            "settings" => Tab::Settings,
            _ => Tab::Keyboard,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Tab::Keyboard => "Keyboard",
            Tab::Syntax => "Syntax",
            Tab::Tips => "Tips & tricks",
            Tab::About => "About",
            Tab::Settings => "Settings",
        }
    }
}

pub enum Action {
    Close,
    /// Open the syntax examples as a new sheet to play with.
    OpenExamples,
    /// Open the update window.
    OpenUpdate,
    /// A choice on the Settings tab moved.
    SettingsChanged,
}

const KEYS: &[(&str, &[(&str, &str)])] = &[
    (
        "Pages",
        &[
            ("Ctrl+N", "New page"),
            ("Ctrl+J", "Go to a page by name"),
            ("Ctrl+Alt+↑  /  Ctrl+Alt+↓", "Previous / next page"),
            ("Ctrl+Tab  /  Ctrl+Shift+Tab", "Next / previous page"),
            ("Ctrl+PgDn  /  Ctrl+PgUp", "Next / previous page"),
            ("Ctrl+1 … Ctrl+9", "Jump to a page"),
            ("Alt+↑  /  Alt+↓", "Move the page up / down"),
            (
                "Ctrl+Shift+Delete",
                "Delete the page (asks first; kept in Recently deleted)",
            ),
            ("Ctrl+B", "Expand / collapse the sidebar"),
        ],
    ),
    (
        "Results",
        &[
            ("Click a result", "Copy it, with a short confirmation"),
            ("Hover a name or number", "See its value"),
            (
                "Ctrl+\\",
                "Hide the answers column (answers sit after each line)",
            ),
            ("Ctrl+Shift+C", "Copy the last result"),
            ("Ctrl+Shift+S", "Copy the page with its answers"),
            ("Ctrl+Shift+M", "Copy the page as a Markdown table"),
            ("Ctrl+Shift+L", "Copy a share link to the page"),
            (
                "Ctrl+P",
                "Preview the page. Print and Export PDF use that view",
            ),
        ],
    ),
    (
        "Autocomplete",
        &[
            ("Tab", "Accept the suggestion"),
            ("Ctrl+Space", "Open suggestions. Enter then accepts"),
            ("↑  /  ↓", "Choose a suggestion"),
            ("Esc", "Dismiss suggestions"),
            (
                "Enter",
                "A new line, unless suggestions were opened with Ctrl+Space",
            ),
        ],
    ),
    (
        "Editing",
        &[
            ("Ctrl+Z  /  Ctrl+Shift+Z", "Undo / redo (per page)"),
            ("Ctrl+A", "Select all"),
            ("Ctrl+Plus  /  Ctrl+Minus", "Zoom in / out"),
            ("Ctrl+0", "Reset zoom"),
        ],
    ),
    (
        "Window",
        &[
            ("F1  /  Ctrl+/", "This help"),
            (
                "Ctrl+,",
                "Settings: theme, answers column, export folder, update checks",
            ),
            ("Esc", "Close the switcher, preview, help, or a dialog"),
            ("Ctrl+W  /  Ctrl+Q", "Quit (everything is saved)"),
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
            "2.5 lakh / 12",
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
            "18% tip on $62",
            "10% off 20% off $100",
            "$84 split 3 ways",
        ],
    ),
    (
        "Units",
        &[
            "3.5 km in miles",
            "2 hours + 45 min in minutes",
            "72 fahrenheit in celsius",
            "10 GB / 4",
            "4 GB at 100 Mbps",
            "60 mph in km/h",
            "2 imperial pints in ml",
        ],
    ),
    (
        "Currency",
        &[
            "$30 in EUR",
            "€50 + £20 in USD",
            "£250 in USD",
            "$100 in JPY",
            "C$100 in USD",
            "€12,50 * 2",
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
            "sum(10, 20, 30)",
            "max($5, $12)",
        ],
    ),
    (
        "Time zones",
        &[
            "6pm PST in Tokyo",
            "3pm PST on Dec 1 in London",
            "in 90 minutes in Tokyo",
            "3pm + 45 min",
            "time in Sydney",
            "10:30 London to New York",
            "now in Berlin",
        ],
    ),
    (
        "Dates",
        &[
            "today",
            "tomorrow + 2 weeks",
            "now - 90 minutes",
            "days until Dec 25",
            "workdays in November",
            "days since Jan 1",
            "Jan 31 + 1 month",
            "Dec 25 - Dec 1",
            "next Friday",
        ],
    ),
    (
        "Number bases",
        &["255 in hex", "0xff in decimal", "10 in binary"],
    ),
    (
        "Design",
        &[
            "16px in rem",
            "12pt in px",
            "#ff8800 in rgb",
            "rgb(51, 102, 153) in hex",
            "hsl(210, 50%, 40%)",
        ],
    ),
    (
        "Notes",
        &[
            "# A header",
            "// a comment line",
            "Rent: $1,450",
            "\"quoted text\" 40 + 2",
            "- hotel = 3 nights * $180",
            "hotel * 2",
            "rent = $100 # monthly",
        ],
    ),
];

const TIPS: &[(&str, &str)] = &[
    (
        "Write like notes",
        "Words Omacus doesn't know are ignored, so `Lunch with Sam $18` just works. A markdown prefix comes off first: `- hotel = 3 nights * $180` sets hotel. `#` at the start of a line is a heading; `#` later on the line is a comment (`rent = $100 # monthly`), drawn in the comment colour. Put labels before a colon (`Rent: $1,450`) to keep them out of the math entirely.",
    ),
    (
        "Blocks and totals",
        "`sum`, `total` and `average` cover the lines above, back to the last blank line or prose heading, and work in a calculation too (`total * 1.08`). A list, a quote or a task stays in the block. `sum(1, 2, 3)`, `avg(2, 4)`, `min` and `max` take arguments. Leave a blank line to start a new block.",
    ),
    (
        "Chain with prev",
        "`prev` is the result of the line above: `prev * 2`, `prev in EUR`.",
    ),
    (
        "Name things",
        "`rent = $1,450` defines a variable for the rest of the page. The name where you define it is one colour, a later use is another, and keywords and units have a third. Hover a name or a number (`1.5k`) to see its value. Autocomplete offers names as you type, with a short description.",
    ),
    (
        "One page per thing",
        "Make a page per trip, budget or project with Ctrl+N. The first line is the title, so start with `# Kyoto trip`.",
    ),
    (
        "Nothing to save",
        "Pages save as you type and reopen with your cursor where you left it. Quit with Ctrl+Q or Ctrl+W. Esc only closes the switcher, the preview, help, or a dialog.",
    ),
    (
        "Preview and print",
        "`Ctrl+P` shows the page as rendered notes. Each source line stays its own line, a blank line stays blank, and a `#` or `//` comment is a quiet note on that line. With the answers column on, each answer sits in a column on the right; with it off, the answer follows the line. Print and Export PDF use that view. Esc returns to editing. Tables are drawn, and a cell is not calculated.",
    ),
    (
        "Deleted isn't gone",
        "Deleted pages go to `{data}/trash`. Pages are plain text files in `{data}/pages`, so search, sync or back them up.",
    ),
    (
        "Copy a page",
        "`Ctrl+Shift+S` copies the whole page with each answer lined up after its line, ready to paste into a message or a doc.",
    ),
    (
        "Jump between pages",
        "`Ctrl+J` and a few letters of a page's name (or something written in it), then Enter. Type `settings` to open Settings. `Ctrl+Alt+↑` and `Ctrl+Alt+↓` move to the previous or next page.",
    ),
    (
        "Copy fast",
        "Click any result to copy it, or `Ctrl+Shift+C` for the last one, then paste anywhere.",
    ),
    (
        "Share a page",
        "Share (top right, or `Ctrl+Shift+L`) copies a link that opens the page, live, in any browser at omacus.com/app. The page is inside the link; nothing is uploaded. Paste a share link into a page to open it here.",
    ),
    (
        "Time zones",
        "Abbreviations follow daylight saving: `PST` in summer means Pacific time. Cities work too: `9am Chicago in Berlin`.",
    ),
    (
        "From the terminal",
        "`omacus '$100 in EUR'` prints one answer. `cat page.txt | omacus -` evaluates a whole page.",
    ),
    (
        "Theme",
        "Settings is the first tab here. `Ctrl+,`, or the gear at the bottom of the sidebar (and of the collapsed strip), opens it. It picks the theme: Light, Dark, Paper, Dusk, the Omarchy theme, or your own file at `{theme}`. Auto uses that file if it exists, then the Omarchy theme, then the system's light or dark mode. `Ctrl+\\` hides the answers column so each answer sits after its line. Export PDF writes to the folder chosen there (Downloads by default), named after the page. If that file is already there, Omacus asks before replacing it.",
    ),
];

/// Shortcut names as the platform writes them: `Ctrl+Shift+C` is `⌘⇧C` on macOS.
pub fn keys(s: &str) -> String {
    if cfg!(target_os = "macos") {
        // ⌘Space is Spotlight. This shortcut is the Control key on every platform.
        s.replace("Ctrl+Space", "⌃Space")
            .replace("Ctrl+", "⌘")
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

/// The app icon at two sizes, so it's crisp both at 16px and in About.
pub struct Logo {
    pub small: egui::TextureHandle,
    pub large: egui::TextureHandle,
}

const SITE: &str = "https://omacus.com";
const REPO: &str = "https://github.com/avinashjoshi/omacus";

/// The help window: a rail of sections on the left, the section on the right.
pub fn show(
    ctx: &egui::Context,
    tab: &mut Tab,
    theme: &Theme,
    engine: &Engine,
    logo: &Logo,
    updates: &Updates,
    settings: &mut Settings,
) -> Option<Action> {
    let mut action = None;
    let screen = ctx.content_rect();
    let size = vec2(
        (screen.width() - 80.0).clamp(360.0, 860.0),
        (screen.height() - 60.0).clamp(240.0, 640.0),
    );
    let available = updates.available();
    let rail_width = if size.x < 560.0 { 130.0 } else { 172.0 };

    let frame = Frame::NONE
        .fill(theme.sidebar)
        .stroke(egui::Stroke::new(1.0, theme.selection))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(0));
    let modal = egui::Modal::new(egui::Id::unique("omacus-help"))
        .frame(frame)
        .backdrop_color(Color32::from_black_alpha(150))
        .show(ctx, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.horizontal_top(|ui| {
                // The rail.
                Frame::NONE
                    .inner_margin(Margin {
                        left: 14,
                        right: 10,
                        top: 18,
                        bottom: 14,
                    })
                    .show(ui, |ui| {
                        ui.set_width(rail_width - 24.0);
                        ui.set_height(size.y - 32.0);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                ui.add(egui::Image::new((logo.small.id(), vec2(22.0, 22.0))));
                                ui.add_space(8.0);
                                ui.label(RichText::new("Omacus").size(17.0).color(theme.accent));
                            });
                            ui.add_space(18.0);
                            for t in [
                                Tab::Settings,
                                Tab::Keyboard,
                                Tab::Syntax,
                                Tab::Tips,
                                Tab::About,
                            ] {
                                let dot = t == Tab::About && available.is_some();
                                if rail_button(ui, theme, t.title(), *tab == t, dot) {
                                    *tab = t;
                                }
                                ui.add_space(2.0);
                            }
                            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                                let version = egui::Button::new(
                                    RichText::new(format!("v{}", update::current()))
                                        .size(12.0)
                                        .color(theme.comment),
                                )
                                .frame(false);
                                if ui.add(version).on_hover_text("About Omacus").clicked() {
                                    *tab = Tab::About;
                                }
                            });
                        });
                    });

                // A hairline between the rail and the section.
                let (line, _) = ui.allocate_exact_size(vec2(1.0, size.y), Sense::hover());
                ui.painter()
                    .rect_filled(line, CornerRadius::ZERO, theme.selection);

                Frame::NONE
                    .fill(theme.background)
                    .corner_radius(CornerRadius {
                        ne: 12,
                        se: 12,
                        ..Default::default()
                    })
                    .inner_margin(Margin {
                        left: 28,
                        right: 20,
                        top: 18,
                        bottom: 14,
                    })
                    .show(ui, |ui| {
                        ui.set_width(size.x - rail_width - 1.0 - 48.0);
                        ui.set_height(size.y - 32.0);
                        ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(tab.title())
                                        .size(19.0)
                                        .color(theme.foreground),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let close =
                                            egui::Button::new(RichText::new("✕").color(theme.dim))
                                                .frame(false);
                                        if ui.add(close).on_hover_text("Close (Esc)").clicked() {
                                            action = Some(Action::Close);
                                        }
                                    },
                                );
                            });
                            ui.add_space(12.0);
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| match tab {
                                    Tab::Keyboard => keyboard(ui, theme),
                                    Tab::Syntax => {
                                        if syntax(ui, theme, engine) {
                                            action = Some(Action::OpenExamples);
                                        }
                                    }
                                    Tab::Tips => tips(ui, theme),
                                    Tab::Settings => {
                                        if crate::settings::controls(ui, settings, theme) {
                                            action = Some(Action::SettingsChanged);
                                        }
                                    }
                                    Tab::About => {
                                        if about(ui, theme, logo, updates, available.as_deref()) {
                                            action = Some(Action::OpenUpdate);
                                        }
                                    }
                                });
                        });
                    });
            });
        });

    if modal.should_close() {
        action.get_or_insert(Action::Close);
    }
    action
}

fn rail_button(ui: &mut egui::Ui, theme: &Theme, label: &str, active: bool, dot: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, CornerRadius::same(6), theme.selection);
    } else if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), theme.hover);
    }
    let color = if active { theme.foreground } else { theme.dim };
    let text = painter.text(
        rect.left_center() + vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        color,
    );
    if dot {
        painter.circle_filled(text.right_center() + vec2(9.0, 0.0), 3.0, theme.accent);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

/// The About section. Returns true when the user wants to install an update.
fn about(
    ui: &mut egui::Ui,
    theme: &Theme,
    logo: &Logo,
    updates: &Updates,
    available: Option<&str>,
) -> bool {
    let mut install = false;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add(egui::Image::new((logo.large.id(), vec2(56.0, 56.0))));
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Omacus").size(22.0).color(theme.accent));
            ui.label(
                RichText::new("Math, the way you write it.")
                    .size(13.5)
                    .color(theme.dim),
            );
        });
    });
    ui.add_space(22.0);

    // Version and updates.
    ui.label(
        RichText::new(format!("Version {}", update::current()))
            .size(14.0)
            .color(theme.foreground),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
        // Only a check that's still in flight hides the button. A finished
        // result (up to date, an update, or an error) always offers another.
        if updates.checked() == Check::Checking {
            ui.spinner();
            ui.label(RichText::new("Checking…").size(13.5).color(theme.dim));
            return;
        }
        if let Some(new) = available {
            ui.label(
                RichText::new(format!("Version {new} is available."))
                    .size(13.5)
                    .color(theme.accent),
            );
            install = primary_button(ui, theme, "Update now").clicked();
            if quiet_button(ui, theme, "What's new").clicked() {
                ui.ctx()
                    .open_url(egui::OpenUrl::new_tab(update::notes_url(new)));
            }
        } else {
            match updates.checked() {
                Check::Done => {
                    ui.label(
                        RichText::new("✓  You're up to date.")
                            .size(13.5)
                            .color(theme.dim),
                    );
                }
                Check::Failed => {
                    ui.label(
                        RichText::new("Couldn't reach GitHub.")
                            .size(13.5)
                            .color(theme.dim),
                    );
                }
                Check::Idle | Check::Checking => {}
            }
        }
        let label = if updates.checked() == Check::Idle {
            "Check for updates"
        } else {
            "Check again"
        };
        if quiet_button(ui, theme, label).clicked() {
            let ctx = ui.ctx().clone();
            updates.check_now(move || ctx.request_repaint());
        }
    });
    ui.add_space(26.0);

    // What Omacus keeps and sends; the long version is omacus.com/privacy.
    ui.label(
        RichText::new("Private by design")
            .size(14.0)
            .color(theme.foreground),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new(
            "Your pages stay on this computer, as plain text files. There's no account and \
             no tracking. Omacus goes online for two things only: exchange rates, about \
             twice a day and saved for offline use, and checking GitHub for a new version. \
             Neither sends anything you've written.",
        )
        .size(13.5)
        .color(theme.dim),
    );
    ui.add_space(26.0);

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(18.0, 6.0);
        for (label, url) in [
            ("Website", SITE.to_string()),
            ("Docs", format!("{SITE}/docs/")),
            ("Releases", update::RELEASES_URL.to_string()),
            ("Privacy", format!("{SITE}/privacy/")),
            ("Report an issue", format!("{REPO}/issues/new/choose")),
            ("Source code", REPO.to_string()),
        ] {
            let link =
                egui::Button::new(RichText::new(label).size(13.5).color(theme.accent)).frame(false);
            let resp = ui.add(link);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
    });
    ui.add_space(26.0);
    ui.label(
        RichText::new("Free and open source under the MIT license. Math by fend.")
            .size(12.5)
            .color(theme.comment),
    );
    install
}

fn primary_button(ui: &mut egui::Ui, theme: &Theme, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(13.5).color(theme.background))
            .fill(theme.accent)
            .corner_radius(CornerRadius::same(6))
            .min_size(vec2(0.0, 30.0)),
    )
}

fn quiet_button(ui: &mut egui::Ui, theme: &Theme, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(13.5).color(theme.foreground))
            .corner_radius(CornerRadius::same(6))
            .min_size(vec2(0.0, 30.0)),
    )
}

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

/// Returns true when "Open as a page" is clicked.
fn syntax(ui: &mut egui::Ui, theme: &Theme, engine: &Engine) -> bool {
    let mut open = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Every result below is calculated live.").color(theme.dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            open = ui
                .button(RichText::new("Open as a page").color(theme.accent))
                .on_hover_text("Create a new page with all these examples")
                .clicked();
        });
    });

    // The examples' answers, worked out once a minute rather than every frame (the
    // minute keeps the clock examples current).
    let minute = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 60);
    let results: Vec<Vec<String>> = ui.ctx().memory_mut(|m| {
        m.data
            .get_temp_mut_or_insert_with(egui::Id::unique(("omacus-help-answers", minute)), || {
                SYNTAX
                    .iter()
                    .map(|(_, lines)| {
                        engine
                            .eval_document(&lines.join("\n"))
                            .into_iter()
                            .map(|r| r.display)
                            .collect()
                    })
                    .collect::<Vec<Vec<String>>>()
            })
            .clone()
    });
    let width = ui.available_width();
    for ((group, lines), results) in SYNTAX.iter().zip(&results) {
        heading(ui, theme, group);
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
                        RichText::new(result.as_str())
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
    use omacus_engine::Rates;

    #[test]
    fn help_says_page_not_sheet() {
        let mut text = String::new();
        for (group, rows) in KEYS {
            text.push_str(group);
            for (keys, what) in *rows {
                text.push_str(keys);
                text.push_str(what);
            }
        }
        for (title, body) in TIPS {
            text.push_str(title);
            text.push_str(body);
        }
        text.push_str("Open as a page");
        text.push_str("Your pages stay on this computer");
        assert!(
            !text.to_lowercase().contains("sheet"),
            "help still says sheet: {text}"
        );
        assert!(
            text.contains("Ctrl+Space"),
            "help should say how to accept a suggestion with Enter"
        );
        assert!(
            text.contains("comment colour"),
            "help should say that a # comment is coloured"
        );
    }

    #[test]
    fn every_example_has_a_result() {
        let engine = Engine::new(Rates::fixed(&[
            ("USD", 1.0),
            ("EUR", 0.9),
            ("INR", 83.0),
            ("GBP", 0.78),
            ("JPY", 150.0),
            ("CAD", 1.36),
        ]));
        for (group, lines) in SYNTAX {
            let results = engine.eval_document(&lines.join("\n"));
            for (line, r) in lines.iter().zip(results) {
                let is_note = omacus_engine::is_header(line) || line.starts_with("//");
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
