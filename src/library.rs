//! Page operations shared by the command line and the MCP server.
//! Both read and write the same files as the desktop window.

use std::path::{Path, PathBuf};

use omacus_engine::{Engine, Kind, LineResult};
use serde::Serialize;

use crate::lock;
use crate::pdf;
use crate::settings;
use crate::sheets::{self, Sheet, Store};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PageInfo {
    pub id: String,
    pub title: String,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EvalLine {
    pub text: String,
    pub answer: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PageView {
    pub id: String,
    pub title: String,
    pub text: String,
    pub lines: Vec<EvalLine>,
    /// The page with each answer lined up, as the window shows it.
    pub rendered: String,
}

pub struct Library {
    store: Store,
}

impl Library {
    pub fn open() -> Self {
        Self {
            store: Store::open_library(),
        }
    }

    #[cfg(test)]
    pub fn open_at(root: PathBuf) -> Self {
        Self {
            store: Store::open_at(root),
        }
    }

    pub fn list(&self) -> Vec<PageInfo> {
        self.store.sheets.iter().map(|s| self.info(s)).collect()
    }

    pub fn show(&self, name: &str, password: Option<&str>) -> Result<PageView, String> {
        let idx = self.index(name, password)?;
        let sheet = &self.store.sheets[idx];
        let text = lock::reveal(&sheet.id, sheet.stored(), password)?;
        Ok(view_text(&sheet.id, &text))
    }

    pub fn create(&mut self, name: &str, body: Option<&str>) -> Result<PageInfo, String> {
        let name = clean_name(name)?;
        self.ensure_dir()?;
        let text = page_text(&name, body);
        // At the top of the root, even when the window's open page sits in a folder.
        let known: Vec<String> = self.store.sheets.iter().map(|s| s.id.clone()).collect();
        self.store.new_sheet_in(None, &text);
        self.store
            .save()
            .map_err(|e| format!("couldn't save: {e}"))?;
        self.store
            .sheets
            .iter()
            .find(|s| !known.iter().any(|old| old == &s.id))
            .map(|s| PageInfo {
                id: s.id.clone(),
                title: s.heading(),
                locked: false,
            })
            .ok_or_else(|| "couldn't save the new page".to_string())
    }

    pub fn append(
        &mut self,
        name: &str,
        extra: &str,
        password: Option<&str>,
    ) -> Result<PageInfo, String> {
        let idx = self.prepare(name, password)?;
        let text = append_text(&self.store.sheets[idx].text, extra);
        self.store.sheets[idx].text = text;
        self.store.sheets[idx].mark_dirty();
        self.store
            .save()
            .map_err(|e| format!("couldn't save: {e}"))?;
        Ok(self.info(&self.store.sheets[idx]))
    }

    pub fn update(
        &mut self,
        name: &str,
        text: &str,
        password: Option<&str>,
    ) -> Result<PageInfo, String> {
        let idx = self.prepare(name, password)?;
        let mut text = text.replace("\r\n", "\n").replace('\r', "\n");
        if !text.ends_with('\n') {
            text.push('\n');
        }
        self.store.sheets[idx].text = text;
        self.store.sheets[idx].mark_dirty();
        self.store
            .save()
            .map_err(|e| format!("couldn't save: {e}"))?;
        Ok(self.info(&self.store.sheets[idx]))
    }

    /// Move a page to Recently deleted.
    pub fn remove(&mut self, name: &str, password: Option<&str>) -> Result<String, String> {
        let idx = self.prepare(name, password)?;
        let title = self.store.sheets[idx].heading();
        self.store.delete(idx);
        self.store
            .save()
            .map_err(|e| format!("couldn't save: {e}"))?;
        Ok(title)
    }

    pub fn export_md(&self, name: &str, password: Option<&str>) -> Result<String, String> {
        let view = self.show(name, password)?;
        let results = line_results(&view.text);
        Ok(omacus_engine::markdown_table(&view.text, &results))
    }

    /// Write a PDF and return its path.
    pub fn export_pdf(
        &self,
        name: &str,
        out: Option<&Path>,
        replace: bool,
        password: Option<&str>,
    ) -> Result<PathBuf, String> {
        let view = self.show(name, password)?;
        let settings = settings::load();
        let path = match out {
            Some(path) if path.is_dir() => pdf::export_path(path, &view.title),
            Some(path) => path.to_path_buf(),
            None => {
                let dir = settings
                    .export_dir
                    .clone()
                    .unwrap_or_else(pdf::default_export_dir);
                pdf::export_path(&dir, &view.title)
            }
        };
        if path.exists() && !replace && !settings.replace_exports {
            return Err(format!(
                "{} already exists. Pass --replace to overwrite it.",
                path.display()
            ));
        }
        let results = line_results(&view.text);
        let answers: Vec<Option<&str>> = results
            .iter()
            .map(|r| {
                if r.display.is_empty() {
                    None
                } else {
                    Some(r.display.as_str())
                }
            })
            .collect();
        let blocks = omacus_engine::markdown::render(&view.text, &answers);
        pdf::write(&view.title, &blocks, &path, !settings.hide_answers)?;
        Ok(path)
    }

    fn info(&self, sheet: &Sheet) -> PageInfo {
        PageInfo {
            id: sheet.id.clone(),
            title: sheet.heading(),
            locked: sheet.locked() || lock::is_locked(sheet.stored()),
        }
    }

    fn index(&self, name: &str, password: Option<&str>) -> Result<usize, String> {
        match resolve(&self.store.sheets, name) {
            Ok(idx) => Ok(idx),
            Err(err) if err.starts_with("no page named") => {
                self.locked_index(name, password).unwrap_or(Err(err))
            }
            Err(err) => Err(err),
        }
    }

    /// A locked page's title is hidden. With the password, match the real one.
    fn locked_index(&self, name: &str, password: Option<&str>) -> Option<Result<usize, String>> {
        let password = password.filter(|p| !p.is_empty())?;
        let needle = fold(name.trim());
        if needle.is_empty() {
            return None;
        }
        let mut hits: Vec<(u8, usize)> = Vec::new();
        for (idx, sheet) in self.store.sheets.iter().enumerate() {
            if !sheet.locked() {
                continue;
            }
            let Ok(text) = lock::reveal(&sheet.id, sheet.stored(), Some(password)) else {
                continue;
            };
            let title = fold(&sheets::heading_of(&text));
            let rank = if title == needle {
                0
            } else if title.starts_with(&needle) {
                1
            } else if title.contains(&needle) {
                2
            } else if subsequence(&needle, &title) {
                3
            } else {
                continue;
            };
            hits.push((rank, idx));
        }
        if hits.is_empty() {
            return None;
        }
        hits.sort_by_key(|(rank, idx)| (*rank, *idx));
        let best = hits[0].0;
        let top: Vec<usize> = hits
            .into_iter()
            .filter(|(rank, _)| *rank == best)
            .map(|(_, idx)| idx)
            .collect();
        match top.as_slice() {
            [one] => Some(Ok(*one)),
            many => Some(Err(ambiguous(name, &self.store.sheets, many))),
        }
    }

    /// Decrypt into this session so a later save writes ciphertext, not the edit.
    fn prepare(&mut self, name: &str, password: Option<&str>) -> Result<usize, String> {
        let idx = self.index(name, password)?;
        let id = self.store.sheets[idx].id.clone();
        if self.store.sheets[idx].locked() && !self.store.sheets[idx].revealed() {
            let Some(password) = password.filter(|p| !p.is_empty()) else {
                return Err(lock::refuse(false));
            };
            self.store
                .unlock_page(&id, password)
                .map_err(|err| match err.as_str() {
                    "Wrong password." => lock::refuse(true),
                    other => other.to_string(),
                })?;
        }
        self.store
            .sheets
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| "That page is gone.".to_string())
    }

    fn ensure_dir(&self) -> Result<(), String> {
        if self.store.directory().is_some() {
            Ok(())
        } else {
            Err("no folder for pages".into())
        }
    }
}

fn view_text(id: &str, text: &str) -> PageView {
    let results = line_results(text);
    PageView {
        id: id.to_string(),
        title: sheets::heading_of(text),
        rendered: omacus_engine::with_answers(text, &results),
        lines: to_lines(text, &results),
        text: text.to_string(),
    }
}

/// The page named `query`: an id, a title, or a unique prefix or fragment of one.
pub fn resolve(sheets: &[Sheet], query: &str) -> Result<usize, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("name a page".into());
    }
    if let Some(idx) = sheets.iter().position(|s| s.id == query) {
        return Ok(idx);
    }
    let needle = fold(query);
    let titles: Vec<(usize, String)> = sheets
        .iter()
        .enumerate()
        .map(|(i, s)| (i, fold(&s.heading())))
        .collect();
    let pick = |hits: Vec<usize>| -> Result<usize, String> {
        match hits.as_slice() {
            [one] => Ok(*one),
            [] => Err(format!("no page named '{query}'")),
            _ => Err(ambiguous(query, sheets, &hits)),
        }
    };
    let exact: Vec<usize> = titles
        .iter()
        .filter(|(_, title)| title == &needle)
        .map(|(i, _)| *i)
        .collect();
    if !exact.is_empty() {
        return pick(exact);
    }
    let prefix: Vec<usize> = titles
        .iter()
        .filter(|(_, title)| title.starts_with(&needle))
        .map(|(i, _)| *i)
        .collect();
    if !prefix.is_empty() {
        return pick(prefix);
    }
    let inside: Vec<usize> = titles
        .iter()
        .filter(|(_, title)| title.contains(&needle))
        .map(|(i, _)| *i)
        .collect();
    if !inside.is_empty() {
        return pick(inside);
    }
    let fuzzy: Vec<usize> = titles
        .iter()
        .filter(|(_, title)| subsequence(&needle, title))
        .map(|(i, _)| *i)
        .collect();
    pick(fuzzy)
}

fn ambiguous(query: &str, sheets: &[Sheet], hits: &[usize]) -> String {
    let mut msg = format!("'{query}' matches more than one page:");
    for idx in hits {
        let sheet = &sheets[*idx];
        msg.push_str(&format!("\n  {}  {}", sheet.id, sheet.heading()));
    }
    msg
}

fn fold(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

fn subsequence(needle: &str, hay: &str) -> bool {
    let mut rest = hay.chars();
    needle.chars().all(|c| rest.any(|h| h == c))
}

fn clean_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.contains(['\n', '\r']) {
        return Err("a page needs a name".into());
    }
    Ok(name.to_string())
}

fn page_text(name: &str, body: Option<&str>) -> String {
    let Some(body) = body else {
        return format!("# {name}\n");
    };
    let body = body.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let heading = sheets::heading_of(&body);
    if !body.trim().is_empty() && heading.eq_ignore_ascii_case(name) && heading != "Untitled" {
        let mut text = body.trim_end().to_string();
        text.push('\n');
        return text;
    }
    let rest = body.trim_start_matches('\n');
    let mut text = format!("# {name}\n{rest}");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

fn append_text(existing: &str, extra: &str) -> String {
    let extra = extra.trim_end_matches(['\n', '\r']);
    if extra.is_empty() {
        return existing.to_string();
    }
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(extra);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn evaluate(text: &str) -> Vec<EvalLine> {
    let text = normalize_eval(text);
    let results = engine().eval_document(&text);
    to_lines(&text, &results)
}

/// 0 when a page was evaluated, 1 when a single expression has no answer.
pub fn eval_status(text: &str, lines: &[EvalLine]) -> i32 {
    let one = !normalize_eval(text).contains('\n');
    if one
        && !lines
            .iter()
            .any(|l| l.kind == "value" && !l.answer.is_empty())
    {
        1
    } else {
        0
    }
}

fn normalize_eval(text: &str) -> String {
    text.trim_end_matches(['\n', '\r'])
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn line_results(text: &str) -> Vec<LineResult> {
    engine().eval_document(text)
}

fn to_lines(text: &str, results: &[LineResult]) -> Vec<EvalLine> {
    text.split('\n')
        .zip(results)
        .map(|(line, result)| EvalLine {
            text: line.to_string(),
            answer: result.display.clone(),
            kind: kind_name(&result.kind).to_string(),
            note: result.note.clone(),
        })
        .collect()
}

fn kind_name(kind: &Kind) -> &'static str {
    match kind {
        Kind::Empty => "empty",
        Kind::Header => "header",
        Kind::Comment => "comment",
        Kind::Value => "value",
        Kind::Error => "error",
    }
}

fn engine() -> &'static Engine {
    use std::sync::OnceLock;
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        // Unit tests, and any run with OMACUS_OFFLINE, skip the rate refresh so a
        // stalled network cannot hold the process. Currency still works from the
        // fixed table below; the desktop app and a normal command do not set this.
        let table = if cfg!(test) || std::env::var_os("OMACUS_OFFLINE").is_some() {
            omacus_engine::Rates::fixed(&[("USD", 1.0), ("EUR", 0.92)])
        } else {
            crate::rates::Rates::load_blocking().table()
        };
        Engine::new(table).with_clock(omacus_engine::monotonic_ms)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("omacus-lib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn eval_prints_each_lines_answer() {
        let lines = evaluate("a = 5\na * 3\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].answer, "5");
        assert_eq!(lines[1].answer, "15");
        assert_eq!(eval_status("a = 5\na * 3\n", &lines), 0);
    }

    #[test]
    fn a_single_expression_with_no_answer_fails() {
        let lines = evaluate("hello");
        assert_eq!(eval_status("hello", &lines), 1);
        assert!(lines[0].answer.is_empty());
    }

    #[test]
    fn miles_are_the_documented_answer() {
        let lines = evaluate("3.5 km in miles");
        assert_eq!(lines[0].answer, "2.17 miles");
    }

    #[test]
    fn names_match_loosely_until_two_pages_fit() {
        let root = temp_root("fuzzy");
        let mut lib = Library::open_at(root.clone());
        lib.create("Trip Budget", None).unwrap();
        lib.create("Groceries", None).unwrap();
        lib.create("Grocery run", None).unwrap();
        assert_eq!(lib.show("trip", None).unwrap().title, "Trip Budget");
        let err = lib.show("gro", None).unwrap_err();
        assert!(err.contains("more than one page"), "{err}");
        assert!(err.contains("Groceries"), "{err}");
        assert!(err.contains("Grocery run"), "{err}");
        let id = lib.show("Trip Budget", None).unwrap().id;
        assert_eq!(lib.show(&id, None).unwrap().title, "Trip Budget");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn append_is_there_for_the_next_open() {
        let root = temp_root("append");
        let mut lib = Library::open_at(root.clone());
        lib.create("Groceries", None).unwrap();
        lib.append("Groceries", "milk $3", None).unwrap();
        let again = Library::open_at(root.clone());
        let view = again.show("groc", None).unwrap();
        assert!(view.text.contains("milk $3"), "{}", view.text);
        assert!(view.rendered.contains("milk"), "{}", view.rendered);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_locked_page_is_not_printed_or_changed() {
        let root = temp_root("lock");
        let mut window = Store::open_at(root.clone());
        window.new_sheet("# Weekend Plans\ncabin $400\n");
        let id = window.sheets[0].id.clone();
        window.lock_page(&id, "open-sesame").unwrap();
        window.save().unwrap();
        let path = root.join("pages").join(format!("{id}.txt"));
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.starts_with("OMACUS-LOCK/1\n"), "{file}");
        assert!(!file.contains("cabin $400"), "{file}");
        assert!(!file.contains("Weekend"), "{file}");
        let state = std::fs::read_to_string(root.join("state.json")).unwrap();
        assert!(!state.contains("open-sesame"), "{state}");
        assert!(!state.contains("cabin"), "{state}");
        assert!(!state.contains("\"hash\""), "{state}");

        let lib = Library::open_at(root.clone());
        let listed = lib.list();
        assert!(
            listed
                .iter()
                .any(|p| p.id == id && p.locked && p.title == sheets::LOCKED_TITLE),
            "{listed:?}"
        );
        let err = lib.show(&id, None).unwrap_err();
        assert!(err.contains("locked"), "{err}");
        assert!(!err.contains("cabin"), "{err}");
        let err = lib.show(&id, Some("nope")).unwrap_err();
        assert!(err.contains("locked"), "{err}");
        assert!(!err.contains("cabin"), "{err}");
        let named = lib.show("Weekend", None).unwrap_err();
        assert!(!named.contains("cabin"), "{named}");
        let view = lib.show("Weekend", Some("open-sesame")).unwrap();
        assert!(view.text.contains("cabin $400"), "{}", view.text);

        let mut lib = Library::open_at(root.clone());
        assert!(lib.append(&id, "hike $12", None).is_err());
        assert!(lib.remove(&id, None).is_err());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("OMACUS-LOCK/1\n"), "{text}");
        assert!(
            !text.contains("cabin $400") && !text.contains("hike $12"),
            "{text}"
        );
        assert!(path.is_file());

        let mut lib = Library::open_at(root.clone());
        lib.append("Weekend", "hike $12", Some("open-sesame"))
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("OMACUS-LOCK/1\n"), "{text}");
        assert!(
            !text.contains("hike $12") && !text.contains("cabin"),
            "{text}"
        );
        let view = Library::open_at(root.clone())
            .show(&id, Some("open-sesame"))
            .unwrap();
        assert!(view.text.contains("hike $12"), "{}", view.text);
        assert!(view.text.contains("cabin $400"), "{}", view.text);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_reference_to_a_locked_page_does_not_read_it() {
        let root = temp_root("lock-ref");
        let mut window = Store::open_at(root.clone());
        window.new_sheet("# Trip Budget\ntotal = 40\n");
        let id = window.sheets[0].id.clone();
        let handle = window.sheets[0].handle.clone();
        window.lock_page(&id, "correct horse").unwrap();
        window.lock_now();
        let sheet = window.sheets.iter().find(|s| s.id == id).unwrap();
        let locked = window.as_reference(sheet, false, false);
        assert_eq!(locked.title, sheets::LOCKED_TITLE);
        assert!(locked.locked);
        assert!(locked.text.is_empty());
        let other = omacus_engine::xref::Page::new(
            "groceries",
            "Groceries",
            &format!("n = @{handle}.total\n"),
        );
        let pages = vec![locked, other];
        let results = omacus_engine::xref::eval_pages(super::engine(), &pages);
        let line = &results[1][0];
        assert!(!line.display.contains("40"), "{line:?}");
        let note = line.note.as_deref().unwrap_or("");
        assert!(note.contains("locked"), "{note}");
        assert!(!note.contains("40"), "{note}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_password_opens_its_page_and_the_file_stays_ciphertext() {
        let root = temp_root("folder-lock");
        let mut window = Store::open_at(root.clone());
        window.new_sheet("# Groceries\nmilk $3\n");
        let page = window.sheets[0].id.clone();
        let folder = window.create_folder(None).unwrap();
        assert!(window.rename_folder(&folder, "Trip Budget"));
        assert!(window.place(&page, sheets::Place::Into(folder.clone())));
        window.lock_folder(&folder, "folder-secret").unwrap();
        window.save().unwrap();
        let file = std::fs::read_to_string(root.join("pages").join(format!("{page}.txt"))).unwrap();
        assert!(file.starts_with("OMACUS-LOCK/1\n"), "{file}");
        assert!(
            !file.contains("milk") && !file.contains("Groceries"),
            "{file}"
        );
        let state = std::fs::read_to_string(root.join("state.json")).unwrap();
        assert!(!state.contains("folder-secret"), "{state}");
        assert!(!state.contains("milk"), "{state}");

        let lib = Library::open_at(root.clone());
        let err = lib.show(&page, None).unwrap_err();
        assert!(err.contains("locked") && !err.contains("milk"), "{err}");
        let view = lib.show("Groc", Some("folder-secret")).unwrap();
        assert!(view.text.contains("milk $3"), "{}", view.text);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn removing_a_page_puts_it_in_recently_deleted() {
        let root = temp_root("rm");
        let mut lib = Library::open_at(root.clone());
        lib.create("Trip Budget", Some("train $40\n")).unwrap();
        lib.create("Groceries", None).unwrap();
        let title = lib.remove("trip", None).unwrap();
        assert_eq!(title, "Trip Budget");
        assert!(lib.show("trip", None).is_err());
        let trash = std::fs::read_dir(root.join("trash")).unwrap().count();
        assert_eq!(trash, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn another_window_sees_a_command_line_edit() {
        let root = temp_root("sync");
        let mut window = Store::open_at(root.clone());
        window.new_sheet("# Groceries\napples $2\n");
        window.save().unwrap();
        // Some clocks only move by whole seconds, so the next write must look newer.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let mut lib = Library::open_at(root.clone());
        lib.append("Groceries", "bread $4", None).unwrap();
        let synced = window.sync();
        assert!(synced.active_changed, "{synced:?}");
        assert!(
            window.sheets[0].text.contains("bread $4"),
            "{}",
            window.sheets[0].text
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn markdown_and_pdf_export_the_page() {
        let root = temp_root("export");
        let mut lib = Library::open_at(root.clone());
        lib.create("Trip Budget", Some("train $40\n")).unwrap();
        let md = lib.export_md("trip", None).unwrap();
        assert!(md.contains("Trip Budget"), "{md}");
        assert!(md.contains("train"), "{md}");
        assert!(md.contains("40"), "{md}");
        let path = root.join("out").join("trip.pdf");
        let written = lib.export_pdf("trip", Some(&path), false, None).unwrap();
        assert_eq!(written, path);
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"%PDF"), "not a pdf");
        assert!(lib.export_pdf("trip", Some(&path), false, None).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
