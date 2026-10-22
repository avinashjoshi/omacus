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
use crate::pdf;
use crate::preview;
use crate::rates::Rates;
use crate::settings::{self, Settings};
use crate::sheets::{SIDEBAR_MAX, SIDEBAR_MIN, Store, Trashed, heading_of};
use crate::theme::{self, Theme};
use crate::update::{self, Method, State, Updates};
use omacus_engine::{self as engine, Engine, Kind, LineResult, complete};

const FONT_SIZE: f32 = 16.0;
const GAP: f32 = 32.0;
/// Collapsed sidebar: an expand button, + and the logo, not an overlay tab. Its
/// logo sits where the open sidebar's is, 24 points in.
const SIDEBAR_RAIL: f32 = 48.0;
/// The sidebar's top row is as tall as ⋯ above the sheet, and level with it.
const BAR_ROW: f32 = 28.0;
/// Sidebar buttons are this square, centered over the rows' ⋯.
const ICON_BOX: f32 = 24.0;
/// Every sheet row is this tall, so a long name cannot wrap into the next one.
const SIDEBAR_ROW: f32 = 32.0;
const SCROLLBAR_ROOM: f32 = 12.0;
const SAVE_DELAY: Duration = Duration::from_millis(400);
const COPIED_FOR: Duration = Duration::from_millis(900);
/// A name hint waits this long after the last keystroke on its line.
const HINT_IDLE: Duration = Duration::from_millis(600);
const TRASH_REFRESH: Duration = Duration::from_millis(500);

const WELCOME: &str = "\
# Welcome to Omacus
// Type on the left, results appear on the right.
// Click a result to copy it. Press F1 for help and shortcuts.
// Pages are listed in the sidebar. Ctrl+B collapses it, Ctrl+J jumps to one.

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
    items: Vec<complete::Suggestion>,
    selected: usize,
    anchor: Pos2,
    /// The anchor was measured from the editor. A shortcut opens the popup
    /// before that measurement exists.
    anchored: bool,
    /// Opened with Ctrl+Space. Enter accepts only then; otherwise Enter is a newline.
    explicit: bool,
}

/// The value under the pointer, kept so a hover does not recompute every frame.
struct HoverCache {
    text: String,
    cursor: usize,
    hit: Option<engine::Hover>,
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
    Settings,
    /// Rendered markdown, with each answer in brackets. Ctrl+P.
    Preview,
}

/// A message at the bottom of the window.
struct Toast {
    text: String,
    at: Instant,
    /// Offers Undo for a deleted sheet.
    undo: bool,
    error: bool,
    /// Stays until replaced ("Working out the answers to copy…").
    working: bool,
}

impl Toast {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            at: Instant::now(),
            undo: false,
            error: false,
            working: false,
        }
    }
    fn working(text: &str) -> Self {
        Self {
            working: true,
            ..Self::info(text)
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
        Duration::from_millis(if self.working {
            120_000
        } else if self.undo {
            6000
        } else if self.error {
            5000
        } else {
            1600
        })
    }
}

/// Ctrl+J (Cmd+J on Mac): find a sheet by name (or by something written in it).
#[derive(Default)]
struct Switcher {
    query: String,
    selected: usize,
}

/// Where Enter goes in the switcher: an open page, one in the trash, or Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Jump {
    Sheet(usize),
    Trash(String),
    Settings,
}

struct Hit {
    title: String,
    jump: Jump,
    badge: &'static str,
}

/// A shortcut handled before the editor, so a held key can't type or flicker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shortcut {
    ToggleSwitcher,
    PrevSheet,
    NextSheet,
    Quit,
    Dismiss,
    Settings,
    ToggleAnswers,
    Preview,
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

/// A copy waiting on its answers: the text as it was when asked, how to lay it out,
/// what to say when it's done, and whether the window has kept focus since (a copy
/// that lands after you've switched away could replace what you copied there).
struct PendingCopy {
    text: String,
    format: fn(&str, &[LineResult]) -> String,
    done: &'static str,
    focused: bool,
}

pub struct OmacusApp {
    store: Store,
    results: Vec<LineResult>,
    /// The text `results` are lined up with, and whether they were worked out for
    /// exactly that text (otherwise they're older answers moved to their lines while
    /// the new ones are on their way).
    results_text: String,
    results_exact: bool,
    vars: HashSet<String>,
    /// For the help's examples.
    engine: Engine,
    /// The sheet on screen is calculated by `calc`. Copies whose answers aren't on
    /// screen yet are worked out by `copier`, one at a time: a newer copy replaces
    /// the one waiting.
    calc: Calculator,
    copier: Calculator,
    copy: Option<PendingCopy>,
    rates: Rates,
    rates_seen: u64,
    theme: Theme,
    theme_changed: Arc<AtomicBool>,
    system_dark: bool,
    settings: Settings,
    settings_open: bool,
    /// Ctrl+P: the page rendered as markdown. Esc returns to editing.
    page_preview: bool,
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
    /// Sheet index waiting for a delete confirmation. Escape cancels.
    pending_delete: Option<usize>,
    /// Trash id shown read-only. Clicking a deleted sheet opens this; it does not restore.
    preview: Option<String>,
    /// Trash id waiting for a "delete forever" confirmation.
    purge_id: Option<String>,
    /// `trash/` listing, refreshed at most every `TRASH_REFRESH`.
    trash: Vec<Trashed>,
    trash_at: Option<Instant>,
    /// When the sheet text last changed, for holding name hints while typing.
    edited_at: Instant,
    /// Line the caret is on, if the editor reported a cursor.
    cursor_line: Option<usize>,
    ctx: egui::Context,
    evaluated_at: Instant,
    popup: Option<Popup>,
    hover: Option<HoverCache>,
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

        let system_dark = is_system_dark(&cc.egui_ctx);
        let settings = settings::load();
        settings::remember(&settings);
        let theme = Theme::resolve(settings.theme, system_dark);
        apply_visuals(&cc.egui_ctx, &theme);

        let ctx = cc.egui_ctx.clone();
        let updates = Updates::check(move || ctx.request_repaint());

        let mut app = Self {
            store: Store::load(WELCOME),
            results: Vec::new(),
            results_text: String::new(),
            results_exact: false,
            vars: HashSet::new(),
            engine: Engine::new(rates.table()).with_clock(engine::monotonic_ms),
            calc: {
                let repaint = cc.egui_ctx.clone();
                Calculator::new(
                    Engine::new(rates.table()).with_clock(engine::monotonic_ms),
                    move || repaint.request_repaint(),
                )
            },
            copier: {
                let repaint = cc.egui_ctx.clone();
                Calculator::new(
                    Engine::new(rates.table()).with_clock(engine::monotonic_ms),
                    move || repaint.request_repaint(),
                )
            },
            copy: None,
            rates_seen: rates.version(),
            rates,
            theme,
            theme_changed,
            system_dark,
            settings,
            settings_open: std::env::var("OMACUS_SCREENSHOT_HELP").is_ok_and(|t| t == "settings"),
            page_preview: std::env::var("OMACUS_SCREENSHOT_HELP").is_ok_and(|t| t == "preview"),
            dirty_since: None,
            copied: None,
            toast: None,
            save_error_at: None,
            synced_at: Instant::now(),
            focused: true,
            ctx: cc.egui_ctx.clone(),
            evaluated_at: Instant::now(),
            popup: None,
            hover: None,
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
            pending_delete: None,
            preview: None,
            purge_id: None,
            trash: Vec::new(),
            trash_at: None,
            edited_at: Instant::now()
                .checked_sub(HINT_IDLE)
                .unwrap_or_else(Instant::now),
            cursor_line: None,
            help: std::env::var("OMACUS_SCREENSHOT_HELP")
                .ok()
                .filter(|t| t != "update" && t != "switcher" && t != "settings")
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
        if self.results_text != sheet.text {
            self.results = engine::realign(&self.results_text, &self.results, &sheet.text);
            self.results_text = sheet.text.clone();
            self.results_exact = false;
        }
        self.vars = engine::defined_variables(&sheet.text);
        self.evaluated_at = Instant::now();
    }

    /// Copy sheet `i` with its answers, as `format` lays them out. The answers on
    /// screen are used when they're up to date; otherwise they're worked out by
    /// `copier`, so the window never waits on a slow line, and copied when they arrive.
    fn copy_with(
        &mut self,
        i: usize,
        format: fn(&str, &[LineResult]) -> String,
        done: &'static str,
    ) {
        let sheet = &self.store.sheets[i];
        let text = sheet.text.clone();
        if i == self.store.active && self.results_exact && self.results_text == text {
            self.copy = None;
            self.ctx.copy_text(format(&text, &self.results));
            self.toast = Some(Toast::info(done));
            return;
        }
        self.copier.request(&sheet.id, &text);
        self.copy = Some(PendingCopy {
            text,
            format,
            done,
            focused: true,
        });
        self.toast = Some(Toast::working("Working out the answers to copy\u{2026}"));
    }

    /// The waiting copy, once its answers are in.
    fn take_copy(&mut self, ctx: &egui::Context) {
        let Some(done) = self.copier.take().filter(|d| d.complete) else {
            return;
        };
        let Some(copy) = self.copy.take_if(|c| c.text == done.text) else {
            return;
        };
        if copy.focused {
            ctx.copy_text((copy.format)(&copy.text, &done.results));
            self.toast = Some(Toast::info(copy.done));
        } else {
            self.toast = Some(Toast::info(
                "Answers ready, not copied: you switched away while they were worked out",
            ));
        }
    }

    /// Answers from the calculator, for the sheet on screen. While the newest are
    /// on their way, the last ones stay, lined up with their lines (a line typed a
    /// moment ago has none yet).
    fn take_answers(&mut self) {
        if let Some(done) = self.calc.take()
            && done.sheet == self.store.current().id
        {
            let text = &self.store.current().text;
            self.results_exact = done.complete && done.text == *text;
            self.results = if self.results_exact {
                done.results
            } else {
                // Some still on their way: those lines keep the answers they show, dimmed.
                engine::realign_onto(&done.text, &done.results, text, &self.results)
            };
            self.results_text = text.clone();
        }
    }

    fn touched(&mut self) {
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    fn sheet_switched(&mut self) {
        self.preview = None;
        self.popup = None;
        self.copied = None;
        // Another sheet's answers mustn't show against this one's lines.
        self.results.clear();
        self.results_text.clear();
        self.results_exact = false;
        self.focus_editor = true;
        self.evaluate();
        self.touched();
    }

    fn apply(&mut self, action: SheetAction) {
        match action {
            SheetAction::Select(i) => self.store.select(i),
            SheetAction::New => self.store.new_sheet(""),
            SheetAction::Delete(i) => {
                if i < self.store.sheets.len() {
                    self.pending_delete = Some(i);
                    self.switcher = None;
                    self.popup = None;
                }
                return;
            }
            SheetAction::Move(i, step) => {
                self.store.select(i);
                self.store.move_active(step);
            }
            SheetAction::Copy(i) => {
                self.copy_with(i, engine::with_answers, "Page copied with answers");
                return;
            }
            SheetAction::Share(i) => {
                let link = omacus_engine::share::link(&self.store.sheets[i].text);
                let note = match link.len() {
                    n if n > 8000 => " (long: some apps may cut it off)",
                    n if n > 2000 => " (fine for chat; some email and SMS may cut it)",
                    _ => "",
                };
                self.copy = None;
                self.toast = Some(Toast::info(format!(
                    "Share link copied, {} characters{note}",
                    link.len()
                )));
                self.ctx.copy_text(link);
                return;
            }
            SheetAction::CopyMarkdown(i) => {
                self.copy_with(i, engine::markdown_table, "Page copied as a Markdown table");
                return;
            }
            SheetAction::Settings => {
                self.open_settings();
                return;
            }
            SheetAction::Preview => {
                self.page_preview = !self.page_preview;
                self.popup = None;
                if !self.page_preview {
                    self.focus_editor = true;
                }
                return;
            }
        }
        self.sheet_switched();
    }

    fn handle_background_updates(&mut self, ctx: &egui::Context) {
        let system_dark = is_system_dark(ctx);
        if self.theme_changed.swap(false, Ordering::Relaxed) || system_dark != self.system_dark {
            self.system_dark = system_dark;
            self.reload_theme(ctx);
        }
        // Both are no-ops unless their schedule is due (rates every 12 hours, updates every 4).
        let repaint = ctx.clone();
        self.rates
            .refresh_if_stale(move || repaint.request_repaint());
        let repaint = ctx.clone();
        self.updates.check_if_due(move || repaint.request_repaint());

        self.take_copy(ctx);

        // Share links from the command line, macOS, or a paste.
        for text in crate::links::take() {
            let new = self.store.open(&text);
            self.toast = Some(Toast::info(if new {
                "Opened the shared page"
            } else {
                "You already have this page; switched to it"
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
        let name = self
            .preview
            .as_ref()
            .and_then(|id| self.trash.iter().find(|t| &t.id == id))
            .map(|t| t.title.clone())
            .unwrap_or_else(|| self.store.sheets[self.store.active].title());
        let title = format!("Omacus — {name}");
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        ctx.request_repaint_after(Duration::from_secs(60));
    }

    /// Keys are consumed here, before the editor sees them.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let chords = ctx.input_mut(|i| take_chords(&mut i.events));
        let mut toggle_switcher = false;
        let mut prev_sheet = false;
        let mut next_sheet = false;
        let mut quit = false;
        let mut dismiss = false;
        let mut open_settings = false;
        let mut toggle_answers = false;
        let mut toggle_preview = false;
        for chord in chords {
            match chord {
                Shortcut::ToggleSwitcher => toggle_switcher = true,
                Shortcut::PrevSheet => prev_sheet = true,
                Shortcut::NextSheet => next_sheet = true,
                Shortcut::Quit => quit = true,
                Shortcut::Dismiss => dismiss = true,
                Shortcut::Settings => open_settings = true,
                Shortcut::ToggleAnswers => toggle_answers = true,
                Shortcut::Preview => toggle_preview = true,
            }
        }
        if quit {
            self.save();
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        let toggle_help = ctx.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::F1) || i.consume_key(Modifiers::COMMAND, Key::Slash)
        });
        if toggle_help {
            self.toggle_help();
        }
        if open_settings {
            if self.settings_open {
                self.settings_open = false;
                self.focus_editor = true;
            } else {
                self.open_settings();
            }
        }
        if toggle_answers {
            self.settings.hide_answers = !self.settings.hide_answers;
            self.persist_settings();
            self.toast = Some(Toast::info(if self.settings.hide_answers {
                "Answers sit after each line"
            } else {
                "Answers are in the column"
            }));
        }
        if toggle_preview {
            self.page_preview = !self.page_preview;
            self.popup = None;
            if !self.page_preview {
                self.focus_editor = true;
            }
        }
        if toggle_switcher {
            self.switcher = match self.switcher {
                Some(_) => None,
                None => Some(Switcher::default()),
            };
            self.settings_open = false;
            self.help = None;
            self.popup = None;
            self.pending_delete = None;
            // Opening or closing must not call `request_focus`. That interrupts
            // IME, and on Wayland the compositor then drops the key release, so
            // winit's repeat timer never stops.
            self.focus_editor = false;
        }
        if prev_sheet {
            self.apply(self.cycle_action(-1));
        }
        if next_sheet {
            self.apply(self.cycle_action(1));
        }
        let closed_switcher = dismiss && self.switcher.is_some();
        if closed_switcher {
            self.switcher = None;
            self.focus_editor = false;
        }
        if self.switcher.is_some() || toggle_switcher || closed_switcher {
            if self.switcher.is_some() {
                self.refresh_trash(false);
            }
            self.switcher_keys(ctx);
            return;
        }
        if self.purge_id.is_some() {
            if dismiss {
                self.purge_id = None;
            }
            return;
        }
        if self.pending_delete.is_some() {
            let confirm = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            if dismiss {
                self.pending_delete = None;
                self.focus_editor = true;
            } else if confirm {
                self.commit_delete();
            }
            return;
        }
        if self.update_open {
            if dismiss && self.updates.state() != State::Installing {
                self.update_open = false;
                self.focus_editor = true;
            }
            return;
        }
        if self.help.is_some() {
            if dismiss {
                self.help = None;
                self.focus_editor = true;
            }
            return;
        }
        if self.settings_open {
            if dismiss {
                self.settings_open = false;
                self.focus_editor = true;
            }
            return;
        }
        if self.page_preview && dismiss {
            self.page_preview = false;
            self.focus_editor = true;
            return;
        }

        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;

        // Ctrl+Space (Control on every platform, so macOS Spotlight keeps ⌘Space)
        // opens suggestions on purpose. Enter accepts only that popup.
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Space)) {
            self.open_completion(true);
        }

        if let Some(popup) = &mut self.popup {
            let explicit = popup.explicit;
            let (tab, enter, up, down) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::Tab),
                    // Enter stays a newline unless the popup was opened on purpose,
                    // so `3 km in⏎` does not become `inch`.
                    explicit && i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                )
            });
            let close = dismiss;
            let n = popup.items.len();
            if up {
                popup.selected = (popup.selected + n - 1) % n;
            }
            if down {
                popup.selected = (popup.selected + 1) % n;
            }
            if close {
                self.popup = None;
            } else if tab || enter {
                let pick = popup.selected;
                self.accept_completion(pick);
            }
        }

        let mut action = None;
        let (copy_last, sidebar) = ctx.input_mut(|i| {
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
            (i.consume_key(cmd_shift, Key::C), i.consume_key(cmd, Key::B))
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
            self.toast = Some(Toast::info(format!("Copied {}", r.display)));
        }
    }

    /// Open suggestions for the word at the caret. `explicit` is Ctrl+Space:
    /// Enter then accepts. Typing opens the same list without that.
    fn open_completion(&mut self, explicit: bool) {
        let cursor = self.store.current().cursor;
        let text = self.store.current().text.clone();
        let Some((start, word)) = complete::word_before(&text, cursor) else {
            return;
        };
        let items = complete::suggest(&word, &self.vars);
        if items.is_empty() {
            return;
        }
        let (anchor, anchored, selected) = match &self.popup {
            Some(popup) if popup.end == cursor => (popup.anchor, popup.anchored, popup.selected),
            _ => (Pos2::ZERO, false, 0),
        };
        self.popup = Some(Popup {
            start,
            end: cursor,
            selected: selected.min(items.len() - 1),
            items,
            anchor,
            anchored,
            explicit,
        });
    }

    fn commit_delete(&mut self) {
        let Some(i) = self.pending_delete.take() else {
            return;
        };
        if i >= self.store.sheets.len() {
            return;
        }
        let title = self.store.sheets[i].title();
        self.store.delete(i);
        self.trash_at = None;
        self.toast = Some(Toast::undo(format!("Deleted \u{201c}{title}\u{201d}")));
        self.sheet_switched();
    }

    fn restore_trashed(&mut self, id: &str) {
        if let Some(title) = self.store.restore(id) {
            self.trash_at = None;
            self.toast = Some(Toast::info(format!("Restored \u{201c}{title}\u{201d}")));
            self.sheet_switched();
        }
    }

    /// Open a deleted sheet to read. Restoring is a separate button.
    fn open_preview(&mut self, id: String) {
        self.refresh_trash(true);
        if self.trash.iter().any(|t| t.id == id) {
            self.preview = Some(id);
            self.popup = None;
            self.focus_editor = false;
        }
    }

    fn current_preview(&mut self) -> Option<Trashed> {
        let id = self.preview.clone()?;
        self.refresh_trash(false);
        match self.trash.iter().find(|t| t.id == id).cloned() {
            Some(item) => Some(item),
            None => {
                self.preview = None;
                None
            }
        }
    }

    fn purge_trashed(&mut self, id: &str) {
        let title = self
            .trash
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.title.clone());
        if self.store.purge(id) {
            self.trash_at = None;
            self.refresh_trash(true);
            if self.preview.as_deref() == Some(id) {
                self.preview = None;
                self.focus_editor = true;
            }
            self.purge_id = None;
            if let Some(title) = title {
                self.toast = Some(Toast::info(format!(
                    "Deleted \u{201c}{title}\u{201d} forever"
                )));
            }
        }
    }

    fn refresh_trash(&mut self, force: bool) {
        let stale = force || self.trash_at.is_none_or(|t| t.elapsed() >= TRASH_REFRESH);
        if stale {
            self.trash = self.store.trashed();
            self.trash_at = Some(Instant::now());
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
        let Some(word) = popup.items.get(pick).map(|item| item.text.clone()) else {
            return;
        };
        let sheet = self.store.current();
        let chars: Vec<char> = sheet.text.chars().collect();
        let mut text: String = chars[..popup.start].iter().collect();
        text.push_str(&word);
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
        self.settings_open = false;
        self.popup = None;
        self.focus_editor = self.help.is_none();
    }

    fn open_settings(&mut self) {
        self.settings_open = true;
        self.help = None;
        self.switcher = None;
        self.popup = None;
        self.update_open = false;
        self.pending_delete = None;
        self.focus_editor = false;
    }

    fn persist_settings(&mut self) {
        if let Err(e) = settings::save(&self.settings) {
            self.toast = Some(Toast::error(format!("Couldn't save settings: {e}")));
        }
    }

    fn reload_theme(&mut self, ctx: &egui::Context) {
        self.theme = Theme::resolve(self.settings.theme, self.system_dark);
        apply_visuals(ctx, &self.theme);
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let outcome = settings::show(ctx, &mut self.settings, &self.theme);
        if outcome.changed {
            self.persist_settings();
            self.reload_theme(ctx);
        }
        if outcome.close {
            self.settings_open = false;
            self.focus_editor = true;
        }
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

    /// Switcher keystrokes, taken out of the event list before the sheet editor
    /// sees them. The sheet keeps keyboard focus, so the IME cursor stays put.
    fn switcher_keys(&mut self, ctx: &egui::Context) {
        let edits = ctx.input_mut(|i| take_switcher_edits(&mut i.events));
        if self.switcher.is_none() {
            return;
        }
        // The hit list is only needed to move or confirm, and only for the
        // query those keys apply to. Typing does not search.
        let mut hits: Option<Vec<Hit>> = None;
        for edit in edits {
            let outcome = {
                let Some(sw) = &mut self.switcher else { return };
                match &edit {
                    SwitcherEdit::Type(_) | SwitcherEdit::Backspace => {
                        apply_switcher_query(&mut sw.query, &mut sw.selected, &edit);
                        SwitcherOutcome::Stay
                    }
                    SwitcherEdit::Up => SwitcherOutcome::Move(-1),
                    SwitcherEdit::Down => SwitcherOutcome::Move(1),
                    SwitcherEdit::Confirm => SwitcherOutcome::Confirm,
                    SwitcherEdit::Close => SwitcherOutcome::Close,
                }
            };
            match outcome {
                SwitcherOutcome::Stay => hits = None,
                SwitcherOutcome::Close => {
                    self.switcher = None;
                    self.focus_editor = false;
                    return;
                }
                SwitcherOutcome::Move(step) => {
                    let n = self.switcher_hits_cached(&mut hits).len().max(1);
                    if let Some(sw) = &mut self.switcher {
                        sw.selected = (sw.selected as isize + step).rem_euclid(n as isize) as usize;
                    }
                }
                SwitcherOutcome::Confirm => {
                    let selected = self.switcher.as_ref().map(|s| s.selected).unwrap_or(0);
                    if let Some(hit) = self.switcher_hits_cached(&mut hits).get(selected) {
                        let jump = hit.jump.clone();
                        self.switcher = None;
                        self.focus_editor = false;
                        self.follow(jump);
                    }
                    return;
                }
            }
        }
    }

    fn switcher_hits_cached<'a>(&self, hits: &'a mut Option<Vec<Hit>>) -> &'a [Hit] {
        hits.get_or_insert_with(|| self.switcher_hits())
    }

    fn switcher_hits(&self) -> Vec<Hit> {
        let Some(sw) = &self.switcher else {
            return Vec::new();
        };
        let titles: Vec<String> = self.store.sheets.iter().map(|s| s.title()).collect();
        let texts: Vec<&str> = self.store.sheets.iter().map(|s| s.text.as_str()).collect();
        let mut hits = Vec::new();
        if settings_command(&sw.query) {
            hits.push(Hit {
                title: "Settings".to_string(),
                jump: Jump::Settings,
                badge: "command",
            });
        }
        hits.extend(
            find_sheets(&titles, &texts, &sw.query)
                .into_iter()
                .map(|i| Hit {
                    title: titles[i].clone(),
                    badge: if i == self.store.active {
                        "current"
                    } else {
                        ""
                    },
                    jump: Jump::Sheet(i),
                }),
        );
        if !sw.query.trim().is_empty() {
            let titles: Vec<String> = self.trash.iter().map(|t| t.title.clone()).collect();
            let texts: Vec<&str> = self.trash.iter().map(|t| t.text.as_str()).collect();
            for i in find_sheets(&titles, &texts, &sw.query) {
                hits.push(Hit {
                    title: titles[i].clone(),
                    badge: "deleted",
                    jump: Jump::Trash(self.trash[i].id.clone()),
                });
            }
        }
        hits
    }

    fn follow(&mut self, jump: Jump) {
        match jump {
            Jump::Sheet(i) => self.apply(SheetAction::Select(i)),
            Jump::Trash(id) => self.open_preview(id),
            Jump::Settings => self.open_settings(),
        }
    }

    /// The Ctrl+J box: a search field and the matching sheets.
    fn show_switcher(&mut self, ctx: &egui::Context) {
        if self.switcher.is_none() {
            return;
        }
        self.refresh_trash(false);
        let theme = self.theme.clone();
        let hits = self.switcher_hits();
        let mut picked: Option<Jump> = None;
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
                // Display only. A focused field here would move the IME cursor
                // on every keystroke; the sheet keeps focus and `switcher_keys`
                // applies the characters.
                let mut shown = sw.query.clone();
                let field = ui.add(
                    TextEdit::singleline(&mut shown)
                        .interactive(false)
                        .hint_text("Go to page…")
                        .font(FontId::proportional(15.0))
                        .text_color(theme.foreground)
                        .frame(egui::Frame::NONE.inner_margin(Margin::symmetric(8, 6)))
                        .desired_width(f32::INFINITY),
                );
                if switcher_takes_focus() {
                    field.request_focus();
                }
                let selected = sw.selected;
                ui.add_space(4.0);
                if hits.is_empty() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("  No page matches").color(theme.comment));
                    ui.add_space(4.0);
                }
                for (row, hit) in hits.iter().take(9).enumerate() {
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
                        &hit.title,
                        FontId::proportional(14.0),
                        color,
                    );
                    if !hit.badge.is_empty() {
                        ui.painter().text(
                            rect.right_center() - vec2(10.0, 0.0),
                            Align2::RIGHT_CENTER,
                            hit.badge,
                            FontId::proportional(12.0),
                            theme.comment,
                        );
                    }
                    if resp.clicked() {
                        picked = Some(hit.jump.clone());
                    }
                }
            });
        if modal.should_close() {
            close = true;
        }
        if let Some(jump) = picked {
            self.switcher = None;
            self.focus_editor = false;
            self.follow(jump);
        } else if close {
            self.switcher = None;
            self.focus_editor = false;
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
                "\u{201c}{title}\u{201d} changed in another window too; its version is a new page"
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
        if !focused && let Some(copy) = &mut self.copy {
            copy.focused = false;
        }
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
                format!("Updated. Restart Omacus to use {latest}; your pages are saved.")
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

    /// ⋯ above the sheet, lined up with the answers: the actions for this sheet (share,
    /// copy, move, delete) without a right-click.
    fn sheet_bar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let i = self.store.active;
        let count = self.store.sheets.len();
        let mut action = None;
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        // Framed, like the web app's: the one control above the sheet. Its right edge
        // lines up with the answers, which leave room for the scrollbar.
        let dots_rect = Rect::from_center_size(
            pos2(row.right() - SCROLLBAR_ROOM - 15.0, row.center().y),
            vec2(30.0, 30.0),
        );
        let dots = ui.interact(dots_rect, ui.scope_id().with("bar-dots"), Sense::click());
        let menu = egui::Popup::menu(&dots).align(egui::RectAlign::BOTTOM_END);
        let lit = dots.hovered() || menu.is_open();
        let painter = ui.painter();
        if lit {
            painter.rect_filled(dots_rect, CornerRadius::same(7), theme.hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        painter.rect_stroke(
            dots_rect,
            CornerRadius::same(7),
            egui::Stroke::new(1.0, theme.selection),
            egui::StrokeKind::Inside,
        );
        let color = if lit { theme.foreground } else { theme.dim };
        for dx in [-6.0, 0.0, 6.0] {
            painter.circle_filled(dots_rect.center() + vec2(dx, 0.0), 2.0, color);
        }
        menu.show(|ui| sheet_menu(ui, i, count, &mut action));
        dots.on_hover_text("Share, copy and more");
        ui.add_space(2.0);

        if let Some(a) = action {
            self.apply(a);
            self.focus_editor = true;
        }
    }

    /// One quiet row: the logo, name and version (opens About, or the update when
    /// one is out, marked with a dot), and "?" for help.
    fn sidebar_footer(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        let available = self.updates.available();
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        let help_rect = icon_slot(row, 0);
        let gear_rect = icon_slot(row, 1);
        let about_rect = Rect::from_min_max(row.min, pos2(gear_rect.left() - 4.0, row.bottom()));
        let about = ui.interact(about_rect, ui.scope_id().with("about"), Sense::click());
        let gear = icon_button(
            ui,
            gear_rect,
            ui.scope_id().with("settings"),
            theme,
            Icon::Gear,
        );
        let help = icon_button(ui, help_rect, ui.scope_id().with("help"), theme, Icon::Help);

        let version_text = format!("v{}", update::current());
        let version_color = if available.is_some() {
            theme.accent
        } else {
            theme.comment
        };
        let version_width = ui.ctx().fonts_mut(|f| {
            f.layout_no_wrap(
                version_text.clone(),
                FontId::proportional(12.0),
                version_color,
            )
            .size()
            .x
        });
        let painter = ui.painter();
        if about.hovered() {
            painter.rect_filled(about_rect, CornerRadius::same(6), theme.hover);
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
        let version_x = name.right() + 6.0;
        if version_x + version_width < gear_rect.left() - 6.0 {
            let version = ui.painter().text(
                pos2(version_x, row.center().y),
                Align2::LEFT_CENTER,
                version_text,
                FontId::proportional(12.0),
                version_color,
            );
            if available.is_some() {
                ui.painter().circle_filled(
                    version.right_center() + vec2(7.0, 0.0),
                    3.0,
                    theme.accent,
                );
            }
        }
        if about.hovered() {
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
        if gear
            .on_hover_text(format!("Settings ({})", help::keys("Ctrl+,")))
            .clicked()
        {
            self.open_settings();
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let mut action = None;
        let mut preview = None;
        let mut restore = None;
        let mut purge = None;
        self.refresh_trash(false);

        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_ROW), Sense::hover());
        ui.painter().text(
            pos2(row.left() + 14.0, row.center().y),
            Align2::LEFT_CENTER,
            "PAGES",
            FontId::proportional(11.0),
            theme.comment,
        );
        let id = ui.scope_id();
        let collapse = icon_button(
            ui,
            icon_slot(row, 0),
            id.with("collapse"),
            &theme,
            Icon::Sidebar,
        );
        let plus = icon_button(
            ui,
            icon_slot(row, 1),
            id.with("new-sheet"),
            &theme,
            Icon::Plus,
        );
        if plus
            .on_hover_text(format!("New page ({})", help::keys("Ctrl+N")))
            .clicked()
        {
            action = Some(SheetAction::New);
        }
        if collapse
            .on_hover_text(format!("Collapse sidebar ({})", help::keys("Ctrl+B")))
            .clicked()
        {
            self.store.toggle_sidebar();
            self.touched();
        }
        ui.add_space(6.0);

        let rows: Vec<(usize, String)> = self
            .store
            .sheets
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.heading()))
            .collect();
        let count = rows.len();
        let viewing = self.preview.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - 38.0)
            .show(ui, |ui| {
                for (i, heading) in rows {
                    let active = i == self.store.active && viewing.is_none();
                    let resp = sidebar_row(ui, &theme, &heading, active, true);
                    let dots = sidebar_dots(ui, &resp.rect, ui.scope_id().with(("sheet-dots", i)));
                    let menu = egui::Popup::menu(&dots);
                    let open = menu.is_open();
                    if active || resp.hovered() || dots.hovered() || open {
                        dots_button(ui, &dots, &theme, open);
                    }
                    menu.show(|ui| sheet_menu(ui, i, count, &mut action));
                    let pick = resp.clicked() && !dots.clicked();
                    dots.on_hover_text("Share, copy and more");
                    if pick {
                        action = Some(SheetAction::Select(i));
                    }
                    resp.context_menu(|ui| sheet_menu(ui, i, count, &mut action));
                }
                if !self.trash.is_empty() {
                    ui.add_space(14.0);
                    section_label(ui, &theme, "RECENTLY DELETED");
                    ui.add_space(4.0);
                    let trash = self.trash.clone();
                    for item in trash {
                        let heading = heading_of(&item.text);
                        let selected = viewing.as_deref() == Some(item.id.as_str());
                        let resp = sidebar_row(ui, &theme, &heading, selected, false);
                        let dots = sidebar_dots(
                            ui,
                            &resp.rect,
                            ui.scope_id().with(("trash-dots", item.id.clone())),
                        );
                        let menu = egui::Popup::menu(&dots);
                        let open = menu.is_open();
                        let mut choice = None;
                        if selected || resp.hovered() || dots.hovered() || open {
                            dots_button(ui, &dots, &theme, open);
                        }
                        menu.show(|ui| trash_menu(ui, &mut choice));
                        let pick = resp.clicked() && !dots.clicked();
                        dots.on_hover_text("Restore or delete forever");
                        resp.context_menu(|ui| trash_menu(ui, &mut choice));
                        if pick {
                            preview = Some(item.id.clone());
                        }
                        match choice {
                            Some(TrashChoice::Restore) => restore = Some(item.id),
                            Some(TrashChoice::Forever) => purge = Some(item.id),
                            None => {}
                        }
                    }
                }
            });

        ui.add_space(6.0);
        self.sidebar_footer(ui, &theme);

        if let Some(a) = action {
            self.apply(a);
        }
        if let Some(id) = preview {
            self.open_preview(id);
        }
        if let Some(id) = restore {
            self.restore_trashed(&id);
        }
        if let Some(id) = purge {
            self.purge_id = Some(id);
        }
    }

    fn sheet(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let font = FontId::monospace(FONT_SIZE);
        // Each line's answer width, swatch included, for placing it after the text.
        let idle = self.edited_at.elapsed();
        let cursor_line = self.cursor_line;
        let (row_height, answers) = ui.ctx().fonts_mut(|f| {
            let answers: Vec<Option<f32>> = self
                .results
                .iter()
                .enumerate()
                .map(|(line, r)| {
                    if r.hint && !r.stale && r.note.is_some() {
                        let editing = cursor_line == Some(line);
                        return name_hint_visible(editing, idle).then_some(16.0);
                    }
                    let text = match (&r.kind, r.note.as_deref()) {
                        (Kind::Value, _) if !r.display.is_empty() => r.display.clone(),
                        (Kind::Error, Some(note)) if !r.stale && !r.hint => note.to_string(),
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
            .event_filter(sheet_event_filter(self.switcher.is_some()))
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
        if self.switcher.is_some() {
            self.focus_editor = false;
        } else if std::mem::take(&mut self.focus_editor) {
            response.request_focus();
        }

        let changed = response.changed();
        if changed {
            self.store.current().mark_dirty();
            self.edited_at = Instant::now();
            self.evaluate();
            self.touched();
        }

        let cursor = output
            .cursor_range
            .filter(|r| r.primary == r.secondary)
            .map(|r| r.primary.index.0);
        if let Some(c) = output.cursor_range.map(|r| r.primary.index.0) {
            self.cursor_line = Some(line_at(&self.store.current().text, c));
            self.store.set_cursor(c);
            if self.store.has_unsaved() {
                self.touched();
            }
        }

        // Suggest while typing; any other cursor movement closes the popup.
        // A popup opened with Ctrl+Space keeps Enter-to-accept while the word grows.
        match cursor {
            Some(c) if changed && response.has_focus() => {
                let explicit = self.popup.as_ref().is_some_and(|p| p.explicit);
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
                        anchored: true,
                        explicit,
                    })
                });
            }
            Some(c) if self.popup.as_ref().is_some_and(|p| p.end == c) => {}
            _ => self.popup = None,
        }
        if let Some(popup) = &mut self.popup
            && !popup.anchored
            && let Some(c) = cursor
        {
            let at = output.galley.pos_from_cursor(CCursor::new(c));
            popup.anchor = output.galley_pos + at.left_bottom().to_vec2() + vec2(0.0, 2.0);
            popup.anchored = true;
        }
        if response.hovered() {
            self.show_hover(ui, &output.galley, output.galley_pos);
        }

        if self.store.current().text.trim().is_empty() {
            ui.painter().text(
                response.rect.left_top() + vec2(2.0, 2.0),
                Align2::LEFT_TOP,
                empty_sheet_hint(self.store.sidebar),
                FontId::proportional(14.0),
                theme.comment,
            );
        }

        // Paint each answer at the end of its line, or on the row made for it below.
        // The column keeps them on the right; hiding it sits each one after the text.
        let column_right = ui.max_rect().right() - SCROLLBAR_ROOM;
        let inline = self.settings.hide_answers;
        let origin = output.galley_pos;
        let mut line = 0;
        let rows = &output.galley.rows;
        for (i, row) in rows.iter().enumerate() {
            let last_of_line = row.ends_with_newline || i + 1 == rows.len();
            if !last_of_line {
                continue;
            }
            if let Some(Some(width)) = answers.get(line) {
                let dropped = answer_drops(row.rect().max.x, *width, editor_width);
                // A heading is taller than a normal row, so the answer sits on the
                // row itself (or in the gap just below it) rather than a fixed step.
                let content_h = row.rect().height().max(row_height);
                let (y, h) = if dropped {
                    (origin.y + row.pos.y + content_h, row_height)
                } else {
                    (origin.y + row.pos.y, content_h)
                };
                let anchor = if inline {
                    let left = if dropped {
                        origin.x
                    } else {
                        origin.x + row.rect().max.x + GAP
                    };
                    AnswerAnchor::Left(left)
                } else {
                    AnswerAnchor::Right(column_right)
                };
                self.paint_result(ui, line, anchor, y, h, &font);
            }
            line += 1;
        }
    }

    fn paint_result(
        &mut self,
        ui: &egui::Ui,
        line: usize,
        anchor: AnswerAnchor,
        y: f32,
        h: f32,
        font: &FontId,
    ) {
        let Some(result) = self.results.get(line) else {
            return;
        };
        let theme = &self.theme;
        // No answer, but a reason ("Can't divide by zero"): shown quietly, not copyable.
        if let (Kind::Error, Some(note), false) =
            (&result.kind, result.note.as_deref(), result.stale)
        {
            if result.hint {
                let editing = self.cursor_line == Some(line);
                if !name_hint_visible(editing, self.edited_at.elapsed()) {
                    if let Some(left) = HINT_IDLE.checked_sub(self.edited_at.elapsed()) {
                        ui.ctx().request_repaint_after(left);
                    }
                    return;
                }
                let center = match anchor {
                    AnswerAnchor::Right(right) => pos2(right - 8.0, y + h / 2.0),
                    AnswerAnchor::Left(left) => pos2(left + 8.0, y + h / 2.0),
                };
                ui.painter().circle_filled(center, 3.0, theme.comment);
                let rect = Rect::from_center_size(center, vec2(16.0, h));
                let resp = ui.interact(
                    rect,
                    ui.scope_id().with(("name-hint", line)),
                    Sense::hover(),
                );
                resp.on_hover_text(note);
                return;
            }
            let galley = ui
                .ctx()
                .fonts_mut(|f| f.layout_no_wrap(note.to_string(), font.clone(), theme.comment));
            let size = galley.size();
            let x = match anchor {
                AnswerAnchor::Right(right) => right - size.x,
                AnswerAnchor::Left(left) => left,
            };
            ui.painter()
                .galley(pos2(x, y + (h - size.y) / 2.0), galley, theme.comment);
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

        // Stale: from before an edit above it, until its new answer arrives.
        let color = if copied || result.stale {
            theme.dim
        } else {
            theme.accent
        };
        let galley = ui
            .ctx()
            .fonts_mut(|f| f.layout_no_wrap(text, font.clone(), color));
        let size = galley.size();
        // A colour answer gets a swatch of it, left of the text.
        let swatch = result.color.filter(|_| !copied);
        let extra = if swatch.is_some() {
            swatch_room(font)
        } else {
            0.0
        };
        let content_left = match anchor {
            AnswerAnchor::Right(right) => right - size.x - extra,
            AnswerAnchor::Left(left) => left,
        };
        let rect = Rect::from_min_size(pos2(content_left, y), vec2(size.x + extra, h))
            .expand2(vec2(6.0, 1.0));
        let response = ui.interact(rect, ui.scope_id().with(("result", line)), Sense::click());

        if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), theme.hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            let display = result.display.clone();
            ui.ctx().copy_text(display.clone());
            self.copied = Some((line, Instant::now()));
            self.toast = Some(Toast::info(format!("Copied {display}")));
            ui.ctx().request_repaint_after(COPIED_FOR);
        }
        if let Some([r, g, b]) = swatch {
            let side = font.size * 0.8;
            let square =
                Rect::from_min_size(pos2(content_left, y + (h - side) / 2.0), vec2(side, side));
            ui.painter()
                .rect_filled(square, CornerRadius::same(3), Color32::from_rgb(r, g, b));
            ui.painter().rect_stroke(
                square,
                CornerRadius::same(3),
                egui::Stroke::new(1.0, theme.selection),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().galley(
            pos2(content_left + extra, y + (h - size.y) / 2.0),
            galley,
            color,
        );
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
                            .map(|item| {
                                item.text.chars().count()
                                    + item.kind_label().chars().count()
                                    + item.detail.chars().count()
                                    + 6
                            })
                            .max()
                            .unwrap_or(8) as f32
                            * (FONT_SIZE - 2.0)
                            * 0.62
                            + 16.0;
                        let width = width.clamp(220.0, 520.0);
                        for (i, item) in popup.items.iter().enumerate() {
                            let (rect, resp) =
                                ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
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
                                &item.text,
                                font.clone(),
                                color,
                            );
                            let meta = format!("{} · {}", item.kind_label(), item.detail);
                            ui.painter().text(
                                pos2(rect.right() - 8.0, rect.center().y),
                                Align2::RIGHT_CENTER,
                                meta,
                                font.clone(),
                                if selected {
                                    theme.foreground
                                } else {
                                    theme.dim
                                },
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

    /// The value of the variable or number under the pointer.
    fn show_hover(&mut self, ui: &egui::Ui, galley: &egui::Galley, origin: Pos2) {
        let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let cc = galley.cursor_from_pos(pointer - origin);
        let text = self.store.current().text.clone();
        let Some(hit) = self.lookup_hover(&text, cc.index.0) else {
            return;
        };
        let start = galley.pos_from_cursor(CCursor::new(hit.start));
        let end = galley.pos_from_cursor(CCursor::new(hit.end));
        let rect = Rect::from_min_max(
            origin + start.left_top().to_vec2(),
            origin + end.right_bottom().to_vec2(),
        )
        .expand(3.0);
        if !rect.contains(pointer) {
            return;
        }
        paint_hover_tip(ui.ctx(), &self.theme, rect, &hit.value);
    }

    fn lookup_hover(&mut self, text: &str, cursor: usize) -> Option<engine::Hover> {
        if let Some(cache) = &self.hover
            && cache.text == text
        {
            if let Some(hit) = &cache.hit
                && cursor >= hit.start
                && cursor <= hit.end
            {
                return Some(hit.clone());
            }
            if cache.hit.is_none() && cache.cursor == cursor {
                return None;
            }
        }
        let hit = self.engine.hover_at(text, cursor);
        self.hover = Some(HoverCache {
            text: text.to_string(),
            cursor,
            hit: hit.clone(),
        });
        hit
    }

    /// A narrow strip when the sidebar is collapsed: the sidebar button on the top
    /// row and + under it, the logo at the bottom where the open sidebar has it.
    /// A click anywhere else on the strip opens it too.
    fn sidebar_rail(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let area = ui.max_rect();
        let id = ui.scope_id();
        let rail = ui.interact(area, id.with("sidebar-rail"), Sense::click());
        let x = area.left() + SIDEBAR_RAIL / 2.0;
        let toggle =
            Rect::from_center_size(pos2(x, area.top() + BAR_ROW / 2.0), Vec2::splat(ICON_BOX));
        let logo = Rect::from_center_size(pos2(x, area.bottom() - BAR_ROW / 2.0), vec2(18.0, 18.0));
        let gear_rect = Rect::from_center_size(
            pos2(x, logo.center().y - (ICON_BOX + 8.0)),
            Vec2::splat(ICON_BOX),
        );
        let gear = icon_button(ui, gear_rect, id.with("rail-settings"), &theme, Icon::Gear);
        let plus = icon_button(
            ui,
            toggle.translate(vec2(0.0, ICON_BOX + 4.0)),
            id.with("rail-new"),
            &theme,
            Icon::Plus,
        );
        let painter = ui.painter();
        let color = if rail.hovered() {
            painter.rect_filled(toggle, CornerRadius::same(6), theme.hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            theme.foreground
        } else {
            theme.dim
        };
        paint_icon(painter, Icon::Sidebar, toggle.center(), color);
        painter.image(
            self.logo.small.id(),
            logo,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        if plus
            .on_hover_text(format!("New page ({})", help::keys("Ctrl+N")))
            .clicked()
        {
            self.apply(SheetAction::New);
        } else if gear
            .on_hover_text(format!("Settings ({})", help::keys("Ctrl+,")))
            .clicked()
        {
            self.open_settings();
        } else if rail
            .on_hover_text(format!("Expand sidebar ({})", help::keys("Ctrl+B")))
            .clicked()
        {
            self.store.toggle_sidebar();
            self.touched();
        }
    }

    fn show_page_preview(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let (text, results) = if let Some(id) = &self.preview {
            let text = self
                .trash
                .iter()
                .find(|t| &t.id == id)
                .map(|t| t.text.clone())
                .unwrap_or_default();
            let results = self.engine.eval_document(&text);
            (text, results)
        } else {
            (self.store.current().text.clone(), self.results.clone())
        };
        let title = crate::sheets::heading_of(&text);
        if let Some(action) = preview::show(ui, &theme, &text, &results) {
            match action {
                preview::Action::Close => {
                    self.page_preview = false;
                    self.focus_editor = true;
                }
                preview::Action::Print => {
                    match pdf::save(&title, &preview::blocks(&text, &results), true) {
                        Ok(_) => {
                            self.toast = Some(Toast::info("Opened the preview to print"));
                        }
                        Err(e) => self.toast = Some(Toast::error(e)),
                    }
                }
                preview::Action::Export => {
                    match pdf::save(&title, &preview::blocks(&text, &results), false) {
                        Ok(path) => {
                            self.toast = Some(Toast::info(format!("Saved {}", path.display())));
                        }
                        Err(e) => self.toast = Some(Toast::error(e)),
                    }
                }
                preview::Action::Copy(text) => {
                    self.toast = Some(Toast::info(format!("Copied {text}")));
                }
            }
        }
    }

    fn preview_sheet(&mut self, ui: &mut egui::Ui, item: &Trashed) {
        let theme = self.theme.clone();
        let heading = heading_of(&item.text);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(heading)
                    .size(16.0)
                    .color(theme.foreground),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let forever = ui.add(
                    egui::Button::new(
                        egui::RichText::new("Delete forever")
                            .size(13.0)
                            .color(theme.dim),
                    )
                    .frame(false),
                );
                let restore = ui.add(
                    egui::Button::new(
                        egui::RichText::new("Restore")
                            .size(13.5)
                            .color(theme.background),
                    )
                    .fill(theme.accent)
                    .corner_radius(CornerRadius::same(6))
                    .min_size(vec2(0.0, 28.0)),
                );
                if forever.clicked() {
                    self.purge_id = Some(item.id.clone());
                }
                if restore.clicked() {
                    let id = item.id.clone();
                    self.restore_trashed(&id);
                }
            });
        });
        ui.label(
            egui::RichText::new("Recently deleted. This copy is read-only.")
                .size(12.5)
                .color(theme.comment),
        );
        ui.add_space(8.0);
        let mut text = item.text.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                TextEdit::multiline(&mut text)
                    .interactive(false)
                    .font(FontId::monospace(FONT_SIZE))
                    .desired_width(ui.available_width())
                    .frame(Frame::NONE)
                    .show(ui);
            });
    }

    fn show_delete(&mut self, ctx: &egui::Context) {
        let Some(i) = self.pending_delete else {
            return;
        };
        if i >= self.store.sheets.len() {
            self.pending_delete = None;
            return;
        }
        let title = self.store.sheets[i].title();
        let theme = self.theme.clone();
        let mut delete = false;
        let mut cancel = false;
        let frame = Frame::NONE
            .fill(theme.sidebar)
            .stroke(egui::Stroke::new(1.0, theme.selection))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(20, 16));
        let modal = egui::Modal::new(egui::Id::unique("omacus-delete"))
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(140))
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(
                    egui::RichText::new(format!("Delete \u{201c}{title}\u{201d}?"))
                        .size(16.0)
                        .color(theme.foreground),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "It's moved to Recently deleted, in the sidebar and the page switcher.",
                    )
                    .size(13.0)
                    .color(theme.dim),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    delete = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Delete")
                                    .size(13.5)
                                    .color(theme.background),
                            )
                            .fill(theme.accent)
                            .corner_radius(CornerRadius::same(6))
                            .min_size(vec2(0.0, 30.0)),
                        )
                        .clicked();
                    cancel = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Cancel")
                                    .size(13.5)
                                    .color(theme.foreground),
                            )
                            .corner_radius(CornerRadius::same(6))
                            .min_size(vec2(0.0, 30.0)),
                        )
                        .clicked();
                });
            });
        if modal.should_close() {
            cancel = true;
        }
        if delete {
            self.commit_delete();
        } else if cancel {
            self.pending_delete = None;
            self.focus_editor = true;
        }
    }

    fn show_purge(&mut self, ctx: &egui::Context) {
        let Some(id) = self.purge_id.clone() else {
            return;
        };
        let title = self
            .trash
            .iter()
            .find(|t| t.id == id)
            .map(|t| heading_of(&t.text))
            .unwrap_or_else(|| "this page".into());
        let theme = self.theme.clone();
        let mut purge = false;
        let mut cancel = false;
        let frame = Frame::NONE
            .fill(theme.sidebar)
            .stroke(egui::Stroke::new(1.0, theme.selection))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(20, 16));
        let modal = egui::Modal::new(egui::Id::unique("omacus-purge"))
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(140))
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(
                    egui::RichText::new(format!("Delete \u{201c}{title}\u{201d} forever?"))
                        .size(16.0)
                        .color(theme.foreground),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "This removes it from Recently deleted. It can't be restored.",
                    )
                    .size(13.0)
                    .color(theme.dim),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    purge = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Delete forever")
                                    .size(13.5)
                                    .color(theme.background),
                            )
                            .fill(theme.accent)
                            .corner_radius(CornerRadius::same(6))
                            .min_size(vec2(0.0, 30.0)),
                        )
                        .clicked();
                    cancel = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Cancel")
                                    .size(13.5)
                                    .color(theme.foreground),
                            )
                            .corner_radius(CornerRadius::same(6))
                            .min_size(vec2(0.0, 30.0)),
                        )
                        .clicked();
                });
            });
        if modal.should_close() {
            cancel = true;
        }
        if purge {
            self.purge_trashed(&id);
        } else if cancel {
            self.purge_id = None;
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
            let frame = Frame::NONE.fill(self.theme.sidebar).inner_margin(Margin {
                left: 10,
                right: 10,
                top: 8,
                bottom: 14,
            });
            let panel = egui::Panel::left("sheets")
                .resizable(true)
                .default_size(self.store.sidebar_width)
                .min_size(SIDEBAR_MIN)
                .max_size(SIDEBAR_MAX)
                .show_separator_line(false)
                .frame(frame)
                .show(ui, |ui| self.sidebar(ui));
            self.store.set_sidebar_width(panel.response.rect.width());
        } else {
            let frame = Frame::NONE.fill(self.theme.sidebar).inner_margin(Margin {
                left: 0,
                right: 0,
                top: 8,
                bottom: 14,
            });
            egui::Panel::left("sheets-rail")
                .resizable(false)
                .exact_size(SIDEBAR_RAIL)
                .show_separator_line(false)
                .frame(frame)
                .show(ui, |ui| self.sidebar_rail(ui));
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
            if self.page_preview {
                self.show_page_preview(ui);
            } else if let Some(item) = self.current_preview() {
                self.preview_sheet(ui, &item);
            } else {
                self.sheet_bar(ui);
                egui::ScrollArea::vertical()
                    .id_salt(("scroll", &self.store.sheets[self.store.active].id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.sheet(ui));
            }
        });

        self.popup(&ctx);
        self.show_help(&ctx);
        self.show_settings(&ctx);
        self.show_update(&ctx);
        self.show_delete(&ctx);
        self.show_purge(&ctx);
        self.show_switcher(&ctx);
        self.show_toast(&ctx);
    }

    // glow's `on_exit` receives the GL context. Windows enables wgpu only
    // (see Cargo.toml), and eframe's trait takes no context then.
    #[cfg(not(windows))]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save();
    }

    #[cfg(windows)]
    fn on_exit(&mut self) {
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
    if item(ui, "Preview", Some("Ctrl+P")) {
        *action = Some(SheetAction::Preview);
    }
    ui.separator();
    if item(ui, "New page", Some("Ctrl+N")) {
        *action = Some(SheetAction::New);
    }
    if i > 0 && item(ui, "Move up", None) {
        *action = Some(SheetAction::Move(i, -1));
    }
    if i + 1 < count && item(ui, "Move down", None) {
        *action = Some(SheetAction::Move(i, 1));
    }
    ui.separator();
    if item(ui, "Delete page", Some("Ctrl+Shift+Delete")) {
        *action = Some(SheetAction::Delete(i));
    }
    ui.separator();
    if item(ui, "Settings", Some("Ctrl+,")) {
        *action = Some(SheetAction::Settings);
    }
}

fn paint_dots(painter: &egui::Painter, center: Pos2, color: Color32) {
    for dx in [-5.0, 0.0, 5.0] {
        painter.circle_filled(center + vec2(dx, 0.0), 1.5, color);
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Plus,
    Sidebar,
    Help,
    Gear,
}

/// Drawn rather than typed, so they match each other, Share and ⋯ in any font.
fn paint_icon(painter: &egui::Painter, icon: Icon, c: Pos2, color: Color32) {
    let stroke = egui::Stroke::new(1.5, color);
    match icon {
        Icon::Plus => {
            painter.line_segment([c + vec2(-6.0, 0.0), c + vec2(6.0, 0.0)], stroke);
            painter.line_segment([c + vec2(0.0, -6.0), c + vec2(0.0, 6.0)], stroke);
        }
        // A window with its left panel marked off.
        Icon::Sidebar => {
            let r = Rect::from_center_size(c, vec2(16.0, 14.0));
            painter.rect_stroke(r, CornerRadius::same(3), stroke, egui::StrokeKind::Middle);
            let x = r.left() + 5.5;
            painter.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], stroke);
        }
        Icon::Help => {
            painter.circle_stroke(c, 7.5, stroke);
            painter.text(
                c + vec2(0.0, 0.5),
                Align2::CENTER_CENTER,
                "?",
                FontId::proportional(11.0),
                color,
            );
        }
        Icon::Gear => {
            painter.circle_stroke(c, 2.7, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * 4.3, c + d * 7.0], stroke);
            }
        }
    }
}

/// An icon in a square that lights up on hover, like ⋯.
fn icon_button(
    ui: &egui::Ui,
    rect: Rect,
    id: egui::Id,
    theme: &Theme,
    icon: Icon,
) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    let color = if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), theme.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        theme.foreground
    } else {
        theme.dim
    };
    paint_icon(ui.painter(), icon, rect.center(), color);
    resp
}

/// Where a hover value sits: above the token, below it when that would leave
/// the window, and shifted so the whole tip stays inside `bounds`. The tip is
/// exactly as wide as the value, so a long amount is never wrapped.
fn hover_tip_rect(token: Rect, text_size: Vec2, bounds: Rect) -> Rect {
    let pad = vec2(12.0, 8.0);
    let size = text_size + pad * 2.0;
    let gap = 6.0;
    let mut y = token.top() - size.y - gap;
    if y < bounds.top() {
        y = token.bottom() + gap;
    }
    if y + size.y > bounds.bottom() {
        y = (bounds.bottom() - size.y).max(bounds.top());
    }
    let mut x = token.left();
    if x + size.x > bounds.right() {
        x = bounds.right() - size.x;
    }
    if x < bounds.left() {
        x = bounds.left();
    }
    Rect::from_min_size(pos2(x, y), size)
}

fn paint_hover_tip(ctx: &egui::Context, theme: &Theme, token: Rect, value: &str) {
    let galley = ctx.fonts_mut(|fonts| {
        fonts.layout_no_wrap(value.to_string(), FontId::monospace(14.0), theme.accent)
    });
    let bounds = ctx.content_rect().shrink(8.0);
    let tip = hover_tip_rect(token, galley.size(), bounds);
    let painter = ctx.layer_painter(egui::LayerId::new(
        Order::Tooltip,
        egui::Id::unique("omacus-hover"),
    ));
    painter.rect_filled(
        tip.translate(vec2(0.0, 2.0)),
        CornerRadius::same(8),
        Color32::from_black_alpha(if theme.dark { 90 } else { 32 }),
    );
    painter.rect_filled(tip, CornerRadius::same(8), theme.sidebar);
    painter.rect_stroke(
        tip,
        CornerRadius::same(8),
        egui::Stroke::new(1.0, theme.selection),
        egui::StrokeKind::Inside,
    );
    painter.galley(tip.min + vec2(12.0, 8.0), galley, theme.accent);
}

/// The switcher opens Settings when the query is the start of that word.
/// A single letter would steal every page whose name starts with "s".
fn settings_command(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.len() >= 2 && "settings".starts_with(&q)
}

/// The sidebar's button column: centered on the rows' ⋯, `slot` buttons in from the right.
fn icon_slot(row: Rect, slot: usize) -> Rect {
    let x = row.right() - 15.0 - slot as f32 * (ICON_BOX + 4.0);
    Rect::from_center_size(pos2(x, row.center().y), Vec2::splat(ICON_BOX))
}

/// "PAGES", "RECENTLY DELETED": lined up with the page names below.
fn section_label(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    ui.painter().text(
        pos2(row.left() + 14.0, row.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(11.0),
        theme.comment,
    );
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
static BOLD_FONT: &[u8] = font!("JetBrainsMono-Bold.ttf");
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
    add("bold", inflate(BOLD_FONT));
    add("currency", inflate(CURRENCY_FONT));
    add("emoji", inflate(EMOJI_FONT));
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        fonts.families.insert(family, chain.clone());
    }
    let mut bold_chain = vec!["bold".to_string()];
    bold_chain.extend(chain.iter().cloned());
    fonts
        .families
        .insert(FontFamily::Name("bold".into()), bold_chain);
    ctx.set_fonts(fonts);
}

/// Width a colour swatch and its gap take beside a result.
/// Whether an answer `width` wide no longer fits after a row ending at `row_end`.
fn answer_drops(row_end: f32, width: f32, wrap_width: f32) -> bool {
    row_end + GAP + width > wrap_width
}

/// Where an answer is painted: the right-hand column, or just after the line.
enum AnswerAnchor {
    Right(f32),
    Left(f32),
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
        #[cfg(windows)]
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Omacus",
        options,
        Box::new(|cc| Ok(Box::new(OmacusApp::new(cc)))),
    )
}

/// Ctrl/Cmd+J toggles the switcher on the key press only. Ctrl/Cmd+Alt+Up/Down
/// walks sheets and may repeat. Escape dismisses; it does not quit. Ctrl+P
/// opens the rendered preview.
fn shortcut_for(command: bool, alt: bool, shift: bool, key: Key, repeat: bool) -> Option<Shortcut> {
    match key {
        Key::Escape if !command && !alt && !shift && !repeat => Some(Shortcut::Dismiss),
        Key::J if command && !alt && !shift && !repeat => Some(Shortcut::ToggleSwitcher),
        Key::ArrowUp if command && alt && !shift => Some(Shortcut::PrevSheet),
        Key::ArrowDown if command && alt && !shift => Some(Shortcut::NextSheet),
        Key::Q | Key::W if command && !alt && !shift && !repeat => Some(Shortcut::Quit),
        // Ctrl+, opens settings. Ctrl+\ hides the answers column. Ctrl+P previews.
        Key::Comma if command && !alt && !shift && !repeat => Some(Shortcut::Settings),
        Key::Backslash if command && !alt && !shift && !repeat => Some(Shortcut::ToggleAnswers),
        Key::P if command && !alt && !shift && !repeat => Some(Shortcut::Preview),
        _ => None,
    }
}

/// True when this press belongs to one of those shortcuts, repeats included,
/// so the editor never sees the key or the letter it would insert.
fn owns_key(command: bool, alt: bool, shift: bool, key: Key) -> bool {
    match key {
        Key::J if command && !alt && !shift => true,
        Key::Escape if !command && !alt && !shift => true,
        Key::ArrowUp | Key::ArrowDown if command && alt && !shift => true,
        Key::Q | Key::W if command && !alt && !shift => true,
        Key::Comma | Key::Backslash | Key::P if command && !alt && !shift => true,
        _ => false,
    }
}

fn take_chords(events: &mut Vec<egui::Event>) -> Vec<Shortcut> {
    let mut swallow_text = false;
    let mut actions = Vec::new();
    for event in events.iter() {
        if let egui::Event::Key {
            key,
            modifiers,
            pressed: true,
            repeat,
            ..
        } = event
            && owns_key(modifiers.command, modifiers.alt, modifiers.shift, *key)
        {
            swallow_text = true;
            if let Some(shortcut) = shortcut_for(
                modifiers.command,
                modifiers.alt,
                modifiers.shift,
                *key,
                *repeat,
            ) {
                actions.push(shortcut);
            }
        }
    }
    events.retain(|event| match event {
        egui::Event::Key {
            key,
            modifiers,
            pressed: true,
            ..
        } if owns_key(modifiers.command, modifiers.alt, modifiers.shift, *key) => false,
        egui::Event::Text(_) if swallow_text => false,
        _ => true,
    });
    actions
}

/// One switcher keystroke after it has been taken out of the event list.
/// A key release is never one of these: only a press, text, or paste is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SwitcherEdit {
    Type(String),
    Backspace,
    Up,
    Down,
    Confirm,
    Close,
}

enum SwitcherOutcome {
    Stay,
    Move(isize),
    Confirm,
    Close,
}

/// The switcher must not take keyboard focus. `request_focus` interrupts IME,
/// which on Wayland drops the release of whatever key is held.
fn switcher_takes_focus() -> bool {
    false
}

/// What the sheet editor captures. Escape is included only while the switcher
/// is open, so egui does not clear focus (and move the IME cursor) on Esc.
fn sheet_event_filter(switcher_open: bool) -> egui::EventFilter {
    egui::EventFilter {
        tab: true,
        horizontal_arrows: true,
        vertical_arrows: true,
        escape: switcher_open,
    }
}

fn switcher_insertable(text: &str) -> bool {
    !text.is_empty() && text != "\n" && text != "\r"
}

/// Remove switcher input from `events` so the focused sheet cannot apply it too.
/// Releases are removed and produce no edit. An IME commit is used only when
/// the same batch has no plain text, so a character delivered both ways is typed once.
fn take_switcher_edits(events: &mut Vec<egui::Event>) -> Vec<SwitcherEdit> {
    let saw_text = events.iter().any(|event| match event {
        egui::Event::Text(text) => switcher_insertable(text),
        egui::Event::Paste(text) => !text.is_empty(),
        _ => false,
    });
    let mut edits = Vec::new();
    events.retain(|event| match event {
        egui::Event::Text(text) => {
            if switcher_insertable(text) {
                edits.push(SwitcherEdit::Type(text.clone()));
            }
            false
        }
        egui::Event::Paste(text) => {
            let flat = text.replace(['\r', '\n'], " ");
            if !flat.is_empty() {
                edits.push(SwitcherEdit::Type(flat));
            }
            false
        }
        egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
            if !saw_text && switcher_insertable(text) {
                edits.push(SwitcherEdit::Type(text.clone()));
            }
            false
        }
        egui::Event::Ime(_) | egui::Event::Copy | egui::Event::Cut | egui::Event::PasteImage(_) => {
            false
        }
        egui::Event::MiddleClickPaste { .. } => false,
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            match key {
                Key::Backspace if !modifiers.command && !modifiers.alt && !modifiers.mac_cmd => {
                    edits.push(SwitcherEdit::Backspace);
                }
                Key::ArrowUp if modifiers.is_none() => edits.push(SwitcherEdit::Up),
                Key::ArrowDown if modifiers.is_none() => edits.push(SwitcherEdit::Down),
                Key::Enter if !modifiers.command && !modifiers.alt => {
                    edits.push(SwitcherEdit::Confirm);
                }
                Key::Escape if modifiers.is_none() => edits.push(SwitcherEdit::Close),
                _ => {}
            }
            false
        }
        egui::Event::Key { pressed: false, .. } => false,
        _ => true,
    });
    edits
}

/// Apply a typing edit to the switcher query. Arrows, Enter, and Esc do not
/// change the text; the caller handles those once it knows the hit list.
fn apply_switcher_query(query: &mut String, selected: &mut usize, edit: &SwitcherEdit) {
    match edit {
        SwitcherEdit::Type(text) => {
            query.push_str(text);
            *selected = 0;
        }
        SwitcherEdit::Backspace => {
            query.pop();
            *selected = 0;
        }
        SwitcherEdit::Up | SwitcherEdit::Down | SwitcherEdit::Confirm | SwitcherEdit::Close => {}
    }
}

fn name_hint_visible(editing_line: bool, idle_for: Duration) -> bool {
    !editing_line || idle_for >= HINT_IDLE
}

fn empty_sheet_hint(sidebar_open: bool) -> &'static str {
    if sidebar_open {
        "Pages are listed on the left. Ctrl+B collapses them, and Ctrl+J jumps to one."
    } else {
        "Pages are in the sidebar. Ctrl+B expands it, or click the strip on the left. Ctrl+J jumps to one."
    }
}

/// One line, cut with … once it runs out of room. A wrapping galley is what made
/// a long name spill into the next row.
fn sidebar_label_job(
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping {
        max_width,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    job
}

/// The orange mark on the open sheet, centered on the row.
fn sidebar_accent(row: Rect) -> Rect {
    Rect::from_center_size(pos2(row.left() + 1.5, row.center().y), vec2(3.0, 16.0))
}

enum TrashChoice {
    Restore,
    Forever,
}

fn sidebar_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    heading: &str,
    selected: bool,
    accent: bool,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), SIDEBAR_ROW), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(5), theme.selection);
        if accent {
            painter.rect_filled(sidebar_accent(rect), CornerRadius::same(2), theme.accent);
        }
    } else if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(5), theme.hover);
    }
    let color = if selected {
        theme.foreground
    } else {
        theme.dim
    };
    let text_width = (rect.width() - 14.0 - 30.0).max(24.0);
    let galley = ui.ctx().fonts_mut(|f| {
        f.layout_job(sidebar_label_job(
            heading,
            FontId::proportional(14.0),
            color,
            text_width,
        ))
    });
    let elided = galley.elided;
    painter.galley(
        pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if elided {
        resp.clone().on_hover_text(heading.to_owned());
    }
    resp
}

fn sidebar_dots(ui: &mut egui::Ui, row: &Rect, id: egui::Id) -> egui::Response {
    let dots_rect =
        Rect::from_center_size(pos2(row.right() - 15.0, row.center().y), vec2(24.0, 24.0));
    ui.interact(dots_rect, id, Sense::click())
}

fn trash_menu(ui: &mut egui::Ui, action: &mut Option<TrashChoice>) {
    if ui.add(egui::Button::new("Restore")).clicked() {
        *action = Some(TrashChoice::Restore);
    }
    if ui.add(egui::Button::new("Delete forever")).clicked() {
        *action = Some(TrashChoice::Forever);
    }
}

fn line_at(text: &str, cursor: usize) -> usize {
    text.chars().take(cursor).filter(|c| *c == '\n').count()
}

#[cfg(test)]
mod tests {
    use super::find_sheets;
    use super::{
        BAR_ROW, HINT_IDLE, SIDEBAR_ROW, Shortcut, SwitcherEdit, apply_switcher_query,
        empty_sheet_hint, hover_tip_rect, icon_slot, line_at, name_hint_visible, owns_key,
        settings_command, sheet_event_filter, shortcut_for, sidebar_accent, sidebar_label_job,
        switcher_takes_focus, take_chords, take_switcher_edits,
    };
    use eframe::egui::{Color32, Event, FontId, ImeEvent, Key, Modifiers, Rect, pos2, vec2};
    use std::time::Duration;

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

    fn press(key: Key, modifiers: Modifiers, repeat: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers,
        }
    }

    fn release(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn switcher_moves_to_ctrl_j_and_ignores_repeats() {
        assert_eq!(
            shortcut_for(true, false, false, Key::J, false),
            Some(Shortcut::ToggleSwitcher)
        );
        assert_eq!(shortcut_for(true, false, false, Key::J, true), None);
        assert!(owns_key(true, false, false, Key::J));
        assert_eq!(
            shortcut_for(true, false, false, Key::P, false),
            Some(Shortcut::Preview)
        );
        assert!(owns_key(true, false, false, Key::P));
        assert_eq!(shortcut_for(true, false, false, Key::P, true), None);
        assert!(!owns_key(false, false, false, Key::P));
        assert_eq!(
            shortcut_for(true, true, false, Key::ArrowDown, true),
            Some(Shortcut::NextSheet)
        );
        assert_eq!(
            shortcut_for(true, true, false, Key::ArrowUp, false),
            Some(Shortcut::PrevSheet)
        );
        // Alt+Up still moves a sheet; it is not the previous-sheet chord.
        assert_eq!(shortcut_for(false, true, false, Key::ArrowUp, false), None);
        assert_eq!(
            shortcut_for(false, false, false, Key::Escape, false),
            Some(Shortcut::Dismiss)
        );
        assert_eq!(shortcut_for(false, false, false, Key::Escape, true), None);
        assert_eq!(
            shortcut_for(true, false, false, Key::Q, false),
            Some(Shortcut::Quit)
        );
        assert_eq!(shortcut_for(true, false, false, Key::W, true), None);

        let mut held = vec![
            press(Key::J, Modifiers::COMMAND, true),
            Event::Text("j".into()),
        ];
        assert!(take_chords(&mut held).is_empty());
        assert!(held.is_empty());

        let mut once = vec![
            press(Key::J, Modifiers::COMMAND, false),
            Event::Text("j".into()),
        ];
        assert_eq!(take_chords(&mut once), vec![Shortcut::ToggleSwitcher]);
        assert!(once.is_empty());

        let mut preview = vec![
            press(Key::P, Modifiers::COMMAND, false),
            Event::Text("p".into()),
        ];
        assert_eq!(take_chords(&mut preview), vec![Shortcut::Preview]);
        assert!(preview.is_empty());
    }

    #[test]
    fn settings_and_the_answers_column_have_shortcuts() {
        assert_eq!(
            shortcut_for(true, false, false, Key::Comma, false),
            Some(Shortcut::Settings)
        );
        // A held comma must not insert a comma, and a repeat must not reopen settings.
        assert_eq!(shortcut_for(true, false, false, Key::Comma, true), None);
        assert!(owns_key(true, false, false, Key::Comma));
        assert!(!owns_key(false, false, false, Key::Comma));
        assert_eq!(
            shortcut_for(true, false, false, Key::Backslash, false),
            Some(Shortcut::ToggleAnswers)
        );
        assert_eq!(shortcut_for(true, false, false, Key::Backslash, true), None);
        assert!(owns_key(true, false, false, Key::Backslash));
        // A plain backslash is still a character.
        assert!(!owns_key(false, false, false, Key::Backslash));

        let mut settings = vec![
            press(Key::Comma, Modifiers::COMMAND, false),
            Event::Text(",".into()),
        ];
        assert_eq!(take_chords(&mut settings), vec![Shortcut::Settings]);
        assert!(settings.is_empty());

        let mut answers = vec![
            press(Key::Backslash, Modifiers::COMMAND, false),
            Event::Text("\\".into()),
        ];
        assert_eq!(take_chords(&mut answers), vec![Shortcut::ToggleAnswers]);
        assert!(answers.is_empty());
    }

    #[test]
    fn switcher_types_from_text_events_and_drops_the_release() {
        assert!(!switcher_takes_focus());
        let mut events = vec![
            Event::Text("j".into()),
            press(Key::J, Modifiers::NONE, false),
            press(Key::J, Modifiers::NONE, true),
            Event::Text("j".into()),
            release(Key::J),
            Event::PointerMoved(eframe::egui::pos2(1.0, 2.0)),
        ];
        let edits = take_switcher_edits(&mut events);
        assert_eq!(
            edits,
            vec![
                SwitcherEdit::Type("j".into()),
                SwitcherEdit::Type("j".into()),
            ]
        );
        assert_eq!(events.len(), 1, "pointer events stay; keys do not");
        let mut query = String::new();
        let mut selected = 4;
        for edit in &edits {
            apply_switcher_query(&mut query, &mut selected, edit);
        }
        assert_eq!(query, "jj");
        assert_eq!(selected, 0);
    }

    #[test]
    fn switcher_backspace_ignores_the_key_release() {
        let mut events = vec![
            press(Key::Backspace, Modifiers::NONE, false),
            release(Key::Backspace),
            press(Key::Backspace, Modifiers::NONE, true),
        ];
        let edits = take_switcher_edits(&mut events);
        assert!(events.is_empty());
        assert_eq!(
            edits,
            vec![SwitcherEdit::Backspace, SwitcherEdit::Backspace]
        );
        let mut query = "caf\u{e9}".to_string();
        let mut selected = 2;
        for edit in &edits {
            apply_switcher_query(&mut query, &mut selected, edit);
        }
        assert_eq!(query, "ca");
        assert_eq!(selected, 0);

        let mut arrows = vec![
            press(Key::ArrowDown, Modifiers::NONE, true),
            release(Key::ArrowDown),
            press(Key::ArrowUp, Modifiers::NONE, false),
            release(Key::ArrowUp),
            press(Key::Enter, Modifiers::NONE, false),
            release(Key::Enter),
            press(Key::Escape, Modifiers::NONE, false),
            release(Key::Escape),
        ];
        assert_eq!(
            take_switcher_edits(&mut arrows),
            vec![
                SwitcherEdit::Down,
                SwitcherEdit::Up,
                SwitcherEdit::Confirm,
                SwitcherEdit::Close,
            ]
        );
        assert!(arrows.is_empty());
    }

    #[test]
    fn switcher_does_not_type_a_character_twice_from_ime() {
        let mut both = vec![
            Event::Text("a".into()),
            Event::Ime(ImeEvent::Commit("a".into())),
            Event::Ime(ImeEvent::Preedit {
                text: String::new(),
                active_range_chars: None,
            }),
        ];
        assert_eq!(
            take_switcher_edits(&mut both),
            vec![SwitcherEdit::Type("a".into())]
        );
        assert!(both.is_empty());

        let mut composed = vec![Event::Ime(ImeEvent::Commit("\u{3042}".into()))];
        assert_eq!(
            take_switcher_edits(&mut composed),
            vec![SwitcherEdit::Type("\u{3042}".into())]
        );
    }

    #[test]
    fn sheet_keeps_escape_while_the_switcher_is_open() {
        let open = sheet_event_filter(true);
        let closed = sheet_event_filter(false);
        let escape = press(Key::Escape, Modifiers::NONE, false);
        assert!(open.matches(&escape));
        assert!(open.matches(&press(Key::ArrowUp, Modifiers::NONE, false)));
        assert!(open.matches(&press(Key::ArrowDown, Modifiers::NONE, false)));
        assert!(open.matches(&press(Key::ArrowLeft, Modifiers::NONE, false)));
        assert!(open.matches(&press(Key::Tab, Modifiers::NONE, false)));
        assert!(!closed.matches(&escape));
        assert!(closed.matches(&press(Key::ArrowDown, Modifiers::NONE, true)));
        assert!(closed.tab && closed.horizontal_arrows && closed.vertical_arrows);
    }

    #[test]
    fn name_hints_wait_out_typing_on_that_line() {
        assert!(!name_hint_visible(true, Duration::from_millis(100)));
        assert!(name_hint_visible(true, HINT_IDLE));
        assert!(name_hint_visible(false, Duration::ZERO));
    }

    #[test]
    fn a_hover_tip_is_as_wide_as_its_value_and_stays_in_the_window() {
        let bounds = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
        let text = vec2(148.0, 16.0);
        let token = Rect::from_min_size(pos2(100.0, 200.0), vec2(40.0, 16.0));
        let tip = hover_tip_rect(token, text, bounds);
        assert!((tip.width() - (text.x + 24.0)).abs() < 0.1);
        assert!(
            tip.width() > tip.height() * 2.0,
            "a value must not stack into a tall box"
        );
        assert!(tip.bottom() <= token.top());
        assert!(tip.left() >= bounds.left() && tip.right() <= bounds.right());
        assert!(tip.top() >= bounds.top());

        let token = Rect::from_min_size(pos2(340.0, 200.0), vec2(30.0, 16.0));
        let tip = hover_tip_rect(token, text, bounds);
        assert!((tip.width() - (text.x + 24.0)).abs() < 0.1);
        assert!(tip.right() <= bounds.right() + 0.1);
        assert!(tip.left() >= bounds.left());

        let token = Rect::from_min_size(pos2(20.0, 4.0), vec2(30.0, 16.0));
        let tip = hover_tip_rect(token, text, bounds);
        assert!(tip.top() >= token.bottom());
        assert!(tip.bottom() <= bounds.bottom());
        assert!((tip.width() - (text.x + 24.0)).abs() < 0.1);
    }

    #[test]
    fn settings_is_a_switcher_command() {
        assert!(!settings_command(""));
        assert!(!settings_command("s"));
        assert!(!settings_command("  s "));
        assert!(settings_command("se"));
        assert!(settings_command("Set"));
        assert!(settings_command("settings"));
        assert!(!settings_command("settingx"));
        assert!(!settings_command("page"));
    }

    #[test]
    fn sidebar_buttons_line_up_with_the_rows_dots() {
        let row = Rect::from_min_size(pos2(10.0, 8.0), vec2(190.0, BAR_ROW));
        let sheet = Rect::from_min_size(pos2(10.0, 40.0), vec2(190.0, SIDEBAR_ROW));
        let dots = Rect::from_center_size(
            pos2(sheet.right() - 15.0, sheet.center().y),
            vec2(24.0, 24.0),
        );
        assert_eq!(icon_slot(row, 0).center().x, dots.center().x);
        assert_eq!(icon_slot(row, 0).center().y, row.center().y);
        assert!(icon_slot(row, 1).right() < icon_slot(row, 0).left());
        // Level with ⋯ above the sheet (8 down, 28 tall, in both panels).
        assert_eq!(row.center().y, 8.0 + 28.0 / 2.0);
    }

    #[test]
    fn empty_sheet_mentions_the_sidebar() {
        assert!(empty_sheet_hint(true).contains("Ctrl+B"));
        assert!(empty_sheet_hint(true).contains("Ctrl+J"));
        assert!(empty_sheet_hint(false).contains("strip on the left"));
        assert!(!empty_sheet_hint(false).to_lowercase().contains("sheet"));
        assert!(!empty_sheet_hint(true).to_lowercase().contains("sheet"));
        assert_eq!(line_at("a\nb\nc", 0), 0);
        assert_eq!(line_at("a\nb\nc", 2), 1);
    }

    #[test]
    fn a_long_sheet_name_stays_on_one_row() {
        let row = Rect::from_min_size(pos2(8.0, 40.0), vec2(180.0, SIDEBAR_ROW));
        let bar = sidebar_accent(row);
        assert!((bar.center().y - row.center().y).abs() < 0.01);
        assert!((bar.height() - 16.0).abs() < 0.01);
        assert!(bar.top() >= row.top() && bar.bottom() <= row.bottom());
        let job = sidebar_label_job(
            "Duo Offer Discussion",
            FontId::proportional(14.0),
            Color32::WHITE,
            80.0,
        );
        assert_eq!(job.wrap.max_rows, 1);
        assert!(job.wrap.break_anywhere);
        assert_eq!(job.wrap.overflow_character, Some('\u{2026}'));
        assert_eq!(job.text, "Duo Offer Discussion");
    }
}
