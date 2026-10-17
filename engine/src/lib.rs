//! The notepad layer on top of fend-core.
//!
//! fend already understands arithmetic, units, currencies and variables. This
//! module adds the notepad conventions: headers, comments, labels, `prev`,
//! `sum`/`avg`, percentage phrases, currency symbols, forgiving prose, and
//! human-friendly result formatting.

use std::collections::HashSet;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use regex::Regex;

pub mod complete;
pub mod extras;
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
    /// shown quietly in place of one.
    pub note: Option<&'static str>,
}

impl LineResult {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            display: String::new(),
            color: None,
            note: None,
        }
    }

    fn value(display: String) -> Self {
        Self {
            kind: Kind::Value,
            display,
            color: None,
            note: None,
        }
    }

    fn error(note: Option<&'static str>) -> Self {
        Self {
            note,
            ..Self::new(Kind::Error)
        }
    }
}

/// How long one line may calculate before it's stopped (`100000!` or `10^10^7`
/// would otherwise freeze the window), and the shorter turn each line gets once
/// a sheet has already taken `SHEET_MS`.
const LINE_MS: f64 = 150.0;
const LATE_LINE_MS: f64 = 20.0;
const SHEET_MS: f64 = 1000.0;
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
        if error.contains("division by zero") {
            Some("Can't divide by zero")
        } else if error.contains("are incompatible") {
            Some("Units don't match")
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
    let line = line.trim();
    line.starts_with('#') && !crate::extras::is_color(line)
}

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
re!(SYMBOL_NUM, r"([€¥₹₩₽₺])\s*(\d[\d,]*(?:\.\d+)?)");
// Dollars with a country prefix: "C$100", "A$50", "US$20".
re!(
    PREFIXED_DOLLAR,
    r"\b(US|CA|C|AU|A|NZ|HK|SG|S)\$\s*(\d[\d,]*(?:\.\d+)?)"
);
// A decimal comma: thousands always come in threes, so "1,5", "12,50" and
// "3,14159" can only mean 1.5, 12.50 and 3.14159 ("1,234" stays 1234).
re!(
    DECIMAL_COMMA,
    r"(^|[^\d,.])(\d+),(\d{1,2}|\d{4,})($|[^\d,])"
);
// A percentage times a plain number is a share of it: "10% * 200" is 20, not
// 2,000%. Divided, it stays a percentage ("50% / 2" is 25%).
re!(PCT_TIMES, r"(\d[\d.]*)\s*%(\s*\*\s*\(?\d)");
re!(TIMES_PCT, r"(\d\)?\s*[*/]\s*)(\d[\d.]*)\s*%");
re!(MONEY_PREFIX, r"[$£]\s*\d[\d,]*(?:\.\d+)?(?:[eE][+-]?\d+)?");
re!(PREV, r"(?i)\bprev\b");
// fend answers "version" with its own version number; in a sheet it's prose.
re!(VERSION_WORD, r"(?i)\bversion\b");
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
re!(NUM_TOKEN, r"^(-?)([$£€¥₹]?)(\d+)(?:\.(\d+))?$");

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
        let mut ctx = self.context();
        let mut block: Vec<String> = Vec::new();
        let mut prev: Option<String> = None;
        let started = self.clock.map(|now| now());
        // Names defined so far, and those holding a percentage ("tax = 8%").
        let mut vars: HashSet<String> = HashSet::new();
        let mut percents: Vec<(String, Regex, String)> = Vec::new();

        text.split('\n')
            .enumerate()
            .map(|(i, line)| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    block.clear();
                    return LineResult::new(Kind::Empty);
                }
                if is_header(trimmed) {
                    block.clear();
                    return LineResult::new(Kind::Header);
                }
                if trimmed.starts_with("//") {
                    return LineResult::new(Kind::Comment);
                }

                if let Some(date) = crate::time::eval(trimmed) {
                    return LineResult::value(date);
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
                if prev.is_none() && PREV.is_match(trimmed) && !ASSIGN.is_match(trimmed) {
                    return LineResult::new(Kind::Error);
                }

                let late = match (self.clock, started) {
                    (Some(now), Some(t0)) => now() - t0 > SHEET_MS,
                    _ => false,
                };
                let budget = Budget::new(self.clock, if late { LATE_LINE_MS } else { LINE_MS });
                let var = format!("_l{i}");
                // `sum`, or `subtotal = sum` to keep it in a variable.
                let total = match ASSIGN.captures(trimmed) {
                    Some(c) => aggregate(&c[2], &block).map(|e| format!("{} = {e}", &c[1])),
                    None => aggregate(trimmed, &block),
                };
                let is_total = total.is_some();
                let target = imperial_target(trimmed);
                let source = target.as_ref().map_or(trimmed, |(expr, _)| expr.as_str());
                let result = match total {
                    Some(expr) => eval_into(&mut ctx, &budget, &var, &expr),
                    None => {
                        // "$100 + tax", where tax = 8%: the same as "$100 + 8%".
                        let mut source = source.to_string();
                        for (_, re, pct) in &percents {
                            source = re.replace_all(&source, format!("${{1}}{pct}")).into_owned();
                        }
                        let expr = self.currency_first(&rewrite(&source, prev.as_deref(), &vars));
                        let expr = self.drop_name_collisions(&expr);
                        if expr.trim().is_empty() {
                            return LineResult::new(Kind::Comment);
                        }
                        eval_forgiving(&mut ctx, &budget, &var, &expr)
                            .or_else(|| self.with_currency(&mut ctx, &budget, &var, &expr))
                    }
                };

                if result.is_some()
                    && let Some(c) = ASSIGN.captures(trimmed)
                {
                    let name = c[1].to_string();
                    let value = c[2].trim();
                    // A redefinition replaces the old value, percentage or not.
                    percents.retain(|(n, ..)| *n != name);
                    if value.ends_with('%')
                        && value[..value.len() - 1].trim().parse::<f64>().is_ok()
                    {
                        let re = Regex::new(&format!(r"([+-]\s*)\b{}\b", regex::escape(&name)));
                        if let Ok(re) = re {
                            percents.push((name.clone(), re, value.to_string()));
                        }
                    }
                    vars.insert(name);
                }

                // Every digit of an approximate answer (see `precisely`), except for
                // base conversions, which fend formats itself.
                let result = result.map(|raw| {
                    if BASE_CONV.is_match(trimmed) {
                        raw
                    } else {
                        precisely(&mut ctx, &budget, &var, raw)
                    }
                });

                match result {
                    Some(raw) => {
                        // A total shouldn't count itself in the next total or average.
                        if !is_total {
                            block.push(var.clone());
                        }
                        prev = Some(var);
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
                        LineResult::value(display)
                    }
                    None => LineResult::error(budget.note()),
                }
            })
            .collect()
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
    fn drop_name_collisions(&self, expr: &str) -> String {
        WORD.replace_all(expr, |c: &regex::Captures| {
            let w = &c[0];
            let m = c.get(0).unwrap();
            let title = w.chars().next().is_some_and(char::is_uppercase)
                && w.chars().skip(1).all(char::is_lowercase);
            let before = expr[..m.start()].trim_end();
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

    /// "rent + 5" where rent is $1,000: a plain number added to or taken from an
    /// amount is in its currency. Only tried when fend refused to mix the two.
    fn with_currency(
        &self,
        ctx: &mut fend_core::Context,
        budget: &Budget,
        var: &str,
        expr: &str,
    ) -> Option<String> {
        let error = budget.error.lock().unwrap().clone()?;
        let c = CURRENCY_VS_PLAIN.captures(&error)?;
        let unit = match (&c[3], &c[4]) {
            ("unitless", "BASE_CURRENCY") => c[2].to_string(),
            ("BASE_CURRENCY", "unitless") => c[1].to_string(),
            _ => return None,
        };
        let (lhs, rhs) = match ASSIGN.captures(expr) {
            Some(c) => (Some(c[1].to_string()), c[2].to_string()),
            None => (None, expr.to_string()),
        };
        let mut rhs = rhs;
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
        .filter_map(|l| ASSIGN.captures(l).map(|c| c[1].to_string()))
        .collect()
}

/// `sum`, `total`, `avg`, `average` over the values since the last blank line or header.
fn aggregate(line: &str, block: &[String]) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let lower = lower.trim_end_matches(':').trim();
    let is_sum = matches!(lower, "sum" | "total");
    let is_avg = matches!(lower, "avg" | "average" | "mean");
    if !(is_sum || is_avg) {
        return None;
    }
    if block.is_empty() {
        return Some("0".into());
    }
    let sum = block.join(" + ");
    Some(if is_avg {
        format!("({sum}) / {}", block.len())
    } else {
        sum
    })
}

/// Turn a notepad-style line into something fend understands.
pub fn rewrite(line: &str, prev: Option<&str>, vars: &HashSet<String>) -> String {
    // Inline comments and quoted text are ignored.
    let mut s = match line.find("//") {
        Some(idx) => line[..idx].to_string(),
        None => line.to_string(),
    };
    s = QUOTED.replace_all(&s, " ").into_owned();
    // Twice: "1,5 + 2,5" shares the space between two matches.
    for _ in 0..2 {
        s = DECIMAL_COMMA
            .replace_all(&s, "${1}${2}.${3}${4}")
            .into_owned();
    }
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

    // Keep the variable name out of the percentage rewrites.
    let (lhs, mut rhs) = match ASSIGN.captures(&s) {
        Some(c) => (Some(c[1].to_string()), c[2].to_string()),
        None => (None, s.clone()),
    };

    for (re, op) in WORD_OP_RES.iter() {
        rhs = re.replace_all(&rhs, *op).into_owned();
    }

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
            // "100 + 10% * 2": the percentage term first (20%), as in ordinary
            // precedence, then applied to the amount.
            let term = c[3].replace(',', "").replace('%', " / 100");
            let step = format!("(({}) * (1 {} ({term})))", &c[1], &c[2]);
            out = format!("{step}{}", &out[c.get(0).unwrap().end()..]);
        }
        out
    } else {
        rhs
    };

    // Any percentage still multiplied by a number (the ones added or taken away
    // were handled above) is that share of it.
    rhs = PCT_TIMES.replace_all(&rhs, "($1 / 100)$2").into_owned();
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
    let assigned = ASSIGN.captures(s).map(|c| c[1].to_string());
    let mut out: Vec<&str> = Vec::new();
    for tok in s.split(' ') {
        if vars.contains(tok) || assigned.as_deref() == Some(tok) {
            out.push(tok);
            continue;
        }
        let after_number = out
            .iter()
            .rev()
            .find(|t| !t.is_empty())
            .is_some_and(|t| t.ends_with(|c: char| c.is_ascii_digit() || c == ')'));
        if PROSE.contains(&tok) && !after_number {
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
                if qualifies_unit(&expr[at + word.len()..]) {
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
            let (body, pct) = match tok.strip_suffix('%') {
                Some(b) => (b, "%"),
                None => (tok, ""),
            };
            match NUM_TOKEN.captures(body) {
                Some(c) => {
                    let n = format_number(&c[3], c.get(4).map(|m| m.as_str()), is_money);
                    format!("{}{}{n}{pct}", &c[1], &c[2])
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
    // Beyond 15 digits an f64 can't hold the number exactly; group fend's digits as
    // they are.
    if int.trim_start_matches('0').len() > 15 {
        return group_thousands(int);
    }
    let value: f64 = format!("{int}.{}", frac.unwrap_or("0"))
        .parse()
        .unwrap_or(0.0);
    let leading = frac.map_or(0, |f| f.chars().take_while(|&c| c == '0').count());
    if value != 0.0 && value < 1e-6 {
        // Tiny: 1.6e-19, not 0.
        let sci = format!("{value:.2e}");
        let (m, e) = sci.split_once('e').unwrap_or((&sci, "0"));
        let m = m.trim_end_matches('0').trim_end_matches('.');
        return format!("{m}e{e}");
    }
    let decimals = if value >= 1.0 || (money && value >= 0.01) {
        2
    } else {
        // Two significant digits for small numbers: 0.000123 -> 0.00012
        leading + 2
    };
    let rounded = format!("{value:.decimals$}");
    let (i, f) = rounded.split_once('.').unwrap_or((&rounded, ""));
    let f = if money && decimals == 2 && !f.trim_end_matches('0').is_empty() {
        f.to_string() // $41.10, not $41.1
    } else {
        f.trim_end_matches('0').to_string()
    };
    let grouped = group_thousands(i);
    if f.is_empty() {
        grouped
    } else {
        format!("{grouped}.{f}")
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
}
