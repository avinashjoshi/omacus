//! The notepad layer on top of fend-core.
//!
//! fend already understands arithmetic, units, currencies and variables. This
//! module adds the notepad conventions: headers, comments, labels, `prev`,
//! `sum`/`avg`, percentage phrases, currency symbols, forgiving prose, and
//! human-friendly result formatting.

use std::sync::LazyLock;

use regex::Regex;

use crate::rates::Rates;

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
}

impl LineResult {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            display: String::new(),
        }
    }
}

pub struct Engine {
    rates: Rates,
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

pub(crate) const LABEL_PATTERN: &str = r"^\s*([A-Za-z][A-Za-z0-9 '\-_&/]*?)\s*:\s+(.*)$";
re!(LABEL, LABEL_PATTERN);
re!(QUOTED, r#""[^"]*""#);
re!(ASSIGN, r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^=].*)$");
re!(PCT_ON, r"(?i)^(.+?)\s*%\s+on\s+(.+)$");
re!(PCT_OFF, r"(?i)^(.+?)\s*%\s+off\s+(.+)$");
re!(PCT_OF_WHAT, r"(?i)^(.+?)\s*%\s+of\s+what\s+is\s+(.+)$");
re!(AS_PCT, r"(?i)^(.+?)\s+as\s+(?:a\s+)?%\s+(?:of\s+)?(.+)$");
re!(IS_WHAT_PCT, r"(?i)^(.+?)\s+is\s+what\s+%\s+of\s+(.+)$");
re!(IS_PCT_OF_WHAT, r"(?i)^(.+?)\s+is\s+(.+?)\s*%\s+of\s+what$");
// "$100 - 20%", "$50 + 8% tax": a percentage of the amount, not of 1.
re!(
    PLUS_MINUS_PCT,
    r"(?i)^([^%]+?)\s*([+-])\s*(\d[\d,]*(?:\.\d+)?)\s*%(?:\s+[a-z][a-z ]*)?$"
);
// fend reads `60 km / 2 hours` as `(60 km / 2) hours`; keep the divisor together.
// Only when the amount before it has a unit too (`10 / 4 inches` stays fend's).
re!(
    DIVISOR,
    r"([A-Za-z)]\s*)/\s*(\(?\d[\d,]*(?:\.\d+)?\)?\s*)([A-Za-z][A-Za-z_]*(?:\^\d+)?(?:/[A-Za-z][A-Za-z_]*(?:\^\d+)?)*)"
);
re!(TIMES_X, r"(\d+)(\s*)([x×])(\s*)(\d)");
re!(SYMBOL_NUM, r"([€¥₹₩₽₺])\s*(\d[\d,]*(?:\.\d+)?)");
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

const COMMON_CURRENCIES: &[&str] = &[
    "USD", "EUR", "GBP", "INR", "JPY", "CNY", "CAD", "AUD", "CHF", "NZD", "SGD", "HKD", "SEK",
    "NOK", "DKK", "MXN", "BRL", "KRW", "ZAR", "AED", "SAR", "RUB", "PLN", "THB", "IDR",
];

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
        Self { rates }
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

        text.split('\n')
            .enumerate()
            .map(|(i, line)| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    block.clear();
                    return LineResult::new(Kind::Empty);
                }
                if trimmed.starts_with('#') {
                    block.clear();
                    return LineResult::new(Kind::Header);
                }
                if trimmed.starts_with("//") {
                    return LineResult::new(Kind::Comment);
                }

                if let Some(date) = crate::time::eval(trimmed) {
                    return LineResult {
                        kind: Kind::Value,
                        display: date,
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
                    Some(expr) => eval_into(&mut ctx, &var, &expr),
                    None => {
                        let expr = self.currency_first(&rewrite(source, prev.as_deref()));
                        let expr = self.drop_name_collisions(&expr);
                        if expr.trim().is_empty() {
                            return LineResult::new(Kind::Comment);
                        }
                        eval_forgiving(&mut ctx, &var, &expr)
                    }
                };

                match result {
                    Some(raw) => {
                        // A total shouldn't count itself in the next total or average.
                        if !is_total {
                            block.push(var.clone());
                        }
                        prev = Some(var);
                        let display = if BASE_CONV.is_match(trimmed) {
                            raw.trim_start_matches("approx. ").to_string()
                        } else {
                            pretty(&raw)
                        };
                        let display = match target {
                            Some((_, name)) => format!("{display} {name}"),
                            None => display,
                        };
                        LineResult {
                            kind: Kind::Value,
                            display,
                        }
                    }
                    None => LineResult::new(Kind::Error),
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

    /// "Lunch with Bob $40": fend reads "Bob" as the Bolivian boliviano. Title-case
    /// words that collide with uncommon currency codes are treated as prose.
    fn drop_name_collisions(&self, expr: &str) -> String {
        WORD.replace_all(expr, |c: &regex::Captures| {
            let w = &c[0];
            let title = w.chars().next().is_some_and(char::is_uppercase)
                && w.chars().skip(1).all(char::is_lowercase);
            let upper = w.to_ascii_uppercase();
            if title && !COMMON_CURRENCIES.contains(&upper.as_str()) && self.rates.knows(&upper) {
                String::new()
            } else {
                w.to_string()
            }
        })
        .into_owned()
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
pub fn rewrite(line: &str, prev: Option<&str>) -> String {
    // Inline comments and quoted text are ignored.
    let mut s = match line.find("//") {
        Some(idx) => line[..idx].to_string(),
        None => line.to_string(),
    };
    s = QUOTED.replace_all(&s, " ").into_owned();
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
    s = TIMES_X
        .replace_all(&s, |c: &regex::Captures| {
            if &c[1] == "0" && c[2].is_empty() && &c[3] == "x" {
                c[0].to_string()
            } else {
                format!("{} * {}", &c[1], &c[5])
            }
        })
        .into_owned();
    s = A_DAY.replace_all(&s, "$1 / $2").into_owned();
    s = AT_PRICE.replace_all(&s, "$1 * $2").into_owned();
    s = drop_prose_words(&s);
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
    } else if let Some(c) = PCT_OFF.captures(&rhs) {
        format!("({}) * (1 - ({}) / 100)", &c[2], &c[1])
    } else if let Some(c) = AS_PCT.captures(&rhs) {
        format!("(({}) / ({})) to %", &c[1], &c[2])
    } else if let Some(c) = PLUS_MINUS_PCT.captures(&rhs) {
        format!(
            "({}) * (1 {} ({}) / 100)",
            &c[1],
            &c[2],
            c[3].replace(',', "")
        )
    } else {
        rhs
    };

    match lhs {
        Some(name) => format!("{name} = {rhs}"),
        None => rhs,
    }
}

/// Words that would be a conversion or operator, not a unit, after a number.
const KEYWORDS: &[&str] = &[
    "in", "to", "as", "of", "per", "mod", "on", "off", "is", "and",
];

/// Little words fend would read as units (I is the imaginary unit, a is an are, at
/// is an atmosphere...). They're prose unless they directly follow a number.
fn drop_prose_words(s: &str) -> String {
    const PROSE: &[&str] = &[
        "I", "a", "A", "an", "An", "the", "The", "at", "At", "my", "My",
    ];
    let mut out: Vec<&str> = Vec::new();
    for tok in s.split(' ') {
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
fn eval_into(ctx: &mut fend_core::Context, var: &str, expr: &str) -> Option<String> {
    let full = match ASSIGN.captures(expr) {
        // `x = 5` -> `x = (5); _l3 = x`
        Some(c) => format!("{} = ({}); {var} = {}", &c[1], &c[2], &c[1]),
        None => format!("{var} = ({expr})"),
    };
    let r = fend_core::evaluate(&full, ctx).ok()?;
    let out = r.get_main_result().trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// Like `eval_into`, but drops stray words fend doesn't know
/// ("Lunch with Bob $40" -> "$40"), so lines can read like notes.
fn eval_forgiving(ctx: &mut fend_core::Context, var: &str, expr: &str) -> Option<String> {
    let meaningful = expr
        .split(|c: char| !c.is_alphabetic())
        .any(|w| MEANINGFUL.contains(&w.to_ascii_lowercase().as_str()));
    if meaningful {
        return None;
    }
    drop_unknown_words(ctx, var, expr).or_else(|| {
        // Prose fend can't parse at all ("Food & drinks ₹2,000 * 4"): start the
        // expression at the first number, currency or bracket instead.
        if ASSIGN.is_match(expr) {
            return None;
        }
        let tokens: Vec<&str> = expr.split_whitespace().collect();
        let start = tokens
            .iter()
            .position(|t| t.starts_with(|c: char| c.is_ascii_digit() || "$£€¥₹(-.".contains(c)))?;
        if start == 0 {
            return None;
        }
        drop_unknown_words(ctx, var, &tokens[start..].join(" "))
    })
}

/// Words that change what a line means; dropping them would give a wrong answer.
const MEANINGFUL: &[&str] = &[
    "until", "since", "ago", "before", "after", "every", "except", "without", "not",
];

fn drop_unknown_words(ctx: &mut fend_core::Context, var: &str, expr: &str) -> Option<String> {
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
        match fend_core::evaluate(&full, ctx) {
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

    fn eval(doc: &str) -> Vec<String> {
        Engine::new(Rates::fixed(&[
            ("USD", 1.0),
            ("EUR", 0.9),
            ("INR", 83.0),
            ("GBP", 0.78),
            ("BOB", 6.9),
            ("CHF", 0.9),
        ]))
        .eval_document(doc)
        .into_iter()
        .map(|r| r.display)
        .collect()
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
        assert_eq!(one("days until Dec 25"), "");
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
