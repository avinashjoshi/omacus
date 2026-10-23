//! The Omacus engine for JavaScript: the same code as the desktop app, compiled to
//! WebAssembly for omacus.com/app. Results go back as JSON strings.

use std::cell::RefCell;
use std::collections::HashMap;

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
        }
    }

    /// Exchange rates as `{"EUR": 0.92, ...}` (units per 1 USD). Returns whether
    /// they were accepted.
    #[wasm_bindgen(js_name = setRates)]
    pub fn set_rates(&self, json: &str) -> bool {
        match serde_json::from_str::<HashMap<String, f64>>(json) {
            Ok(table) if !table.is_empty() => {
                self.rates.set(table);
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
        let cursor = char_index_from_utf16(text, utf16_pos as usize);
        match self.engine.hover_at(text, cursor) {
            Some(hit) => json!({
                "start": utf16_from_chars(text, hit.start),
                "end": utf16_from_chars(text, hit.end),
                "value": hit.value,
            })
            .to_string(),
            None => "null".into(),
        }
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
    let parsed: Vec<serde_json::Value> = serde_json::from_str(answers_json).unwrap_or_default();
    let answers: Vec<Option<String>> = parsed.iter().map(pill_text).collect();
    let refs: Vec<Option<&str>> = answers.iter().map(|s| s.as_deref()).collect();
    omacus_engine::markdown::preview_html(text, &refs)
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
}
