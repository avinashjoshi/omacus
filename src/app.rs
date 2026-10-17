//! The calculator window: a sidebar of sheets, the sheet text, and live results on the right.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Key,
    Margin, Modifiers, Order, Pos2, Rect, Sense, TextEdit, Vec2, ViewportCommand, pos2, vec2,
};

use crate::calc::Calculator;
use crate::help;
use crate::highlight;
use crate::rates::Rates;
use crate::sheets::Store;
use crate::theme::{self, Theme};
use crate::update::{self, Method, State, Updates};
use omacus_engine::{self as engine, Engine, Kind, LineResult, complete};

const FONT_SIZE: f32 = 16.0;
const SIDEBAR_WIDTH: f32 = 210.0;
const GAP: f32 = 32.0;
const SCROLLBAR_ROOM: f32 = 12.0;
const SAVE_DELAY: Duration = Duration::from_millis(400);
const COPIED_FOR: Duration = Duration::from_millis(900);

const WELCOME: &str = "\
# Welcome to Omacus
// Type on the left, results appear on the right.
// Click a result to copy it. Press F1 for help and shortcuts.

Rent: $1,450
Groceries $320 + $85
Internet €40 in USD
total

rate = $85/hour
rate * 6 hours * 4
prev in INR

20% of 340
15% off $1,299
$120 as % of $480

3.5 km in miles
2 hours + 45 min in minutes
72 fahrenheit in celsius
10 GB / 4

3pm PST in IST
time in Tokyo
today + 3 weeks
";

/// The autocomplete popup.
struct Popup {
    /// Char range of the word being completed.
    start: usize,
    end: usize,
    items: Vec<String>,
    selected: usize,
    anchor: Pos2,
}

enum SheetAction {
    Select(usize),
    New,
    Delete(usize),
    Move(usize, isize),
    /// Copy a sheet as text with its answers lined up.
    Copy(usize),
    /// Copy a sheet as a Markdown table.
    CopyMarkdown(usize),
    /// Copy a link that opens the sheet in a browser (or in Omacus).
    Share(usize),
}

/// A message at the bottom of the window.
struct Toast {
    text: String,
    at: Instant,
    /// Offers Undo for a deleted sheet.
    undo: bool,
    error: bool,
}

impl Toast {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            at: Instant::now(),
            undo: false,
            error: false,
        }
    }
    fn undo(text: String) -> Self {
        Self {
            undo: true,
            ..Self::info(text)
        }
    }
    fn error(text: String) -> Self {
        Self {
            error: true,
            ..Self::info(text)
        }
    }
    fn lasts(&self) -> Duration {
        Duration::from_millis(if self.undo {
            6000
        } else if self.error {
            5000
        } else {
            1600
        })
    }
}

/// Ctrl+P: find a sheet by name (or by something written in it).
#[derive(Default)]
struct Switcher {
    query: String,
    selected: usize,
}

/// Sheets matching the switcher's query, best first: title starts with it, title
/// contains it, title has its letters in order, then sheets that mention it.
fn find_sheets(titles: &[String], texts: &[&str], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..titles.len()).collect();
    }
    let in_order = |hay: &str| {
        let mut chars = hay.chars();
        q.chars()
            .filter(|c| !c.is_whitespace())
            .all(|c| chars.any(|h| h == c))
    };
    let mut ranked: Vec<(u8, usize)> = titles
        .iter()
        .zip(texts)
        .enumerate()
        .filter_map(|(i, (title, text))| {
            let title = title.to_lowercase();
            let rank = if title.starts_with(&q) {
                0
            } else if title.contains(&q) {
                1
            } else if in_order(&title) {
                2
            } else if text.to_lowercase().contains(&q) {
                3
            } else {
                return None;
            };
            Some((rank, i))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().map(|(_, i)| i).collect()
}

pub struct OmacusApp {
    store: Store,
    results: Vec<LineResult>,
    /// The text `results` are the answers for (the sheet may be ahead of it).
    results_text: String,
    vars: HashSet<String>,
    /// For one-off answers (Copy with answers); the sheet itself is calculated by
    /// `calc`, off this thread.
    engine: Engine,
    calc: Calculator,
    rates: Rates,
    rates_seen: u64,
    theme: Theme,
    theme_changed: Arc<AtomicBool>,
    system_dark: bool,
    dirty_since: Option<Instant>,
    copied: Option<(usize, Instant)>,
    /// A short confirmation at the bottom of the window ("Sheet copied…").
    toast: Option<Toast>,
    /// When a save last failed, so the error isn't repeated every 400ms.
    save_error_at: Option<Instant>,
    /// When another window's changes were last looked for, and whether this one had focus.
    synced_at: Instant,
    focused: bool,
    switcher: Option<Switcher>,
    ctx: egui::Context,
    evaluated_at: Instant,
    popup: Option<Popup>,
    /// Cursor to apply to the editor after it is next shown.
    pending_cursor: Option<usize>,
    /// Sheets whose saved cursor has been put back this session.
    restored: HashSet<String>,
    focus_editor: bool,
    title: String,
    help: Option<help::Tab>,
    updates: Updates,
    update_open: bool,
    /// The app icon, small (sidebar, help rail) and large (About).
    logo: help::Logo,
    /// Developer option for the website: `OMACUS_SCREENSHOT=out.png` saves the window's
    /// own pixels (nothing behind it) and quits.
    screenshot: Option<(std::path::PathBuf, u32)>,
}

impl OmacusApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_fonts(&cc.egui_ctx);

        let ctx = cc.egui_ctx.clone();
        let rates = Rates::load(move || ctx.request_repaint());
        let ctx = cc.egui_ctx.clone();
        let theme_changed = theme::watch(move || ctx.request_repaint());
        let ctx = cc.egui_ctx.clone();
        let updates = Updates::check(move || ctx.request_repaint());

        let system_dark = is_system_dark(&cc.egui_ctx);
        let theme = Theme::load(system_dark);
        apply_visuals(&cc.egui_ctx, &theme);

        let mut app = Self {
            store: Store::load(WELCOME),
            results: Vec::new(),
            results_text: String::new(),
            vars: HashSet::new(),
            engine: Engine::new(rates.table()).with_clock(engine::monotonic_ms),
            calc: {
                let repaint = cc.egui_ctx.clone();
                Calculator::new(
                    Engine::new(rates.table()).with_clock(engine::monotonic_ms),
                    move || repaint.request_repaint(),
                )
            },
            rates_seen: rates.version(),
            rates,
            theme,
            theme_changed,
            system_dark,
            dirty_since: None,
            copied: None,
            toast: None,
            save_error_at: None,
            synced_at: Instant::now(),
            focused: true,
            ctx: cc.egui_ctx.clone(),
            evaluated_at: Instant::now(),
            popup: None,
            pending_cursor: None,
            restored: HashSet::new(),
            focus_editor: true,
            title: String::new(),
            updates,
            logo: help::Logo {
                small: load_logo(
                    &cc.egui_ctx,
                    "small",
                    include_bytes!("../assets/icon-64.png"),
                ),
                large: load_logo(
                    &cc.egui_ctx,
                    "large",
                    include_bytes!("../assets/icon-256.png"),
                ),
            },
            // With OMACUS_SCREENSHOT, "update" opens the update window instead of help.
            update_open: std::env::var("OMACUS_SCREENSHOT_HELP").is_ok_and(|t| t == "update"),
            switcher: std::env::var("OMACUS_SCREENSHOT_HELP")
                .is_ok_and(|t| t == "switcher")
                .then(|| Switcher {
                    query: "k".into(),
                    selected: 0,
                }),
            help: std::env::var("OMACUS_SCREENSHOT_HELP")
                .ok()
                .filter(|t| t != "update" && t != "switcher")
                .map(|t| help::Tab::from_name(&t)),
            screenshot: std::env::var_os("OMACUS_SCREENSHOT").map(|p| (p.into(), 0)),
        };
        app.evaluate();
        app
    }

    /// Ask for fresh answers; they arrive in `take_answers`.
    fn evaluate(&mut self) {
        let sheet = self.store.current();
        self.calc.request(&sheet.id, &sheet.text);
        self.vars = engine::defined_variables(&sheet.text);
        self.evaluated_at = Instant::now();
    }

    /// Sheet `i`'s answers: the ones on screen when they're up to date, so copying
    /// never waits on a slow line; otherwise worked out now.
    fn answers_for(&self, i: usize) -> Vec<LineResult> {
        let text = &self.store.sheets[i].text;
        if i == self.store.active && self.results_text == *text {
            return self.results.clone();
        }
        self.engine.eval_document(text)
    }

    /// Answers from the calculator, for the sheet on screen. While the newest are
    /// on their way, the last ones stay (a line typed a moment ago has none yet).
    fn take_answers(&mut self) {
        if let Some(done) = self.calc.take()
            && done.sheet == self.store.current().id
        {
            self.results = done.results;
            self.results_text = done.text;
        }
    }

    fn touched(&mut self) {
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    fn sheet_switched(&mut self) {
        self.popup = None;
        self.copied = None;
        // Another sheet's answers mustn't show against this one's lines.
        self.results.clear();
        self.results_text.clear();
        self.focus_editor = true;
        self.evaluate();
        self.touched();
    }

    fn apply(&mut self, action: SheetAction) {
        match action {
            SheetAction::Select(i) => self.store.select(i),
            SheetAction::New => self.store.new_sheet(""),
            SheetAction::Delete(i) => {
                let title = self.store.sheets[i].title();
                self.store.delete(i);
                self.toast = Some(Toast::undo(format!("Deleted \u{201c}{title}\u{201d}")));
            }
            SheetAction::Move(i, step) => {
                self.store.select(i);
                self.store.move_active(step);
            }
            SheetAction::Copy(i) => {
                let results = self.answers_for(i);
                let text = &self.store.sheets[i].text;
                let ctx = self.ctx.clone();
                ctx.copy_text(engine::with_answers(text, &results));
                self.toast = Some(Toast::info("Sheet copied with answers"));
                return;
            }
            SheetAction::Share(i) => {
                let link = omacus_engine::share::link(&self.store.sheets[i].text);
                let note = match link.len() {
                    n if n > 8000 => " (long: some apps may cut it off)",
                    n if n > 2000 => " (fine for chat; some email and SMS may cut it)",
                    _ => "",
                };
                self.toast = Some(Toast::info(format!(
                    "Share link copied, {} characters{note}",
                    link.len()
                )));
                self.ctx.copy_text(link);
                return;
            }
            SheetAction::CopyMarkdown(i) => {
                let results = self.answers_for(i);
                let text = &self.store.sheets[i].text;
                let ctx = self.ctx.clone();
                ctx.copy_text(engine::markdown_table(text, &results));
                self.toast = Some(Toast::info("Sheet copied as a Markdown table"));
                return;
            }
        }
        self.sheet_switched();
    }

    fn handle_background_updates(&mut self, ctx: &egui::Context) {
        let system_dark = is_system_dark(ctx);
        if self.theme_changed.swap(false, Ordering::Relaxed) || system_dark != self.system_dark {
            self.system_dark = system_dark;
            self.theme = Theme::load(system_dark);
            apply_visuals(ctx, &self.theme);
        }
        // Both are no-ops unless their schedule is due (rates every 12 hours, updates every 4).
        let repaint = ctx.clone();
        self.rates
            .refresh_if_stale(move || repaint.request_repaint());
        let repaint = ctx.clone();
        self.updates.check_if_due(move || repaint.request_repaint());

        // Share links from the command line, macOS, or a paste.
        for text in crate::links::take() {
            let new = self.store.open(&text);
            self.toast = Some(Toast::info(if new {
                "Opened the shared sheet"
            } else {
                "You already have this sheet; switched to it"
            }));
            self.sheet_switched();
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }

        let v = self.rates.version();
        // Re-evaluate when fresh rates land, and every minute so `now` stays current.
        if v != self.rates_seen || self.evaluated_at.elapsed() > Duration::from_secs(60) {
            self.rates_seen = v;
            self.evaluate();
        }
        self.sync_with_other_windows(ctx);
        if self.store.has_unsaved() {
            let since = *self.dirty_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= SAVE_DELAY {
                self.save();
                self.dirty_since = None;
            } else {
                ctx.request_repaint_after(SAVE_DELAY);
            }
        }
        let title = format!("Omacus — {}", self.store.sheets[self.store.active].title());
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        ctx.request_repaint_after(Duration::from_secs(60));
    }

    /// Keys are consumed here, before the editor sees them.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let toggle_help = ctx.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::F1) || i.consume_key(Modifiers::COMMAND, Key::Slash)
        });
        if toggle_help {
            self.toggle_help();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::P)) {
            self.switcher = match self.switcher {
                Some(_) => None,
                None => Some(Switcher::default()),
            };
            self.help = None;
            self.popup = None;
            self.focus_editor = self.switcher.is_none();
        }
        if self.switcher.is_some() {
            self.switcher_keys(ctx);
            return;
        }
        if self.update_open {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                && self.updates.state() != State::Installing
            {
                self.update_open = false;
                self.focus_editor = true;
            }
            return;
        }
        if self.help.is_some() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.help = None;
                self.focus_editor = true;
            }
            return;
        }

        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;

        if let Some(popup) = &mut self.popup {
            let (accept, up, down, close) = ctx.input_mut(|i| {
                (
                    // Enter stays a newline: `3 km in⏎` must not become `inch`.
                    i.consume_key(Modifiers::NONE, Key::Tab),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            let n = popup.items.len();
            if up {
                popup.selected = (popup.selected + n - 1) % n;
            }
            if down {
                popup.selected = (popup.selected + 1) % n;
            }
            if close {
                self.popup = None;
            } else if accept {
                let pick = popup.selected;
                self.accept_completion(pick);
            }
        }

        let mut action = None;
        let (quit, copy_last, sidebar) = ctx.input_mut(|i| {
            if i.consume_key(cmd_shift, Key::S) {
                action = Some(SheetAction::Copy(self.store.active));
            }
            if i.consume_key(cmd_shift, Key::M) {
                action = Some(SheetAction::CopyMarkdown(self.store.active));
            }
            if i.consume_key(cmd_shift, Key::L) {
                action = Some(SheetAction::Share(self.store.active));
            }
            // A pasted share link opens its sheet instead of landing as text.
            i.events.retain(|e| match e {
                egui::Event::Paste(text) if omacus_engine::share::is_link(text) => {
                    crate::links::push(omacus_engine::share::decode(text).unwrap_or_default());
                    false
                }
                _ => true,
            });
            if i.consume_key(cmd, Key::N) {
                action = Some(SheetAction::New);
            }
            if i.consume_key(cmd_shift, Key::Tab) || i.consume_key(cmd, Key::PageUp) {
                action = Some(self.cycle_action(-1));
            }
            if i.consume_key(cmd, Key::Tab) || i.consume_key(cmd, Key::PageDown) {
                action = Some(self.cycle_action(1));
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowUp) {
                action = Some(SheetAction::Move(self.store.active, -1));
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowDown) {
                action = Some(SheetAction::Move(self.store.active, 1));
            }
            // Ctrl+Delete is "delete the next word" in the editor; deleting a whole
            // sheet takes Shift too.
            if i.consume_key(cmd_shift, Key::Delete) {
                action = Some(SheetAction::Delete(self.store.active));
            }
            let digits = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ];
            for (n, key) in digits.into_iter().enumerate() {
                if i.consume_key(cmd, key) {
                    action = Some(SheetAction::Select(n));
                }
            }
            (
                i.consume_key(Modifiers::NONE, Key::Escape)
                    || i.consume_key(cmd, Key::Q)
                    || i.consume_key(cmd, Key::W),
                i.consume_key(cmd_shift, Key::C),
                i.consume_key(cmd, Key::B),
            )
        });

        if let Some(a) = action {
            self.apply(a);
        }
        if sidebar {
            self.store.toggle_sidebar();
            self.touched();
        }
        if copy_last
            && let Some((idx, r)) = self
                .results
                .iter()
                .enumerate()
                .rev()
                .find(|(_, r)| !r.display.is_empty())
        {
            ctx.copy_text(r.display.clone());
            self.copied = Some((idx, Instant::now()));
        }
        if quit {
            self.save();
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn cycle_action(&self, step: isize) -> SheetAction {
        let n = self.store.sheets.len() as isize;
        SheetAction::Select((self.store.active as isize + step).rem_euclid(n) as usize)
    }

    fn accept_completion(&mut self, pick: usize) {
        let Some(popup) = self.popup.take() else {
            return;
        };
        let Some(word) = popup.items.get(pick) else {
            return;
        };
        let sheet = self.store.current();
        let chars: Vec<char> = sheet.text.chars().collect();
        let mut text: String = chars[..popup.start].iter().collect();
        text.push_str(word);
        text.extend(&chars[popup.end.min(chars.len())..]);
        sheet.text = text;
        sheet.mark_dirty();
        self.pending_cursor = Some(popup.start + word.chars().count());
        self.evaluate();
        self.touched();
    }

    fn take_screenshot(&mut self, ctx: &egui::Context) {
        let Some((path, frames)) = &mut self.screenshot else {
            return;
        };
        *frames += 1;
        // Give fonts, layout and the window size a few frames to settle.
        if *frames == 20 {
            let path = path.clone();
            let ctx2 = ctx.clone();
            ctx.request_screenshot(move |image| {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                let [w, h] = image.size;
                if let Err(e) =
                    image::save_buffer(&path, &bytes, w as u32, h as u32, image::ColorType::Rgba8)
                {
                    eprintln!("omacus: screenshot failed: {e}");
                }
                ctx2.send_viewport_cmd(ViewportCommand::Close);
                ctx2.request_repaint();
            });
        }
        ctx.request_repaint();
    }

    fn toggle_help(&mut self) {
        self.help = match self.help {
            Some(_) => None,
            None => Some(help::Tab::default()),
        };
        self.popup = None;
        self.focus_editor = self.help.is_none();
    }

    fn show_help(&mut self, ctx: &egui::Context) {
        let Some(mut tab) = self.help else { return };
        let action = help::show(
            ctx,
            &mut tab,
            &self.theme,
            &self.engine,
            &self.logo,
            &self.updates,
        );
        self.help = Some(tab);
        match action {
            Some(help::Action::Close) => {
                self.help = None;
                self.focus_editor = true;
            }
            Some(help::Action::OpenExamples) => {
                self.help = None;
                self.store.new_sheet(&help::examples_sheet());
                self.sheet_switched();
            }
            Some(help::Action::OpenUpdate) => {
                self.help = None;
                self.update_open = true;
            }
            None => {}
        }
    }

    /// Arrows, Enter and Esc for the switcher, before its text box sees them.
    fn switcher_keys(&mut self, ctx: &egui::Context) {
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        let matches = self.switcher_matches();
        let Some(sw) = &mut self.switcher else { return };
        let n = matches.len().max(1);
        if up {
            sw.selected = (sw.selected + n - 1) % n;
        }
        if down {
            sw.selected = (sw.selected + 1) % n;
        }
        if esc {
            self.switcher = None;
            self.focus_editor = true;
        } else if enter && let Some(&i) = matches.get(sw.selected) {
            self.switcher = None;
            self.focus_editor = true;
            self.apply(SheetAction::Select(i));
        }
    }

    fn switcher_matches(&self) -> Vec<usize> {
        let Some(sw) = &self.switcher else {
            return Vec::new();
        };
        let titles: Vec<String> = self.store.sheets.iter().map(|s| s.title()).collect();
        let texts: Vec<&str> = self.store.sheets.iter().map(|s| s.text.as_str()).collect();
        find_sheets(&titles, &texts, &sw.query)
    }

    /// The Ctrl+P box: a search field and the matching sheets.
    fn show_switcher(&mut self, ctx: &egui::Context) {
        if self.switcher.is_none() {
            return;
        }
        let theme = self.theme.clone();
        let matches = self.switcher_matches();
        let titles: Vec<String> = self.store.sheets.iter().map(|s| s.title()).collect();
        let mut picked = None;
        let mut close = false;

        let frame = Frame::NONE
            .fill(theme.sidebar)
            .stroke(egui::Stroke::new(1.0, theme.selection))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(8));
        let width = (ctx.content_rect().width() - 80.0).clamp(280.0, 460.0);
        let modal = egui::Modal::new(egui::Id::unique("omacus-switcher"))
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(110))
            .area(
                egui::Modal::default_area(egui::Id::unique("omacus-switcher-area"))
                    .anchor(Align2::CENTER_TOP, vec2(0.0, 80.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                let sw = self.switcher.as_mut().unwrap();
                let before = sw.query.clone();
                let field = ui.add(
                    TextEdit::singleline(&mut sw.query)
                        .hint_text("Go to sheet…")
                        .font(FontId::proportional(15.0))
                        .frame(egui::Frame::NONE.inner_margin(Margin::symmetric(8, 6)))
                        .desired_width(f32::INFINITY),
                );
                field.request_focus();
                if sw.query != before {
                    sw.selected = 0;
                }
                let selected = sw.selected;
                ui.add_space(4.0);
                if matches.is_empty() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("  No sheet matches").color(theme.comment));
                    ui.add_space(4.0);
                }
                for (row, &i) in matches.iter().take(9).enumerate() {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                    if row == selected {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(6), theme.selection);
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(6), theme.hover);
                    }
                    let color = if row == selected {
                        theme.foreground
                    } else {
                        theme.dim
                    };
                    ui.painter().text(
                        rect.left_center() + vec2(10.0, 0.0),
                        Align2::LEFT_CENTER,
                        &titles[i],
                        FontId::proportional(14.0),
                        color,
                    );
                    if i == self.store.active {
                        ui.painter().text(
                            rect.right_center() - vec2(10.0, 0.0),
                            Align2::RIGHT_CENTER,
                            "current",
                            FontId::proportional(12.0),
                            theme.comment,
                        );
                    }
                    if resp.clicked() {
                        picked = Some(i);
                    }
                }
            });
        if modal.should_close() {
            close = true;
        }
        if let Some(i) = picked {
            self.switcher = None;
            self.focus_editor = true;
            self.apply(SheetAction::Select(i));
        } else if close {
            self.switcher = None;
            self.focus_editor = true;
        }
    }

    /// Save, and say so when it fails (once every 30 seconds at most).
    fn save(&mut self) {
        if let Err(e) = self.store.save()
            && self
                .save_error_at
                .is_none_or(|t| t.elapsed() > Duration::from_secs(30))
        {
            self.save_error_at = Some(Instant::now());
            self.toast = Some(Toast::error(format!("Couldn't save {e}")));
        }
        if let Some(title) = self.store.take_conflicts().first() {
            self.toast = Some(Toast::info(format!(
                "\u{201c}{title}\u{201d} changed in another window too; its version is a new sheet"
            )));
        }
    }

    /// Pick up sheets another Omacus window changed: when this window regains focus,
    /// and every few seconds.
    fn sync_with_other_windows(&mut self, ctx: &egui::Context) {
        const EVERY: Duration = Duration::from_secs(3);
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let regained = focused && !self.focused;
        self.focused = focused;
        if !regained && self.synced_at.elapsed() < EVERY {
            ctx.request_repaint_after(EVERY);
            return;
        }
        self.synced_at = Instant::now();
        let synced = self.store.sync();
        if synced.active_changed {
            self.popup = None;
            self.pending_cursor = Some(self.store.current().cursor);
            self.evaluate();
        }
        ctx.request_repaint_after(EVERY);
    }

    /// A short message at the bottom of the window, fading out; Undo when a sheet
    /// was just deleted.
    fn show_toast(&mut self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else { return };
        let show_for = toast.lasts();
        let age = toast.at.elapsed();
        if age > show_for {
            self.toast = None;
            return;
        }
        let fade = 1.0 - (age.as_secs_f32() / show_for.as_secs_f32()).powi(6);
        let theme = self.theme.clone();
        let (text, undo, error) = (toast.text.clone(), toast.undo, toast.error);
        let mut undone = false;
        egui::Area::new(egui::Id::unique("omacus-toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
            .order(Order::Foreground)
            .interactable(undo)
            .show(ctx, |ui| {
                Frame::NONE
                    .fill(theme.selection.gamma_multiply(fade))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let color = if error {
                                theme.variable
                            } else {
                                theme.foreground
                            };
                            ui.label(
                                egui::RichText::new(text.as_str())
                                    .size(13.0)
                                    .color(color.gamma_multiply(fade)),
                            );
                            if undo {
                                let button = egui::Button::new(
                                    egui::RichText::new("Undo")
                                        .size(13.0)
                                        .color(theme.accent.gamma_multiply(fade)),
                                )
                                .frame(false);
                                undone = ui.add(button).clicked();
                            }
                        });
                    });
            });
        if undone {
            self.toast = None;
            if self.store.undo_delete().is_some() {
                self.sheet_switched();
                self.touched();
            }
        }
        ctx.request_repaint();
    }

    /// The update window: what's new, and installing it.
    fn show_update(&mut self, ctx: &egui::Context) {
        if !self.update_open {
            return;
        }
        let Some(latest) = self.updates.available() else {
            self.update_open = false;
            return;
        };
        let theme = self.theme.clone();
        let state = self.updates.state();
        let method = update::method();
        let busy = state == State::Installing;
        let (mut install, mut restart, mut close) = (false, false, false);

        let body = match (&state, method) {
            (State::Installing, _) => "Updating… this takes a few seconds.".to_string(),
            (State::Installed, _) => {
                format!("Updated. Restart Omacus to use {latest}; your sheets are saved.")
            }
            (State::Failed(e), _) => format!("The update didn't finish: {e}"),
            (_, Method::Installer | Method::Replace) => format!("You have {}.", update::current()),
            (_, Method::Download) => format!(
                "You have {}. Download the new version from the releases page.",
                update::current()
            ),
        };

        let frame = Frame::NONE
            .fill(theme.sidebar)
            .stroke(egui::Stroke::new(1.0, theme.selection))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(24, 20));
        let modal = egui::Modal::new(egui::Id::unique("omacus-update"))
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Omacus").size(20.0).color(theme.accent));
                    ui.label(
                        egui::RichText::new(format!("{latest} is available"))
                            .size(15.0)
                            .color(theme.foreground),
                    );
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new(body).size(13.5).color(theme.dim));
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    let primary = |ui: &mut egui::Ui, label: &str| {
                        ui.add(
                            egui::Button::new(
                                egui::RichText::new(label)
                                    .size(13.5)
                                    .color(theme.background),
                            )
                            .fill(theme.accent)
                            .corner_radius(CornerRadius::same(6))
                            .min_size(vec2(0.0, 30.0)),
                        )
                    };
                    match (&state, method) {
                        (State::Installing, _) => {
                            ui.spinner();
                        }
                        (State::Installed, _) => restart = primary(ui, "Restart").clicked(),
                        (State::Failed(_), _) | (_, Method::Download) => {
                            if primary(ui, "Download").clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                                    "{}/latest",
                                    update::RELEASES_URL
                                )));
                            }
                        }
                        (_, Method::Installer | Method::Replace) => {
                            install = primary(ui, "Update now").clicked()
                        }
                    }
                    if !busy {
                        let quiet = |label: &str| {
                            egui::Button::new(
                                egui::RichText::new(label)
                                    .size(13.5)
                                    .color(theme.foreground),
                            )
                            .corner_radius(CornerRadius::same(6))
                            .min_size(vec2(0.0, 30.0))
                        };
                        if ui.add(quiet("What's new")).clicked() {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(update::notes_url(&latest)));
                        }
                        let later = if state == State::Installed {
                            "Later"
                        } else {
                            "Not now"
                        };
                        close = ui.add(quiet(later)).clicked();
                    }
                });
            });
        if modal.should_close() && !busy {
            close = true;
        }

        if install {
            let ctx = ctx.clone();
            self.updates.install(move || ctx.request_repaint());
        }
        if restart {
            self.save();
            match update::relaunch() {
                Ok(()) => ctx.send_viewport_cmd(ViewportCommand::Close),
                Err(e) => eprintln!("omacus: couldn't restart: {e}"),
            }
        }
        if close {
            self.update_open = false;
            self.focus_editor = true;
        }
    }

    /// One quiet row: the logo, name and version (opens About, or the update when
    /// one is out, marked with a dot), and "?" for help.
    /// Share and ⋯ above the sheet, lined up with the answers: the actions for this sheet,
    /// without a right-click. Quiet until the pointer comes near.
    fn sheet_bar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let i = self.store.active;
        let count = self.store.sheets.len();
        let mut action = None;
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        let near = ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|p| row.expand2(vec2(0.0, 24.0)).contains(p));

        let dots_rect = Rect::from_min_size(pos2(row.right() - 28.0, row.top()), vec2(28.0, 28.0));
        let dots = ui.interact(dots_rect, ui.scope_id().with("bar-dots"), Sense::click());
        let font = FontId::proportional(13.0);
        let label = ui.painter().layout_no_wrap("Share".into(), font, theme.dim);
        let share_rect = Rect::from_min_max(
            pos2(dots_rect.left() - 4.0 - label.size().x - 24.0, row.top()),
            pos2(dots_rect.left() - 4.0, row.bottom()),
        );
        let share = ui.interact(share_rect, ui.scope_id().with("bar-share"), Sense::click());

        let painter = ui.painter();
        if share.hovered() {
            painter.rect_filled(share_rect, CornerRadius::same(6), theme.hover);
        }
        let color = if share.hovered() {
            theme.foreground
        } else if near {
            theme.dim
        } else {
            theme.comment
        };
        // A small arrow out of a box, drawn so it doesn't depend on the font.
        let icon = pos2(share_rect.left() + 13.0, row.center().y);
        let stroke = egui::Stroke::new(1.4, color);
        painter.line_segment([icon + vec2(0.0, 3.0), icon + vec2(0.0, -5.0)], stroke);
        painter.line_segment([icon + vec2(-3.0, -2.0), icon + vec2(0.0, -5.0)], stroke);
        painter.line_segment([icon + vec2(3.0, -2.0), icon + vec2(0.0, -5.0)], stroke);
        painter.line_segment([icon + vec2(-5.0, 0.0), icon + vec2(-5.0, 5.0)], stroke);
        painter.line_segment([icon + vec2(-5.0, 5.0), icon + vec2(5.0, 5.0)], stroke);
        painter.line_segment([icon + vec2(5.0, 5.0), icon + vec2(5.0, 0.0)], stroke);
        painter.text(
            pos2(share_rect.left() + 22.0, row.center().y),
            Align2::LEFT_CENTER,
            "Share",
            FontId::proportional(13.0),
            color,
        );

        let menu = egui::Popup::menu(&dots).align(egui::RectAlign::BOTTOM_END);
        let open = menu.is_open();
        if near || open {
            dots_button(ui, &dots, &theme, open);
        } else {
            paint_dots(ui.painter(), dots_rect.center(), theme.comment);
        }
        menu.show(|ui| sheet_menu(ui, i, count, &mut action));

        if share.hovered() || dots.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if share
            .on_hover_text(format!(
                "Copy a link to this sheet ({})",
                help::keys("Ctrl+Shift+L")
            ))
            .clicked()
        {
            action = Some(SheetAction::Share(i));
        }
        dots.on_hover_text("Copy, share and more");
        ui.add_space(2.0);

        if let Some(a) = action {
            self.apply(a);
            self.focus_editor = true;
        }
    }

    fn sidebar_footer(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        let available = self.updates.available();
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        let help_rect = Rect::from_min_size(pos2(row.right() - 28.0, row.top()), vec2(28.0, 28.0));
        let about_rect = Rect::from_min_max(row.min, pos2(help_rect.left() - 4.0, row.bottom()));
        let about = ui.interact(about_rect, ui.scope_id().with("about"), Sense::click());
        let help = ui.interact(help_rect, ui.scope_id().with("help"), Sense::click());

        let painter = ui.painter();
        for (rect, resp) in [(about_rect, &about), (help_rect, &help)] {
            if resp.hovered() {
                painter.rect_filled(rect, CornerRadius::same(6), theme.hover);
            }
        }
        let logo =
            Rect::from_center_size(pos2(row.left() + 14.0, row.center().y), vec2(18.0, 18.0));
        painter.image(
            self.logo.small.id(),
            logo,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let name = painter.text(
            pos2(logo.right() + 8.0, row.center().y),
            Align2::LEFT_CENTER,
            "omacus",
            FontId::proportional(13.0),
            theme.dim,
        );
        let version = painter.text(
            pos2(name.right() + 6.0, row.center().y),
            Align2::LEFT_CENTER,
            format!("v{}", update::current()),
            FontId::proportional(12.0),
            if available.is_some() {
                theme.accent
            } else {
                theme.comment
            },
        );
        if available.is_some() {
            painter.circle_filled(version.right_center() + vec2(7.0, 0.0), 3.0, theme.accent);
        }
        painter.text(
            help_rect.center(),
            Align2::CENTER_CENTER,
            "?",
            FontId::proportional(14.0),
            if help.hovered() {
                theme.foreground
            } else {
                theme.dim
            },
        );

        if about.hovered() || help.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let about_tip = match &available {
            Some(v) => format!("Version {v} is available"),
            None => "About Omacus".to_string(),
        };
        if about.on_hover_text(about_tip).clicked() {
            if available.is_some() {
                self.update_open = true;
                self.help = None;
            } else {
                self.help = Some(help::Tab::About);
                self.popup = None;
            }
        }
        if help
            .on_hover_text(format!("Help & shortcuts ({})", help::keys("F1")))
            .clicked()
        {
            self.toggle_help();
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let mut action = None;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("SHEETS")
                    .size(11.0)
                    .color(theme.comment),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let plus = ui.add(
                    egui::Button::new(egui::RichText::new("+").size(16.0).color(theme.dim))
                        .frame(false),
                );
                if plus
                    .on_hover_text(format!("New sheet ({})", help::keys("Ctrl+N")))
                    .clicked()
                {
                    action = Some(SheetAction::New);
                }
            });
        });
        ui.add_space(6.0);

        let titles: Vec<String> = self.store.sheets.iter().map(|s| s.title()).collect();
        let count = titles.len();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - 38.0)
            .show(ui, |ui| {
                for (i, title) in titles.into_iter().enumerate() {
                    let active = i == self.store.active;
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                    let painter = ui.painter();
                    if active {
                        painter.rect_filled(rect, CornerRadius::same(5), theme.selection);
                        let bar = Rect::from_min_size(rect.min + vec2(0.0, 7.0), vec2(3.0, 16.0));
                        painter.rect_filled(bar, CornerRadius::same(2), theme.accent);
                    } else if resp.hovered() {
                        painter.rect_filled(rect, CornerRadius::same(5), theme.hover);
                    }
                    let color = if active { theme.foreground } else { theme.dim };
                    let font = FontId::proportional(14.0);
                    let galley = painter.layout(title, font, color, rect.width() - 22.0);
                    painter.galley(
                        pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
                        galley,
                        color,
                    );

                    // A ⋯ on the selected or hovered row opens the same menu as a right-click.
                    let dots_rect = Rect::from_center_size(
                        pos2(rect.right() - 15.0, rect.center().y),
                        vec2(24.0, 24.0),
                    );
                    let dots = ui.interact(
                        dots_rect,
                        ui.scope_id().with(("sheet-dots", i)),
                        Sense::click(),
                    );
                    let menu = egui::Popup::menu(&dots);
                    let open = menu.is_open();
                    if active || resp.hovered() || dots.hovered() || open {
                        dots_button(ui, &dots, &theme, open);
                    }
                    menu.show(|ui| sheet_menu(ui, i, count, &mut action));
                    dots.on_hover_text("Share, copy and more");

                    if resp.clicked() {
                        action = Some(SheetAction::Select(i));
                    }
                    resp.context_menu(|ui| sheet_menu(ui, i, count, &mut action));
                }
            });

        ui.add_space(6.0);
        self.sidebar_footer(ui, &theme);

        if let Some(a) = action {
            self.apply(a);
        }
    }

    fn sheet(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let font = FontId::monospace(FONT_SIZE);
        // Each line's answer width, swatch included, for placing it after the text.
        let (row_height, answers) = ui.ctx().fonts_mut(|f| {
            let answers: Vec<Option<f32>> = self
                .results
                .iter()
                .map(|r| {
                    let text = match (&r.kind, r.note) {
                        (Kind::Value, _) if !r.display.is_empty() => r.display.clone(),
                        (Kind::Error, Some(note)) => note.to_string(),
                        _ => return None,
                    };
                    let swatch = if r.color.is_some() {
                        swatch_room(&font)
                    } else {
                        0.0
                    };
                    Some(
                        f.layout_no_wrap(text, font.clone(), Color32::WHITE)
                            .size()
                            .x
                            + swatch,
                    )
                })
                .collect();
            (f.row_height(&font), answers)
        });

        // Lines use the full width. An answer sits at the end of its line's last row,
        // or, when that row leaves no room, on a row of its own just below.
        let editor_width = (ui.available_width() - SCROLLBAR_ROOM).max(120.0);
        let min_rows = (ui.available_height() / row_height).floor().max(1.0) as usize;

        let sheet_id = self.store.current().id.clone();
        let vars = &self.vars;
        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = highlight::layout(buf.as_str(), vars, &theme, font.clone());
            job.wrap.max_width = wrap_width;
            let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
            make_room_for_answers(galley, &answers, wrap_width, row_height)
        };

        // One editor per sheet, so each keeps its own cursor and undo history.
        let output = TextEdit::multiline(&mut self.store.sheets[self.store.active].text)
            .id_salt(("sheet", &sheet_id))
            .font(font.clone())
            .frame(Frame::NONE)
            .desired_width(editor_width)
            .desired_rows(min_rows)
            .lock_focus(true)
            .layouter(&mut layouter)
            .show(ui);
        let response = &output.response.response;

        if self.restored.insert(sheet_id) {
            self.pending_cursor
                .get_or_insert(self.store.current().cursor);
        }
        if let Some(pos) = self.pending_cursor.take() {
            let mut state = output.state.clone();
            state
                .cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(pos))));
            state.store(ui.ctx(), response.id);
            response.scroll_to_me(None);
            ui.ctx().request_repaint();
        }
        if std::mem::take(&mut self.focus_editor) {
            response.request_focus();
        }

        let changed = response.changed();
        if changed {
            self.store.current().mark_dirty();
            self.evaluate();
            self.touched();
        }

        let cursor = output
            .cursor_range
            .filter(|r| r.primary == r.secondary)
            .map(|r| r.primary.index.0);
        if let Some(c) = output.cursor_range.map(|r| r.primary.index.0) {
            self.store.set_cursor(c);
            if self.store.has_unsaved() {
                self.touched();
            }
        }

        // Suggest while typing; any other cursor movement closes the popup.
        match cursor {
            Some(c) if changed && response.has_focus() => {
                let text = &self.store.sheets[self.store.active].text;
                self.popup = complete::word_before(text, c).and_then(|(start, word)| {
                    let items = complete::suggest(&word, &self.vars);
                    let at = output.galley.pos_from_cursor(CCursor::new(c));
                    (!items.is_empty()).then(|| Popup {
                        start,
                        end: c,
                        items,
                        selected: 0,
                        anchor: output.galley_pos + at.left_bottom().to_vec2() + vec2(0.0, 2.0),
                    })
                });
            }
            Some(c) if self.popup.as_ref().is_some_and(|p| p.end == c) => {}
            _ => self.popup = None,
        }

        // Paint each answer at the end of its line, or on the row made for it below.
        let right = ui.max_rect().right() - SCROLLBAR_ROOM;
        let origin = output.galley_pos;
        let mut line = 0;
        let rows = &output.galley.rows;
        for (i, row) in rows.iter().enumerate() {
            let last_of_line = row.ends_with_newline || i + 1 == rows.len();
            if !last_of_line {
                continue;
            }
            if let Some(Some(width)) = answers.get(line) {
                let mut y = origin.y + row.pos.y;
                if answer_drops(row.rect().max.x, *width, editor_width) {
                    y += row_height;
                }
                self.paint_result(ui, line, right, y, row_height, &font);
            }
            line += 1;
        }
    }

    fn paint_result(
        &mut self,
        ui: &egui::Ui,
        line: usize,
        right: f32,
        y: f32,
        h: f32,
        font: &FontId,
    ) {
        let Some(result) = self.results.get(line) else {
            return;
        };
        let theme = &self.theme;
        // No answer, but a reason ("Can't divide by zero"): shown quietly, not copyable.
        if let (Kind::Error, Some(note)) = (&result.kind, result.note) {
            let galley = ui
                .ctx()
                .fonts_mut(|f| f.layout_no_wrap(note.to_string(), font.clone(), theme.comment));
            let size = galley.size();
            ui.painter().galley(
                pos2(right - size.x, y + (h - size.y) / 2.0),
                galley,
                theme.comment,
            );
            return;
        }
        if result.kind != Kind::Value || result.display.is_empty() {
            return;
        }
        let copied = self
            .copied
            .is_some_and(|(l, t)| l == line && t.elapsed() < COPIED_FOR);
        let text = if copied {
            "copied".to_string()
        } else {
            result.display.clone()
        };

        let galley = ui
            .ctx()
            .fonts_mut(|f| f.layout_no_wrap(text, font.clone(), theme.accent));
        let size = galley.size();
        // A colour answer gets a swatch of it, left of the text.
        let swatch = result.color.filter(|_| !copied);
        let extra = if swatch.is_some() {
            swatch_room(font)
        } else {
            0.0
        };
        let rect = Rect::from_min_size(pos2(right - size.x - extra, y), vec2(size.x + extra, h))
            .expand2(vec2(6.0, 1.0));
        let response = ui.interact(rect, ui.scope_id().with(("result", line)), Sense::click());

        if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), theme.hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            ui.ctx().copy_text(result.display.clone());
            self.copied = Some((line, Instant::now()));
            ui.ctx().request_repaint_after(COPIED_FOR);
        }
        let color = if copied { theme.dim } else { theme.accent };
        if let Some([r, g, b]) = swatch {
            let side = font.size * 0.8;
            let square = Rect::from_min_size(
                pos2(right - size.x - extra, y + (h - side) / 2.0),
                vec2(side, side),
            );
            ui.painter()
                .rect_filled(square, CornerRadius::same(3), Color32::from_rgb(r, g, b));
            ui.painter().rect_stroke(
                square,
                CornerRadius::same(3),
                egui::Stroke::new(1.0, theme.selection),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter()
            .galley(pos2(right - size.x, y + (h - size.y) / 2.0), galley, color);
    }

    fn popup(&mut self, ctx: &egui::Context) {
        let Some(popup) = &self.popup else { return };
        let theme = &self.theme;
        let font = FontId::monospace(FONT_SIZE - 2.0);
        let mut clicked = None;
        egui::Area::new(egui::Id::unique("omacus-completions"))
            .order(Order::Foreground)
            .fixed_pos(popup.anchor)
            .show(ctx, |ui| {
                Frame::NONE
                    .fill(theme.sidebar)
                    .stroke(egui::Stroke::new(1.0, theme.selection))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(4))
                    .show(ui, |ui| {
                        let width = popup
                            .items
                            .iter()
                            .map(|s| s.chars().count())
                            .max()
                            .unwrap_or(4) as f32
                            * (FONT_SIZE - 2.0)
                            * 0.62
                            + 24.0;
                        for (i, item) in popup.items.iter().enumerate() {
                            let (rect, resp) = ui
                                .allocate_exact_size(vec2(width.max(140.0), 24.0), Sense::click());
                            let selected = i == popup.selected;
                            if selected || resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(4),
                                    theme.selection,
                                );
                            }
                            let color = if selected {
                                theme.accent
                            } else {
                                theme.foreground
                            };
                            ui.painter().text(
                                pos2(rect.left() + 8.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                item,
                                font.clone(),
                                color,
                            );
                            if resp.clicked() {
                                clicked = Some(i);
                            }
                        }
                    });
            });
        if let Some(i) = clicked {
            self.accept_completion(i);
            self.focus_editor = true;
        }
    }
}

impl eframe::App for OmacusApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_background_updates(&ctx);
        self.take_answers();
        self.take_screenshot(&ctx);
        self.handle_shortcuts(&ctx);

        if self.store.sidebar {
            let frame = Frame::NONE
                .fill(self.theme.sidebar)
                .inner_margin(Margin::symmetric(10, 14));
            egui::Panel::left("sheets")
                .exact_size(SIDEBAR_WIDTH)
                .resizable(false)
                .frame(frame)
                .show(ui, |ui| self.sidebar(ui));
        }

        let frame = Frame::NONE
            .fill(self.theme.background)
            .inner_margin(Margin {
                left: 24,
                right: 24,
                top: 8,
                bottom: 20,
            });
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            self.sheet_bar(ui);
            egui::ScrollArea::vertical()
                .id_salt(("scroll", &self.store.sheets[self.store.active].id))
                .auto_shrink([false, false])
                .show(ui, |ui| self.sheet(ui));
        });

        self.popup(&ctx);
        self.show_help(&ctx);
        self.show_update(&ctx);
        self.show_switcher(&ctx);
        self.show_toast(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme.background.to_normalized_gamma_f32()
    }
}

/// The actions for sheet `i`: the right-click menu, and the ⋯ on its row and above it.
fn sheet_menu(ui: &mut egui::Ui, i: usize, count: usize, action: &mut Option<SheetAction>) {
    let item = |ui: &mut egui::Ui, text: &str, keys: Option<&str>| {
        let mut button = egui::Button::new(text);
        if let Some(k) = keys {
            button = button.shortcut_text(help::keys(k));
        }
        ui.add(button).clicked()
    };
    if item(ui, "Share link", Some("Ctrl+Shift+L")) {
        *action = Some(SheetAction::Share(i));
    }
    if item(ui, "Copy with answers", Some("Ctrl+Shift+S")) {
        *action = Some(SheetAction::Copy(i));
    }
    if item(ui, "Copy as Markdown table", Some("Ctrl+Shift+M")) {
        *action = Some(SheetAction::CopyMarkdown(i));
    }
    ui.separator();
    if item(ui, "New sheet", Some("Ctrl+N")) {
        *action = Some(SheetAction::New);
    }
    if i > 0 && item(ui, "Move up", None) {
        *action = Some(SheetAction::Move(i, -1));
    }
    if i + 1 < count && item(ui, "Move down", None) {
        *action = Some(SheetAction::Move(i, 1));
    }
    ui.separator();
    if item(ui, "Delete sheet", Some("Ctrl+Shift+Delete")) {
        *action = Some(SheetAction::Delete(i));
    }
}

fn paint_dots(painter: &egui::Painter, center: Pos2, color: Color32) {
    for dx in [-5.0, 0.0, 5.0] {
        painter.circle_filled(center + vec2(dx, 0.0), 1.5, color);
    }
}

/// A ⋯ button: brighter, with a background, when hovered or its menu is open.
fn dots_button(ui: &egui::Ui, resp: &egui::Response, theme: &Theme, open: bool) {
    let painter = ui.painter();
    if resp.hovered() || open {
        painter.rect_filled(resp.rect, CornerRadius::same(6), theme.hover);
    }
    let color = if resp.hovered() || open {
        theme.foreground
    } else {
        theme.dim
    };
    paint_dots(painter, resp.rect.center(), color);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

fn load_logo(ctx: &egui::Context, name: &str, png: &[u8]) -> egui::TextureHandle {
    let image = image::load_from_memory(png)
        .expect("bundled icon is a valid PNG")
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    // Mipmaps keep it crisp at 16px.
    let options = egui::TextureOptions {
        mipmap_mode: Some(egui::TextureFilter::Linear),
        ..egui::TextureOptions::LINEAR
    };
    ctx.load_texture(format!("omacus-logo-{name}"), pixels, options)
}

fn apply_visuals(ctx: &egui::Context, theme: &Theme) {
    let mut v = if theme.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = theme.background;
    v.window_fill = theme.sidebar;
    v.extreme_bg_color = theme.background;
    v.override_text_color = Some(theme.foreground);
    v.selection.bg_fill = theme.selection;
    v.selection.stroke.color = theme.foreground;
    v.text_cursor.stroke.color = theme.accent;
    v.text_cursor.stroke.width = 2.0;
    v.widgets.hovered.weak_bg_fill = theme.hover;
    ctx.set_visuals(v);
}

/// JetBrains Mono ships inside the binary so Omacus looks the same everywhere.
// egui's own fonts (1.4 MB) are left out; these cover everything Omacus shows. They are
// stored deflated (see build.rs) and inflated once at startup.
macro_rules! font {
    ($name:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/", $name, ".deflate"))
    };
}
static BUNDLED_FONT: &[u8] = font!("JetBrainsMono-Regular.ttf");
/// ₹, ₩, ₺ and the other currency signs JetBrains Mono lacks, cut from Noto Sans.
static CURRENCY_FONT: &[u8] = font!("NotoSans-Currency.ttf");
/// Emoji people type into notes ("✈ Flights").
static EMOJI_FONT: &[u8] = font!("NotoEmoji-Regular.ttf");

fn inflate(packed: &[u8]) -> FontData {
    FontData::from_owned(miniz_oxide::inflate::decompress_to_vec(packed).expect("bundled font"))
}
static ICON: &[u8] = include_bytes!("../assets/icon-256.png");

/// On Linux, honour the user's monospace font (JetBrains Mono Nerd Font on Omarchy).
#[cfg(target_os = "linux")]
fn system_font() -> Option<Vec<u8>> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", "monospace"])
        .output()
        .ok()?;
    let path = String::from_utf8(out.stdout).ok()?;
    let path = path.trim();
    // Font collections (.ttc) need an index; skip them.
    if !(path.ends_with(".ttf") || path.ends_with(".otf")) {
        return None;
    }
    std::fs::read(path).ok()
}

#[cfg(not(target_os = "linux"))]
fn system_font() -> Option<Vec<u8>> {
    None
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::empty();
    let mut chain = Vec::new();
    // The user's font first, then the bundled ones for anything it lacks.
    let mut add = |name: &str, data: FontData| {
        fonts.font_data.insert(name.into(), Arc::new(data));
        chain.push(name.to_string());
    };
    if let Some(bytes) = system_font() {
        add("system", FontData::from_owned(bytes));
    }
    add("mono", inflate(BUNDLED_FONT));
    add("currency", inflate(CURRENCY_FONT));
    add("emoji", inflate(EMOJI_FONT));
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        fonts.families.insert(family, chain.clone());
    }
    ctx.set_fonts(fonts);
}

/// Width a colour swatch and its gap take beside a result.
/// Whether an answer `width` wide no longer fits after a row ending at `row_end`.
fn answer_drops(row_end: f32, width: f32, wrap_width: f32) -> bool {
    row_end + GAP + width > wrap_width
}

/// Push rows down to make a row for each answer that doesn't fit beside its line.
fn make_room_for_answers(
    galley: Arc<egui::Galley>,
    answers: &[Option<f32>],
    wrap_width: f32,
    row_height: f32,
) -> Arc<egui::Galley> {
    let mut drops = Vec::new();
    let mut line = 0;
    for (i, row) in galley.rows.iter().enumerate() {
        if row.ends_with_newline || i + 1 == galley.rows.len() {
            if let Some(Some(width)) = answers.get(line)
                && answer_drops(row.rect().max.x, *width, wrap_width)
            {
                drops.push(i);
            }
            line += 1;
        }
    }
    if drops.is_empty() {
        return galley;
    }
    let mut galley = Arc::unwrap_or_clone(galley);
    let mut shift = 0.0;
    let mut next = drops.iter().peekable();
    for (i, row) in galley.rows.iter_mut().enumerate() {
        row.pos.y += shift;
        if next.peek() == Some(&&i) {
            next.next();
            shift += row_height;
        }
    }
    galley.rect.max.y += shift;
    galley.mesh_bounds.max.y += shift;
    Arc::new(galley)
}

fn swatch_room(font: &FontId) -> f32 {
    font.size * 0.8 + 8.0
}

fn is_system_dark(ctx: &egui::Context) -> bool {
    ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark)
}

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("omacus")
            .with_title("Omacus")
            .with_inner_size(Vec2::new(980.0, 600.0))
            .with_min_inner_size(Vec2::new(420.0, 220.0))
            .with_icon(eframe::icon_data::from_png_bytes(ICON).unwrap_or_default()),
        ..Default::default()
    };
    eframe::run_native(
        "Omacus",
        options,
        Box::new(|cc| Ok(Box::new(OmacusApp::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::find_sheets;

    #[test]
    fn finds_sheets_by_name_then_contents() {
        let titles = [
            "Kyoto in autumn",
            "Invoice — October",
            "Kitchen",
            "Monthly budget",
        ]
        .map(String::from);
        let texts = [
            "# Kyoto\nRyokan",
            "# Invoice\nrate",
            "# Kitchen\n2 cups",
            "# Budget\nRent",
        ];
        assert_eq!(find_sheets(&titles, &texts, "ki"), vec![2, 0]);
        assert_eq!(find_sheets(&titles, &texts, "oct"), vec![1]);
        assert_eq!(find_sheets(&titles, &texts, "mb"), vec![3]);
        assert_eq!(find_sheets(&titles, &texts, "ryokan"), vec![0]);
        assert_eq!(find_sheets(&titles, &texts, ""), vec![0, 1, 2, 3]);
        assert!(find_sheets(&titles, &texts, "zzz").is_empty());
    }
}
