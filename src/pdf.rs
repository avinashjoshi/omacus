//! A preview, as a PDF. Standard Helvetica, so it opens everywhere and needs no
//! embedded font. Characters Helvetica can't draw become a close ASCII stand-in.

use std::path::{Path, PathBuf};

use omacus_engine::markdown::{Block, Inline, Item, Rich};

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
        vec!["BT /F1 11 Tf 72 740 Td ( ) Tj ET".to_string()]
    } else {
        pages
    };
    let mut objects: Vec<Vec<u8>> = Vec::new();
    // 1 catalog, 2 pages, 3 font, then content/page pairs.
    objects.push(Vec::new());
    objects.push(Vec::new());
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());

    let mut kids = Vec::new();
    for content in &pages {
        let content_id = objects.len() + 1;
        let page_id = content_id + 1;
        let data = format!("{content}\n");
        let stream = format!("<< /Length {} >>\nstream\n{data}endstream", data.len());
        objects.push(stream.into_bytes());
        let page = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {content_id} 0 R /Resources << /Font << /F1 3 0 R >> >> >>"
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

fn layout(blocks: &[Block]) -> Vec<String> {
    let mut pages = Vec::new();
    let mut lines: Vec<(f32, String)> = Vec::new();
    let push = |lines: &mut Vec<(f32, String)>, size: f32, text: &str, indent: f32| {
        for (n, row) in wrap(text, 88).into_iter().enumerate() {
            let shown = if n == 0 {
                row
            } else {
                format!("{}{row}", " ".repeat((indent / 4.0) as usize))
            };
            lines.push((size, shown));
        }
    };
    for block in blocks {
        match block {
            Block::Heading { level, rich } => {
                let size = match level {
                    1 => 18.0,
                    2 => 15.0,
                    _ => 13.0,
                };
                push(&mut lines, size, &with_answer(rich), 0.0);
                lines.push((8.0, String::new()));
            }
            Block::Lines(rows) => {
                for rich in rows {
                    push(&mut lines, 11.0, &with_answer(rich), 0.0);
                }
                lines.push((8.0, String::new()));
            }
            Block::List { items, .. } => {
                for item in items {
                    push(&mut lines, 11.0, &item_line(item), 16.0);
                }
                lines.push((8.0, String::new()));
            }
            Block::Quote(rows) => {
                for rich in rows {
                    push(&mut lines, 11.0, &format!("| {}", with_answer(rich)), 16.0);
                }
                lines.push((8.0, String::new()));
            }
            Block::Table { header, rows } => {
                for (n, row) in rows.iter().enumerate() {
                    let cells: Vec<String> = row.iter().map(|c| plain(c)).collect();
                    let line = cells.join("  |  ");
                    let line = if *header && n == 0 {
                        line.to_uppercase()
                    } else {
                        line
                    };
                    push(&mut lines, 11.0, &line, 0.0);
                }
                lines.push((8.0, String::new()));
            }
        }
    }
    while lines.last().is_some_and(|(_, s)| s.is_empty()) {
        lines.pop();
    }

    let mut y = 750.0;
    let mut ops = String::from("BT\n");
    let mut started = false;
    for (size, text) in lines {
        let leading = size + 5.0;
        if y - leading < 64.0 && started {
            ops.push_str("ET\n");
            pages.push(std::mem::take(&mut ops));
            ops = String::from("BT\n");
            y = 750.0;
            started = false;
        }
        if !started {
            ops.push_str(&format!("/F1 {size} Tf\n72 {y} Td\n"));
            started = true;
        } else {
            ops.push_str(&format!("/F1 {size} Tf\n0 -{leading} Td\n"));
        }
        y -= leading;
        ops.push_str(&format!("({}) Tj\n", pdf_text(&text)));
    }
    if started {
        ops.push_str("ET\n");
        pages.push(ops);
    }
    pages
}

fn item_line(item: &Item) -> String {
    let mark = match item.checked {
        Some(true) => "[x] ",
        Some(false) => "[ ] ",
        None => match item.number {
            Some(n) => return format!("{n}. {}", with_answer(&item.rich)),
            None => "- ",
        },
    };
    format!("{mark}{}", with_answer(&item.rich))
}

fn with_answer(rich: &Rich) -> String {
    let text = plain(&rich.inlines);
    match &rich.answer {
        Some(answer) if !answer.is_empty() => format!("{text}   [{answer}]"),
        _ => text,
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

fn wrap(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut rows = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let extra = if line.is_empty() { 0 } else { 1 };
        if !line.is_empty() && line.chars().count() + extra + word.chars().count() > width {
            rows.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
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

    #[test]
    fn a_pdf_holds_the_line_and_its_answer() {
        let blocks = render("- hotel = 3 nights * $180\n", &[Some("$540")]);
        let bytes = bytes("Kyoto", &blocks);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("hotel = 3 nights * $180"));
        assert!(text.contains("[$540]"));
        assert!(text.contains("%%EOF"));
    }
}
