//! The notepad layer on top of fend-core.
//!
//! fend already understands arithmetic, units, currencies and variables. This
//! module adds the notepad conventions: headers, comments, labels, `prev`,
//! `sum`/`avg`, percentage phrases, currency symbols, forgiving prose, and
//! human-friendly result formatting.

use std::collections::HashSet;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use regex::Regex;

pub mod complete;
pub mod extras;
pub mod markdown;
mod rates;
pub mod share;
pub mod time;

pub use rates::Rates;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Empty,
    Header,
    Comment,
    Value,
    Error,
}

#[derive(Clone, Debug)]
pub struct LineResult {
    pub kind: Kind,
    /// Formatted result shown in the results column (empty when nothing to show).
    pub display: String,
    /// A colour answer ("#ff8800 in rgb"), shown as a swatch beside it.
    pub color: Option<[u8; 3]>,
    /// Why a line that looks like math has no answer ("Can't divide by zero"),
    /// shown quietly in place of one. Owned, because a name hint names the
    /// variable ("Did you mean rent?").
    pub note: Option<String>,
    /// `note` is a hint about a name (unknown, defined further down, or a
    /// different case). The app shows it as a dim marker once the line is idle,
    /// rather than while the letters are still being typed.
    pub hint: bool,
    /// Read from the clock ("today", "time in Tokyo"), so never reused as is.
    pub clock: bool,
    /// An answer from before the latest edit, kept in place (and shown dimmed)
    /// until the new one arrives: a line below an edit may depend on it.
    pub stale: bool,
}

/// What the lines above leave for the next one: fend's variables, the block for
/// `total`, `prev`, and the names defined so far.
#[derive(Clone)]
struct State {
    ctx: fend_core::Context,
    block: Vec<Term>,
    prev: Option<String>,
    vars: HashSet<String>,
    /// Variables holding a percentage ("tax = 8%"), for "$100 + tax".
    percents: Vec<(String, Regex, String)>,
}

/// A line in the block a `total` adds up: fend's variable for its answer, and
/// whether that's a plain number (which a total of dollars counts as dollars).
#[derive(Clone)]
struct Term {
    var: String,
    plain: bool,
    /// A percentage ("discount = 10%") or a rate ("$85 / hour"): not an amount,
    /// so left out of a total that doesn't add up with it.
    aside: bool,
}

/// A sheet's last calculation, kept between edits by `Engine::eval_sheet`.
#[derive(Clone, Default)]
pub struct Sheet {
    lines: Vec<String>,
    results: Vec<LineResult>,
    /// The state after each line.
    after: Vec<State>,
    rates_version: u64,
    /// Names assigned anywhere in the text last calculated. A hint on an earlier
    /// line ("z is defined below") depends on them, so a new assignment can't
    /// reuse those lines.
    defs: HashSet<String>,
}

impl Sheet {
    /// The answers worked out so far for the text last asked for: all of them, or
    /// the first lines' when `Engine::eval_sheet_until` stopped part way.
    pub fn done(&self) -> &[LineResult] {
        &self.results
    }
}

impl LineResult {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            display: String::new(),
            color: None,
            note: None,
            hint: false,
            clock: false,
            stale: false,
        }
    }

    fn value(display: String) -> Self {
        Self {
            kind: Kind::Value,
            display,
            color: None,
            note: None,
            hint: false,
            clock: false,
            stale: false,
        }
    }

    fn error(note: Option<&str>) -> Self {
        Self {
            note: note.map(str::to_string),
            ..Self::new(Kind::Error)
        }
    }

    /// A missing answer with a name hint. `hint` tells the app to wait out typing.
    fn hint(note: String) -> Self {
        Self {
            hint: true,
            note: Some(note),
            ..Self::new(Kind::Error)
        }
    }
}

/// The first calculation in a process sets things up once (fend's units, the
/// patterns here): about 80 ms in a debug build and 12 ms in release, where a line
/// takes under 1. Done before the first line's clock starts, so it never counts
/// against that line's time limit, which on a slow phone or a busy machine it could
/// use up. Another thread arriving meanwhile waits here, also before its clock.
fn warm_up() {
    static WARM: Once = Once::new();
    WARM.call_once(|| {
        // No clock, so this doesn't come back here.
        let engine = Engine {
            rates: Rates::fixed(&[("USD", 1.0), ("EUR", 0.9)]),
            clock: None,
        };
        engine.eval_document("x = 2 km in m\n3 ft in cm\n$5 + 10% in EUR\nx * 2\ntotal");
    });
}

/// How long one line may calculate before it's stopped (`100000!` or `10^10^7`
/// would otherwise run for minutes). The same for every line, wherever it sits.
const LINE_MS: f64 = 150.0;
/// Without a clock, a cap on fend's progress checks instead. Ordinary lines use a
/// few thousand; `10000!` about 900,000.
const MAX_CHECKS: u64 = 2_000_000;
/// With a clock, a backstop in case checks stop being cheap.
const MAX_CHECKS_TIMED: u64 = 200_000_000;

/// One line's allowance, handed to fend, which checks it as it works.
struct Budget {
    checks: AtomicU64,
    max_checks: u64,
    clock: Option<fn() -> f64>,
    deadline: f64,
    stopped: AtomicBool,
    /// fend's last error on this line, to explain a missing answer.
    error: Mutex<Option<String>>,
}

impl Budget {
    fn new(clock: Option<fn() -> f64>, ms: f64) -> Self {
        Self {
            checks: AtomicU64::new(0),
            max_checks: if clock.is_some() {
                MAX_CHECKS_TIMED
            } else {
                MAX_CHECKS
            },
            clock,
            deadline: clock.map_or(0.0, |now| now() + ms),
            stopped: AtomicBool::new(false),
            error: Mutex::new(None),
        }
    }

    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    fn evaluate(
        &self,
        input: &str,
        ctx: &mut fend_core::Context,
    ) -> Result<fend_core::FendResult, String> {
        if self.stopped() {
            return Err("interrupted".into());
        }
        let r = fend_core::evaluate_with_interrupt(input, ctx, self);
        if let Err(e) = &r {
            *self.error.lock().unwrap() = Some(e.clone());
        }
        r
    }

    /// Why the line has no answer, when it's worth saying.
    fn note(&self) -> Option<&'static str> {
        if self.stopped() {
            return Some("Too big to work out");
        }
        let error = self.error.lock().unwrap();
        let error = error.as_deref()?;
        if error.contains("division by zero") || error.contains("modulo by zero") {
            Some("Can't divide by zero")
        } else if error.contains("modulo is only supported for positive integers") {
            Some("mod needs whole numbers, 0 or more")
        } else if error.contains("are incompatible") {
            Some("Units don't match")
        } else if error.contains("must lie in the interval") || error.contains("is undefined") {
            Some("Not defined")
        } else {
            None
        }
    }
}

impl fend_core::Interrupt for Budget {
    fn should_interrupt(&self) -> bool {
        let n = self.checks.fetch_add(1, Ordering::Relaxed);
        let over = n >= self.max_checks
            || (n.is_multiple_of(128) && self.clock.is_some_and(|now| now() > self.deadline));
        if over {
            self.stopped.store(true, Ordering::Relaxed);
        }
        over
    }
}

/// Milliseconds since the first call: a clock for `Engine::with_clock` outside the
/// browser (where the web app passes `Date.now`).
#[cfg(not(target_arch = "wasm32"))]
pub fn monotonic_ms() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
        * 1000.0
}

/// A `#` line is a header, unless it's a colour ("#ff8800", "#f80 in rgb").
pub fn is_header(line: &str) -> bool {
    matches!(markdown::classify(line).form, markdown::Form::Heading(_))
}

/// A prefixed line (heading, list, task, quote) is calculated only when the words
/// after the marker are a calculation. A title like "Week 1" is not: the forgiving
/// reader would otherwise turn it into the number 1 and stop it being a heading.
fn looks_like_math(body: &str, vars: &HashSet<String>) -> bool {
    let body = body.trim();
    if body.is_empty() {
        return false;
    }
    if aggregate(body).is_some() || ASSIGN.is_match(body) || PREV.is_match(body) {
        return true;
    }
    if body.contains([
        '$', '£', '€', '¥', '₹', '₩', '₽', '₺', '%', '^', '*', '/', '+', '=',
    ]) {
        return true;
    }
    if body.contains(" - ") || body.contains(" – ") || body.contains(" — ") {
        return true;
    }
    if body
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || matches!(c, '(' | '.' | '-'))
    {
        return true;
    }
    if body.contains('(') && body.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    if LONE_WORD.is_match(body)
        && (vars.contains(body) || matches!(body, "pi" | "π" | "e" | "tau" | "τ" | "phi" | "φ"))
    {
        return true;
    }
    crate::time::eval(body).is_some()
        || crate::time::impossible_date(body)
        || crate::extras::eval(body).is_some()
}

/// An answer worth showing. A name hint on a heading is still a title.
fn shown_math(result: &LineResult) -> bool {
    match result.kind {
        Kind::Value => !result.display.is_empty(),
        Kind::Error => result.note.is_some() && !result.hint,
        _ => false,
    }
}

#[derive(Clone)]
pub struct Engine {
    /// Milliseconds from any fixed point, for the per-line time limit. Without
    /// one, lines are limited by how much work fend does instead.
    clock: Option<fn() -> f64>,
    rates: Rates,
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

pub const LABEL_PATTERN: &str = r"^\s*([A-Za-z][A-Za-z0-9 '\-_&/]*?)\s*:\s+(.*)$";
re!(LABEL, LABEL_PATTERN);
re!(QUOTED, r#""[^"]*""#);
re!(ASSIGN, r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^=].*)$");
// "5% on $200", and with a word in between: "18% tip on $62".
re!(PCT_ON, r"(?i)^(.+?)\s*%\s+(?:[a-z]+\s+)?on\s+(.+)$");
re!(PCT_OFF, r"(?i)^(.+?)\s*%\s+off\s+(.+)$");
re!(PCT_OF_WHAT, r"(?i)^(.+?)\s*%\s+of\s+what\s+is\s+(.+)$");
re!(AS_PCT, r"(?i)^(.+?)\s+as\s+(?:a\s+)?%\s+(?:of\s+)?(.+)$");
re!(IS_WHAT_PCT, r"(?i)^(.+?)\s+is\s+what\s+%\s+of\s+(.+)$");
re!(IS_PCT_OF_WHAT, r"(?i)^(.+?)\s+is\s+(.+?)\s*%\s+of\s+what$");
// "$100 - 20%", "$50 + 8% tax": a percentage of the amount, not of 1. Applied
// left to right, so "$100 - 10% - 10%" is $81.
re!(
    PLUS_MINUS_PCT_STEP,
    r"^([^%]+?)\s*([+-])\s*(\d[\d,]*(?:\.\d+)?\s*%(?:\s*[*/]\s*\d[\d,]*(?:\.\d+)?)*)"
);
// A plain number added to or taken from an amount ("rent + 5").
re!(
    BARE_NUMBER,
    r"(^|[+\-(]\s*)(\d[\d,]*(?:\.\d+)?)(\s*(?:[+\-)]|$))"
);
// fend's complaint when a currency meets a plain number.
re!(
    CURRENCY_VS_PLAIN,
    r"cannot convert from (\S+) to (\S+): units '(\w+)' and '(\w+)' are incompatible"
);
// fend reads `60 km / 2 hours` as `(60 km / 2) hours`; keep the divisor together.
// Only when the amount before it has a unit too (`10 / 4 inches` stays fend's).
re!(
    DIVISOR,
    r"([A-Za-z)]\s*)/\s*(\(?\d[\d,]*(?:\.\d+)?(?:[eE][+-]?\d+)?\)?\s*)([A-Za-df-zA-DF-Z_][A-Za-z_]*(?:\^\d+)?(?:/[A-Za-z][A-Za-z_]*(?:\^\d+)?)*)"
);
re!(TIMES_X, r"(\d+\)?)(\s*)([x×])(\s*)(\d|\()");
re!(
    SYMBOL_NUM,
    r"([€¥₹₩₽₺])\s*(\d[\d,]*(?:\.\d+)?(?:[eE][+-]?\d+)?)\b"
);
// Dollars with a country prefix: "C$100", "A$50", "US$20".
re!(
    PREFIXED_DOLLAR,
    r"\b(US|CA|C|AU|A|NZ|HK|SG|S)\$\s*(\d[\d,]*(?:\.\d+)?)"
);
// European grouping: "1.234,56" is 1234.56 and "1.000.000" is a million (dots
// in threes; anything else, like "1.5" or "1.000", keeps the dot as a decimal point).
re!(
    DOT_THOUSANDS,
    r"(^|[^\d.,])(\d{1,3}(?:\.\d{3})+),(\d+)($|[^\d.,])"
);
re!(
    DOT_MILLIONS,
    r"(^|[^\d.,])(\d{1,3}(?:\.\d{3}){2,})($|[^\d.,])"
);
// "$-5" is minus five dollars (fend reads it as "$1 - 5").
re!(SYMBOL_MINUS, r"(^|[^\w)])([$£€¥₹])\s*-\s*(\d)");
// A decimal comma: thousands always come in threes, so "1,5", "12,50" and
// "3,14159" can only mean 1.5, 12.50 and 3.14159 ("1,234" stays 1234).
re!(
    DECIMAL_COMMA,
    r"(^|[^\d,.])(\d+),(\d{1,2}|\d{4,})($|[^\d,])"
);
// A percentage times anything is that share of it, worked out first as the
// order of operations says: "10% * 200" is 20, not 2,000%, "100 + 10% * 50" is
// 105 and "$120k + 0.5% * valuation" adds 0.5% of the valuation. Divided, it
// stays a percentage ("50% / 2" is 25%; "100 + 10% / 2" is 105).
re!(PCT_SHARE, r"(\d[\d.]*)\s*%(\s*\*)");
// "%" between two numbers is mod ("7.5 % 2"); anywhere else it's a percentage.
re!(MOD_WRITTEN, r"(?i)\bmod\b|\d\s*%\s*[\d(]");
re!(TIMES_PCT, r"(\d\)?\s*[*/]\s*)(\d[\d.]*)\s*%");
re!(MONEY_PREFIX, r"[$£]\s*\d[\d,]*(?:\.\d+)?(?:[eE][+-]?\d+)?");
re!(PREV, r"(?i)\bprev\b");
re!(LETTERS, r"[A-Za-z]+");
// "$5m", "$1mm", "$2.5bn": a scale after a currency amount is part of the number
// (k thousand, m/mm million, b/bn billion). Alone, "1m" is metres, "3mm" is
// millimetres and "1.5B" is bytes. The letter is never dropped: an amount with a
// suffix this doesn't know is left unanswered, not shown as the bare number.
re!(
    MONEY_SCALE,
    r"(?i)([$£€¥₹₩₽₺])\s*(\d[\d,]*(?:\.\d+)?)\s*(mm|bn|k|m|b)\b"
);
re!(TOTAL_WORD, r"(?i)\b(?:total|sum|average|avg|mean)\b");
re!(CONVERSION_START, r"(?i)^(?:in|to|as)\b");
// A line of one word, which is a calculation only as a variable or a constant:
// "r" alone would be 1 Rankine, "kg" 1 kg.
re!(LONE_WORD, r"^[A-Za-zπτφ]+$");
re!(IDENT, r"\b[A-Za-z_][A-Za-z0-9_]*\b");
// fend answers "version" with its own version number; in a sheet it's prose.
re!(VERSION_WORD, r"(?i)\bversion\b");
// "2 lakh" -> 200000, "1.5 crore" -> 15000000.
re!(
    LAKH_CRORE,
    r"(?i)\b(\d[\d,]*(?:\.\d+)?)\s*(lakhs?|lacs?|crores?)\b"
);
// "11 in" is inches when nothing it could convert into follows: "5 ft 11 in",
// "12 in in cm", "3 in + 2 in" (but "100 in hex" stays a conversion).
re!(INCHES, r"(\d)\s*in(\s*(?:$|[+\-*/),]|(?:in|to|as)\b))");
// "20%off" -> "20% off".
re!(PCT_GLUED, r"(?i)%(off|on|of)\b");
// "100 - 20% of 50": the share is worked out first (100 - 10).
re!(
    PLUS_MINUS_PCT_OF,
    r"(?i)([+\-]\s*)(\d[\d,]*(?:\.\d+)?)\s*%\s+of\s+(\(?\$?\d[\d,]*(?:\.\d+)?\)?(?:\s+[A-Z]{3}\b)?)"
);
// "50 percent" -> "50%".
re!(PERCENT_WORD, r"(?i)(\d)\s*(?:percent|per\s+cent)\b");
// "half of 50", "twice $40", "a third of 90": quantities, not prose.
re!(HALF, r"(?i)\b(?:a\s+)?half(?:\s+of)?\s+");
re!(THIRD, r"(?i)\b(?:a\s+|one\s+)?third(?:\s+of)?\s+");
re!(QUARTER, r"(?i)\b(?:a\s+|one\s+)?quarter(?:\s+of)?\s+");
re!(TWICE, r"(?i)\b(?:twice|double)\s+");
re!(TRIPLE, r"(?i)\btriple\s+");
// "$4 a day" -> "$4 / day".
re!(
    A_DAY,
    r"(?i)(\d[\d,.]*\)?)\s+(?:a|an|per)\s+(day|week|month|year|hour|minute|night|person|head)s?\b"
);
// "$84 split 3 ways", "split $84 between 4 people" -> "$84 / 3".
re!(
    SPLIT,
    r"(?i)^(?:split\s+(.+?)|(.+?)\s+split)\s+(?:(\d+)\s+ways|(?:between|among|across|by)\s+(\d+)(?:\s+(?:people|persons|friends|of\s+us))?)$"
);
// "4 GB at 100 Mbps": how long a download takes ("(4 GB) / (100 Mbps)").
re!(
    TRANSFER,
    r"(?i)^(.*\d.*?\b(?:[kmgtpe]i?b|[kmgtpe]?bytes?|[kmgtpe]?bits?))\s+(?:at|over|@)\s+(.*\d.*?(?:bps|b/s|bits?/s|bytes?/s|per\s+second))$"
);
// "3 pizzas at $12" -> "3 pizzas * $12".
re!(AT_PRICE, r"(?i)(\d.*?)\s+at\s+([$£€¥₹(]|\d)");
// "$2.4M / $180k per month" is cash divided by a rate. fend would divide first and
// apply "per month" to the result (a huge number per month). The rate stays together.
// A bare count does not ("$5,000 / 12 per month" is a monthly amount).
re!(
    RATE_DIVISOR,
    r"/\s*([$£€¥₹]\s*\d[\d,]*(?:\.\d+)?[kKM]?|\d[\d,]*(?:\.\d+)?[kKM])\s*(?:(?i:per)|/)\s*((?i)months?|years?|weeks?|days?|hours?|minutes?|nights?|people|persons?|heads?|mo|yr)\b"
);
// "$180k per month in years": the same money over a year, not a conversion into time.
re!(
    RATE_IN_TIME,
    r"(?i)((?:per|/)\s*(?:months?|years?|weeks?|days?|mo|yr))\s+(?:in|to|as)\s+(years?|months?|weeks?|days?|yr|mo|annually)\b"
);
// "k" and "M" after a number are thousand and million ("$180k", "2.4M"), not a
// separate factor fend can slide past a division. Not "5km" or "3MB".
re!(SCALE_SUFFIX, r"(^|[^\w])(\d[\d,]*(?:\.\d+)?)([kKM])\b");
// "$5M at $20M pre" is a round: the raise and the pre-money valuation.
re!(
    AT_PRE,
    r"(?i)^\s*(.+?)\s+at\s+(?:an?\s+)?(.+?)\s+pre(?:-?money|\s+money)?\s*$"
);
// "$5M at $25M post" and "$5M post at $25M": the dilution of that round.
re!(
    AT_POST,
    r"(?i)^\s*(.+?)\s+at\s+(?:an?\s+)?(.+?)\s+post(?:-?money|\s+money)?\s*$"
);
re!(
    POST_AT,
    r"(?i)^\s*(.+?)\s+post(?:-?money|\s+money)?\s+at\s+(?:an?\s+)?(.+?)\s*$"
);
// "MRR", "monthly" and "annually" name the period of an amount. "in ARR" names
// the period to convert into.
re!(
    PERIOD_OF_RATE,
    r"(?i)((?:per|/)\s*(?:months?|years?|weeks?|days?|mo|yr))\s+(monthly|annually)\b"
);
re!(
    PERIOD_TARGET,
    r"(?i)\b(in|to|as)\s+(monthly|annually|MRR|ARR)\b"
);
re!(
    PERIOD_WORD,
    r"(?i)(\d[\d,.]*\)?(?:\s+[A-Z]{3})?|\)(?:\s+[A-Z]{3})?)\s+(monthly|annually|MRR|ARR)\b"
);
// A time of day the time module didn't claim ("Movie at 7pm"): not math.
re!(CLOCK_WORD, r"(?i)\b\d{1,2}(?::\d{2})?\s?[ap]\.?m\b");
// A label ending in a number, then the amount: "Day 3 $50", "iPhone 15 $799".
re!(
    LABEL_NUMBER,
    r"^[A-Za-z][^\d=]*?\d[\d,.]*\s+(\(?[$£€¥₹]?\s*\d.*)$"
);
// Two amounts side by side once prose is dropped ("Room 12 $50").
re!(SIDE_BY_SIDE, r"\d[\d,.]*\)?\s+(\(?[$£€¥₹]?\s*\d)");
re!(UNKNOWN_IDENT, r"unknown identifier '([^']+)'");
re!(
    BASE_CONV,
    r"(?i)\b(in|to|as)\s+(hex|hexadecimal|binary|bin|octal|oct|base\s*\d+)\b"
);
re!(WORD, r"\b[A-Za-z]{3}\b");
// fend writes negative money as "$-5".
re!(NUM_TOKEN, r"^(-?)([$£€¥₹]?)(-?)(\d+)(?:\.(\d+))?$");

const WORD_OPS: &[(&str, &str)] = &[
    (r"(?i)\bmultiplied\s+by\b", "*"),
    (r"(?i)\bdivided\s+by\b", "/"),
    (r"(?i)\btimes\b", "*"),
    (r"(?i)\bplus\b", "+"),
    (r"(?i)\bminus\b", "-"),
];

/// Units written as two words, mapped to the names fend knows.
const UNIT_ALIASES: &[(&str, &str)] = &[
    (r"(?i)\bnautical\s+miles?\b", "nautical_mile"),
    (r"(?i)\b(?:fluid\s+ounces?|fl\.?\s*oz)\b", "floz"),
    (r"(?i)\bshort\s+tons?\b", "short_ton"),
    (r"(?i)\bmetric\s+(?:tons?|tonnes?)\b", "tonne"),
    (r"(?i)\bsq\.?\s*(?:ft|feet|foot)\b", "ft^2"),
    (r"(?i)\bsq\.?\s*(?:m|meters?|metres?)\b", "m^2"),
    (r"(?i)\bus\s+gallons?\b", "gallon"),
];

/// Imperial measures fend doesn't have, as their metric size and display name.
const IMPERIAL: &[(&str, &str, &str)] = &[
    ("gallon", "4.54609 liters", "imperial gallons"),
    ("quart", "1.1365225 liters", "imperial quarts"),
    ("pint", "568.26125 ml", "imperial pints"),
    ("floz", "28.4130625 ml", "imperial fl oz"),
];

re!(
    IMPERIAL_UNIT,
    r"(?i)\b(?:imperial|uk|british)\s+(gallons?|quarts?|pints?|fluid\s+ounces?|fl\.?\s*oz)\b"
);
re!(
    IMPERIAL_TARGET,
    r"(?i)^(.*\S)\s+(?:in|to|as)\s+(?:imperial|uk|british)\s+(gallons?|quarts?|pints?|fluid\s+ounces?|fl\.?\s*oz)\s*$"
);
// "CHF 300": a currency code written before the amount.
re!(CODE_FIRST, r"\b([A-Z]{3})\s+(\d[\d,]*(?:\.\d+)?)\b");

fn imperial(word: &str) -> &'static (&'static str, &'static str, &'static str) {
    let w = word.to_ascii_lowercase();
    IMPERIAL
        .iter()
        .find(|(key, ..)| w.starts_with(&key[..3]) || (*key == "floz" && w.starts_with('f')))
        .unwrap_or(&IMPERIAL[0])
}

/// "40 liters in imperial gallons": fend has no imperial units, so divide by the
/// unit's size and name the answer ourselves.
fn imperial_target(line: &str) -> Option<(String, &'static str)> {
    let line = match LABEL.captures(line) {
        Some(c) => c.get(2).unwrap().as_str(),
        None => line,
    };
    let c = IMPERIAL_TARGET.captures(line)?;
    let (_, size, name) = imperial(&c[2]);
    Some((format!("({}) / ({size})", &c[1]), name))
}

static UNIT_ALIAS_RES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    UNIT_ALIASES
        .iter()
        .map(|(p, r)| (Regex::new(p).unwrap(), *r))
        .collect()
});

static WORD_OP_RES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    WORD_OPS
        .iter()
        .map(|(p, r)| (Regex::new(p).unwrap(), *r))
        .collect()
});

const SYMBOL_CODES: &[(char, &str)] = &[
    ('€', "EUR"),
    ('¥', "JPY"),
    ('₹', "INR"),
    ('₩', "KRW"),
    ('₽', "RUB"),
    ('₺', "TRY"),
];

impl Engine {
    pub fn new(rates: Rates) -> Self {
        Self { rates, clock: None }
    }

    /// Limit each line by time, read from `clock` (milliseconds).
    pub fn with_clock(mut self, clock: fn() -> f64) -> Self {
        self.clock = Some(clock);
        self
    }

    fn context(&self) -> fend_core::Context {
        let mut ctx = fend_core::Context::new();
        ctx.set_exchange_rate_handler_v2(self.rates.clone());
        ctx
    }

    /// Evaluate every line of a sheet. Lines share one context, so variables
    /// defined above are visible below.
    pub fn eval_document(&self, text: &str) -> Vec<LineResult> {
        let defs = defined_variables(text);
        let mut st = self.fresh();
        text.split('\n')
            .enumerate()
            .map(|(i, line)| self.eval_in(&mut st, i, line, &defs))
            .collect()
    }

    /// Like `eval_document`, reusing `sheet`'s work from the last call: lines above
    /// the first one that changed keep their answers, and calculation resumes from
    /// the state just before it. Typing at the end of a long sheet redoes one line.
    pub fn eval_sheet(&self, text: &str, sheet: &mut Sheet) -> Vec<LineResult> {
        self.eval_sheet_until(text, sheet, || false)
            .expect("never stopped")
    }

    /// `eval_sheet` that gives up (returning `None`) when `stop` says so, checked
    /// between lines: newer text has arrived. The lines done so far are kept for
    /// the next call.
    pub fn eval_sheet_until(
        &self,
        text: &str,
        sheet: &mut Sheet,
        stop: impl Fn() -> bool,
    ) -> Option<Vec<LineResult>> {
        let lines: Vec<&str> = text.split('\n').collect();
        let defs = defined_variables(text);
        let defs_changed = sheet.defs != defs;
        sheet.defs = defs.clone();
        let version = self.rates.version();
        let same = if sheet.rates_version == version && !defs_changed {
            lines
                .iter()
                .zip(&sheet.lines)
                .take_while(|(a, b)| **a == b.as_str())
                .count()
        } else {
            0
        };
        sheet.lines.truncate(same);
        sheet.results.truncate(same);
        sheet.after.truncate(same);
        sheet.rates_version = version;
        // Answers from the clock ("today", "time in Tokyo") are fresh every time;
        // they never change the state the lines below see.
        for (line, result) in sheet.lines.iter().zip(sheet.results.iter_mut()) {
            if result.clock
                && let Some(now) = crate::time::eval(markdown::math_body(line).trim())
            {
                result.display = now;
            }
        }
        let mut st = sheet.after.last().cloned().unwrap_or_else(|| self.fresh());
        for (i, line) in lines.iter().enumerate().skip(same) {
            if stop() {
                return None;
            }
            let result = self.eval_in(&mut st, i, line, &defs);
            sheet.lines.push((*line).to_string());
            sheet.results.push(result);
            sheet.after.push(st.clone());
        }
        Some(sheet.results.clone())
    }

    fn fresh(&self) -> State {
        State {
            ctx: self.context(),
            block: Vec::new(),
            prev: None,
            vars: HashSet::new(),
            percents: Vec::new(),
        }
    }

    /// Line `i` of a sheet, given the state the lines above it left.
    fn eval_in(&self, st: &mut State, i: usize, line: &str, defs: &HashSet<String>) -> LineResult {
        let class = markdown::classify(line);
        match class.form {
            markdown::Form::Blank => {
                st.block.clear();
                return LineResult::new(Kind::Empty);
            }
            markdown::Form::Comment => return LineResult::new(Kind::Comment),
            // A table is drawn in the preview. Its cells are not math, and it
            // does not start a new block the way a blank line does.
            markdown::Form::Table => return LineResult::new(Kind::Error),
            _ => {}
        }
        let trimmed = class.body.trim();
        let prefixed = !matches!(class.form, markdown::Form::Text);
        // A heading, list item or quote is prose unless what remains is a
        // calculation. "Week 1" stays a heading; "hotel = 3 nights * $180" runs.
        // A prose heading still ends the block that `total` adds up.
        if prefixed && (trimmed.is_empty() || !looks_like_math(trimmed, &st.vars)) {
            if matches!(class.form, markdown::Form::Heading(_)) {
                st.block.clear();
                return LineResult::new(Kind::Header);
            }
            return LineResult::new(Kind::Error);
        }
        let result = self.eval_math(st, i, trimmed, defs);
        if matches!(class.form, markdown::Form::Heading(_)) && !shown_math(&result) {
            st.block.clear();
            return LineResult::new(Kind::Header);
        }
        result
    }

    /// The calculation on one line, after any markdown prefix has been taken off.
    fn eval_math(
        &self,
        st: &mut State,
        i: usize,
        trimmed: &str,
        defs: &HashSet<String>,
    ) -> LineResult {
        if let Some(date) = crate::time::eval(trimmed) {
            return LineResult {
                clock: true,
                ..LineResult::value(date)
            };
        }
        if crate::time::impossible_date(trimmed) {
            return LineResult::error(Some("Not a date"));
        }
        if let Some(extra) = crate::extras::eval(trimmed) {
            return LineResult {
                color: extra.color,
                ..LineResult::value(extra.display)
            };
        }

        // A time of day the time module didn't understand isn't math
        // ("Movie at 7pm" would be 7 picometres). Very long lines are
        // skipped: fend's parser can take minutes on them.
        if CLOCK_WORD.is_match(trimmed) || trimmed.len() > 2000 {
            return LineResult::new(Kind::Error);
        }
        if st.prev.is_none() && PREV.is_match(trimmed) && !ASSIGN.is_match(trimmed) {
            return LineResult::new(Kind::Error);
        }
        if LONE_WORD.is_match(trimmed)
            && !st.vars.contains(trimmed)
            && aggregate(trimmed).is_none()
            && !PREV.is_match(trimmed)
            && !matches!(trimmed, "pi" | "π" | "e" | "tau" | "τ" | "phi" | "φ")
        {
            // "kg" or "USD" alone is a unit, not a missing variable. "nope" is.
            return match self.name_hint(trimmed, &st.vars, defs) {
                Some(h) if h.starts_with("Unknown name") && self.fend_knows_word(trimmed) => {
                    LineResult::new(Kind::Error)
                }
                Some(h) => LineResult::hint(h),
                None => LineResult::new(Kind::Error),
            };
        }

        if self.clock.is_some() {
            warm_up();
        }
        let budget = Budget::new(self.clock, LINE_MS);
        let var = format!("_l{i}");
        // `sum`, or `subtotal = sum` to keep it in a variable.
        let whole = match ASSIGN.captures(trimmed) {
            Some(c) => aggregate(&c[2]).map(|avg| (Some(c[1].to_string()), avg)),
            None => aggregate(trimmed).map(|avg| (None, avg)),
        };
        let mut is_total = whole.is_some();
        let target = imperial_target(trimmed);
        let source = target.as_ref().map_or(trimmed, |(expr, _)| expr.as_str());
        let result = match whole {
            Some((name, avg)) => {
                total_into(&mut st.ctx, &budget, &var, &st.block, avg, name.as_deref())
            }
            None => {
                // "$100 + tax", where tax = 8%: the same as "$100 + 8%".
                let mut source = source.to_string();
                for (_, re, pct) in &st.percents {
                    source = re.replace_all(&source, format!("${{1}}{pct}")).into_owned();
                }
                // "total * 1.08": the total goes in a variable of its own first.
                let (source, used) = totals_inside(&source, &st.vars, i);
                is_total |= used.iter().any(Option::is_some);
                let mut vars = std::borrow::Cow::Borrowed(&st.vars);
                for name in used.iter().flatten() {
                    vars.to_mut().insert(name.clone());
                }
                let expr = self.currency_first(&rewrite(&source, st.prev.as_deref(), &vars));
                let expr = self.drop_name_collisions(&expr, &vars);
                if expr.trim().is_empty() {
                    return LineResult::new(Kind::Comment);
                }
                let totals_ok = used.iter().zip([false, true]).all(|(name, avg)| {
                    name.as_ref().is_none_or(|name| {
                        total_into(&mut st.ctx, &budget, name, &st.block, avg, None).is_some()
                    })
                });
                let mut eval = |expr: &str| {
                    eval_forgiving(&mut st.ctx, &budget, &var, expr)
                        .or_else(|| self.with_currency(&mut st.ctx, &budget, &var, expr))
                };
                match totals_ok.then(|| eval(&expr)).flatten() {
                    // "Nails $8" is 8 nail $ to fend, "Food 6 days * $80" 480 day $:
                    // money times a unit means a label's words were read as units.
                    Some(raw) if self.money_times_unit(&raw) => {
                        let prose = self.units_as_prose(&expr, &vars);
                        eval(&prose).filter(|raw| !self.money_times_unit(raw))
                    }
                    raw => raw,
                }
            }
        };

        if result.is_some()
            && let Some(c) = ASSIGN.captures(trimmed)
        {
            let name = c[1].to_string();
            let value = c[2].trim();
            // A redefinition replaces the old value, percentage or not.
            st.percents.retain(|(n, ..)| *n != name);
            if value.ends_with('%') && value[..value.len() - 1].trim().parse::<f64>().is_ok() {
                let re = Regex::new(&format!(r"([+-]\s*)\b{}\b", regex::escape(&name)));
                if let Ok(re) = re {
                    st.percents.push((name.clone(), re, value.to_string()));
                }
            }
            st.vars.insert(name);
        }

        // Every digit of an approximate answer (see `precisely`), except for
        // base conversions, which fend formats itself.
        let result = result.map(|raw| {
            if BASE_CONV.is_match(trimmed) {
                raw
            } else {
                precisely(&mut st.ctx, &budget, &var, raw)
            }
        });

        match result {
            Some(raw) => {
                // A total shouldn't count itself in the next total or average.
                if !is_total {
                    st.block.push(Term {
                        var: var.clone(),
                        plain: is_plain_number(&raw),
                        aside: raw.ends_with('%') || raw.contains(" / "),
                    });
                }
                st.prev = Some(var);
                let display = if BASE_CONV.is_match(trimmed) {
                    raw.trim_start_matches("approx. ").to_string()
                } else if let Some(time) = transfer_time(trimmed, &raw) {
                    time
                } else {
                    pretty(&raw)
                };
                let display = match target {
                    Some((_, name)) => format!("{display} {name}"),
                    None => display,
                };
                // "$5M at $20M pre" is the post-money sum. Say the dilution too.
                let display = match pre_money_terms(trimmed) {
                    Some((raise, pre)) => {
                        let share = self.round_share(
                            &mut st.ctx,
                            &budget,
                            &raise,
                            &pre,
                            st.prev.as_deref(),
                            &st.vars,
                        );
                        match share {
                            Some(pct) => format!("{display} post, {pct}"),
                            None => format!("{display} post"),
                        }
                    }
                    None => display,
                };
                LineResult::value(display)
            }
            None => {
                // A "mod" note that the line didn't actually write is suppressed on
                // purpose. Don't replace that silence with a name hint.
                let raw = budget.note();
                let noted = raw
                    .filter(|n| !n.starts_with("mod ") || MOD_WRITTEN.is_match(trimmed))
                    .or_else(|| crate::time::note(trimmed));
                match noted {
                    Some(n) => LineResult::error(Some(n)),
                    None if raw.is_some() => LineResult::error(None),
                    None => match self.name_hint(trimmed, &st.vars, defs) {
                        Some(h) => LineResult::hint(h),
                        None => LineResult::error(None),
                    },
                }
            }
        }
    }

    /// An answer in money times some other unit ("480 day $"), which nobody means.
    fn money_times_unit(&self, raw: &str) -> bool {
        let tokens: Vec<&str> = raw.trim_start_matches("approx. ").split(' ').collect();
        let money = |t: &str| t.contains(['$', '£', '€', '¥', '₹']) || self.rates.knows(t);
        !tokens.contains(&"/")
            && tokens.iter().any(|t| money(t))
            && tokens
                .iter()
                .any(|t| t.chars().all(char::is_alphabetic) && !money(t))
    }

    /// `expr` with the words fend would read as units made unknown, so they're
    /// dropped as a label's words are: variables, currencies and a few words that
    /// do something stay.
    fn units_as_prose(&self, expr: &str, vars: &HashSet<String>) -> String {
        let (lhs, rhs) = match ASSIGN.captures(expr) {
            Some(c) => (format!("{} = ", &c[1]), c[2].to_string()),
            None => (String::new(), expr.to_string()),
        };
        let rhs = LETTERS.replace_all(&rhs, |c: &regex::Captures| {
            let m = c.get(0).unwrap();
            let w = m.as_str();
            let before = &rhs[..m.start()];
            let glued = before.ends_with(|ch: char| ch.is_ascii_digit() || ch == '_');
            // A suffix stuck to the amount ("($5)m", "($5)kg"). Dropping it would
            // show the bare amount, a different number. Leave it, so the line stays
            // unanswered unless the suffix was a scale and already expanded.
            let keep = vars.contains(w)
                || self.rates.knows(w)
                || (glued && matches!(w, "k" | "K" | "M"))
                || before.ends_with('_')
                || stuck_to_money(before)
                || matches!(
                    w.to_ascii_lowercase().as_str(),
                    "in" | "to"
                        | "as"
                        | "of"
                        | "per"
                        | "mod"
                        | "pi"
                        | "e"
                        | "sqrt"
                        | "ln"
                        | "log"
                        | "abs"
                        | "round"
                        | "floor"
                        | "ceil"
                        | "sin"
                        | "cos"
                        | "tan"
                );
            if keep {
                w.to_string()
            } else {
                format!(" _w{w}")
            }
        });
        format!("{lhs}{rhs}")
    }

    /// The dilution of a round, "20%", from the raise and the pre-money amount.
    fn round_share(
        &self,
        ctx: &mut fend_core::Context,
        budget: &Budget,
        raise: &str,
        pre: &str,
        prev: Option<&str>,
        vars: &HashSet<String>,
    ) -> Option<String> {
        let expr = format!("(({raise}) / (({raise}) + ({pre}))) in %");
        let expr =
            self.drop_name_collisions(&self.currency_first(&rewrite(&expr, prev, vars)), vars);
        let raw = budget.evaluate(&expr, ctx).ok()?;
        let shown = pretty(raw.get_main_result().trim());
        shown.ends_with('%').then_some(shown)
    }

    /// "CHF 300" -> "300 CHF", for codes fend has a rate for.
    fn currency_first(&self, expr: &str) -> String {
        CODE_FIRST
            .replace_all(expr, |c: &regex::Captures| {
                if self.rates.knows(&c[1]) {
                    format!("{} {}", &c[2], &c[1])
                } else {
                    c[0].to_string()
                }
            })
            .into_owned()
    }

    /// "Lunch with Bob $40", "Lunch for Cad $40": fend reads a title-case word that
    /// spells a currency code as that currency. It's prose unless it follows a
    /// number ("50 Cad") or names a conversion ("in Cad").
    ///
    /// An assignment's left-hand side is always a variable, and so is a name
    /// already defined (`Try = 5`, then `Try`). A lone `Try`, or `Try * 2` before
    /// that, stays in the expression so the line can say it's unknown instead of
    /// disappearing.
    fn drop_name_collisions(&self, expr: &str, vars: &HashSet<String>) -> String {
        let assigned = ASSIGN.captures(expr).map(|c| c[1].to_string());
        WORD.replace_all(expr, |c: &regex::Captures| {
            let w = &c[0];
            if vars.contains(w) || assigned.as_deref() == Some(w) {
                return w.to_string();
            }
            let m = c.get(0).unwrap();
            let title = w.chars().next().is_some_and(char::is_uppercase)
                && w.chars().skip(1).all(char::is_lowercase);
            let before = expr[..m.start()].trim_end();
            let after = expr[m.end()..].trim_start();
            let alone = before.is_empty() && after.is_empty();
            let in_expr = before.ends_with(|ch: char| "+-*/^=(,".contains(ch))
                || after.starts_with(|ch: char| "+-*/^=),".contains(ch));
            if alone || in_expr {
                return w.to_string();
            }
            let after_amount = before.ends_with(|ch: char| ch.is_ascii_digit() || ch == ')');
            let after_conversion = before
                .rsplit(|ch: char| !ch.is_alphabetic())
                .next()
                .is_some_and(|word| {
                    matches!(
                        word.to_ascii_lowercase().as_str(),
                        "in" | "to" | "as" | "per"
                    )
                });
            let upper = w.to_ascii_uppercase();
            if title && !after_amount && !after_conversion && self.rates.knows(&upper) {
                String::new()
            } else {
                w.to_string()
            }
        })
        .into_owned()
    }

    /// A one-word line fend already understands as a unit or a constant ("kg",
    /// "hour"). Those stay blank; a word it doesn't know is an undefined name.
    fn fend_knows_word(&self, word: &str) -> bool {
        let mut ctx = self.context();
        let budget = Budget::new(None, LINE_MS);
        budget.evaluate(word, &mut ctx).is_ok()
    }

    /// Why a line that didn't calculate mentions a name: a different case, a
    /// definition further down the sheet, or a name that isn't defined at all.
    fn name_hint(
        &self,
        line: &str,
        above: &HashSet<String>,
        defs: &HashSet<String>,
    ) -> Option<String> {
        let code = line.split("//").next().unwrap_or(line);
        let lhs = ASSIGN
            .captures(code)
            .and_then(|c| c.get(1))
            .map(|m| m.range());
        let mut meant = None;
        let mut below = None;
        let mut unknown = None;
        // A word that isn't part of the calculation ("bonus" in "sales + a bonus").
        // One of those means the line is a sentence, not a missing name.
        let mut loose = false;
        for c in IDENT.captures_iter(code) {
            let m = c.get(0).unwrap();
            let w = m.as_str();
            if lhs.as_ref().is_some_and(|r| r.contains(&m.start())) {
                continue;
            }
            if above.contains(w) {
                continue;
            }
            if unit_position(code, m.start()) || call_position(code, m.end()) {
                continue;
            }
            if let Some(name) = case_suggestion(w, above, defs) {
                meant.get_or_insert_with(|| format!("Did you mean {name}?"));
                continue;
            }
            if defs.contains(w) {
                below.get_or_insert_with(|| format!("{w} is defined below"));
                continue;
            }
            if is_name_keyword(w) || self.bare_currency(w) {
                continue;
            }
            // "Alex $90k ... + a bonus" mentions a plus sign, but Alex isn't in
            // the calculation. Only a name sitting where a value would.
            if code.trim() == w || expr_position(code, m.start(), m.end()) {
                unknown.get_or_insert_with(|| format!("Unknown name '{w}'"));
            } else {
                loose = true;
            }
        }
        if let Some(s) = meant {
            return Some(s);
        }
        if let Some(s) = below {
            return Some(s);
        }
        if unknown.is_some() && !loose && mentions_math(code) {
            return unknown;
        }
        None
    }

    fn bare_currency(&self, w: &str) -> bool {
        w.len() == 3 && w.bytes().all(|b| b.is_ascii_uppercase()) && self.rates.knows(w)
    }

    /// "rent + 5" where rent is $1,000: a plain number added to or taken from an
    /// amount is in its currency. Only tried when fend refused to mix the two.
    fn with_currency(
        &self,
        ctx: &mut fend_core::Context,
        budget: &Budget,
        var: &str,
        expr: &str,
    ) -> Option<String> {
        let unit = currency_vs_plain(budget)?;
        let (lhs, rhs) = match ASSIGN.captures(expr) {
            Some(c) => (Some(c[1].to_string()), c[2].to_string()),
            None => (None, expr.to_string()),
        };
        let mut rhs = plain_terms_in(&rhs, &unit);
        loop {
            let next = BARE_NUMBER
                .replace_all(&rhs, |c: &regex::Captures| {
                    let amount = if unit.chars().all(|ch| ch.is_ascii_alphabetic()) {
                        format!("({} {unit})", &c[2])
                    } else {
                        format!("({unit}{})", &c[2])
                    };
                    format!("{}{amount}{}", &c[1], &c[3])
                })
                .into_owned();
            if next == rhs {
                break;
            }
            rhs = next;
        }
        let expr = match lhs {
            Some(name) => format!("{name} = {rhs}"),
            None => rhs,
        };
        eval_into(ctx, budget, var, &expr)
    }

    /// Evaluate a single expression in a fresh context (used by the CLI).
    pub fn eval_line(&self, line: &str) -> Option<String> {
        self.eval_document(line)
            .pop()
            .filter(|r| r.kind == Kind::Value)
            .map(|r| r.display)
    }
}

/// Where each line of `new` text was in `old`, for moving answers along with their
/// lines: lines unchanged from the top and from the bottom are found, the lines
/// that changed (`None`) aren't. The flag is whether the line is below the edit,
/// where its old answer may be out of date.
pub fn realign_map(old: &str, new: &str) -> Vec<(Option<usize>, bool)> {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.split('\n').collect(), new.split('\n').collect());
    let top = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let bottom = a[top..]
        .iter()
        .rev()
        .zip(b[top..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let same = a.len() == b.len() && top == a.len();
    (0..b.len())
        .map(|i| {
            if i < top {
                (Some(i), false)
            } else if i >= b.len() - bottom {
                (Some(i + a.len() - b.len()), !same)
            } else {
                (None, false)
            }
        })
        .collect()
}

/// Answers worked out for `old` text, moved to where their lines are in `new`, for
/// showing while `new`'s own answers are on their way (by position alone, inserting
/// a line showed every answer below it one line off). Answers below the edit are
/// marked stale; the lines that changed have none yet.
pub fn realign(old: &str, results: &[LineResult], new: &str) -> Vec<LineResult> {
    realign_onto(old, results, new, &[])
}

/// `realign` for answers that may cover only the first lines of `old` so far (a
/// calculation part way through): where `results` has no answer for a line, the one
/// in `shown` (already lined up with `new`) stays, marked stale.
pub fn realign_onto(
    old: &str,
    results: &[LineResult],
    new: &str,
    shown: &[LineResult],
) -> Vec<LineResult> {
    realign_map(old, new)
        .into_iter()
        .enumerate()
        .map(|(i, (from, below))| {
            let (r, stale) = match from.and_then(|j| results.get(j)) {
                Some(r) => (r, below),
                None => match shown.get(i) {
                    Some(r) => (r, true),
                    None => return LineResult::new(Kind::Empty),
                },
            };
            LineResult {
                stale: r.stale || stale,
                ..r.clone()
            }
        })
        .collect()
}

/// The sheet as plain text with each answer lined up after its line, the way the
/// app shows it, for pasting into a message or a document.
pub fn with_answers(text: &str, results: &[LineResult]) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let answered = |i: usize| results.get(i).is_some_and(|r| !r.display.is_empty());
    let width = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| answered(*i))
        .map(|(_, l)| l.trim_end().chars().count())
        .max()
        .unwrap_or(0)
        .min(56)
        + 4;
    let mut out: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line = line.trim_end();
            match results.get(i).filter(|_| answered(i)) {
                Some(r) => format!("{line:<width$}{}", r.display),
                None => line.to_string(),
            }
        })
        .collect();
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out.join("\n")
}

/// The sheet as a Markdown table, for Slack, GitHub or a doc: each line beside its
/// answer. The first header names the table; later headers become bold rows, and
/// blank lines and `//` notes are left out.
pub fn markdown_table(text: &str, results: &[LineResult]) -> String {
    let cell = |s: &str| s.trim().replace('|', "\\|");
    let mut lines = text.split('\n').zip(results).peekable();
    while lines.peek().is_some_and(|(l, _)| l.trim().is_empty()) {
        lines.next();
    }
    let title = match lines.peek() {
        Some((l, r)) if r.kind == Kind::Header => {
            let t = cell(l.trim_start_matches('#'));
            lines.next();
            t
        }
        _ => String::new(),
    };
    let mut out = vec![format!("| {title} | |"), "|---|---:|".to_string()];
    for (line, r) in lines {
        match r.kind {
            Kind::Empty | Kind::Comment => {}
            Kind::Header => out.push(format!("| **{}** | |", cell(line.trim_start_matches('#')))),
            _ => out.push(format!("| {} | {} |", cell(line), cell(&r.display))),
        }
    }
    out.join("\n")
}

/// Names assigned anywhere in the sheet (`rent = $1200`), for highlighting.
pub fn defined_variables(text: &str) -> std::collections::HashSet<String> {
    text.lines()
        .filter_map(|l| {
            ASSIGN
                .captures(markdown::math_body(l))
                .map(|c| c[1].to_string())
        })
        .collect()
}

/// `sum`, `total`, `avg`, `average` over the values since the last blank line or header.
/// A line that's just `total` or `average`: whether it's an average.
fn aggregate(line: &str) -> Option<bool> {
    let lower = line.to_ascii_lowercase();
    match lower.trim_end_matches(':').trim() {
        "sum" | "total" => Some(false),
        "avg" | "average" | "mean" => Some(true),
        _ => None,
    }
}

/// The lines above, added up (or averaged) into `var`, and kept in `name` too
/// when given. Plain numbers among dollars count as dollars, as on one line, and
/// percentages and rates are left out when the amounts don't add up with them.
fn total_into(
    ctx: &mut fend_core::Context,
    budget: &Budget,
    var: &str,
    block: &[Term],
    avg: bool,
    name: Option<&str>,
) -> Option<String> {
    let amounts: Vec<Term> = block.iter().filter(|t| !t.aside).cloned().collect();
    let mut tries = vec![block];
    if amounts.len() < block.len() {
        tries.push(&amounts);
    }
    tries
        .into_iter()
        .find_map(|block| sum_into(ctx, budget, var, block, avg, name))
}

fn sum_into(
    ctx: &mut fend_core::Context,
    budget: &Budget,
    var: &str,
    block: &[Term],
    avg: bool,
    name: Option<&str>,
) -> Option<String> {
    let expr = |unit: Option<&str>| {
        let terms: Vec<String> = block
            .iter()
            .map(|t| match unit {
                Some(u) if t.plain && u.chars().all(|ch| ch.is_ascii_alphabetic()) => {
                    format!("({} * 1 {u})", t.var)
                }
                Some(u) if t.plain => format!("({} * ({u}1))", t.var),
                _ => t.var.clone(),
            })
            .collect();
        let sum = match (terms.is_empty(), avg) {
            (true, _) => "0".to_string(),
            (false, false) => terms.join(" + "),
            (false, true) => format!("({}) / {}", terms.join(" + "), terms.len()),
        };
        match name {
            Some(name) => format!("{name} = {sum}"),
            None => sum,
        }
    };
    eval_into(ctx, budget, var, &expr(None)).or_else(|| {
        let unit = currency_vs_plain(budget)?;
        eval_into(ctx, budget, var, &expr(Some(&unit)))
    })
}

/// `total` and `average` used in a calculation ("total * 1.08", "100 - total"),
/// replaced with variables `_t{i}` and `_a{i}` for the caller to fill in: returned
/// beside the expression, as Some when used. Only next to an operator, a bracket or
/// a conversion: "rent + total vibes" is prose. A variable of that name wins.
fn totals_inside(expr: &str, vars: &HashSet<String>, i: usize) -> (String, [Option<String>; 2]) {
    let mut used = [None, None];
    let out = TOTAL_WORD
        .replace_all(expr, |c: &regex::Captures| {
            let m = c.get(0).unwrap();
            let before = expr[..m.start()].trim_end();
            let after = expr[m.end()..].trim_start();
            let ok_before =
                before.is_empty() || before.ends_with(|ch: char| "+-*/×÷^(=,".contains(ch));
            let ok_after = after.is_empty()
                || after.starts_with(|ch: char| "+-*/×÷^)".contains(ch))
                || CONVERSION_START.is_match(after);
            if vars.contains(m.as_str()) || !ok_before || !ok_after {
                return m.as_str().to_string();
            }
            let avg = !matches!(m.as_str().to_ascii_lowercase().as_str(), "total" | "sum");
            let name = format!("{}{i}", if avg { "_a" } else { "_t" });
            used[usize::from(avg)] = Some(name.clone());
            name
        })
        .into_owned();
    (out, used)
}

/// Terms of a sum that are plain arithmetic ("$200 - 5% * 2" has "(5 / 100) * 2")
/// given the currency, as a lone number already is.
fn plain_terms_in(expr: &str, unit: &str) -> String {
    let mut terms = Vec::new();
    let (mut depth, mut start, mut last) = (0i32, 0, ' ');
    for (at, ch) in expr.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            '+' | '-' if depth == 0 && (last.is_ascii_digit() || last == ')') => {
                terms.push(&expr[start..at]);
                terms.push(&expr[at..at + 1]);
                start = at + 1;
            }
            _ => {}
        }
        if !ch.is_whitespace() {
            last = ch;
        }
    }
    terms.push(&expr[start..]);
    terms
        .into_iter()
        .map(|t| {
            let arithmetic = t.chars().any(|ch| "*/^".contains(ch))
                && t.chars()
                    .all(|ch| ch.is_ascii_digit() || " .,()*/^".contains(ch));
            match arithmetic {
                false => t.to_string(),
                true if unit.chars().all(|ch| ch.is_ascii_alphabetic()) => {
                    format!(" (({}) * 1 {unit}) ", t.trim())
                }
                true => format!(" (({}) * ({unit}1)) ", t.trim()),
            }
        })
        .collect::<String>()
}

/// What fend's error says when dollars meet a plain number: the currency.
fn currency_vs_plain(budget: &Budget) -> Option<String> {
    let error = budget.error.lock().unwrap().clone()?;
    let c = CURRENCY_VS_PLAIN.captures(&error)?;
    match (&c[3], &c[4]) {
        ("unitless", "BASE_CURRENCY") => Some(c[2].to_string()),
        ("BASE_CURRENCY", "unitless") => Some(c[1].to_string()),
        _ => None,
    }
}

/// fend's answer is a number with no unit: "8", "-1,234.5", "approx. 3.33".
fn is_plain_number(raw: &str) -> bool {
    raw.trim_start_matches("approx. ")
        .replace(',', "")
        .parse::<f64>()
        .is_ok()
}

/// "year", "years", "yr" and "annually" are one period, and the same for months.
fn canonical_period(word: &str) -> &'static str {
    match word.to_ascii_lowercase().as_str() {
        "year" | "years" | "yr" | "annually" | "arr" => "year",
        "month" | "months" | "mo" | "monthly" | "mrr" => "month",
        "week" | "weeks" => "week",
        "day" | "days" => "day",
        _ => "year",
    }
}

/// The currency already written on the line, as the unit of a rate ("$/year").
fn money_per(expr: &str, period: &str) -> String {
    let period = canonical_period(period);
    if expr.contains('£') || expr.contains("GBP") {
        // "GBP/year" prints as "1,080,000 GBP / year". "£/year" prints as "1080000£ / year".
        format!("GBP/{period}")
    } else if expr.contains('€') || expr.contains("EUR") {
        format!("EUR/{period}")
    } else if expr.contains('₹') || expr.contains("INR") {
        format!("INR/{period}")
    } else if expr.contains('¥') || expr.contains("JPY") {
        format!("JPY/{period}")
    } else if expr.contains('$') || expr.contains("USD") {
        format!("$/{period}")
    } else {
        format!("1/{period}")
    }
}

/// "$5m" -> "$5000000", "$2.5bn" -> "$2500000000". Only after a currency symbol.
fn expand_money_scales(s: &str) -> String {
    MONEY_SCALE
        .replace_all(s, |c: &regex::Captures| {
            let places = match c[3].to_ascii_lowercase().as_str() {
                "k" => 3,
                "m" | "mm" => 6,
                "b" | "bn" => 9,
                _ => return c[0].to_string(),
            };
            format!("{}{}", &c[1], shift_point(&c[2].replace(',', ""), places))
        })
        .into_owned()
}

/// "$180k" -> "$180000", so the thousand stays on the number. Lowercase "m" is
/// metres here; a million only when a currency symbol marks it (see `MONEY_SCALE`).
fn expand_scales(s: &str) -> String {
    SCALE_SUFFIX
        .replace_all(s, |c: &regex::Captures| {
            let places = if &c[3] == "M" { 6 } else { 3 };
            format!("{}{}", &c[1], shift_point(&c[2].replace(',', ""), places))
        })
        .into_owned()
}

/// "/ $180k per month" -> "/ ($180k / month)", before the suffix is expanded.
fn group_rate_divisors(s: &str) -> String {
    RATE_DIVISOR
        .replace_all(s, |c: &regex::Captures| {
            format!("/ ({} / {})", c[1].trim(), c[2].to_ascii_lowercase())
        })
        .into_owned()
}

/// "per month in years" -> "per month in $/year", when this rate is the whole line.
/// A slash further left is a duration ("cash / burn in years"); fend converts that.
fn rescale_rates(s: &str) -> String {
    RATE_IN_TIME
        .replace_all(s, |c: &regex::Captures| {
            let at = c.get(0).unwrap().start();
            if s[..at].contains('/') {
                return c[0].to_string();
            }
            format!("{} in {}", &c[1], money_per(s, &c[2]))
        })
        .into_owned()
}

fn period_rate(word: &str) -> &'static str {
    match word.to_ascii_lowercase().as_str() {
        "annually" | "arr" => "per year",
        _ => "per month",
    }
}

/// A round written out: pre-money gives the post-money sum, post-money the dilution.
fn valuation(rhs: &str) -> Option<String> {
    let rhs = rhs.trim();
    if let Some(c) = AT_PRE.captures(rhs) {
        return Some(format!("({}) + ({})", c[1].trim(), c[2].trim()));
    }
    if let Some(c) = AT_POST.captures(rhs) {
        return Some(format!("(({}) / ({})) in %", c[1].trim(), c[2].trim()));
    }
    if let Some(c) = POST_AT.captures(rhs) {
        return Some(format!("(({}) / ({})) in %", c[1].trim(), c[2].trim()));
    }
    None
}

/// The part of a line a calculation sees: no comment, no label, and the right of `=`.
fn calc_rhs(line: &str) -> String {
    let line = match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    };
    let line = line.trim();
    let owned;
    let line = match LABEL.captures(line) {
        Some(c) => {
            owned = c[2].trim().to_string();
            owned.as_str()
        }
        None => line,
    };
    match ASSIGN.captures(line) {
        Some(c) => c[2].trim().to_string(),
        None => line.to_string(),
    }
}

/// "$5M at $20M pre", if this line is that round. The two amounts, as written.
fn pre_money_terms(line: &str) -> Option<(String, String)> {
    let rhs = calc_rhs(line);
    let c = AT_PRE.captures(&rhs)?;
    Some((c[1].trim().to_string(), c[2].trim().to_string()))
}

fn map_rhs(s: &str, f: impl FnOnce(&str) -> String) -> String {
    match ASSIGN.captures(s) {
        Some(c) => format!("{} = {}", &c[1], f(c[2].trim())),
        None => f(s),
    }
}

/// Rounds, and words for a period ("MRR", "annually"), on the right of `=`.
fn founder_line(s: &str, vars: &HashSet<String>) -> String {
    map_rhs(s, |rhs| {
        if let Some(done) = valuation(rhs) {
            return done;
        }
        let kept = |word: &str| vars.contains(word);
        let s = PERIOD_OF_RATE
            .replace_all(rhs, |c: &regex::Captures| {
                let into = if c[2].eq_ignore_ascii_case("monthly") {
                    "months"
                } else {
                    "years"
                };
                format!("{} in {into}", &c[1])
            })
            .into_owned();
        let s = PERIOD_TARGET
            .replace_all(&s, |c: &regex::Captures| {
                if kept(&c[2]) {
                    c[0].to_string()
                } else {
                    format!("{} {}", &c[1], canonical_period(&c[2]))
                }
            })
            .into_owned();
        PERIOD_WORD
            .replace_all(&s, |c: &regex::Captures| {
                if kept(&c[2]) {
                    c[0].to_string()
                } else {
                    format!("{} {}", &c[1], period_rate(&c[2]))
                }
            })
            .into_owned()
    })
}

/// `sum(1, 2, 3)` and the same for `total`, `avg`, `average`, `mean`, `min`, `max`,
/// turned into arithmetic fend can evaluate. A comma inside the parentheses
/// separates arguments unless it groups thousands (`1,000`, `$1,000`). An
/// unclosed call is left as written. Nested calls are expanded from the inside.
fn expand_calls(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if let Some((name, paren)) = match_call(&chars, i) {
            if let Some(end) = matching_paren(&chars, paren) {
                let inside: String = chars[paren + 1..end].iter().collect();
                let args: Vec<String> = split_args(&inside)
                    .into_iter()
                    .filter(|a| !a.is_empty())
                    .map(|a| expand_calls(&a))
                    .collect();
                match render_call(name, &args) {
                    Some(rendered) => out.push_str(&rendered),
                    None => out.extend(chars[i..=end].iter()),
                }
                i = end + 1;
                continue;
            }
            // No closing parenthesis: leave the rest of the line alone.
            out.extend(chars[i..].iter());
            break;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

const CALLS: &[&str] = &["average", "total", "mean", "sum", "avg", "min", "max"];

/// A function name at `i`, and the index of the `(` that opens it.
fn match_call(chars: &[char], i: usize) -> Option<(&'static str, usize)> {
    if i > 0 && is_ident_char(chars[i - 1]) {
        return None;
    }
    for name in CALLS {
        let n: Vec<char> = name.chars().collect();
        if i + n.len() > chars.len() {
            continue;
        }
        if !chars[i..i + n.len()]
            .iter()
            .zip(&n)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            continue;
        }
        let after_name = i + n.len();
        if chars.get(after_name).is_some_and(|c| is_ident_char(*c)) {
            continue;
        }
        let mut p = after_name;
        while chars.get(p).is_some_and(|c| c.is_whitespace()) {
            p += 1;
        }
        if chars.get(p) == Some(&'(') {
            return Some((*name, p));
        }
    }
    None
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn matching_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in chars.iter().enumerate().skip(open) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Arguments of one call. A thousands comma (`1,000`) stays inside an argument;
/// `sum(1, 000)` does not, because of the space.
fn split_args(inside: &str) -> Vec<String> {
    let chars: Vec<char> = inside.chars().collect();
    let mut args = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    for (i, c) in chars.iter().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 && !is_thousands_comma(&chars, i) => {
                args.push(
                    chars[start..i]
                        .iter()
                        .collect::<String>()
                        .trim()
                        .to_string(),
                );
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = chars[start..].iter().collect::<String>().trim().to_string();
    if args.is_empty() && last.is_empty() {
        return Vec::new();
    }
    args.push(last);
    args
}

/// `1,000` and `1,000,000`: the comma has exactly three digits after it, and one
/// to three before it (three when another comma sits just before those digits).
fn is_thousands_comma(chars: &[char], i: usize) -> bool {
    if chars.get(i + 1).is_some_and(|c| c.is_whitespace()) {
        return false;
    }
    let mut right = 0;
    let mut r = i + 1;
    while chars.get(r).is_some_and(|c| c.is_ascii_digit()) {
        right += 1;
        r += 1;
        if right > 3 {
            return false;
        }
    }
    if right != 3 {
        return false;
    }
    let mut left = 0;
    let mut l = i;
    while l > 0 && chars[l - 1].is_ascii_digit() {
        left += 1;
        l -= 1;
        if left > 3 {
            return false;
        }
    }
    if left == 0 || left > 3 {
        return false;
    }
    if l > 0 && chars[l - 1] == ',' && left != 3 {
        return false;
    }
    true
}

fn render_call(name: &str, args: &[String]) -> Option<String> {
    let wrapped: Vec<String> = args.iter().map(|a| format!("({a})")).collect();
    match name.to_ascii_lowercase().as_str() {
        "sum" | "total" => {
            if wrapped.is_empty() {
                Some("0".to_string())
            } else {
                Some(format!("({})", wrapped.join("+")))
            }
        }
        "avg" | "average" | "mean" => {
            let n = wrapped.len();
            (n > 0).then(|| format!("(({})/{n})", wrapped.join("+")))
        }
        "max" => (!wrapped.is_empty()).then(|| fold_extreme(true, &wrapped)),
        "min" => (!wrapped.is_empty()).then(|| fold_extreme(false, &wrapped)),
        _ => None,
    }
}

/// `max(a, b) = (a + b + abs(a - b)) / 2`, folded. fend has `abs` and no `min`/`max`,
/// and `abs` keeps a unit, so `max($1, $5)` stays in dollars.
fn fold_extreme(max: bool, args: &[String]) -> String {
    let mut acc = args[0].clone();
    let sign = if max { "+" } else { "-" };
    for arg in &args[1..] {
        acc = format!("(({acc}+{arg}{sign}abs({acc}-{arg}))/2)");
    }
    acc
}

fn unit_position(code: &str, start: usize) -> bool {
    let before = code[..start].trim_end();
    if before.ends_with(|ch: char| ch.is_ascii_digit() || ch == ')') {
        return true;
    }
    let prev = before
        .rsplit(|ch: char| !ch.is_alphabetic())
        .next()
        .unwrap_or("");
    matches!(
        prev.to_ascii_lowercase().as_str(),
        "in" | "to" | "as" | "per"
    )
}

fn call_position(code: &str, end: usize) -> bool {
    code[end..].trim_start().starts_with('(')
}

/// The word is an operand: `nope + 1`, `2 * nope`, `(nope)`.
fn expr_position(code: &str, start: usize, end: usize) -> bool {
    let before = code[..start].trim_end();
    let after = code[end..].trim_start();
    before.ends_with(|ch: char| "+-*/^=(,".contains(ch))
        || after.starts_with(|ch: char| "+-*/^=),".contains(ch))
}

fn case_suggestion(word: &str, above: &HashSet<String>, defs: &HashSet<String>) -> Option<String> {
    let pick = |set: &HashSet<String>| {
        let mut hits: Vec<&String> = set
            .iter()
            .filter(|n| n.eq_ignore_ascii_case(word) && n.as_str() != word)
            .collect();
        hits.sort();
        (hits.len() == 1).then(|| hits[0].clone())
    };
    pick(above).or_else(|| pick(defs))
}

fn mentions_math(code: &str) -> bool {
    let t = code.trim();
    if t.contains(['=', '+', '-', '*', '/', '^', '%', '(']) {
        return true;
    }
    let starts_name = t
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    !t.is_empty()
        && !t.contains(char::is_whitespace)
        && starts_name
        && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_name_keyword(w: &str) -> bool {
    matches!(
        w.to_ascii_lowercase().as_str(),
        "in" | "to"
            | "as"
            | "of"
            | "per"
            | "mod"
            | "on"
            | "off"
            | "is"
            | "and"
            | "or"
            | "the"
            | "a"
            | "an"
            | "at"
            | "for"
            | "with"
            | "by"
            | "from"
            | "plus"
            | "minus"
            | "times"
            | "prev"
            | "sum"
            | "total"
            | "avg"
            | "average"
            | "mean"
            | "min"
            | "max"
            | "today"
            | "tomorrow"
            | "yesterday"
            | "now"
            | "sqrt"
            | "ln"
            | "log"
            | "abs"
            | "round"
            | "floor"
            | "ceil"
            | "sin"
            | "cos"
            | "tan"
            | "pi"
            | "e"
            | "tau"
            | "phi"
            | "hex"
            | "hexadecimal"
            | "binary"
            | "bin"
            | "octal"
            | "not"
    )
}

/// Turn a notepad-style line into something fend understands.
pub fn rewrite(line: &str, prev: Option<&str>, vars: &HashSet<String>) -> String {
    // Inline comments and quoted text are ignored.
    let mut s = match line.find("//") {
        Some(idx) => line[..idx].to_string(),
        None => line.to_string(),
    };
    s = QUOTED.replace_all(&s, " ").into_owned();
    // Before commas become decimal points: inside sum()/avg()/min()/max() a comma
    // separates arguments, except a thousands group ("sum(1,000, 2,000)").
    s = expand_calls(&s);
    // Twice: "1,5 + 2,5" shares the space between two matches.
    for _ in 0..2 {
        s = DOT_THOUSANDS
            .replace_all(&s, |c: &regex::Captures| {
                format!("{}{}.{}{}", &c[1], c[2].replace('.', ""), &c[3], &c[4])
            })
            .into_owned();
        s = DOT_MILLIONS
            .replace_all(&s, |c: &regex::Captures| {
                format!("{}{}{}", &c[1], c[2].replace('.', ""), &c[3])
            })
            .into_owned();
        s = DECIMAL_COMMA
            .replace_all(&s, "${1}${2}.${3}${4}")
            .into_owned();
    }
    s = SYMBOL_MINUS.replace_all(&s, "$1-$2$3").into_owned();
    // After "1,5" has become "1.5", and while "US$5m" still has its "$".
    s = expand_money_scales(&s);
    s = PREFIXED_DOLLAR
        .replace_all(&s, |c: &regex::Captures| {
            let code = match &c[1] {
                "US" => "USD",
                "CA" | "C" => "CAD",
                "AU" | "A" => "AUD",
                "NZ" => "NZD",
                "HK" => "HKD",
                _ => "SGD",
            };
            format!("{} {code}", &c[2])
        })
        .into_owned();
    s = VERSION_WORD.replace_all(&s, " ").into_owned();
    s = PERCENT_WORD.replace_all(&s, "$1%").into_owned();
    s = PCT_GLUED.replace_all(&s, "% $1").into_owned();
    s = INCHES.replace_all(&s, "$1 inches$2").into_owned();
    s = LAKH_CRORE
        .replace_all(&s, |c: &regex::Captures| {
            let zeros = if c[2].to_ascii_lowercase().starts_with('c') {
                7
            } else {
                5
            };
            shift_point(&c[1].replace(',', ""), zeros)
        })
        .into_owned();
    s = HALF.replace_all(&s, "0.5 * ").into_owned();
    s = THIRD.replace_all(&s, "(1/3) * ").into_owned();
    s = QUARTER.replace_all(&s, "0.25 * ").into_owned();
    s = TWICE.replace_all(&s, "2 * ").into_owned();
    s = TRIPLE.replace_all(&s, "3 * ").into_owned();

    // "Groceries: $40" -> "$40"
    if let Some(c) = LABEL.captures(&s) {
        s = c[2].to_string();
    }

    if let Some(c) = SPLIT.captures(&s) {
        let amount = c.get(1).or(c.get(2)).map_or("", |m| m.as_str());
        let ways = c.get(3).or(c.get(4)).map_or("", |m| m.as_str());
        s = format!("({amount}) / {ways}");
    }
    if let Some(c) = TRANSFER.captures(&s) {
        s = format!("({}) / ({})", &c[1], &c[2]);
    }

    // While "k" and "M" are still on the number, so "$180k per month" can be kept
    // on the right of a division. Expanding them first would hide which number
    // was a rate.
    s = group_rate_divisors(&s);
    s = expand_scales(&s);

    // Currency symbols fend doesn't know: "€20" -> "20 EUR", "in €" -> "in EUR".
    s = SYMBOL_NUM
        .replace_all(&s, |c: &regex::Captures| {
            let sym = c[1].chars().next().unwrap();
            format!("{} {}", &c[2], symbol_code(sym))
        })
        .into_owned();
    for (sym, code) in SYMBOL_CODES {
        s = s.replace(*sym, code);
    }

    // fend misparses `3 * $95`; `3 * ($95)` is fine.
    s = MONEY_PREFIX.replace_all(&s, "($0)").into_owned();
    // The number belongs to the label, or fend would multiply it in.
    if let Some(c) = LABEL_NUMBER.captures(&s) {
        s = c[1].to_string();
    }

    if let Some(p) = prev {
        s = PREV.replace_all(&s, p).into_owned();
    }

    // "5 x 3" is a product; "0x10" is hex.
    // Again until nothing changes: "2 x 3 x 4" shares the 3 between two matches.
    loop {
        let next = TIMES_X
            .replace_all(&s, |c: &regex::Captures| {
                if &c[1] == "0" && c[2].is_empty() && &c[3] == "x" {
                    c[0].to_string()
                } else {
                    format!("{} * {}", &c[1], &c[5])
                }
            })
            .into_owned();
        if next == s {
            break;
        }
        s = next;
    }
    s = founder_line(&s, vars);
    s = A_DAY.replace_all(&s, "$1 / $2").into_owned();
    s = AT_PRICE.replace_all(&s, "$1 * $2").into_owned();
    s = drop_prose_words(&s, vars);
    s = IMPERIAL_UNIT
        .replace_all(&s, |c: &regex::Captures| format!("({})", imperial(&c[1]).1))
        .into_owned();
    for (re, unit) in UNIT_ALIAS_RES.iter() {
        s = re.replace_all(&s, *unit).into_owned();
    }
    s = DIVISOR
        .replace_all(&s, |c: &regex::Captures| {
            let unit = c[3].split('/').next().unwrap_or("").to_ascii_lowercase();
            if KEYWORDS.contains(&unit.as_str()) {
                c[0].to_string()
            } else {
                format!("{}/ ({}{})", &c[1], &c[2], &c[3])
            }
        })
        .into_owned();
    s = rescale_rates(&s);

    // Keep the variable name out of the percentage rewrites.
    let (lhs, mut rhs) = match ASSIGN.captures(&s) {
        Some(c) => (Some(c[1].to_string()), c[2].to_string()),
        None => (None, s.clone()),
    };

    for (re, op) in WORD_OP_RES.iter() {
        rhs = re.replace_all(&rhs, *op).into_owned();
    }

    rhs = PLUS_MINUS_PCT_OF
        .replace_all(&rhs, |c: &regex::Captures| {
            format!("{}(({} / 100) * {})", &c[1], c[2].replace(',', ""), &c[3])
        })
        .into_owned();
    rhs = PCT_SHARE.replace_all(&rhs, "($1 / 100)$2").into_owned();
    rhs = if let Some(c) = IS_WHAT_PCT.captures(&rhs) {
        format!("(({}) / ({})) to %", &c[1], &c[2])
    } else if let Some(c) = IS_PCT_OF_WHAT.captures(&rhs) {
        format!("({}) / (({}) / 100)", &c[1], &c[2])
    } else if let Some(c) = PCT_OF_WHAT.captures(&rhs) {
        format!("({}) / (({}) / 100)", &c[2], &c[1])
    } else if let Some(c) = PCT_ON.captures(&rhs) {
        format!("({}) * (1 + ({}) / 100)", &c[2], &c[1])
    } else if PCT_OFF.is_match(&rhs) {
        discounts(&rhs)
    } else if let Some(c) = AS_PCT.captures(&rhs) {
        format!("(({}) / ({})) to %", &c[1], &c[2])
    } else if PLUS_MINUS_PCT_STEP.is_match(&rhs) {
        // One percentage at a time, each applied to the total so far.
        let mut out = rhs;
        while let Some(c) = PLUS_MINUS_PCT_STEP.captures(&out) {
            // "100 + 10% / 2": the percentage term first (5%), as in ordinary
            // precedence, then applied to the amount.
            let term = c[3].replace(',', "").replace('%', " / 100");
            let step = format!("(({}) * (1 {} ({term})))", &c[1], &c[2]);
            out = format!("{step}{}", &out[c.get(0).unwrap().end()..]);
        }
        out
    } else {
        rhs
    };

    // A number times a percentage is that share of it too ("200 * 10%").
    rhs = TIMES_PCT.replace_all(&rhs, "$1($2 / 100)").into_owned();

    match lhs {
        Some(name) => format!("{name} = {rhs}"),
        None => rhs,
    }
}

/// "10% off 20% off $100": each discount applied in turn ($72).
fn discounts(s: &str) -> String {
    match PCT_OFF.captures(s) {
        Some(c) => format!("({}) * (1 - ({}) / 100)", discounts(&c[2]), &c[1]),
        None => s.to_string(),
    }
}

/// Words that would be a conversion or operator, not a unit, after a number.
const KEYWORDS: &[&str] = &[
    "in", "to", "as", "of", "per", "mod", "on", "off", "is", "and",
];

/// Little words fend would read as units (I is the imaginary unit, a is an are, at
/// is an atmosphere...). They're prose unless they directly follow a number, or
/// are a variable: one defined above, or being defined on this line.
fn drop_prose_words(s: &str, vars: &HashSet<String>) -> String {
    const PROSE: &[&str] = &[
        "I", "a", "A", "an", "An", "the", "The", "at", "At", "my", "My",
    ];
    // Words that join a value to the rest of the line, like an operator.
    const JOINS: &[&str] = &[
        "times",
        "plus",
        "minus",
        "multiplied",
        "divided",
        "by",
        "mod",
        "in",
        "to",
        "as",
        "of",
        "per",
        "x",
    ];
    let joins = |t: &str| {
        t.starts_with(['+', '-', '*', '/', '^', '=', '(', ')', '%', '×', '!'])
            || t.ends_with(['+', '-', '*', '/', '^', '=', '(', '×'])
            || JOINS.contains(&t.to_ascii_lowercase().as_str())
    };
    let assigned = ASSIGN.captures(s).map(|c| c[1].to_string());
    let toks: Vec<&str> = s.split(' ').collect();
    let mut out: Vec<&str> = Vec::new();
    for (i, &tok) in toks.iter().enumerate() {
        let prose = PROSE.contains(&tok);
        let var = vars.contains(tok) || assigned.as_deref() == Some(tok);
        // A short word that's also a variable is the variable only when it's on
        // its own or next to an operator: after "I = 3", "I * 2" is 6 but
        // "I spent $40" is still $40 (from the third review).
        let before = out.iter().rev().find(|t| !t.is_empty()).copied();
        let after = toks[i + 1..].iter().find(|t| !t.is_empty()).copied();
        let used = !prose
            || assigned.as_deref() == Some(tok)
            || (before.is_none() && after.is_none())
            || before.is_some_and(joins)
            || after.is_some_and(joins);
        if var && used {
            out.push(tok);
            continue;
        }
        let after_number =
            before.is_some_and(|t| t.ends_with(|c: char| c.is_ascii_digit() || c == ')'));
        if prose && !after_number {
            continue;
        }
        out.push(tok);
    }
    out.join(" ")
}

fn symbol_code(sym: char) -> &'static str {
    SYMBOL_CODES
        .iter()
        .find(|(c, _)| *c == sym)
        .map_or("", |(_, code)| code)
}

/// Evaluate `expr` and bind the result to the hidden line variable `var`.
fn eval_into(
    ctx: &mut fend_core::Context,
    budget: &Budget,
    var: &str,
    expr: &str,
) -> Option<String> {
    let full = match ASSIGN.captures(expr) {
        // `x = 5` -> `x = (5); _l3 = x`
        Some(c) => format!("{} = ({}); {var} = {}", &c[1], &c[2], &c[1]),
        None => format!("{var} = ({expr})"),
    };
    let r = budget.evaluate(&full, ctx).ok()?;
    let out = r.get_main_result().trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// Like `eval_into`, but drops stray words fend doesn't know
/// ("Lunch with Bob $40" -> "$40"), so lines can read like notes.
fn eval_forgiving(
    ctx: &mut fend_core::Context,
    budget: &Budget,
    var: &str,
    expr: &str,
) -> Option<String> {
    let meaningful = expr
        .split(|c: char| !c.is_alphabetic())
        .any(|w| MEANINGFUL.contains(&w.to_ascii_lowercase().as_str()));
    if meaningful {
        return None;
    }
    drop_unknown_words(ctx, budget, var, expr).or_else(|| {
        // Prose fend can't parse at all ("Food & drinks ₹2,000 * 4"): start the
        // expression at the first number, currency or bracket instead.
        if ASSIGN.is_match(expr) {
            return None;
        }
        let tokens: Vec<&str> = expr.split_whitespace().collect();
        let start = tokens
            .iter()
            .position(|t| t.starts_with(|c: char| c.is_ascii_digit() || "$£€¥₹(-.".contains(c)))?;
        // Only skip words. Skipping an operator or a variable ("rent + 5" -> "5")
        // would answer a different sum.
        if start == 0
            || tokens[start] == "-"
            || tokens[..start]
                .iter()
                .any(|t| t.contains(['+', '-', '*', '/', '^', '=', '%']) || t.starts_with("_l"))
        {
            return None;
        }
        drop_unknown_words(ctx, budget, var, &tokens[start..].join(" "))
    })
}

/// Words that change what a line means; dropping them would give a wrong answer.
const MEANINGFUL: &[&str] = &[
    "until", "since", "ago", "before", "after", "every", "except", "without", "not",
];

fn drop_unknown_words(
    ctx: &mut fend_core::Context,
    budget: &Budget,
    var: &str,
    expr: &str,
) -> Option<String> {
    let mut expr = expr.to_string();
    let mut dropped = false;
    for _ in 0..12 {
        // Prose dropped from between two numbers leaves them side by side, and fend
        // would multiply them ("Room 12 costs $50" -> 12 * $50). The amount is the
        // second one; the first was part of the label.
        if dropped
            && !ASSIGN.is_match(&expr)
            && let Some(m) = SIDE_BY_SIDE.captures(&expr).and_then(|c| c.get(1))
        {
            expr = expr[m.start()..].to_string();
            continue;
        }
        let full = match ASSIGN.captures(&expr) {
            Some(c) => format!("{} = ({}); {var} = {}", &c[1], &c[2], &c[1]),
            None => format!("{var} = ({expr})"),
        };
        match budget.evaluate(&full, ctx) {
            Ok(r) => {
                let out = r.get_main_result().trim().to_string();
                return (!out.is_empty()).then_some(out);
            }
            Err(e) => {
                if std::env::var_os("OMACUS_DEBUG").is_some() {
                    eprintln!("{full:?} -> {e}");
                }
                let word = UNKNOWN_IDENT.captures(&e)?.get(1)?.as_str().to_string();
                if MEANINGFUL.contains(&word.to_ascii_lowercase().as_str()) {
                    return None;
                }
                let at = find_word(&expr, &word)?;
                // "$5xyz": the letters are a suffix of the amount. Dropping them
                // would leave the bare number, which is a different amount.
                if stuck_to_money(&expr[..at]) {
                    return None;
                }
                if qualifies_unit(&expr[at + word.len()..]) {
                    return None;
                }
                // A word in front of an operator is what it works on: dropping
                // "Income" from "Income - total" would leave minus the total.
                let before = expr[..at].trim_end();
                let after = expr[at + word.len()..].trim_start();
                if (before.is_empty() || before.ends_with(|c: char| "+-*/^×÷(=".contains(c)))
                    && after.starts_with(|c: char| "+-*/^×÷".contains(c))
                {
                    return None;
                }
                let next = format!("{} {}", &expr[..at], &expr[at + word.len()..]);
                if !next.chars().any(|c| c.is_ascii_alphanumeric()) {
                    return None;
                }
                expr = next;
                dropped = true;
            }
        }
    }
    None
}

/// Letters with no space between them and a currency amount: "$5m", "($5)xyz".
/// A separate word ("3 kg", "Nails $8") or a rate ("$85/hour") is not a suffix.
fn stuck_to_money(before: &str) -> bool {
    if !before.ends_with(|c: char| c.is_ascii_digit() || c == ')' || "$£€¥₹₩₽₺".contains(c))
    {
        return false;
    }
    match before.rfind(['$', '£', '€', '¥', '₹', '₩', '₽', '₺']) {
        Some(i) => !before[i..].chars().any(char::is_whitespace),
        None => false,
    }
}

/// An approximate answer with every digit Omacus might show. fend rounds to ten
/// decimal places, which loses tiny values ("approx. 0.0000000003"), and in fend
/// 1.5.8 a rounding carry can land in the wrong digit (2/3/10^10 prints
/// "approx. 1", 12.3 + that prints "approx. 12.4"). So the value is read back from
/// the line's variable scaled by 10^20 (or 10^60 for the very smallest) and the
/// decimal point moved back; Omacus does its own rounding for display.
fn precisely(ctx: &mut fend_core::Context, budget: &Budget, var: &str, raw: String) -> String {
    let Some(body) = raw.strip_prefix("approx. ") else {
        return raw;
    };
    // Only a plain number, perhaps with a unit: not a complex number ("1 + 2i"),
    // a list, or anything else fend writes in several parts.
    let unit_of = |b: &str| {
        b.split_once(' ')
            .map_or(String::new(), |(_, u)| u.to_string())
    };
    let raw_unit = unit_of(body);
    if raw_unit.split(' ').any(|t| {
        matches!(t, "+" | "-" | "i")
            || t.ends_with('i') && t.len() > 1 && t[..t.len() - 1].parse::<f64>().is_ok()
    }) || body.split(' ').next().is_some_and(|n| n.ends_with('i'))
    {
        return raw;
    }
    for shift in [20usize, 60] {
        let Ok(scaled) = budget.evaluate(&format!("({var}) * 1e{shift}"), ctx) else {
            return raw;
        };
        let scaled = scaled.get_main_result().trim().to_string();
        let body = scaled.trim_start_matches("approx. ");
        let (number, unit) = body.split_once(' ').unwrap_or((body, ""));
        if unit != raw_unit {
            return raw;
        }
        let (sign, digits) = match number.strip_prefix('-') {
            Some(d) => ("-", d),
            None => ("", number),
        };
        let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
        if int.is_empty() || !int.chars().chain(frac.chars()).all(|c| c.is_ascii_digit()) {
            return raw;
        }
        let int = int.trim_start_matches('0');
        if int.is_empty() && shift == 20 {
            continue; // smaller than 10^-20: look further
        }
        let shifted = if int.len() > shift {
            format!(
                "{}.{}{frac}",
                &int[..int.len() - shift],
                &int[int.len() - shift..]
            )
        } else {
            format!("0.{}{int}{frac}", "0".repeat(shift - int.len()))
        };
        let unit = if unit.is_empty() {
            String::new()
        } else {
            format!(" {unit}")
        };
        return format!("{sign}{shifted}{unit}");
    }
    raw
}

/// Where `word` appears as a whole word in `s`.
fn find_word(s: &str, word: &str) -> Option<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    s.match_indices(word).map(|(i, _)| i).find(|&i| {
        let before = s[..i].chars().next_back().is_none_or(|c| !is_word(c));
        let after = s[i + word.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_word(c));
        before && after
    })
}

/// Whether `rest` (what follows an unknown word) starts with a unit, like "gallon"
/// after "imperial". Dropping the word would quietly change the answer, so the
/// line gets no result instead.
fn qualifies_unit(rest: &str) -> bool {
    let Some(next) = rest
        .split_whitespace()
        .next()
        .filter(|w| w.chars().all(|c| c.is_ascii_alphabetic()))
    else {
        return false;
    };
    let next = next.to_ascii_lowercase();
    if KEYWORDS.contains(&next.as_str()) || matches!(next.as_str(), "with" | "for" | "at") {
        return false;
    }
    thread_local! {
        static KNOWN: std::cell::RefCell<std::collections::HashMap<String, bool>> = Default::default();
    }
    KNOWN.with(|known| {
        *known.borrow_mut().entry(next.clone()).or_insert_with(|| {
            let mut ctx = fend_core::Context::new();
            fend_core::evaluate(&format!("1 {next}"), &mut ctx)
                .is_ok_and(|r| !r.get_main_result().trim().is_empty())
        })
    })
}

/// Round long decimals and add thousands separators for display.
pub fn pretty(raw: &str) -> String {
    let s = raw.trim_start_matches("approx. ");
    if looks_like_date(s) {
        return s.to_string();
    }
    let is_money = s.starts_with(['$', '£', '€', '¥', '₹'])
        || s.split_whitespace().any(|t| {
            t.ends_with('$') || (t.len() == 3 && t.chars().all(|c| c.is_ascii_uppercase()))
        });

    // "44.86 USD" reads better as "$44.86".
    let dollars;
    let s = match s.strip_suffix(" USD") {
        Some(n) if NUM_TOKEN.is_match(n) && !n.starts_with('$') => {
            dollars = match n.strip_prefix('-') {
                Some(n) => format!("-${n}"),
                None => format!("${n}"),
            };
            dollars.as_str()
        }
        _ => s,
    };

    s.split(' ')
        .map(|tok| {
            // fend writes derived money units as "85$ / hour".
            let moved;
            let tok = match tok.strip_suffix('$') {
                Some(n) if NUM_TOKEN.is_match(n) => {
                    moved = format!("${n}");
                    moved.as_str()
                }
                _ => tok,
            };
            let tok = match tok {
                "celsius" => "°C",
                "fahrenheit" => "°F",
                _ => tok,
            };
            // "25%", and the imaginary part of a complex answer ("1.3643763538i").
            let (body, pct) = match (tok.strip_suffix('%'), tok.strip_suffix('i')) {
                (Some(b), _) => (b, "%"),
                (_, Some(b)) if NUM_TOKEN.is_match(b) => (b, "i"),
                _ => (tok, ""),
            };
            match NUM_TOKEN.captures(body) {
                Some(c) => {
                    let n = format_number(&c[4], c.get(5).map(|m| m.as_str()), is_money);
                    let sign = if c[1].is_empty() { &c[3] } else { &c[1] };
                    format!("{sign}{}{n}{pct}", &c[2])
                }
                None => tok.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .replacen("$-", "-$", 1)
        .replacen("£-", "-£", 1)
}

/// A download time reads better as "5 min 20 s" than "320 seconds".
fn transfer_time(line: &str, raw: &str) -> Option<String> {
    let line = LABEL
        .captures(line)
        .map_or(line.to_string(), |c| c[2].to_string());
    if !TRANSFER.is_match(&line) {
        return None;
    }
    let raw = raw.trim_start_matches("approx. ");
    let secs: f64 = [" seconds", " second", " s"]
        .iter()
        .find_map(|unit| raw.strip_suffix(unit))?
        .replace(',', "")
        .parse()
        .ok()?;
    Some(human_duration(secs))
}

/// "0.5 s", "32 s", "5 min 20 s", "2 h 13 min", "3 days 4 h".
fn human_duration(secs: f64) -> String {
    if secs < 10.0 {
        let s = format!("{secs:.1}");
        return format!("{} s", s.trim_end_matches('0').trim_end_matches('.'));
    }
    let total = secs.round() as u64;
    let parts = [
        (total / 86_400, "day"),
        (total % 86_400 / 3_600, "h"),
        (total % 3_600 / 60, "min"),
        (total % 60, "s"),
    ];
    let shown: Vec<String> = parts
        .iter()
        .skip_while(|(n, _)| *n == 0)
        .take(2)
        .filter(|(n, _)| *n > 0)
        .map(|(n, unit)| match *unit {
            "day" if *n != 1 => format!("{n} days"),
            _ => format!("{n} {unit}"),
        })
        .collect();
    shown.join(" ")
}

fn looks_like_date(s: &str) -> bool {
    const NAMES: &[&str] = &[
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    NAMES.iter().any(|n| s.contains(n))
}

fn format_number(int: &str, frac: Option<&str>, money: bool) -> String {
    // Rounded on the decimal digits themselves, half away from zero: an f64 would
    // turn 2.675 into 2.67499… and show 2.67.
    let int = int.trim_start_matches('0');
    let frac = frac.unwrap_or("");
    let leading = frac.chars().take_while(|&c| c == '0').count();
    if int.is_empty() && leading == frac.len() {
        return "0".to_string();
    }
    if int.is_empty() && leading >= 6 {
        // Tiny: 1.6e-19, not 0. Three significant digits.
        let (mut digits, carried) = round_digits(&frac[leading..], 3);
        digits.truncate(3);
        let exp = -(leading as i64) - 1 + carried as i64;
        let (first, rest) = digits.split_at(1);
        let rest = rest.trim_end_matches('0');
        return match rest {
            "" => format!("{first}e{exp}"),
            _ => format!("{first}.{rest}e{exp}"),
        };
    }
    let decimals = if !int.is_empty() || (money && leading < 2) {
        2
    } else {
        // Two significant digits for small numbers: 0.000123 -> 0.00012
        leading + 2
    };
    let (digits, _) = round_digits(&format!("{int}{frac}"), int.len() + decimals);
    let (i, f) = digits.split_at(digits.len() - decimals);
    let i = match i.trim_start_matches('0') {
        "" => "0",
        i => i,
    };
    let f = if money && decimals == 2 && !f.trim_end_matches('0').is_empty() {
        f // $41.10, not $41.1
    } else {
        f.trim_end_matches('0')
    };
    let grouped = group_thousands(i);
    if f.is_empty() {
        grouped
    } else {
        format!("{grouped}.{f}")
    }
}

/// The first `keep` of `digits` (padded with zeros), rounded half up on the next
/// one, and whether rounding carried into a new leading digit ("9995", 3 -> "1000").
fn round_digits(digits: &str, keep: usize) -> (String, bool) {
    let mut out: Vec<u8> = digits.bytes().take(keep).collect();
    out.resize(keep, b'0');
    if digits.as_bytes().get(keep).is_some_and(|&d| d >= b'5') {
        for d in out.iter_mut().rev() {
            if *d == b'9' {
                *d = b'0';
            } else {
                *d += 1;
                return (String::from_utf8(out).unwrap_or_default(), false);
            }
        }
        out.insert(0, b'1');
        return (String::from_utf8(out).unwrap_or_default(), true);
    }
    (String::from_utf8(out).unwrap_or_default(), false)
}

/// `num` times 10^`places`, worked out on the digits ("1.5", 5 -> "150000").
fn shift_point(num: &str, places: usize) -> String {
    let (int, frac) = num.split_once('.').unwrap_or((num, ""));
    let mut frac = frac.to_string();
    if frac.len() < places {
        frac.push_str(&"0".repeat(places - frac.len()));
    }
    let (more, rest) = frac.split_at(places);
    let int = format!("{int}{more}");
    let int = match int.trim_start_matches('0') {
        "" => "0",
        i => i,
    };
    match rest {
        "" => int.to_string(),
        _ => format!("{int}.{rest}"),
    }
}

fn group_thousands(int: &str) -> String {
    let bytes = int.as_bytes();
    let mut out = String::with_capacity(int.len() + int.len() / 3);
    for (idx, b) in bytes.iter().enumerate() {
        if idx > 0 && (bytes.len() - idx).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_results(doc: &str) -> Vec<LineResult> {
        Engine::new(Rates::fixed(&[
            ("USD", 1.0),
            ("EUR", 0.9),
            ("INR", 83.0),
            ("GBP", 0.78),
            ("BOB", 6.9),
            ("CHF", 0.9),
        ]))
        .eval_document(doc)
    }

    fn eval(doc: &str) -> Vec<String> {
        eval_results(doc).into_iter().map(|r| r.display).collect()
    }

    fn one(line: &str) -> String {
        eval(line).pop().unwrap()
    }

    /// A missing name is a hint. Prose and a bare currency code stay blank, and a
    /// hard error (divide by zero) is not delayed as a hint.
    #[test]
    fn name_hints_mark_unknown_names_only() {
        let rs = eval_results("a = 5\nA\ny = z + 1\nz = 4\nnope\nhello world\nUSD\n10 / 0");
        assert!(rs[1].hint);
        assert_eq!(rs[1].note.as_deref(), Some("Did you mean a?"));
        assert!(rs[2].hint);
        assert_eq!(rs[2].note.as_deref(), Some("z is defined below"));
        assert_eq!(rs[3].display, "4");
        assert!(rs[4].hint);
        assert_eq!(rs[4].note.as_deref(), Some("Unknown name 'nope'"));
        assert!(!rs[5].hint);
        assert_eq!(rs[5].note, None);
        assert!(!rs[6].hint);
        assert_eq!(rs[6].note, None);
        assert!(!rs[7].hint);
        assert_eq!(rs[7].note.as_deref(), Some("Can't divide by zero"));
    }

    /// "%" is mod only between two numbers; a sheet's prose with percentages in it
    /// said "mod needs whole numbers, 0 or more".
    #[test]
    fn percentages_in_prose_are_not_mod() {
        let prose = "Alex $90k, 0.5%, 1% share of sales + a bonus";
        let r = eval_results(prose).pop().unwrap();
        assert_eq!((r.display.as_str(), r.note.as_deref()), ("", None));
        let r = eval_results("7.5 % 2").pop().unwrap();
        assert_eq!(
            r.note.as_deref(),
            Some("mod needs whole numbers, 0 or more")
        );
    }

    #[test]
    fn realigns_answers_to_edited_text() {
        let old = "a = 1\nb = 2\nc = 3";
        let results = eval_results(old);
        let shown = |new: &str| -> Vec<String> {
            realign(old, &results, new)
                .into_iter()
                .map(|r| r.display)
                .collect()
        };
        // A line inserted: the answers below it move down with their lines.
        assert_eq!(shown("a = 1\n\nb = 2\nc = 3"), ["1", "", "2", "3"]);
        assert_eq!(shown("new\na = 1\nb = 2\nc = 3"), ["", "1", "2", "3"]);
        // Removed, changed, appended.
        assert_eq!(shown("a = 1\nc = 3"), ["1", "3"]);
        assert_eq!(shown("a = 1\nb = 20\nc = 3"), ["1", "", "3"]);
        assert_eq!(shown("a = 1\nb = 2\nc = 3\nd"), ["1", "2", "3", ""]);
        assert_eq!(shown(""), [""]);
        assert_eq!(shown(old), ["1", "2", "3"]);
        // Repeated lines: the top match wins, never an answer from the wrong place.
        let old = "x\nx\nx";
        let results = eval_results("1\n2\n3");
        let got: Vec<String> = realign(old, &results, "x\nx")
            .into_iter()
            .map(|r| r.display)
            .collect();
        assert_eq!(got, ["1", "2"]);
    }

    #[test]
    fn answers_part_way_through_fill_in_over_the_old_ones() {
        let shown = eval_results("1\n2\n3");
        let text = "a = 5\nb = 6\nc = 7";
        let pairs = |rs: Vec<LineResult>| -> Vec<(String, bool)> {
            rs.into_iter().map(|r| (r.display, r.stale)).collect()
        };
        // The first line's new answer has arrived; the others keep theirs, dimmed.
        let part = &eval_results(text)[..1];
        assert_eq!(
            pairs(realign_onto(text, part, text, &shown)),
            [("5".into(), false), ("2".into(), true), ("3".into(), true)]
        );
        // All of them: nothing old is left.
        assert_eq!(
            pairs(realign_onto(text, &eval_results(text), text, &shown)),
            [
                ("5".into(), false),
                ("6".into(), false),
                ("7".into(), false)
            ]
        );
    }

    #[test]
    fn answers_below_an_edit_are_stale() {
        let old = "a = 1\nb = a + 1\nc = b + 1";
        let results = eval_results(old);
        let stale = |new: &str| -> Vec<bool> {
            realign(old, &results, new)
                .into_iter()
                .map(|r| r.stale)
                .collect()
        };
        // Above the edit: still right. Below it: maybe not, until recalculated.
        assert_eq!(stale("a = 1\nb = a + 2\nc = b + 1"), [false, false, true]);
        assert_eq!(stale("a = 5\nb = a + 1\nc = b + 1"), [false, true, true]);
        assert_eq!(
            stale("a = 1\nb = a + 1\nc = b + 1\nd"),
            [false, false, false, false]
        );
        assert_eq!(stale(old), [false, false, false]);
        // Stale stays stale through a second edit before the answers arrive.
        let moved = realign(old, &results, "a = 5\nb = a + 1\nc = b + 1");
        let again = realign(
            "a = 5\nb = a + 1\nc = b + 1",
            &moved,
            "a = 5\nb = a + 1\nc = b + 1\n",
        );
        assert_eq!(
            again.iter().map(|r| r.stale).collect::<Vec<_>>(),
            [false, true, true, false]
        );
    }

    #[test]
    fn arithmetic_and_formatting() {
        assert_eq!(one("2 + 2"), "4");
        assert_eq!(one("1200 * 12"), "14,400");
        assert_eq!(one("1/3"), "0.33");
        assert_eq!(one("0.000123 * 1"), "0.00012");
        assert_eq!(one("5 x 3"), "15");
        assert_eq!(one("10 times 3"), "30");
    }

    #[test]
    fn percentages() {
        assert_eq!(one("20% of 150"), "30");
        assert_eq!(one("5% on 200"), "210");
        assert_eq!(one("6% off 40"), "37.6");
        assert_eq!(one("50 as a % of 200"), "25%");
        assert_eq!(one("5% of what is 6"), "120");
        assert_eq!(one("6 is what % of 120"), "5%");
        assert_eq!(one("40 is 20% of what"), "200");
        assert_eq!(one("100 - 20%"), "80");
        assert_eq!(one("$50 + 8% tax"), "$54");
        assert_eq!(one("$1,000 + 7.5%"), "$1,075");
    }

    #[test]
    fn units_and_currency() {
        assert_eq!(one("3 km in miles"), "1.86 miles");
        assert_eq!(one("$30 in EUR"), "27 EUR");
        assert_eq!(one("€9 in USD"), "$10");
        assert_eq!(one("rate = $85/hour\nrate * 6 hours * 4"), "$2,040");
        assert_eq!(one("$30 in EUR"), "27 EUR");
        assert_eq!(one("$30 + 10 EUR"), "$41.11");
        assert_eq!(one("2 hours + 30 mins in minutes"), "150 minutes");
        assert_eq!(one("$85/hour"), "$85 / hour");
        assert_eq!(one("72 fahrenheit in celsius"), "22.22 °C");
        assert_eq!(one("60 km / 2 hours"), "30 km / hour");
        assert_eq!(one("2 liters / 250 ml"), "8");
        assert_eq!(one("1,200 km / 15 km/liter"), "80 liters");
        assert_eq!(one("$1,450 / 4 weeks"), "$362.50 / week");
        assert_eq!(one("1 nautical mile in km"), "1.85 km");
        assert_eq!(one("8 fl oz in ml"), "236.59 ml");
        assert_eq!(one("120 sq ft in m^2"), "11.15 m^2");
    }

    #[test]
    fn qualified_units_are_not_guessed() {
        // A word in front of a unit changes it; dropping it would give a wrong answer.
        assert_eq!(one("1 royal gallon in liters"), "");
        assert_eq!(one("Movie tickets 2 * $14"), "$28");
    }

    #[test]
    fn copies_a_sheet_with_answers() {
        let engine = Engine::new(Rates::fixed(&[("USD", 1.0)]));
        let text = "# Lunch\nPizza $24\nTip: 18% of prev\n\n";
        let copied = with_answers(text, &engine.eval_document(text));
        assert_eq!(
            copied,
            "# Lunch\nPizza $24           $24\nTip: 18% of prev    $4.32"
        );
    }

    #[test]
    fn numbers_in_labels_are_not_multiplied() {
        assert_eq!(one("Day 3 $50"), "$50");
        assert_eq!(one("Room 12 costs $50"), "$50");
        assert_eq!(one("iPhone 15 $799"), "$799");
        assert_eq!(one("Item 2: $5"), "$5");
        assert_eq!(one("Movie tickets 2 * $14"), "$28");
    }

    #[test]
    fn currency_scale_suffix_is_part_of_the_amount() {
        // On its own line, and everywhere else a currency amount is written.
        assert_eq!(one("$5m"), "$5,000,000");
        assert_eq!(one("$5 m"), "$5,000,000");
        assert_eq!(one("$1m"), "$1,000,000");
        assert_eq!(one("$1M"), "$1,000,000");
        assert_eq!(one("$1mm"), "$1,000,000");
        assert_eq!(one("$1MM"), "$1,000,000");
        assert_eq!(one("$180k"), "$180,000");
        assert_eq!(one("$180K"), "$180,000");
        assert_eq!(one("$2.5bn"), "$2,500,000,000");
        assert_eq!(one("$2.5b"), "$2,500,000,000");
        assert_eq!(one("$2.5B"), "$2,500,000,000");
        assert_eq!(one("$2.5BN"), "$2,500,000,000");
        assert_eq!(one("$10m + $1m"), "$11,000,000");
        assert_eq!(one("20% of $5m"), "$1,000,000");
        assert_eq!(one("$5m per month"), "$5,000,000 / month");
        assert_eq!(one("$2.4M / $1m per month"), "2.4 months");
        assert_eq!(one("$2.4m / $180k per month"), "13.33 months");
        assert_eq!(one("a=$1m at $10m pre"), "$11,000,000 post, 9.09%");
        assert_eq!(one("Food $5m"), "$5,000,000");
        assert_eq!(one("Seed: $5m"), "$5,000,000");
        assert_eq!(one("$5m in EUR"), "4,500,000 EUR");
        assert_eq!(one("€5m"), "5,000,000 EUR");
        assert_eq!(one("£5m"), "£5,000,000");
        assert_eq!(one("₹5m"), "5,000,000 INR");
        // No currency symbol: the letter stays a unit.
        assert_eq!(one("1m"), "1 m");
        assert_eq!(one("3mm"), "3 mm");
        assert_eq!(one("1.5B"), "1.5 B");
        assert_eq!(one("5km"), "5 km");
        assert_eq!(one("3MB"), "3 MB");
        // A suffix that isn't a scale is not thrown away to leave the bare amount.
        assert_eq!(one("$5xyz"), "");
        assert_eq!(one("$5kg"), "");
        assert_eq!(one("$5km"), "");
        assert_eq!(one("€5xyz"), "");
        assert_ne!(one("$5m"), "$5");
        assert_ne!(one("$5kg"), "$5");
    }

    #[test]
    fn quantity_words_count() {
        assert_eq!(one("half of 50"), "25");
        assert_eq!(one("twice $40"), "$80");
        assert_eq!(one("a third of 90"), "30");
        assert_eq!(one("a quarter of $200"), "$50");
        assert_eq!(one("50 percent off 80"), "40");
        // Counted by the time module, not multiplied out by fend.
        assert!(one("days until Dec 25").ends_with(" days"));
    }

    #[test]
    fn splits_tips_and_downloads() {
        assert_eq!(one("$84 split 3 ways"), "$28");
        assert_eq!(one("split $84 3 ways"), "$28");
        assert_eq!(one("Dinner: $240 split between 4 people"), "$60");
        assert_eq!(one("18% tip on $62"), "$73.16");
        assert_eq!(one("$62 + 18% tip"), "$73.16");
        assert_eq!(one("4 GB at 100 Mbps"), "5 min 20 s");
        assert_eq!(one("Game: 80 GB at 50 Mbps"), "3 h 33 min");
        assert_eq!(one("700 MB @ 10 MB/s"), "1 min 10 s");
        assert_eq!(one("10 MB at 100 Mbps"), "0.8 s");
        // "at" a price is still multiplication.
        assert_eq!(one("3 pizzas at $12"), "$36");
    }

    #[test]
    fn copies_a_markdown_table() {
        let doc = "# Trip | Kyoto\n// notes\nFlights $1,180\n\n## Food\nLunch $18\ntotal";
        let table = markdown_table(doc, &eval_results(doc));
        assert_eq!(
            table,
            "| Trip \\| Kyoto | |\n|---|---:|\n| Flights $1,180 | $1,180 |\n| **Food** | |\n| Lunch $18 | $18 |\n| total | $18 |"
        );
    }

    #[test]
    fn colors_and_css_units() {
        let r = eval_results("#ff8800 in rgb\n#ff8800\n# Palette\n16px in rem");
        assert_eq!(r[0].display, "rgb(255, 136, 0)");
        assert_eq!(r[0].color, Some([255, 136, 0]));
        assert_eq!(r[1].kind, Kind::Value);
        assert_eq!(r[2].kind, Kind::Header);
        assert_eq!(r[3].display, "1 rem");
    }

    #[test]
    fn little_words_are_prose() {
        assert_eq!(one("I spent $40"), "$40");
        assert_eq!(one("A coffee $4"), "$4");
        assert_eq!(one("Coffee $4 a day"), "$4 / day");
        assert_eq!(one("3 pizzas at $12"), "$36");
        assert_eq!(one("Movie at 7pm"), "");
    }

    #[test]
    fn exact_big_and_visible_tiny_numbers() {
        assert_eq!(one("12345678901234567890"), "12,345,678,901,234,567,890");
        assert_eq!(one("1e-12"), "1e-12");
        assert_eq!(one("0.000123"), "0.00012");
        assert_eq!(one("$5 - $10"), "-$5");
        assert_eq!(one("0x10 + 1"), "0x11"); // fend answers hex in hex
        assert_eq!(one("5 x 3"), "15");
    }

    #[test]
    fn division_keeps_conversions() {
        assert_eq!(one("$1200 / 12 in EUR"), "90 EUR");
        assert_eq!(one("10 / 4 inches"), "2.5 inches");
        assert_eq!(one("60 km / 2 hours"), "30 km / hour");
    }

    #[test]
    fn prev_needs_a_line_above() {
        assert_eq!(one("prev * 2"), "");
    }

    #[test]
    fn version_is_prose() {
        assert_eq!(one("version"), "");
        assert_eq!(one("Pro version $40"), "$40");
    }

    #[test]
    fn imperial_measures() {
        assert_eq!(one("1 imperial gallon in liters"), "4.55 liters");
        assert_eq!(one("2 uk pints in ml"), "1,136.52 ml");
        assert_eq!(one("20 liters in imperial gallons"), "4.4 imperial gallons");
        assert_eq!(
            one("Fuel: 40 liters in imperial gallons"),
            "8.8 imperial gallons"
        );
        assert_eq!(one("1 imperial gallon in gallons"), "1.2 gallons");
        assert_eq!(one("1 pint in uk pints"), "0.83 imperial pints");
    }

    #[test]
    fn currency_code_before_amount() {
        assert_eq!(one("CHF 300 in USD"), "$333.33");
        assert_eq!(one("USD 30"), "$30");
    }

    #[test]
    fn notepad_conventions() {
        let doc = "# Budget\nRent: $1,200\nLunch with Bob $40 // friday\nsum\nprev * 2\n\n\"note\" 5\navg";
        assert_eq!(
            eval(doc),
            vec!["", "$1,200", "$40", "$1,240", "$2,480", "", "5", "5"]
        );
    }

    #[test]
    fn prose_with_symbols_and_named_totals() {
        let doc = "Food & drinks ₹2,000 * 4\nTaxi ₹800\nsubtotal = sum\ngst = 18% of subtotal\nsubtotal + gst";
        assert_eq!(
            eval(doc),
            vec![
                "8,000 INR",
                "800 INR",
                "8,800 INR",
                "1,584 INR",
                "10,384 INR"
            ]
        );
    }

    #[test]
    fn totals_do_not_count_themselves() {
        let doc = "Coffee $4.50\nTaxi $12\ntotal\naverage";
        assert_eq!(eval(doc), vec!["$4.50", "$12", "$16.50", "$8.25"]);
    }

    #[test]
    fn variables() {
        let doc = "rent = $1200\nrent * 12\nx = 5\nx + rent";
        let r = eval(doc);
        assert_eq!(r[0], "$1,200");
        assert_eq!(r[1], "$14,400");
        assert_eq!(r[2], "5");
    }

    #[test]
    fn base_conversion_is_not_grouped() {
        assert_eq!(one("255 in binary"), "11111111");
        assert_eq!(one("255 in hex"), "ff");
    }

    #[test]
    fn money_multiplication_and_dates() {
        assert_eq!(one("3 * $95"), "$285");
        assert_eq!(one("Hotel 3 nights * $95"), "$285");
        let today = chrono::Local::now().format("%A, %-d %B %Y").to_string();
        assert_eq!(one("today"), today);
    }

    #[test]
    fn prose_is_ignored() {
        assert_eq!(one("hello world"), "");
        assert_eq!(one("10 apples + 5"), "15");
    }

    #[test]
    fn markdown_prefixes_compute_and_variables_cross_prose() {
        let doc = "- hotel = 3 nights * $180\npack sunscreen\nhotel\n# Next\nhotel * 2";
        let r = eval_results(doc);
        assert_eq!(r[0].display, "$540");
        assert_eq!(r[1].display, "");
        assert_eq!(r[2].display, "$540");
        assert_eq!(r[3].kind, Kind::Header);
        assert_eq!(r[4].display, "$1,080");
        assert!(defined_variables(doc).contains("hotel"));
        assert!(defined_variables("- [x] hotel = 1\n| a | b |\n").contains("hotel"));
        assert!(!defined_variables("| hotel = 1 |\n").contains("hotel"));
    }
}
