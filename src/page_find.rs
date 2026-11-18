//! Literal, case-insensitive page search. Offsets are Unicode character indexes.
#[derive(Default)]
pub struct PageFind {
    pub open: bool,
    pub focus: bool,
    pub query: String,
    pub index: usize,
    pub reveal: bool,
    pub page: String,
}

pub fn matches(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let needle: Vec<String> = query.chars().map(|c| c.to_lowercase().collect()).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let hay: Vec<String> = text.chars().map(|c| c.to_lowercase().collect()).collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at + needle.len() <= hay.len() {
        if hay[at..at + needle.len()] == needle {
            found.push(at..at + needle.len());
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
}

pub fn step(index: usize, count: usize, backwards: bool) -> usize {
    if count == 0 {
        0
    } else if backwards {
        (index + count - 1) % count
    } else {
        (index + 1) % count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_unicode_matches_use_character_offsets() {
        assert_eq!(
            matches("🍎 Café\ncAFÉ + café", "café"),
            vec![2..6, 7..11, 14..18]
        );
        assert_eq!(matches("a.b a+b", "."), vec![1..2]);
        assert!(matches("hello", "").is_empty());
        assert!(matches("hello", "missing").is_empty());
        assert_eq!(matches("aaaa", "aa"), vec![0..2, 2..4]);
    }
    #[test]
    fn navigation_wraps_and_handles_no_matches() {
        assert_eq!(step(2, 3, false), 0);
        assert_eq!(step(0, 3, true), 2);
        assert_eq!(step(0, 0, true), 0);
    }
}
