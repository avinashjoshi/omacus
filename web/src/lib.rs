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
    json!({ "start": start, "items": items }).to_string()
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

/// Names assigned in the sheet, for highlighting.
#[wasm_bindgen]
pub fn variables(sheet: &str) -> String {
    let vars: Vec<String> = omacus_engine::defined_variables(sheet)
        .into_iter()
        .collect();
    serde_json::to_string(&vars).unwrap_or_default()
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
        assert!(
            out["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i == "tomorrow")
        );
        assert_eq!(suggest("x", ""), "null");
    }
}
