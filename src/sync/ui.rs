//! Sync screens in this window. Copy matches the web app, and the six
//! desktop dialogs match the approved mockups. Colors come from the theme.

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, vec2};

use crate::design;
use crate::theme::{self, Theme};

use super::session::{CLIPBOARD_NOTE, Screen, Session};

/// Draw the open sync screen. `Some(id)` asks the window to unlock that page
/// with the password already on this computer.
pub fn show(ctx: &egui::Context, theme: &Theme, session: &mut Session) -> Option<String> {
    let mut unlock = None;
    if session.screen != Screen::Closed {
        let frame = design::dialog(theme);
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("omacus-sync"))
            .frame(frame)
            .backdrop_color(design::backdrop(theme))
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.set_max_width(440.0);
                unlock = body(ui, theme, session);
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
        if modal.should_close() {
            close = true;
        }
        if close && unlock.is_none() {
            session.not_now();
        }
    }
    if session.move_pending().is_some() {
        let frame = design::dialog(theme);
        egui::Modal::new(egui::Id::unique("omacus-sync-move"))
            .frame(frame)
            .backdrop_color(design::backdrop(theme))
            .show(ctx, |ui| {
                ui.set_width(440.0);
                move_body(ui, theme, session);
            });
    }
    unlock
}

fn body(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    let screen = session.screen;
    match screen {
        Screen::Intro => intro(ui, theme, session),
        Screen::Choice => choice(ui, theme, session),
        Screen::Start => start(ui, theme, session),
        Screen::Words => words(ui, theme, session),
        Screen::Check => check(ui, theme, session),
        Screen::Ready => ready(ui, theme, session),
        Screen::Home => home(ui, theme, session),
        Screen::Invite => invite(ui, theme, session),
        Screen::Join => join(ui, theme, session),
        Screen::Scan => scan(ui, theme, session),
        Screen::Waiting => waiting(ui, theme, session),
        Screen::Match => matching(ui, theme, session),
        Screen::Approve => approve(ui, theme, session),
        Screen::Recover => recover(ui, theme, session),
        Screen::OffAsk => off(ui, theme, session, false),
        Screen::OffFailed => off(ui, theme, session, true),
        Screen::RemoveAsk => remove_ask(ui, theme, session),
        Screen::Removed => removed(ui, theme, session),
        Screen::PagesHere => pages_here(ui, theme, session),
        Screen::Locked => locked(ui, theme, session),
        Screen::Clash => clash(ui, theme, session),
        Screen::Rename => rename(ui, theme, session),
        Screen::PassSet => passphrase(ui, theme, session, true),
        Screen::PassEnter => passphrase(ui, theme, session, false),
        Screen::Closed => None,
    }
}

fn title(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::token::TYPE_TITLE)
            .color(theme.foreground),
    );
    ui.add_space(8.0);
}

fn para(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::token::TYPE_BODY)
            .color(theme.dim),
    );
    ui.add_space(8.0);
}

fn hint(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::token::TYPE_CAPTION)
            .color(theme.text_muted()),
    );
    ui.add_space(6.0);
}

fn err(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    if text.is_empty() {
        return;
    }
    ui.label(
        RichText::new(text)
            .size(theme::token::TYPE_BODY)
            .color(theme.danger()),
    );
    ui.add_space(8.0);
}

fn progress(ui: &mut egui::Ui, theme: &Theme, step: usize) {
    ui.horizontal(|ui| {
        for i in 1..=4 {
            let fill = if i <= step { theme.accent } else { theme.hover };
            let (rect, _) = ui.allocate_exact_size(vec2(24.0, 4.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(2), fill);
        }
    });
    ui.add_space(8.0);
}

fn back(ui: &mut egui::Ui, theme: &Theme, label: &str) -> bool {
    ui.add(
        egui::Button::new(
            RichText::new(label)
                .size(theme::token::TYPE_BODY)
                .color(theme.dim),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .frame(false),
    )
    .clicked()
}

fn card(ui: &mut egui::Ui, theme: &Theme, glyph: &str, heading: &str, detail: &str) -> bool {
    let frame = egui::Frame::NONE
        .fill(theme.background)
        .stroke(Stroke::new(1.0, theme.border()))
        .corner_radius(CornerRadius::same(theme::token::RADIUS_MD))
        .inner_margin(egui::Margin::same(12));
    let response = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (tile, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    tile,
                    CornerRadius::same(theme::token::RADIUS_MD),
                    theme.surface(),
                );
                ui.painter().text(
                    tile.center(),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    egui::FontId::proportional(theme::token::TYPE_TITLE),
                    theme.accent,
                );
                ui.add_space(12.0);
                let text_width = (ui.available_width() - 28.0).max(40.0);
                ui.vertical(|ui| {
                    ui.set_width(text_width);
                    ui.label(
                        RichText::new(heading)
                            .size(theme::token::TYPE_BODY)
                            .color(theme.foreground)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(detail)
                            .size(theme::token::TYPE_CAPTION)
                            .color(theme.dim),
                    );
                });
                ui.label(
                    RichText::new("›")
                        .size(theme::token::TYPE_TITLE)
                        .color(theme.text_muted()),
                );
            });
        })
        .response;
    ui.add_space(8.0);
    response.interact(egui::Sense::click()).clicked()
}

fn intro(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    title(ui, theme, "How sync works");
    for line in super::intro_lines() {
        para(ui, theme, line);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if design::primary(ui, theme, "Set up sync").clicked() {
            session.begin_setup();
        }
        if design::quiet(ui, theme, "Not now").clicked() {
            session.not_now();
        }
    });
    None
}

fn choice(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    if back(ui, theme, "← Back") {
        session.open_intro();
    }
    title(ui, theme, "Turn on sync");
    para(
        ui,
        theme,
        "End-to-end encrypted. Only your devices can read your pages — not even our server.",
    );
    if card(
        ui,
        theme,
        "✦",
        "This is my first device",
        "Start fresh and keep these pages in sync from here.",
    ) {
        session.choose_first();
    }
    if card(
        ui,
        theme,
        "⇄",
        "My pages already sync on another device",
        "Join with a code from that phone or computer.",
    ) {
        session.choose_join();
    }
    None
}

fn start(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    progress(ui, theme, 1);
    if back(ui, theme, "← Back") {
        session.begin_setup();
    }
    title(ui, theme, "Start fresh on this computer");
    para(
        ui,
        theme,
        "Your pages get encrypted here before they leave. The server stores them but can't read them.",
    );
    field_label(ui, theme, "Name this computer");
    let name = session.name_buf();
    ui.add(
        egui::TextEdit::singleline(name)
            .desired_width(f32::INFINITY)
            .hint_text("Omacus on this computer"),
    );
    hint(
        ui,
        theme,
        "Shown on your other devices so you can tell them apart.",
    );
    field_label(ui, theme, "Invite code");
    let access = session.access_buf();
    ui.add(
        egui::TextEdit::singleline(access)
            .desired_width(f32::INFINITY)
            .hint_text("Code you were given"),
    );
    err(ui, theme, &session.error.clone());
    hint(ui, theme, "omacus sync is invite-only for now.");
    hint(ui, theme, "Not the code from another device.");
    if design::primary(ui, theme, "Continue").clicked() {
        let name = session.name_buf().clone();
        session.set_name(&name);
        let _ = session.continue_access();
    }
    None
}

fn words(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    progress(ui, theme, 2);
    if back(ui, theme, "← Back") {
        session.choose_first();
    }
    title(ui, theme, "Save your recovery words");
    para(
        ui,
        theme,
        "These 12 words are the only way back in if you lose every device. Omacus never keeps a copy, and you won't see them again.",
    );
    if let Some(grid) = session.words_grid().cloned() {
        egui::Grid::new("sync-words").num_columns(3).show(ui, |ui| {
            for (i, word) in grid.iter().enumerate() {
                ui.label(
                    RichText::new(format!("{}. {word}", i + 1))
                        .size(theme::token::TYPE_BODY)
                        .color(theme.foreground)
                        .monospace(),
                );
                if (i + 1) % 3 == 0 {
                    ui.end_row();
                }
            }
        });
    }
    ui.add_space(8.0);
    let mut wrote = session.wrote();
    if ui
        .checkbox(
            &mut wrote,
            "I wrote these down or printed them somewhere safe.",
        )
        .changed()
    {
        session.set_wrote(wrote);
    }
    ui.add_space(8.0);
    err(ui, theme, &session.error.clone());
    ui.horizontal(|ui| {
        ui.add_enabled_ui(wrote, |ui| {
            if design::primary(ui, theme, "I saved them").clicked() {
                session.go_check();
            }
        });
        if design::quiet(ui, theme, "Print").clicked()
            && let Err(error) = session.print_words()
        {
            session.error = error;
        }
        if design::quiet(ui, theme, "Copy").clicked()
            && let Err(error) = session.copy_words()
        {
            session.error = error;
        }
    });
    ui.add_space(8.0);
    hint(ui, theme, CLIPBOARD_NOTE);
    None
}

fn check(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    progress(ui, theme, 3);
    if back(ui, theme, "← Back") {
        session.screen = Screen::Words;
    }
    let asks = session.asks();
    title(ui, theme, "Do these match your paper?");
    para(
        ui,
        theme,
        &format!(
            "Pick word {}, word {}, and word {}.",
            asks[0], asks[1], asks[2]
        ),
    );
    err(ui, theme, &session.error.clone());
    for (which, n) in asks.into_iter().enumerate() {
        ui.label(
            RichText::new(format!("Word {n}"))
                .size(theme::token::TYPE_LABEL)
                .color(theme.foreground),
        );
        let choices = session.choice_list(which).to_vec();
        ui.horizontal_wrapped(|ui| {
            for word in choices {
                if design::quiet(ui, theme, &word).clicked() {
                    session.pick(which, &word);
                }
            }
        });
        ui.add_space(6.0);
    }
    if design::primary(ui, theme, "These match").clicked() {
        let _ = session.confirm();
    }
    None
}

fn pages_here(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    if !session.lock_settled() && session.first_locked().is_some() {
        session.screen = Screen::Locked;
        return locked(ui, theme, session);
    }
    title(ui, theme, "Pages already on this computer");
    para(
        ui,
        theme,
        "These pages are files on this computer. You can bring them into sync, or leave them here.",
    );
    if card(
        ui,
        theme,
        "↑",
        "Bring these pages into sync",
        "They stay as files here, and your other phones get a copy. Nothing is deleted.",
    ) {
        let _ = session.bring();
    }
    if card(
        ui,
        theme,
        "·",
        "Leave them on this computer",
        "Sync starts empty. These files stay where they are, and they are not uploaded.",
    ) {
        let _ = session.leave_files();
    }
    if back(ui, theme, "← Back") {
        session.screen = Screen::Check;
    }
    err(ui, theme, &session.error.clone());
    None
}

fn locked(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    let heading = session.locked_title();
    title(ui, theme, &format!("{heading} is locked"));
    para(
        ui,
        theme,
        "This page is locked with a password on this computer. That password is not a sync key. Unlock it to bring it into sync, or leave it here. The password never leaves this computer.",
    );
    let mut unlock = None;
    ui.horizontal(|ui| {
        if design::primary(ui, theme, "Unlock").clicked() {
            unlock = session.first_locked().map(str::to_string);
            session.close_screen();
        }
        if design::quiet(ui, theme, "Leave it here").clicked() {
            session.leave_locked();
        }
    });
    unlock
}

fn clash(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    let title_text = session
        .clash()
        .map(|clash| clash.title.clone())
        .unwrap_or_else(|| "This page".into());
    title(ui, theme, &format!("{title_text} is on both sides"));
    para(
        ui,
        theme,
        "One is a file on this computer. One is already in your synced pages. Both stay. Nothing is deleted.",
    );
    para(
        ui,
        theme,
        &format!(
            "The file here will be called {title_text}, this computer, so you can tell them apart."
        ),
    );
    ui.horizontal(|ui| {
        if design::primary(ui, theme, "Keep both").clicked() {
            let _ = session.keep_both();
        }
        if design::quiet(ui, theme, "Leave this file here").clicked() {
            let _ = session.leave_clash();
        }
    });
    err(ui, theme, &session.error.clone());
    None
}

fn scan(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    title(ui, theme, "Scan the code");
    para(
        ui,
        theme,
        "Point the camera at the picture on your other phone or computer.",
    );
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 220.0), egui::Sense::hover());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(theme::token::RADIUS_MD),
        theme.background,
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(theme::token::RADIUS_MD),
        Stroke::new(1.0, theme.border()),
        egui::StrokeKind::Inside,
    );
    let finder = egui::Rect::from_center_size(rect.center(), vec2(140.0, 140.0));
    ui.painter().rect_stroke(
        finder,
        CornerRadius::same(theme::token::RADIUS_MD),
        Stroke::new(2.0, theme.accent),
        egui::StrokeKind::Inside,
    );
    ui.add_space(8.0);
    para(
        ui,
        theme,
        "If this computer has no camera, or you say no, type the code instead.",
    );
    if back(ui, theme, "Type the code instead") {
        session.type_instead();
    }
    None
}

fn ready(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    progress(ui, theme, 4);
    title(ui, theme, "Sync is on");
    para(
        ui,
        theme,
        "Your pages are encrypted and saved. Add another phone or computer to keep them in step.",
    );
    devices(ui, theme, session);
    if design::primary(ui, theme, "Add a phone or computer").clicked() {
        let _ = session.start_invite();
    }
    ui.horizontal(|ui| {
        if design::quiet(ui, theme, "Done").clicked() {
            session.done();
        }
        if design::quiet(ui, theme, "Turn sync off").clicked() {
            session.ask_off();
        }
    });
    None
}

fn home(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    if back(ui, theme, "← Back") {
        session.done();
    }
    session.observe_list();
    title(ui, theme, "Sync is on");
    let list_note = session.list_note_text();
    if !list_note.is_empty() {
        para(ui, theme, list_note);
        if design::quiet(ui, theme, "Got it").clicked() {
            session.ack_list();
        }
    }
    if !session.notice.is_empty() {
        para(ui, theme, &session.notice.clone());
    }
    para(ui, theme, "Your pages stay on your phones and computers.");
    para(
        ui,
        theme,
        "A page that left the list is in Recently deleted. Restore puts it back.",
    );
    err(ui, theme, &session.error.clone());
    field_label(ui, theme, "Name for these pages");
    let vault = session.vault_buf();
    ui.add(egui::TextEdit::singleline(vault).desired_width(f32::INFINITY));
    let vault = session.vault_buf().clone();
    session.set_vault_name(&vault);
    devices(ui, theme, session);
    if design::primary(ui, theme, "Add a phone or computer").clicked() {
        let _ = session.start_invite();
    }
    if design::quiet(ui, theme, "Turn sync off").clicked() {
        session.ask_off();
    }
    None
}

fn devices(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) {
    let rows = session.devices();
    for row in rows {
        ui.horizontal(|ui| {
            if row.here {
                let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(rect.center(), 4.0, theme.defined);
            }
            let name = if row.here {
                format!("{} · this computer", row.name)
            } else {
                row.name.clone()
            };
            ui.label(
                RichText::new(name)
                    .size(theme::token::TYPE_BODY)
                    .color(theme.foreground),
            );
            if design::quiet(ui, theme, "Rename").clicked() {
                session.begin_rename(&row.id);
            }
            if !row.here && design::quiet(ui, theme, "Remove").clicked() {
                session.ask_remove(&row.id);
            }
        });
    }
    ui.add_space(8.0);
}

fn invite(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    if back(ui, theme, "← Back") {
        session.show_home();
    }
    title(ui, theme, "Add a phone or computer");
    para(
        ui,
        theme,
        "On the other phone or computer, open Sync and choose My pages already sync on another device. Type this code. It works for about 5 minutes, and only once.",
    );
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(session.invite_code())
                .size(theme::token::TYPE_DISPLAY)
                .color(theme.foreground)
                .monospace(),
        );
        if design::quiet(ui, theme, "Copy").clicked()
            && let Err(error) = session.copy_invite()
        {
            session.error = error;
        }
    });
    err(ui, theme, &session.error.clone());
    hint(ui, theme, "It looks like K7QM-4R2P-9HNX.");
    if let Ok(modules) = session.invite_modules() {
        let side = (modules.len() as f32).sqrt() as usize;
        let cell = 4.0;
        let (rect, _) = ui.allocate_exact_size(
            vec2(side as f32 * cell, side as f32 * cell),
            egui::Sense::hover(),
        );
        for y in 0..side {
            for x in 0..side {
                if modules.get(y * side + x) == Some(&true) {
                    let r = egui::Rect::from_min_size(
                        rect.min + vec2(x as f32 * cell, y as f32 * cell),
                        vec2(cell, cell),
                    );
                    ui.painter()
                        .rect_filled(r, CornerRadius::ZERO, theme.foreground);
                }
            }
        }
    }
    ui.add_space(8.0);
    para(ui, theme, "Or scan this picture");
    None
}

fn join(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    if back(ui, theme, "← Back") {
        session.begin_setup();
    }
    title(ui, theme, "Join your synced pages");
    para(
        ui,
        theme,
        "On a device that already syncs, open Sync and choose Add a phone or computer. Scan the QR code it shows, or type the code.",
    );
    field_label(ui, theme, "Join code");
    let mut scan = false;
    ui.horizontal(|ui| {
        let code = session.join_buf();
        ui.add(
            egui::TextEdit::singleline(code)
                .desired_width(220.0)
                .hint_text("K7QM-4R2P-9HNX"),
        );
        scan = design::quiet(ui, theme, "Scan QR").clicked();
    });
    if scan {
        session.open_scan();
    }
    err(ui, theme, &session.error.clone());
    hint(
        ui,
        theme,
        "12 characters · or tap the scan button to scan the QR code",
    );
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Lost your other devices?")
                .size(theme::token::TYPE_BODY)
                .color(theme.dim),
        );
        if ui
            .add(
                egui::Button::new(
                    RichText::new("Use recovery words →")
                        .size(theme::token::TYPE_BODY)
                        .color(theme.accent),
                )
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::NONE)
                .frame(false),
            )
            .clicked()
        {
            session.choose_recover();
        }
    });
    ui.add_space(8.0);
    field_label(ui, theme, "Name this computer");
    let name = session.name_buf();
    ui.add(
        egui::TextEdit::singleline(name)
            .desired_width(f32::INFINITY)
            .hint_text("Omacus on this computer"),
    );
    hint(
        ui,
        theme,
        "Shown on your other devices so you can tell them apart.",
    );
    para(
        ui,
        theme,
        "Pages already on this computer will be combined with your synced pages. Nothing is deleted.",
    );
    if design::primary(ui, theme, "Join").clicked() {
        let code = session.join_buf().clone();
        let name = session.name_buf().clone();
        session.set_name(&name);
        let _ = session.submit_join(&code);
    }
    None
}

fn waiting(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    title(ui, theme, "Approve on your other device");
    para(
        ui,
        theme,
        "The same six numbers should be on that phone or computer. Tap Yes there only if they match.",
    );
    ui.label(
        RichText::new(session.numbers())
            .size(theme::token::TYPE_DISPLAY)
            .color(theme.foreground)
            .monospace(),
    );
    para(ui, theme, "Waiting for that phone or computer");
    if design::quiet(ui, theme, "Cancel").clicked() {
        session.not_now();
    }
    None
}

fn matching(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    title(ui, theme, "Do these match?");
    para(
        ui,
        theme,
        "The same six numbers and the same four words should be on your other phone or computer.",
    );
    ui.label(
        RichText::new(session.numbers())
            .size(theme::token::TYPE_DISPLAY)
            .color(theme.foreground)
            .monospace(),
    );
    para(ui, theme, &session.four_words());
    ui.horizontal(|ui| {
        if design::primary(ui, theme, "These match").clicked() {
            let _ = session.accept_match();
        }
        if design::quiet(ui, theme, "Cancel").clicked() {
            session.not_now();
        }
    });
    err(ui, theme, &session.error.clone());
    None
}

fn approve(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    let name = session.pending_label().to_string();
    let heading = if name.is_empty() {
        "Is this your phone or computer?".to_string()
    } else {
        format!("Is this {name}?")
    };
    title(ui, theme, &heading);
    para(
        ui,
        theme,
        "It wants to open your pages. The same six numbers and the same four words should be on that screen. Those numbers only check this step. Tap Yes, approve it only if they match.",
    );
    ui.label(
        RichText::new(session.numbers())
            .size(theme::token::TYPE_DISPLAY)
            .color(theme.foreground)
            .monospace(),
    );
    para(ui, theme, &session.four_words());
    err(ui, theme, &session.error.clone());
    ui.horizontal(|ui| {
        if design::primary(ui, theme, "Yes, approve it").clicked() {
            let _ = session.approve();
        }
        if design::quiet(ui, theme, "No").clicked() {
            let _ = session.deny();
        }
    });
    None
}

fn recover(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    if back(ui, theme, "← Back") {
        session.choose_join();
    }
    title(ui, theme, "Use your recovery words");
    field_label(ui, theme, "Name this computer");
    let name = session.name_buf();
    ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
    para(
        ui,
        theme,
        "Type or paste the 12 recovery words. Numbers and capitals are fine. A page that is not in the list is in Recently deleted. Restore puts it back.",
    );
    field_label(ui, theme, "Recovery words");
    let phrase = session.phrase_buf();
    ui.add(
        egui::TextEdit::multiline(phrase)
            .desired_width(f32::INFINITY)
            .desired_rows(4),
    );
    err(ui, theme, &session.error.clone());
    if design::primary(ui, theme, "Open my pages").clicked() {
        let name = session.name_buf().clone();
        session.set_name(&name);
        let _ = session.recover();
    }
    None
}

fn off(ui: &mut egui::Ui, theme: &Theme, session: &mut Session, failed: bool) -> Option<String> {
    if back(ui, theme, "← Back") {
        session.show_home();
    }
    title(ui, theme, "Turn sync off on this computer?");
    para(
        ui,
        theme,
        "Your pages stay here. This computer leaves the list, so the other phones and computers stop showing it. If this is the last one, that is fine. The recovery words still open the pages.",
    );
    err(ui, theme, &session.error.clone());
    ui.horizontal(|ui| {
        let label = if failed { "Try again" } else { "Turn sync off" };
        if design::primary(ui, theme, label).clicked() {
            let _ = session.turn_off();
        }
        if failed && design::quiet(ui, theme, "Just turn off here").clicked() {
            session.turn_off_here();
        }
        if design::quiet(ui, theme, "Not now").clicked() {
            session.show_home();
        }
    });
    None
}

fn remove_ask(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    let id = session.removing().to_string();
    let name = session
        .devices()
        .into_iter()
        .find(|row| row.id == id)
        .map(|row| row.name)
        .unwrap_or_else(|| "this phone or computer".into());
    let heading = if name == "Phone or computer" {
        "Remove this phone or computer?".to_string()
    } else {
        format!("Remove {name}?")
    };
    title(ui, theme, &heading);
    para(
        ui,
        theme,
        "It stops getting new pages. Pages it already saved stay readable on it. New edits stay with the phones and computers that are still linked.",
    );
    ui.horizontal(|ui| {
        if design::danger(ui, theme, "Remove").clicked() {
            let _ = session.remove_device(&id);
        }
        if design::quiet(ui, theme, "Keep it").clicked() {
            session.show_home();
        }
    });
    err(ui, theme, &session.error.clone());
    None
}

fn removed(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    title(ui, theme, "This computer was removed from sync");
    para(ui, theme, "Your pages are still here.");
    if design::primary(ui, theme, "Join again").clicked() {
        session.choose_join();
    }
    None
}

fn passphrase(
    ui: &mut egui::Ui,
    theme: &Theme,
    session: &mut Session,
    setting: bool,
) -> Option<String> {
    if back(ui, theme, "← Back") {
        session.back_from_pass();
        return None;
    }
    if setting {
        title(ui, theme, "Choose a passphrase");
        para(
            ui,
            theme,
            "This computer keeps the sync key in a file locked with a passphrase. You type it when sync starts here. Omacus does not keep the passphrase.",
        );
    } else {
        title(ui, theme, "Enter the passphrase");
        para(
            ui,
            theme,
            "Type the passphrase to open sync on this computer.",
        );
    }
    err(ui, theme, &session.error.clone());
    field_label(ui, theme, "Passphrase");
    let pass = session.pass_buf();
    ui.add(
        egui::TextEdit::singleline(pass)
            .password(true)
            .desired_width(f32::INFINITY)
            .hint_text(if setting { "At least 8 characters" } else { "" }),
    );
    ui.add_space(8.0);
    if setting {
        field_label(ui, theme, "Type it again");
        let again = session.pass_again();
        ui.add(
            egui::TextEdit::singleline(again)
                .password(true)
                .desired_width(f32::INFINITY),
        );
        ui.add_space(8.0);
    }
    let label = if setting { "Continue" } else { "Open sync" };
    if design::primary(ui, theme, label).clicked() {
        let _ = session.submit_pass();
    }
    None
}

fn rename(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) -> Option<String> {
    title(ui, theme, "Rename");
    let text = session.rename_buf();
    ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if design::primary(ui, theme, "Save").clicked() {
            let name = session.rename_buf().clone();
            let _ = session.save_rename(&name);
        }
        if design::quiet(ui, theme, "Cancel").clicked() {
            session.show_home();
        }
    });
    None
}

fn move_body(ui: &mut egui::Ui, theme: &Theme, session: &mut Session) {
    let inside = session.move_pending().is_some_and(|ask| ask.inside);
    let title_text = session
        .move_pending()
        .map(|ask| ask.title.clone())
        .unwrap_or_default();
    let name = session.vault_name().to_string();
    if inside {
        title(
            ui,
            theme,
            &format!("Move \u{201c}{title_text}\u{201d} into {name}?"),
        );
        para(
            ui,
            theme,
            &format!(
                "It joins {name}. Other phones and computers get this copy. If they already have it, it stays one page. Nothing is deleted. You can move it back."
            ),
        );
    } else {
        title(
            ui,
            theme,
            &format!("Move \u{201c}{title_text}\u{201d} out of {name}?"),
        );
        para(
            ui,
            theme,
            &format!(
                "It stays on this computer. Other phones and computers take it out of {name}. The copy already there stays there and will not get later changes. Nothing is deleted. You can move it back."
            ),
        );
    }
    ui.horizontal(|ui| {
        if design::primary(ui, theme, "Move").clicked() {
            let _ = session.confirm_move();
        }
        if design::quiet(ui, theme, "Not now").clicked() {
            session.cancel_move();
        }
    });
}

fn field_label(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::token::TYPE_CAPTION)
            .color(theme.text_muted()),
    );
}
