//! A page is one plain-text document. Any line can be markdown, math, or both.
//!
//! The marker at the start (`#`, `-`, `*`, `1.`, `- [ ]`, `>`) is markdown. What
//! follows is math when it reads as a calculation, and prose otherwise. `#` at the
//! start of a line is a heading; `#` later on the line is a comment, like `//`,
//! except inside quotes and except a hex colour such as `#ff8800`.

/// How a line opens, before its words are read as math.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Blank,
    Comment,
    /// A table row. Rendered in the preview, and not calculated.
    Table,
    Heading(u8),
    Bullet,
    Numbered(u32),
    /// A task list item. `true` when the box is checked.
    Task(bool),
    Quote,
    /// An ordinary line: no markdown prefix.
    Text,
}

/// A line split into its markdown and the words a calculation sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Classified<'a> {
    pub form: Form,
    /// The words after the prefix, with a trailing comment removed.
    pub body: &'a str,
    /// Byte index of a `//` or mid-line `#` comment.
    pub comment_at: Option<usize>,
}

/// Split `line` into a markdown prefix, the text after it, and a trailing comment.
pub fn classify(line: &str) -> Classified<'_> {
    let comment_at = find_comment(line);
    let code = match comment_at {
        Some(i) => &line[..i],
        None => line,
    };
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Classified {
            form: if comment_at.is_some() {
                Form::Comment
            } else {
                Form::Blank
            },
            body: "",
            comment_at,
        };
    }
    if trimmed.starts_with('|') {
        return Classified {
            form: Form::Table,
            body: "",
            comment_at,
        };
    }
    if crate::extras::is_color(trimmed) {
        return Classified {
            form: Form::Text,
            body: trimmed,
            comment_at,
        };
    }
    let (form, body) = peel(trimmed);
    Classified {
        form,
        body: body.trim(),
        comment_at,
    }
}

/// The words a calculation sees. Empty for a blank line, a comment, or a table row.
pub fn math_body(line: &str) -> &str {
    let c = classify(line);
    match c.form {
        Form::Blank | Form::Comment | Form::Table => "",
        _ => c.body,
    }
}

fn peel(trimmed: &str) -> (Form, &str) {
    let mut form = None;
    let mut rest = trimmed;
    // A quote can wrap a list item (`> - hotel = …`). The first marker decides
    // how the line is drawn; every marker comes off before the math.
    for _ in 0..8 {
        let Some((next, after)) = peel_one(rest) else {
            break;
        };
        if form.is_none() {
            form = Some(next);
        }
        rest = after.trim();
    }
    (form.unwrap_or(Form::Text), rest)
}

fn peel_one(s: &str) -> Option<(Form, &str)> {
    if s.starts_with('#') {
        let n = s.bytes().take_while(|b| *b == b'#').count();
        let rest = s[n..].trim_start();
        let level = n.clamp(1, 6) as u8;
        return Some((Form::Heading(level), rest));
    }
    if let Some(task) = task(s) {
        return Some(task);
    }
    if let Some(rest) = bullet_rest(s) {
        return Some((Form::Bullet, rest));
    }
    if let Some((n, rest)) = numbered(s) {
        return Some((Form::Numbered(n), rest));
    }
    if let Some(rest) = s.strip_prefix('>') {
        return Some((Form::Quote, rest));
    }
    None
}

fn bullet_rest(s: &str) -> Option<&str> {
    let rest = s.strip_prefix('-').or_else(|| s.strip_prefix('*'))?;
    if rest.starts_with(' ') || rest.starts_with('\t') {
        Some(rest)
    } else {
        None
    }
}

fn task(s: &str) -> Option<(Form, &str)> {
    let rest = bullet_rest(s)?.trim_start();
    let rest = rest.strip_prefix('[')?;
    let mut chars = rest.chars();
    let mark = chars.next()?;
    if !matches!(mark, ' ' | 'x' | 'X') {
        return None;
    }
    let after = &rest[mark.len_utf8()..];
    let after = after.strip_prefix(']')?;
    if !after.is_empty() && !after.starts_with(char::is_whitespace) {
        return None;
    }
    Some((Form::Task(matches!(mark, 'x' | 'X')), after))
}

fn numbered(s: &str) -> Option<(u32, &str)> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 6 {
        return None;
    }
    let after_digits = &s[digits..];
    let rest = after_digits.strip_prefix('.')?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let n = s[..digits].parse().ok()?;
    Some((n, rest))
}

/// Index of a comment marker. A `#` at the start of the line is a heading, and a
/// `#` that begins a hex colour (`#ff8800`, `#f80`) is that colour.
fn find_comment(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut seen = false;
    let mut in_quote = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_quote {
            if c == b'"' {
                in_quote = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_quote = true;
            seen = true;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return Some(i);
        }
        if c == b'#' {
            if hex_color_at(line, i) {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                seen = true;
                continue;
            }
            if !seen {
                while i < bytes.len() && bytes[i] == b'#' {
                    i += 1;
                }
                seen = true;
                continue;
            }
            return Some(i);
        }
        seen = true;
        // Advance one UTF-8 character so a multibyte letter isn't split.
        i += utf8_len(c);
    }
    None
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first < 0xE0 {
        2
    } else if first < 0xF0 {
        3
    } else {
        4
    }
}

/// `#` plus 3 or 6 hex digits on a boundary, and not a word of only letters
/// (`#bad`, `#cafe` stay headings or comments).
fn hex_color_at(line: &str, at: usize) -> bool {
    let rest = &line[at + 1..];
    let n = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
    if n != 3 && n != 6 {
        return false;
    }
    let hex = &rest[..n];
    let boundary = rest[n..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_ascii_alphanumeric());
    boundary && (n == 6 || hex.bytes().any(|b| b.is_ascii_digit()))
}

// -------------------------------------------------------------------------------------
// Emphasis, for the light styling of edit mode. Ranges are byte offsets into the
// original line. Markers stay in the text so the cursor doesn't drift.

/// Bold and italic spans on one line, comments excluded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Emphasis {
    pub bold: Vec<(usize, usize)>,
    pub italic: Vec<(usize, usize)>,
    /// The `**` and `*` characters themselves, drawn quieter than the words.
    pub markers: Vec<(usize, usize)>,
}

pub fn emphasis(line: &str) -> Emphasis {
    let end = find_comment(line).unwrap_or(line.len());
    let bytes = line.as_bytes();
    let mut out = Emphasis::default();
    let mut i = 0;
    while i < end {
        if bytes[i] == b'`'
            && let Some(close) = find_byte(bytes, i + 1, end, b'`')
        {
            i = close + 1;
            continue;
        }
        if let Some((marker, len)) = opener(bytes, i, end)
            && let Some(close) = find_marker(bytes, i + len, end, marker)
        {
            let inner_start = i + len;
            let inner_end = close;
            let inner = &line[inner_start..inner_end];
            if emphasis_inner(inner) {
                let span = (inner_start, inner_end);
                if len == 2 {
                    out.bold.push(span);
                } else if italic_ok(line, i, close + 1, marker) {
                    out.italic.push(span);
                } else {
                    i += 1;
                    continue;
                }
                out.markers.push((i, i + len));
                out.markers.push((close, close + len));
                i = close + len;
                continue;
            }
        }
        i += utf8_len(bytes[i]);
    }
    out
}

fn opener(bytes: &[u8], i: usize, end: usize) -> Option<(&'static [u8], usize)> {
    if i + 1 < end && bytes[i] == b'*' && bytes[i + 1] == b'*' {
        return Some((b"**", 2));
    }
    if i + 1 < end && bytes[i] == b'_' && bytes[i + 1] == b'_' {
        return Some((b"__", 2));
    }
    if bytes[i] == b'*' {
        return Some((b"*", 1));
    }
    if bytes[i] == b'_' {
        return Some((b"_", 1));
    }
    None
}

fn find_marker(bytes: &[u8], from: usize, end: usize, marker: &[u8]) -> Option<usize> {
    let mut i = from;
    while i + marker.len() <= end {
        if &bytes[i..i + marker.len()] == marker {
            // `**` inside a search for `*` would be the bold marker, not italic.
            if marker == b"*" && i + 1 < end && bytes[i + 1] == b'*' {
                i += 2;
                continue;
            }
            if marker == b"_" && i + 1 < end && bytes[i + 1] == b'_' {
                i += 2;
                continue;
            }
            return Some(i);
        }
        i += utf8_len(bytes[i]);
    }
    None
}

fn find_byte(bytes: &[u8], from: usize, end: usize, want: u8) -> Option<usize> {
    bytes[from..end]
        .iter()
        .position(|b| *b == want)
        .map(|p| from + p)
}

fn emphasis_inner(inner: &str) -> bool {
    !inner.is_empty()
        && !inner.starts_with(char::is_whitespace)
        && !inner.ends_with(char::is_whitespace)
}

/// `_word_` is italic. `hotel_name` is a name, and `2 * 3` has no closing star.
fn italic_ok(line: &str, open: usize, after_close: usize, marker: &[u8]) -> bool {
    if marker == b"_" || marker == b"*" {
        let before = line[..open].chars().next_back();
        let after = line[after_close..].chars().next();
        let glued = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if glued(before) || glued(after) {
            return false;
        }
    }
    true
}

/// Byte offset → UTF-16 offset, for the browser editor.
pub fn utf16_offset(line: &str, byte: usize) -> usize {
    line[..byte.min(line.len())]
        .chars()
        .map(char::len_utf16)
        .sum()
}

// -------------------------------------------------------------------------------------
// A rendered page: headings, lists, quotes and tables, with each answer beside its line.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Strong(String),
    Em(String),
    Code(String),
    Link { text: String, href: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rich {
    pub inlines: Vec<Inline>,
    pub answer: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub checked: Option<bool>,
    pub number: Option<u32>,
    pub rich: Rich,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        rich: Rich,
    },
    Lines(Vec<Rich>),
    List {
        ordered: bool,
        items: Vec<Item>,
    },
    Quote(Vec<Rich>),
    Table {
        header: bool,
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

/// Render `text` for the preview. `answers` lines up with the lines of `text`;
/// `None` means that line has nothing to show. Table cells are not calculated.
pub fn render(text: &str, answers: &[Option<&str>]) -> Vec<Block> {
    let lines: Vec<&str> = text.split('\n').collect();
    let answer = |i: usize| {
        answers
            .get(i)
            .and_then(|a| *a)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let class = classify(lines[i]);
        match class.form {
            Form::Blank | Form::Comment => i += 1,
            Form::Table => {
                let start = i;
                while i < lines.len() && matches!(classify(lines[i]).form, Form::Table) {
                    i += 1;
                }
                if let Some(table) = table_block(&lines[start..i]) {
                    blocks.push(table);
                }
            }
            Form::Heading(level) => {
                blocks.push(Block::Heading {
                    level,
                    rich: rich(class.body, answer(i)),
                });
                i += 1;
            }
            Form::Bullet | Form::Task(_) => {
                let mut items = Vec::new();
                while i < lines.len() {
                    let c = classify(lines[i]);
                    match c.form {
                        Form::Bullet => items.push(Item {
                            checked: None,
                            number: None,
                            rich: rich(c.body, answer(i)),
                        }),
                        Form::Task(checked) => items.push(Item {
                            checked: Some(checked),
                            number: None,
                            rich: rich(c.body, answer(i)),
                        }),
                        _ => break,
                    }
                    i += 1;
                }
                blocks.push(Block::List {
                    ordered: false,
                    items,
                });
            }
            Form::Numbered(_) => {
                let mut items = Vec::new();
                while i < lines.len() {
                    let c = classify(lines[i]);
                    let Form::Numbered(n) = c.form else {
                        break;
                    };
                    items.push(Item {
                        checked: None,
                        number: Some(n),
                        rich: rich(c.body, answer(i)),
                    });
                    i += 1;
                }
                blocks.push(Block::List {
                    ordered: true,
                    items,
                });
            }
            Form::Quote => {
                let mut rows = Vec::new();
                while i < lines.len() {
                    let c = classify(lines[i]);
                    if !matches!(c.form, Form::Quote) {
                        break;
                    }
                    rows.push(rich(c.body, answer(i)));
                    i += 1;
                }
                blocks.push(Block::Quote(rows));
            }
            Form::Text => {
                let mut rows = Vec::new();
                while i < lines.len() {
                    let c = classify(lines[i]);
                    if !matches!(c.form, Form::Text) {
                        break;
                    }
                    rows.push(rich(c.body, answer(i)));
                    i += 1;
                }
                blocks.push(Block::Lines(rows));
            }
        }
    }
    blocks
}

fn rich(body: &str, answer: Option<String>) -> Rich {
    Rich {
        inlines: inlines(body),
        answer,
    }
}

fn table_block(lines: &[&str]) -> Option<Block> {
    let parsed: Vec<Vec<String>> = lines.iter().map(|l| table_cells(l)).collect();
    if parsed.is_empty() {
        return None;
    }
    let rule = parsed.iter().position(|row| is_rule(row));
    let rows: Vec<Vec<String>> = match rule {
        Some(0) => parsed.into_iter().skip(1).filter(|r| !is_rule(r)).collect(),
        Some(i) => {
            let mut rows = vec![parsed[0].clone()];
            rows.extend(parsed.into_iter().skip(i + 1).filter(|r| !is_rule(r)));
            rows
        }
        None => parsed.into_iter().filter(|r| !is_rule(r)).collect(),
    };
    if rows.is_empty() {
        return None;
    }
    Some(Block::Table {
        header: rule.is_some_and(|i| i > 0),
        rows: rows
            .into_iter()
            .map(|r| r.into_iter().map(|cell| inlines(&cell)).collect())
            .collect(),
    })
}

fn table_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim().trim_start_matches('|');
    let trimmed = trimmed.trim_end().trim_end_matches('|');
    if trimmed.trim().is_empty() && line.trim().starts_with('|') {
        return Vec::new();
    }
    trimmed.split('|').map(|c| c.trim().to_string()).collect()
}

fn is_rule(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|c| {
            let t = c.trim().trim_matches(':');
            !t.is_empty() && t.chars().all(|ch| ch == '-')
        })
}

fn inlines(s: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut Vec<Inline>| {
        if !plain.is_empty() {
            out.push(Inline::Text(std::mem::take(plain)));
        }
    };
    while i < bytes.len() {
        if bytes[i] == b'`'
            && let Some(close) = find_byte(bytes, i + 1, bytes.len(), b'`')
        {
            flush(&mut plain, &mut out);
            out.push(Inline::Code(s[i + 1..close].to_string()));
            i = close + 1;
            continue;
        }
        if bytes[i] == b'['
            && let Some((text, href, next)) = link_at(s, i)
        {
            flush(&mut plain, &mut out);
            out.push(Inline::Link { text, href });
            i = next;
            continue;
        }
        if let Some((marker, len)) = opener(bytes, i, bytes.len())
            && let Some(close) = find_marker(bytes, i + len, bytes.len(), marker)
        {
            let inner = &s[i + len..close];
            if emphasis_inner(inner) && italic_ok(s, i, close + len, marker) {
                flush(&mut plain, &mut out);
                if len == 2 {
                    out.push(Inline::Strong(inner.to_string()));
                } else {
                    out.push(Inline::Em(inner.to_string()));
                }
                i = close + len;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }
    flush(&mut plain, &mut out);
    if out.is_empty() {
        out.push(Inline::Text(String::new()));
    }
    out
}

fn link_at(s: &str, i: usize) -> Option<(String, String, usize)> {
    let rest = &s[i + 1..];
    let end_text = rest.find(']')?;
    let after = &rest[end_text + 1..];
    if !after.starts_with('(') {
        return None;
    }
    let end_href = after.find(')')?;
    let text = rest[..end_text].to_string();
    let href = after[1..end_href].trim().to_string();
    if !(href.starts_with("https://") || href.starts_with("http://") || href.starts_with("mailto:"))
    {
        return None;
    }
    if text.is_empty() {
        return None;
    }
    let next = i + 1 + end_text + 1 + end_href + 1;
    Some((text, href, next))
}

/// The preview as HTML: one fragment, each answer as `[ value ]` in a button.
pub fn preview_html(text: &str, answers: &[Option<&str>]) -> String {
    let mut html = String::from("<article class=\"md\">");
    for block in render(text, answers) {
        match block {
            Block::Heading { level, rich } => {
                let level = level.clamp(1, 6);
                html.push_str(&format!("<h{level}>"));
                push_rich(&mut html, &rich);
                html.push_str(&format!("</h{level}>"));
            }
            Block::Lines(rows) => {
                for rich in rows {
                    html.push_str("<p class=\"line\">");
                    push_rich(&mut html, &rich);
                    html.push_str("</p>");
                }
            }
            Block::List { ordered, items } => {
                html.push_str(if ordered { "<ol>" } else { "<ul>" });
                for item in items {
                    html.push_str("<li");
                    if let Some(n) = item.number {
                        html.push_str(&format!(" value=\"{n}\""));
                    }
                    if item.checked.is_some() {
                        html.push_str(" class=\"task\"");
                    }
                    html.push('>');
                    if let Some(checked) = item.checked {
                        html.push_str("<input type=\"checkbox\" disabled");
                        if checked {
                            html.push_str(" checked");
                        }
                        html.push('>');
                    }
                    push_rich(&mut html, &item.rich);
                    html.push_str("</li>");
                }
                html.push_str(if ordered { "</ol>" } else { "</ul>" });
            }
            Block::Quote(rows) => {
                html.push_str("<blockquote>");
                for rich in rows {
                    html.push_str("<p class=\"line\">");
                    push_rich(&mut html, &rich);
                    html.push_str("</p>");
                }
                html.push_str("</blockquote>");
            }
            Block::Table { header, rows } => {
                html.push_str("<table>");
                for (n, row) in rows.iter().enumerate() {
                    let cell = if header && n == 0 { "th" } else { "td" };
                    html.push_str("<tr>");
                    for cell_inlines in row {
                        html.push_str(&format!("<{cell}>"));
                        push_inlines(&mut html, cell_inlines);
                        html.push_str(&format!("</{cell}>"));
                    }
                    html.push_str("</tr>");
                }
                html.push_str("</table>");
            }
        }
    }
    html.push_str("</article>");
    html
}

fn push_rich(html: &mut String, rich: &Rich) {
    push_inlines(html, &rich.inlines);
    if let Some(answer) = &rich.answer {
        // The button's text is the brackets Avi wants. `data-value` is the raw
        // answer, so a click copies that and not the brackets.
        html.push_str("<button type=\"button\" class=\"pill\" data-value=\"");
        html.push_str(&escape(answer));
        html.push_str("\">[ ");
        html.push_str(&escape(answer));
        html.push_str(" ]</button>");
    }
}

fn push_inlines(html: &mut String, inlines: &[Inline]) {
    for inline in inlines {
        match inline {
            Inline::Text(s) => html.push_str(&escape(s)),
            Inline::Strong(s) => {
                html.push_str("<strong>");
                html.push_str(&escape(s));
                html.push_str("</strong>");
            }
            Inline::Em(s) => {
                html.push_str("<em>");
                html.push_str(&escape(s));
                html.push_str("</em>");
            }
            Inline::Code(s) => {
                html.push_str("<code>");
                html.push_str(&escape(s));
                html.push_str("</code>");
            }
            Inline::Link { text, href } => {
                html.push_str("<a href=\"");
                html.push_str(&escape(href));
                html.push_str("\">");
                html.push_str(&escape(text));
                html.push_str("</a>");
            }
        }
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_and_comments() {
        let c = classify("- hotel = 3 nights * $180");
        assert_eq!(c.form, Form::Bullet);
        assert_eq!(c.body, "hotel = 3 nights * $180");

        let c = classify("- [x] packed");
        assert_eq!(c.form, Form::Task(true));
        assert_eq!(c.body, "packed");

        let c = classify("- [ ] later");
        assert_eq!(c.form, Form::Task(false));
        assert_eq!(c.body, "later");

        let c = classify("## Week 1");
        assert_eq!(c.form, Form::Heading(2));
        assert_eq!(c.body, "Week 1");

        let c = classify("1. taxi = $12");
        assert_eq!(c.form, Form::Numbered(1));
        assert_eq!(c.body, "taxi = $12");

        let c = classify("> tip = 10% of $40");
        assert_eq!(c.form, Form::Quote);
        assert_eq!(c.body, "tip = 10% of $40");

        let c = classify("> - hotel = $5");
        assert_eq!(c.form, Form::Quote);
        assert_eq!(c.body, "hotel = $5");

        let c = classify("rent = $100 # monthly");
        assert_eq!(c.body, "rent = $100");
        assert!(c.comment_at.is_some());

        let c = classify("# Trip # draft");
        assert_eq!(c.form, Form::Heading(1));
        assert_eq!(c.body, "Trip");

        let c = classify("#ff8800 in rgb");
        assert_eq!(c.form, Form::Text);
        assert_eq!(c.body, "#ff8800 in rgb");
        assert!(c.comment_at.is_none());

        let c = classify("\"say # hi\" 1 + 1");
        assert!(c.comment_at.is_none());
        assert_eq!(c.body, "\"say # hi\" 1 + 1");

        let c = classify("| nights | rate |");
        assert_eq!(c.form, Form::Table);

        let c = classify("-$2.50");
        assert_eq!(c.form, Form::Text);
        assert_eq!(c.body, "-$2.50");

        let c = classify("// a note");
        assert_eq!(c.form, Form::Comment);
    }

    #[test]
    fn bold_does_not_eat_multiplication() {
        let e = emphasis("2 * 3");
        assert!(e.italic.is_empty());
        assert!(e.bold.is_empty());
        let e = emphasis("a **bold** word and *one* more");
        assert_eq!(e.bold, vec![(4, 8)]);
        assert_eq!(e.italic, vec![(21, 24)]);
        let e = emphasis("hotel_name = 1");
        assert!(e.italic.is_empty());
    }

    #[test]
    fn preview_puts_a_pill_on_the_line_and_renders_a_table() {
        let html = preview_html("- hotel = 3 nights * $180\n", &[Some("$540")]);
        assert!(html.contains("<li>"));
        assert!(html.contains("hotel = 3 nights * $180"));
        assert!(html.contains(
            "<button type=\"button\" class=\"pill\" data-value=\"$540\">[ $540 ]</button>"
        ));

        let quoted = preview_html("say **hello**\n", &[None]);
        assert!(quoted.contains("<strong>hello</strong>"));

        let table = preview_html("| a | b |\n| --- | --- |\n| 1 | 2 |\n", &[None, None, None]);
        assert!(table.contains("<th>a</th>"));
        assert!(table.contains("<td>1</td>"));
        assert!(!table.contains("pill"));
    }

    #[test]
    fn preview_escapes_html() {
        let html = preview_html("2 < 3 & 4\n", &[Some("true < false")]);
        assert!(html.contains("2 &lt; 3 &amp; 4"));
        assert!(html.contains("data-value=\"true &lt; false\">[ true &lt; false ]</button>"));
        assert!(!html.contains("<script"));
    }
}
