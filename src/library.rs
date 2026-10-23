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
        let sheet = self.sheet(name)?;
        self.unlocked(sheet, password)?;
        Ok(self.view(sheet))
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
        let idx = self.index(name)?;
        self.unlocked(&self.store.sheets[idx], password)?;
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
        let idx = self.index(name)?;
        self.unlocked(&self.store.sheets[idx], password)?;
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
        let idx = self.index(name)?;
        let title = self.store.sheets[idx].heading();
        self.unlocked(&self.store.sheets[idx], password)?;
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
            locked: self
                .store
                .directory()
                .is_some_and(|dir| lock::is_locked(dir, &sheet.id)),
        }
    }

    fn view(&self, sheet: &Sheet) -> PageView {
        let results = line_results(&sheet.text);
        PageView {
            id: sheet.id.clone(),
            title: sheet.heading(),
            rendered: omacus_engine::with_answers(&sheet.text, &results),
            lines: to_lines(&sheet.text, &results),
            text: sheet.text.clone(),
        }
    }

    fn sheet(&self, name: &str) -> Result<&Sheet, String> {
        let idx = self.index(name)?;
        Ok(&self.store.sheets[idx])
    }

    fn index(&self, name: &str) -> Result<usize, String> {
        resolve(&self.store.sheets, name)
    }

    fn unlocked(&self, sheet: &Sheet, password: Option<&str>) -> Result<(), String> {
        let Some(dir) = self.store.directory() else {
            return Ok(());
        };
        match lock::refusal(dir, &sheet.id, &sheet.text, password) {
            Some(message) => Err(message),
            None => Ok(()),
        }
    }

    fn ensure_dir(&self) -> Result<(), String> {
        if self.store.directory().is_some() {
            Ok(())
        } else {
            Err("no folder for pages".into())
        }
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
        let mut lib = Library::open_at(root.clone());
        let info = lib.create("Weekend Plans", Some("cabin $400\n")).unwrap();
        let secret = "cabin $400";
        assert!(lib.show("Weekend", None).unwrap().text.contains(secret));
        let hash = lock::password_hash("salt", "open-sesame");
        let state = format!(
            r#"{{"locks":{{"{}":{{"salt":"salt","hash":"{hash}"}}}}}}"#,
            info.id
        );
        std::fs::write(root.join("state.json"), state).unwrap();

        let lib = Library::open_at(root.clone());
        let err = lib.show("Weekend", None).unwrap_err();
        assert!(!err.contains(secret), "{err}");
        assert!(err.contains("locked"), "{err}");
        let err = lib.show("Weekend", Some("nope")).unwrap_err();
        assert!(!err.contains(secret), "{err}");
        let view = lib.show("Weekend", Some("open-sesame")).unwrap();
        assert!(view.text.contains(secret));

        let mut lib = Library::open_at(root.clone());
        assert!(lib.append("Weekend", "extra", None).is_err());
        let text =
            std::fs::read_to_string(root.join("pages").join(format!("{}.txt", info.id))).unwrap();
        assert!(!text.contains("extra"), "{text}");
        assert!(lib.remove("Weekend", None).is_err());
        assert!(
            root.join("pages")
                .join(format!("{}.txt", info.id))
                .is_file()
        );
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
