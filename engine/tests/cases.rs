//! Runs the sheets in cases.txt and checks every line's answer. A failure lists
//! each line that differs, with what it got.

use omacus_engine::{Engine, Kind, Rates};

fn engine() -> Engine {
    Engine::new(Rates::fixed(&[
        ("USD", 1.0),
        ("EUR", 0.9),
        ("GBP", 0.78),
        ("INR", 83.0),
        ("CAD", 1.36),
        ("JPY", 150.0),
        ("CHF", 0.9),
        ("BOB", 6.9),
    ]))
    .with_clock(omacus_engine::monotonic_ms)
}

#[test]
fn cases() {
    let text = include_str!("cases.txt");
    let body = text.split_once("\n---\n").map_or(text, |(_, rest)| rest);
    let engine = engine();
    let mut failures = Vec::new();
    for sheet in body.split("\n---\n") {
        let mut lines = Vec::new();
        let mut expected = Vec::new();
        for line in sheet.lines() {
            match line.rsplit_once(" =>") {
                Some((input, want)) => {
                    lines.push(input.to_string());
                    expected.push(Some(want.trim().to_string()));
                }
                None => {
                    lines.push(line.to_string());
                    expected.push(None);
                }
            }
        }
        let results = engine.eval_document(&lines.join("\n"));
        for ((line, want), got) in lines.iter().zip(&expected).zip(&results) {
            let Some(want) = want else { continue };
            let got_text = match (&got.kind, got.note) {
                (Kind::Value, _) => got.display.clone(),
                (_, Some(note)) => format!("!{note}"),
                _ => String::new(),
            };
            let ok = if want.starts_with('!') || !want.is_empty() {
                &got_text == want
            } else {
                got.kind != Kind::Value
            };
            if !ok {
                failures.push(format!("  {line:40} want {want:?}, got {got_text:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} lines differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
