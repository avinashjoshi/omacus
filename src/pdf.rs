//! A preview, as a PDF. Standard Helvetica, so it opens everywhere and needs no
//! embedded font. Characters Helvetica can't draw become a close ASCII stand-in.
//!
//! The page is set like a short document: a real margin, a heading hierarchy,
//! body leading that can be read, and each answer in brackets in a dark ink
//! that still shows up when the page is printed in black and white. A wrapped
//! line, a list item and a table row move to the next page together.

use std::path::{Path, PathBuf};

use omacus_engine::markdown::{Block, Inline, Rich};

const PAGE_W: f32 = 612.0;
const PAGE_H: f32 = 792.0;
const MARGIN_X: f32 = 68.0;
const MARGIN_TOP: f32 = 72.0;
const MARGIN_BOTTOM: f32 = 68.0;
const CONTENT_W: f32 = PAGE_W - MARGIN_X * 2.0;
/// Dark blue. It stays readable when a printer turns the page grey.
const ANSWER_INK: &str = "0.102 0.227 0.357";

/// Write the preview to `path`, replacing whatever is already there.
pub fn write(
    title: &str,
    blocks: &[Block],
    path: &Path,
    answers_column: bool,
) -> Result<(), String> {
    let bytes = bytes(title, blocks, answers_column);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

/// Downloads, or the temp directory when the OS has no Downloads folder.
pub fn default_export_dir() -> PathBuf {
    dirs::download_dir().unwrap_or_else(std::env::temp_dir)
}

/// `Soflo Acquisition.pdf`. The name is stable: a second export uses the same path.
pub fn export_path(dir: &Path, title: &str) -> PathBuf {
    dir.join(file_name(title))
}

/// "Downloads" for the OS downloads folder, otherwise the folder's own name.
pub fn folder_label(dir: &Path) -> String {
    if dirs::download_dir().is_some_and(|downloads| downloads == dir) {
        return "Downloads".into();
    }
    dir.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "that folder".into())
}

/// An existing file asks before it is overwritten, unless the setting says not to.
pub fn needs_replace_prompt(exists: bool, always_replace: bool) -> bool {
    exists && !always_replace
}

/// A temp PDF for the print dialog. The name is the page title, overwritten in place.
pub fn print_preview(
    title: &str,
    blocks: &[Block],
    answers_column: bool,
) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(file_name(title));
    write(title, blocks, &path, answers_column)?;
    open_path(&path)?;
    Ok(path)
}

/// Open `dir` in the system file manager.
pub fn open_folder(dir: &Path) -> Result<(), String> {
    let result = {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg(dir).spawn()
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer").arg(dir).spawn()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            std::process::Command::new("xdg-open").arg(dir).spawn()
        }
    };
    result
        .map(|_| ())
        .map_err(|e| format!("Couldn't open the folder ({e})"))
}

pub fn bytes(title: &str, blocks: &[Block], answers_column: bool) -> Vec<u8> {
    let pages = layout(title, blocks, answers_column);
    let pages = if pages.is_empty() {
        vec!["BT /F1 11 Tf 0 0 0 rg 68 720 Td ( ) Tj ET".to_string()]
    } else {
        pages
    };
    // 1 catalog, 2 pages, 3–5 fonts, then content/page pairs.
    let mut objects: Vec<Vec<u8>> = vec![
        Vec::new(),
        Vec::new(),
        font_object("Helvetica"),
        font_object("Helvetica-Bold"),
        font_object("Helvetica-Oblique"),
    ];

    let mut kids = Vec::new();
    for content in &pages {
        let content_id = objects.len() + 1;
        let page_id = content_id + 1;
        let data = format!("{content}\n");
        let stream = format!("<< /Length {} >>\nstream\n{data}endstream", data.len());
        objects.push(stream.into_bytes());
        let page = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {content_id} 0 R /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R >> >> >>"
        );
        objects.push(page.into_bytes());
        kids.push(format!("{page_id} 0 R"));
    }
    let title = pdf_string(title);
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    objects[1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        kids.len()
    )
    .into_bytes();

    let mut out = Vec::from("%PDF-1.4\n".as_bytes());
    let mut xref = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        xref.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(obj);
        out.extend(b"\nendobj\n");
    }
    let startxref = out.len();
    out.extend(format!("xref\n0 {}\n", xref.len()).as_bytes());
    out.extend(b"0000000000 65535 f \n");
    for off in xref.iter().skip(1) {
        out.extend(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info << /Title {title} >> >>\nstartxref\n{startxref}\n%%EOF\n",
            xref.len()
        )
        .as_bytes(),
    );
    out
}

fn font_object(name: &str) -> Vec<u8> {
    format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} >>").into_bytes()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ink {
    Body,
    Comment,
    Answer,
}

struct Span {
    text: String,
    ink: Ink,
}

struct Run {
    size: f32,
    leading: f32,
    bold: bool,
    oblique: bool,
    indent: f32,
    spans: Vec<Span>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Rule {
    None,
    Light,
    Strong,
    /// A short accent under a title.
    Accent,
}

#[derive(Clone, Copy)]
enum Mark {
    None,
    Bullet,
    Task(bool),
    Number(u32),
}

struct Group {
    gap: f32,
    runs: Vec<Run>,
    rule: Rule,
    /// A thin bar in the left margin, for a quoted line.
    bar: bool,
    /// Set when the answers column is on: drawn at the right margin of the first line.
    column_answer: Option<String>,
    mark: Mark,
}

fn layout(title: &str, blocks: &[Block], answers_column: bool) -> Vec<String> {
    let reserve = if answers_column {
        answer_reserve(blocks)
    } else {
        0.0
    };
    paginate(title, &groups_of(blocks, answers_column, reserve))
}

fn answer_reserve(blocks: &[Block]) -> f32 {
    let mut widest = 0.0_f32;
    let mut see = |rich: &Rich| {
        if let Some(answer) = rich.answer.as_deref().filter(|s| !s.is_empty()) {
            let w = bracket(answer).chars().count() as f32 * 11.0 * 0.52;
            widest = widest.max(w);
        }
    };
    for block in blocks {
        match block {
            Block::Heading { rich, .. } => see(rich),
            Block::Lines(rows) | Block::Quote(rows) => rows.iter().for_each(&mut see),
            Block::List { items, .. } => items.iter().for_each(|item| see(&item.rich)),
            Block::Table { .. } | Block::Blank => {}
        }
    }
    if widest == 0.0 {
        0.0
    } else {
        (widest + 18.0).clamp(78.0, CONTENT_W * 0.46)
    }
}

fn groups_of(blocks: &[Block], column: bool, reserve: f32) -> Vec<Group> {
    let mut groups = Vec::new();
    // Space carried onto the next block, so a paragraph doesn't glue itself
    // to the heading that follows. A blank line is that space, so it clears this.
    let mut after = 0.0;
    let push = |groups: &mut Vec<Group>, mut group: Group, after: &mut f32| {
        group.gap += *after;
        *after = 0.0;
        groups.push(group);
    };
    for block in blocks {
        match block {
            Block::Heading { level, rich } => {
                let (size, leading, gap, rule) = match level {
                    1 => (22.0, 30.0, 4.0, Rule::Accent),
                    2 => (16.0, 22.0, 16.0, Rule::None),
                    _ => (13.0, 18.0, 12.0, Rule::None),
                };
                let (runs, column_answer) =
                    runs_for(rich, face(size, leading, true, false, 0.0, column, reserve));
                push(
                    &mut groups,
                    Group {
                        gap,
                        runs,
                        rule,
                        bar: false,
                        column_answer,
                        mark: Mark::None,
                    },
                    &mut after,
                );
                after = if *level == 1 { 10.0 } else { 6.0 };
            }
            Block::Lines(rows) => {
                for rich in rows {
                    let (runs, column_answer) =
                        runs_for(rich, face(11.5, 17.0, false, false, 0.0, column, reserve));
                    push(
                        &mut groups,
                        Group {
                            gap: 0.0,
                            runs,
                            rule: Rule::None,
                            bar: false,
                            column_answer,
                            mark: Mark::None,
                        },
                        &mut after,
                    );
                }
                after = 8.0;
            }
            Block::Blank => {
                after = 0.0;
                push(
                    &mut groups,
                    Group {
                        gap: 0.0,
                        runs: vec![empty_run(11.5, 17.0)],
                        rule: Rule::None,
                        bar: false,
                        column_answer: None,
                        mark: Mark::None,
                    },
                    &mut after,
                );
            }
            Block::List { items, .. } => {
                for item in items {
                    let (mark, indent) = match item.checked {
                        Some(checked) => (Mark::Task(checked), 16.0),
                        None => match item.number {
                            Some(n) => (Mark::Number(n), 22.0),
                            None => (Mark::Bullet, 14.0),
                        },
                    };
                    let (runs, column_answer) = runs_for(
                        &item.rich,
                        face(11.5, 17.0, false, false, indent, column, reserve),
                    );
                    push(
                        &mut groups,
                        Group {
                            gap: 1.0,
                            runs,
                            rule: Rule::None,
                            bar: false,
                            column_answer,
                            mark,
                        },
                        &mut after,
                    );
                }
                after = 8.0;
            }
            Block::Quote(rows) => {
                for rich in rows {
                    let (runs, column_answer) =
                        runs_for(rich, face(11.5, 17.0, false, true, 16.0, column, reserve));
                    push(
                        &mut groups,
                        Group {
                            gap: 0.0,
                            runs,
                            rule: Rule::None,
                            bar: true,
                            column_answer,
                            mark: Mark::None,
                        },
                        &mut after,
                    );
                }
                after = 10.0;
            }
            Block::Table { header, rows } => {
                let plain_rows: Vec<Vec<String>> = rows
                    .iter()
                    .map(|r| r.iter().map(|c| plain(c)).collect())
                    .collect();
                let widths = table_widths(&plain_rows);
                for (n, cells) in plain_rows.iter().enumerate() {
                    let head = *header && n == 0;
                    let runs = runs_for_text(
                        &format_row(cells, &widths),
                        None,
                        None,
                        face(10.0, 16.0, head, false, 0.0, false, 0.0),
                    );
                    push(
                        &mut groups,
                        Group {
                            gap: if n == 0 { 8.0 } else { 0.0 },
                            runs,
                            rule: if head { Rule::Strong } else { Rule::Light },
                            bar: false,
                            column_answer: None,
                            mark: Mark::None,
                        },
                        &mut after,
                    );
                }
                after = 12.0;
            }
        }
    }
    if let Some(first) = groups.first_mut() {
        first.gap = 0.0;
    }
    groups
}

struct Face {
    size: f32,
    leading: f32,
    bold: bool,
    oblique: bool,
    indent: f32,
    column: bool,
    reserve: f32,
}

fn face(
    size: f32,
    leading: f32,
    bold: bool,
    oblique: bool,
    indent: f32,
    column: bool,
    reserve: f32,
) -> Face {
    Face {
        size,
        leading,
        bold,
        oblique,
        indent,
        column,
        reserve,
    }
}

fn runs_for(rich: &Rich, face: Face) -> (Vec<Run>, Option<String>) {
    let comment = rich.comment.as_deref().filter(|s| !s.is_empty());
    let answer = rich
        .answer
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(bracket);
    let inline = if face.column { None } else { answer.clone() };
    let parked = if face.column { answer } else { None };
    let runs = runs_for_text(&plain(&rich.inlines), comment, inline.as_deref(), face);
    (runs, parked)
}

fn runs_for_text(text: &str, comment: Option<&str>, answer: Option<&str>, face: Face) -> Vec<Run> {
    let Face {
        size,
        leading,
        bold,
        oblique,
        indent,
        column,
        reserve,
    } = face;
    let reserve = if column { reserve } else { 0.0 };
    let width = wrap_chars(size, indent, reserve);
    let rows = wrap(text, width);
    let mut runs = Vec::new();
    let last = rows.len().saturating_sub(1);
    for (i, row) in rows.into_iter().enumerate() {
        let hang = if i == 0 { indent } else { indent + 14.0 };
        let mut spans = vec![Span {
            text: row,
            ink: Ink::Body,
        }];
        if i == last {
            attach(&mut spans, comment, Ink::Comment, width);
            if let Some(comment) = comment
                && spans.iter().all(|s| s.ink != Ink::Comment)
            {
                runs.push(run(size, leading, bold, oblique, hang, spans));
                spans = vec![Span {
                    text: comment.to_string(),
                    ink: Ink::Comment,
                }];
            }
            attach(&mut spans, answer, Ink::Answer, width);
            // An answer that did not fit becomes its own line, still with the expression.
            if let Some(answer) = answer
                && spans.iter().all(|s| s.ink != Ink::Answer)
            {
                runs.push(run(size, leading, bold, oblique, hang, spans));
                runs.push(run(
                    size,
                    leading,
                    bold,
                    oblique,
                    indent + 14.0,
                    vec![Span {
                        text: answer.to_string(),
                        ink: Ink::Answer,
                    }],
                ));
                return runs;
            }
        }
        runs.push(run(size, leading, bold, oblique, hang, spans));
    }
    if runs.is_empty() {
        let mut spans = Vec::new();
        if let Some(comment) = comment {
            spans.push(Span {
                text: comment.to_string(),
                ink: Ink::Comment,
            });
        }
        if let Some(answer) = answer {
            spans.push(Span {
                text: answer.to_string(),
                ink: Ink::Answer,
            });
        }
        if spans.is_empty() {
            spans.push(Span {
                text: String::new(),
                ink: Ink::Body,
            });
        }
        runs.push(run(size, leading, bold, oblique, indent, spans));
    }
    runs
}

/// Put `extra` on the line when it fits; otherwise leave it for the caller.
fn attach(spans: &mut Vec<Span>, extra: Option<&str>, ink: Ink, width: usize) {
    let Some(extra) = extra else { return };
    let used: usize = spans.iter().map(|s| s.text.chars().count()).sum();
    let gap = spans.iter().any(|s| !s.text.is_empty());
    if used + usize::from(gap) + extra.chars().count() <= width {
        spans.push(Span {
            text: if gap {
                format!(" {extra}")
            } else {
                extra.to_string()
            },
            ink,
        });
    }
}

fn run(size: f32, leading: f32, bold: bool, oblique: bool, indent: f32, spans: Vec<Span>) -> Run {
    Run {
        size,
        leading,
        bold,
        oblique,
        indent,
        spans,
    }
}

fn empty_run(size: f32, leading: f32) -> Run {
    run(
        size,
        leading,
        false,
        false,
        0.0,
        vec![Span {
            text: String::new(),
            ink: Ink::Body,
        }],
    )
}

fn bracket(answer: &str) -> String {
    format!("[ {answer} ]")
}

fn plain(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(s)
            | Inline::Strong(s)
            | Inline::Em(s)
            | Inline::Code(s)
            | Inline::Link { text: s, .. } => out.push_str(s),
        }
    }
    out
}

fn table_widths(rows: &[Vec<String>]) -> Vec<usize> {
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut widths = vec![0; cols];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count()).min(28);
        }
    }
    widths
}

fn format_row(cells: &[String], widths: &[usize]) -> String {
    let mut parts = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        let width = widths.get(i).copied().unwrap_or(0);
        let mut shown: String = cell.chars().take(width.max(1)).collect();
        let pad = width.saturating_sub(shown.chars().count());
        shown.extend(std::iter::repeat_n(' ', pad));
        parts.push(shown);
    }
    parts.join("   ")
}

fn wrap_chars(size: f32, indent: f32, reserve: f32) -> usize {
    let width = (CONTENT_W - indent - reserve).max(48.0);
    ((width / (size * 0.52)) as usize).max(12)
}

/// Groups that fit on a fresh page move there whole, so a wrapped line is not
/// torn across the page break.
fn paginate(title: &str, groups: &[Group]) -> Vec<String> {
    let top = PAGE_H - MARGIN_TOP;
    let usable = top - MARGIN_BOTTOM;
    let mut pages = Vec::new();
    let mut ops = String::new();
    let mut y = top;
    let mut started = false;
    let mut in_text = false;
    let mut page_no = 1_u32;

    for group in groups {
        let rule_room = match group.rule {
            Rule::None => 0.0,
            Rule::Accent => 8.0,
            _ => 5.0,
        };
        let at_top = (y - top).abs() < 0.5;
        let gap = if at_top { 0.0 } else { group.gap };
        let body = gap + group.runs.iter().map(|r| r.leading).sum::<f32>() + rule_room;
        if started && y - body < MARGIN_BOTTOM && body <= usable {
            close_text(&mut ops, &mut in_text);
            paint_footer(&mut ops, title, page_no);
            pages.push(std::mem::take(&mut ops));
            page_no += 1;
            y = top;
        }
        let gap = if (y - top).abs() < 0.5 {
            0.0
        } else {
            group.gap
        };
        y -= gap;
        let bar_top = y;
        let mark_baseline = group
            .runs
            .first()
            .map(|run| y - run.size * 0.80)
            .unwrap_or(y);
        draw_mark(&mut ops, &mut in_text, group.mark, mark_baseline);
        for (i, run) in group.runs.iter().enumerate() {
            let baseline = y - run.size * 0.80;
            ensure_text(&mut ops, &mut in_text);
            ops.push_str(&format!(
                "1 0 0 1 {} {} Tm\n",
                num(MARGIN_X + run.indent),
                num(baseline),
            ));
            for span in &run.spans {
                if span.text.is_empty() {
                    continue;
                }
                let (font, color) = match span.ink {
                    Ink::Body if run.bold => ("F2", "0 0 0"),
                    Ink::Body if run.oblique => ("F3", "0.29 0.27 0.25"),
                    Ink::Body => ("F1", "0 0 0"),
                    Ink::Comment => ("F3", "0.45 0.43 0.40"),
                    Ink::Answer => ("F1", ANSWER_INK),
                };
                ops.push_str(&format!(
                    "/{font} {} Tf\n{color} rg\n({}) Tj\n",
                    num(run.size),
                    pdf_text(&span.text),
                ));
            }
            if i == 0
                && let Some(answer) = &group.column_answer
            {
                let w = answer.chars().count() as f32 * run.size * 0.52;
                let x = PAGE_W - MARGIN_X - w;
                ops.push_str(&format!(
                    "/F1 {} Tf\n{ANSWER_INK} rg\n1 0 0 1 {} {} Tm\n({}) Tj\n",
                    num(run.size),
                    num(x),
                    num(baseline),
                    pdf_text(answer),
                ));
            }
            y -= run.leading;
        }
        if group.bar {
            close_text(&mut ops, &mut in_text);
            let x = MARGIN_X + 2.0;
            ops.push_str(&format!(
                "{ANSWER_INK} RG\n1.25 w\n{} {} m {} {} l S\n",
                num(x),
                num(bar_top - 2.0),
                num(x),
                num(y + 4.0),
            ));
        }
        if group.rule != Rule::None {
            close_text(&mut ops, &mut in_text);
            let rule_y = y + 4.0;
            match group.rule {
                Rule::Accent => {
                    ops.push_str(&format!(
                        "{ANSWER_INK} RG\n0.8 w\n{} {} m {} {} l S\n",
                        num(MARGIN_X),
                        num(rule_y),
                        num(MARGIN_X + 36.0),
                        num(rule_y),
                    ));
                }
                Rule::Strong => {
                    ops.push_str(&format!(
                        "0.16 0.16 0.16 RG\n0.9 w\n{} {} m {} {} l S\n",
                        num(MARGIN_X),
                        num(rule_y),
                        num(PAGE_W - MARGIN_X),
                        num(rule_y),
                    ));
                }
                Rule::Light => {
                    ops.push_str(&format!(
                        "0.82 0.82 0.82 RG\n0.4 w\n{} {} m {} {} l S\n",
                        num(MARGIN_X),
                        num(rule_y),
                        num(PAGE_W - MARGIN_X),
                        num(rule_y),
                    ));
                }
                Rule::None => {}
            }
            y -= rule_room;
        }
        started = true;
    }
    close_text(&mut ops, &mut in_text);
    if started {
        paint_footer(&mut ops, title, page_no);
        pages.push(ops);
    }
    pages
}

fn draw_mark(ops: &mut String, in_text: &mut bool, mark: Mark, baseline: f32) {
    match mark {
        Mark::None => {}
        Mark::Bullet => {
            close_text(ops, in_text);
            ops.push_str(&format!(
                "0.25 0.25 0.25 rg\n{} {} 3.2 3.2 re\nf\n",
                num(MARGIN_X + 0.2),
                num(baseline + 1.4),
            ));
        }
        Mark::Task(checked) => {
            close_text(ops, in_text);
            let x = MARGIN_X;
            let y = baseline - 1.0;
            ops.push_str(&format!(
                "0.25 0.25 0.25 RG\n0.8 w\n{} {} 8 8 re\nS\n",
                num(x),
                num(y),
            ));
            if checked {
                ops.push_str(&format!(
                    "{ANSWER_INK} RG\n1.3 w\n{} {} m {} {} l {} {} l S\n",
                    num(x + 1.6),
                    num(y + 3.4),
                    num(x + 3.3),
                    num(y + 1.6),
                    num(x + 6.6),
                    num(y + 6.2),
                ));
            }
        }
        Mark::Number(n) => {
            ensure_text(ops, in_text);
            ops.push_str(&format!(
                "/F1 11 Tf\n0.35 0.35 0.35 rg\n1 0 0 1 {} {} Tm\n({}) Tj\n",
                num(MARGIN_X),
                num(baseline),
                pdf_text(&format!("{n}.")),
            ));
        }
    }
}

fn paint_footer(ops: &mut String, title: &str, page: u32) {
    let y = 36.0;
    ops.push_str(&format!(
        "0.78 0.78 0.78 RG\n0.4 w\n{} {} m {} {} l S\n",
        num(MARGIN_X),
        num(y + 14.0),
        num(PAGE_W - MARGIN_X),
        num(y + 14.0),
    ));
    let page_s = page.to_string();
    let page_w = page_s.chars().count() as f32 * 9.0 * 0.52;
    let max_chars = ((CONTENT_W - page_w - 16.0) / (9.0 * 0.5)) as usize;
    let mut shown: String = title.chars().take(max_chars.max(8)).collect();
    if shown.chars().count() < title.chars().count() {
        shown.push_str("...");
    }
    ops.push_str(&format!(
        "BT\n/F1 9 Tf\n0.45 0.43 0.40 rg\n1 0 0 1 {} {} Tm\n({}) Tj\nET\n",
        num(MARGIN_X),
        num(y),
        pdf_text(&shown),
    ));
    ops.push_str(&format!(
        "BT\n/F1 9 Tf\n0.45 0.43 0.40 rg\n1 0 0 1 {} {} Tm\n({}) Tj\nET\n",
        num(PAGE_W - MARGIN_X - page_w),
        num(y),
        pdf_text(&page_s),
    ));
}

fn ensure_text(ops: &mut String, in_text: &mut bool) {
    if !*in_text {
        ops.push_str("BT\n");
        *in_text = true;
    }
}

fn close_text(ops: &mut String, in_text: &mut bool) {
    if *in_text {
        ops.push_str("ET\n");
        *in_text = false;
    }
}

fn num(n: f32) -> String {
    let s = format!("{n:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut rows = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let extra = usize::from(!line.is_empty());
        if !line.is_empty() && line.chars().count() + extra + word.chars().count() > width {
            rows.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        // A single word longer than the measure stays intact: an answer is one
        // word, so it is never broken a character at a time.
        line.push_str(word);
    }
    if !line.is_empty() {
        rows.push(line);
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

fn pdf_string(s: &str) -> String {
    format!("({})", pdf_text(s))
}

fn pdf_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '£' => out.push_str("\\243"),
            '¥' => out.push_str("\\245"),
            '€' => out.push_str("\\200"),
            '—' | '–' | '−' => out.push('-'),
            '“' | '”' => out.push('"'),
            '‘' | '’' => out.push('\''),
            '×' => out.push('x'),
            '•' => out.push('-'),
            '₹' => out.push_str("Rs "),
            c if (' '..='~').contains(&c) => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

fn file_name(title: &str) -> String {
    let name: String = title
        .chars()
        .map(|c| {
            if "\\/:*?\"<>|".contains(c) || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim().trim_matches('.').trim();
    let name: String = if name.is_empty() {
        "Page".to_string()
    } else {
        name.chars().take(80).collect()
    };
    format!("{name}.pdf")
}

fn open_path(path: &Path) -> Result<(), String> {
    let result = {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg(path).spawn()
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", &path.display().to_string()])
                .spawn()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            std::process::Command::new("xdg-open").arg(path).spawn()
        }
    };
    result
        .map(|_| ())
        .map_err(|e| format!("Couldn't open the PDF ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omacus_engine::markdown::render;

    fn streams(pdf: &str) -> Vec<&str> {
        pdf.split("stream\n")
            .skip(1)
            .filter_map(|s| s.split("endstream").next())
            .collect()
    }

    #[test]
    fn a_pdf_holds_the_line_and_its_answer() {
        let blocks = render("- hotel = 3 nights * $180\n", &[Some("$540")]);
        let bytes = bytes("Kyoto", &blocks, true);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("hotel = 3 nights * $180"));
        assert!(text.contains("[ $540 ]"));
        assert!(text.contains(ANSWER_INK));
        assert!(text.contains("/Helvetica-Bold"));
        assert!(text.contains("%%EOF"));
    }

    #[test]
    fn a_wrapped_line_moves_to_the_next_page_whole() {
        let mut source = String::from("# Kyoto\n");
        let mut answers = vec![None];
        for i in 0..48 {
            source.push_str(&format!("day {i} = $10\n"));
            answers.push(Some("$10"));
        }
        source.push_str(
            "hotel = 3 nights * $180 plus a long note about the quiet room near the river and the breakfast that comes with it\n",
        );
        answers.push(Some("$540"));
        let refs: Vec<Option<&str>> = answers.iter().map(|s| s.as_deref()).collect();
        let text =
            String::from_utf8_lossy(&bytes("Kyoto", &render(&source, &refs), true)).into_owned();
        let pages = streams(&text);
        assert!(
            pages.len() >= 2,
            "expected a second page, got {}",
            pages.len()
        );
        let quiet: Vec<usize> = pages
            .iter()
            .enumerate()
            .filter(|(_, s)| s.contains("quiet room"))
            .map(|(i, _)| i)
            .collect();
        let breakfast: Vec<usize> = pages
            .iter()
            .enumerate()
            .filter(|(_, s)| s.contains("breakfast"))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(quiet, breakfast, "the wrapped line was split across pages");
        assert_eq!(quiet.len(), 1);
        assert!(pages[quiet[0]].contains("[ $540 ]"));
        assert_ne!(
            quiet[0], 0,
            "the long line should have moved onto the next page"
        );
    }

    #[test]
    fn a_sample_page_sets_like_a_document() {
        let source = "\
# Northwind trip

Flights: 4 * $1,180
Hotel: 5 nights * $240

## On the ground

- hotel = 3 nights * $180
- tax = 8% of hotel

| item | amount |
| --- | --- |
| flights | $4,720 |
| hotel | $1,200 |

> tip = 18% on $62
";
        let answers = [
            None,           // # Northwind trip
            None,           // blank
            Some("$4,720"), // Flights
            Some("$1,200"), // Hotel
            None,           // blank
            None,           // ## On the ground
            None,           // blank
            Some("$540"),   // hotel
            Some("$43.20"), // tax
            None,           // blank
            None,           // table header
            None,           // table rule
            None,           // flights row
            None,           // hotel row
            None,           // blank
            Some("$11.16"), // tip
        ];
        let refs: Vec<Option<&str>> = answers.to_vec();
        let blocks = render(source, &refs);
        let bytes = bytes("Northwind trip", &blocks, true);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("[ $4,720 ]"));
        assert!(text.contains("[ $43.20 ]"));
        assert!(text.contains("/F3 "));
        if let Ok(dir) = std::env::var("OMACUS_SHOTS") {
            let path = std::path::Path::new(&dir).join("northwind.pdf");
            std::fs::write(path, bytes).expect("write sample pdf");
        }
    }

    #[test]
    fn a_heading_is_bold_and_a_table_has_a_rule() {
        let blocks = render(
            "# Trip\n\n| nights | rate |\n| --- | --- |\n| 3 | $180 |\n",
            &[None, None, None, None],
        );
        let text = String::from_utf8_lossy(&bytes("Trip", &blocks, true)).into_owned();
        assert!(text.contains("/F2 22 Tf"), "{text}");
        assert!(
            text.contains("0.16 0.16 0.16 RG"),
            "the header rule is missing"
        );
    }

    #[test]
    fn a_comment_is_kept_and_blank_lines_open_a_gap() {
        let tight = render("a = 5\nb = 6\n", &[Some("5"), Some("6")]);
        let loose = render(
            "a = 5 # first\n\n\nb = 6\n// only a note\n",
            &[Some("5"), None, None, Some("6"), None],
        );
        let tight_pdf = String::from_utf8_lossy(&bytes("Notes", &tight, true)).into_owned();
        let loose_pdf = String::from_utf8_lossy(&bytes("Notes", &loose, true)).into_owned();
        assert!(loose_pdf.contains("first"));
        assert!(loose_pdf.contains("only a note"));
        assert!(loose_pdf.contains("/F3 "));
        let tight_gap = gap_between(&tight_pdf, "(a = 5)", "(b = 6)");
        let loose_gap = gap_between(&loose_pdf, "(a = 5)", "(b = 6)");
        assert!(
            loose_gap > tight_gap + 20.0,
            "blank lines collapsed: tight {tight_gap}, loose {loose_gap}"
        );
    }

    #[test]
    fn answers_in_the_column_share_a_right_edge() {
        let blocks = render("a = 5\nbb = 6\n", &[Some("5"), Some("$40")]);
        let text = String::from_utf8_lossy(&bytes("Column", &blocks, true)).into_owned();
        let right = |needle: &str| {
            let x = tm_before(&text, needle);
            let chars = needle
                .trim_start_matches('(')
                .trim_end_matches(')')
                .chars()
                .count() as f32;
            x + chars * 11.5 * 0.52
        };
        let a = right("([ 5 ])");
        let b = right("([ $40 ])");
        assert!((a - b).abs() < 1.0, "right edges {a} and {b}");
    }

    #[test]
    fn a_second_export_replaces_the_same_file() {
        let dir = std::env::temp_dir().join(format!("omacus-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blocks = render("rent = 1\n", &[Some("1")]);
        let path = export_path(&dir, "Soflo Acquisition");
        write("Soflo Acquisition", &blocks, &path, true).unwrap();
        write("Soflo Acquisition", &blocks, &path, false).unwrap();
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["Soflo Acquisition.pdf".to_string()]);
        assert!(needs_replace_prompt(true, false));
        assert!(!needs_replace_prompt(true, true));
        assert!(!needs_replace_prompt(false, false));
        if let Some(downloads) = dirs::download_dir() {
            assert_eq!(folder_label(&downloads), "Downloads");
        }
        let named = dir.join("Northwind");
        std::fs::create_dir_all(&named).unwrap();
        assert_eq!(folder_label(&named), "Northwind");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn gap_between(pdf: &str, top: &str, bottom: &str) -> f32 {
        let y_top = tm_y_before(pdf, top);
        let y_bottom = tm_y_before(pdf, bottom);
        y_top - y_bottom
    }

    fn tm_before(pdf: &str, needle: &str) -> f32 {
        let (x, _) = tm_xy_before(pdf, needle);
        x
    }

    fn tm_y_before(pdf: &str, needle: &str) -> f32 {
        let (_, y) = tm_xy_before(pdf, needle);
        y
    }

    fn tm_xy_before(pdf: &str, needle: &str) -> (f32, f32) {
        let mut x = 0.0;
        let mut y = 0.0;
        for line in pdf.lines() {
            if let Some(rest) = line.strip_prefix("1 0 0 1 ") {
                let mut parts = rest.split_whitespace();
                if let (Some(xs), Some(ys)) = (parts.next(), parts.next()) {
                    x = xs.parse().unwrap_or(x);
                    y = ys.parse().unwrap_or(y);
                }
            }
            if line.contains(needle) {
                return (x, y);
            }
        }
        panic!("missing {needle}");
    }
}
