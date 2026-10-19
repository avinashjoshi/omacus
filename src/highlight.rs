//! Syntax coloring for the editor.

use std::collections::HashSet;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};

use crate::theme::Theme;

const KEYWORDS: &[&str] = &[
    "in",
    "to",
    "as",
    "of",
    "on",
    "off",
    "what",
    "is",
    "prev",
    "sum",
    "total",
    "avg",
    "average",
    "mean",
    "min",
    "max",
    "today",
    "now",
    "tomorrow",
    "yesterday",
    "plus",
    "minus",
    "times",
    "mod",
];

pub fn layout(text: &str, vars: &HashSet<String>, theme: &Theme, font: FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut push = |s: &str, color: Color32| {
        if !s.is_empty() {
            job.append(s, 0.0, TextFormat::simple(font.clone(), color));
        }
    };

    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            push("\n", theme.foreground);
        }
        let trimmed = line.trim_start();
        if omacus_engine::is_header(trimmed) {
            push(line, theme.accent);
            continue;
        }

        let (code, comment) = match line.find("//") {
            Some(idx) => line.split_at(idx),
            None => (line, ""),
        };

        // "Label: ..." prefix is dimmed.
        let mut rest = code;
        if let Some(idx) = code.find(':') {
            let label = &code[..idx];
            if code[idx + 1..].starts_with(' ')
                && label.trim().chars().next().is_some_and(char::is_alphabetic)
                && label
                    .chars()
                    .all(|c| c.is_alphanumeric() || " '-_&/".contains(c))
            {
                push(&code[..=idx], theme.dim);
                rest = &code[idx + 1..];
            }
        }

        tokens(rest, vars, theme, &mut push);
        push(comment, theme.comment);
    }
    job
}

fn tokens(s: &str, vars: &HashSet<String>, theme: &Theme, push: &mut impl FnMut(&str, Color32)) {
    let mut chars = s.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        let mut end = start + c.len_utf8();
        chars.next();
        let color = if c == '"' {
            while let Some(&(i, ch)) = chars.peek() {
                chars.next();
                end = i + ch.len_utf8();
                if ch == '"' {
                    break;
                }
            }
            theme.comment
        } else if c.is_ascii_digit() {
            while let Some(&(i, ch)) = chars.peek() {
                if !(ch.is_ascii_alphanumeric() || ch == '.' || ch == ',' || ch == '_') {
                    break;
                }
                chars.next();
                end = i + ch.len_utf8();
            }
            theme.number
        } else if c.is_alphabetic() || c == '_' {
            while let Some(&(i, ch)) = chars.peek() {
                if !(ch.is_alphanumeric() || ch == '_') {
                    break;
                }
                chars.next();
                end = i + ch.len_utf8();
            }
            let word = &s[start..end];
            if vars.contains(word) {
                theme.variable
            } else if KEYWORDS.contains(&word.to_ascii_lowercase().as_str()) {
                theme.keyword
            } else {
                theme.foreground
            }
        } else if "$£€¥₹₩₽₺%".contains(c) {
            theme.number
        } else if c.is_whitespace() {
            theme.foreground
        } else {
            theme.dim
        };
        push(&s[start..end], color);
    }
}
