//! Autocomplete suggestions for the word under the cursor.

use std::collections::HashSet;

const KEYWORDS: &[&str] = &[
    "prev",
    "sum",
    "total",
    "average",
    "today",
    "tomorrow",
    "yesterday",
    "now",
    "time",
    "in",
];

const MAX: usize = 8;

/// Suggestions for `prefix`: sheet variables first, then keywords, time zones and fend's units.
pub fn suggest(prefix: &str, vars: &HashSet<String>) -> Vec<String> {
    // Connecting words are almost always complete as typed.
    const STOP: &[&str] = &["in", "to", "as", "of", "on", "at", "is", "by", "per", "off"];
    if prefix.chars().count() < 2 || STOP.contains(&prefix.to_lowercase().as_str()) {
        return Vec::new();
    }
    let lower = prefix.to_lowercase();
    let starts = |w: &str| w.to_lowercase().starts_with(&lower) && w.len() > prefix.len();

    let mut vars: Vec<String> = vars.iter().filter(|v| starts(v)).cloned().collect();
    vars.sort();
    let units = fend_core::get_completions_for_prefix(prefix).1;

    let mut candidates = vars;
    candidates.extend(KEYWORDS.iter().filter(|k| starts(k)).map(|k| k.to_string()));
    candidates.extend(
        crate::time::completion_words()
            .filter(|z| starts(z))
            .map(String::from),
    );
    candidates.extend(
        units
            .iter()
            .map(|c| c.display().to_string())
            .filter(|u| starts(u)),
    );

    let mut out: Vec<String> = Vec::new();
    for c in candidates {
        if !out.contains(&c) {
            out.push(c);
        }
        if out.len() == MAX {
            break;
        }
    }
    out
}

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
        assert_eq!(suggest("re", &vars)[0], "rent");
        assert!(suggest("mil", &vars).contains(&"mile".to_string()));
        assert!(suggest("fahr", &vars).contains(&"fahrenheit".to_string()));
        assert!(suggest("PS", &vars).contains(&"PST".to_string()));
        assert!(suggest("r", &vars).is_empty());
        assert!(suggest("in", &vars).is_empty());
    }

    #[test]
    fn finds_word_at_cursor() {
        assert_eq!(word_before("3 km in mi", 10), Some((8, "mi".into())));
        assert_eq!(word_before("x = 1.5", 7), None);
        assert_eq!(word_before("// note", 7), None);
        assert_eq!(word_before("rent\nre", 7), Some((5, "re".into())));
    }
}
