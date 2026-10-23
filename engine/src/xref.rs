//! A name from another page: `@handle.variable`.
//!
//! `@trip.total` is the final value of `total` on the page whose handle is
//! `trip` — the last assignment on that page, after the other pages have been
//! worked out. A page's own `total` is a different name; nothing crosses pages
//! unless a line writes `@handle.name`.
//!
//! `@name.var` (with a dot) is always a page reference. A bare `@name` with no
//! dot is left unparsed, so it can later be a person mention in prose.
//!
//! A locked page is a hook for password-locked pages. Nothing sets `locked`
//! until that feature stores a password; while it is set, a reference to the
//! page shows "🔒 locked" instead of the value.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use regex::Regex;
use std::sync::LazyLock;

use crate::complete::Suggestion;
use crate::markdown;

/// One page the reference resolver can see.
#[derive(Clone, Debug)]
pub struct Page {
    /// Stable id from the app, so a rename can name the page it rewrote.
    pub id: String,
    pub handle: String,
    pub title: String,
    pub text: String,
    /// Set when the page is password-locked. References do not read it.
    pub locked: bool,
    /// The page is in Recently deleted.
    pub deleted: bool,
    /// Another window has the file open, so a rename must not rewrite it.
    /// Checked at rename time; evaluation still reads `text`.
    pub elsewhere: bool,
}

impl Page {
    pub fn new(handle: &str, title: &str, text: &str) -> Self {
        Self {
            id: String::new(),
            handle: handle.to_string(),
            title: title.to_string(),
            text: text.to_string(),
            locked: false,
            deleted: false,
            elsewhere: false,
        }
    }
}

/// A `@handle.name` outside quotes and comments. Offsets are into that line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtRef {
    /// Char index of `@`.
    pub start: usize,
    /// Char index just after the variable name.
    pub end: usize,
    /// Lowercase handle.
    pub handle: String,
    pub name: String,
    pub byte_start: usize,
    pub byte_end: usize,
    /// Bytes of `@handle` (the `@` and the handle, not the dot or the name).
    pub handle_bytes: (usize, usize),
    pub name_bytes: (usize, usize),
}

/// What `prepare` did to a line before it is calculated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prepared {
    /// The line, with each reference replaced by its value.
    Ready(String),
    /// A quiet hint. The line is not calculated.
    Hint(String),
    /// A reference's page has the name, but its value is not known yet.
    /// Only while pages are being resolved; a finished book says
    /// "circular reference" instead.
    Wait,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Slot {
    Value(String),
    Hint(String),
    Pending,
    Unset,
}

#[derive(Clone, Debug)]
struct Meta {
    handle: String,
    title: String,
    text: String,
    locked: bool,
    deleted: bool,
    /// Every name assigned on the page. The last assignment is the final one.
    names: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Book {
    pages: Vec<Meta>,
    slots: HashMap<(String, String), Slot>,
    resolving: bool,
    fingerprint: u64,
    current: String,
}

struct Ctx {
    book: Book,
}

thread_local! {
    static CURRENT: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

/// Clears the installed book when dropped, including on panic, so one test
/// cannot leak a catalog into the next.
pub struct Guard {
    _private: (),
}

impl Drop for Guard {
    fn drop(&mut self) {
        CURRENT.with(|c| *c.borrow_mut() = None);
    }
}

/// Install `book` for the current thread while calculating `current`'s text.
pub fn install_for(book: &Book, current: &str) -> Guard {
    let mut book = book.clone();
    book.current = current.to_lowercase();
    CURRENT.with(|c| {
        *c.borrow_mut() = Some(Ctx { book });
    });
    Guard { _private: () }
}

pub fn install(book: &Book) -> Guard {
    install_for(book, "")
}

/// `^[a-z][a-z0-9]*$`.
pub fn valid_handle(handle: &str) -> bool {
    let mut chars = handle.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// A short unique handle for `title`: the lowercase letters and digits of the
/// first word that starts with a letter. "Trip Budget" is `trip`. The next
/// page with the same first word gets the words joined (`tripbudget`), and
/// after that `trip2`, `trip3`, …. A title with no such word is `page`.
pub fn allocate_handle(title: &str, taken: &HashSet<String>) -> String {
    let taken: HashSet<String> = taken.iter().map(|h| h.to_lowercase()).collect();
    let words: Vec<String> = title
        .split_whitespace()
        .map(slug_word)
        .filter(|w| !w.is_empty())
        .collect();
    let start = words.iter().position(|w| starts_letter(w));
    let base = match start {
        Some(i) => {
            let word = words[i].trim_start_matches(|c: char| !c.is_ascii_lowercase());
            if word.is_empty() { "page" } else { word }
        }
        None => "page",
    };
    let base = if valid_handle(base) {
        base.to_string()
    } else {
        "page".into()
    };
    if !taken.contains(&base) {
        return base;
    }
    if let Some(i) = start {
        let longer: String = words[i..].join("");
        let longer = longer.trim_start_matches(|c: char| !c.is_ascii_lowercase());
        if valid_handle(longer) && longer != base && !taken.contains(longer) {
            return longer.to_string();
        }
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}{n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn slug_word(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn starts_letter(word: &str) -> bool {
    word.chars().next().is_some_and(|c| c.is_ascii_lowercase())
}

static ASSIGN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^=].*)$").unwrap());

fn assignment_name(line: &str) -> Option<String> {
    let body = markdown::math_body(line);
    ASSIGN.captures(body).map(|c| c[1].to_string())
}

/// `@handle.name` on this line. A bare `@name` is not a reference: see the
/// module note. `mail@trip.total` is not one either; `@` has to start a token.
pub fn page_refs(line: &str) -> Vec<AtRef> {
    let comment_at = markdown::classify(line).comment_at.unwrap_or(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut byte = 0;
    let mut quoted = false;
    let mut out = Vec::new();
    while i < chars.len() {
        if byte >= comment_at {
            break;
        }
        let c = chars[i];
        if c == '"' {
            quoted = !quoted;
            byte += c.len_utf8();
            i += 1;
            continue;
        }
        if !quoted
            && c == '@'
            && let Some((end, handle, name)) = try_ref(&chars, i)
        {
            let byte_start = byte;
            let handle_bytes = (byte_start, byte_start + 1 + handle.len());
            let name_bytes = (
                byte_start + 1 + handle.len() + 1,
                byte_start + 1 + handle.len() + 1 + name.len(),
            );
            let byte_end = name_bytes.1;
            out.push(AtRef {
                start: i,
                end,
                handle,
                name,
                byte_start,
                byte_end,
                handle_bytes,
                name_bytes,
            });
            i = end;
            byte = byte_end;
            continue;
        }
        byte += c.len_utf8();
        i += 1;
    }
    out
}

fn try_ref(chars: &[char], at: usize) -> Option<(usize, String, String)> {
    if at > 0 && is_ident_char(chars[at - 1]) {
        return None;
    }
    let mut i = at + 1;
    if i >= chars.len() || !chars[i].is_ascii_alphabetic() {
        return None;
    }
    let h0 = i;
    i += 1;
    while i < chars.len() && chars[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i < chars.len() && chars[i] == '_' {
        return None;
    }
    if i >= chars.len() || chars[i] != '.' {
        return None;
    }
    let handle: String = chars[h0..i].iter().collect::<String>().to_lowercase();
    if !valid_handle(&handle) {
        return None;
    }
    i += 1;
    if i >= chars.len() || !(chars[i].is_ascii_alphabetic() || chars[i] == '_') {
        return None;
    }
    let n0 = i;
    i += 1;
    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
        i += 1;
    }
    let name: String = chars[n0..i].iter().collect();
    Some((i, handle, name))
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// True when `text` contains a dotted page reference.
pub fn contains_page_ref(text: &str) -> bool {
    text.split('\n').any(|line| !page_refs(line).is_empty())
}

/// Substitute references, or report why the line cannot be calculated yet.
pub fn prepare(line: &str) -> Prepared {
    let refs = page_refs(line);
    if refs.is_empty() {
        return Prepared::Ready(line.to_string());
    }
    let Some(found) = CURRENT.with(|c| c.borrow().as_ref().map(|ctx| lookups(&ctx.book, &refs)))
    else {
        return Prepared::Hint(format!("unknown page {}", refs[0].handle));
    };
    if let Some(hint) = found.iter().find_map(|item| match item {
        Lookup::Hint(note) => Some(note.clone()),
        _ => None,
    }) {
        return Prepared::Hint(hint);
    }
    if found.iter().any(|item| matches!(item, Lookup::Wait)) {
        return Prepared::Wait;
    }
    let mut line = line.to_string();
    for (r, item) in refs.iter().zip(found).rev() {
        let Lookup::Value(value) = item else {
            continue;
        };
        let expr = subst_expr(&value);
        line.replace_range(r.byte_start..r.byte_end, &expr);
    }
    Prepared::Ready(line)
}

fn subst_expr(display: &str) -> String {
    let chars: Vec<char> = display.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        if *c == ','
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit())
        {
            continue;
        }
        out.push(*c);
    }
    format!("({out})")
}

enum Lookup {
    Value(String),
    Hint(String),
    Wait,
}

fn lookups(book: &Book, refs: &[AtRef]) -> Vec<Lookup> {
    refs.iter()
        .map(|r| lookup(book, &r.handle, &r.name))
        .collect()
}

fn lookup(book: &Book, handle: &str, name: &str) -> Lookup {
    let handle = handle.to_lowercase();
    let Some(page) = book.pages.iter().find(|p| p.handle == handle) else {
        return Lookup::Hint(format!("unknown page {handle}"));
    };
    if page.deleted {
        return Lookup::Hint("page in Recently deleted".into());
    }
    if page.locked {
        return Lookup::Hint("🔒 locked".into());
    }
    let key = (handle, name.to_string());
    match book.slots.get(&key) {
        Some(Slot::Value(value)) => Lookup::Value(value.clone()),
        Some(Slot::Hint(note)) => Lookup::Hint(note.clone()),
        Some(Slot::Unset) => Lookup::Hint(missing(name, &page.title)),
        Some(Slot::Pending) => pending(book),
        None if page.names.iter().any(|n| n == name) => pending(book),
        None => Lookup::Hint(missing(name, &page.title)),
    }
}

fn pending(book: &Book) -> Lookup {
    if book.resolving {
        Lookup::Wait
    } else {
        Lookup::Hint("circular reference".into())
    }
}

fn missing(name: &str, title: &str) -> String {
    let title = if title.is_empty() { "that page" } else { title };
    format!("{name} isn't defined on {title}")
}

fn last_assignments(text: &str) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for (i, line) in text.split('\n').enumerate() {
        if let Some(name) = assignment_name(line) {
            map.insert(name, i);
        }
    }
    map
}

fn slot_from(line: &str, result: Option<&crate::LineResult>) -> Slot {
    let Some(result) = result else {
        return Slot::Pending;
    };
    if result.kind == crate::Kind::Value && !result.display.is_empty() {
        return Slot::Value(result.display.clone());
    }
    if result.hint || result.note.is_some() {
        return Slot::Hint(result.note.clone().unwrap_or_default());
    }
    if contains_page_ref(line) {
        Slot::Pending
    } else {
        Slot::Unset
    }
}

fn settled_changed(old: Option<&Slot>, new: &Slot) -> bool {
    match (old, new) {
        (Some(Slot::Value(a)), Slot::Value(b)) => a != b,
        (Some(Slot::Hint(a)), Slot::Hint(b)) => a != b,
        (Some(Slot::Value(_)), Slot::Hint(_)) | (Some(Slot::Hint(_)), Slot::Value(_)) => true,
        (None | Some(Slot::Pending | Slot::Unset), Slot::Value(_) | Slot::Hint(_)) => true,
        _ => false,
    }
}

/// The final value of every name on every unlocked page.
///
/// Each round calculates every page with the values known so far, and keeps
/// only the last assignment of each name. A reference whose value is not ready
/// waits. When a round changes nothing, a name that is still waiting and whose
/// last assignment line itself contains a page reference is a cycle
/// ("circular reference"). A last assignment that never referred to another
/// page and produced no value stays unset, so a lookup says the name isn't
/// defined. Chains resolve one hop per round.
pub fn resolve(engine: &crate::Engine, pages: &[Page]) -> Book {
    let mut book = Book::from_pages(pages);
    book.resolving = true;
    for _ in 0..128 {
        let guard = install(&book);
        let mut next = book.slots.clone();
        let mut progress = false;
        for page in pages {
            if page.locked || page.deleted || !valid_handle(&page.handle.to_lowercase()) {
                continue;
            }
            let handle = page.handle.to_lowercase();
            let results = engine.eval_document(&page.text);
            for (name, line_idx) in last_assignments(&page.text) {
                let line = page.text.split('\n').nth(line_idx).unwrap_or("");
                let slot = slot_from(line, results.get(line_idx));
                let key = (handle.clone(), name);
                if settled_changed(book.slots.get(&key), &slot) {
                    progress = true;
                }
                next.insert(key, slot);
            }
        }
        drop(guard);
        book.slots = next;
        if !progress {
            break;
        }
    }
    book.seal();
    book
}

impl Book {
    fn from_pages(pages: &[Page]) -> Self {
        let pages_meta = pages
            .iter()
            .map(|p| Meta {
                handle: p.handle.to_lowercase(),
                title: p.title.clone(),
                text: p.text.clone(),
                locked: p.locked,
                deleted: p.deleted,
                names: {
                    let mut names = Vec::new();
                    let mut seen = HashSet::new();
                    for line in p.text.split('\n') {
                        if let Some(name) = assignment_name(line)
                            && seen.insert(name.clone())
                        {
                            names.push(name);
                        }
                    }
                    names
                },
            })
            .collect();
        Self {
            pages: pages_meta,
            slots: HashMap::new(),
            resolving: false,
            fingerprint: fingerprint(pages),
            current: String::new(),
        }
    }

    fn seal(&mut self) {
        for slot in self.slots.values_mut() {
            if *slot == Slot::Pending {
                *slot = Slot::Hint("circular reference".into());
            }
        }
        self.resolving = false;
    }
}

/// Hash of everything a reference can read. A change means saved answers on
/// other pages are out of date.
pub fn fingerprint(pages: &[Page]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for page in pages {
        h = mix(h, &page.handle.to_lowercase());
        h = mix(h, &page.title);
        h = mix(h, &page.text);
        h = mix(h, if page.locked { "1" } else { "0" });
        h = mix(h, if page.deleted { "1" } else { "0" });
        h = h.wrapping_add(0xff);
    }
    h
}

fn mix(mut h: u64, s: &str) -> u64 {
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn current_fingerprint() -> u64 {
    CURRENT.with(|c| c.borrow().as_ref().map_or(0, |ctx| ctx.book.fingerprint))
}

/// Calculate every page with cross-page references filled in.
pub fn eval_pages(engine: &crate::Engine, pages: &[Page]) -> Vec<Vec<crate::LineResult>> {
    let book = resolve(engine, pages);
    pages
        .iter()
        .map(|page| {
            let _guard = install_for(&book, &page.handle);
            engine.eval_document(&page.text)
        })
        .collect()
}

/// The reference under `cursor` (a char index into `text`), if any.
pub fn ref_at(text: &str, cursor: usize) -> Option<AtRef> {
    let (line, line_at, col) = line_col(text, cursor)?;
    let r = page_refs(line)
        .into_iter()
        .find(|r| col >= r.start && col < r.end || (col == r.end && r.end > r.start))?;
    let mut r = r;
    r.start += line_at;
    r.end += line_at;
    Some(r)
}

pub fn hover_ref(text: &str, cursor: usize) -> Option<crate::Hover> {
    let r = ref_at(text, cursor)?;
    let (value, title) = CURRENT.with(|c| {
        let ctx = c.borrow();
        let book = ctx.as_ref()?;
        let title = book
            .book
            .pages
            .iter()
            .find(|p| p.handle == r.handle)
            .map(|p| p.title.clone())
            .filter(|t| !t.is_empty());
        let value = match lookup(&book.book, &r.handle, &r.name) {
            Lookup::Value(value) => value,
            Lookup::Hint(note) => note,
            Lookup::Wait => "circular reference".into(),
        };
        Some((value, title))
    })?;
    Some(crate::Hover {
        start: r.start,
        end: r.end,
        value,
        source_title: title,
        source_handle: Some(r.handle),
        used_by: 0,
    })
}

/// How many other pages reference `name` on the page currently being calculated.
pub fn used_by_here(name: &str) -> usize {
    CURRENT.with(|c| {
        let ctx = c.borrow();
        let Some(ctx) = ctx.as_ref() else {
            return 0;
        };
        let handle = ctx.book.current.clone();
        if handle.is_empty() {
            return 0;
        }
        ctx.book
            .pages
            .iter()
            .filter(|p| !p.deleted && p.handle != handle && page_uses(&p.text, &handle, name))
            .count()
    })
}

fn page_uses(text: &str, handle: &str, name: &str) -> bool {
    text.split('\n').any(|line| {
        page_refs(line)
            .iter()
            .any(|r| r.handle == handle && r.name == name)
    })
}

pub fn used_by(pages: &[Page], handle: &str, name: &str) -> usize {
    let handle = handle.to_lowercase();
    pages
        .iter()
        .filter(|p| {
            !p.deleted && p.handle.to_lowercase() != handle && page_uses(&p.text, &handle, name)
        })
        .count()
}

fn line_col(text: &str, cursor: usize) -> Option<(&str, usize, usize)> {
    if cursor > text.chars().count() {
        return None;
    }
    let mut line_idx = 0;
    let mut line_at = 0;
    for (i, ch) in text.chars().enumerate() {
        if i == cursor {
            break;
        }
        if ch == '\n' {
            line_idx += 1;
            line_at = i + 1;
        }
    }
    let line = text.split('\n').nth(line_idx)?;
    Some((line, line_at, cursor - line_at))
}

/// Replace `@old.` with `@new.` when `old` is the whole handle.
/// `@trip.` does not match `@trip2.`.
pub fn rewrite_handle(text: &str, old: &str, new: &str) -> (String, usize) {
    let old = old.to_lowercase();
    let new = new.to_lowercase();
    if old == new || !valid_handle(&new) {
        return (text.to_string(), 0);
    }
    let mut count = 0;
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let refs: Vec<AtRef> = page_refs(line)
            .into_iter()
            .filter(|r| r.handle == old)
            .collect();
        if refs.is_empty() {
            out.push_str(line);
            continue;
        }
        let mut cursor = 0;
        for r in refs {
            out.push_str(&line[cursor..r.handle_bytes.0]);
            out.push('@');
            out.push_str(&new);
            cursor = r.handle_bytes.1;
            count += 1;
        }
        out.push_str(&line[cursor..]);
    }
    (out, count)
}

/// Replace `@handle.old` with `@handle.new` when `old` is the whole variable name.
pub fn rewrite_var_refs(text: &str, handle: &str, old: &str, new: &str) -> (String, usize) {
    let handle = handle.to_lowercase();
    if old == new {
        return (text.to_string(), 0);
    }
    let mut count = 0;
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let refs: Vec<AtRef> = page_refs(line)
            .into_iter()
            .filter(|r| r.handle == handle && r.name == old)
            .collect();
        if refs.is_empty() {
            out.push_str(line);
            continue;
        }
        let mut cursor = 0;
        for r in refs {
            out.push_str(&line[cursor..r.name_bytes.0]);
            out.push_str(new);
            cursor = r.name_bytes.1;
            count += 1;
        }
        out.push_str(&line[cursor..]);
    }
    (out, count)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageEdit {
    pub id: String,
    pub index: usize,
    pub text: String,
    pub handle: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Propagation {
    pub edits: Vec<PageEdit>,
    pub here: usize,
    pub elsewhere: usize,
    pub skipped_locked: usize,
    pub skipped_elsewhere: usize,
    pub blurb: String,
    pub cursor: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandleFail {
    Invalid,
    Taken { handle: String },
}

impl HandleFail {
    pub fn message(&self) -> String {
        match self {
            HandleFail::Invalid => {
                "Handles start with a letter and use only letters and digits".into()
            }
            HandleFail::Taken { handle } => format!("{handle} is already used"),
        }
    }
}

pub fn rename_blurb(
    old: &str,
    new: &str,
    here: usize,
    elsewhere: usize,
    skipped_locked: usize,
    skipped_elsewhere: usize,
) -> String {
    let mut text = format!("Rename {old} → {new}");
    let mut parts = Vec::new();
    match here {
        0 => {}
        1 => parts.push("1 use here".to_string()),
        n => parts.push(format!("{n} uses here")),
    }
    match elsewhere {
        0 => {}
        1 => parts.push("1 on another page".to_string()),
        n => parts.push(format!("{n} on other pages")),
    }
    if !parts.is_empty() {
        text.push_str(" (");
        text.push_str(&parts.join(", "));
        text.push(')');
    }
    append_skipped(&mut text, skipped_locked, skipped_elsewhere);
    text
}

fn append_skipped(text: &mut String, locked: usize, elsewhere: usize) {
    let mut notes = Vec::new();
    match locked {
        0 => {}
        1 => notes.push("A locked page was left unchanged".to_string()),
        n => notes.push(format!("{n} locked pages were left unchanged")),
    }
    match elsewhere {
        0 => {}
        1 => notes.push("A page open in another window was left unchanged".to_string()),
        n => notes.push(format!(
            "{n} pages open in another window were left unchanged"
        )),
    }
    if !notes.is_empty() {
        text.push_str(". ");
        text.push_str(&notes.join(". "));
        text.push('.');
    }
}

/// Rename the variable under `cursor` on `pages[source]`, and `@handle.old` on
/// every other page that can be rewritten, as one change.
pub fn propagate_rename(
    pages: &[Page],
    source: usize,
    cursor: usize,
    new_name: &str,
) -> Result<Propagation, crate::rename::RenameFail> {
    let page = pages
        .get(source)
        .ok_or(crate::rename::RenameFail::NotAVariable)?;
    let new_name = new_name.trim();
    let renamed = crate::rename::rename_variable(&page.text, cursor, new_name)?;
    let binding = crate::rename::binding_at(&page.text, cursor)
        .ok_or(crate::rename::RenameFail::NotAVariable)?;
    let old = binding.name;
    let handle = page.handle.to_lowercase();
    let (source_text, extra) = rewrite_var_refs(&renamed.text, &handle, &old, new_name);
    let here = binding.references + extra;
    let mut edits = Vec::new();
    if source_text != page.text {
        edits.push(PageEdit {
            id: page.id.clone(),
            index: source,
            text: source_text,
            handle: handle.clone(),
        });
    }
    let mut elsewhere = 0;
    let mut skipped_locked = 0;
    let mut skipped_elsewhere = 0;
    for (index, other) in pages.iter().enumerate() {
        if index == source {
            continue;
        }
        let (text, n) = rewrite_var_refs(&other.text, &handle, &old, new_name);
        if n == 0 {
            continue;
        }
        if other.locked {
            skipped_locked += 1;
            continue;
        }
        if other.elsewhere {
            skipped_elsewhere += 1;
            continue;
        }
        elsewhere += n;
        edits.push(PageEdit {
            id: other.id.clone(),
            index,
            text,
            handle: other.handle.to_lowercase(),
        });
    }
    let blurb = rename_blurb(
        &old,
        new_name,
        here,
        elsewhere,
        skipped_locked,
        skipped_elsewhere,
    );
    Ok(Propagation {
        edits,
        here,
        elsewhere,
        skipped_locked,
        skipped_elsewhere,
        blurb,
        cursor: Some(renamed.cursor),
    })
}

/// The label for a rename that has not been applied yet. `None` when `cursor`
/// is not on a variable or the name is unchanged.
pub fn rename_preview(
    pages: &[Page],
    source: usize,
    cursor: usize,
    new_name: &str,
) -> Option<String> {
    match propagate_rename(pages, source, cursor, new_name) {
        Ok(p) if p.edits.is_empty() && p.here == 0 && p.elsewhere == 0 => None,
        Ok(p) => Some(p.blurb),
        Err(crate::rename::RenameFail::Collision { name }) => {
            Some(format!("{name} is already used"))
        }
        Err(_) => None,
    }
}

pub fn propagate_handle(
    pages: &[Page],
    source: usize,
    new_handle: &str,
) -> Result<Propagation, HandleFail> {
    let page = pages.get(source).ok_or(HandleFail::Invalid)?;
    let new_handle = new_handle.trim().to_lowercase();
    if !valid_handle(&new_handle) {
        return Err(HandleFail::Invalid);
    }
    let old = page.handle.to_lowercase();
    if pages
        .iter()
        .enumerate()
        .any(|(i, other)| i != source && other.handle.to_lowercase() == new_handle)
    {
        return Err(HandleFail::Taken { handle: new_handle });
    }
    if old == new_handle {
        return Ok(Propagation {
            edits: Vec::new(),
            here: 0,
            elsewhere: 0,
            skipped_locked: 0,
            skipped_elsewhere: 0,
            blurb: format!("Handle {old}"),
            cursor: None,
        });
    }
    let mut edits = Vec::new();
    let mut here = 0;
    let mut elsewhere = 0;
    let mut skipped_locked = 0;
    let mut skipped_elsewhere = 0;
    for (index, other) in pages.iter().enumerate() {
        let (text, n) = rewrite_handle(&other.text, &old, &new_handle);
        let handle = if index == source {
            new_handle.clone()
        } else {
            other.handle.to_lowercase()
        };
        let changed = text != other.text || handle != other.handle.to_lowercase();
        if n == 0 && !changed {
            continue;
        }
        if index != source && n > 0 && other.locked {
            skipped_locked += 1;
            continue;
        }
        if index != source && n > 0 && other.elsewhere {
            skipped_elsewhere += 1;
            continue;
        }
        if index == source {
            here += n;
        } else {
            elsewhere += n;
        }
        if changed {
            edits.push(PageEdit {
                id: other.id.clone(),
                index,
                text,
                handle,
            });
        }
    }
    let refs = here + elsewhere;
    let mut blurb = if refs == 0 {
        format!("Handle {old} → {new_handle}")
    } else if refs == 1 {
        format!("Handle {old} → {new_handle} (1 reference)")
    } else {
        format!("Handle {old} → {new_handle} ({refs} references)")
    };
    append_skipped(&mut blurb, skipped_locked, skipped_elsewhere);
    Ok(Propagation {
        edits,
        here,
        elsewhere,
        skipped_locked,
        skipped_elsewhere,
        blurb,
        cursor: None,
    })
}

/// Names on `handle`, in definition order, with the value a reference would see.
pub fn page_variables(
    engine: &crate::Engine,
    pages: &[Page],
    handle: &str,
) -> Vec<(String, String)> {
    let book = resolve(engine, pages);
    let handle = handle.to_lowercase();
    let Some(meta) = book.pages.iter().find(|p| p.handle == handle) else {
        return Vec::new();
    };
    meta.names
        .iter()
        .map(|name| {
            let detail = match lookup(&book, handle.as_str(), name) {
                Lookup::Value(value) => value,
                Lookup::Hint(note) => note,
                Lookup::Wait => "circular reference".into(),
            };
            (name.clone(), detail)
        })
        .collect()
}

/// What typing `@` should offer. `start` is the char index the suggestion replaces.
pub struct AtPick {
    pub start: usize,
    pub items: Vec<Suggestion>,
}

enum Query {
    /// `@` and a partial handle.
    Page { at: usize, query: String },
    /// `@handle.` and a partial variable name. `name_at` is the name's char index.
    Var {
        name_at: usize,
        handle: String,
        query: String,
    },
}

/// Suggestions for a page reference being typed at `cursor`.
pub fn at_picks(
    engine: &crate::Engine,
    text: &str,
    cursor: usize,
    pages: &[Page],
) -> Option<AtPick> {
    let (line, line_at, col) = line_col(text, cursor)?;
    let query = query_at(line, col)?;
    match query {
        Query::Page { at, query } => {
            let items = page_suggestions(pages, &query);
            if items.is_empty() {
                return None;
            }
            Some(AtPick {
                start: line_at + at,
                items,
            })
        }
        Query::Var {
            name_at,
            handle,
            query,
        } => {
            let vars = page_variables(engine, pages, &handle);
            let items = var_suggestions(&vars, &query);
            if items.is_empty() {
                return None;
            }
            Some(AtPick {
                start: line_at + name_at,
                items,
            })
        }
    }
}

fn query_at(line: &str, col: usize) -> Option<Query> {
    let chars: Vec<char> = line.chars().collect();
    if col > chars.len() {
        return None;
    }
    let mut i = col;
    while i > 0 && is_ident_char(chars[i - 1]) {
        i -= 1;
    }
    let name_at = i;
    if i > 0 && chars[i - 1] == '.' {
        i -= 1;
        let dot = i;
        while i > 0 && chars[i - 1].is_ascii_alphanumeric() {
            i -= 1;
        }
        if i == 0 || chars[i - 1] != '@' {
            return None;
        }
        let at = i - 1;
        if at > 0 && is_ident_char(chars[at - 1]) {
            return None;
        }
        if !code_at(line, at) {
            return None;
        }
        let handle: String = chars[at + 1..dot].iter().collect::<String>().to_lowercase();
        if !valid_handle(&handle) {
            return None;
        }
        let query: String = chars[name_at..col].iter().collect();
        return Some(Query::Var {
            name_at,
            handle,
            query,
        });
    }
    if i == 0 || chars[i - 1] != '@' {
        return None;
    }
    let at = i - 1;
    if at > 0 && is_ident_char(chars[at - 1]) {
        return None;
    }
    if !code_at(line, at) {
        return None;
    }
    let query: String = chars[at + 1..col].iter().collect();
    if query.chars().any(|c| !c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(Query::Page { at, query })
}

fn code_at(line: &str, char_at: usize) -> bool {
    let byte = line.chars().take(char_at).map(|c| c.len_utf8()).sum();
    let comment_at = markdown::classify(line).comment_at.unwrap_or(line.len());
    if byte >= comment_at {
        return false;
    }
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        if i >= byte {
            break;
        }
        if c == '"' {
            quoted = !quoted;
        }
    }
    !quoted
}

fn page_suggestions(pages: &[Page], query: &str) -> Vec<Suggestion> {
    let mut scored: Vec<(i32, String, String)> = pages
        .iter()
        .filter(|p| !p.deleted && valid_handle(&p.handle.to_lowercase()))
        .filter_map(|p| {
            let handle = p.handle.to_lowercase();
            let score = match_score(&handle, &p.title, query)?;
            Some((score, handle, p.title.clone()))
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.2.to_lowercase().cmp(&b.2.to_lowercase()))
    });
    scored.truncate(8);
    scored
        .into_iter()
        .map(|(_, handle, title)| Suggestion {
            text: format!("@{handle}."),
            kind: "page",
            detail: if title.is_empty() { handle } else { title },
        })
        .collect()
}

fn match_score(handle: &str, title: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(10);
    }
    let q = query.to_lowercase();
    let title = title.to_lowercase();
    if handle.starts_with(&q) {
        return Some(0);
    }
    if title.split_whitespace().any(|w| w.starts_with(&q)) {
        return Some(1);
    }
    if handle.contains(&q) {
        return Some(2);
    }
    if title.contains(&q) {
        return Some(3);
    }
    if is_subsequence(&q, handle) || is_subsequence(&q, &title) {
        return Some(4);
    }
    None
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut rest = hay.chars();
    needle.chars().all(|c| rest.any(|h| h == c))
}

fn var_suggestions(vars: &[(String, String)], query: &str) -> Vec<Suggestion> {
    let q = query.to_lowercase();
    let mut items: Vec<Suggestion> = vars
        .iter()
        .filter(|(name, _)| query.is_empty() || name.to_lowercase().starts_with(&q))
        .map(|(name, detail)| Suggestion {
            text: name.clone(),
            kind: "variable",
            detail: detail.clone(),
        })
        .collect();
    items.truncate(20);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Engine, Kind, Rates};

    fn engine() -> Engine {
        Engine::new(Rates::fixed(&[("USD", 1.0), ("EUR", 0.9)]))
    }

    fn page(handle: &str, title: &str, text: &str) -> Page {
        Page::new(handle, title, text)
    }

    fn line(pages: &[Page], handle: &str, index: usize) -> crate::LineResult {
        let i = pages.iter().position(|p| p.handle == handle).unwrap();
        eval_pages(&engine(), pages)[i][index].clone()
    }

    fn hint(result: &crate::LineResult) -> &str {
        assert!(result.hint, "expected a hint, got {result:?}");
        result.note.as_deref().unwrap_or("")
    }

    #[test]
    fn a_handle_is_the_first_word_then_a_longer_slug_then_a_number() {
        let mut taken = HashSet::new();
        let trip = allocate_handle("Trip Budget", &taken);
        assert_eq!(trip, "trip");
        taken.insert(trip);
        assert_eq!(allocate_handle("Trip Budget", &taken), "tripbudget");
        taken.insert("tripbudget".into());
        assert_eq!(allocate_handle("Trip Budget", &taken), "trip2");
        assert_eq!(allocate_handle("2024 Plans", &HashSet::new()), "plans");
        assert_eq!(allocate_handle("2024", &HashSet::new()), "page");
        assert_eq!(allocate_handle("Untitled", &HashSet::new()), "untitled");
        assert_eq!(allocate_handle("", &HashSet::new()), "page");
    }

    #[test]
    fn a_reference_uses_the_final_value_on_that_page() {
        let pages = vec![
            page("trip", "Trip Budget", "total = 1\ntotal = $40\n"),
            page("groceries", "Groceries", "cost = @trip.total * 2\n"),
        ];
        let cost = line(&pages, "groceries", 0);
        assert_eq!(cost.kind, Kind::Value);
        assert_eq!(cost.display, "$80");
    }

    #[test]
    fn a_forward_reference_on_the_same_page_is_the_final_value() {
        let pages = vec![page(
            "trip",
            "Trip Budget",
            "shown = @trip.total\ntotal = 4\n",
        )];
        assert_eq!(line(&pages, "trip", 0).display, "4");
        assert_eq!(line(&pages, "trip", 1).display, "4");
    }

    #[test]
    fn names_stay_on_their_own_page() {
        let pages = vec![
            page("trip", "Trip Budget", "total = 4\n"),
            page("groceries", "Groceries", "total = 9\nshown = @trip.total\n"),
        ];
        assert_eq!(line(&pages, "groceries", 0).display, "9");
        assert_eq!(line(&pages, "groceries", 1).display, "4");
    }

    #[test]
    fn a_bare_at_name_is_not_a_page_reference() {
        let pages = vec![page("trip", "Trip Budget", "note @maya\n1 + 2\n")];
        let results = eval_pages(&engine(), &pages);
        assert!(results[0][0].note.is_none());
        assert_eq!(results[0][1].display, "3");
        let refs = page_refs("hello @maya");
        assert!(refs.is_empty());
        let mailed = page_refs("mail@trip.total");
        assert!(mailed.is_empty());
    }

    #[test]
    fn a_missing_page_or_name_is_a_quiet_hint() {
        let pages = vec![
            page("trip", "Trip Budget", "seats = 10\n"),
            page(
                "groceries",
                "Groceries",
                "a = @nope.total\nb = @trip.total\n",
            ),
        ];
        assert_eq!(hint(&line(&pages, "groceries", 0)), "unknown page nope");
        assert_eq!(
            hint(&line(&pages, "groceries", 1)),
            "total isn't defined on Trip Budget"
        );
        // No catalog at all: still a hint, not a crash.
        let lone = engine().eval_document("n = @trip.total");
        assert_eq!(hint(&lone[0]), "unknown page trip");
    }

    #[test]
    fn a_deleted_page_and_a_locked_page_are_hints() {
        let mut deleted = page("trip", "Trip Budget", "total = 5\n");
        deleted.deleted = true;
        let mut locked = page("trip", "Trip Budget", "total = 5\n");
        locked.locked = true;
        let gone = vec![deleted, page("groceries", "Groceries", "n = @trip.total\n")];
        assert_eq!(
            hint(&line(&gone, "groceries", 0)),
            "page in Recently deleted"
        );
        let shut = vec![locked, page("groceries", "Groceries", "n = @trip.total\n")];
        assert_eq!(hint(&line(&shut, "groceries", 0)), "🔒 locked");
    }

    #[test]
    fn a_cycle_across_pages_is_a_circular_reference() {
        let pages = vec![
            page("trip", "Trip Budget", "a = @groceries.b\n"),
            page("groceries", "Groceries", "b = @trip.a\n"),
        ];
        assert_eq!(hint(&line(&pages, "trip", 0)), "circular reference");
        assert_eq!(hint(&line(&pages, "groceries", 0)), "circular reference");
    }

    #[test]
    fn a_name_that_points_at_itself_is_a_cycle_and_a_plain_failure_is_not() {
        let cycle = vec![page("trip", "Trip Budget", "total = @trip.total + 1\n")];
        assert_eq!(hint(&line(&cycle, "trip", 0)), "circular reference");
        let plain = vec![
            page("trip", "Trip Budget", "total = hello\n"),
            page("groceries", "Groceries", "n = @trip.total\n"),
        ];
        let note = line(&plain, "groceries", 0).note.unwrap();
        assert_ne!(note, "circular reference");
        assert!(
            note.contains("isn't defined") || note.contains("hello") || note.contains("Unknown")
        );
    }

    #[test]
    fn removing_a_variable_is_a_hint_and_putting_it_back_resolves() {
        let missing = vec![
            page("trip", "Trip Budget", "seats = 10\n"),
            page("groceries", "Groceries", "n = @trip.total\n"),
        ];
        assert_eq!(
            hint(&line(&missing, "groceries", 0)),
            "total isn't defined on Trip Budget"
        );
        let back = vec![
            page("trip", "Trip Budget", "total = 5\n"),
            page("groceries", "Groceries", "n = @trip.total\n"),
        ];
        assert_eq!(line(&back, "groceries", 0).display, "5");
    }

    #[test]
    fn renaming_a_variable_updates_references_on_other_pages() {
        let pages = vec![
            page("trip", "Trip Budget", "total = 5\nshown = total * 2\n"),
            page(
                "groceries",
                "Groceries",
                "n = @trip.total\nalso = @trip.total\n",
            ),
            page("weekend", "Weekend Plans", "total = 1\n"),
        ];
        let at = pages[0].text.find("total").unwrap();
        let done = propagate_rename(&pages, 0, at, "sum").unwrap();
        assert_eq!(done.here, 1);
        assert_eq!(done.elsewhere, 2);
        assert_eq!(
            done.blurb,
            "Rename total → sum (1 use here, 2 on other pages)"
        );
        let trip = done.edits.iter().find(|e| e.index == 0).unwrap();
        assert!(trip.text.starts_with("sum = 5\n"));
        let groceries = done.edits.iter().find(|e| e.index == 1).unwrap();
        assert_eq!(groceries.text, "n = @trip.sum\nalso = @trip.sum\n");
        assert!(done.edits.iter().all(|e| e.index != 2));
    }

    #[test]
    fn a_locked_page_and_a_page_open_elsewhere_are_left_unchanged() {
        let mut locked = page("groceries", "Groceries", "n = @trip.total\n");
        locked.locked = true;
        let mut elsewhere = page("weekend", "Weekend Plans", "n = @trip.total\n");
        elsewhere.elsewhere = true;
        let pages = vec![
            page("trip", "Trip Budget", "total = 5\n"),
            locked,
            elsewhere,
        ];
        let done = propagate_rename(&pages, 0, 0, "sum").unwrap();
        assert!(done.edits.iter().all(|e| e.index == 0));
        assert_eq!(done.skipped_locked, 1);
        assert_eq!(done.skipped_elsewhere, 1);
        assert!(done.blurb.contains("locked page"));
        assert!(done.blurb.contains("another window"));
    }

    #[test]
    fn changing_a_handle_updates_references_and_not_a_longer_handle() {
        let pages = vec![
            page("trip", "Trip Budget", "total = 5\n"),
            page(
                "groceries",
                "Groceries",
                "n = @trip.total\nm = @trip2.total\n",
            ),
            page("trip2", "Trip Two", "total = 1\n"),
        ];
        let done = propagate_handle(&pages, 0, "Journey").unwrap();
        assert_eq!(done.elsewhere, 1);
        let groceries = done.edits.iter().find(|e| e.index == 1).unwrap();
        assert_eq!(groceries.text, "n = @journey.total\nm = @trip2.total\n");
        let trip = done.edits.iter().find(|e| e.index == 0).unwrap();
        assert_eq!(trip.handle, "journey");
    }

    #[test]
    fn used_by_counts_other_pages() {
        let pages = vec![
            page("trip", "Trip Budget", "total = 5\n"),
            page("groceries", "Groceries", "n = @trip.total\n"),
            page("weekend", "Weekend Plans", "m = @trip.total\n"),
        ];
        assert_eq!(used_by(&pages, "trip", "total"), 2);
        assert_eq!(used_by(&pages, "trip", "seats"), 0);
    }

    #[test]
    fn typing_at_offers_pages_then_their_variables() {
        let pages = vec![
            page("trip", "Trip Budget", "total = $40\nseats = 10\n"),
            page("groceries", "Groceries", ""),
        ];
        let text = "@tr";
        let pick = at_picks(&engine(), text, text.chars().count(), &pages).unwrap();
        assert_eq!(pick.items[0].text, "@trip.");
        assert_eq!(pick.items[0].detail, "Trip Budget");
        let text = "@trip.";
        let pick = at_picks(&engine(), text, text.chars().count(), &pages).unwrap();
        assert_eq!(pick.start, "@trip.".chars().count());
        assert_eq!(pick.items[0].text, "total");
        assert_eq!(pick.items[0].detail, "$40");
        assert!(pick.items.iter().any(|item| item.text == "seats"));
    }

    #[test]
    fn a_comment_and_a_quote_do_not_read_a_reference() {
        let pages = vec![
            page("trip", "Trip Budget", "total = 5\n"),
            page(
                "groceries",
                "Groceries",
                "cost = 1 # @trip.missing\nnote = \"@trip.missing\"\n",
            ),
        ];
        assert_eq!(line(&pages, "groceries", 0).display, "1");
        assert!(line(&pages, "groceries", 1).note.is_none() || !line(&pages, "groceries", 1).hint);
    }
}
