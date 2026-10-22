//! A preview, as a PDF. Standard Helvetica, so it opens everywhere and needs no
//! embedded font. Characters Helvetica can't draw become a close ASCII stand-in.
//!
//! The page is set like a short document: a real margin, a heading hierarchy,
//! body leading that can be read, and each answer in brackets in a dark ink
//! that still shows up when the page is printed in black and white. A wrapped
//! line, a list item and a table row move to the next page together.

use std::path::{Path, PathBuf};

use omacus_engine::markdown::{Block, Inline, Item, Rich};

const PAGE_W: f32 = 612.0;
const PAGE_H: f32 = 792.0;
const MARGIN_X: f32 = 68.0;
const MARGIN_TOP: f32 = 72.0;
const MARGIN_BOTTOM: f32 = 68.0;
const CONTENT_W: f32 = PAGE_W - MARGIN_X * 2.0;
/// Dark blue. It stays readable when a printer turns the page grey.
const ANSWER_INK: &str = "0.102 0.227 0.357";

/// Write the preview to `path`.
pub fn write(title: &str, blocks: &[Block], path: &Path) -> Result<(), String> {
    let bytes = bytes(title, blocks);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

/// Save into Downloads (or the temp directory) and, when `open` is set, open it.
pub fn save(title: &str, blocks: &[Block], open: bool) -> Result<PathBuf, String> {
    let dir = if open {
        std::env::temp_dir()
    } else {
        dirs::download_dir().unwrap_or_else(std::env::temp_dir)
    };
    let path = unique_path(&dir, &file_name(title));
    write(title, blocks, &path)?;
    if open {
        open_path(&path)?;
    }
    Ok(path)
}

pub fn bytes(title: &str, blocks: &[Block]) -> Vec<u8> {
    let pages = layout(blocks);
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

struct Span {
    text: String,
    colored: bool,
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
}

struct Group {
    gap: f32,
    runs: Vec<Run>,
    rule: Rule,
    /// A thin bar in the left margin, for a quoted line.
    bar: bool,
}

fn layout(blocks: &[Block]) -> Vec<String> {
    paginate(&groups_of(blocks))
}

fn groups_of(blocks: &[Block]) -> Vec<Group> {
    let mut groups = Vec::new();
    // Space carried onto the next block, so a paragraph doesn't glue itself
    // to the heading that follows.
    let mut after = 0.0;
    let push = |groups: &mut Vec<Group>, mut group: Group, after: &mut f32| {
        group.gap += *after;
        *after = 0.0;
        groups.push(group);
    };
    for block in blocks {
        match block {
            Block::Heading { level, rich } => {
                let (size, leading, gap) = match level {
                    1 => (20.0, 28.0, 18.0),
                    2 => (15.0, 22.0, 16.0),
                    _ => (12.5, 18.0, 12.0),
                };
                push(
                    &mut groups,
                    Group {
                        gap,
                        runs: runs_for(rich, size, leading, true, false, 0.0),
                        rule: Rule::None,
                        bar: false,
                    },
                    &mut after,
                );
                after = 6.0;
            }
            Block::Lines(rows) => {
                for rich in rows {
                    push(
                        &mut groups,
                        Group {
                            gap: 2.0,
                            runs: runs_for(rich, 11.0, 16.0, false, false, 0.0),
                            rule: Rule::None,
                            bar: false,
                        },
                        &mut after,
                    );
                }
                after = 8.0;
            }
            Block::List { items, .. } => {
                for item in items {
                    push(
                        &mut groups,
                        Group {
                            gap: 2.0,
                            runs: runs_for_text(
                                &item_line(item),
                                item.rich.answer.as_deref(),
                                11.0,
                                16.0,
                                false,
                                false,
                                4.0,
                            ),
                            rule: Rule::None,
                            bar: false,
                        },
                        &mut after,
                    );
                }
                after = 8.0;
            }
            Block::Quote(rows) => {
                for rich in rows {
                    push(
                        &mut groups,
                        Group {
                            gap: 1.0,
                            runs: runs_for(rich, 11.0, 16.0, false, true, 14.0),
                            rule: Rule::None,
                            bar: true,
                        },
                        &mut after,
                    );
                }
                after = 8.0;
            }
            Block::Table { header, rows } => {
                let plain_rows: Vec<Vec<String>> = rows
                    .iter()
                    .map(|r| r.iter().map(|c| plain(c)).collect())
                    .collect();
                let widths = table_widths(&plain_rows);
                for (n, cells) in plain_rows.iter().enumerate() {
                    let head = *header && n == 0;
                    push(
                        &mut groups,
                        Group {
                            gap: if n == 0 { 6.0 } else { 1.0 },
                            runs: runs_for_text(
                                &format_row(cells, &widths),
                                None,
                                10.0,
                                15.0,
                                head,
                                false,
                                0.0,
                            ),
                            rule: if head { Rule::Strong } else { Rule::Light },
                            bar: false,
                        },
                        &mut after,
                    );
                }
                after = 10.0;
            }
        }
    }
    if let Some(first) = groups.first_mut() {
        first.gap = 0.0;
    }
    groups
}

fn runs_for(
    rich: &Rich,
    size: f32,
    leading: f32,
    bold: bool,
    oblique: bool,
    indent: f32,
) -> Vec<Run> {
    runs_for_text(
        &plain(&rich.inlines),
        rich.answer.as_deref(),
        size,
        leading,
        bold,
        oblique,
        indent,
    )
}

fn runs_for_text(
    text: &str,
    answer: Option<&str>,
    size: f32,
    leading: f32,
    bold: bool,
    oblique: bool,
    indent: f32,
) -> Vec<Run> {
    let answer = answer.filter(|a| !a.is_empty()).map(bracket);
    let width = wrap_chars(size, indent);
    let rows = wrap(text, width);
    let mut runs = Vec::new();
    let last = rows.len().saturating_sub(1);
    for (i, row) in rows.into_iter().enumerate() {
        let hang = if i == 0 { indent } else { indent + 14.0 };
        let mut spans = vec![Span {
            text: row,
            colored: false,
        }];
        if i == last
            && let Some(answer) = &answer
        {
            let sep = usize::from(!spans[0].text.is_empty());
            let used = spans[0].text.chars().count() + sep + answer.chars().count();
            if used <= width {
                if sep == 1 {
                    spans[0].text.push_str("   ");
                }
                spans.push(Span {
                    text: answer.clone(),
                    colored: true,
                });
            } else {
                runs.push(run(size, leading, bold, oblique, hang, spans));
                runs.push(run(
                    size,
                    leading,
                    bold,
                    oblique,
                    indent + 14.0,
                    vec![Span {
                        text: answer.clone(),
                        colored: true,
                    }],
                ));
                return runs;
            }
        }
        runs.push(run(size, leading, bold, oblique, hang, spans));
    }
    if runs.is_empty() {
        runs.push(run(
            size,
            leading,
            bold,
            oblique,
            indent,
            vec![Span {
                text: String::new(),
                colored: false,
            }],
        ));
    }
    runs
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

fn bracket(answer: &str) -> String {
    format!("[ {answer} ]")
}

fn item_line(item: &Item) -> String {
    let body = plain(&item.rich.inlines);
    match item.checked {
        Some(true) => format!("[x] {body}"),
        Some(false) => format!("[ ] {body}"),
        None => match item.number {
            Some(n) => format!("{n}. {body}"),
            None => format!("- {body}"),
        },
    }
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

fn wrap_chars(size: f32, indent: f32) -> usize {
    let width = (CONTENT_W - indent).max(80.0);
    ((width / (size * 0.52)) as usize).max(16)
}

/// Groups that fit on a fresh page move there whole, so a wrapped line is not
/// torn across the page break.
fn paginate(groups: &[Group]) -> Vec<String> {
    let top = PAGE_H - MARGIN_TOP;
    let usable = top - MARGIN_BOTTOM;
    let mut pages = Vec::new();
    let mut ops = String::new();
    let mut y = top;
    let mut started = false;
    let mut in_text = false;

    for group in groups {
        let rule_room = match group.rule {
            Rule::None => 0.0,
            _ => 5.0,
        };
        let at_top = (y - top).abs() < 0.5;
        let gap = if at_top { 0.0 } else { group.gap };
        let body = gap + group.runs.iter().map(|r| r.leading).sum::<f32>() + rule_room;
        if started && y - body < MARGIN_BOTTOM && body <= usable {
            close_text(&mut ops, &mut in_text);
            pages.push(std::mem::take(&mut ops));
            y = top;
        }
        let gap = if (y - top).abs() < 0.5 {
            0.0
        } else {
            group.gap
        };
        y -= gap;
        let bar_top = y;
        for run in &group.runs {
            let baseline = y - run.size * 0.80;
            ensure_text(&mut ops, &mut in_text);
            let font = if run.bold {
                "F2"
            } else if run.oblique {
                "F3"
            } else {
                "F1"
            };
            ops.push_str(&format!(
                "/{font} {} Tf\n0 0 0 rg\n1 0 0 1 {} {} Tm\n",
                num(run.size),
                num(MARGIN_X + run.indent),
                num(baseline),
            ));
            for span in &run.spans {
                if span.text.is_empty() {
                    continue;
                }
                if span.colored {
                    ops.push_str(ANSWER_INK);
                    ops.push_str(" rg\n");
                }
                ops.push_str(&format!("({}) Tj\n", pdf_text(&span.text)));
                if span.colored {
                    ops.push_str("0 0 0 rg\n");
                }
            }
            y -= run.leading;
        }
        if group.bar {
            close_text(&mut ops, &mut in_text);
            let x = MARGIN_X + 2.0;
            ops.push_str(&format!(
                "{ANSWER_INK} RG\n1 w\n{} {} m {} {} l S\n",
                num(x),
                num(bar_top - 2.0),
                num(x),
                num(y + 4.0),
            ));
        }
        if group.rule != Rule::None {
            close_text(&mut ops, &mut in_text);
            let (gray, width) = match group.rule {
                Rule::Strong => ("0.16", "0.9"),
                _ => ("0.78", "0.4"),
            };
            let rule_y = y + 3.0;
            ops.push_str(&format!(
                "{gray} {gray} {gray} RG\n{width} w\n{} {} m {} {} l S\n",
                num(MARGIN_X),
                num(rule_y),
                num(PAGE_W - MARGIN_X),
                num(rule_y),
            ));
            y -= rule_room;
        }
        started = true;
    }
    close_text(&mut ops, &mut in_text);
    if started {
        pages.push(ops);
    }
    pages
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

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let direct = dir.join(name);
    if !direct.exists() {
        return direct;
    }
    let stem = name.trim_end_matches(".pdf");
    for n in 2..1000 {
        let path = dir.join(format!("{stem} {n}.pdf"));
        if !path.exists() {
            return path;
        }
    }
    dir.join(name)
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
        let bytes = bytes("Kyoto", &blocks);
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
        let text = String::from_utf8_lossy(&bytes("Kyoto", &render(&source, &refs))).into_owned();
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
        let bytes = bytes("Northwind trip", &blocks);
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
        let text = String::from_utf8_lossy(&bytes("Trip", &blocks)).into_owned();
        assert!(text.contains("/F2 20 Tf"), "{text}");
        assert!(
            text.contains("0.16 0.16 0.16 RG"),
            "the header rule is missing"
        );
    }
}
