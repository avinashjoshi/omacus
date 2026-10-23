//! Autocomplete suggestions for the word under the cursor.

use std::collections::HashSet;
use std::sync::LazyLock;

const KEYWORDS: &[&str] = &[
    "prev",
    "sum",
    "total",
    "average",
    "min",
    "max",
    "today",
    "tomorrow",
    "yesterday",
    "now",
    "time",
    "in",
];

/// Words drawn in the keyword colour, including ones that are never suggested
/// (`in`, `of`, `plus`) because they are already a whole word.
const COLOR_WORDS: &[&str] = &[
    "in",
    "to",
    "as",
    "of",
    "on",
    "off",
    "what",
    "is",
    "prev",
    "sum",
    "total",
    "avg",
    "average",
    "mean",
    "min",
    "max",
    "today",
    "now",
    "tomorrow",
    "yesterday",
    "plus",
    "minus",
    "times",
    "mod",
    "time",
];

const MAX: usize = 8;

/// One suggestion: the text to insert, a kind, and a short description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub text: String,
    /// `variable`, `keyword`, `unit`, or `zone`.
    pub kind: &'static str,
    pub detail: String,
}

impl Suggestion {
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            "zone" => "time zone",
            other => other,
        }
    }
}

/// Suggestions for `prefix`: page variables first, then keywords, time zones and fend's units.
pub fn suggest(prefix: &str, vars: &HashSet<String>) -> Vec<Suggestion> {
    // Connecting words are almost always complete as typed.
    const STOP: &[&str] = &["in", "to", "as", "of", "on", "at", "is", "by", "per", "off"];
    if prefix.chars().count() < 2 || STOP.contains(&prefix.to_lowercase().as_str()) {
        return Vec::new();
    }
    let lower = prefix.to_lowercase();
    let starts = |w: &str| w.to_lowercase().starts_with(&lower) && w.len() > prefix.len();

    let mut names: Vec<String> = vars.iter().filter(|v| starts(v)).cloned().collect();
    names.sort();
    let units = fend_core::get_completions_for_prefix(prefix).1;

    let mut candidates = Vec::new();
    for name in names {
        candidates.push(Suggestion {
            text: name,
            kind: "variable",
            detail: "a name on this page".to_string(),
        });
    }
    for word in KEYWORDS.iter().copied().filter(|k| starts(k)) {
        candidates.push(Suggestion {
            text: word.to_string(),
            kind: "keyword",
            detail: keyword_detail(word).to_string(),
        });
    }
    for zone in crate::time::completion_words().filter(|z| starts(z)) {
        candidates.push(Suggestion {
            text: zone.to_string(),
            kind: "zone",
            detail: "a time zone".to_string(),
        });
    }
    for unit in units
        .iter()
        .map(|c| c.display().to_string())
        .filter(|u| starts(u))
    {
        candidates.push(Suggestion {
            text: unit,
            kind: "unit",
            detail: "a unit".to_string(),
        });
    }

    let mut out = Vec::new();
    for item in candidates {
        if out.iter().any(|seen: &Suggestion| seen.text == item.text) {
            continue;
        }
        out.push(item);
        if out.len() == MAX {
            break;
        }
    }
    out
}

/// `keyword`, `unit`, `zone`, or nothing. Used to colour a word that is not a page variable.
pub fn word_kind(word: &str) -> Option<&'static str> {
    let lower = word.to_ascii_lowercase();
    if COLOR_WORDS.contains(&lower.as_str()) {
        return Some("keyword");
    }
    if ZONES.contains(&lower) {
        return Some("zone");
    }
    if UNITS.contains(&lower) {
        return Some("unit");
    }
    None
}

fn keyword_detail(word: &str) -> &'static str {
    match word {
        "prev" => "the line above",
        "sum" | "total" => "lines since a blank line or heading",
        "average" => "the average of those lines",
        "min" => "the smallest value",
        "max" => "the largest value",
        "today" => "today's date",
        "tomorrow" => "tomorrow's date",
        "yesterday" => "yesterday's date",
        "now" => "the time now",
        "time" => "the time in a place",
        "in" => "convert into",
        _ => "a keyword",
    }
}

static ZONES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    crate::time::completion_words()
        .map(|z| z.to_ascii_lowercase())
        .collect()
});

/// Unit names fend completes, lowercased. Built once from each letter, because a
/// one-letter prefix lists every longer unit that starts with it.
static UNITS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut set = HashSet::new();
    for letter in 'a'..='z' {
        let prefix = letter.to_string();
        for unit in fend_core::get_completions_for_prefix(&prefix).1 {
            set.insert(unit.display().to_ascii_lowercase());
        }
    }
    set
});

/// The identifier ending at `cursor` (a char index): `(start_char, word)`.
pub fn word_before(text: &str, cursor: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    if cursor > chars.len() {
        return None;
    }
    // Only complete at the end of a word, not in the middle of one.
    if chars
        .get(cursor)
        .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    {
        return None;
    }
    let mut start = cursor;
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    let word: String = chars[start..cursor].iter().collect();
    // Skip numbers ("1.5k") and comments/headers.
    if !word.chars().next().is_some_and(char::is_alphabetic) {
        return None;
    }
    let line_start = chars[..start]
        .iter()
        .rposition(|&c| c == '\n')
        .map_or(0, |i| i + 1);
    let line: String = chars[line_start..start].iter().collect();
    if line.trim_start().starts_with('#')
        || line.contains("//")
        || line.matches('"').count() % 2 == 1
    {
        return None;
    }
    Some((start, word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_vars_units_and_zones() {
        let vars = HashSet::from(["rent".to_string(), "rate".to_string()]);
        let first = &suggest("re", &vars)[0];
        assert_eq!(first.text, "rent");
        assert_eq!(first.kind, "variable");
        assert_eq!(first.detail, "a name on this page");
        let mile = suggest("mil", &vars)
            .into_iter()
            .find(|s| s.text == "mile")
            .expect("mile");
        assert_eq!(mile.kind, "unit");
        assert_eq!(mile.detail, "a unit");
        assert!(
            suggest("fahr", &vars)
                .iter()
                .any(|s| s.text == "fahrenheit")
        );
        let pst = suggest("PS", &vars)
            .into_iter()
            .find(|s| s.text == "PST")
            .expect("PST");
        assert_eq!(pst.kind, "zone");
        assert_eq!(pst.detail, "a time zone");
        let tomorrow = suggest("tomo", &vars)
            .into_iter()
            .find(|s| s.text == "tomorrow")
            .expect("tomorrow");
        assert_eq!(tomorrow.kind, "keyword");
        assert_eq!(tomorrow.detail, "tomorrow's date");
        assert!(suggest("r", &vars).is_empty());
        assert!(suggest("in", &vars).is_empty());
    }

    #[test]
    fn colours_units_zones_and_keywords() {
        assert_eq!(word_kind("mile"), Some("unit"));
        assert_eq!(word_kind("PST"), Some("zone"));
        assert_eq!(word_kind("total"), Some("keyword"));
        assert_eq!(word_kind("rent"), None);
    }

    #[test]
    fn finds_word_at_cursor() {
        assert_eq!(word_before("3 km in mi", 10), Some((8, "mi".into())));
        assert_eq!(word_before("x = 1.5", 7), None);
        assert_eq!(word_before("// note", 7), None);
        assert_eq!(word_before("rent\nre", 7), Some((5, "re".into())));
    }
}
