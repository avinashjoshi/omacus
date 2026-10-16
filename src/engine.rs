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

re!(LABEL, r"^\s*([A-Za-z][A-Za-z '\-_&/]*?)\s*:\s+(.*)$");
re!(QUOTED, r#""[^"]*""#);
re!(ASSIGN, r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^=].*)$");
re!(PCT_ON, r"(?i)^(.+?)\s*%\s+on\s+(.+)$");
re!(PCT_OFF, r"(?i)^(.+?)\s*%\s+off\s+(.+)$");
re!(PCT_OF_WHAT, r"(?i)^(.+?)\s*%\s+of\s+what\s+is\s+(.+)$");
re!(AS_PCT, r"(?i)^(.+?)\s+as\s+(?:a\s+)?%\s+(?:of\s+)?(.+)$");
re!(TIMES_X, r"(\d)\s*[x×]\s*(\d)");
re!(SYMBOL_NUM, r"([€¥₹₩₽₺])\s*(\d[\d,]*(?:\.\d+)?)");
re!(MONEY_PREFIX, r"[$£]\s*\d[\d,]*(?:\.\d+)?");
re!(PREV, r"(?i)\bprev\b");
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

                let var = format!("_l{i}");
                let total = aggregate(trimmed, &block);
                let is_total = total.is_some();
                let result = match total {
                    Some(expr) => eval_into(&mut ctx, &var, &expr),
                    None => {
                        let expr = self.drop_name_collisions(&rewrite(trimmed, prev.as_deref()));
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

    if let Some(p) = prev {
        s = PREV.replace_all(&s, p).into_owned();
    }

    s = TIMES_X.replace_all(&s, "$1 * $2").into_owned();

    // Keep the variable name out of the percentage rewrites.
    let (lhs, mut rhs) = match ASSIGN.captures(&s) {
        Some(c) => (Some(c[1].to_string()), c[2].to_string()),
        None => (None, s.clone()),
    };

    for (re, op) in WORD_OP_RES.iter() {
        rhs = re.replace_all(&rhs, *op).into_owned();
    }

    rhs = if let Some(c) = PCT_OF_WHAT.captures(&rhs) {
        format!("({}) / (({}) / 100)", &c[2], &c[1])
    } else if let Some(c) = PCT_ON.captures(&rhs) {
        format!("({}) * (1 + ({}) / 100)", &c[2], &c[1])
    } else if let Some(c) = PCT_OFF.captures(&rhs) {
        format!("({}) * (1 - ({}) / 100)", &c[2], &c[1])
    } else if let Some(c) = AS_PCT.captures(&rhs) {
        format!("(({}) / ({})) to %", &c[1], &c[2])
    } else {
        rhs
    };

    match lhs {
        Some(name) => format!("{name} = {rhs}"),
        None => rhs,
    }
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
    let mut expr = expr.to_string();
    for _ in 0..12 {
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
                let re = Regex::new(&format!(r"\b{}\b", regex::escape(&word))).ok()?;
                let next = re.replace(&expr, " ").into_owned();
                if next == expr || !next.chars().any(|c| c.is_ascii_alphanumeric()) {
                    return None;
                }
                expr = next;
            }
        }
    }
    None
}

/// Round long decimals and add thousands separators for display.
pub fn pretty(raw: &str) -> String {
    let s = raw.trim_start_matches("approx. ");
    if looks_like_date(s) {
        return s.to_string();
    }
    let is_money = s.starts_with(['$', '£', '€', '¥', '₹'])
        || s.split_whitespace()
            .any(|t| t.len() == 3 && t.chars().all(|c| c.is_ascii_uppercase()));

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
    let value: f64 = format!("{int}.{}", frac.unwrap_or("0"))
        .parse()
        .unwrap_or(0.0);
    let decimals = if money || value >= 1.0 {
        2
    } else {
        // Keep two significant digits for small numbers: 0.000123 -> 0.00012
        let leading = frac.map_or(0, |f| f.chars().take_while(|&c| c == '0').count());
        (leading + 2).min(10)
    };
    let rounded = format!("{value:.decimals$}");
    let (i, f) = rounded.split_once('.').unwrap_or((&rounded, ""));
    let f = if money && !f.trim_end_matches('0').is_empty() {
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
