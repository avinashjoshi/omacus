//! The first line a process calculates gets its whole time limit (from the fifth
//! review): fend sets itself up on its first calculation, about 80 ms in a debug
//! build, and that used to count against whichever line came first. On a slow
//! phone or a busy machine, a plain "1 m in cm" then showed "Too big to work out", and kept
//! it until edited. Its own test binary, so nothing else has calculated first.

use omacus_engine::{Engine, Rates};

/// A machine five times slower than this one. Twenty times left each line only
/// about 7.5 ms of the 150 ms budget, so a pause on a busy runner failed `2 + 2`.
fn slow_clock() -> f64 {
    omacus_engine::monotonic_ms() * 5.0
}

#[test]
fn first_line_gets_its_whole_time_limit() {
    let engine = Engine::new(Rates::fixed(&[("USD", 1.0)])).with_clock(slow_clock);
    let results = engine.eval_document("1 m in cm\n2 + 2");
    assert_eq!(results[0].display, "100 cm", "note: {:?}", results[0].note);
    assert_eq!(results[1].display, "4");
}
