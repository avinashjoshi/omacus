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

use crate::complete;
use crate::engine::{self, Engine, Kind, LineResult};
use crate::help;
use crate::highlight;
use crate::rates::Rates;
use crate::sheets::Store;
use crate::theme::{self, Theme};

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
}

pub struct OmacusApp {
    store: Store,
    results: Vec<LineResult>,
    vars: HashSet<String>,
    engine: Engine,
    rates: Rates,
    rates_seen: u64,
    theme: Theme,
    theme_changed: Arc<AtomicBool>,
    system_dark: bool,
    dirty_since: Option<Instant>,
    copied: Option<(usize, Instant)>,
    evaluated_at: Instant,
    popup: Option<Popup>,
    /// Cursor to apply to the editor after it is next shown.
    pending_cursor: Option<usize>,
    /// Sheets whose saved cursor has been put back this session.
    restored: HashSet<String>,
    focus_editor: bool,
    title: String,
    help: Option<help::Tab>,
}

impl OmacusApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_fonts(&cc.egui_ctx);

        let ctx = cc.egui_ctx.clone();
        let rates = Rates::load(move || ctx.request_repaint());
        let ctx = cc.egui_ctx.clone();
        let theme_changed = theme::watch(move || ctx.request_repaint());

        let system_dark = is_system_dark(&cc.egui_ctx);
        let theme = Theme::load(system_dark);
        apply_visuals(&cc.egui_ctx, &theme);

        let mut app = Self {
            store: Store::load(WELCOME),
            results: Vec::new(),
            vars: HashSet::new(),
            engine: Engine::new(rates.clone()),
            rates_seen: rates.version(),
            rates,
            theme,
            theme_changed,
            system_dark,
            dirty_since: None,
            copied: None,
            evaluated_at: Instant::now(),
            popup: None,
            pending_cursor: None,
            restored: HashSet::new(),
            focus_editor: true,
            title: String::new(),
            help: None,
        };
        app.evaluate();
        app
    }

    fn evaluate(&mut self) {
        let text = &self.store.current().text;
        self.results = self.engine.eval_document(text);
        self.vars = engine::defined_variables(text);
        self.evaluated_at = Instant::now();
    }

    fn touched(&mut self) {
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    fn sheet_switched(&mut self) {
        self.popup = None;
        self.copied = None;
        self.focus_editor = true;
        self.evaluate();
        self.touched();
    }

    fn apply(&mut self, action: SheetAction) {
        match action {
            SheetAction::Select(i) => self.store.select(i),
            SheetAction::New => self.store.new_sheet(""),
            SheetAction::Delete(i) => self.store.delete(i),
            SheetAction::Move(i, step) => {
                self.store.select(i);
                self.store.move_active(step);
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
        let v = self.rates.version();
        // Re-evaluate when fresh rates land, and every minute so `now` stays current.
        if v != self.rates_seen || self.evaluated_at.elapsed() > Duration::from_secs(60) {
            self.rates_seen = v;
            self.evaluate();
        }
        if self.store.has_unsaved() {
            let since = *self.dirty_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= SAVE_DELAY {
                self.store.save();
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
            if i.consume_key(cmd, Key::Delete) {
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
            self.store.save();
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
        let action = help::show(ctx, &mut tab, &self.theme, &self.engine);
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
            None => {}
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
            .max_height(ui.available_height() - 34.0)
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

                    if resp.clicked() {
                        action = Some(SheetAction::Select(i));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("New sheet").clicked() {
                            action = Some(SheetAction::New);
                        }
                        if i > 0 && ui.button("Move up").clicked() {
                            action = Some(SheetAction::Move(i, -1));
                        }
                        if i + 1 < count && ui.button("Move down").clicked() {
                            action = Some(SheetAction::Move(i, 1));
                        }
                        ui.separator();
                        if ui.button("Delete sheet").clicked() {
                            action = Some(SheetAction::Delete(i));
                        }
                    });
                }
            });

        ui.add_space(6.0);
        let help = ui.add(
            egui::Button::new(
                egui::RichText::new(format!("?  Help & shortcuts   {}", help::keys("F1")))
                    .size(12.0)
                    .color(theme.comment),
            )
            .frame(false),
        );
        if help.clicked() {
            self.toggle_help();
        }

        if let Some(a) = action {
            self.apply(a);
        }
    }

    fn sheet(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        let font = FontId::monospace(FONT_SIZE);
        let (row_height, results_width) = ui.ctx().fonts_mut(|f| {
            let widest = self
                .results
                .iter()
                .map(|r| {
                    f.layout_no_wrap(r.display.clone(), font.clone(), Color32::WHITE)
                        .size()
                        .x
                })
                .fold(0.0, f32::max);
            (f.row_height(&font), widest)
        });

        let full = ui.available_width();
        let results_width = results_width.clamp(80.0, full * 0.45);
        let editor_width = (full - results_width - GAP - SCROLLBAR_ROOM).max(120.0);
        let min_rows = (ui.available_height() / row_height).floor().max(1.0) as usize;

        let sheet_id = self.store.current().id.clone();
        let vars = &self.vars;
        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = highlight::layout(buf.as_str(), vars, &theme, font.clone());
            job.wrap.max_width = wrap_width;
            ui.ctx().fonts_mut(|f| f.layout_job(job))
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

        // Paint each result on the first visual row of its line.
        let right = ui.max_rect().right() - SCROLLBAR_ROOM;
        let origin = output.galley_pos;
        let mut line = 0;
        let mut at_line_start = true;
        for row in &output.galley.rows {
            if at_line_start {
                let y = origin.y + row.pos.y;
                self.paint_result(ui, line, right, y, row_height, &font);
            }
            at_line_start = row.ends_with_newline;
            if row.ends_with_newline {
                line += 1;
            }
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
        if result.kind != Kind::Value || result.display.is_empty() {
            return;
        }
        let theme = &self.theme;
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
        let rect =
            Rect::from_min_size(pos2(right - size.x, y), vec2(size.x, h)).expand2(vec2(6.0, 1.0));
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
            .inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("scroll", &self.store.sheets[self.store.active].id))
                .auto_shrink([false, false])
                .show(ui, |ui| self.sheet(ui));
        });

        self.popup(&ctx);
        self.show_help(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.store.save();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme.background.to_normalized_gamma_f32()
    }
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
static BUNDLED_FONT: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
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
    let data = match system_font() {
        Some(bytes) => FontData::from_owned(bytes),
        None => FontData::from_static(BUNDLED_FONT),
    };
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("mono".into(), Arc::new(data));
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "mono".into());
    }
    ctx.set_fonts(fonts);
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
