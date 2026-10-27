//! Sync, drawn in the Settings window with the same cards and buttons.
//! Demo copy only. Nothing here talks to a mailbox.

use eframe::egui::{self, RichText};

use crate::design;
use crate::settings;
use crate::theme::{self, Theme};

const WORDS: [&str; 12] = [
    "harbor", "ferry", "oats", "bread", "cake", "pantry", "kept", "meadow", "lantern", "river",
    "cedar", "maple",
];

#[cfg(test)]
fn screens() -> &'static [&'static str] {
    &[
        "intro",
        "fresh",
        "words",
        "check",
        "join-words",
        "join-wait",
        "approve",
        "approve-timeout",
        "devices",
        "remove",
        "paused",
        "list-changed",
        "keyring",
        "off",
    ]
}

pub fn show(ui: &mut egui::Ui, theme: &Theme) {
    let which = std::env::var("OMACUS_SYNC_MOCK").unwrap_or_else(|_| "intro".into());
    match which.as_str() {
        "fresh" => fresh(ui, theme),
        "words" => words(ui, theme),
        "check" => check(ui, theme),
        "join-words" => join_words(ui, theme),
        "join-wait" => join_wait(ui, theme),
        "approve" => approve(ui, theme, false),
        "approve-timeout" => approve(ui, theme, true),
        "devices" => devices(ui, theme, false),
        "remove" => remove(ui, theme),
        "paused" => paused(ui, theme),
        "list-changed" => devices(ui, theme, true),
        "keyring" => keyring(ui, theme),
        "off" => turn_off(ui, theme),
        _ => intro(ui, theme),
    }
}

fn intro(ui: &mut egui::Ui, theme: &Theme) {
    settings::group(ui, theme, "How sync works", |ui| {
        for line in super::intro_lines() {
            settings::caption(ui, theme, line);
            ui.add_space(theme::token::SPACE_2);
        }
        row(
            ui,
            theme,
            &[("Set up sync", Btn::Primary), ("Not now", Btn::Quiet)],
        );
    });
}

fn fresh(ui: &mut egui::Ui, theme: &Theme) {
    settings::group(ui, theme, "Start fresh on this computer", |ui| {
        settings::caption(
            ui,
            theme,
            "Step 1 of 4. Your pages get encrypted here before they leave. The server stores them but can't read them.",
        );
        ui.add_space(theme::token::SPACE_3);
        field(ui, "Name this computer", "Harbor desk");
        settings::caption(
            ui,
            theme,
            "Shown on your other devices so you can tell them apart.",
        );
        ui.add_space(theme::token::SPACE_3);
        field(ui, "Invite code", "");
        settings::caption(
            ui,
            theme,
            "Code you were given. Not the code from another device.",
        );
        ui.add_space(theme::token::SPACE_3);
        row(
            ui,
            theme,
            &[("Continue", Btn::Primary), ("Back", Btn::Quiet)],
        );
    });
}

fn words(ui: &mut egui::Ui, theme: &Theme) {
    settings::group(ui, theme, "Save your recovery words", |ui| {
        settings::caption(
            ui,
            theme,
            "Step 2 of 4. These 12 words are the only way back in if you lose every device. Omacus never keeps a copy, and you won't see them again.",
        );
        ui.add_space(theme::token::SPACE_3);
        word_grid(ui, theme);
        ui.add_space(theme::token::SPACE_3);
        let mut wrote = true;
        settings::labeled(
            ui,
            theme,
            "I wrote these down or printed them somewhere safe.",
            "A clipboard history may keep these words. Printing them, or writing them down, is safest.",
            |ui| {
                design::toggle(ui, theme, &mut wrote);
            },
        );
        ui.add_space(theme::token::SPACE_3);
        row(
            ui,
            theme,
            &[
                ("I saved them", Btn::Primary),
                ("Print", Btn::Quiet),
                ("Copy", Btn::Quiet),
                ("Back", Btn::Quiet),
            ],
        );
    });
}

fn check(ui: &mut egui::Ui, theme: &Theme) {
    settings::group(ui, theme, "Do these match your paper?", |ui| {
        settings::caption(ui, theme, "Step 3 of 4. Pick word 3, word 7, and word 11.");
        ui.add_space(theme::token::SPACE_3);
        ask(
            ui,
            theme,
            "Word 3",
            &["oats", "bread", "ferry", "cake", "harbor", "pantry"],
        );
        ask(
            ui,
            theme,
            "Word 7",
            &["kept", "meadow", "maple", "cedar", "river", "lantern"],
        );
        ask(
            ui,
            theme,
            "Word 11",
            &["cedar", "maple", "oats", "bread", "ferry", "cake"],
        );
        row(
            ui,
            theme,
            &[("These match", Btn::Primary), ("Back", Btn::Quiet)],
        );
    });
}

fn join_words(ui: &mut egui::Ui, theme: &Theme) {
    settings::group(ui, theme, "Use your recovery words", |ui| {
        field(ui, "Name this computer", "Harbor desk");
        ui.add_space(theme::token::SPACE_2);
        settings::caption(
            ui,
            theme,
            "Type or paste the 12 recovery words. Numbers and capitals are fine.",
        );
        ui.add_space(theme::token::SPACE_2);
        let mut phrase =
            "harbor ferry oats bread cake pantry kept meadow lantern river cedar maple".to_string();
        ui.add(
            egui::TextEdit::multiline(&mut phrase)
                .desired_width(f32::INFINITY)
                .desired_rows(3)
                .interactive(false),
        );
        ui.add_space(theme::token::SPACE_2);
        settings::caption(
            ui,
            theme,
            "Pages already on this computer will be combined with your synced pages. Nothing is deleted.",
        );
        ui.add_space(theme::token::SPACE_3);
        row(
            ui,
            theme,
            &[("Open my pages", Btn::Primary), ("Back", Btn::Quiet)],
        );
    });
}

fn join_wait(ui: &mut egui::Ui, theme: &Theme) {
    settings::group(ui, theme, "Approve on your other device", |ui| {
        settings::caption(
            ui,
            theme,
            "The same six numbers should be on that phone or computer. Tap Yes there only if they match.",
        );
        ui.add_space(theme::token::SPACE_3);
        code(ui, theme, "482 916");
        settings::caption(ui, theme, "Waiting for that phone or computer.");
        ui.add_space(theme::token::SPACE_3);
        row(ui, theme, &[("Cancel", Btn::Quiet)]);
    });
}

fn approve(ui: &mut egui::Ui, theme: &Theme, timed_out: bool) {
    settings::group(ui, theme, "Is this Harbor phone?", |ui| {
        settings::caption(
            ui,
            theme,
            "It wants to open your pages. The same six numbers and the same four words should be on that screen. Tap Yes, approve it only if they match.",
        );
        ui.add_space(theme::token::SPACE_3);
        code(ui, theme, "482 916");
        ui.label(
            RichText::new("oats  ferry  bread  cake")
                .size(theme::token::TYPE_TITLE)
                .color(theme.text()),
        );
        ui.add_space(theme::token::SPACE_3);
        if timed_out {
            ui.label(
                RichText::new("That took too long. Your pages are still here.")
                    .size(theme::token::TYPE_BODY)
                    .color(theme.danger()),
            );
            ui.add_space(theme::token::SPACE_3);
        }
        row(
            ui,
            theme,
            &[("Yes, approve it", Btn::Primary), ("No", Btn::Quiet)],
        );
    });
}

fn devices(ui: &mut egui::Ui, theme: &Theme, notice: bool) {
    if notice {
        settings::group(ui, theme, "List changed", |ui| {
            let note = egui::Button::new(
                RichText::new("Harbor phone is now on the list.")
                    .size(theme::token::TYPE_BODY)
                    .color(theme.warning()),
            )
            .fill(theme.surface())
            .stroke(egui::Stroke::new(1.0, theme.warning()))
            .corner_radius(egui::CornerRadius::same(theme::token::RADIUS_MD))
            .min_size(egui::vec2(ui.available_width(), theme::token::HIT));
            let _ = ui.add_sized([ui.available_width(), theme::token::HIT], note);
            ui.add_space(theme::token::SPACE_2);
            settings::caption(
                ui,
                theme,
                "Sync is still on. This note does not stop pages from saving.",
            );
            ui.add_space(theme::token::SPACE_3);
            row(ui, theme, &[("Got it", Btn::Quiet)]);
        });
        ui.add_space(theme::token::SPACE_3);
    }
    settings::group(ui, theme, "My pages", |ui| {
        settings::caption(
            ui,
            theme,
            "Live. Pages keep saving while this list is open.",
        );
        ui.add_space(theme::token::SPACE_3);
        device(ui, theme, "Harbor desk", "this computer", false);
        device(ui, theme, "Harbor phone", "phone", true);
        ui.add_space(theme::token::SPACE_3);
        row(
            ui,
            theme,
            &[
                ("Add a phone or computer", Btn::Primary),
                ("Turn sync off", Btn::Quiet),
            ],
        );
    });
}

fn remove(ui: &mut egui::Ui, theme: &Theme) {
    settings::group(ui, theme, "Remove Harbor phone?", |ui| {
        settings::caption(
            ui,
            theme,
            "It stops getting new pages. Pages it already saved stay readable on it. New edits stay with the phones and computers that are still linked.",
        );
        ui.add_space(theme::token::SPACE_3);
        row(
            ui,
            theme,
            &[("Remove", Btn::Danger), ("Keep it", Btn::Quiet)],
        );
    });
}

fn paused(ui: &mut egui::Ui, theme: &Theme) {
    settings::group(ui, theme, "Sync is paused", |ui| {
        settings::caption(
            ui,
            theme,
            "This computer is not on the list, so new pages stay here. Your pages are still here. The Sync button in the sidebar stays.",
        );
        ui.add_space(theme::token::SPACE_3);
        row(
            ui,
            theme,
            &[
                ("Sync", Btn::Quiet),
                ("Join again", Btn::Primary),
                ("Check again", Btn::Quiet),
            ],
        );
    });
}

fn keyring(ui: &mut egui::Ui, theme: &Theme) {
    settings::group(ui, theme, "Set up your keyring", |ui| {
        settings::caption(
            ui,
            theme,
            "The key store on this computer is not unlocked. Set it up once. After that, Omacus does not ask for a password when this app opens.",
        );
        ui.add_space(theme::token::SPACE_2);
        settings::caption(
            ui,
            theme,
            "This leaves the key store without a password. Omacus can open it again without asking. A password on the key store asks again every time.",
        );
        ui.add_space(theme::token::SPACE_3);
        row(
            ui,
            theme,
            &[
                ("Set up the key store", Btn::Primary),
                ("Use a passphrase instead", Btn::Quiet),
            ],
        );
    });
}

fn turn_off(ui: &mut egui::Ui, theme: &Theme) {
    settings::group(ui, theme, "Turn sync off on this computer?", |ui| {
        settings::caption(
            ui,
            theme,
            "Your pages stay here. This computer leaves the list, so the other phones and computers stop showing it. If this is the last one, that is fine. The recovery words still open the pages.",
        );
        ui.add_space(theme::token::SPACE_3);
        row(
            ui,
            theme,
            &[("Turn sync off", Btn::Primary), ("Not now", Btn::Quiet)],
        );
    });
}

fn word_grid(ui: &mut egui::Ui, theme: &Theme) {
    egui::Grid::new("sync-mock-words")
        .num_columns(3)
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            for (i, word) in WORDS.iter().enumerate() {
                ui.label(
                    RichText::new(format!("{}. {word}", i + 1))
                        .size(theme::token::TYPE_BODY)
                        .color(theme.text()),
                );
                if (i + 1) % 3 == 0 {
                    ui.end_row();
                }
            }
        });
}

fn ask(ui: &mut egui::Ui, theme: &Theme, title: &str, choices: &[&str]) {
    settings::caption(ui, theme, title);
    ui.add_space(theme::token::SPACE_1);
    ui.horizontal_wrapped(|ui| {
        for choice in choices {
            let _ = design::quiet(ui, theme, choice);
        }
    });
    ui.add_space(theme::token::SPACE_3);
}

fn code(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .color(theme.text())
            .font(egui::FontId::monospace(theme::token::TYPE_TITLE)),
    );
    ui.add_space(theme::token::SPACE_2);
}

fn device(ui: &mut egui::Ui, theme: &Theme, name: &str, kind: &str, remove: bool) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{name} · {kind}"))
                .size(theme::token::TYPE_LABEL)
                .color(theme.text()),
        );
        if remove {
            let _ = design::quiet(ui, theme, "Remove");
        }
    });
    ui.add_space(theme::token::SPACE_2);
}

fn field(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).size(theme::token::TYPE_LABEL));
    let mut text = value.to_string();
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .desired_width(f32::INFINITY)
            .interactive(false),
    );
    ui.add_space(theme::token::SPACE_1);
}

enum Btn {
    Primary,
    Quiet,
    Danger,
}

fn row(ui: &mut egui::Ui, theme: &Theme, buttons: &[(&str, Btn)]) {
    ui.horizontal_wrapped(|ui| {
        for (label, kind) in buttons {
            let _ = match kind {
                Btn::Primary => design::primary(ui, theme, label),
                Btn::Quiet => design::quiet(ui, theme, label),
                Btn::Danger => design::danger(ui, theme, label),
            };
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_sync_tab_covers_every_step() {
        let have = screens();
        for need in [
            "intro",
            "fresh",
            "words",
            "check",
            "join-words",
            "join-wait",
            "approve",
            "approve-timeout",
            "devices",
            "remove",
            "paused",
            "list-changed",
            "keyring",
            "off",
        ] {
            assert!(have.contains(&need), "{need}");
        }
    }
}
