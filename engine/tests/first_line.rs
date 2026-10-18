//! The first line a process calculates gets its whole time limit (from the fifth
//! review): fend sets itself up on its first calculation, about 80 ms in a debug
//! build, and that used to count against whichever line came first. On a slow
//! phone or a busy machine, a plain "r" then showed "Too big to work out", and kept
//! it until edited. Its own test binary, so nothing else has calculated first.

use omacus_engine::{Engine, Rates};

/// A machine twenty times slower than this one.
fn slow_clock() -> f64 {
    omacus_engine::monotonic_ms() * 20.0
}

#[test]
fn first_line_gets_its_whole_time_limit() {
    let engine = Engine::new(Rates::fixed(&[("USD", 1.0)])).with_clock(slow_clock);
    let results = engine.eval_document("r\n2 + 2");
    assert_eq!(results[0].display, "1 R", "note: {:?}", results[0].note);
    assert_eq!(results[1].display, "4");
}
