//! Rename one variable everywhere it is used on the page.
//!
//! A use belongs to the latest definition of that exact name on an earlier line.
//! The right-hand side of a redefinition (`kg = kg + 1`) still sees the previous
//! one, because the new value is stored only after the line is calculated.
//! Comments, quotes, prose, and a bare `total` / `sum` / `avg` are not uses.
//! A unit or currency word is a use only once a variable of that name is in
//! scope and the word sits where a value would (`3 kg` after `kg = 4`).

use std::collections::{HashMap, HashSet};

use crate::markdown::{self, Form};

/// Why a rename was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameFail {
    /// The cursor is not on a variable name.
    NotAVariable,
    /// The new name is empty or not an identifier.
    InvalidName,
    /// Some other definition on the page already uses `name`.
    Collision { name: String },
}

impl RenameFail {
    pub fn message(&self) -> String {
        match self {
            RenameFail::NotAVariable => String::new(),
            RenameFail::InvalidName => "Names start with a letter".to_string(),
            RenameFail::Collision { name } => format!("{name} is already used"),
        }
    }
}

/// A half-open range of char indexes into the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// The variable under the cursor, and every place that name is that variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
    /// The occurrence the cursor is on.
    pub token_start: usize,
    pub token_end: usize,
    /// Uses other than the definition.
    pub references: usize,
    /// Definition first, then each use, in page order.
    pub spans: Vec<Span>,
    pub definition: Span,
}

/// The page after a rename, and a cursor just after the token that was renamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Renamed {
    pub text: String,
    pub cursor: usize,
}

/// Someone edited a definition's name in place. `spans` are the uses still
/// written with `old_name`, in the text as it is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionEdit {
    pub old_name: String,
    pub new_name: String,
    pub references: usize,
    pub spans: Vec<Span>,
    pub collision: bool,
}

impl DefinitionEdit {
    pub fn message(&self) -> String {
        if self.collision {
            format!("{} is already used", self.new_name)
        } else {
            update_label(self.references)
        }
    }

    /// Write `new_name` over the uses. The definition is already the new name.
    pub fn apply(&self, text: &str) -> String {
        rewrite(text, &self.spans, &self.new_name)
    }
}

pub fn update_label(references: usize) -> String {
    if references == 1 {
        "Update 1 reference?".to_string()
    } else {
        format!("Update {references} references?")
    }
}

/// The binding whose name is under `cursor` (a char index). `None` for prose,
/// comments, numbers, and a word that isn't this variable.
pub fn binding_at(text: &str, cursor: usize) -> Option<Binding> {
    let bindings = collect(text);
    let (index, occ) = locate(&bindings, cursor)?;
    Some(publish(&bindings[index], occ))
}

/// Replace every occurrence of the variable under `cursor` with `new_name`.
/// The same name is a no-op. Invalid names and collisions are refused.
pub fn rename_variable(text: &str, cursor: usize, new_name: &str) -> Result<Renamed, RenameFail> {
    let new_name = new_name.trim();
    let bindings = collect(text);
    let (index, occ) = locate(&bindings, cursor).ok_or(RenameFail::NotAVariable)?;
    let binding = &bindings[index];
    let token_start = binding.occs[occ].start;
    let token_end = binding.occs[occ].end;
    if new_name == binding.name {
        return Ok(Renamed {
            text: text.to_string(),
            cursor: token_end,
        });
    }
    if !is_ident(new_name) {
        return Err(RenameFail::InvalidName);
    }
    if bindings
        .iter()
        .enumerate()
        .any(|(i, other)| i != index && other.name == new_name)
    {
        return Err(RenameFail::Collision {
            name: new_name.to_string(),
        });
    }
    let spans: Vec<Span> = binding.occs.iter().map(occ_span).collect();
    let text = rewrite(text, &spans, new_name);
    let cursor = cursor_after(&spans, token_start, new_name);
    Ok(Renamed { text, cursor })
}

/// A definition whose name was the only thing that changed between `old` and
/// `new`. `None` when the edit isn't that, the new name isn't an identifier,
/// or nothing else refers to the old name and the new one is free.
pub fn definition_change(old: &str, new: &str) -> Option<DefinitionEdit> {
    let old_lines: Vec<&str> = old.split('\n').collect();
    let new_lines: Vec<&str> = new.split('\n').collect();
    if old_lines.len() != new_lines.len() {
        return None;
    }
    let changed: Vec<usize> = old_lines
        .iter()
        .zip(&new_lines)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    if changed.len() != 1 {
        return None;
    }
    let index = changed[0];
    let (old_line, new_line) = (old_lines[index], new_lines[index]);
    let (os, oe) = crate::definition_span(old_line)?;
    let (ns, ne) = crate::definition_span(new_line)?;
    if old_line[..os] != new_line[..ns] || old_line[oe..] != new_line[ne..] {
        return None;
    }
    let old_name = &old_line[os..oe];
    let new_name = &new_line[ns..ne];
    if old_name == new_name || !is_ident(new_name) {
        return None;
    }

    let line_start = char_offset_of_line(old, index);
    let def_at = line_start + old_line[..os].chars().count();
    let binding = binding_at(old, def_at)?;
    if binding.name != old_name {
        return None;
    }
    let delta = new_name.chars().count() as isize - old_name.chars().count() as isize;
    let spans: Vec<Span> = binding
        .spans
        .iter()
        .filter(|span| *span != &binding.definition)
        .map(|span| Span {
            start: (span.start as isize + delta) as usize,
            end: (span.end as isize + delta) as usize,
        })
        .collect();

    let new_def = Span {
        start: line_start + new_line[..ns].chars().count(),
        end: line_start + new_line[..ns].chars().count() + new_name.chars().count(),
    };
    let collision = definitions(new)
        .into_iter()
        .any(|(name, span)| name == new_name && span != new_def);
    if spans.is_empty() && !collision {
        return None;
    }
    let references = spans.len();
    Some(DefinitionEdit {
        old_name: old_name.to_string(),
        new_name: new_name.to_string(),
        references,
        spans,
        collision,
    })
}

/// Write `new_name` over each span. Spans are char indexes, in any order.
pub fn rewrite(text: &str, spans: &[Span], new_name: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span> = spans.to_vec();
    spans.sort_by_key(|span| span.start);
    let mut out = String::new();
    let mut at = 0;
    for span in spans {
        if span.start < at || span.end < span.start || span.end > chars.len() {
            continue;
        }
        out.extend(chars[at..span.start].iter());
        out.push_str(new_name);
        at = span.end;
    }
    out.extend(chars[at..].iter());
    out
}

struct Occ {
    start: usize,
    end: usize,
    def: bool,
}

struct Found {
    name: String,
    occs: Vec<Occ>,
}

fn collect(text: &str) -> Vec<Found> {
    let mut active: HashMap<String, usize> = HashMap::new();
    let mut bindings: Vec<Found> = Vec::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut offset = 0;
    for (i, line) in lines.iter().enumerate() {
        let class = markdown::classify(line);
        let def = crate::definition_span(line);
        let vars: HashSet<String> = active.keys().cloned().collect();
        let mut defined: Option<(String, usize)> = None;
        if !matches!(class.form, Form::Blank | Form::Comment | Form::Table) {
            for token in identifiers(line) {
                let is_def =
                    def.is_some_and(|(from, to)| (token.byte_start, token.byte_end) == (from, to));
                let start = offset + token.start;
                let end = offset + token.end;
                if is_def {
                    let id = bindings.len();
                    bindings.push(Found {
                        name: token.text.clone(),
                        occs: vec![Occ {
                            start,
                            end,
                            def: true,
                        }],
                    });
                    defined = Some((token.text, id));
                    continue;
                }
                let Some(&id) = active.get(&token.text) else {
                    continue;
                };
                if is_reference(line, &token.text, token.byte_start, token.byte_end, &vars) {
                    bindings[id].occs.push(Occ {
                        start,
                        end,
                        def: false,
                    });
                }
            }
        }
        if let Some((name, id)) = defined {
            active.insert(name, id);
        }
        offset += line.chars().count();
        if i + 1 < lines.len() {
            offset += 1;
        }
    }
    bindings
}

struct Token {
    start: usize,
    end: usize,
    byte_start: usize,
    byte_end: usize,
    text: String,
}

/// Identifiers outside comments and quotes. A letter glued to a number
/// (`1.5k`) is a scale, not a name.
fn identifiers(line: &str) -> Vec<Token> {
    let class = markdown::classify(line);
    if matches!(class.form, Form::Blank | Form::Comment | Form::Table) {
        return Vec::new();
    }
    let comment_at = class.comment_at;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut byte = 0;
    let mut quoted = false;
    let mut out = Vec::new();
    while i < chars.len() {
        if comment_at.is_some_and(|at| byte >= at) {
            break;
        }
        let c = chars[i];
        if c == '"' {
            quoted = !quoted;
            byte += c.len_utf8();
            i += 1;
            continue;
        }
        let ident = c.is_ascii_alphabetic() || c == '_';
        if !quoted && ident {
            let start = i;
            let byte_start = byte;
            let glued = start > 0 && chars[start - 1].is_ascii_digit();
            loop {
                byte += chars[i].len_utf8();
                i += 1;
                if i >= chars.len() || !(chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    break;
                }
            }
            if !glued {
                out.push(Token {
                    start,
                    end: i,
                    byte_start,
                    byte_end: byte,
                    text: chars[start..i].iter().collect(),
                });
            }
            continue;
        }
        byte += c.len_utf8();
        i += 1;
    }
    out
}

fn is_reference(line: &str, word: &str, start: usize, end: usize, vars: &HashSet<String>) -> bool {
    let class = markdown::classify(line);
    if matches!(class.form, Form::Blank | Form::Comment | Form::Table) {
        return false;
    }
    if bare_aggregate(class.body, word) {
        return false;
    }
    if !crate::looks_like_math(class.body, vars) {
        return false;
    }
    if class.body.trim() == word {
        return true;
    }
    let code = match class.comment_at {
        Some(at) => &line[..at],
        None => line,
    };
    expr_position(code, start, end) || follows_number(code, start)
}

/// `total` on its own, or `subtotal = total`, is the block total, not a variable.
fn bare_aggregate(body: &str, word: &str) -> bool {
    let trimmed = body.trim();
    if crate::aggregate(trimmed).is_some() && trimmed.eq_ignore_ascii_case(word) {
        return true;
    }
    assignment(body).is_some_and(|(_, rhs)| {
        let rhs = rhs.trim();
        crate::aggregate(rhs).is_some() && rhs.eq_ignore_ascii_case(word)
    })
}

fn assignment(body: &str) -> Option<(&str, &str)> {
    let trimmed = body.trim();
    let eq = trimmed.find('=')?;
    if trimmed[eq + 1..].starts_with('=') {
        return None;
    }
    let name = trimmed[..eq].trim();
    if !is_ident(name) {
        return None;
    }
    Some((name, &trimmed[eq + 1..]))
}

fn expr_position(code: &str, start: usize, end: usize) -> bool {
    let before = code[..start].trim_end();
    let after = code[end..].trim_start();
    before.ends_with(|ch: char| "+-*/^=(,".contains(ch))
        || after.starts_with(|ch: char| "+-*/^=(),".contains(ch))
}

fn follows_number(code: &str, start: usize) -> bool {
    code[..start]
        .trim_end()
        .ends_with(|ch: char| ch.is_ascii_digit() || ch == ')')
}

fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn locate(bindings: &[Found], cursor: usize) -> Option<(usize, usize)> {
    let mut edge = None;
    for (bi, binding) in bindings.iter().enumerate() {
        for (oi, occ) in binding.occs.iter().enumerate() {
            if cursor >= occ.start && cursor < occ.end {
                return Some((bi, oi));
            }
            if cursor == occ.end && occ.end > occ.start {
                edge = Some((bi, oi));
            }
        }
    }
    edge
}

fn publish(binding: &Found, occ_index: usize) -> Binding {
    let (token_start, token_end) = {
        let occ = &binding.occs[occ_index];
        (occ.start, occ.end)
    };
    let definition = binding
        .occs
        .iter()
        .find(|occ| occ.def)
        .map(occ_span)
        .unwrap_or(Span {
            start: token_start,
            end: token_end,
        });
    Binding {
        name: binding.name.clone(),
        token_start,
        token_end,
        references: binding.occs.iter().filter(|occ| !occ.def).count(),
        spans: binding.occs.iter().map(occ_span).collect(),
        definition,
    }
}

fn occ_span(occ: &Occ) -> Span {
    Span {
        start: occ.start,
        end: occ.end,
    }
}

fn cursor_after(spans: &[Span], token_start: usize, new_name: &str) -> usize {
    let new_len = new_name.chars().count();
    let mut shift = 0isize;
    for span in spans {
        let start = (span.start as isize + shift) as usize;
        if span.start == token_start {
            return start + new_len;
        }
        shift += new_len as isize - (span.end - span.start) as isize;
    }
    0
}

fn char_offset_of_line(text: &str, index: usize) -> usize {
    let mut offset = 0;
    for (i, line) in text.split('\n').enumerate() {
        if i == index {
            return offset;
        }
        offset += line.chars().count() + 1;
    }
    offset
}

fn definitions(text: &str) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    let mut offset = 0;
    let lines: Vec<&str> = text.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        if let Some((from, to)) = crate::definition_span(line) {
            let start = offset + line[..from].chars().count();
            let end = offset + line[..to].chars().count();
            out.push((line[from..to].to_string(), Span { start, end }));
        }
        offset += line.chars().count();
        if i + 1 < lines.len() {
            offset += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renamed(text: &str, needle: &str, nth: usize, name: &str) -> String {
        let at = nth_name(text, needle, nth);
        rename_variable(text, at, name).unwrap().text
    }

    fn nth_name(text: &str, needle: &str, nth: usize) -> usize {
        let mut seen = 0;
        let mut offset = 0;
        for line in text.split('\n') {
            for token in identifiers(line) {
                if token.text == needle {
                    if seen == nth {
                        return offset + token.start;
                    }
                    seen += 1;
                }
            }
            offset += line.chars().count() + 1;
        }
        panic!("{needle} #{nth} not in {text:?} (saw {seen})");
    }

    #[test]
    fn a_redefinition_shadows_the_name_below_it() {
        let text = "\
rent = 1
rent + 1
rent = 9
rent + 1
rental = rent
";
        assert_eq!(
            renamed(text, "rent", 0, "fee"),
            "\
fee = 1
fee + 1
rent = 9
rent + 1
rental = rent
"
        );
        assert_eq!(
            renamed(text, "rent", 2, "cost"),
            "\
rent = 1
rent + 1
cost = 9
cost + 1
rental = cost
"
        );
    }

    #[test]
    fn the_right_hand_side_of_a_redefinition_is_the_previous_name() {
        let text = "\
kg = 1
kg = kg + 1
kg
";
        assert_eq!(
            renamed(text, "kg", 0, "grams"),
            "\
grams = 1
kg = grams + 1
kg
"
        );
        assert_eq!(
            renamed(text, "kg", 1, "mass"),
            "\
kg = 1
mass = kg + 1
mass
"
        );
    }

    #[test]
    fn a_longer_name_that_starts_the_same_way_is_left_alone() {
        let text = "\
rent = 1
rental = 2
rent + rental
rent_total = rent
";
        assert_eq!(
            renamed(text, "rent", 0, "fee"),
            "\
fee = 1
rental = 2
fee + rental
rent_total = fee
"
        );
        let at = nth_name(text, "rental", 0);
        assert_eq!(binding_at(text, at).unwrap().name, "rental");
        assert!(binding_at(text, at).unwrap().references >= 1);

        let share = "\
revenue = 1
revenue_share = 2
revenue + revenue_share
";
        assert_eq!(
            renamed(share, "revenue", 0, "sales"),
            "\
sales = 1
revenue_share = 2
sales + revenue_share
"
        );
    }

    #[test]
    fn try_and_kg_rename_only_where_they_are_the_variable() {
        let text = "\
100 Try
the Try rate
Try = 5
Try * 2
100 TRY
Lunch for Try $40
3 kg
kg = 4
3 kg
kg * 2
";
        assert!(binding_at(text, nth_name(text, "Try", 0)).is_none());
        assert!(binding_at(text, nth_name(text, "Try", 1)).is_none());
        assert_eq!(
            renamed(text, "Try", 2, "Lira"),
            "\
100 Try
the Try rate
Lira = 5
Lira * 2
100 TRY
Lunch for Try $40
3 kg
kg = 4
3 kg
kg * 2
"
        );
        assert_eq!(
            renamed(text, "kg", 1, "kilo"),
            "\
100 Try
the Try rate
Try = 5
Try * 2
100 TRY
Lunch for Try $40
3 kg
kilo = 4
3 kilo
kilo * 2
"
        );
        // The kilogram before the definition is the unit, not the variable.
        assert!(binding_at(text, nth_name(text, "kg", 0)).is_none());
        assert_eq!(
            binding_at(text, nth_name(text, "kg", 2)).unwrap().name,
            "kg"
        );
    }

    #[test]
    fn comments_quotes_and_prose_are_not_references() {
        let text = "\
rent = 1
rent * 2 # rent
// rent = 4
\"rent\" + rent
- rent * 2
- rent is due
";
        assert_eq!(
            renamed(text, "rent", 0, "fee"),
            "\
fee = 1
fee * 2 # rent
// rent = 4
\"rent\" + fee
- fee * 2
- rent is due
"
        );
    }

    #[test]
    fn a_bare_total_is_the_block_not_the_variable() {
        let text = "\
total = 5
10
20
total
subtotal = total
total * 2
";
        assert_eq!(
            renamed(text, "total", 0, "gross"),
            "\
gross = 5
10
20
total
subtotal = total
gross * 2
"
        );
    }

    #[test]
    fn a_bad_name_or_a_name_already_used_is_refused() {
        let text = "fee = 1\nrent = 2\nrent * 3\n";
        let at = nth_name(text, "rent", 0);
        assert_eq!(
            rename_variable(text, at, "fee"),
            Err(RenameFail::Collision { name: "fee".into() })
        );
        assert_eq!(
            rename_variable(text, at, "1rent"),
            Err(RenameFail::InvalidName)
        );
        assert_eq!(rename_variable(text, at, ""), Err(RenameFail::InvalidName));
        assert_eq!(rename_variable(text, at, "rent ").unwrap().text, text);
        assert_eq!(binding_at(text, 0).unwrap().name, "fee");
        assert_eq!(
            rename_variable(text, 0, "cost").unwrap().text,
            "cost = 1\nrent = 2\nrent * 3\n"
        );
        let same = rename_variable(text, at, "rent").unwrap();
        assert_eq!(same.text, text);
        assert_eq!(same.cursor, at + "rent".len());
    }

    #[test]
    fn the_cursor_lands_just_after_the_token_that_was_renamed() {
        let text = "rent = 1\nrent";
        let at = nth_name(text, "rent", 1);
        let out = rename_variable(text, at, "fee").unwrap();
        assert_eq!(out.text, "fee = 1\nfee");
        assert_eq!(out.cursor, out.text.chars().count());
        let at_def = nth_name(text, "rent", 0) + 2;
        let out = rename_variable(text, at_def, "fee").unwrap();
        assert_eq!(out.cursor, "fee".len());
    }

    #[test]
    fn editing_the_definition_offers_the_other_references() {
        let old = "rent = 1\nrent + 1\nrental = rent\n";
        let new = "fee = 1\nrent + 1\nrental = rent\n";
        let edit = definition_change(old, new).unwrap();
        assert_eq!(edit.old_name, "rent");
        assert_eq!(edit.new_name, "fee");
        assert_eq!(edit.references, 2);
        assert!(!edit.collision);
        assert_eq!(edit.message(), "Update 2 references?");
        assert_eq!(edit.apply(new), "fee = 1\nfee + 1\nrental = fee\n");

        // Typing the new name one letter at a time is still the original rename.
        let partial = definition_change(old, "fe = 1\nrent + 1\nrental = rent\n").unwrap();
        assert_eq!(partial.old_name, "rent");
        assert_eq!(partial.new_name, "fe");
        assert_eq!(partial.references, 2);

        let page = "# Northwind\n\nrent = 1,200\nrent * 2\nrental = rent + 50\n# the rent is due\n100 TRY\nfee = 9";
        let dues = page.replacen("rent =", "dues =", 1);
        let edit = definition_change(page, &dues).unwrap();
        assert_eq!(edit.new_name, "dues");
        assert_eq!(edit.references, 2);
        assert_eq!(edit.message(), "Update 2 references?");

        let only = definition_change("rent = 1\n", "fee = 1\n");
        assert!(only.is_none());

        let collided = "fee = 3\nrent = 1\nrent + 1\n";
        let edited = "fee = 3\nfee = 1\nrent + 1\n";
        let edit = definition_change(collided, edited).unwrap();
        assert!(edit.collision);
        assert_eq!(edit.message(), "fee is already used");

        assert!(definition_change(old, "fee = 2\nrent + 1\nrental = rent\n").is_none());
        assert!(definition_change(old, "fee = 1\nrent + 1\n").is_none());
    }

    #[test]
    fn one_reference_is_named_in_the_singular() {
        assert_eq!(update_label(1), "Update 1 reference?");
        assert_eq!(update_label(3), "Update 3 references?");
    }
}
