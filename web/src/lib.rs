//! The Omacus engine for JavaScript: the same code as the desktop app, compiled to
//! WebAssembly for omacus.com/app. Results go back as JSON strings.

use std::collections::HashMap;

use omacus_engine::{Engine, Kind, Rates, complete};
use serde_json::json;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Calc {
    engine: Engine,
    rates: Rates,
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
        Self {
            engine: Engine::new(rates.clone()),
            rates,
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

    /// Every line's answer: `[{"kind": "value", "display": "$28", "color": null}, ...]`.
    pub fn evaluate(&self, text: &str) -> String {
        let results: Vec<_> = self
            .engine
            .eval_document(text)
            .into_iter()
            .map(|r| {
                json!({
                    "kind": kind(&r.kind),
                    "display": r.display,
                    "color": r.color.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")),
                })
            })
            .collect();
        serde_json::to_string(&results).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

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
