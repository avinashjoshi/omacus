//! Runs the sheets in cases.txt and checks every line's answer. A failure lists
//! each line that differs, with what it got.

use omacus_engine::{Engine, Kind, LineResult, Rates, Sheet};

/// The engine for these tests. Each line's allowance counts fend's steps instead of
/// timing it, so an answer never depends on how fast or busy the machine is: in a
/// debug build on a loaded CI machine, two runs that should agree didn't. (The
/// first line also used to pay for fend setting itself up, about 80 ms; the engine
/// now does that before the clock starts, see first_line.rs.)
fn engine() -> Engine {
    counted()
}

fn counted() -> Engine {
    Engine::new(Rates::fixed(&[
        ("USD", 1.0),
        ("EUR", 0.9),
        ("GBP", 0.78),
        ("INR", 83.0),
        ("CAD", 1.36),
        ("JPY", 150.0),
        ("CHF", 0.9),
        ("BOB", 6.9),
        ("AUD", 1.5),
    ]))
}

/// The engine as the apps run it, with a time limit per line. Only for lines far
/// inside or far past the limit.
fn timed() -> Engine {
    counted().with_clock(omacus_engine::monotonic_ms)
}

#[test]
fn cases() {
    // Git on Windows may check the file out with CRLF line endings.
    let text = include_str!("cases.txt").replace("\r\n", "\n");
    let body = text
        .split_once("\n---\n")
        .map_or(text.as_str(), |(_, rest)| rest);
    let (counted, timed) = (engine(), timed());
    let mut failures = Vec::new();
    for sheet in body.split("\n---\n") {
        // Lines that run out of time stop at the time limit, rather than after
        // the far longer step cap.
        let engine = if sheet.contains("100000!") || sheet.contains("10^10^7") {
            &timed
        } else {
            &counted
        };
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

/// A line's answer shouldn't depend on where it sits or how long the lines above it
/// took (from the second review: 50 slow lines got 36 answers and 14 "too big").
#[test]
fn same_line_same_answer_anywhere() {
    // Without a clock, each line's allowance counts fend's steps, so the answer can
    // depend only on where the line sits, never on the machine's speed or load. With
    // a clock, only lines far inside or far past the time limit, even in a debug build
    // on a slow, busy machine (Windows CI missed 150 ms on 3000!).
    for (engine, line, copies, want) in [
        (counted(), "3000!", 30, None),
        (timed(), "2^64", 50, Some("18,446,744,073,709,551,616")),
        (timed(), "100!", 50, None),
        (timed(), "100000!", 12, Some("")),
    ] {
        let sheet = vec![line; copies].join("\n");
        let results = engine.eval_document(&sheet);
        let first = (&results[0].display, results[0].note);
        if let Some(want) = want {
            assert_eq!(results[0].display, want, "{line}");
        } else {
            assert!(!results[0].display.is_empty(), "{line} got no answer");
        }
        let differ = results
            .iter()
            .filter(|r| (&r.display, r.note) != first)
            .count();
        assert_eq!(
            differ, 0,
            "{differ} of {copies} copies of {line:?} got a different answer"
        );
    }
}

fn same(a: &[LineResult], b: &[LineResult]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.kind == y.kind && x.display == y.display && x.note == y.note && x.color == y.color
        })
}

fn sheets() -> Vec<String> {
    let text = include_str!("cases.txt").replace("\r\n", "\n");
    let body = text
        .split_once("\n---\n")
        .map_or(text.as_str(), |(_, rest)| rest)
        .to_string();
    body.split("\n---\n")
        .map(|sheet| {
            sheet
                .lines()
                .map(|l| l.rsplit_once(" =>").map_or(l, |(input, _)| input))
                // Lines that use their whole time limit are checked elsewhere;
                // here they'd only make every step slow.
                .filter(|l| !l.contains("100000!") && !l.contains("10^10^7"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

/// Reusing earlier lines' work must give exactly what calculating from scratch
/// gives, at every keystroke: each sheet in cases.txt typed one character at a
/// time, then edited in the middle.
#[test]
fn reusing_work_matches_starting_over() {
    let engine = engine();
    let mut seed = 0x2545_f491_u64;
    let mut rand = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n.max(1) as u64) as usize
    };
    let all = sheets().join("\n\n");
    for sheet in sheets().iter().chain(std::iter::once(&all)) {
        let mut cache = Sheet::default();
        // Typed from the start: each sheet a character at a time, and the long
        // combined one at about 80 line ends (each check redoes it from scratch).
        let chars: Vec<char> = sheet.chars().collect();
        let steps: Vec<usize> = if std::ptr::eq(sheet, &all) {
            let ends: Vec<usize> = (0..chars.len()).filter(|&i| chars[i] == '\n').collect();
            let every = ends.len().div_ceil(80);
            ends.into_iter()
                .step_by(every)
                .chain([chars.len()])
                .collect()
        } else {
            (0..=chars.len()).collect()
        };
        for end in steps {
            let text: String = chars[..end].iter().collect();
            let reused = engine.eval_sheet(&text, &mut cache);
            assert!(
                same(&reused, &engine.eval_document(&text)),
                "typing {text:?}"
            );
        }
        // Then lines changed, inserted and removed anywhere.
        let mut lines: Vec<String> = sheet.lines().map(str::to_string).collect();
        for _ in 0..40 {
            let at = rand(lines.len() + 1);
            match rand(4) {
                0 if at < lines.len() => {
                    lines.remove(at);
                }
                1 => lines.insert(at, "x = 7".into()),
                2 if at < lines.len() => lines[at].push_str(" + 1"),
                _ => lines.insert(at, "total".into()),
            }
            let text = lines.join("\n");
            let reused = engine.eval_sheet(&text, &mut cache);
            assert!(
                same(&reused, &engine.eval_document(&text)),
                "editing {text:?}"
            );
        }
    }
}

/// A calculation cut off between any two lines (newer text arrived) must leave the
/// sheet's saved work right: finishing it later gives what starting over gives,
/// even when the text changed in between.
#[test]
fn stopping_part_way_keeps_saved_work_right() {
    use std::cell::Cell;
    let engine = engine();
    for sheet in sheets() {
        let lines = sheet.lines().count();
        let edited = sheet.replacen('\n', " + 1\n", 1);
        for after in 0..=lines {
            for next in [&sheet, &edited] {
                let mut cache = Sheet::default();
                let checks = Cell::new(0);
                let stopped = engine.eval_sheet_until(&sheet, &mut cache, || {
                    checks.set(checks.get() + 1);
                    checks.get() > after
                });
                if after == 0 && lines > 1 {
                    assert!(stopped.is_none(), "never stopped: {sheet:?}");
                }
                if let Some(done) = stopped {
                    assert!(same(&done, &engine.eval_document(&sheet)), "{sheet:?}");
                }
                let finished = engine.eval_sheet(next, &mut cache);
                assert!(
                    same(&finished, &engine.eval_document(next)),
                    "stopped after {after} lines, then {next:?}"
                );
            }
        }
    }
}
