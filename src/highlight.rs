//! Syntax coloring for the editor.
//!
//! Edit mode stays plain text. Headings are larger and bold, `**bold**` and
//! `*italic*` are styled, and a comment (`//`, or `#` after the start of the
//! line) is drawn in the comment colour. The characters stay in the text, so
//! the cursor doesn't drift.

use std::collections::HashSet;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontFamily, FontId};

use crate::theme::Theme;
use omacus_engine::complete;
use omacus_engine::markdown::{self, Form};

#[derive(Clone, Copy)]
struct Look {
    color: Color32,
    bold: bool,
    italic: bool,
    size: f32,
}

pub fn layout(text: &str, vars: &HashSet<String>, theme: &Theme, font: FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            append(
                &mut job,
                "\n",
                look(theme.foreground, false, false, font.size),
            );
        }
        paint_line(&mut job, line, vars, theme, &font);
    }
    job
}

fn paint_line(
    job: &mut LayoutJob,
    line: &str,
    vars: &HashSet<String>,
    theme: &Theme,
    font: &FontId,
) {
    let class = markdown::classify(line);
    let emph = markdown::emphasis(line);
    let comment_at = class.comment_at.unwrap_or(line.len());
    let heading = match class.form {
        Form::Heading(level) => Some(level),
        _ => None,
    };
    let size = heading.map_or(font.size, |level| heading_size(font.size, level));

    // One look per byte, then grouped. Lines are short.
    let mut looks = vec![
        look(
            if heading.is_some() {
                theme.accent
            } else {
                theme.foreground
            },
            heading.is_some(),
            false,
            size,
        );
        line.len().max(1)
    ];
    if looks.len() > line.len() {
        looks.truncate(line.len());
    }
    if line.is_empty() {
        return;
    }

    if heading.is_none() {
        for (start, end, color) in token_colors(line, comment_at, vars, theme) {
            for look in looks.iter_mut().take(end).skip(start) {
                look.color = color;
            }
        }
    }
    if comment_at < line.len() {
        for look in looks.iter_mut().skip(comment_at) {
            *look = look_with(*look, theme.comment, false, false);
        }
    }
    for (start, end) in &emph.bold {
        for item in looks.iter_mut().take(*end).skip(*start) {
            item.bold = true;
        }
    }
    for (start, end) in &emph.italic {
        for item in looks.iter_mut().take(*end).skip(*start) {
            item.italic = true;
        }
    }
    for (start, end) in emph.markers {
        for item in looks.iter_mut().take(end).skip(start) {
            *item = look_with(*item, theme.comment, false, false);
        }
    }

    let mut start = 0;
    while start < line.len() {
        let style = looks[start];
        let mut end = start + 1;
        while end < line.len() && same(looks[end], style) {
            end += 1;
        }
        // Don't split a UTF-8 character. `looks` is per byte, so step to a boundary.
        while end < line.len() && !line.is_char_boundary(end) {
            end += 1;
        }
        if line.is_char_boundary(start) {
            append(job, &line[start..end], style);
        }
        start = end;
    }
}

fn token_colors(
    line: &str,
    comment_at: usize,
    vars: &HashSet<String>,
    theme: &Theme,
) -> Vec<(usize, usize, Color32)> {
    let code = &line[..comment_at];
    let mut out = Vec::new();
    let mut rest_from = 0;
    if let Some(idx) = code.find(':') {
        let label = &code[..idx];
        if code[idx + 1..].starts_with(' ')
            && label.trim().chars().next().is_some_and(char::is_alphabetic)
            && label
                .chars()
                .all(|c| c.is_alphanumeric() || " '-_&/".contains(c))
        {
            out.push((0, idx + 1, theme.dim));
            rest_from = idx + 1;
        }
    }
    let s = code;
    let mut chars = s[rest_from..].char_indices();
    while let Some((rel, c)) = chars.next() {
        let start = rest_from + rel;
        let mut end = start + c.len_utf8();
        let color = if c == '"' {
            for (rel2, ch) in chars.by_ref() {
                end = rest_from + rel2 + ch.len_utf8();
                if ch == '"' {
                    break;
                }
            }
            theme.comment
        } else if c.is_ascii_digit() {
            for (rel2, ch) in chars.clone() {
                if !(ch.is_ascii_alphanumeric() || ch == '.' || ch == ',' || ch == '_') {
                    break;
                }
                chars.next();
                end = rest_from + rel2 + ch.len_utf8();
            }
            theme.number
        } else if c.is_alphabetic() || c == '_' {
            for (rel2, ch) in chars.clone() {
                if !(ch.is_alphanumeric() || ch == '_') {
                    break;
                }
                chars.next();
                end = rest_from + rel2 + ch.len_utf8();
            }
            let word = &s[start..end];
            let defined = omacus_engine::definition_span(line) == Some((start, end));
            if defined {
                theme.defined
            } else if vars.contains(word) {
                theme.variable
            } else if complete::word_kind(word).is_some() {
                theme.keyword
            } else {
                continue;
            }
        } else if "$£€¥₹₩₽₺%".contains(c) {
            theme.number
        } else if c.is_whitespace() {
            continue;
        } else {
            theme.dim
        };
        out.push((start, end, color));
    }
    out
}

fn heading_size(base: f32, level: u8) -> f32 {
    match level {
        1 => base + 8.0,
        2 => base + 4.0,
        3 => base + 2.0,
        _ => base + 1.0,
    }
}

fn look(color: Color32, bold: bool, italic: bool, size: f32) -> Look {
    Look {
        color,
        bold,
        italic,
        size,
    }
}

fn look_with(look: Look, color: Color32, bold: bool, italic: bool) -> Look {
    Look {
        color,
        bold,
        italic,
        size: look.size,
    }
}

fn same(a: Look, b: Look) -> bool {
    a.color == b.color && a.bold == b.bold && a.italic == b.italic && a.size == b.size
}

fn append(job: &mut LayoutJob, text: &str, look: Look) {
    if text.is_empty() {
        return;
    }
    let family = if look.bold {
        FontFamily::Name("bold".into())
    } else {
        FontFamily::Monospace
    };
    let mut format = TextFormat::simple(FontId::new(look.size, family), look.color);
    format.italics = look.italic;
    job.append(text, 0.0, format);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn job(text: &str) -> LayoutJob {
        layout(
            text,
            &HashSet::new(),
            &Theme::dark(),
            FontId::monospace(16.0),
        )
    }

    #[test]
    fn a_heading_is_larger_than_the_body() {
        let job = job("# Trip\n2 + 2");
        let sizes: Vec<f32> = job.sections.iter().map(|s| s.format.font_id.size).collect();
        assert!(sizes.contains(&24.0), "{sizes:?}");
        assert!(sizes.contains(&16.0), "{sizes:?}");
    }

    #[test]
    fn a_definition_is_a_different_colour_from_a_use_and_a_unit() {
        let theme = Theme::dark();
        let vars = HashSet::from(["rent".to_string()]);
        let job = layout(
            "rent = $10\nrent\nmile",
            &vars,
            &theme,
            FontId::monospace(16.0),
        );
        let colors = colors_of(&job, "rent");
        assert_eq!(colors.len(), 2, "{colors:?}");
        assert_eq!(colors[0], theme.defined);
        assert_eq!(colors[1], theme.variable);
        assert_eq!(colors_of(&job, "mile"), vec![theme.keyword]);
        let noted = layout("rent = 1 # monthly", &vars, &theme, FontId::monospace(16.0));
        assert_eq!(colors_of(&noted, "# monthly"), vec![theme.comment]);
    }

    fn colors_of(job: &LayoutJob, needle: &str) -> Vec<Color32> {
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(i) = job.text[from..].find(needle) {
            let at = from + i;
            let color = job
                .sections
                .iter()
                .find(|s| s.byte_range.start.0 <= at && at < s.byte_range.end.0)
                .map(|s| s.format.color)
                .unwrap();
            out.push(color);
            from = at + needle.len();
        }
        out
    }

    #[test]
    fn bold_markers_are_quiet_and_the_word_is_bold() {
        let job = job("a **bold** word");
        let bold = job.sections.iter().find(|s| {
            s.format.font_id.size == 16.0 && matches!(s.format.font_id.family, FontFamily::Name(_))
        });
        assert!(bold.is_some());
    }
}
