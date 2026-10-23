//! The Omacus engine for JavaScript: the same code as the desktop app, compiled to
//! WebAssembly for app.omacus.com. Results go back as JSON strings.

use std::cell::RefCell;
use std::collections::HashMap;

use omacus_engine::xref::{self, Page};
use omacus_engine::{Engine, Kind, LineResult, Rates, Sheet, complete};
use serde_json::json;
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    /// The browser's clock, for the engine's per-line time limit.
    #[wasm_bindgen(js_namespace = Date)]
    fn now() -> f64;
}

#[wasm_bindgen]
pub struct Calc {
    engine: Engine,
    rates: Rates,
    /// Each sheet's last calculation, so an edit redoes only the lines from the
    /// first one that changed.
    sheets: RefCell<HashMap<String, Sheet>>,
    /// The last resolved catalog, so a slice doesn't resolve every page again.
    book: RefCell<Option<xref::Book>>,
    book_fp: RefCell<u64>,
}

impl Default for Calc {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Calc {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        let rates = Rates::default();
        let engine = Engine::new(rates.clone());
        #[cfg(target_arch = "wasm32")]
        let engine = engine.with_clock(now);
        Self {
            engine,
            rates,
            sheets: RefCell::default(),
            book: RefCell::default(),
            book_fp: RefCell::new(0),
        }
    }

    /// Exchange rates as `{"EUR": 0.92, ...}` (units per 1 USD). Returns whether
    /// they were accepted.
    #[wasm_bindgen(js_name = setRates)]
    pub fn set_rates(&self, json: &str) -> bool {
        match serde_json::from_str::<HashMap<String, f64>>(json) {
            Ok(table) if !table.is_empty() => {
                self.rates.set(table);
                *self.book.borrow_mut() = None;
                true
            }
            _ => false,
        }
    }

    /// Every line's answer: `[{"kind": "value", "display": "$28", "color": null,
    /// "note": null}, ...]`, where `note` says why a line has no answer.
    pub fn evaluate(&self, text: &str) -> String {
        to_json(self.engine.eval_document(text))
    }

    /// The value under a UTF-16 cursor: `{"start", "end", "value"}` or `null`.
    /// `start` and `end` are UTF-16 offsets of the token.
    #[wasm_bindgen(js_name = hoverValue)]
    pub fn hover_value(&self, text: &str, utf16_pos: u32) -> String {
        self.hover_value_with(text, utf16_pos, "", "[]")
    }

    /// `hoverValue` with the other pages, so `@handle.name` includes its value
    /// and the source page. `pages_json` is an array of page objects.
    #[wasm_bindgen(js_name = hoverValueWith)]
    pub fn hover_value_with(
        &self,
        text: &str,
        utf16_pos: u32,
        handle: &str,
        pages_json: &str,
    ) -> String {
        let cursor = char_index_from_utf16(text, utf16_pos as usize);
        let pages = parse_pages(pages_json);
        match self.engine.hover_with(text, cursor, handle, &pages) {
            Some(hit) => json!({
                "start": utf16_from_chars(text, hit.start),
                "end": utf16_from_chars(text, hit.end),
                "value": hit.value,
                "sourceTitle": hit.source_title,
                "sourceHandle": hit.source_handle,
                "usedBy": hit.used_by,
            })
            .to_string(),
            None => "null".into(),
        }
    }

    /// Suggestions at a UTF-16 cursor. A `@` offers pages, then that page's
    /// variables; otherwise the word being typed, as `suggest` does.
    #[wasm_bindgen(js_name = suggestAt)]
    pub fn suggest_at(&self, text: &str, utf16_pos: u32, pages_json: &str) -> String {
        let cursor = char_index_from_utf16(text, utf16_pos as usize);
        let pages = parse_pages(pages_json);
        if let Some(pick) = xref::at_picks(&self.engine, text, cursor, &pages) {
            return suggestion_json(text, pick.start, &pick.items);
        }
        let Some((start, word)) = complete::word_before(text, cursor) else {
            return "null".into();
        };
        let items = complete::suggest(&word, &omacus_engine::defined_variables(text));
        if items.is_empty() {
            return "null".into();
        }
        suggestion_json(text, start, &items)
    }

    /// `evaluate`, reusing the last calculation of the sheet `id`.
    #[wasm_bindgen(js_name = evaluateSheet)]
    pub fn evaluate_sheet(&self, id: &str, text: &str) -> String {
        let mut sheets = self.sheets.borrow_mut();
        if sheets.len() > 32 && !sheets.contains_key(id) {
            sheets.clear();
        }
        let sheet = sheets.entry(id.to_string()).or_default();
        to_json(self.engine.eval_sheet(text, sheet))
    }

    /// `evaluateSheet` for at most about `ms` milliseconds (checked between lines, so
    /// one slow line can take its own time limit beyond that), as `{"done": bool,
    /// "results": [...]}`: every line's answer, or the first lines' when time ran out.
    /// Nothing is lost by stopping: the next call carries on from there, so a worker
    /// can answer messages (newer text) between slices.
    #[wasm_bindgen(js_name = evaluateSheetFor)]
    pub fn evaluate_sheet_for(&self, id: &str, text: &str, ms: f64) -> String {
        let mut sheets = self.sheets.borrow_mut();
        if sheets.len() > 32 && !sheets.contains_key(id) {
            sheets.clear();
        }
        let sheet = sheets.entry(id.to_string()).or_default();
        let deadline = clock() + ms;
        let done = self
            .engine
            .eval_sheet_until(text, sheet, || clock() > deadline)
            .is_some();
        format!(
            r#"{{"done":{done},"results":{}}}"#,
            to_json(sheet.done().to_vec())
        )
    }

    /// `evaluateSheet` with other pages installed, so `@handle.name` resolves.
    #[wasm_bindgen(js_name = evaluateSheetWith)]
    pub fn evaluate_sheet_with(
        &self,
        id: &str,
        handle: &str,
        text: &str,
        pages_json: &str,
    ) -> String {
        let _guard = self.install_pages(handle, pages_json);
        self.evaluate_sheet(id, text)
    }

    /// `evaluateSheetFor` with other pages installed.
    #[wasm_bindgen(js_name = evaluateSheetForWith)]
    pub fn evaluate_sheet_for_with(
        &self,
        id: &str,
        handle: &str,
        text: &str,
        pages_json: &str,
        ms: f64,
    ) -> String {
        let _guard = self.install_pages(handle, pages_json);
        self.evaluate_sheet_for(id, text, ms)
    }

    /// Start sheet `to` from sheet `from`'s saved work (for a copy of text that may
    /// differ from what's being edited, without losing either's progress).
    #[wasm_bindgen(js_name = forkSheet)]
    pub fn fork_sheet(&self, from: &str, to: &str) {
        let mut sheets = self.sheets.borrow_mut();
        let sheet = sheets.get(from).cloned().unwrap_or_default();
        sheets.insert(to.to_string(), sheet);
    }

    #[wasm_bindgen(js_name = dropSheet)]
    pub fn drop_sheet(&self, id: &str) {
        self.sheets.borrow_mut().remove(id);
    }

    /// The sheet with each answer lined up after its line.
    #[wasm_bindgen(js_name = withAnswers)]
    pub fn with_answers(&self, text: &str) -> String {
        omacus_engine::with_answers(text, &self.engine.eval_document(text))
    }

    /// The sheet as a Markdown table.
    #[wasm_bindgen(js_name = markdownTable)]
    pub fn markdown_table(&self, text: &str) -> String {
        omacus_engine::markdown_table(text, &self.engine.eval_document(text))
    }
}

impl Calc {
    /// Resolve `pages_json` (reusing the last book when nothing changed) and
    /// install it for the page being calculated.
    fn install_pages(&self, handle: &str, pages_json: &str) -> xref::Guard {
        let pages = parse_pages(pages_json);
        let fp = xref::fingerprint(&pages);
        let book = {
            let mut slot = self.book.borrow_mut();
            let mut seen = self.book_fp.borrow_mut();
            if slot.is_none() || *seen != fp {
                *slot = Some(xref::resolve(&self.engine, &pages));
                *seen = fp;
            }
            slot.clone().expect("book was just stored")
        };
        xref::install_for(&book, handle)
    }
}

/// Where each line of `new` was in `old` (the engine's `realign_map`), for moving
/// answers along with their lines: the old line's index, -1 for a changed line, or
/// -2 - index for a line below the edit, whose answer may be out of date.
#[wasm_bindgen(js_name = realignMap)]
pub fn realign_map(old: &str, new: &str) -> Vec<i32> {
    omacus_engine::realign_map(old, new)
        .into_iter()
        .map(|(from, stale)| match from {
            None => -1,
            Some(i) if stale => -2 - i as i32,
            Some(i) => i as i32,
        })
        .collect()
}

/// The browser's clock, or (in native tests) the engine's.
fn clock() -> f64 {
    #[cfg(target_arch = "wasm32")]
    return now();
    #[cfg(not(target_arch = "wasm32"))]
    return omacus_engine::monotonic_ms();
}

/// The sheet laid out with answers already worked out (`results` as `evaluate`
/// gives them), as text with the answers lined up or (`markdown`) as a Markdown
/// table. Nothing is calculated, so the page can copy within the click that asked.
#[wasm_bindgen(js_name = layoutAnswers)]
pub fn layout_answers(text: &str, results: &str, markdown: bool) -> String {
    let parsed: Vec<serde_json::Value> = serde_json::from_str(results).unwrap_or_default();
    let results: Vec<LineResult> = parsed
        .iter()
        .map(|r| LineResult {
            kind: match r["kind"].as_str() {
                Some("header") => Kind::Header,
                Some("comment") => Kind::Comment,
                Some("value") => Kind::Value,
                Some("none") => Kind::Error,
                _ => Kind::Empty,
            },
            display: r["display"].as_str().unwrap_or_default().to_string(),
            color: None,
            note: None,
            hint: false,
            clock: false,
            stale: false,
        })
        .collect();
    if markdown {
        omacus_engine::markdown_table(text, &results)
    } else {
        omacus_engine::with_answers(text, &results)
    }
}

fn char_index_from_utf16(text: &str, utf16: usize) -> usize {
    let mut seen = 0;
    for (i, ch) in text.chars().enumerate() {
        if seen >= utf16 {
            return i;
        }
        seen += ch.len_utf16();
    }
    text.chars().count()
}

fn utf16_from_chars(text: &str, chars: usize) -> usize {
    text.chars().take(chars).map(char::len_utf16).sum()
}

fn kind(k: &Kind) -> &'static str {
    match k {
        Kind::Empty => "empty",
        Kind::Header => "header",
        Kind::Comment => "comment",
        Kind::Value => "value",
        Kind::Error => "none",
    }
}

fn suggestion_json(
    text: &str,
    start: usize,
    items: &[omacus_engine::complete::Suggestion],
) -> String {
    let start = utf16_from_chars(text, start);
    let items: Vec<_> = items
        .iter()
        .map(|item| {
            json!({
                "text": item.text,
                "kind": item.kind,
                "detail": item.detail,
            })
        })
        .collect();
    json!({ "start": start, "items": items }).to_string()
}

fn parse_pages(json: &str) -> Vec<Page> {
    let Ok(raw) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    raw.into_iter()
        .map(|v| Page {
            id: v["id"].as_str().unwrap_or("").to_string(),
            handle: v["handle"].as_str().unwrap_or("").to_string(),
            title: v["title"].as_str().unwrap_or("").to_string(),
            text: v["text"].as_str().unwrap_or("").to_string(),
            locked: v["locked"].as_bool().unwrap_or(false),
            deleted: v["deleted"].as_bool().unwrap_or(false),
            elsewhere: v["elsewhere"].as_bool().unwrap_or(false),
        })
        .collect()
}

/// Suggestions for the word being typed. `before` is the sheet up to the cursor;
/// returns `{"start": <UTF-16 offset of the word>, "items": [...]}` or `null`.
#[wasm_bindgen]
pub fn suggest(before: &str, sheet: &str) -> String {
    let cursor = before.chars().count();
    let Some((start, word)) = complete::word_before(before, cursor) else {
        return "null".into();
    };
    let items = complete::suggest(&word, &omacus_engine::defined_variables(sheet));
    if items.is_empty() {
        return "null".into();
    }
    let start: usize = before.chars().take(start).map(char::len_utf16).sum();
    let items: Vec<_> = items
        .into_iter()
        .map(|item| {
            json!({
                "text": item.text,
                "kind": item.kind,
                "detail": item.detail,
            })
        })
        .collect();
    json!({ "start": start, "items": items }).to_string()
}

/// `keyword`, `unit`, `zone`, or `""`. Page variables are coloured by the editor.
#[wasm_bindgen(js_name = wordKind)]
pub fn word_kind(word: &str) -> String {
    complete::word_kind(word).unwrap_or("").to_string()
}

/// The `v1.…` part of a share link for a sheet (the same as the desktop app makes).
#[wasm_bindgen(js_name = shareEncode)]
pub fn share_encode(text: &str) -> String {
    omacus_engine::share::encode(text)
}

/// The sheet in a share link or its `v1.…` part, or `undefined` if it isn't one
/// (or would expand past 1 MB).
#[wasm_bindgen(js_name = shareDecode)]
pub fn share_decode(link: &str) -> Option<String> {
    omacus_engine::share::decode(link)
}

/// Whether a line is a header (`# Trip`) rather than a colour (`#ff8800`).
#[wasm_bindgen(js_name = isHeader)]
pub fn is_header(line: &str) -> bool {
    omacus_engine::is_header(line)
}

/// Editor marks for one line, with UTF-16 offsets (what the browser editor uses).
/// `heading` is 1–6 or null, `comment` is the offset of a `//` or mid-line `#`,
/// and `bold`, `italic` and `markers` are `[from, to]` spans.
#[wasm_bindgen(js_name = pageMarks)]
pub fn page_marks(line: &str) -> String {
    use omacus_engine::markdown::{self, Form};
    let class = markdown::classify(line);
    let emph = markdown::emphasis(line);
    let spans = |ranges: &[(usize, usize)]| {
        ranges
            .iter()
            .map(|(from, to)| {
                json!([
                    markdown::utf16_offset(line, *from),
                    markdown::utf16_offset(line, *to),
                ])
            })
            .collect::<Vec<_>>()
    };
    json!({
        "heading": match class.form {
            Form::Heading(level) => Some(level),
            _ => None,
        },
        "comment": class.comment_at.map(|at| markdown::utf16_offset(line, at)),
        "define": omacus_engine::definition_span(line).map(|(from, to)| {
            json!([
                markdown::utf16_offset(line, from),
                markdown::utf16_offset(line, to),
            ])
        }),
        "bold": spans(&emph.bold),
        "italic": spans(&emph.italic),
        "markers": spans(&emph.markers),
    })
    .to_string()
}

/// The preview as HTML. `answers_json` is the array `evaluate` returns; each
/// value (and a real error, not a name hint) becomes `[ value ]`.
#[wasm_bindgen(js_name = previewHtml)]
pub fn preview_html(text: &str, answers_json: &str) -> String {
    let refs = preview_answers(answers_json);
    let borrowed: Vec<Option<&str>> = refs.iter().map(|s| s.as_deref()).collect();
    omacus_engine::markdown::preview_html(text, &borrowed)
}

/// The same preview as JSON blocks, so the page can build DOM nodes.
/// `answers_json` matches `previewHtml`.
#[wasm_bindgen(js_name = previewBlocks)]
pub fn preview_blocks(text: &str, answers_json: &str) -> String {
    let refs = preview_answers(answers_json);
    let borrowed: Vec<Option<&str>> = refs.iter().map(|s| s.as_deref()).collect();
    let blocks = omacus_engine::markdown::render(text, &borrowed);
    serde_json::to_string(&blocks.iter().map(block_json).collect::<Vec<_>>())
        .unwrap_or_else(|_| "[]".into())
}

fn preview_answers(answers_json: &str) -> Vec<Option<String>> {
    let parsed: Vec<serde_json::Value> = serde_json::from_str(answers_json).unwrap_or_default();
    parsed.iter().map(pill_text).collect()
}

fn inline_json(inline: &omacus_engine::markdown::Inline) -> serde_json::Value {
    use omacus_engine::markdown::Inline;
    match inline {
        Inline::Text(s) => json!({"t": "text", "s": s}),
        Inline::Strong(s) => json!({"t": "strong", "s": s}),
        Inline::Em(s) => json!({"t": "em", "s": s}),
        Inline::Code(s) => json!({"t": "code", "s": s}),
        Inline::Link { text, href } => json!({"t": "link", "s": text, "href": href}),
    }
}

fn rich_json(rich: &omacus_engine::markdown::Rich) -> serde_json::Value {
    json!({
        "inlines": rich.inlines.iter().map(inline_json).collect::<Vec<_>>(),
        "comment": rich.comment,
        "answer": rich.answer,
    })
}

fn block_json(block: &omacus_engine::markdown::Block) -> serde_json::Value {
    use omacus_engine::markdown::Block;
    match block {
        Block::Heading { level, rich } => {
            json!({"t": "heading", "level": level, "rich": rich_json(rich)})
        }
        Block::Lines(rows) => {
            json!({"t": "lines", "rows": rows.iter().map(rich_json).collect::<Vec<_>>()})
        }
        Block::Blank => json!({"t": "blank"}),
        Block::List { ordered, items } => json!({
            "t": "list",
            "ordered": ordered,
            "items": items.iter().map(|item| json!({
                "checked": item.checked,
                "number": item.number,
                "rich": rich_json(&item.rich),
            })).collect::<Vec<_>>(),
        }),
        Block::Quote(rows) => {
            json!({"t": "quote", "rows": rows.iter().map(rich_json).collect::<Vec<_>>()})
        }
        Block::Code { lines } => json!({"t": "code", "lines": lines}),
        Block::Table { header, rows } => json!({
            "t": "table",
            "header": header,
            "rows": rows.iter().map(|row| row.iter().map(|cell| cell.iter().map(inline_json).collect::<Vec<_>>()).collect::<Vec<_>>()).collect::<Vec<_>>(),
        }),
    }
}

fn pill_text(result: &serde_json::Value) -> Option<String> {
    let kind = result["kind"].as_str().unwrap_or("");
    let display = result["display"].as_str().unwrap_or("");
    let hint = result["hint"].as_bool().unwrap_or(false);
    if kind == "value" && !display.is_empty() {
        return Some(display.to_string());
    }
    if kind == "none" && !hint {
        let note = result["note"].as_str().unwrap_or("");
        if !note.is_empty() {
            return Some(note.to_string());
        }
    }
    None
}

/// Names assigned in the sheet, for highlighting.
#[wasm_bindgen]
pub fn variables(sheet: &str) -> String {
    let vars: Vec<String> = omacus_engine::defined_variables(sheet)
        .into_iter()
        .collect();
    serde_json::to_string(&vars).unwrap_or_default()
}

/// The variable under a UTF-16 cursor: `{name, start, end, references, spans}`
/// or `null`. Offsets are UTF-16, which is what the browser editor uses.
#[wasm_bindgen(js_name = bindingAt)]
pub fn binding_at(text: &str, utf16_pos: u32) -> String {
    let cursor = char_index_from_utf16(text, utf16_pos as usize);
    match omacus_engine::rename::binding_at(text, cursor) {
        Some(binding) => json!({
            "name": binding.name,
            "start": utf16_from_chars(text, binding.token_start),
            "end": utf16_from_chars(text, binding.token_end),
            "references": binding.references,
            "definition": [
                utf16_from_chars(text, binding.definition.start),
                utf16_from_chars(text, binding.definition.end),
            ],
            "spans": utf16_spans(text, &binding.spans),
        })
        .to_string(),
        None => "null".into(),
    }
}

/// Rename the variable under a UTF-16 cursor. `{text, cursor}` on success
/// (`cursor` is the UTF-16 index just after that token), or `{error, message}`.
#[wasm_bindgen(js_name = renameVariable)]
pub fn rename_variable(text: &str, utf16_pos: u32, new_name: &str) -> String {
    let cursor = char_index_from_utf16(text, utf16_pos as usize);
    match omacus_engine::rename::rename_variable(text, cursor, new_name) {
        Ok(renamed) => json!({
            "text": renamed.text,
            "cursor": utf16_from_chars(&renamed.text, renamed.cursor),
        })
        .to_string(),
        Err(err) => {
            let code = match err {
                omacus_engine::rename::RenameFail::NotAVariable => "none",
                omacus_engine::rename::RenameFail::InvalidName => "invalid",
                omacus_engine::rename::RenameFail::Collision { .. } => "collision",
            };
            json!({ "error": code, "message": err.message() }).to_string()
        }
    }
}

/// A definition name edited in place: `{oldName, newName, references, spans,
/// collision, message}` or `null`. `spans` are the uses, in the new text.
#[wasm_bindgen(js_name = definitionChange)]
pub fn definition_change(old: &str, new: &str) -> String {
    match omacus_engine::rename::definition_change(old, new) {
        Some(edit) => json!({
            "oldName": edit.old_name,
            "newName": edit.new_name,
            "references": edit.references,
            "spans": utf16_spans(new, &edit.spans),
            "collision": edit.collision,
            "message": edit.message(),
        })
        .to_string(),
        None => "null".into(),
    }
}

/// The `@handle.name` under a UTF-16 cursor, or `null`. A bare `@name` is not one.
#[wasm_bindgen(js_name = pageRefAt)]
pub fn page_ref_at(text: &str, utf16_pos: u32) -> String {
    let cursor = char_index_from_utf16(text, utf16_pos as usize);
    match xref::ref_at(text, cursor) {
        Some(r) => json!({ "handle": r.handle, "name": r.name }).to_string(),
        None => "null".into(),
    }
}

/// Same rules as `xref::allocate_handle`. `taken_json` is a JSON array of handles.
#[wasm_bindgen(js_name = allocateHandle)]
pub fn allocate_handle(title: &str, taken_json: &str) -> String {
    let taken: Vec<String> = serde_json::from_str(taken_json).unwrap_or_default();
    let set: std::collections::HashSet<String> = taken.into_iter().collect();
    xref::allocate_handle(title, &set)
}

/// Replace `@handle.old` with `@handle.new`. `{text, count}`.
#[wasm_bindgen(js_name = rewriteVarRefs)]
pub fn rewrite_var_refs(text: &str, handle: &str, old: &str, new_name: &str) -> String {
    let (text, count) = xref::rewrite_var_refs(text, handle, old, new_name);
    json!({ "text": text, "count": count }).to_string()
}

#[wasm_bindgen(js_name = renameBlurb)]
pub fn rename_blurb(
    old: &str,
    new_name: &str,
    here: u32,
    elsewhere: u32,
    locked: u32,
    away: u32,
) -> String {
    xref::rename_blurb(
        old,
        new_name,
        here as usize,
        elsewhere as usize,
        locked as usize,
        away as usize,
    )
}

/// Rename the variable under a UTF-16 cursor on `pages_json[source]`, and
/// `@handle.old` on the other pages. `{edits, blurb, cursor}` or `{error, message}`.
#[wasm_bindgen(js_name = propagateRename)]
pub fn propagate_rename(pages_json: &str, source: u32, utf16_pos: u32, new_name: &str) -> String {
    let pages = parse_pages(pages_json);
    let Some(page) = pages.get(source as usize) else {
        return json!({ "error": "none", "message": "Put the cursor on a variable name." })
            .to_string();
    };
    let cursor = char_index_from_utf16(&page.text, utf16_pos as usize);
    let source_text = page.text.clone();
    match xref::propagate_rename(&pages, source as usize, cursor, new_name) {
        Ok(done) => {
            let cursor = done.cursor.map(|at| {
                let text = done
                    .edits
                    .iter()
                    .find(|edit| edit.index == source as usize)
                    .map(|edit| edit.text.as_str())
                    .unwrap_or(source_text.as_str());
                utf16_from_chars(text, at)
            });
            let edits: Vec<_> = done
                .edits
                .iter()
                .map(|edit| json!({ "id": edit.id, "text": edit.text, "handle": edit.handle }))
                .collect();
            json!({
                "edits": edits,
                "blurb": done.blurb,
                "cursor": cursor,
                "here": done.here,
                "elsewhere": done.elsewhere,
            })
            .to_string()
        }
        Err(err) => {
            let code = match err {
                omacus_engine::rename::RenameFail::NotAVariable => "none",
                omacus_engine::rename::RenameFail::InvalidName => "invalid",
                omacus_engine::rename::RenameFail::Collision { .. } => "collision",
            };
            json!({ "error": code, "message": err.message() }).to_string()
        }
    }
}

/// Change the handle of `pages_json[source]` and rewrite `@old.` everywhere.
#[wasm_bindgen(js_name = propagateHandle)]
pub fn propagate_handle(pages_json: &str, source: u32, new_handle: &str) -> String {
    let pages = parse_pages(pages_json);
    match xref::propagate_handle(&pages, source as usize, new_handle) {
        Ok(done) => {
            let edits: Vec<_> = done
                .edits
                .iter()
                .map(|edit| json!({ "id": edit.id, "text": edit.text, "handle": edit.handle }))
                .collect();
            json!({
                "edits": edits,
                "blurb": done.blurb,
                "here": done.here,
                "elsewhere": done.elsewhere,
            })
            .to_string()
        }
        Err(err) => json!({ "error": "handle", "message": err.message() }).to_string(),
    }
}

fn lock_error(err: omacus_engine::lock::LockError) -> String {
    json!({ "ok": false, "error": err.message() }).to_string()
}

fn wrap_json(wrap: &omacus_engine::lock::Wrap) -> serde_json::Value {
    json!({
        "salt": wrap.salt,
        "nonce": wrap.nonce,
        "wrapped": wrap.wrapped,
        "memory_kib": wrap.memory_kib,
        "iterations": wrap.iterations,
        "lanes": wrap.lanes,
    })
}

fn wrap_from(text: &str) -> Result<omacus_engine::lock::Wrap, omacus_engine::lock::LockError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| omacus_engine::lock::LockError::Corrupt)?;
    let field = |name: &str| value[name].as_str().unwrap_or("").to_string();
    let number = |name: &str| value[name].as_u64().unwrap_or(0) as u32;
    Ok(omacus_engine::lock::Wrap {
        salt: field("salt"),
        nonce: field("nonce"),
        wrapped: field("wrapped"),
        memory_kib: number("memory_kib"),
        iterations: number("iterations"),
        lanes: number("lanes"),
    })
}

/// A new content key and the Argon2 wrap that holds it. `{ok, key, wrap}` or `{ok:false, error}`.
#[wasm_bindgen(js_name = lockCreate)]
pub fn lock_create(password: &str) -> String {
    match omacus_engine::lock::create(password) {
        Ok((key, wrap)) => {
            json!({ "ok": true, "key": key.encode(), "wrap": wrap_json(&wrap) }).to_string()
        }
        Err(err) => lock_error(err),
    }
}

/// Wrap an existing content key under a new password. `{ok, wrap}`.
#[wasm_bindgen(js_name = lockWrap)]
pub fn lock_wrap(password: &str, key: &str) -> String {
    let key = match omacus_engine::lock::ContentKey::decode(key) {
        Ok(key) => key,
        Err(err) => return lock_error(err),
    };
    match omacus_engine::lock::wrap_key(password, &key) {
        Ok(wrap) => json!({ "ok": true, "wrap": wrap_json(&wrap) }).to_string(),
        Err(err) => lock_error(err),
    }
}

/// Encrypt `plaintext` for page id `aad`. `wrap` is the JSON from `lockCreate`. `{ok, text}`.
#[wasm_bindgen(js_name = lockSeal)]
pub fn lock_seal(key: &str, plaintext: &str, wrap: &str, aad: &str) -> String {
    let key = match omacus_engine::lock::ContentKey::decode(key) {
        Ok(key) => key,
        Err(err) => return lock_error(err),
    };
    let wrap = match wrap_from(wrap) {
        Ok(wrap) => wrap,
        Err(err) => return lock_error(err),
    };
    match omacus_engine::lock::seal(&key, plaintext, &wrap, aad.as_bytes()) {
        Ok(text) => json!({ "ok": true, "text": text }).to_string(),
        Err(err) => lock_error(err),
    }
}

/// Decrypt with a content key this session already holds. `{ok, text}`.
#[wasm_bindgen(js_name = lockOpen)]
pub fn lock_open(key: &str, stored: &str, aad: &str) -> String {
    let key = match omacus_engine::lock::ContentKey::decode(key) {
        Ok(key) => key,
        Err(err) => return lock_error(err),
    };
    match omacus_engine::lock::open_key(&key, stored, aad.as_bytes()) {
        Ok(text) => json!({ "ok": true, "text": text }).to_string(),
        Err(err) => lock_error(err),
    }
}

/// Check a password against a wrap and return the content key. `{ok, key}`.
#[wasm_bindgen(js_name = lockUnwrap)]
pub fn lock_unwrap(password: &str, wrap: &str) -> String {
    let wrap = match wrap_from(wrap) {
        Ok(wrap) => wrap,
        Err(err) => return lock_error(err),
    };
    match omacus_engine::lock::unwrap_key(password, &wrap) {
        Ok(key) => json!({ "ok": true, "key": key.encode() }).to_string(),
        Err(err) => lock_error(err),
    }
}

/// Unlock a stored page with a password. `{ok, text, key}`.
#[wasm_bindgen(js_name = lockOpenPassword)]
pub fn lock_open_password(password: &str, stored: &str, aad: &str) -> String {
    match omacus_engine::lock::open_password(password, stored, aad.as_bytes()) {
        Ok((text, key)) => json!({ "ok": true, "text": text, "key": key.encode() }).to_string(),
        Err(err) => lock_error(err),
    }
}

/// Whether `text` is an encrypted page file.
#[wasm_bindgen(js_name = lockIs)]
pub fn lock_is(text: &str) -> bool {
    omacus_engine::lock::is_lock(text)
}

fn utf16_spans(text: &str, spans: &[omacus_engine::rename::Span]) -> Vec<Vec<usize>> {
    spans
        .iter()
        .map(|span| {
            vec![
                utf16_from_chars(text, span.start),
                utf16_from_chars(text, span.end),
            ]
        })
        .collect()
}

/// Answers as JSON for app.js.
fn to_json(results: Vec<omacus_engine::LineResult>) -> String {
    let results: Vec<_> = results
        .into_iter()
        .map(|r| {
            json!({
                "kind": kind(&r.kind),
                "display": r.display,
                "color": r.color.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")),
                "note": r.note,
                "hint": r.hint,
            })
        })
        .collect();
    serde_json::to_string(&results).unwrap_or_default()
}

fn open_doc(bytes: &[u8]) -> Option<omacus_doc::Library> {
    if bytes.is_empty() {
        return Some(omacus_doc::Library::new());
    }
    omacus_doc::Library::load(bytes).ok()
}

fn conflicts_json(lib: &omacus_doc::Library) -> String {
    let rows: Vec<_> = lib
        .conflicts()
        .into_iter()
        .map(|conflict| {
            json!({
                "page_id": conflict.page_id,
                "line_id": conflict.line_id,
                "ours": conflict.ours,
                "theirs": conflict.theirs,
            })
        })
        .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
}

/// Build a document from pages that already exist. `json` is `{pages, folders}`.
#[wasm_bindgen(js_name = docMigrate)]
pub fn doc_migrate(json: &str) -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_str(json).unwrap_or(serde_json::json!({}));
    let mut lib = omacus_doc::Library::new();
    if let Some(pages) = value.get("pages").and_then(|p| p.as_array()) {
        for page in pages {
            let id = page.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() || id == "shared" {
                continue;
            }
            let text = page.get("text").and_then(|v| v.as_str()).unwrap_or("");
            lib.reconcile(id, text, None);
            lib.note_written(id, text);
        }
    }
    let mut seeds = Vec::new();
    if let Some(folders) = value.get("folders").and_then(|p| p.as_array()) {
        for folder in folders {
            let Some(id) = folder.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            seeds.push(omacus_doc::FolderSeed {
                id: id.to_string(),
                name: folder
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                parent: folder
                    .get("parent")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            });
        }
    }
    lib.set_folders(&seeds);
    lib.save()
}

/// Current text of one page. Empty when the page is not in the document.
#[wasm_bindgen(js_name = docSnapshot)]
pub fn doc_snapshot(bytes: &[u8], page_id: &str) -> String {
    open_doc(bytes)
        .and_then(|lib| lib.snapshot(page_id))
        .unwrap_or_default()
}

/// Replace one page with `text`, keeping line ids where the line is the same.
#[wasm_bindgen(js_name = docApply)]
pub fn doc_apply(bytes: &[u8], page_id: &str, text: &str) -> Vec<u8> {
    let Some(mut lib) = open_doc(bytes) else {
        return bytes.to_vec();
    };
    if page_id.is_empty() || page_id == "shared" {
        return lib.save();
    }
    lib.reconcile(page_id, text, None);
    lib.note_written(page_id, text);
    lib.save()
}

/// Fold an outside copy of a page into the document.
#[wasm_bindgen(js_name = docMergeDisk)]
pub fn doc_merge_disk(bytes: &[u8], page_id: &str, local: &str, disk: &str) -> Vec<u8> {
    let Some(mut lib) = open_doc(bytes) else {
        return bytes.to_vec();
    };
    lib.reconcile(page_id, local, Some(disk));
    lib.save()
}

/// Same-line conflicts, as JSON.
#[wasm_bindgen(js_name = docConflicts)]
pub fn doc_conflicts(bytes: &[u8]) -> String {
    open_doc(bytes)
        .map(|lib| conflicts_json(&lib))
        .unwrap_or_else(|| "[]".into())
}

/// Choose one side of a conflict. `theirs` uses the other copy.
#[wasm_bindgen(js_name = docPick)]
pub fn doc_pick(bytes: &[u8], page_id: &str, line_id: &str, theirs: bool) -> Vec<u8> {
    let Some(mut lib) = open_doc(bytes) else {
        return bytes.to_vec();
    };
    lib.pick(page_id, line_id, theirs);
    lib.save()
}

/// Drop a page from the document. The caller still has the file or the trash copy.
#[wasm_bindgen(js_name = docRemove)]
pub fn doc_remove(bytes: &[u8], page_id: &str) -> Vec<u8> {
    let Some(mut lib) = open_doc(bytes) else {
        return bytes.to_vec();
    };
    lib.remove_page(page_id);
    lib.save()
}

/// Folder parent pointers. A loop is broken. A third level is refused.
#[wasm_bindgen(js_name = docSetFolders)]
pub fn doc_set_folders(bytes: &[u8], folders_json: &str) -> Vec<u8> {
    let Some(mut lib) = open_doc(bytes) else {
        return bytes.to_vec();
    };
    let value: serde_json::Value =
        serde_json::from_str(folders_json).unwrap_or(serde_json::json!([]));
    let mut seeds = Vec::new();
    if let Some(folders) = value.as_array() {
        for folder in folders {
            let Some(id) = folder.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            seeds.push(omacus_doc::FolderSeed {
                id: id.to_string(),
                name: folder
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                parent: folder
                    .get("parent")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            });
        }
    }
    lib.set_folders(&seeds);
    lib.save()
}

/// Merge two documents. A different document keeps both pages.
#[wasm_bindgen(js_name = docUnion)]
pub fn doc_union(left: &[u8], right: &[u8]) -> Vec<u8> {
    match (open_doc(left), open_doc(right)) {
        (Some(a), Some(b)) => {
            let mut union = a.union(b);
            union.save()
        }
        (Some(mut a), None) => a.save(),
        (None, Some(mut b)) => b.save(),
        (None, None) => left.to_vec(),
    }
}

/// The vault id stored on the document.
#[wasm_bindgen(js_name = docVault)]
pub fn doc_vault(bytes: &[u8]) -> String {
    open_doc(bytes)
        .map(|lib| lib.vault())
        .unwrap_or_else(|| omacus_doc::VAULT_ID.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Laying out answers already worked out matches working them out again.
    #[test]
    fn lays_out_answers_it_is_given() {
        let calc = Calc::new();
        assert!(calc.set_rates(r#"{"USD": 1, "EUR": 0.9}"#));
        let text = "# Trip\nHotel $300\nFood $120\n\ntotal\n// note\n10 / 0";
        let results = calc.evaluate(text);
        assert_eq!(
            layout_answers(text, &results, false),
            calc.with_answers(text)
        );
        assert_eq!(
            layout_answers(text, &results, true),
            calc.markdown_table(text)
        );
    }

    #[test]
    fn evaluates_a_sheet_as_json() {
        let calc = Calc::new();
        assert!(calc.set_rates(r#"{"USD": 1, "EUR": 0.9}"#));
        let out: serde_json::Value =
            serde_json::from_str(&calc.evaluate("# Trip\n$84 split 3 ways\n#ff8800\n$10 in EUR"))
                .unwrap();
        assert_eq!(out[0]["kind"], "header");
        assert_eq!(out[1]["display"], "$28");
        assert_eq!(out[2]["color"], "#ff8800");
        assert_eq!(out[3]["display"], "9 EUR");
    }

    #[test]
    fn suggests_with_utf16_offsets() {
        // "€" is 3 bytes in Rust but 1 unit in JavaScript: "tomo" starts at 3.
        let out: serde_json::Value = serde_json::from_str(&suggest("€5 tomo", "")).unwrap();
        assert_eq!(out["start"], 3);
        let tomorrow = out["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["text"] == "tomorrow")
            .expect("tomorrow");
        assert_eq!(tomorrow["kind"], "keyword");
        assert_eq!(tomorrow["detail"], "tomorrow's date");
        assert_eq!(suggest("x", ""), "null");
    }

    #[test]
    fn rename_reports_utf16_and_refuses_a_collision() {
        let text = "rent = 1\nrent + 1\nfee = 3\n";
        let at = text.rfind("rent").unwrap() as u32;
        let binding: serde_json::Value = serde_json::from_str(&binding_at(text, at)).unwrap();
        assert_eq!(binding["name"], "rent");
        assert_eq!(binding["references"], 1);
        let renamed: serde_json::Value =
            serde_json::from_str(&rename_variable(text, 0, "cost")).unwrap();
        assert_eq!(renamed["text"], "cost = 1\ncost + 1\nfee = 3\n");
        let refused: serde_json::Value =
            serde_json::from_str(&rename_variable(text, 0, "fee")).unwrap();
        assert_eq!(refused["error"], "collision");
        assert_eq!(refused["message"], "fee is already used");
        let edit: serde_json::Value =
            serde_json::from_str(&definition_change("rent = 1\nrent\n", "fee = 1\nrent\n"))
                .unwrap();
        assert_eq!(edit["references"], 1);
        assert_eq!(edit["message"], "Update 1 reference?");
    }

    #[test]
    fn hover_value_uses_the_page_above() {
        let calc = Calc::new();
        assert!(calc.set_rates(r#"{"USD": 1}"#));
        let text = "rent = $1,450\nrent * 2";
        let at = text.rfind("rent").unwrap() as u32;
        let out: serde_json::Value = serde_json::from_str(&calc.hover_value(text, at)).unwrap();
        assert_eq!(out["value"], "$1,450");
        assert!(calc.hover_value("42", 0) == "null");
    }

    #[test]
    fn page_marks_name_the_definition() {
        let marks: serde_json::Value =
            serde_json::from_str(&page_marks("- hotel = 3 nights * $180")).unwrap();
        assert_eq!(marks["define"], json!([2, 7]));
        let noted: serde_json::Value =
            serde_json::from_str(&page_marks("rent = $100 # monthly")).unwrap();
        assert_eq!(noted["define"], json!([0, 4]));
        assert_eq!(noted["comment"], 12);
    }

    #[test]
    fn page_marks_use_utf16_and_keep_a_mid_line_hash() {
        let marks: serde_json::Value =
            serde_json::from_str(&page_marks("€ **bold** # note")).unwrap();
        // "€" is one UTF-16 unit and three bytes. The bold word starts after "€ **".
        assert_eq!(marks["bold"][0], json!([4, 8]));
        assert_eq!(marks["markers"][0], json!([2, 4]));
        assert_eq!(marks["comment"], 11);
        assert!(marks["heading"].is_null());

        let heading: serde_json::Value = serde_json::from_str(&page_marks("## Week 1")).unwrap();
        assert_eq!(heading["heading"], 2);
        assert!(heading["comment"].is_null());

        let color: serde_json::Value = serde_json::from_str(&page_marks("#ff8800")).unwrap();
        assert!(color["heading"].is_null());
        assert!(color["comment"].is_null());
    }

    #[test]
    fn lock_round_trip_over_the_wasm_api() {
        let created: serde_json::Value =
            serde_json::from_str(&lock_create("correct horse")).unwrap();
        assert_eq!(created["ok"], true);
        let sealed: serde_json::Value = serde_json::from_str(&lock_seal(
            created["key"].as_str().unwrap(),
            "# Groceries\nmilk $3",
            &created["wrap"].to_string(),
            "page-1",
        ))
        .unwrap();
        assert_eq!(sealed["ok"], true);
        let file = sealed["text"].as_str().unwrap();
        assert!(lock_is(file));
        assert!(
            !file.contains("Groceries") && !file.contains("milk"),
            "{file}"
        );
        let opened: serde_json::Value =
            serde_json::from_str(&lock_open_password("correct horse", file, "page-1")).unwrap();
        assert_eq!(opened["text"], "# Groceries\nmilk $3");
        assert_eq!(opened["key"], created["key"]);
        let wrong: serde_json::Value =
            serde_json::from_str(&lock_open_password("nope", file, "page-1")).unwrap();
        assert_eq!(wrong["ok"], false);
        assert_eq!(wrong["error"], "Wrong password.");
        let empty: serde_json::Value = serde_json::from_str(&lock_create("")).unwrap();
        assert_eq!(empty["error"], "Enter a password.");
    }

    #[test]
    fn preview_puts_a_pill_on_a_list_item() {
        let calc = Calc::new();
        let text = "- hotel = 3 nights * $180\npack sunscreen\nhotel * 2\nrent = $100 # monthly";
        let answers = calc.evaluate(text);
        let html = preview_html(text, &answers);
        assert!(html.contains("<li>"), "{html}");
        assert!(
            html.contains("data-value=\"$540\">[ $540 ]</button>"),
            "{html}"
        );
        assert!(
            html.contains("data-value=\"$1,080\">[ $1,080 ]</button>"),
            "{html}"
        );
        assert!(
            html.contains("data-value=\"$100\">[ $100 ]</button>"),
            "{html}"
        );
        assert!(html.contains("pack sunscreen"), "{html}");
        assert!(!html.contains("# monthly"), "{html}");
    }

    #[test]
    fn preview_blocks_keep_answers_as_text() {
        let calc = Calc::new();
        let text = "# Trip Budget\nferry = $40\n<script>alert(1)</script>\n";
        let answers = calc.evaluate(text);
        let raw = preview_blocks(text, &answers);
        let blocks: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(blocks.is_array(), "{raw}");
        assert!(raw.contains("Trip Budget"), "{raw}");
        assert!(raw.contains("$40"), "{raw}");
        assert!(raw.contains("<script>alert(1)</script>"), "{raw}");
        assert!(!raw.contains("<article"), "{raw}");
        let fenced = preview_blocks("```\nmilk = $3\na < b\n```\n", "[]");
        assert!(fenced.contains("milk = $3"), "{fenced}");
        assert!(fenced.contains("a < b"), "{fenced}");
        assert!(!fenced.contains("<pre"), "{fenced}");
    }

    #[test]
    fn a_migrated_page_merges_another_line_and_keeps_a_conflict() {
        let bytes = doc_migrate(
            r##"{"pages":[{"id":"trip","text":"# Trip Budget\nferry = 40\n"}],"folders":[{"id":"trips","name":"Trips"}]}"##,
        );
        assert_eq!(doc_vault(&bytes), "personal");
        let edited = doc_apply(&bytes, "trip", "# Trip Budget\nferry = 50\n");
        assert!(doc_snapshot(&edited, "trip").contains("ferry = 50"));
        let merged = doc_merge_disk(
            &bytes,
            "trip",
            "# Trip Budget\nferry = 50\n",
            "# Trip Budget\nferry = 40\nmilk $3\n",
        );
        let snap = doc_snapshot(&merged, "trip");
        assert!(snap.contains("ferry = 50"), "{snap}");
        assert!(snap.contains("milk $3"), "{snap}");
        assert!(!snap.contains("ferry = 40\n"), "{snap}");
        let conflicted = doc_merge_disk(
            &bytes,
            "trip",
            "# Trip Budget\nferry = 50\n",
            "# Trip Budget\nferry = 60\n",
        );
        let rows: serde_json::Value = serde_json::from_str(&doc_conflicts(&conflicted)).unwrap();
        assert_eq!(rows[0]["ours"], "ferry = 50");
        assert_eq!(rows[0]["theirs"], "ferry = 60");
        let picked = doc_pick(
            &conflicted,
            rows[0]["page_id"].as_str().unwrap(),
            rows[0]["line_id"].as_str().unwrap(),
            true,
        );
        assert!(doc_snapshot(&picked, "trip").contains("ferry = 60"));
        assert!(doc_conflicts(&picked) == "[]");
        let again = doc_apply(&picked, "trip", &doc_snapshot(&picked, "trip"));
        assert!(doc_snapshot(&again, "trip").contains("ferry = 60"));
    }

    #[test]
    fn a_page_reference_uses_the_other_page() {
        let calc = Calc::new();
        assert!(calc.set_rates(r#"{"USD": 1}"#));
        let pages = r#"[{"id":"a","handle":"trip","title":"Trip Budget","text":"total = $40","locked":false,"deleted":false},{"id":"b","handle":"groceries","title":"Groceries","text":"cost = @trip.total * 2","locked":false,"deleted":false}]"#;
        let out: serde_json::Value = serde_json::from_str(&calc.evaluate_sheet_with(
            "b",
            "groceries",
            "cost = @trip.total * 2",
            pages,
        ))
        .unwrap();
        assert_eq!(out[0]["display"], "$80");
        let hover: serde_json::Value = serde_json::from_str(&calc.hover_value_with(
            "cost = @trip.total * 2",
            10,
            "groceries",
            pages,
        ))
        .unwrap();
        assert_eq!(hover["value"], "$40");
        assert_eq!(hover["sourceTitle"], "Trip Budget");
        let picks: serde_json::Value =
            serde_json::from_str(&calc.suggest_at("@tr", 3, pages)).unwrap();
        assert_eq!(picks["items"][0]["text"], "@trip.");
        assert_eq!(allocate_handle("Trip Budget", "[]"), "trip");
        assert_eq!(allocate_handle("Trip Budget", r#"["trip"]"#), "tripbudget");
    }
}
