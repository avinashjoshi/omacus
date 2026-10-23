//! The rendered page: markdown, with each answer in brackets.
//!
//! This is what Ctrl+P shows, and what Print and Export PDF send out.

use eframe::egui::{self, CornerRadius, FontFamily, FontId, RichText, Sense, Vec2};

use crate::theme::Theme;
use omacus_engine::markdown::{self, Block, Inline, Rich};
use omacus_engine::{Kind, LineResult};

pub enum Action {
    Close,
    Print,
    Export,
    Copy(String),
}

/// The words to put in brackets. Name hints stay in the editor; they aren't part of
/// the page someone would print.
pub fn answers(results: &[LineResult]) -> Vec<Option<String>> {
    results
        .iter()
        .map(|r| match r.kind {
            Kind::Value if !r.display.is_empty() => Some(r.display.clone()),
            Kind::Error if !r.hint => r.note.clone().filter(|n| !n.is_empty()),
            _ => None,
        })
        .collect()
}

pub fn blocks(text: &str, results: &[LineResult]) -> Vec<Block> {
    let owned = answers(results);
    let refs: Vec<Option<&str>> = owned.iter().map(|s| s.as_deref()).collect();
    markdown::render(text, &refs)
}

pub fn show(
    ui: &mut egui::Ui,
    theme: &Theme,
    text: &str,
    results: &[LineResult],
    column: bool,
) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        let edit = ui.add(
            egui::Button::new(RichText::new("Edit").size(13.5).color(theme.background))
                .fill(theme.accent)
                .corner_radius(CornerRadius::same(6))
                .min_size(Vec2::new(0.0, 28.0)),
        );
        if edit.on_hover_text("Back to the page (Esc)").clicked() {
            action = Some(Action::Close);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let export = ui.add(bar_button(theme, "Export PDF"));
            if export.on_hover_text("Save this preview as a PDF").clicked() {
                action = Some(Action::Export);
            }
            let print = ui.add(bar_button(theme, "Print"));
            if print.on_hover_text("Open this preview to print").clicked() {
                action = Some(Action::Print);
            }
        });
    });
    ui.add_space(8.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let owned = answers(results);
            let refs: Vec<Option<&str>> = owned.iter().map(|s| s.as_deref()).collect();
            // A reading measure, centered, so a wide window does not stretch the note.
            let measure = ui.available_width().min(680.0);
            let pad = ((ui.available_width() - measure) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_width(measure);
                    for (n, block) in markdown::render(text, &refs).into_iter().enumerate() {
                        if let Some(copy) = block_ui(ui, theme, n, block, column) {
                            action = Some(Action::Copy(copy));
                        }
                    }
                });
            });
        });
    action
}

fn bar_button<'a>(theme: &'a Theme, label: &'a str) -> egui::Button<'a> {
    egui::Button::new(RichText::new(label).size(13.0).color(theme.foreground))
        .fill(theme.sidebar)
        .corner_radius(CornerRadius::same(6))
        .min_size(Vec2::new(0.0, 28.0))
}

fn block_ui(
    ui: &mut egui::Ui,
    theme: &Theme,
    n: usize,
    block: Block,
    column: bool,
) -> Option<String> {
    let mut copied = None;
    match block {
        Block::Heading { level, rich } => {
            let size = match level {
                1 => 28.0,
                2 => 22.0,
                _ => 18.0,
            };
            copied = line_ui(ui, theme, &rich, size, true, column);
            if level == 1 {
                ui.add_space(6.0);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(36.0, 2.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(1), theme.accent);
            }
            ui.add_space(8.0);
        }
        Block::Lines(rows) => {
            for rich in rows {
                copied = line_ui(ui, theme, &rich, 16.0, false, column).or(copied);
            }
            ui.add_space(6.0);
        }
        Block::Blank => ui.add_space(18.0),
        Block::List { ordered, items } => {
            for (i, item) in items.into_iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let mark = match item.checked {
                        Some(true) => "☑".to_string(),
                        Some(false) => "☐".to_string(),
                        None if ordered => format!("{}.", item.number.unwrap_or((i + 1) as u32)),
                        None => "•".to_string(),
                    };
                    ui.label(RichText::new(mark).size(16.0).color(theme.dim));
                    let width = ui.available_width();
                    ui.allocate_ui_with_layout(
                        Vec2::new(width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            if let Some(text) = line_ui(ui, theme, &item.rich, 16.0, false, column)
                            {
                                copied = Some(text);
                            }
                        },
                    );
                });
            }
            ui.add_space(8.0);
        }
        Block::Quote(rows) => {
            let response = ui
                .horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        for rich in rows {
                            if let Some(text) = line_ui(ui, theme, &rich, 16.0, false, column) {
                                copied = Some(text);
                            }
                        }
                    });
                })
                .response;
            let bar = egui::Rect::from_min_max(
                response.rect.left_top(),
                egui::pos2(response.rect.left() + 3.0, response.rect.bottom()),
            );
            ui.painter()
                .rect_filled(bar, CornerRadius::same(1), theme.accent);
            ui.add_space(8.0);
        }
        Block::Table { header, rows } => {
            egui::Grid::new(("preview-table", n))
                .striped(true)
                .spacing(Vec2::new(16.0, 6.0))
                .show(ui, |ui| {
                    for (i, row) in rows.into_iter().enumerate() {
                        let head = header && i == 0;
                        for cell in row {
                            inlines_ui(ui, theme, &cell, 14.0, head);
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(10.0);
        }
    }
    copied
}

fn line_ui(
    ui: &mut egui::Ui,
    theme: &Theme,
    rich: &Rich,
    size: f32,
    strong: bool,
    column: bool,
) -> Option<String> {
    let mut copied = None;
    let park = column && rich.answer.is_some();
    let answer_w = if park { 150.0 } else { 0.0 };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let left = (ui.available_width() - answer_w).max(48.0);
        ui.allocate_ui_with_layout(
            Vec2::new(left, 0.0),
            egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                inlines_ui(ui, theme, &rich.inlines, size, strong);
                if let Some(comment) = rich.comment.as_deref().filter(|s| !s.is_empty()) {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(comment)
                            .italics()
                            .size((size - 1.0).max(12.0))
                            .color(theme.comment),
                    );
                }
            },
        );
        if let Some(answer) = &rich.answer {
            let mut place = |ui: &mut egui::Ui| {
                let pill = ui.add(pill_button(theme, answer, size));
                if pill.on_hover_text("Click to copy").clicked() {
                    ui.ctx().copy_text(answer.clone());
                    copied = Some(answer.clone());
                }
            };
            if park {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), place);
            } else {
                ui.add_space(8.0);
                place(ui);
            }
        }
    });
    copied
}

fn pill_button<'a>(theme: &'a Theme, answer: &'a str, size: f32) -> egui::Button<'a> {
    egui::Button::new(
        RichText::new(format!("[ {answer} ]"))
            .font(FontId::monospace((size - 1.0).max(12.0)))
            .color(theme.accent),
    )
    .fill(theme.accent.gamma_multiply(0.14))
    .corner_radius(CornerRadius::same(8))
    .min_size(Vec2::new(0.0, size + 4.0))
}

fn inlines_ui(ui: &mut egui::Ui, theme: &Theme, inlines: &[Inline], size: f32, strong: bool) {
    for inline in inlines {
        let (text, bold, italic, code) = match inline {
            Inline::Text(s) => (s.as_str(), strong, false, false),
            Inline::Strong(s) => (s.as_str(), true, false, false),
            Inline::Em(s) => (s.as_str(), strong, true, false),
            Inline::Code(s) => (s.as_str(), false, false, true),
            Inline::Link { text, href } => {
                ui.hyperlink_to(RichText::new(text).size(size).color(theme.accent), href);
                continue;
            }
        };
        let mut rich = RichText::new(text).size(size).color(theme.foreground);
        if bold {
            rich = rich.font(FontId::new(size, FontFamily::Name("bold".into())));
        }
        if italic {
            rich = rich.italics();
        }
        if code {
            rich = rich
                .font(FontId::monospace(size - 1.0))
                .background_color(theme.hover);
        }
        ui.label(rich);
    }
}
