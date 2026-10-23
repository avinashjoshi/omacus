//! User-facing copy says "page", not "sheet". Class names, the source file
//! `src/sheets.rs`, and one sentence about the old `sheets` folder are the
//! exceptions: the files on disk keep their format.

use std::path::Path;

#[test]
fn site_readme_and_docs_say_page() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![
        root.join("README.md"),
        root.join("site/llms.txt"),
        root.join("site/llms-full.txt"),
    ];
    collect_html(&root.join("site"), &mut files);
    let mut problems = Vec::new();
    for path in files {
        // The updates page repeats each release's own notes, including the one
        // that renamed sheets to pages. That record is not rewritten.
        if path.starts_with(root.join("site/updates")) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let visible = strip_non_copy(&text);
        if let Some(hit) = first_sheet_word(&visible) {
            problems.push(format!(
                "{}: …{}…",
                path.strip_prefix(root).unwrap_or(&path).display(),
                hit
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "still says sheet:\n{}",
        problems.join("\n")
    );
}

#[test]
fn web_app_sentences_say_page() {
    let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("site/app/app.js"))
        .unwrap();
    for phrase in [
        "New sheet",
        "your sheets",
        "this sheet",
        "Sheet copied",
        "long sheet",
        "Delete this sheet",
        "aria-label\": \"Sheet\"",
        "keeps your sheets",
    ] {
        assert!(!js.contains(phrase), "app.js still contains {phrase:?}");
    }
    assert!(js.contains("\"page:\""), "pages are stored under page:");
    assert!(
        js.contains("\"sheet:\""),
        "older sheet: keys are still read"
    );
}

fn collect_html(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_html(&path, out);
        } else if path.extension().is_some_and(|e| e == "html") {
            out.push(path);
        }
    }
}

/// Drop class names, data attributes, generator markers and style/script that
/// aren't sentences a person reads.
fn strip_non_copy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix("<!-- sheet ")
            && let Some(end) = stripped.find("-->")
        {
            rest = &stripped[end + 3..];
            continue;
        }
        if rest.starts_with("class=\"") || rest.starts_with("class='") {
            let quote = rest.chars().nth(6).unwrap();
            rest = &rest[7..];
            if let Some(end) = rest.find(quote) {
                rest = &rest[end + quote.len_utf8()..];
                continue;
            }
        }
        if let Some(after) = rest.strip_prefix("data-") {
            let name_end = after
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
                .unwrap_or(after.len());
            rest = &after[name_end..];
            if rest.starts_with("=\"") || rest.starts_with("='") {
                let quote = rest.chars().nth(1).unwrap();
                rest = &rest[2..];
                if let Some(end) = rest.find(quote) {
                    rest = &rest[end + quote.len_utf8()..];
                }
            }
            continue;
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

fn first_sheet_word(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut i = 0;
    while let Some(rel) = lower[i..].find("sheet") {
        let at = i + rel;
        let plural = lower[at..].starts_with("sheets");
        let end = at + if plural { 6 } else { 5 };
        let before = text[..at].chars().next_back();
        let after = text[end..].chars().next();
        let glued = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.';
        let inside_word = before.is_some_and(glued) || after.is_some_and(glued);
        let legacy = {
            let from = at.saturating_sub(80);
            let window = &lower[from..end.min(lower.len())];
            window.contains("folder named") || window.contains("src/")
        };
        if !inside_word && !legacy {
            let from = at.saturating_sub(40);
            let to = (end + 40).min(text.len());
            return Some(text[from..to].replace('\n', " "));
        }
        i = end;
    }
    None
}
