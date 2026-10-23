//! Frames, buttons and widget visuals shared by every screen.
//! Colors, type, spacing and radii come from [`crate::theme`]. See `docs/DESIGN.md`.

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke, vec2};

use crate::theme::{self, Theme};

/// A soft drop shadow. Dialogs and popups use it; the editor does not.
pub fn shadow(theme: &Theme) -> egui::Shadow {
    egui::Shadow {
        offset: [0, 4],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(if theme.dark { 80 } else { 28 }),
    }
}

/// Scrim behind a modal. One strength for every dialog.
pub fn backdrop(theme: &Theme) -> Color32 {
    Color32::from_black_alpha(if theme.dark { 160 } else { 96 })
}

/// A modal: surface fill, hairline border, 12px radius, standard padding.
pub fn dialog(theme: &Theme) -> Frame {
    dialog_pad(theme, Margin::symmetric(20, 16))
}

pub fn dialog_pad(theme: &Theme, margin: impl Into<Margin>) -> Frame {
    Frame::NONE
        .fill(theme.surface())
        .stroke(Stroke::new(1.0, theme.border()))
        .corner_radius(CornerRadius::same(theme::token::RADIUS_LG))
        .inner_margin(margin)
        .shadow(shadow(theme))
}

/// A popup or menu card. 8px radius. Callers keep their own padding.
pub fn popup(theme: &Theme) -> Frame {
    popup_pad(theme, Margin::same(8))
}

pub fn popup_pad(theme: &Theme, margin: impl Into<Margin>) -> Frame {
    Frame::NONE
        .fill(theme.surface())
        .stroke(Stroke::new(1.0, theme.border()))
        .corner_radius(CornerRadius::same(theme::token::RADIUS_MD))
        .inner_margin(margin)
        .shadow(shadow(theme))
}

/// A focused text field: accent ring, not a selection-colored stroke.
pub fn field(theme: &Theme) -> Frame {
    Frame::NONE
        .fill(theme.surface())
        .stroke(Stroke::new(1.5, theme.accent))
        .corner_radius(CornerRadius::same(theme::token::RADIUS_MD))
        .inner_margin(Margin::same(8))
}

fn label(text: &str, color: Color32) -> RichText {
    RichText::new(text)
        .size(theme::token::TYPE_BODY)
        .color(color)
}

pub fn primary(ui: &mut egui::Ui, theme: &Theme, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(label(text, theme.on_accent()))
            .fill(theme.accent)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(theme::token::RADIUS_MD))
            .min_size(vec2(0.0, theme::token::HIT)),
    )
}

pub fn quiet(ui: &mut egui::Ui, theme: &Theme, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(label(text, theme.text()))
            .fill(theme.background)
            .stroke(Stroke::new(1.0, theme.border()))
            .corner_radius(CornerRadius::same(theme::token::RADIUS_MD))
            .min_size(vec2(0.0, theme::token::HIT)),
    )
}

pub fn danger(ui: &mut egui::Ui, theme: &Theme, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(label(text, theme.ink_on(theme.danger())))
            .fill(theme.danger())
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(theme::token::RADIUS_MD))
            .min_size(vec2(0.0, theme::token::HIT)),
    )
}

/// A 36×22 switch inside a 44×32 hit target. `on` is the value shown.
pub fn toggle(ui: &mut egui::Ui, theme: &Theme, on: &mut bool) -> egui::Response {
    let (rect, mut resp) =
        ui.allocate_exact_size(vec2(44.0, theme::token::HIT), egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let track = egui::Rect::from_center_size(rect.center(), vec2(36.0, 22.0));
    let radius = CornerRadius::same(11);
    let fill = if *on { theme.accent } else { theme.hover };
    ui.painter().rect_filled(track, radius, fill);
    ui.painter().rect_stroke(
        track,
        radius,
        Stroke::new(1.0, if *on { theme.accent } else { theme.border() }),
        egui::StrokeKind::Inside,
    );
    let knob_x = if *on {
        track.right() - 11.0
    } else {
        track.left() + 11.0
    };
    let knob = if *on { theme.on_accent() } else { theme.text() };
    ui.painter()
        .circle_filled(egui::pos2(knob_x, track.center().y), 7.0, knob);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn radius_row() -> CornerRadius {
    CornerRadius::same(theme::token::RADIUS_MD)
}

/// Widget colors for egui's built-in controls (combo boxes, text edits, menus).
/// Does not change item spacing: that would shift the editor and the sidebar.
pub fn apply(ctx: &egui::Context, theme: &Theme) {
    let mut visuals = if theme.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = theme.background;
    visuals.window_fill = theme.surface();
    visuals.extreme_bg_color = theme.background;
    visuals.faint_bg_color = theme.hover;
    visuals.code_bg_color = theme.surface();
    visuals.override_text_color = Some(theme.text());
    visuals.selection.bg_fill = theme.selection;
    visuals.selection.stroke = Stroke::new(1.0, theme.accent);
    visuals.hyperlink_color = theme.accent;
    visuals.error_fg_color = theme.danger();
    visuals.warn_fg_color = theme.warning();
    visuals.window_corner_radius = CornerRadius::same(theme::token::RADIUS_LG);
    visuals.menu_corner_radius = radius_row();
    visuals.window_stroke = Stroke::new(1.0, theme.border());
    visuals.window_shadow = shadow(theme);
    visuals.popup_shadow = shadow(theme);
    visuals.text_cursor.stroke.color = theme.accent;
    visuals.text_cursor.stroke.width = 2.0;

    let radius = radius_row();
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
        widget.fg_stroke.color = theme.text();
        widget.bg_stroke = Stroke::new(1.0, theme.border());
    }
    visuals.widgets.noninteractive.fg_stroke.color = theme.text_secondary();
    visuals.widgets.inactive.weak_bg_fill = theme.background;
    visuals.widgets.inactive.bg_fill = theme.background;
    visuals.widgets.hovered.weak_bg_fill = theme.hover;
    visuals.widgets.hovered.bg_fill = theme.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, theme.border_strong());
    visuals.widgets.active.weak_bg_fill = theme.hover;
    visuals.widgets.active.bg_fill = theme.selection;
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, theme.accent);
    visuals.widgets.open.weak_bg_fill = theme.surface();
    visuals.widgets.open.bg_fill = theme.surface();
    ctx.set_visuals(visuals);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_follow_the_radius_scale() {
        let theme = Theme::dark();
        assert_eq!(
            dialog(&theme).corner_radius,
            CornerRadius::same(theme::token::RADIUS_LG)
        );
        assert_eq!(
            popup(&theme).corner_radius,
            CornerRadius::same(theme::token::RADIUS_MD)
        );
        assert_eq!(
            field(&theme).stroke.color,
            theme.accent,
            "a focused field uses the accent ring"
        );
        assert_ne!(dialog(&theme).stroke.color, theme.selection);
    }
}
