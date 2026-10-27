//! The calculator window: a sidebar of sheets, the sheet text, and live results on the right.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Key,
    Margin, Modifiers, Order, Pos2, Rect, Sense, TextEdit, Vec2, ViewportCommand, pos2, vec2,
};

use crate::calc::Calculator;
use crate::design;
use crate::help;
use crate::highlight;
use crate::pdf;
use crate::preview;
use crate::rates::Rates;
use crate::settings::{self, Settings};
use crate::sheets::{Place, SIDEBAR_MAX, SIDEBAR_MIN, SideKind, Store, Trashed, heading_of};
use crate::sync;
use crate::theme::{self, Theme};
use crate::update::{self, Method, State, Updates};
use omacus_engine::{self as engine, Engine, Kind, LineResult, complete};

const FONT_SIZE: f32 = 16.0;
const GAP: f32 = theme::token::SPACE_6;
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
// The Examples folder has a few short pages. Monthly budget reads @trip.total.

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

/// F2 or Rename variable: an inline field for the new name.
#[derive(Clone)]
struct RenameSession {
    /// Char index that `binding_at` was opened on.
    cursor: usize,
    token_start: usize,
    buffer: String,
    error: Option<String>,
    focus: bool,
}

/// The definition's name was edited in the page. Offer to update the uses.
#[derive(Clone)]
struct RefPrompt {
    spans: Vec<(usize, usize)>,
    old_name: String,
    new_name: String,
    label: String,
    /// False when the message is a collision, which has no button.
    apply: bool,
    collision: bool,
}

/// One undo restores every page a rename rewrote.
#[derive(Clone)]
struct CrossUndo {
    id: String,
    before_text: String,
    before_handle: String,
    after_text: String,
    /// id, text, handle before the rename.
    others: Vec<(String, String, String)>,
    armed: bool,
}

/// The Handle… field for the page's short name.
struct HandleEdit {
    index: usize,
    buffer: String,
    error: Option<String>,
    focus: bool,
}

fn ref_prompt_from(edit: engine::rename::DefinitionEdit) -> RefPrompt {
    let label = edit.message();
    let apply = !edit.collision && edit.references > 0;
    RefPrompt {
        spans: edit
            .spans
            .iter()
            .map(|span| (span.start, span.end))
            .collect(),
        old_name: edit.old_name,
        new_name: edit.new_name,
        label,
        apply,
        collision: edit.collision,
    }
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
    /// Rename the variable under the caret. F2.
    Rename,
    /// Rendered markdown, with each answer in brackets. Ctrl+P.
    Preview,
    /// Edit the page's `@handle` name.
    Handle(usize),
    /// A folder at the root, or inside `Some` folder when that isn't already nested.
    NewFolderAt(Option<String>),
    /// A page at the top of a folder. `None` is the root.
    NewIn(Option<String>),
    MoveTo(usize),
    /// Ask before a page joins My pages, or leaves them on this computer.
    SyncSet(usize, bool),
    ToggleFolder(String),
    RenameFolder(String),
    DeleteFolder(String),
    LockPage(usize),
    LockFolder(String),
    UnlockFolder(String),
    ChangePage(usize),
    ChangeFolder(String),
    RemovePage(usize),
    RemoveFolder(String),
    LockNow,
}

/// Password dialog. Unlocking the page on screen uses the page itself, not this.
#[derive(Clone)]
enum LockAsk {
    CreatePage(String),
    CreateFolder(String),
    UnlockFolder(String),
    ChangePage(String),
    ChangeFolder(String),
    RemovePage(String),
    RemoveFolder(String),
}

/// What the page menu needs to know about a lock. `lock` is `Some(true)` when closed.
#[derive(Clone, Copy)]
struct PageMenu {
    covered: bool,
    lock: Option<bool>,
}

/// What a sidebar drag is carrying.
#[derive(Clone, Debug)]
struct DragPayload {
    id: String,
    folder: bool,
}

/// Where a drag may land, filled in while the sidebar rows are drawn.
struct DropTarget<'a> {
    folder: bool,
    collapsed: bool,
    dropped: &'a mut Option<(String, Place)>,
    expand_id: &'a mut Option<String>,
}

/// A folder whose name is being typed in a real text field.
#[derive(Clone)]
struct FolderRename {
    id: String,
    text: String,
    /// Select the whole name on the next frame, so typing replaces it.
    select_all: bool,
}

#[derive(Clone)]
enum PendingDelete {
    Page(usize),
    Folder(String),
}

/// A message at the bottom of the window.
struct Toast {
    text: String,
    at: Instant,
    /// Offers Undo for a deleted sheet, or for a page moved between sets.
    undo: bool,
    /// Undo puts a page back in the set it just left.
    sync_move: bool,
    error: bool,
    /// Stays until replaced ("Working out the answers to copy…").
    working: bool,
    /// "Open folder" after an export.
    open_folder: Option<std::path::PathBuf>,
}

/// An export waiting for Replace / Cancel.
struct PendingExport {
    path: std::path::PathBuf,
    folder_label: String,
    file_name: String,
    title: String,
    blocks: Vec<omacus_engine::markdown::Block>,
    column: bool,
}

impl Toast {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            at: Instant::now(),
            undo: false,
            sync_move: false,
            error: false,
            working: false,
            open_folder: None,
        }
    }
    fn saved(text: impl Into<String>, dir: std::path::PathBuf) -> Self {
        Self {
            open_folder: Some(dir),
            ..Self::info(text)
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
    fn moved(text: String) -> Self {
        Self {
            undo: true,
            sync_move: true,
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
        } else if self.undo || self.open_folder.is_some() {
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
    NewFolder,
    MoveToFolder,
    /// F2: rename the variable under the caret.
    Rename,
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
    /// Export that already has a file at the destination, waiting for Replace.
    pending_export: Option<PendingExport>,
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
    /// A page or folder waiting for a delete confirmation. Escape cancels.
    pending_delete: Option<PendingDelete>,
    /// Page id whose Move-to-folder list is open.
    move_page: Option<String>,
    /// Folder rename field. The sheet keeps focus; keystrokes land here.
    rename_folder: Option<FolderRename>,
    /// Collapsed folder id a drag has been hovering, and since when.
    drag_expand: Option<(String, Instant)>,
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
    /// Inline rename, opened with F2 or Rename variable.
    rename: Option<RenameSession>,
    /// Enter was pressed in the rename field this frame.
    rename_commit: bool,
    /// Enter was pressed in the folder-name field this frame.
    rename_folder_commit: bool,
    /// Open a rename on this char index once the editor has been shown.
    rename_at: Option<usize>,
    /// Char index the context menu was opened on.
    rename_menu_at: Option<usize>,
    /// "Update N references?" after a definition's name was edited.
    ref_prompt: Option<RefPrompt>,
    /// Pages rewritten together, restored by one undo.
    cross_undo: Option<CrossUndo>,
    /// Handle… dialog.
    handle_edit: Option<HandleEdit>,
    /// Enter was pressed in the handle field this frame.
    handle_commit: bool,
    /// Open this page after the editor has finished drawing.
    pending_open: Option<String>,
    /// The page before the definition rename started, so typing the new name
    /// one letter at a time still finds the original references.
    ref_base: Option<String>,
    /// Esc or Dismiss hides the prompt until the edit is no longer that rename.
    ref_dismissed: bool,
    /// The page editor, so a rename can be one undo step.
    editor_id: Option<egui::Id>,
    /// Screenshot hook: open the rename field on the sample page.
    demo_rename: bool,
    /// Cursor to apply to the editor after it is next shown.
    pending_cursor: Option<usize>,
    /// Sheets whose saved cursor has been put back this session.
    restored: HashSet<String>,
    focus_editor: bool,
    title: String,
    help: Option<help::Tab>,
    updates: Updates,
    update_open: bool,
    list_open: bool,
    /// Sync in this window. Pages stay files. A sealed copy is what leaves.
    sync: sync::Session,
    sync_poll: Instant,
    /// Page id and the set it should return to if Undo is tapped.
    sync_undo: Option<(String, bool)>,
    /// The app icon, small (sidebar, help rail) and large (About).
    logo: help::Logo,
    /// Developer option for the website: `OMACUS_SCREENSHOT=out.png` saves the window's
    /// own pixels (nothing behind it) and quits.
    screenshot: Option<(std::path::PathBuf, u32)>,
    lock_ask: Option<LockAsk>,
    lock_password: String,
    lock_confirm: String,
    lock_current: String,
    lock_error: String,
    /// The password field takes focus on the frame the dialog opens.
    lock_focus: bool,
    /// Folder to add a page to once its password is entered.
    pending_new: Option<String>,
    /// Password typed into the locked-page prompt.
    gate_password: String,
    gate_error: String,
    /// Last key or pointer, for the idle lock.
    last_input: Instant,
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
        let theme = if std::env::var("OMACUS_SYNC_SCREEN").is_ok() {
            let light = std::env::var("OMACUS_SCREENSHOT_THEME").ok().as_deref() == Some("light");
            Theme::resolve(
                if light {
                    settings::ThemeChoice::Light
                } else {
                    settings::ThemeChoice::Dark
                },
                !light,
            )
        } else {
            Theme::resolve(settings.theme, system_dark)
        };
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
            pending_export: std::env::var("OMACUS_SCREENSHOT_HELP")
                .is_ok_and(|t| t == "replace")
                .then(|| PendingExport {
                    path: pdf::default_export_dir().join("Trip Budget.pdf"),
                    folder_label: "Downloads".into(),
                    file_name: "Trip Budget.pdf".into(),
                    title: "Trip Budget".into(),
                    blocks: Vec::new(),
                    column: true,
                }),
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
            rename: None,
            rename_commit: false,
            rename_folder_commit: false,
            rename_at: None,
            rename_menu_at: None,
            ref_prompt: None,
            cross_undo: None,
            handle_edit: None,
            handle_commit: false,
            pending_open: None,
            ref_base: None,
            ref_dismissed: false,
            editor_id: None,
            demo_rename: false,
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
            list_open: false,
            sync: sync::Session::open(sync_dir(), &sync_origin()),
            sync_poll: Instant::now(),
            sync_undo: None,
            switcher: std::env::var("OMACUS_SCREENSHOT_HELP")
                .is_ok_and(|t| t == "switcher")
                .then(|| Switcher {
                    query: "k".into(),
                    selected: 0,
                }),
            pending_delete: None,
            move_page: None,
            rename_folder: None,
            drag_expand: None,
            preview: None,
            purge_id: None,
            trash: Vec::new(),
            trash_at: None,
            edited_at: Instant::now()
                .checked_sub(HINT_IDLE)
                .unwrap_or_else(Instant::now),
            cursor_line: None,
            help: match std::env::var("OMACUS_SCREENSHOT_HELP").ok().as_deref() {
                Some(
                    "update" | "switcher" | "preview" | "replace" | "rename" | "renamed" | "picker"
                    | "hover" | "pick",
                )
                | None => None,
                Some("settings") => Some(help::Tab::Settings),
                Some(name) => Some(help::Tab::from_name(name)),
            },
            screenshot: std::env::var_os("OMACUS_SCREENSHOT").map(|p| (p.into(), 0)),
            lock_ask: None,
            lock_password: String::new(),
            lock_confirm: String::new(),
            lock_current: String::new(),
            lock_error: String::new(),
            lock_focus: false,
            pending_new: None,
            gate_password: String::new(),
            gate_error: String::new(),
            last_input: Instant::now(),
        };
        if let Ok(kind) = std::env::var("OMACUS_SYNC_SCREEN") {
            seed_sync_screen(&mut app.store, &kind);
            app.sync.adopt(local_pages(&app.store));
            app.sync.preview(&kind);
            if kind == "waiting"
                && let Some(sheet) = app.store.sheets.iter().find(|s| s.heading() == "Scratch")
            {
                let id = sheet.id.clone();
                app.sync.set_in_set(&id, false);
            }
            if kind == "list-sheet" {
                app.list_open = true;
            }
        }
        if let Ok(mode) = std::env::var("OMACUS_SCREENSHOT_HELP")
            && (mode == "rename" || mode == "renamed")
        {
            let sample = "\
# Northwind

rent = 1,200
rent * 2
rental = rent + 50
# the rent is due
100 TRY
";
            let at = sample
                .match_indices("rent")
                .nth(1)
                .map(|(i, _)| i)
                .unwrap_or(0);
            app.store.current().text = if mode == "renamed" {
                engine::rename::rename_variable(sample, at, "fee")
                    .map(|renamed| renamed.text)
                    .unwrap_or_else(|_| sample.to_string())
            } else {
                sample.to_string()
            };
            app.store.current().mark_dirty();
            app.demo_rename = mode == "rename";
        }
        if std::env::var("OMACUS_SCREENSHOT_HELP").ok().as_deref() == Some("pick") {
            app.store.demo_line_conflict();
        }
        app.evaluate();
        app
    }

    /// Every page a `@handle.name` can read, including Recently deleted.
    /// `for_rename` marks a page another window has written since we read it.
    fn page_catalog(&self, for_rename: bool) -> Vec<engine::xref::Page> {
        let active = self.store.sheets[self.store.active].id.clone();
        let mut seen = HashSet::new();
        let mut pages = Vec::new();
        for sheet in &self.store.sheets {
            seen.insert(sheet.id.clone());
            let elsewhere = for_rename && sheet.id != active && self.store.opened_elsewhere(sheet);
            pages.push(self.store.as_reference(sheet, false, elsewhere));
        }
        for sheet in self.store.pending_sheets() {
            if !seen.insert(sheet.id.clone()) {
                continue;
            }
            pages.push(self.store.as_reference(sheet, true, false));
        }
        for item in &self.trash {
            if !seen.insert(item.id.clone()) {
                continue;
            }
            pages.push(engine::xref::Page {
                id: item.id.clone(),
                handle: item.handle.clone(),
                title: item.title.clone(),
                text: item.text.clone(),
                locked: item.locked,
                deleted: true,
                elsewhere: false,
            });
        }
        pages
    }

    /// Ask for fresh answers; they arrive in `take_answers`.
    fn evaluate(&mut self) {
        self.refresh_trash(false);
        let pages = self.page_catalog(false);
        let sheet = self.store.current();
        self.calc
            .request_with(&sheet.id, &sheet.handle, &sheet.text, &pages);
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
        let id = self.store.sheets[i].id.clone();
        let handle = self.store.sheets[i].handle.clone();
        let text = self.store.sheets[i].text.clone();
        if i == self.store.active && self.results_exact && self.results_text == text {
            self.copy = None;
            self.ctx.copy_text(format(&text, &self.results));
            self.toast = Some(Toast::info(done));
            return;
        }
        let pages = self.page_catalog(false);
        self.copier.request_with(&id, &handle, &text, &pages);
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
            SheetAction::New => {
                if !self.try_new_page(None) {
                    return;
                }
            }
            SheetAction::Delete(i) => {
                if i < self.store.sheets.len() {
                    self.pending_delete = Some(PendingDelete::Page(i));
                    self.switcher = None;
                    self.popup = None;
                }
                return;
            }
            SheetAction::NewFolderAt(parent) => {
                if let Some(id) = self.store.create_folder(parent.as_deref()) {
                    let text = self
                        .store
                        .folder_name(&id)
                        .unwrap_or_else(|| "New folder".into());
                    self.rename_folder = Some(FolderRename {
                        id,
                        text,
                        select_all: true,
                    });
                    self.switcher = None;
                    self.popup = None;
                    self.focus_editor = false;
                    self.touched();
                }
                return;
            }
            SheetAction::NewIn(parent) => {
                if !self.try_new_page(Some(parent)) {
                    return;
                }
            }
            SheetAction::MoveTo(i) => {
                if let Some(id) = self.store.sheets.get(i).map(|s| s.id.clone()) {
                    self.move_page = Some(id);
                    self.switcher = None;
                    self.popup = None;
                }
                return;
            }
            SheetAction::SyncSet(i, inside) => {
                if let Some(sheet) = self.store.sheets.get(i) {
                    let id = sheet.id.clone();
                    let title = sheet.heading();
                    self.sync.ask_set_move(&id, &title, inside);
                }
                return;
            }
            SheetAction::ToggleFolder(id) => {
                self.store.toggle_folder(&id);
                self.touched();
                return;
            }
            SheetAction::RenameFolder(id) => {
                let text = self.store.folder_name(&id).unwrap_or_default();
                self.rename_folder = Some(FolderRename {
                    id,
                    text,
                    select_all: true,
                });
                self.switcher = None;
                self.popup = None;
                self.focus_editor = false;
                return;
            }
            SheetAction::DeleteFolder(id) => {
                self.pending_delete = Some(PendingDelete::Folder(id));
                self.switcher = None;
                self.popup = None;
                return;
            }
            SheetAction::Move(i, step) => {
                self.store.select(i);
                self.store.move_active(step);
            }
            SheetAction::Copy(i) => {
                if self.refuse_locked(i) {
                    return;
                }
                self.copy_with(i, engine::with_answers, "Page copied with answers");
                return;
            }
            SheetAction::Share(i) => {
                if self.refuse_locked(i) {
                    return;
                }
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
                if self.refuse_locked(i) {
                    return;
                }
                self.copy_with(i, engine::markdown_table, "Page copied as a Markdown table");
                return;
            }
            SheetAction::Handle(i) => {
                if i < self.store.sheets.len() {
                    self.handle_edit = Some(HandleEdit {
                        index: i,
                        buffer: self.store.sheets[i].handle.clone(),
                        error: None,
                        focus: true,
                    });
                    self.popup = None;
                    self.focus_editor = false;
                }
                return;
            }
            SheetAction::Rename => {
                self.rename_at = Some(self.store.current().cursor);
                self.focus_editor = false;
                self.popup = None;
                return;
            }
            SheetAction::Preview => {
                if self
                    .store
                    .concealed(&self.store.sheets[self.store.active].id)
                {
                    self.toast = Some(Toast::info("Unlock this page to preview or print it."));
                    return;
                }
                self.page_preview = !self.page_preview;
                self.popup = None;
                if !self.page_preview {
                    self.focus_editor = true;
                }
                return;
            }
            SheetAction::LockPage(i) => {
                if let Some(id) = self.store.sheets.get(i).map(|s| s.id.clone()) {
                    self.open_lock(LockAsk::CreatePage(id));
                }
                return;
            }
            SheetAction::LockFolder(id) => {
                self.open_lock(LockAsk::CreateFolder(id));
                return;
            }
            SheetAction::UnlockFolder(id) => {
                self.open_lock(LockAsk::UnlockFolder(id));
                return;
            }
            SheetAction::ChangePage(i) => {
                if let Some(id) = self.store.sheets.get(i).map(|s| s.id.clone()) {
                    self.open_lock(LockAsk::ChangePage(id));
                }
                return;
            }
            SheetAction::ChangeFolder(id) => {
                self.open_lock(LockAsk::ChangeFolder(id));
                return;
            }
            SheetAction::RemovePage(i) => {
                if let Some(id) = self.store.sheets.get(i).map(|s| s.id.clone()) {
                    self.open_lock(LockAsk::RemovePage(id));
                }
                return;
            }
            SheetAction::RemoveFolder(id) => {
                self.open_lock(LockAsk::RemoveFolder(id));
                return;
            }
            SheetAction::LockNow => {
                self.lock_everything();
                return;
            }
        }
        self.sheet_switched();
    }

    /// `None` is a page in the open page's folder. `Some` is an explicit folder
    /// (`Some(None)` is the root). `false` when a locked folder has to be opened first.
    fn try_new_page(&mut self, parent: Option<Option<String>>) -> bool {
        let created = match &parent {
            None => self.store.new_sheet(""),
            Some(folder) => self.store.new_sheet_in(folder.as_deref(), ""),
        };
        if created {
            return true;
        }
        let folder = match parent {
            None => self
                .store
                .sheets
                .get(self.store.active)
                .and_then(|s| self.store.folder_of(&s.id)),
            Some(folder) => folder,
        };
        if let Some(id) = self.store.needs_unlock(folder.as_deref()) {
            self.pending_new = Some(id.clone());
            self.open_lock(LockAsk::UnlockFolder(id));
            self.toast = Some(Toast::info("Unlock the folder before adding a page."));
        }
        false
    }

    fn refuse_locked(&mut self, i: usize) -> bool {
        let locked = self
            .store
            .sheets
            .get(i)
            .is_some_and(|s| self.store.concealed(&s.id));
        if locked {
            self.toast = Some(Toast::info("Unlock this page first."));
        }
        locked
    }

    fn open_lock(&mut self, ask: LockAsk) {
        self.lock_ask = Some(ask);
        self.lock_password.clear();
        self.lock_confirm.clear();
        self.lock_current.clear();
        self.lock_error.clear();
        self.lock_focus = true;
        self.focus_editor = false;
        self.switcher = None;
        self.popup = None;
    }

    fn lock_everything(&mut self) {
        self.store.lock_now();
        self.gate_password.clear();
        self.gate_error.clear();
        self.page_preview = false;
        self.save();
        self.sheet_switched();
        self.toast = Some(Toast::info("Locked"));
        self.last_input = Instant::now();
    }

    fn page_menu(&self, i: usize) -> PageMenu {
        let id = self
            .store
            .sheets
            .get(i)
            .map(|s| s.id.as_str())
            .unwrap_or("");
        PageMenu {
            covered: self.store.folder_covers_page(id),
            lock: self.store.page_lock(id),
        }
    }

    /// Move a page or folder. A closed locked folder refuses plaintext dropped into it.
    fn place_page(&mut self, id: &str, spot: Place) {
        let folder = match &spot {
            Place::Into(folder) => Some(Some(folder.clone())),
            Place::RootEnd => Some(None),
            _ => None,
        };
        let into_locked = match &spot {
            Place::Into(folder) => self.store.folder_lock(folder) == Some(true),
            _ => false,
        };
        if self.store.place(id, spot) {
            if let Some(folder) = folder {
                self.sync.set_place(id, folder.as_deref());
            }
            self.touched();
        } else if into_locked {
            self.toast = Some(Toast::info(
                "Unlock the folder before moving a page into it.",
            ));
        }
    }

    fn submit_lock(&mut self) {
        let Some(ask) = self.lock_ask.clone() else {
            return;
        };
        let mismatch = "Those passwords don't match.";
        let result = match &ask {
            LockAsk::CreatePage(id) => {
                if self.lock_password != self.lock_confirm {
                    Err(mismatch.into())
                } else {
                    self.store.lock_page(id, &self.lock_password)
                }
            }
            LockAsk::CreateFolder(id) => {
                if self.lock_password != self.lock_confirm {
                    Err(mismatch.into())
                } else {
                    self.store.lock_folder(id, &self.lock_password)
                }
            }
            LockAsk::UnlockFolder(id) => self.store.unlock_folder(id, &self.lock_password),
            LockAsk::ChangePage(id) => {
                if self.lock_password != self.lock_confirm {
                    Err(mismatch.into())
                } else {
                    self.store
                        .change_page_password(id, &self.lock_current, &self.lock_password)
                }
            }
            LockAsk::ChangeFolder(id) => {
                if self.lock_password != self.lock_confirm {
                    Err(mismatch.into())
                } else {
                    self.store
                        .change_folder_password(id, &self.lock_current, &self.lock_password)
                }
            }
            LockAsk::RemovePage(id) => self.store.remove_page_lock(id, &self.lock_password),
            LockAsk::RemoveFolder(id) => self.store.remove_folder_lock(id),
        };
        match result {
            Ok(()) => {
                let created = matches!(ask, LockAsk::CreatePage(_) | LockAsk::CreateFolder(_));
                if created {
                    self.lock_ask = None;
                    self.lock_everything();
                    return;
                }
                let unlocked = matches!(ask, LockAsk::UnlockFolder(_));
                let folder = self.pending_new.clone();
                self.lock_ask = None;
                self.focus_editor = true;
                if unlocked && let Some(id) = folder {
                    self.pending_new = None;
                    self.store.new_sheet_in(Some(&id), "");
                }
                self.sheet_switched();
                self.touched();
            }
            Err(e) => self.lock_error = e,
        }
    }

    fn submit_gate(&mut self) {
        let id = self.store.sheets[self.store.active].id.clone();
        match self.store.unlock_page(&id, &self.gate_password) {
            Ok(()) => {
                self.gate_password.clear();
                self.gate_error.clear();
                self.focus_editor = true;
                self.last_input = Instant::now();
                self.sheet_switched();
            }
            Err(e) => self.gate_error = e,
        }
    }

    fn handle_background_updates(&mut self, ctx: &egui::Context) {
        let busy = ctx.input(|i| {
            i.pointer.any_pressed()
                || i.pointer.delta() != egui::Vec2::ZERO
                || i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key { pressed: true, .. }
                            | egui::Event::Text(_)
                            | egui::Event::PointerMoved(_)
                    )
                })
        });
        if busy {
            self.last_input = Instant::now();
        }
        let minutes = self.settings.lock_minutes;
        if minutes > 0
            && self.store.any_unlocked()
            && self.last_input.elapsed() >= Duration::from_secs(u64::from(minutes) * 60)
        {
            self.lock_everything();
        }
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
        // The password dialog wants the keystrokes, including Esc to close.
        if self.lock_ask.is_some() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.lock_ask = None;
                self.pending_new = None;
                self.focus_editor = true;
            }
            return;
        }
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
            self.store.lock_now();
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
            if self.help == Some(help::Tab::Settings) {
                self.help = None;
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
        if self.move_page.is_some() {
            if dismiss {
                self.move_page = None;
                self.focus_editor = true;
            }
            return;
        }
        if self.rename_folder.is_some() {
            // Leave text, paste, arrows, and selection keys for the text field.
            // Enter and Escape are the only keys this dialog handles itself.
            let key = ctx.input_mut(|i| take_name_field_key(&mut i.events));
            if dismiss || key == Some(NameFieldKey::Cancel) {
                self.rename_folder = None;
                self.rename_folder_commit = false;
                self.focus_editor = true;
            } else if key == Some(NameFieldKey::Confirm) {
                self.rename_folder_commit = true;
            }
            return;
        }
        if self.purge_id.is_some() {
            if dismiss {
                self.purge_id = None;
            }
            return;
        }
        if self.handle_edit.is_some() {
            let key = ctx.input_mut(|i| take_name_field_key(&mut i.events));
            if dismiss || key == Some(NameFieldKey::Cancel) {
                self.handle_edit = None;
                self.handle_commit = false;
                self.focus_editor = true;
            } else if key == Some(NameFieldKey::Confirm) {
                self.handle_commit = true;
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
        if self.pending_export.is_some() {
            let confirm = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            if dismiss {
                self.pending_export = None;
                self.focus_editor = true;
            } else if confirm {
                self.commit_export();
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
        if self.page_preview && dismiss {
            self.page_preview = false;
            self.focus_editor = true;
            return;
        }

        if self.rename.is_some() {
            let key = ctx.input_mut(|i| take_name_field_key(&mut i.events));
            if dismiss || key == Some(NameFieldKey::Cancel) {
                self.rename = None;
                self.focus_editor = true;
            } else if key == Some(NameFieldKey::Confirm) {
                self.rename_commit = true;
            }
            // Editing keys stay in the queue for the text field.
            return;
        }
        if self.ref_prompt.is_some() && dismiss {
            self.ref_prompt = None;
            self.ref_dismissed = true;
            self.focus_editor = true;
            return;
        }
        if self.rename.is_none() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2)) {
            self.rename_at = Some(self.store.current().cursor);
            self.focus_editor = false;
            self.popup = None;
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
        match self.pending_delete.take() {
            Some(PendingDelete::Page(i)) => {
                if i >= self.store.sheets.len() {
                    return;
                }
                let id = self.store.sheets[i].id.clone();
                let title = self.store.sheets[i].title();
                self.store.delete(i);
                self.sync.delete_page(&id);
                self.trash_at = None;
                self.toast = Some(Toast::undo(format!("Deleted \u{201c}{title}\u{201d}")));
                self.sheet_switched();
            }
            Some(PendingDelete::Folder(id)) => {
                let Some(name) = self.store.delete_folder(&id) else {
                    return;
                };
                self.trash_at = None;
                self.toast = Some(Toast::undo(format!("Deleted \u{201c}{name}\u{201d}")));
                self.sheet_switched();
            }
            None => {}
        }
    }

    fn restore_trashed(&mut self, id: &str) {
        if let Some(title) = self.store.restore(id) {
            if let Some(sheet) = self.store.sheets.iter().find(|sheet| sheet.id == id) {
                let text = sheet.text.clone();
                self.sync.restore_page(id, &text);
            }
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
        let Some(item) = popup.items.get(pick).cloned() else {
            return;
        };
        let sheet = self.store.current();
        let chars: Vec<char> = sheet.text.chars().collect();
        let mut text: String = chars[..popup.start].iter().collect();
        text.push_str(&item.text);
        text.extend(&chars[popup.end.min(chars.len())..]);
        sheet.text = text;
        sheet.mark_dirty();
        let cursor = popup.start + item.text.chars().count();
        self.pending_cursor = Some(cursor);
        if item.kind == "page" {
            let pages = self.page_catalog(false);
            let text = self.store.current().text.clone();
            if let Some(next) = engine::xref::at_picks(&self.engine, &text, cursor, &pages) {
                self.popup = Some(Popup {
                    start: next.start,
                    end: cursor,
                    items: next.items,
                    selected: 0,
                    anchor: Pos2::ZERO,
                    anchored: false,
                    explicit: true,
                });
            }
        }
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

    fn open_settings(&mut self) {
        self.help = Some(help::Tab::Settings);
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

    fn export_everything(&mut self) {
        let Some(root) = self.store.library_root() else {
            self.toast = Some(Toast::error("No folder for pages.".to_string()));
            return;
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title("Export everything")
            .set_file_name("omacus-backup.zip")
            .add_filter("Zip", &["zip"]);
        if let Some(dir) = &self.settings.export_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        match crate::backup::export_library(&root, settings::path().as_deref(), &path) {
            Ok(n) => {
                let pages = if n == 1 { "page" } else { "pages" };
                self.toast = Some(Toast::info(format!("Exported everything ({n} {pages}).")));
            }
            Err(e) => self.toast = Some(Toast::error(e)),
        }
    }

    fn reload_theme(&mut self, ctx: &egui::Context) {
        self.theme = Theme::resolve(self.settings.theme, self.system_dark);
        apply_visuals(ctx, &self.theme);
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
            &mut self.settings,
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
            Some(help::Action::SettingsChanged) => {
                self.persist_settings();
                self.reload_theme(ctx);
            }
            Some(help::Action::ExportEverything) => {
                self.export_everything();
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
        if new_folder_command(&sw.query) {
            hits.push(Hit {
                title: "New folder".to_string(),
                jump: Jump::NewFolder,
                badge: "command",
            });
        }
        if move_folder_command(&sw.query) {
            hits.push(Hit {
                title: "Move to folder…".to_string(),
                jump: Jump::MoveToFolder,
                badge: "command",
            });
        }
        if rename_command(&sw.query) {
            hits.push(Hit {
                title: "Rename variable".to_string(),
                jump: Jump::Rename,
                badge: "F2",
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
            Jump::NewFolder => self.apply(SheetAction::NewFolderAt(None)),
            Jump::MoveToFolder => self.apply(SheetAction::MoveTo(self.store.active)),
            Jump::Rename => {
                self.rename_at = Some(self.store.current().cursor);
                self.focus_editor = false;
                self.popup = None;
            }
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

        let frame = design::dialog_pad(&theme, Margin::same(8));
        let width = (ctx.content_rect().width() - 80.0).clamp(280.0, 460.0);
        let modal = egui::Modal::new(egui::Id::unique("omacus-switcher"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
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
                    let (rect, resp) = ui.allocate_exact_size(
                        vec2(ui.available_width(), theme::token::HIT),
                        Sense::click(),
                    );
                    if row == selected {
                        ui.painter()
                            .rect_filled(rect, design::radius_row(), theme.selection);
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, design::radius_row(), theme.hover);
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
        let (text, undo, sync_move, error, folder) = (
            toast.text.clone(),
            toast.undo,
            toast.sync_move,
            toast.error,
            toast.open_folder.clone(),
        );
        let mut undone = false;
        let mut open_dir = None;
        egui::Area::new(egui::Id::unique("omacus-toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
            .order(Order::Foreground)
            .interactable(undo || folder.is_some())
            .show(ctx, |ui| {
                Frame::NONE
                    .fill(theme.surface().gamma_multiply(fade))
                    .stroke(egui::Stroke::new(1.0, theme.border().gamma_multiply(fade)))
                    .corner_radius(CornerRadius::same(theme::token::RADIUS_LG))
                    .inner_margin(Margin::symmetric(16, 8))
                    .shadow(design::shadow(&theme))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let color = if error {
                                theme.danger()
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
                            if folder.is_some() {
                                let button = egui::Button::new(
                                    egui::RichText::new("Open folder")
                                        .size(13.0)
                                        .color(theme.accent.gamma_multiply(fade)),
                                )
                                .frame(false);
                                if ui.add(button).clicked() {
                                    open_dir = folder.clone();
                                }
                            }
                        });
                    });
            });
        if let Some(dir) = open_dir
            && let Err(e) = pdf::open_folder(&dir)
        {
            self.toast = Some(Toast::error(e));
        }
        if undone {
            self.toast = None;
            if sync_move {
                if let Some((id, inside)) = self.sync_undo.take() {
                    self.sync.set_in_set(&id, inside);
                    let _ = self.sync.sync_now();
                }
            } else if self.cross_undo.is_some() {
                self.restore_cross_undo();
            } else if self.store.undo_delete().is_some() {
                self.sheet_switched();
                self.touched();
            }
        }
        ctx.request_repaint();
    }

    /// How sync works, then the setup, in this window.
    fn show_sync(&mut self, ctx: &egui::Context) {
        self.tick_sync(ctx);
        let theme = self.theme.clone();
        if let Some(id) = sync::show(ctx, &theme, &mut self.sync) {
            if let Some(i) = self.store.sheets.iter().position(|sheet| sheet.id == id) {
                self.store.select(i);
            }
            self.focus_editor = true;
        }
        if let Some(ask) = self.sync.take_announced() {
            let where_name = if ask.inside {
                self.sync.vault_name().to_string()
            } else {
                "On this computer".to_string()
            };
            self.sync_undo = Some((ask.id, !ask.inside));
            self.toast = Some(Toast::moved(format!(
                "Moved {} to {where_name}.",
                ask.title
            )));
        }
    }

    fn tick_sync(&mut self, ctx: &egui::Context) {
        if self.sync.active() && self.sync.screen == sync::Screen::Closed {
            let edits: Vec<(String, String)> = self
                .store
                .sheets
                .iter()
                .filter(|sheet| sheet.revealed())
                .map(|sheet| (sheet.id.clone(), sheet.text.clone()))
                .collect();
            for (id, text) in edits {
                let changed = self
                    .sync
                    .page_text(&id)
                    .map(|have| have != text)
                    .unwrap_or(true);
                if changed {
                    self.sync.note_edit(&id, &text);
                }
            }
        }
        let wait = self.sync.waiting() || self.sync.screen == sync::Screen::Waiting;
        let every = if wait {
            Duration::from_millis(800)
        } else {
            Duration::from_secs(4)
        };
        if self.sync.active() && self.sync_poll.elapsed() >= every {
            self.sync_poll = Instant::now();
            let _ = self.sync.poll();
            if self.store.sync().active_changed {
                self.evaluate();
            }
        }
        if self.sync.screen != sync::Screen::Closed
            || self.sync.chip(Instant::now()).label().is_some()
        {
            ctx.request_repaint_after(every);
        }
    }

    fn sync_bar_on(&self) -> bool {
        self.sync.screen != sync::Screen::Closed
            || self.sync.grouped()
            || self.sync.chip(Instant::now()).label().is_some()
    }

    fn sync_title(&mut self, ui: &mut egui::Ui) {
        let theme = &self.theme;
        let heading = self
            .store
            .sheets
            .get(self.store.active)
            .map(|sheet| sheet.heading())
            .unwrap_or_default();
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::hover());
        ui.painter().text(
            pos2(row.left() + 8.0, row.center().y),
            Align2::LEFT_CENTER,
            heading,
            FontId::proportional(theme::token::TYPE_LABEL),
            theme.foreground,
        );
        let chip = self.sync.chip(Instant::now());
        if let Some(label) = chip.label() {
            let ink = if chip == sync::Chip::NotSaved {
                theme.danger()
            } else {
                theme.foreground
            };
            let dot = if chip == sync::Chip::NotSaved {
                theme.danger()
            } else {
                theme.defined
            };
            let galley = ui.painter().layout_no_wrap(
                label.to_string(),
                FontId::proportional(theme::token::TYPE_BODY),
                ink,
            );
            let width = galley.size().x + 28.0;
            let chip = Rect::from_min_size(
                pos2(row.right() - width - 8.0, row.center().y - 14.0),
                vec2(width, 28.0),
            );
            ui.painter().rect_filled(
                chip,
                CornerRadius::same(theme::token::RADIUS_MD),
                theme.background,
            );
            ui.painter().rect_stroke(
                chip,
                CornerRadius::same(theme::token::RADIUS_MD),
                egui::Stroke::new(1.0, theme.border()),
                egui::StrokeKind::Inside,
            );
            ui.painter()
                .circle_filled(pos2(chip.left() + 12.0, chip.center().y), 4.0, dot);
            ui.painter().galley(
                pos2(chip.left() + 20.0, chip.center().y - galley.size().y / 2.0),
                galley,
                ink,
            );
            if self.sync.chip(Instant::now()) == sync::Chip::ListChanged {
                let hit = ui.interact(chip, ui.scope_id().with("sync-status"), Sense::click());
                if hit.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if hit.clicked() {
                    self.list_open = true;
                }
            }
        }
        ui.painter().line_segment(
            [
                pos2(row.left(), row.bottom()),
                pos2(row.right(), row.bottom()),
            ],
            egui::Stroke::new(1.0, theme.border()),
        );
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

        let frame = design::dialog_pad(&theme, Margin::symmetric(24, 20));
        let modal = egui::Modal::new(egui::Id::unique("omacus-update"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
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
                    match (&state, method) {
                        (State::Installing, _) => {
                            ui.spinner();
                        }
                        (State::Installed, _) => {
                            restart = design::primary(ui, &theme, "Restart").clicked()
                        }
                        (State::Failed(_), _) | (_, Method::Download) => {
                            if design::primary(ui, &theme, "Download").clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                                    "{}/latest",
                                    update::RELEASES_URL
                                )));
                            }
                        }
                        (_, Method::Installer | Method::Replace) => {
                            install = design::primary(ui, &theme, "Update now").clicked()
                        }
                    }
                    if !busy {
                        if design::quiet(ui, &theme, "What's new").clicked() {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(update::notes_url(&latest)));
                        }
                        let later = if state == State::Installed {
                            "Later"
                        } else {
                            "Not now"
                        };
                        close = design::quiet(ui, &theme, later).clicked();
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

    fn show_list_change(&mut self, ctx: &egui::Context) {
        if !self.list_open {
            return;
        }
        let theme = self.theme.clone();
        let body = self.sync.list_change_sentence_now();
        let mut got_it = false;
        let mut review = false;
        let frame = design::dialog_pad(&theme, Margin::symmetric(24, 20));
        let modal = egui::Modal::new(egui::Id::unique("omacus-list-changed"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    egui::RichText::new("List changed")
                        .size(theme::token::TYPE_TITLE)
                        .color(theme.foreground),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(body)
                        .size(theme::token::TYPE_BODY)
                        .color(theme.dim),
                );
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    got_it = design::primary(ui, &theme, "Got it").clicked();
                    review = design::quiet(ui, &theme, "Review devices").clicked();
                });
            });
        if modal.should_close() {
            got_it = true;
        }
        if review {
            self.list_open = false;
            self.sync.show_home();
        } else if got_it {
            self.list_open = false;
            self.sync.ack_list();
        }
    }

    fn show_removal(&mut self, ctx: &egui::Context) {
        let count = self.sync.removal_count();
        if count == 0 {
            return;
        }
        let theme = self.theme.clone();
        let body = self.sync.removal_body();
        let titles = self.sync.removal_titles();
        let mut keep = false;
        let mut remove = false;
        let frame = design::dialog_pad(&theme, Margin::symmetric(24, 20));
        let modal = egui::Modal::new(egui::Id::unique("omacus-removal-ask"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    egui::RichText::new("Remove these pages?")
                        .size(theme::token::TYPE_TITLE)
                        .color(theme.foreground),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(body)
                        .size(theme::token::TYPE_BODY)
                        .color(theme.dim),
                );
                if !titles.is_empty() {
                    ui.add_space(8.0);
                    for title in &titles {
                        ui.label(
                            egui::RichText::new(title)
                                .size(theme::token::TYPE_BODY)
                                .color(theme.foreground),
                        );
                    }
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    keep = design::primary(ui, &theme, "Keep them").clicked();
                    remove = design::danger(ui, &theme, "Remove them").clicked();
                });
            });
        if modal.should_close() {
            keep = true;
        }
        if remove {
            self.sync.confirm_removal();
        } else if keep {
            self.sync.decline_removal();
        }
    }

    /// ⋯ above the sheet, lined up with the answers: the actions for this sheet (share,
    /// copy, move, delete) without a right-click.
    fn sheet_bar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let i = self.store.active;
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
            painter.rect_filled(dots_rect, design::radius_row(), theme.hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        painter.rect_stroke(
            dots_rect,
            design::radius_row(),
            egui::Stroke::new(1.0, theme.border()),
            egui::StrokeKind::Inside,
        );
        let color = if lit { theme.foreground } else { theme.dim };
        for dx in [-6.0, 0.0, 6.0] {
            painter.circle_filled(dots_rect.center() + vec2(dx, 0.0), 2.0, color);
        }
        let (first, last) = self.store.at_sibling_edge(i);
        let page_menu = self.page_menu(i);
        let in_set = self
            .store
            .sheets
            .get(i)
            .and_then(|sheet| self.sync.grouped().then(|| self.sync.in_set(&sheet.id)));
        menu.show(|ui| sheet_menu(ui, i, !first, !last, page_menu, in_set, &mut action));
        dots.on_hover_text("Share, copy and more");
        ui.add_space(2.0);

        if let Some(a) = action {
            self.apply(a);
            if self.lock_ask.is_none() {
                self.focus_editor = true;
            }
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
            painter.rect_filled(
                about_rect,
                CornerRadius::same(theme::token::RADIUS_SM),
                theme.hover,
            );
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

        let grouped = self.sync.grouped();
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_ROW), Sense::hover());
        if !grouped {
            ui.painter().text(
                pos2(row.left() + 14.0, row.center().y),
                Align2::LEFT_CENTER,
                "Pages",
                FontId::proportional(theme::token::TYPE_CAPTION),
                theme.text_muted(),
            );
        }
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
        let folder_btn = icon_button(
            ui,
            icon_slot(row, 2),
            id.with("new-folder"),
            &theme,
            Icon::Folder,
        );
        if folder_btn.on_hover_text("New folder").clicked() {
            action = Some(SheetAction::NewFolderAt(None));
        }
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

        let entries = self.store.side_entries();
        let headings: HashMap<String, String> = self
            .store
            .sheets
            .iter()
            .map(|s| (s.id.clone(), s.heading()))
            .collect();
        let indexes: HashMap<String, usize> = self
            .store
            .sheets
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), i))
            .collect();
        let viewing = self.preview.clone();
        let mut dropped: Option<(String, Place)> = None;
        let mut expand_id: Option<String> = None;
        let vault_rows = vault_row_indexes(&entries, |id| self.sync.in_set(id));
        let passes = if grouped {
            vec![SidePass::Vault, SidePass::Here]
        } else {
            vec![SidePass::All]
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height((ui.available_height() - 38.0 - theme::token::HIT - 8.0).max(0.0))
            .show(ui, |ui| {
                for pass in passes {
                    if grouped {
                        if pass == SidePass::Here {
                            ui.add_space(10.0);
                        }
                        let label = match pass {
                            SidePass::Vault => self.sync.vault_name().to_string(),
                            SidePass::Here => "On this computer".to_string(),
                            SidePass::All => String::new(),
                        };
                        section_label(ui, &theme, &label);
                        ui.add_space(4.0);
                    }
                    let mut shown = 0usize;
                    for (index, entry) in entries.iter().enumerate() {
                        let page_in = match &entry.kind {
                            SideKind::Page { id } => self.sync.in_set(id),
                            SideKind::Folder { .. } => false,
                        };
                        let keep = match pass {
                            SidePass::All => true,
                            SidePass::Vault => vault_rows.contains(&index),
                            SidePass::Here => {
                                matches!(entry.kind, SideKind::Page { .. }) && !page_in
                            }
                        };
                        if !keep {
                            continue;
                        }
                        shown += 1;
                        let indent = if pass == SidePass::Here {
                            0.0
                        } else {
                            entry.depth as f32 * 14.0
                        };
                        match &entry.kind {
                            SideKind::Folder {
                                id: folder_id,
                                name,
                                collapsed,
                            } => {
                                let resp = sidebar_row(
                                    ui,
                                    &theme,
                                    RowLook {
                                        heading: name,
                                        selected: false,
                                        accent: false,
                                        indent,
                                        chevron: Some(*collapsed),
                                        drag: true,
                                        lock: self.store.folder_lock(folder_id),
                                        bright: false,
                                    },
                                );
                                resp.dnd_set_drag_payload(DragPayload {
                                    id: folder_id.clone(),
                                    folder: true,
                                });
                                let dots = sidebar_dots(
                                    ui,
                                    &resp.rect,
                                    ui.scope_id().with(("folder-dots", folder_id.clone())),
                                );
                                let menu = egui::Popup::menu(&dots);
                                let open = menu.is_open();
                                if resp.hovered() || dots.hovered() || open {
                                    dots_button(ui, &dots, &theme, open);
                                }
                                let can_nest = self.store.can_nest_folder(Some(folder_id));
                                let flock = self.store.folder_lock(folder_id);
                                let any_open = self.store.any_unlocked();
                                let folder_id = folder_id.clone();
                                menu.show(|ui| {
                                    folder_menu(
                                        ui,
                                        &folder_id,
                                        can_nest,
                                        flock,
                                        any_open,
                                        &mut action,
                                    );
                                });
                                let pick = resp.clicked() && !dots.clicked();
                                dots.on_hover_text("Rename, or delete the folder");
                                resp.context_menu(|ui| {
                                    folder_menu(
                                        ui,
                                        &folder_id,
                                        can_nest,
                                        flock,
                                        any_open,
                                        &mut action,
                                    );
                                });
                                if pick {
                                    action = Some(SheetAction::ToggleFolder(folder_id.clone()));
                                }
                                self.note_drop(
                                    ui,
                                    &resp,
                                    &folder_id,
                                    DropTarget {
                                        folder: true,
                                        collapsed: *collapsed,
                                        dropped: &mut dropped,
                                        expand_id: &mut expand_id,
                                    },
                                );
                            }
                            SideKind::Page { id } => {
                                let Some(&i) = indexes.get(id) else {
                                    continue;
                                };
                                let heading =
                                    headings.get(id).map(String::as_str).unwrap_or("Untitled");
                                let active = i == self.store.active && viewing.is_none();
                                let (first, last) = self.store.at_sibling_edge(i);
                                let page_menu = self.page_menu(i);
                                let in_set = self.sync.grouped().then_some(page_in);
                                let resp = sidebar_row(
                                    ui,
                                    &theme,
                                    RowLook {
                                        heading,
                                        selected: active,
                                        accent: true,
                                        indent,
                                        chevron: None,
                                        drag: true,
                                        lock: self.store.page_lock(id),
                                        bright: false,
                                    },
                                );
                                resp.dnd_set_drag_payload(DragPayload {
                                    id: id.clone(),
                                    folder: false,
                                });
                                let dots = sidebar_dots(
                                    ui,
                                    &resp.rect,
                                    ui.scope_id().with(("sheet-dots", i)),
                                );
                                let menu = egui::Popup::menu(&dots);
                                let open = menu.is_open();
                                if active || resp.hovered() || dots.hovered() || open {
                                    dots_button(ui, &dots, &theme, open);
                                }
                                menu.show(|ui| {
                                    sheet_menu(ui, i, !first, !last, page_menu, in_set, &mut action)
                                });
                                let pick = resp.clicked() && !dots.clicked();
                                dots.on_hover_text("Share, copy and more");
                                if pick {
                                    action = Some(SheetAction::Select(i));
                                }
                                resp.context_menu(|ui| {
                                    sheet_menu(
                                        ui,
                                        i,
                                        !first,
                                        !last,
                                        page_menu,
                                        in_set,
                                        &mut action,
                                    );
                                });
                                self.note_drop(
                                    ui,
                                    &resp,
                                    id,
                                    DropTarget {
                                        folder: false,
                                        collapsed: false,
                                        dropped: &mut dropped,
                                        expand_id: &mut expand_id,
                                    },
                                );
                            }
                        }
                    }
                    if grouped && pass == SidePass::Here && shown == 0 {
                        let (hint, _) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
                        ui.painter().text(
                            pos2(hint.left() + 14.0, hint.center().y),
                            Align2::LEFT_CENTER,
                            "Pages you move here stay on this computer.",
                            FontId::proportional(theme::token::TYPE_CAPTION),
                            theme.text_muted(),
                        );
                    }
                }
                if grouped && self.sync.waiting() {
                    let resp = sidebar_row(
                        ui,
                        &theme,
                        RowLook {
                            heading: "Device waiting",
                            selected: false,
                            accent: false,
                            indent: 0.0,
                            chevron: None,
                            drag: false,
                            lock: None,
                            bright: true,
                        },
                    );
                    if resp
                        .on_hover_text("Approve this phone or computer")
                        .clicked()
                    {
                        self.sync.open_approve();
                    }
                }
                if !self.trash.is_empty() {
                    ui.add_space(14.0);
                    section_label(ui, &theme, "Recently deleted");
                    ui.add_space(4.0);
                    let trash = self.trash.clone();
                    for item in trash {
                        let heading = if item.locked {
                            crate::sheets::LOCKED_TITLE.to_string()
                        } else {
                            heading_of(&item.text)
                        };
                        let selected = viewing.as_deref() == Some(item.id.as_str());
                        let resp = sidebar_row(
                            ui,
                            &theme,
                            RowLook {
                                heading: &heading,
                                selected,
                                accent: false,
                                indent: 0.0,
                                chevron: None,
                                drag: false,
                                lock: item.locked.then_some(true),
                                bright: false,
                            },
                        );
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
                let (rest, filler) = ui.allocate_exact_size(
                    vec2(ui.available_width(), ui.available_height().max(28.0)),
                    Sense::click(),
                );
                filler.context_menu(|ui| {
                    if ui.button("New page").clicked() {
                        action = Some(SheetAction::New);
                    }
                    if ui.button("New folder").clicked() {
                        action = Some(SheetAction::NewFolderAt(None));
                    }
                });
                if let Some(payload) = filler.dnd_hover_payload::<DragPayload>() {
                    paint_drop_line(ui.painter(), rest, true, theme.accent);
                    let _ = payload;
                }
                if let Some(payload) = filler.dnd_release_payload::<DragPayload>() {
                    dropped = Some((payload.id.clone(), Place::RootEnd));
                }
            });

        if let Some(id) = expand_id {
            let due = match &self.drag_expand {
                Some((hover, at)) if hover == &id => at.elapsed() >= Duration::from_millis(450),
                _ => {
                    self.drag_expand = Some((id, Instant::now()));
                    false
                }
            };
            if due {
                let id = self.drag_expand.take().map(|(id, _)| id);
                if let Some(id) = id {
                    self.store.expand_folder(&id);
                    self.touched();
                }
            }
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        } else {
            self.drag_expand = None;
        }
        if egui::DragAndDrop::payload::<DragPayload>(ui.ctx()).is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
            self.paint_drag_chip(ui.ctx());
        }
        if let Some((id, spot)) = dropped {
            let folder = self.store.folder_name(&id).is_some();
            let target = match &spot {
                Place::Before(target) | Place::After(target) | Place::Into(target) => {
                    Some(target.clone())
                }
                Place::RootEnd => None,
            };
            let ask = if self.sync.grouped() && !folder {
                target.and_then(|target| {
                    if self.store.folder_name(&target).is_some() {
                        return None;
                    }
                    if self.sync.in_set(&id) == self.sync.in_set(&target) {
                        return None;
                    }
                    let title = self
                        .store
                        .sheets
                        .iter()
                        .find(|sheet| sheet.id == id)
                        .map(|sheet| sheet.heading())
                        .unwrap_or_default();
                    Some((title, self.sync.in_set(&target)))
                })
            } else {
                None
            };
            if let Some((title, inside)) = ask {
                self.sync.ask_set_move(&id, &title, inside);
            } else {
                self.place_page(&id, spot);
            }
        }

        ui.add_space(6.0);
        if self.sync.offer_setup() {
            let turn_on = ui.add_sized(
                [ui.available_width(), theme::token::HIT],
                egui::Button::new(
                    egui::RichText::new("Turn on sync")
                        .size(theme::token::TYPE_BODY)
                        .color(theme.text()),
                )
                .fill(theme.background)
                .stroke(egui::Stroke::new(1.0, theme.border()))
                .corner_radius(design::radius_row())
                .min_size(vec2(ui.available_width(), theme::token::HIT)),
            );
            if turn_on.on_hover_text("How sync works").clicked() {
                self.sync.open_intro();
            }
            ui.add_space(6.0);
        }
        if self.sync.grouped() {
            let open_sync = ui.add_sized(
                [ui.available_width(), theme::token::HIT],
                egui::Button::new(
                    egui::RichText::new("Sync")
                        .size(theme::token::TYPE_BODY)
                        .color(theme.text()),
                )
                .fill(theme.background)
                .stroke(egui::Stroke::new(1.0, theme.border()))
                .corner_radius(design::radius_row())
                .min_size(vec2(ui.available_width(), theme::token::HIT)),
            );
            if open_sync.on_hover_text("Sync settings").clicked() {
                self.sync.show_home();
            }
            ui.add_space(6.0);
        }
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

    /// A drag hovering `resp`. Paints the drop line or the folder highlight, and
    /// remembers a collapsed folder so it can open after a short hover.
    fn note_drop(&self, ui: &egui::Ui, resp: &egui::Response, id: &str, target: DropTarget) {
        let Some(payload) = resp.dnd_hover_payload::<DragPayload>() else {
            return;
        };
        if payload.id == id {
            return;
        }
        let DropTarget {
            folder,
            collapsed,
            dropped,
            expand_id,
        } = target;
        let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let rect = resp.rect;
        let rel = ((pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        let into = folder && (0.28..0.72).contains(&rel);
        let spot = if into {
            if collapsed {
                *expand_id = Some(id.to_string());
            }
            ui.painter().rect_stroke(
                rect,
                design::radius_row(),
                egui::Stroke::new(1.5, self.theme.accent),
                egui::StrokeKind::Inside,
            );
            Place::Into(id.to_string())
        } else if rel < 0.5 {
            paint_drop_line(ui.painter(), rect, true, self.theme.accent);
            Place::Before(id.to_string())
        } else {
            paint_drop_line(ui.painter(), rect, false, self.theme.accent);
            Place::After(id.to_string())
        };
        if resp.dnd_release_payload::<DragPayload>().is_some() {
            *dropped = Some((payload.id.clone(), spot));
        }
    }

    fn paint_drag_chip(&self, ctx: &egui::Context) {
        let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(ctx) else {
            return;
        };
        let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let label = if payload.folder {
            self.store
                .folder_name(&payload.id)
                .unwrap_or_else(|| "Folder".into())
        } else {
            self.store
                .sheets
                .iter()
                .find(|s| s.id == payload.id)
                .map(|s| s.heading())
                .unwrap_or_else(|| "Page".into())
        };
        let theme = &self.theme;
        let galley = ctx.fonts_mut(|f| {
            f.layout_job(sidebar_label_job(
                &label,
                FontId::proportional(13.0),
                theme.foreground,
                180.0,
            ))
        });
        let size = galley.size() + vec2(16.0, 8.0);
        let rect = Rect::from_min_size(pos + vec2(12.0, 12.0), size);
        let painter = ctx.layer_painter(egui::LayerId::new(
            Order::Tooltip,
            egui::Id::unique("omacus-drag"),
        ));
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::token::RADIUS_SM),
            theme.sidebar,
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(theme::token::RADIUS_SM),
            egui::Stroke::new(1.0, theme.accent),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.min + vec2(8.0, 4.0), galley, theme.foreground);
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

        // A name field is a real text editor. The page must not keep the keys.
        if self.naming() {
            self.focus_editor = false;
            if let Some(id) = self.editor_id {
                ui.ctx().memory_mut(|mem| mem.surrender_focus(id));
            }
        }

        // One editor per sheet, so each keeps its own cursor and undo history.
        let before_edit = self.store.sheets[self.store.active].text.clone();
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

        self.editor_id = Some(response.id);
        if self.demo_rename {
            self.demo_rename = false;
            let text = self.store.current().text.clone();
            if let Some((at, _)) = text.match_indices("rent").nth(1) {
                self.open_rename(at);
                if let Some(session) = &mut self.rename {
                    session.buffer = "fee".into();
                }
            }
        } else if let Some(at) = self.rename_at.take() {
            self.open_rename(at);
            if self.rename.is_none() {
                self.focus_editor = true;
                self.toast = Some(Toast::info("Put the cursor on a variable name."));
            }
        }
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
            let after_edit = self.store.current().text.clone();
            self.note_definition_edit(&before_edit, &after_edit);
            self.note_cross_undo();
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
                let text = self.store.sheets[self.store.active].text.clone();
                let at = output.galley.pos_from_cursor(CCursor::new(c));
                let anchor = output.galley_pos + at.left_bottom().to_vec2() + vec2(0.0, 2.0);
                let pages = self.page_catalog(false);
                if let Some(pick) = engine::xref::at_picks(&self.engine, &text, c, &pages) {
                    // Enter accepts: choosing a page inserts `@handle.` and the
                    // variable list opens on the next key only if we keep it up.
                    self.popup = Some(Popup {
                        start: pick.start,
                        end: c,
                        items: pick.items,
                        selected: 0,
                        anchor,
                        anchored: true,
                        explicit: true,
                    });
                } else {
                    self.popup = complete::word_before(&text, c).and_then(|(start, word)| {
                        let items = complete::suggest(&word, &self.vars);
                        (!items.is_empty()).then_some(Popup {
                            start,
                            end: c,
                            items,
                            selected: 0,
                            anchor,
                            anchored: true,
                            explicit,
                        })
                    });
                }
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
        if response.clicked()
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        {
            let cc = output.galley.cursor_from_pos(pos - output.galley_pos);
            let text = self.store.current().text.clone();
            if let Some(r) = engine::xref::ref_at(&text, cc.index.0) {
                self.pending_open = Some(r.handle);
            }
        }
        if response.hovered() {
            let menu_open = response.secondary_clicked() || response.context_menu_opened();
            self.show_hover(ui, &output.galley, output.galley_pos, menu_open);
        }
        self.rename_menu(ui, response, &output.galley, output.galley_pos);
        self.show_rename_field(ui, &output.galley, output.galley_pos);
        self.show_ref_prompt(ui, &output.galley, output.galley_pos);
        if self.rename_commit {
            self.rename_commit = false;
            self.commit_rename(ui.ctx());
        }

        if self.store.current().text.trim().is_empty() {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                empty_sheet_hint(self.store.sidebar),
                0.0,
                egui::TextFormat::simple(
                    FontId::proportional(theme::token::TYPE_LABEL),
                    theme.text_secondary(),
                ),
            );
            job.wrap.max_width = (response.rect.width() - 8.0).max(40.0);
            let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
            ui.painter().galley(
                response.rect.left_top() + vec2(2.0, 2.0),
                galley,
                theme.text_secondary(),
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
        if let Some(handle) = self.pending_open.take() {
            self.open_handle(&handle);
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
                egui::Stroke::new(1.0, theme.border()),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().galley(
            pos2(content_left + extra, y + (h - size.y) / 2.0),
            galley,
            color,
        );
    }

    /// Pins a popup or hover card for `OMACUS_SCREENSHOT_HELP`. The editor clears
    /// `popup` when the cursor has not moved, so this runs after the editor.
    fn stage_screenshot(&mut self, ctx: &egui::Context) {
        let Ok(mode) = std::env::var("OMACUS_SCREENSHOT_HELP") else {
            return;
        };
        match mode.as_str() {
            "picker" => {
                self.popup = Some(Popup {
                    start: 0,
                    end: 0,
                    items: vec![
                        complete::Suggestion {
                            text: "@trip".into(),
                            kind: "page",
                            detail: "Trip Budget".into(),
                        },
                        complete::Suggestion {
                            text: "total".into(),
                            kind: "variable",
                            detail: "Trip Budget".into(),
                        },
                    ],
                    selected: 0,
                    anchor: pos2(360.0, 168.0),
                    anchored: true,
                    explicit: true,
                });
            }
            "hover" => {
                let token = Rect::from_min_size(pos2(300.0, 150.0), vec2(72.0, 20.0));
                paint_hover_tip(
                    ctx,
                    &self.theme,
                    token,
                    "$186",
                    &["rent + groceries".into()],
                );
            }
            _ => {}
        }
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
                design::popup(theme).show(ui, |ui| {
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
                            ui.allocate_exact_size(vec2(width, theme::token::HIT), Sense::click());
                        let selected = i == popup.selected;
                        if selected {
                            ui.painter()
                                .rect_filled(rect, design::radius_row(), theme.selection);
                        } else if resp.hovered() {
                            ui.painter()
                                .rect_filled(rect, design::radius_row(), theme.hover);
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

    fn naming(&self) -> bool {
        self.rename.is_some()
            || self.rename_at.is_some()
            || self.rename_folder.is_some()
            || self.handle_edit.is_some()
    }

    fn pop_state(&self, value: bool, menu: bool) -> PopState {
        PopState {
            naming: self.naming(),
            menu,
            prompt: self.ref_prompt.is_some(),
            completion: self.popup.is_some(),
            value,
            typing: self.edited_at.elapsed() < HINT_IDLE,
            dialog: self.switcher.is_some()
                || self.help.is_some()
                || self.page_preview
                || self.pending_delete.is_some()
                || self.move_page.is_some()
                || self.lock_ask.is_some(),
        }
    }

    /// The value of the variable or number under the pointer.
    fn show_hover(&mut self, ui: &egui::Ui, galley: &egui::Galley, origin: Pos2, menu_open: bool) {
        if shown_pop(self.pop_state(true, menu_open)) != ShownPop::Value {
            return;
        }
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
        let mut extras = Vec::new();
        if let Some(title) = hit
            .source_title
            .as_deref()
            .filter(|title| !title.is_empty())
        {
            extras.push(hover_from_line(title));
        } else if let Some(handle) = hit
            .source_handle
            .as_deref()
            .filter(|handle| !handle.is_empty())
        {
            extras.push(hover_from_line(&format!("@{handle}")));
        }
        match hit.used_by {
            0 => {}
            1 => extras.push("Used by 1 other page".to_string()),
            n => extras.push(format!("Used by {n} other pages")),
        }
        let tip = paint_hover_tip(ui.ctx(), &self.theme, rect, &hit.value, &extras);
        if let Some(handle) = hit.source_handle.clone()
            && ui.input(|i| i.pointer.any_click())
            && tip.contains(pointer)
        {
            self.pending_open = Some(handle);
        }
    }

    fn open_rename(&mut self, cursor: usize) {
        let text = self.store.current().text.clone();
        let Some(binding) = engine::rename::binding_at(&text, cursor) else {
            return;
        };
        self.popup = None;
        self.ref_prompt = None;
        self.ref_base = None;
        self.ref_dismissed = false;
        self.focus_editor = false;
        self.rename = Some(RenameSession {
            cursor,
            token_start: binding.token_start,
            buffer: binding.name,
            error: None,
            focus: true,
        });
    }

    fn rename_menu(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        galley: &egui::Galley,
        origin: Pos2,
    ) {
        if response.secondary_clicked()
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        {
            let cc = galley.cursor_from_pos(pos - origin);
            self.rename_menu_at = Some(cc.index.0);
        }
        if self.rename.is_some() {
            self.rename_menu_at = None;
            return;
        }
        let menu_at = self.rename_menu_at;
        let text = self.store.current().text.clone();
        let on_name = menu_at.is_some_and(|at| engine::rename::binding_at(&text, at).is_some());
        if !on_name {
            return;
        }
        let mut chosen = None;
        response.context_menu(|ui| {
            if ui
                .add(egui::Button::new("Rename variable").shortcut_text("F2"))
                .clicked()
            {
                chosen = menu_at;
            }
        });
        if let Some(at) = chosen {
            self.open_rename(at);
        }
    }

    fn show_rename_field(&mut self, ui: &egui::Ui, galley: &egui::Galley, origin: Pos2) {
        let Some(session) = &self.rename else {
            return;
        };
        let start = galley.pos_from_cursor(CCursor::new(session.token_start));
        let pos = origin + start.left_bottom().to_vec2() + vec2(0.0, 4.0);
        let theme = self.theme.clone();
        let mut buffer = session.buffer.clone();
        let error = session.error.clone();
        let focus = session.focus;
        let blurb = error.is_none().then(|| {
            engine::xref::rename_preview(
                &self.page_catalog(true),
                self.store.active,
                session.cursor,
                &session.buffer,
            )
        });
        let mut written = None;
        egui::Area::new(egui::Id::unique("omacus-rename"))
            .order(Order::Foreground)
            .fixed_pos(pos)
            .interactable(true)
            .show(ui.ctx(), |ui| {
                design::field(&theme).show(ui, |ui| {
                    let width = (buffer.chars().count().max(8) as f32) * 10.0 + 12.0;
                    let chars = buffer.chars().count();
                    let out = TextEdit::singleline(&mut buffer)
                        .id(egui::Id::unique("omacus-rename-field"))
                        .font(FontId::monospace(FONT_SIZE))
                        .desired_width(width.clamp(96.0, 280.0))
                        .hint_text("New name")
                        .show(ui);
                    place_name_caret(ui.ctx(), &out, chars, focus);
                    if let Some(err) = &error {
                        ui.label(egui::RichText::new(err).size(12.5).color(theme.danger()));
                    } else if let Some(Some(blurb)) = &blurb {
                        ui.label(egui::RichText::new(blurb).size(12.0).color(theme.dim));
                    }
                });
                written = Some(buffer);
            });
        if let Some(session) = &mut self.rename {
            if let Some(buffer) = written {
                session.buffer = buffer;
            }
            session.focus = false;
        }
    }

    fn show_ref_prompt(&mut self, ui: &egui::Ui, galley: &egui::Galley, origin: Pos2) {
        if self.rename.is_some() {
            return;
        }
        let Some(prompt) = self.ref_prompt.clone() else {
            return;
        };
        let at = self.store.current().cursor;
        let pos = galley.pos_from_cursor(CCursor::new(at));
        let anchor = origin + pos.left_bottom().to_vec2() + vec2(0.0, 6.0);
        let theme = self.theme.clone();
        let mut apply = false;
        let mut dismiss = false;
        egui::Area::new(egui::Id::unique("omacus-ref-prompt"))
            .order(Order::Foreground)
            .fixed_pos(anchor)
            .interactable(true)
            .show(ui.ctx(), |ui| {
                design::popup_pad(&theme, Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if prompt.apply {
                            if design::primary(ui, &theme, &prompt.label).clicked() {
                                apply = true;
                            }
                        } else {
                            ui.label(
                                egui::RichText::new(&prompt.label)
                                    .size(13.0)
                                    .color(theme.danger()),
                            );
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Dismiss").size(12.5).color(theme.dim),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            dismiss = true;
                        }
                    });
                });
            });
        if apply {
            self.apply_ref_prompt(ui.ctx());
        } else if dismiss {
            self.ref_prompt = None;
            self.ref_dismissed = true;
        }
    }

    fn note_definition_edit(&mut self, before: &str, after: &str) {
        if self.rename.is_some() {
            return;
        }
        let continued = self.ref_base.is_some();
        let base = self.ref_base.clone().unwrap_or_else(|| before.to_string());
        if let Some(edit) = engine::rename::definition_change(&base, after) {
            if !continued {
                self.ref_dismissed = false;
            }
            self.ref_base = Some(base);
            let mut prompt = ref_prompt_from(edit);
            self.note_foreign_uses(&mut prompt);
            self.ref_prompt = (!self.ref_dismissed).then_some(prompt);
        } else if let Some(edit) = engine::rename::definition_change(before, after) {
            self.ref_dismissed = false;
            self.ref_base = Some(before.to_string());
            let mut prompt = ref_prompt_from(edit);
            self.note_foreign_uses(&mut prompt);
            self.ref_prompt = Some(prompt);
        } else {
            self.ref_base = None;
            self.ref_dismissed = false;
            self.ref_prompt = None;
        }
    }

    fn commit_rename(&mut self, ctx: &egui::Context) {
        let Some(session) = self.rename.clone() else {
            return;
        };
        let pages = self.page_catalog(true);
        let source = self.store.active;
        match engine::xref::propagate_rename(&pages, source, session.cursor, &session.buffer) {
            Ok(done) => {
                self.apply_propagation(ctx, &done);
                if let Some(cursor) = done.cursor {
                    self.pending_cursor = Some(cursor);
                }
                self.rename = None;
                self.ref_prompt = None;
                self.ref_base = None;
                self.ref_dismissed = false;
                self.focus_editor = true;
            }
            Err(engine::rename::RenameFail::NotAVariable) => {
                self.rename = None;
                self.focus_editor = true;
            }
            Err(err) => {
                if let Some(session) = &mut self.rename {
                    session.error = Some(err.message());
                    session.focus = true;
                }
            }
        }
    }

    fn apply_ref_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.ref_prompt.take() else {
            return;
        };
        self.ref_base = None;
        self.ref_dismissed = false;
        if !prompt.apply {
            return;
        }
        let old = self.store.current().text.clone();
        let spans: Vec<engine::rename::Span> = prompt
            .spans
            .iter()
            .map(|(start, end)| engine::rename::Span {
                start: *start,
                end: *end,
            })
            .collect();
        let text = engine::rename::rewrite(&old, &spans, &prompt.new_name);
        let handle = self.store.current().handle.clone();
        let (text, _) =
            engine::xref::rewrite_var_refs(&text, &handle, &prompt.old_name, &prompt.new_name);
        let mut edits = Vec::new();
        if text != old {
            edits.push(engine::xref::PageEdit {
                id: self.store.current().id.clone(),
                index: self.store.active,
                text,
                handle: handle.clone(),
            });
        }
        let pages = self.page_catalog(true);
        for (index, page) in pages.iter().enumerate() {
            if page.handle.eq_ignore_ascii_case(&handle) {
                continue;
            }
            let (text, n) = engine::xref::rewrite_var_refs(
                &page.text,
                &handle,
                &prompt.old_name,
                &prompt.new_name,
            );
            if n == 0 || page.locked || page.elsewhere {
                continue;
            }
            edits.push(engine::xref::PageEdit {
                id: page.id.clone(),
                index,
                text,
                handle: page.handle.clone(),
            });
        }
        self.apply_propagation(
            ctx,
            &engine::xref::Propagation {
                edits,
                here: prompt.spans.len(),
                elsewhere: 0,
                skipped_locked: 0,
                skipped_elsewhere: 0,
                blurb: prompt.label,
                cursor: None,
            },
        );
        self.focus_editor = true;
    }

    /// Count `@handle.old` on other pages and fold that into the prompt.
    fn note_foreign_uses(&mut self, prompt: &mut RefPrompt) {
        if prompt.collision {
            return;
        }
        let handle = self.store.current().handle.clone();
        let pages = self.page_catalog(true);
        let mut elsewhere = 0;
        let mut skipped_locked = 0;
        let mut skipped_elsewhere = 0;
        for page in &pages {
            if page.handle.eq_ignore_ascii_case(&handle) {
                continue;
            }
            let (_, n) = engine::xref::rewrite_var_refs(
                &page.text,
                &handle,
                &prompt.old_name,
                &prompt.new_name,
            );
            if n == 0 {
                continue;
            }
            if page.locked {
                skipped_locked += 1;
            } else if page.elsewhere {
                skipped_elsewhere += 1;
            } else {
                elsewhere += n;
            }
        }
        if elsewhere == 0 && skipped_locked == 0 && skipped_elsewhere == 0 {
            return;
        }
        prompt.label = engine::xref::rename_blurb(
            &prompt.old_name,
            &prompt.new_name,
            prompt.spans.len(),
            elsewhere,
            skipped_locked,
            skipped_elsewhere,
        );
        if elsewhere > 0 {
            prompt.apply = true;
        }
    }

    /// Switch to the page named `handle`, or open it from Recently deleted.
    fn open_handle(&mut self, handle: &str) {
        if let Some(i) = self
            .store
            .sheets
            .iter()
            .position(|s| s.handle.eq_ignore_ascii_case(handle))
        {
            self.preview = None;
            if i != self.store.active {
                self.store.select(i);
                self.sheet_switched();
            }
            return;
        }
        self.refresh_trash(false);
        if let Some(item) = self
            .trash
            .iter()
            .find(|t| t.handle.eq_ignore_ascii_case(handle))
        {
            self.open_preview(item.id.clone());
        }
    }

    /// After the editor undoes back to the text from before a cross-page rename,
    /// put the other pages back too. That is one undo step.
    fn note_cross_undo(&mut self) {
        let Some(undo) = &self.cross_undo else {
            return;
        };
        if self.store.current().id != undo.id || !undo.armed {
            return;
        }
        if self.store.current().text != undo.before_text {
            return;
        }
        let undo = self.cross_undo.take().expect("just checked");
        let id = undo.id.clone();
        let before_handle = undo.before_handle.clone();
        self.restore_cross_pages(&undo);
        if self.store.current().id == id && self.store.current().handle != before_handle {
            self.store.remember_handle(&id, &before_handle);
        }
        self.toast = None;
        self.trash_at = None;
        self.evaluate();
        self.touched();
    }

    fn restore_cross_pages(&mut self, undo: &CrossUndo) {
        for (id, text, handle) in &undo.others {
            if let Some(sheet) = self.store.sheets.iter_mut().find(|s| &s.id == id)
                && &sheet.text != text
            {
                sheet.text = text.clone();
                sheet.mark_dirty();
            }
            let now = self
                .store
                .sheets
                .iter()
                .find(|s| &s.id == id)
                .map(|s| s.handle.clone());
            if now.as_deref() != Some(handle.as_str()) {
                self.store.remember_handle(id, handle);
            }
            self.store.rewrite_saved(id, text, handle);
        }
    }

    /// Write a rename or handle change onto every page it touched.
    fn apply_propagation(&mut self, ctx: &egui::Context, done: &engine::xref::Propagation) {
        if done.edits.is_empty() {
            return;
        }
        let active = self.store.active;
        let active_id = self.store.sheets[active].id.clone();
        let before_text = self.store.sheets[active].text.clone();
        let before_handle = self.store.sheets[active].handle.clone();
        let mut others = Vec::new();
        for edit in &done.edits {
            if edit.id == active_id {
                continue;
            }
            if let Some(sheet) = self.store.sheets.iter().find(|s| s.id == edit.id) {
                others.push((sheet.id.clone(), sheet.text.clone(), sheet.handle.clone()));
            } else if let Some(sheet) = self
                .store
                .pending_sheets()
                .iter()
                .find(|sheet| sheet.id == edit.id)
            {
                others.push((sheet.id.clone(), sheet.text.clone(), sheet.handle.clone()));
            } else if let Some(item) = self.trash.iter().find(|t| t.id == edit.id) {
                others.push((item.id.clone(), item.text.clone(), item.handle.clone()));
            }
        }
        let mut after_text = before_text.clone();
        let mut current_text_changed = false;
        for edit in &done.edits {
            if edit.id == active_id {
                if self.store.sheets[active].text != edit.text {
                    self.store.sheets[active].text = edit.text.clone();
                    self.store.sheets[active].mark_dirty();
                    after_text = edit.text.clone();
                    current_text_changed = true;
                }
                if self.store.sheets[active].handle != edit.handle {
                    self.store.remember_handle(&edit.id, &edit.handle);
                }
            } else if self.store.sheets.iter().any(|s| s.id == edit.id) {
                if let Some(sheet) = self.store.sheets.iter_mut().find(|s| s.id == edit.id)
                    && sheet.text != edit.text
                {
                    sheet.text = edit.text.clone();
                    sheet.mark_dirty();
                }
                let now = self
                    .store
                    .sheets
                    .iter()
                    .find(|s| s.id == edit.id)
                    .map(|s| s.handle.clone());
                if now.as_deref() != Some(edit.handle.as_str()) {
                    self.store.remember_handle(&edit.id, &edit.handle);
                }
            } else {
                self.store.rewrite_saved(&edit.id, &edit.text, &edit.handle);
            }
        }
        self.cross_undo = Some(CrossUndo {
            id: active_id,
            before_text: before_text.clone(),
            before_handle,
            after_text,
            others,
            armed: true,
        });
        if current_text_changed {
            self.push_editor_undo(ctx, &before_text);
        } else {
            self.toast = Some(Toast::undo(done.blurb.clone()));
        }
        self.trash_at = None;
        self.evaluate();
        self.touched();
    }

    /// Toast Undo, when the page on screen did not change and the editor has
    /// nothing to undo.
    fn restore_cross_undo(&mut self) {
        let Some(undo) = self.cross_undo.take() else {
            return;
        };
        let id = undo.id.clone();
        let before_text = undo.before_text.clone();
        let after_text = undo.after_text.clone();
        let before_handle = undo.before_handle.clone();
        if let Some(sheet) = self.store.sheets.iter_mut().find(|s| s.id == id)
            && before_text != after_text
            && sheet.text == after_text
        {
            sheet.text = before_text;
            sheet.mark_dirty();
        }
        let now = self
            .store
            .sheets
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.handle.clone());
        if now.as_deref() != Some(before_handle.as_str()) {
            self.store.remember_handle(&id, &before_handle);
        }
        self.restore_cross_pages(&undo);
        self.trash_at = None;
        self.evaluate();
        self.touched();
    }

    fn show_handle(&mut self, ctx: &egui::Context) {
        let Some(edit) = &self.handle_edit else {
            return;
        };
        let index = edit.index;
        if index >= self.store.sheets.len() {
            self.handle_edit = None;
            return;
        }
        let theme = self.theme.clone();
        let mut buffer = edit.buffer.clone();
        let error = edit.error.clone();
        let focus = edit.focus;
        let current = self.store.sheets[index].handle.clone();
        let title = self.store.sheets[index].title();
        let pages = self.page_catalog(true);
        let blurb = if buffer.trim().eq_ignore_ascii_case(&current) {
            None
        } else {
            match engine::xref::propagate_handle(&pages, index, &buffer) {
                Ok(done) => Some(done.blurb),
                Err(err) => Some(err.message()),
            }
        };
        let mut save = false;
        let mut cancel = false;
        let frame = design::dialog(&theme);
        let modal = egui::Modal::new(egui::Id::unique("omacus-handle"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    egui::RichText::new(format!("Handle for \u{201c}{title}\u{201d}"))
                        .size(theme::token::TYPE_TITLE)
                        .color(theme.foreground),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Other pages read this page as @handle.name. Renaming the page leaves the handle as it is.",
                    )
                    .size(13.0)
                    .color(theme.dim),
                );
                ui.add_space(10.0);
                let chars = buffer.chars().count();
                let out = TextEdit::singleline(&mut buffer)
                    .id(egui::Id::unique("omacus-handle-field"))
                    .font(FontId::monospace(FONT_SIZE))
                    .desired_width(ui.available_width())
                    .hint_text("handle")
                    .show(ui);
                place_name_caret(ui.ctx(), &out, chars, focus);
                if let Some(note) = error.as_ref().or(blurb.as_ref()) {
                    ui.add_space(6.0);
                    let bad = error.is_some()
                        || note.contains("already used")
                        || note.contains("letters and digits");
                    ui.label(
                        egui::RichText::new(note)
                            .size(12.5)
                            .color(if bad { theme.danger() } else { theme.dim }),
                    );
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    save = design::primary(ui, &theme, "Save").clicked();
                    cancel = design::quiet(ui, &theme, "Cancel").clicked();
                });
            });
        if let Some(edit) = &mut self.handle_edit {
            edit.buffer = buffer;
            edit.focus = false;
        }
        if modal.should_close() {
            cancel = true;
        }
        let commit = std::mem::take(&mut self.handle_commit) || save;
        if cancel {
            self.handle_edit = None;
            self.handle_commit = false;
            self.focus_editor = true;
        } else if commit {
            self.commit_handle(ctx);
        }
    }

    fn commit_handle(&mut self, ctx: &egui::Context) {
        let Some(edit) = self.handle_edit.as_ref() else {
            return;
        };
        let index = edit.index;
        let buffer = edit.buffer.clone();
        if index >= self.store.sheets.len() {
            self.handle_edit = None;
            return;
        }
        let pages = self.page_catalog(true);
        match engine::xref::propagate_handle(&pages, index, &buffer) {
            Ok(done) => {
                self.apply_propagation(ctx, &done);
                self.handle_edit = None;
                self.focus_editor = true;
            }
            Err(err) => {
                if let Some(edit) = &mut self.handle_edit {
                    edit.error = Some(err.message());
                    edit.focus = true;
                }
            }
        }
    }

    /// One undo step: the page as it was before this rename.
    fn push_editor_undo(&self, ctx: &egui::Context, old_text: &str) {
        let Some(id) = self.editor_id else {
            return;
        };
        let cursor = self.store.sheets[self.store.active].cursor;
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        let mut undoer = state.undoer();
        undoer.add_undo(&(
            CCursorRange::one(CCursor::new(cursor)),
            old_text.to_string(),
        ));
        state.set_undoer(undoer);
        state.store(ctx, id);
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
        let pages = self.page_catalog(false);
        let handle = self.store.current().handle.clone();
        let hit = self.engine.hover_with(text, cursor, &handle, &pages);
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
        let sync_rect = Rect::from_center_size(
            pos2(x, gear_rect.center().y - (ICON_BOX + 8.0)),
            Vec2::splat(ICON_BOX),
        );
        let turn_on = icon_button(ui, sync_rect, id.with("rail-sync"), &theme, Icon::Cloud);
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
            painter.rect_filled(
                toggle,
                CornerRadius::same(theme::token::RADIUS_SM),
                theme.hover,
            );
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
        } else if turn_on
            .on_hover_text(if self.sync.grouped() {
                "Sync"
            } else {
                "Turn on sync"
            })
            .clicked()
        {
            if self.sync.offer_setup() {
                self.sync.open_intro();
            } else if self.sync.grouped() {
                self.sync.show_home();
            }
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
        let column = !self.settings.hide_answers;
        if let Some(action) = preview::show(ui, &theme, &text, &results, column) {
            match action {
                preview::Action::Close => {
                    self.page_preview = false;
                    self.focus_editor = true;
                }
                preview::Action::Print => {
                    match pdf::print_preview(&title, &preview::blocks(&text, &results), column) {
                        Ok(_) => {
                            self.toast = Some(Toast::info("Opened the preview to print"));
                        }
                        Err(e) => self.toast = Some(Toast::error(e)),
                    }
                }
                preview::Action::Export => {
                    self.begin_export(&title, preview::blocks(&text, &results));
                }
                preview::Action::Copy(text) => {
                    self.toast = Some(Toast::info(format!("Copied {text}")));
                }
            }
        }
    }

    fn preview_sheet(&mut self, ui: &mut egui::Ui, item: &Trashed) {
        let theme = self.theme.clone();
        let heading = if item.locked {
            crate::sheets::LOCKED_TITLE.to_string()
        } else {
            heading_of(&item.text)
        };
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(heading)
                    .size(theme::token::TYPE_TITLE)
                    .color(theme.foreground),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let forever = ui.add(
                    egui::Button::new(
                        egui::RichText::new("Delete forever")
                            .size(theme::token::TYPE_BODY)
                            .color(theme.danger()),
                    )
                    .frame(false),
                );
                let restore = design::primary(ui, &theme, "Restore");
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
        if item.locked {
            ui.label(
                egui::RichText::new(
                    "This page is locked. Restore it and enter the password to read it.",
                )
                .size(13.5)
                .color(theme.dim),
            );
            return;
        }
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

    fn export_dir(&self) -> std::path::PathBuf {
        self.settings
            .export_dir
            .clone()
            .unwrap_or_else(pdf::default_export_dir)
    }

    fn begin_export(&mut self, title: &str, blocks: Vec<omacus_engine::markdown::Block>) {
        let dir = self.export_dir();
        let path = pdf::export_path(&dir, title);
        let column = !self.settings.hide_answers;
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Page.pdf".into());
        let folder_label = pdf::folder_label(&dir);
        let pending = PendingExport {
            path: path.clone(),
            folder_label,
            file_name,
            title: title.to_string(),
            blocks,
            column,
        };
        if pdf::needs_replace_prompt(path.exists(), self.settings.replace_exports) {
            self.pending_export = Some(pending);
            self.focus_editor = false;
        } else {
            self.write_export(pending);
        }
    }

    fn commit_export(&mut self) {
        if let Some(pending) = self.pending_export.take() {
            self.write_export(pending);
        }
    }

    fn write_export(&mut self, pending: PendingExport) {
        let dir = pending
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| pending.path.clone());
        match pdf::write(
            &pending.title,
            &pending.blocks,
            &pending.path,
            pending.column,
        ) {
            Ok(()) => {
                self.toast = Some(Toast::saved(
                    format!("Saved {} in {}", pending.file_name, pending.folder_label),
                    dir,
                ));
            }
            Err(e) => self.toast = Some(Toast::error(e)),
        }
    }

    fn show_replace(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_export else {
            return;
        };
        let file_name = pending.file_name.clone();
        let folder_label = pending.folder_label.clone();
        let theme = self.theme.clone();
        let mut replace = false;
        let mut cancel = false;
        let frame = design::dialog(&theme);
        let modal = egui::Modal::new(egui::Id::unique("omacus-replace"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{file_name} already exists in {folder_label}. Replace it?"
                    ))
                    .size(theme::token::TYPE_TITLE)
                    .color(theme.foreground),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    replace = design::primary(ui, &theme, "Replace").clicked();
                    cancel = design::quiet(ui, &theme, "Cancel").clicked();
                });
            });
        if modal.should_close() {
            cancel = true;
        }
        if replace {
            self.commit_export();
        } else if cancel {
            self.pending_export = None;
            self.focus_editor = true;
        }
    }

    fn show_delete(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_delete.clone() else {
            return;
        };
        let (title, detail) = match &pending {
            PendingDelete::Page(i) => {
                if *i >= self.store.sheets.len() {
                    self.pending_delete = None;
                    return;
                }
                (
                    self.store.sheets[*i].title(),
                    "It's moved to Recently deleted, in the sidebar and the page switcher.",
                )
            }
            PendingDelete::Folder(id) => {
                let Some(name) = self.store.folder_name(id) else {
                    self.pending_delete = None;
                    return;
                };
                (
                    name,
                    "Pages inside move to Recently deleted, where they can be restored.",
                )
            }
        };
        let theme = self.theme.clone();
        let mut delete = false;
        let mut cancel = false;
        let frame = design::dialog(&theme);
        let modal = egui::Modal::new(egui::Id::unique("omacus-delete"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(
                    egui::RichText::new(format!("Delete \u{201c}{title}\u{201d}?"))
                        .size(theme::token::TYPE_TITLE)
                        .color(theme.foreground),
                );
                ui.add_space(6.0);
                ui.label(egui::RichText::new(detail).size(13.0).color(theme.dim));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    delete = design::danger(ui, &theme, "Delete").clicked();
                    cancel = design::quiet(ui, &theme, "Cancel").clicked();
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

    /// The same line was changed in two places. One question, two actions.
    fn show_pick(&mut self, ctx: &egui::Context) {
        let Some(conflict) = self.store.line_conflicts().into_iter().next() else {
            return;
        };
        let sealed = conflict.line_id == "body"
            || conflict.ours.starts_with("OMACUS-LOCK/")
            || conflict.theirs.starts_with("OMACUS-LOCK/");
        let (title, detail, keep_label, other_label) = if sealed {
            (
                "Pick one copy",
                "This locked page was changed in two places.",
                "Keep this copy",
                "Use the other copy",
            )
        } else {
            (
                "Pick one line",
                "This line was changed in two places.",
                "Keep this line",
                "Use the other line",
            )
        };
        let theme = self.theme.clone();
        let ours = conflict.ours.clone();
        let theirs = conflict.theirs.clone();
        let mut keep = false;
        let mut other = false;
        let frame = design::dialog(&theme);
        let modal = egui::Modal::new(egui::Id::unique("omacus-pick"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    egui::RichText::new(title)
                        .size(theme::token::TYPE_TITLE)
                        .color(theme.foreground),
                );
                ui.add_space(6.0);
                ui.label(egui::RichText::new(detail).size(13.0).color(theme.dim));
                if !sealed {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("This window")
                            .size(12.0)
                            .color(theme.dim),
                    );
                    ui.label(
                        egui::RichText::new(&ours)
                            .size(theme::token::TYPE_BODY)
                            .family(egui::FontFamily::Monospace)
                            .color(theme.foreground),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("The other copy")
                            .size(12.0)
                            .color(theme.dim),
                    );
                    ui.label(
                        egui::RichText::new(&theirs)
                            .size(theme::token::TYPE_BODY)
                            .family(egui::FontFamily::Monospace)
                            .color(theme.foreground),
                    );
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    keep = design::primary(ui, &theme, keep_label).clicked();
                    other = design::quiet(ui, &theme, other_label).clicked();
                });
            });
        if modal.should_close() {
            keep = true;
        }
        if keep || other {
            let page = conflict.page_id.clone();
            let line = conflict.line_id.clone();
            let active = self
                .store
                .sheets
                .get(self.store.active)
                .map(|s| s.id.clone());
            if self.store.pick_line(&page, &line, other && !keep) {
                self.save();
                if active.as_deref() == Some(page.as_str()) {
                    self.evaluate();
                }
            }
        }
    }

    fn show_lock_gate(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        ui.add_space(36.0);
        ui.label(
            egui::RichText::new("This page is locked")
                .size(18.0)
                .color(theme.foreground),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Enter the password to open it.")
                .size(13.5)
                .color(theme.dim),
        );
        ui.add_space(12.0);
        let mut password = self.gate_password.clone();
        let resp = ui.add(
            TextEdit::singleline(&mut password)
                .password(true)
                .hint_text("Password")
                .desired_width(280.0)
                .font(FontId::proportional(15.0)),
        );
        if std::mem::take(&mut self.focus_editor) {
            resp.request_focus();
        }
        let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if password != self.gate_password {
            self.gate_error.clear();
        }
        self.gate_password = password;
        if !self.gate_error.is_empty() {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(&self.gate_error)
                    .size(theme::token::TYPE_CAPTION)
                    .color(theme.danger()),
            );
        }
        ui.add_space(10.0);
        let button = design::primary(ui, &theme, "Unlock").clicked();
        if submit || button {
            self.submit_gate();
        }
    }

    fn show_lock(&mut self, ctx: &egui::Context) {
        let Some(ask) = self.lock_ask.clone() else {
            return;
        };
        let theme = self.theme.clone();
        let mut password = self.lock_password.clone();
        let mut confirm = self.lock_confirm.clone();
        let mut current = self.lock_current.clone();
        let error = self.lock_error.clone();
        let mut grab = self.lock_focus;
        let mut submit = false;
        let mut cancel = false;
        let (title, detail, confirm_label, fields) = match &ask {
            LockAsk::CreatePage(_) => (
                "Lock page",
                "Save this password in a password manager. A forgotten password can't be recovered.",
                "Lock",
                LockFields::New,
            ),
            LockAsk::CreateFolder(_) => (
                "Lock folder",
                "One password locks every page in this folder. Save this password in a password manager. A forgotten password can't be recovered.",
                "Lock",
                LockFields::New,
            ),
            LockAsk::UnlockFolder(_) => (
                "Unlock folder",
                "Enter the password to open the pages in this folder.",
                "Unlock",
                LockFields::Password,
            ),
            LockAsk::ChangePage(_) | LockAsk::ChangeFolder(_) => (
                "Change password",
                "A forgotten password can't be recovered.",
                "Change",
                LockFields::Change,
            ),
            LockAsk::RemovePage(_) => (
                "Remove lock",
                "Enter the password. The page stays on this device, without a lock.",
                "Remove lock",
                LockFields::Password,
            ),
            LockAsk::RemoveFolder(_) => (
                "Remove lock",
                "Pages in this folder will be stored without a password.",
                "Remove lock",
                LockFields::None,
            ),
        };
        let frame = design::dialog(&theme);
        let modal = egui::Modal::new(egui::Id::unique("omacus-lock"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
            .show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(
                    egui::RichText::new(title)
                        .size(theme::token::TYPE_TITLE)
                        .color(theme.foreground),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(detail)
                        .size(theme::token::TYPE_BODY)
                        .color(theme.text_secondary()),
                );
                ui.add_space(10.0);
                let enter = match fields {
                    LockFields::None => false,
                    LockFields::Password => {
                        password_field(ui, &mut password, "Password", &mut grab)
                    }
                    LockFields::New => {
                        let first = password_field(ui, &mut password, "Password", &mut grab);
                        ui.add_space(6.0);
                        let second =
                            password_field(ui, &mut confirm, "Confirm password", &mut grab);
                        first || second
                    }
                    LockFields::Change => {
                        let first = password_field(ui, &mut current, "Current password", &mut grab);
                        ui.add_space(6.0);
                        let second = password_field(ui, &mut password, "New password", &mut grab);
                        ui.add_space(6.0);
                        let third = password_field(ui, &mut confirm, "Confirm password", &mut grab);
                        first || second || third
                    }
                };
                if !error.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(&error)
                            .size(theme::token::TYPE_CAPTION)
                            .color(theme.danger()),
                    );
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let remove = matches!(ask, LockAsk::RemovePage(_) | LockAsk::RemoveFolder(_));
                    submit = if remove {
                        design::danger(ui, &theme, confirm_label).clicked()
                    } else {
                        design::primary(ui, &theme, confirm_label).clicked()
                    } || enter;
                    cancel = design::quiet(ui, &theme, "Cancel").clicked();
                });
            });
        self.lock_password = password;
        self.lock_confirm = confirm;
        self.lock_current = current;
        self.lock_focus = grab;
        if modal.should_close() {
            cancel = true;
        }
        if submit {
            self.submit_lock();
        } else if cancel {
            self.lock_ask = None;
            self.pending_new = None;
            self.focus_editor = true;
        }
    }

    fn show_move(&mut self, ctx: &egui::Context) {
        let Some(page_id) = self.move_page.clone() else {
            return;
        };
        if self.store.sheets.iter().all(|s| s.id != page_id) {
            self.move_page = None;
            return;
        }
        let theme = self.theme.clone();
        let folders = self.store.folders();
        let current = self.store.folder_of(&page_id);
        let mut picked: Option<Option<String>> = None;
        let mut cancel = false;
        let frame = design::dialog_pad(&theme, Margin::symmetric(16, 14));
        let modal = egui::Modal::new(egui::Id::unique("omacus-move"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
            .show(ctx, |ui| {
                ui.set_width(300.0);
                ui.label(
                    egui::RichText::new("Move to folder")
                        .size(theme::token::TYPE_TITLE)
                        .color(theme.foreground),
                );
                ui.add_space(8.0);
                let root_label = if current.is_none() {
                    "No folder  ·  current"
                } else {
                    "No folder"
                };
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(root_label)
                                .size(13.5)
                                .color(theme.foreground),
                        )
                        .min_size(vec2(ui.available_width(), theme::token::HIT)),
                    )
                    .clicked()
                {
                    picked = Some(None);
                }
                if folders.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("No folders yet. Create one from the sidebar.")
                            .size(12.5)
                            .color(theme.comment),
                    );
                }
                for folder in &folders {
                    let here = current.as_deref() == Some(folder.id.as_str());
                    let name = if here {
                        format!("{}  ·  current", folder.name)
                    } else {
                        folder.name.clone()
                    };
                    ui.add_space(2.0);
                    let pad = (folder.depth.saturating_sub(1) as f32) * 14.0;
                    ui.horizontal(|ui| {
                        if pad > 0.0 {
                            ui.add_space(pad);
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(name).size(13.5).color(theme.foreground),
                                )
                                .min_size(vec2(ui.available_width(), theme::token::HIT)),
                            )
                            .clicked()
                        {
                            picked = Some(Some(folder.id.clone()));
                        }
                    });
                }
                ui.add_space(8.0);
                cancel = design::quiet(ui, &theme, "Cancel").clicked();
            });
        if modal.should_close() {
            cancel = true;
        }
        if let Some(parent) = picked {
            let moving = current.as_deref() != parent.as_deref();
            if moving {
                let spot = match parent {
                    Some(id) => Place::Into(id),
                    None => Place::RootEnd,
                };
                self.place_page(&page_id, spot);
            }
            self.move_page = None;
            self.focus_editor = true;
        } else if cancel {
            self.move_page = None;
            self.focus_editor = true;
        }
    }

    fn show_rename(&mut self, ctx: &egui::Context) {
        let Some(rename) = self.rename_folder.clone() else {
            return;
        };
        let theme = self.theme.clone();
        let mut shown = rename.text.clone();
        let select_all = rename.select_all;
        let mut cancel = false;
        let frame = design::dialog_pad(&theme, Margin::symmetric(16, 14));
        let modal = egui::Modal::new(egui::Id::unique("omacus-rename-folder"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
            .show(ctx, |ui| {
                ui.set_width(300.0);
                ui.label(
                    egui::RichText::new("Rename folder")
                        .size(theme::token::TYPE_TITLE)
                        .color(theme.foreground),
                );
                ui.add_space(8.0);
                // A focused field. The page editor has given up the keyboard,
                // and only Enter and Escape are taken before this runs.
                let chars = shown.chars().count();
                let out = TextEdit::singleline(&mut shown)
                    .id(egui::Id::unique("omacus-folder-name"))
                    .font(FontId::proportional(15.0))
                    .desired_width(f32::INFINITY)
                    .show(ui);
                place_name_caret(ui.ctx(), &out, chars, select_all);
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Enter keeps the name. Esc cancels.")
                        .size(12.0)
                        .color(theme.comment),
                );
                ui.add_space(8.0);
                cancel = design::quiet(ui, &theme, "Cancel").clicked();
            });
        if let Some(rename) = &mut self.rename_folder {
            rename.text = shown;
            rename.select_all = false;
        }
        let commit = std::mem::take(&mut self.rename_folder_commit);
        if modal.should_close() || cancel {
            self.rename_folder = None;
            self.rename_folder_commit = false;
            self.focus_editor = true;
        } else if commit && let Some(rename) = self.rename_folder.take() {
            self.store.rename_folder(&rename.id, &rename.text);
            self.touched();
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
            .map(|t| {
                if t.locked {
                    crate::sheets::LOCKED_TITLE.into()
                } else {
                    heading_of(&t.text)
                }
            })
            .unwrap_or_else(|| "this page".into());
        let theme = self.theme.clone();
        let mut purge = false;
        let mut cancel = false;
        let frame = design::dialog(&theme);
        let modal = egui::Modal::new(egui::Id::unique("omacus-purge"))
            .frame(frame)
            .backdrop_color(design::backdrop(&theme))
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(
                    egui::RichText::new(format!("Delete \u{201c}{title}\u{201d} forever?"))
                        .size(theme::token::TYPE_TITLE)
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
                    purge = design::danger(ui, &theme, "Delete forever").clicked();
                    cancel = design::quiet(ui, &theme, "Cancel").clicked();
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
                left: theme::token::SPACE_5 as i8,
                right: theme::token::SPACE_5 as i8,
                top: 8,
                bottom: 20,
            });
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            let concealed = self
                .store
                .sheets
                .get(self.store.active)
                .is_some_and(|s| self.store.concealed(&s.id));
            if self.page_preview {
                self.show_page_preview(ui);
            } else if let Some(item) = self.current_preview() {
                self.preview_sheet(ui, &item);
            } else if concealed {
                self.show_lock_gate(ui);
            } else {
                if self.sync_bar_on() {
                    self.sync_title(ui);
                }
                self.sheet_bar(ui);
                egui::ScrollArea::vertical()
                    .id_salt(("scroll", &self.store.sheets[self.store.active].id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.sheet(ui));
            }
        });

        self.stage_screenshot(&ctx);
        self.popup(&ctx);
        self.show_help(&ctx);
        self.show_update(&ctx);
        self.show_list_change(&ctx);
        self.show_removal(&ctx);
        self.show_sync(&ctx);
        self.show_delete(&ctx);
        self.show_pick(&ctx);
        self.show_move(&ctx);
        self.show_lock(&ctx);
        self.show_rename(&ctx);
        self.show_handle(&ctx);
        self.show_replace(&ctx);
        self.show_purge(&ctx);
        self.show_switcher(&ctx);
        self.show_toast(&ctx);
    }

    // glow's `on_exit` receives the GL context. Windows enables wgpu only
    // (see Cargo.toml), and eframe's trait takes no context then.
    #[cfg(not(windows))]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        sync::release_clipboard();
        self.store.lock_now();
        self.save();
    }

    #[cfg(windows)]
    fn on_exit(&mut self) {
        sync::release_clipboard();
        self.store.lock_now();
        self.save();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme.background.to_normalized_gamma_f32()
    }
}

/// The actions for sheet `i`: the right-click menu, and the ⋯ on its row and above it.
fn sheet_menu(
    ui: &mut egui::Ui,
    i: usize,
    can_up: bool,
    can_down: bool,
    menu: PageMenu,
    in_set: Option<bool>,
    action: &mut Option<SheetAction>,
) {
    let item = |ui: &mut egui::Ui, text: &str, keys: Option<&str>| {
        let mut button = egui::Button::new(text);
        if let Some(k) = keys {
            button = button.shortcut_text(help::keys(k));
        }
        ui.add(button).clicked()
    };
    if item(ui, "Rename", Some("F2")) {
        *action = Some(SheetAction::Rename);
    }
    if item(ui, "Share link", Some("Ctrl+Shift+L")) {
        *action = Some(SheetAction::Share(i));
    }
    ui.menu_button("Copy / Export", |ui| {
        if item(ui, "Copy with answers", Some("Ctrl+Shift+S")) {
            *action = Some(SheetAction::Copy(i));
        }
        if item(ui, "Copy as Markdown table", Some("Ctrl+Shift+M")) {
            *action = Some(SheetAction::CopyMarkdown(i));
        }
        if item(ui, "Preview", Some("Ctrl+P")) {
            *action = Some(SheetAction::Preview);
        }
    });
    if item(ui, "Handle\u{2026}", None) {
        *action = Some(SheetAction::Handle(i));
    }
    if can_up && item(ui, "Move up", None) {
        *action = Some(SheetAction::Move(i, -1));
    }
    if can_down && item(ui, "Move down", None) {
        *action = Some(SheetAction::Move(i, 1));
    }
    if item(ui, "Move to folder…", None) {
        *action = Some(SheetAction::MoveTo(i));
    }
    if let Some(inside) = in_set {
        let label = if inside {
            "Move to this computer"
        } else {
            "Move to My pages"
        };
        if item(ui, label, None) {
            *action = Some(SheetAction::SyncSet(i, !inside));
        }
    }
    ui.separator();
    lock_page_items(ui, i, menu, action);
    ui.separator();
    if item(ui, "Delete page", Some("Ctrl+Shift+Delete")) {
        *action = Some(SheetAction::Delete(i));
    }
}

fn lock_page_items(ui: &mut egui::Ui, i: usize, menu: PageMenu, action: &mut Option<SheetAction>) {
    let item = |ui: &mut egui::Ui, text: &str| ui.add(egui::Button::new(text)).clicked();
    // A page with no password of its own, and not inside a locked folder.
    if menu.lock.is_none() {
        if item(ui, "Lock…") {
            *action = Some(SheetAction::LockPage(i));
        }
        return;
    }
    if menu.lock == Some(true) {
        if item(ui, "Unlock…") {
            *action = Some(SheetAction::Select(i));
        }
    } else if item(ui, "Lock now") {
        *action = Some(SheetAction::LockNow);
    }
    if !menu.covered {
        if menu.lock == Some(false) && item(ui, "Change password…") {
            *action = Some(SheetAction::ChangePage(i));
        }
        if item(ui, "Remove lock…") {
            *action = Some(SheetAction::RemovePage(i));
        }
    }
}

fn folder_menu(
    ui: &mut egui::Ui,
    id: &str,
    can_nest: bool,
    lock: Option<bool>,
    any_open: bool,
    action: &mut Option<SheetAction>,
) {
    if ui.button("New page").clicked() {
        *action = Some(SheetAction::NewIn(Some(id.to_string())));
    }
    if can_nest && ui.button("New folder").clicked() {
        *action = Some(SheetAction::NewFolderAt(Some(id.to_string())));
    }
    if ui.button("Rename").clicked() {
        *action = Some(SheetAction::RenameFolder(id.to_string()));
    }
    ui.separator();
    match lock {
        None => {
            if ui.button("Lock…").clicked() {
                *action = Some(SheetAction::LockFolder(id.to_string()));
            }
        }
        Some(true) => {
            if ui.button("Unlock…").clicked() {
                *action = Some(SheetAction::UnlockFolder(id.to_string()));
            }
        }
        Some(false) => {
            if ui.button("Change password…").clicked() {
                *action = Some(SheetAction::ChangeFolder(id.to_string()));
            }
            if ui.button("Remove lock…").clicked() {
                *action = Some(SheetAction::RemoveFolder(id.to_string()));
            }
        }
    }
    if any_open && ui.button("Lock now").clicked() {
        *action = Some(SheetAction::LockNow);
    }
    ui.separator();
    if ui.button("Delete folder").clicked() {
        *action = Some(SheetAction::DeleteFolder(id.to_string()));
    }
}

enum LockFields {
    None,
    Password,
    New,
    Change,
}

/// A masked single-line field. Enter submits. `grab` focuses the first field once.
fn password_field(ui: &mut egui::Ui, text: &mut String, hint: &str, grab: &mut bool) -> bool {
    let resp = ui.add(
        TextEdit::singleline(text)
            .password(true)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .font(FontId::proportional(15.0)),
    );
    if *grab {
        resp.request_focus();
        *grab = false;
    }
    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
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
    Folder,
    Cloud,
}

fn sync_dir() -> std::path::PathBuf {
    crate::sheets::data_root().unwrap_or_else(|| std::env::temp_dir().join("omacus"))
}

fn sync_origin() -> String {
    std::env::var("OMACUS_SYNC_URL").unwrap_or_else(|_| "https://sync.omacus.com".into())
}

fn local_pages(store: &Store) -> sync::LocalPages {
    let doc = store
        .directory()
        .and_then(|dir| std::fs::read(dir.join("library.automerge")).ok())
        .unwrap_or_else(|| omacus_doc::Library::new().save());
    sync::LocalPages {
        doc,
        pages: store
            .sheets
            .iter()
            .map(|sheet| (sheet.id.clone(), sheet.stored().to_string()))
            .collect(),
        locked: store
            .sheets
            .iter()
            .filter(|sheet| sheet.locked() && !sheet.revealed())
            .map(|sheet| sheet.id.clone())
            .collect(),
    }
}

fn seed_sync_screen(store: &mut Store, kind: &str) {
    let trip = "Trip Budget\nFlights $640\nHotel, 4 nights $820";
    if store.sheets.is_empty() {
        store.new_sheet(trip);
    } else {
        store.sheets[0].text = trip.into();
        store.sheets[0].mark_dirty();
    }
    if !store
        .sheets
        .iter()
        .any(|sheet| sheet.heading() == "Groceries")
    {
        store.new_sheet("Groceries\nmilk 3");
    }
    if kind == "locked" {
        store.new_sheet("Harbor notes\npier 12");
        if let Some(sheet) = store
            .sheets
            .iter()
            .find(|sheet| sheet.text.starts_with("Harbor notes"))
        {
            let id = sheet.id.clone();
            let _ = store.lock_page(&id, "harbor-secret");
        }
    }
    if kind == "waiting"
        && !store
            .sheets
            .iter()
            .any(|sheet| sheet.heading() == "Scratch")
    {
        store.new_sheet("Scratch\n");
    }
    if kind == "locked" {
        if let Some(i) = store.sheets.iter().position(|sheet| sheet.locked()) {
            store.select(i);
        }
    } else if let Some(i) = store
        .sheets
        .iter()
        .position(|sheet| sheet.heading() == "Trip Budget")
    {
        store.select(i);
    }
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
        // A tabbed folder, same stroke weight as the other icons.
        Icon::Folder => {
            let r = Rect::from_center_size(c + vec2(0.0, 1.0), vec2(15.0, 10.0));
            painter.line_segment(
                [pos2(r.left(), r.top()), pos2(r.left() + 5.0, r.top())],
                stroke,
            );
            painter.line_segment(
                [
                    pos2(r.left() + 5.0, r.top()),
                    pos2(r.left() + 7.0, r.top() - 3.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    pos2(r.left() + 7.0, r.top() - 3.0),
                    pos2(r.right(), r.top() - 3.0),
                ],
                stroke,
            );
            painter.rect_stroke(r, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
        }
        Icon::Cloud => {
            painter.circle_stroke(c + vec2(-2.6, 0.6), 3.1, stroke);
            painter.circle_stroke(c + vec2(2.4, -0.6), 3.8, stroke);
            painter.line_segment([c + vec2(-5.6, 3.2), c + vec2(6.0, 3.2)], stroke);
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
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::token::RADIUS_SM),
            theme.hover,
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        theme.foreground
    } else {
        theme.dim
    };
    paint_icon(ui.painter(), icon, rect.center(), color);
    resp
}

/// "from Trip Budget" under a cross-page value, inside the same card.
fn hover_from_line(name: &str) -> String {
    format!("from {name}")
}

/// Where a hover value sits: above the token, below it when that would leave
/// the window, and shifted so the whole tip stays inside `bounds`. The tip is
/// exactly as wide as its widest line, so a long amount is never wrapped.
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

fn paint_hover_tip(
    ctx: &egui::Context,
    theme: &Theme,
    token: Rect,
    value: &str,
    extras: &[String],
) -> Rect {
    let font = FontId::monospace(14.0);
    let galley =
        ctx.fonts_mut(|fonts| fonts.layout_no_wrap(value.to_string(), font.clone(), theme.accent));
    let extra: Vec<_> = extras
        .iter()
        .map(|line| {
            ctx.fonts_mut(|fonts| {
                fonts.layout_no_wrap(line.clone(), FontId::proportional(12.5), theme.dim)
            })
        })
        .collect();
    let width = extra.iter().fold(galley.size().x, |w, g| w.max(g.size().x));
    let height = galley.size().y + extra.iter().map(|g| g.size().y + 2.0).sum::<f32>();
    let bounds = ctx.content_rect().shrink(8.0);
    let tip = hover_tip_rect(token, vec2(width, height), bounds);
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
        egui::Stroke::new(1.0, theme.border()),
        egui::StrokeKind::Inside,
    );
    painter.galley(tip.min + vec2(12.0, 8.0), galley.clone(), theme.accent);
    let mut y = tip.min.y + 8.0 + galley.size().y;
    for galley in extra {
        y += 2.0;
        let h = galley.size().y;
        painter.galley(pos2(tip.min.x + 12.0, y), galley, theme.dim);
        y += h;
    }
    tip
}

/// The switcher opens Settings when the query is the start of that word.
/// A single letter would steal every page whose name starts with "s".
fn settings_command(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.len() >= 2 && "settings".starts_with(&q)
}

/// The switcher offers New folder once the query is the start of that command.
fn new_folder_command(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    (q.len() >= 2 && "new folder".starts_with(&q)) || (q.len() >= 3 && "folder".starts_with(&q))
}

/// The switcher offers Move to folder… for the open page.
fn move_folder_command(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.len() >= 2 && "move to folder".starts_with(&q)
}

/// The switcher offers Rename variable, and shows F2 beside it.
fn rename_command(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.len() >= 2 && ("rename".starts_with(&q) || "rename variable".starts_with(&q))
}

/// Which floating layer may be on screen. At most one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShownPop {
    None,
    Value,
    Completion,
    Rename,
    Menu,
    Prompt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PopState {
    naming: bool,
    menu: bool,
    prompt: bool,
    completion: bool,
    value: bool,
    /// The page changed in the last moment. Value tips stay hidden.
    typing: bool,
    /// A dialog is covering the page (switcher, help, preview, delete, move, lock).
    dialog: bool,
}

fn shown_pop(state: PopState) -> ShownPop {
    if state.dialog {
        return ShownPop::None;
    }
    if state.naming {
        return ShownPop::Rename;
    }
    if state.menu {
        return ShownPop::Menu;
    }
    if state.prompt {
        return ShownPop::Prompt;
    }
    if state.completion {
        return ShownPop::Completion;
    }
    if state.value && !state.typing {
        ShownPop::Value
    } else {
        ShownPop::None
    }
}

/// Enter confirms a name and Escape cancels it. Everything else — letters,
/// digits, spaces, arrows, Home/End, Delete, copy, cut, paste — stays in
/// `events` for the text field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NameFieldKey {
    Confirm,
    Cancel,
}

fn take_name_field_key(events: &mut Vec<egui::Event>) -> Option<NameFieldKey> {
    let mut found = None;
    events.retain(|event| match event {
        egui::Event::Key {
            key: Key::Enter,
            pressed: true,
            modifiers,
            repeat,
            ..
        } if !modifiers.command && !modifiers.alt && !modifiers.shift => {
            if !repeat {
                found = Some(NameFieldKey::Confirm);
            }
            false
        }
        egui::Event::Key {
            key: Key::Escape,
            pressed: true,
            modifiers,
            repeat,
            ..
        } if modifiers.is_none() => {
            if !repeat {
                found = Some(NameFieldKey::Cancel);
            }
            false
        }
        _ => true,
    });
    found
}

/// Focus a name field once, and select its text when it has just opened.
/// Requesting focus on later frames would interrupt IME.
fn place_name_caret(
    ctx: &egui::Context,
    out: &egui::text_edit::TextEditOutput,
    chars: usize,
    select_all: bool,
) {
    if select_all || !out.response.has_focus() {
        out.response.request_focus();
    }
    if !select_all {
        return;
    }
    let mut state = out.state.clone();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(0),
        CCursor::new(chars),
    )));
    state.store(ctx, out.response.id);
}

/// The sidebar's button column: centered on the rows' ⋯, `slot` buttons in from the right.
fn icon_slot(row: Rect, slot: usize) -> Rect {
    let x = row.right() - 15.0 - slot as f32 * (ICON_BOX + 4.0);
    Rect::from_center_size(pos2(x, row.center().y), Vec2::splat(ICON_BOX))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidePass {
    All,
    Vault,
    Here,
}

/// Folder rows that still hold a page in My pages, plus those pages.
fn vault_row_indexes(
    entries: &[crate::sheets::SideEntry],
    in_vault: impl Fn(&str) -> bool,
) -> HashSet<usize> {
    let mut show = HashSet::new();
    let mut stack: Vec<(usize, usize, bool)> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        while stack
            .last()
            .is_some_and(|(_, depth, _)| *depth >= entry.depth)
        {
            let (idx, _, has) = stack.pop().unwrap();
            if has {
                show.insert(idx);
            }
        }
        match &entry.kind {
            SideKind::Folder { .. } => stack.push((index, entry.depth, false)),
            SideKind::Page { id } => {
                if in_vault(id) {
                    show.insert(index);
                    for item in &mut stack {
                        item.2 = true;
                    }
                }
            }
        }
    }
    while let Some((idx, _, has)) = stack.pop() {
        if has {
            show.insert(idx);
        }
    }
    show
}

/// "Pages", "Recently deleted": lined up with the page names below.
fn section_label(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    ui.painter().text(
        pos2(row.left() + 14.0, row.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(theme::token::TYPE_CAPTION),
        theme.text_muted(),
    );
}

/// A ⋯ button: brighter, with a background, when hovered or its menu is open.
fn dots_button(ui: &egui::Ui, resp: &egui::Response, theme: &Theme, open: bool) {
    let painter = ui.painter();
    if resp.hovered() || open {
        painter.rect_filled(resp.rect, design::radius_row(), theme.hover);
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
    design::apply(ctx, theme);
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
    let size = if std::env::var("OMACUS_SYNC_SCREEN").is_ok() {
        Vec2::new(1280.0, 800.0)
    } else {
        Vec2::new(980.0, 600.0)
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("omacus")
            .with_title("Omacus")
            .with_inner_size(size)
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

struct RowLook<'a> {
    heading: &'a str,
    selected: bool,
    accent: bool,
    indent: f32,
    /// `Some(collapsed)` draws a disclosure triangle.
    chevron: Option<bool>,
    drag: bool,
    /// `Some(true)` is a closed lock, `Some(false)` an open one.
    lock: Option<bool>,
    /// Foreground text when the row is not the open page.
    bright: bool,
}

fn sidebar_row(ui: &mut egui::Ui, theme: &Theme, look: RowLook<'_>) -> egui::Response {
    let sense = if look.drag {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), SIDEBAR_ROW), sense);
    let painter = ui.painter();
    if look.selected {
        painter.rect_filled(rect, design::radius_row(), theme.selection);
        if look.accent {
            painter.rect_filled(sidebar_accent(rect), CornerRadius::same(2), theme.accent);
        }
    } else if resp.hovered() {
        painter.rect_filled(rect, design::radius_row(), theme.hover);
    }
    let color = if look.selected || look.bright {
        theme.foreground
    } else {
        theme.dim
    };
    let mut text_x = rect.left() + 14.0 + look.indent;
    if let Some(collapsed) = look.chevron {
        paint_chevron(
            painter,
            pos2(text_x + 1.0, rect.center().y),
            collapsed,
            color,
        );
        text_x += 16.0;
    }
    let lock_room = if look.lock.is_some() { 18.0 } else { 0.0 };
    let text_width = (rect.right() - text_x - 30.0 - lock_room).max(24.0);
    let galley = ui.ctx().fonts_mut(|f| {
        f.layout_job(sidebar_label_job(
            look.heading,
            FontId::proportional(14.0),
            color,
            text_width,
        ))
    });
    let elided = galley.elided;
    painter.galley(
        pos2(text_x, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if let Some(closed) = look.lock {
        paint_lock(
            painter,
            pos2(rect.right() - 28.0, rect.center().y),
            closed,
            color,
        );
    }
    if elided {
        resp.clone().on_hover_text(look.heading.to_owned());
    }
    resp
}

fn paint_lock(painter: &egui::Painter, c: Pos2, closed: bool, color: Color32) {
    let stroke = egui::Stroke::new(1.3, color);
    let body = Rect::from_center_size(c + vec2(0.0, 2.2), vec2(8.0, 6.0));
    painter.rect_stroke(
        body,
        CornerRadius::same(1),
        stroke,
        egui::StrokeKind::Middle,
    );
    let left = c + vec2(-2.4, 0.0);
    let right = c + vec2(2.4, if closed { -3.2 } else { -4.6 });
    painter.line_segment([left, left + vec2(0.0, -3.2)], stroke);
    painter.line_segment([left + vec2(0.0, -3.2), right], stroke);
    painter.line_segment(
        [right, right + vec2(0.0, if closed { 3.2 } else { 2.0 })],
        stroke,
    );
}

fn paint_chevron(painter: &egui::Painter, c: Pos2, collapsed: bool, color: Color32) {
    let stroke = egui::Stroke::new(1.4, color);
    if collapsed {
        painter.line_segment([c + vec2(-3.0, -4.0), c + vec2(3.0, 0.0)], stroke);
        painter.line_segment([c + vec2(3.0, 0.0), c + vec2(-3.0, 4.0)], stroke);
    } else {
        painter.line_segment([c + vec2(-4.0, -2.0), c + vec2(0.0, 3.0)], stroke);
        painter.line_segment([c + vec2(0.0, 3.0), c + vec2(4.0, -2.0)], stroke);
    }
}

fn paint_drop_line(painter: &egui::Painter, row: Rect, top: bool, color: Color32) {
    let y = if top { row.top() } else { row.bottom() };
    let line = Rect::from_center_size(
        pos2(row.center().x, y),
        vec2((row.width() - 16.0).max(8.0), 2.0),
    );
    painter.rect_filled(line, CornerRadius::same(1), color);
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
        BAR_ROW, HINT_IDLE, NameFieldKey, PopState, SIDEBAR_ROW, Shortcut, ShownPop, SwitcherEdit,
        apply_switcher_query, empty_sheet_hint, hover_from_line, hover_tip_rect, icon_slot,
        line_at, move_folder_command, name_hint_visible, new_folder_command, owns_key,
        rename_command, settings_command, sheet_event_filter, shortcut_for, shown_pop,
        sidebar_accent, sidebar_label_job, switcher_takes_focus, take_chords, take_name_field_key,
        take_switcher_edits,
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

        // A cross-page card is one rectangle: the value and "from Trip Budget"
        // share it, and it still sits above the reference when there is room.
        assert_eq!(hover_from_line("Trip Budget"), "from Trip Budget");
        let value = vec2(36.0, 16.0);
        let from = vec2(108.0, 14.0);
        let card = vec2(value.x.max(from.x), value.y + 2.0 + from.y);
        let token = Rect::from_min_size(pos2(80.0, 180.0), vec2(70.0, 18.0));
        let tip = hover_tip_rect(token, card, bounds);
        assert!(
            tip.bottom() <= token.top(),
            "the card overlaps the reference"
        );
        assert!(tip.width() + 0.1 >= card.x + 24.0);
        assert!(tip.height() > value.y + 16.0);
    }

    #[test]
    fn turn_on_sync_explains_the_idea_before_setup() {
        let text = crate::sync::intro_lines().join(" ");
        assert!(text.contains("cannot read them"));
        assert!(text.contains("12 recovery words"));
        assert!(text.contains("link another phone"));
        assert!(text.contains("stay as files on this computer"));
        assert!(text.contains("encrypted on this computer"));
        assert!(!text.to_lowercase().contains("paused"));
        assert!(!text.contains("coming soon"));
        assert!(!text.contains("continues in the web app"));
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
        assert!(!new_folder_command(""));
        assert!(!new_folder_command("f"));
        assert!(new_folder_command("ne"));
        assert!(new_folder_command("folder"));
        assert!(!new_folder_command("folders"));
        assert!(!move_folder_command("m"));
        assert!(!rename_command(""));
        assert!(!rename_command("r"));
        assert!(rename_command("re"));
        assert!(rename_command("Rename"));
        assert!(rename_command("rename variable"));
        assert!(!rename_command("renamed"));
        assert!(move_folder_command("mo"));
        assert!(move_folder_command("move to"));
        assert!(!move_folder_command("movie"));
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
            "Weekend Plans",
            FontId::proportional(14.0),
            Color32::WHITE,
            80.0,
        );
        assert_eq!(job.wrap.max_rows, 1);
        assert!(job.wrap.break_anywhere);
        assert_eq!(job.wrap.overflow_character, Some('\u{2026}'));
        assert_eq!(job.text, "Weekend Plans");
    }

    fn idle_pop(value: bool) -> PopState {
        PopState {
            naming: false,
            menu: false,
            prompt: false,
            completion: false,
            value,
            typing: false,
            dialog: false,
        }
    }

    #[test]
    fn only_one_popup_is_visible() {
        let value = idle_pop(true);
        assert_eq!(shown_pop(value), ShownPop::Value);
        assert_eq!(
            shown_pop(PopState {
                typing: true,
                ..value
            }),
            ShownPop::None
        );
        assert_eq!(
            shown_pop(PopState {
                completion: true,
                ..value
            }),
            ShownPop::Completion
        );
        assert_eq!(
            shown_pop(PopState {
                completion: true,
                typing: true,
                ..value
            }),
            ShownPop::Completion
        );
        assert_eq!(
            shown_pop(PopState {
                prompt: true,
                completion: true,
                ..value
            }),
            ShownPop::Prompt
        );
        assert_eq!(
            shown_pop(PopState {
                menu: true,
                prompt: true,
                completion: true,
                ..value
            }),
            ShownPop::Menu
        );
        assert_eq!(
            shown_pop(PopState {
                naming: true,
                menu: true,
                value: true,
                ..value
            }),
            ShownPop::Rename
        );
        assert_eq!(
            shown_pop(PopState {
                dialog: true,
                naming: true,
                value: true,
                ..value
            }),
            ShownPop::None
        );
        assert_eq!(shown_pop(idle_pop(false)), ShownPop::None);
    }

    #[test]
    fn a_name_field_keeps_editing_keys() {
        let editing = vec![
            Event::Text("2".into()),
            Event::Text(" ".into()),
            Event::Text(".".into()),
            Event::Paste("Trip Budget".into()),
            press(Key::A, Modifiers::COMMAND, false),
            press(Key::ArrowLeft, Modifiers::SHIFT, false),
            press(Key::ArrowRight, Modifiers::NONE, false),
            press(Key::Home, Modifiers::NONE, false),
            press(Key::End, Modifiers::NONE, false),
            press(Key::Delete, Modifiers::NONE, false),
            press(Key::Backspace, Modifiers::COMMAND, false),
            Event::Copy,
            Event::Cut,
        ];
        let mut kept = editing.clone();
        assert_eq!(take_name_field_key(&mut kept), None);
        assert_eq!(kept.len(), editing.len());

        // The folder dialog used to feed every key through the switcher, which
        // appends text and drops selection, copy, and cut.
        let mut stolen = editing.clone();
        let edits = take_switcher_edits(&mut stolen);
        assert!(
            stolen.is_empty(),
            "the switcher path removes editing events"
        );
        assert!(
            edits
                .iter()
                .any(|edit| matches!(edit, SwitcherEdit::Type(text) if text == "2"))
        );
        assert!(
            edits
                .iter()
                .any(|edit| matches!(edit, SwitcherEdit::Type(text) if text == " "))
        );
        assert!(
            edits
                .iter()
                .any(|edit| matches!(edit, SwitcherEdit::Type(text) if text == "Trip Budget"))
        );
        assert_eq!(edits.len(), 4, "only the four text inserts survive");

        let mut enter = vec![
            press(Key::Enter, Modifiers::NONE, false),
            Event::Text("a".into()),
        ];
        assert_eq!(take_name_field_key(&mut enter), Some(NameFieldKey::Confirm));
        assert!(matches!(enter.as_slice(), [Event::Text(text)] if text == "a"));

        let mut esc = vec![press(Key::Escape, Modifiers::NONE, false)];
        assert_eq!(take_name_field_key(&mut esc), Some(NameFieldKey::Cancel));
        assert!(esc.is_empty());
    }
}
