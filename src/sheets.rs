//! Sheets live as plain text files in `~/.local/share/omacus/sheets/`, Sublime-style:
//! every edit is saved, nothing asks to be saved, and closing the window loses nothing.
//! `state.json` remembers order, the active sheet, cursors and the sidebar.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub struct Sheet {
    pub id: String,
    pub text: String,
    pub cursor: usize,
    dirty: bool,
}

impl Sheet {
    pub fn title(&self) -> String {
        let line = self
            .text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let title = line.trim_start_matches(['#', '/', ' ']).trim();
        if title.is_empty() {
            return "Untitled".into();
        }
        let mut t: String = title.chars().take(32).collect();
        if title.chars().count() > 32 {
            t.push('…');
        }
        t
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    order: Vec<String>,
    active: String,
    cursors: HashMap<String, usize>,
    sidebar: Option<bool>,
}

pub struct Store {
    dir: Option<PathBuf>,
    pub sheets: Vec<Sheet>,
    pub active: usize,
    pub sidebar: bool,
    state_dirty: bool,
    /// The last deleted sheet and where it was, so it can be put back.
    deleted: Option<(usize, Sheet)>,
}

impl Store {
    pub fn load(welcome: &str) -> Self {
        Self::load_in(dirs::data_dir().map(|d| d.join("omacus")), welcome)
    }

    fn load_in(root: Option<PathBuf>, welcome: &str) -> Self {
        let dir = root.as_ref().map(|r| r.join("sheets"));
        let state: State = root
            .as_ref()
            .and_then(|r| std::fs::read(r.join("state.json")).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();

        let mut sheets = Vec::new();
        if let Some(dir) = &dir {
            let _ = std::fs::create_dir_all(dir);
            if let Some(root) = &root {
                migrate_single_sheet(root, dir);
            }
            let mut found: Vec<(SystemTime, String)> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    let id = name.strip_suffix(".txt")?.to_string();
                    let modified = e.metadata().and_then(|m| m.modified()).ok()?;
                    Some((modified, id))
                })
                .collect();
            // Saved order first, then anything new on disk, newest first.
            found.sort_by_key(|f| std::cmp::Reverse(f.0));
            let rank = |id: &str| {
                state
                    .order
                    .iter()
                    .position(|o| o == id)
                    .unwrap_or(usize::MAX)
            };
            found.sort_by_key(|(_, id)| rank(id));

            for (_, id) in found {
                if let Ok(text) = std::fs::read_to_string(dir.join(format!("{id}.txt"))) {
                    let cursor = state
                        .cursors
                        .get(&id)
                        .copied()
                        .unwrap_or(text.chars().count());
                    sheets.push(Sheet {
                        id,
                        text,
                        cursor,
                        dirty: false,
                    });
                }
            }
        }

        let mut store = Self {
            dir,
            active: 0,
            sidebar: state.sidebar.unwrap_or(true),
            sheets,
            state_dirty: false,
            deleted: None,
        };
        if store.sheets.is_empty() {
            store.new_sheet(welcome);
        }
        store.active = store
            .sheets
            .iter()
            .position(|s| s.id == state.active)
            .unwrap_or(0);
        store
    }

    pub fn current(&mut self) -> &mut Sheet {
        &mut self.sheets[self.active]
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.sheets.len() && idx != self.active {
            self.active = idx;
            self.state_dirty = true;
        }
    }

    /// An id no sheet in memory or on disk has, so a new sheet never overwrites another.
    fn unused_id(&self) -> String {
        loop {
            let id = new_id();
            let on_disk = self
                .dir
                .as_ref()
                .is_some_and(|d| d.join(format!("{id}.txt")).exists());
            if !on_disk && !self.sheets.iter().any(|s| s.id == id) {
                return id;
            }
        }
    }

    /// New sheets go to the top of the list, like a notes app.
    pub fn new_sheet(&mut self, text: &str) {
        let sheet = Sheet {
            id: self.unused_id(),
            text: text.to_string(),
            cursor: text.chars().count(),
            dirty: true,
        };
        self.sheets.insert(0, sheet);
        self.active = 0;
        self.state_dirty = true;
    }

    /// Deleted sheets are moved to `trash/`, never destroyed.
    pub fn delete(&mut self, idx: usize) {
        if idx >= self.sheets.len() {
            return;
        }
        let sheet = self.sheets.remove(idx);
        if let Some(dir) = &self.dir {
            // The trash gets the text as it is now, unsaved edits included.
            let trash = dir.with_file_name("trash");
            let _ = std::fs::create_dir_all(&trash);
            if write_atomic(
                &trash.join(format!("{}.txt", sheet.id)),
                sheet.text.as_bytes(),
            )
            .is_ok()
            {
                let _ = std::fs::remove_file(dir.join(format!("{}.txt", sheet.id)));
            }
        }
        self.deleted = Some((idx, sheet));
        if self.sheets.is_empty() {
            self.new_sheet("");
        }
        if self.active > idx || self.active >= self.sheets.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.state_dirty = true;
    }

    pub fn move_active(&mut self, step: isize) {
        let to = self.active as isize + step;
        if to >= 0 && (to as usize) < self.sheets.len() {
            self.sheets.swap(self.active, to as usize);
            self.active = to as usize;
            self.state_dirty = true;
        }
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar = !self.sidebar;
        self.state_dirty = true;
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        let sheet = &mut self.sheets[self.active];
        if sheet.cursor != cursor {
            sheet.cursor = cursor;
            self.state_dirty = true;
        }
    }

    pub fn has_unsaved(&self) -> bool {
        self.state_dirty || self.sheets.iter().any(|s| s.dirty)
    }

    /// Put back the sheet deleted last, where it was. Returns its title.
    pub fn undo_delete(&mut self) -> Option<String> {
        let (idx, mut sheet) = self.deleted.take()?;
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_file(
                dir.with_file_name("trash")
                    .join(format!("{}.txt", sheet.id)),
            );
        }
        sheet.dirty = true;
        let title = sheet.title();
        // A deleted last sheet was replaced by an empty one; drop that again.
        if self.sheets.len() == 1 && self.sheets[0].text.is_empty() {
            self.sheets.clear();
        }
        let idx = idx.min(self.sheets.len());
        self.sheets.insert(idx, sheet);
        self.active = idx;
        self.state_dirty = true;
        Some(title)
    }

    /// Write changed sheets and the state. Keeps going past a failure, and returns
    /// the first error so it can be shown.
    pub fn save(&mut self) -> Result<(), String> {
        let Some(dir) = self.dir.clone() else {
            return Ok(());
        };
        let mut first_error = None;
        for sheet in self.sheets.iter_mut().filter(|s| s.dirty) {
            match write_atomic(
                &dir.join(format!("{}.txt", sheet.id)),
                sheet.text.as_bytes(),
            ) {
                Ok(()) => sheet.dirty = false,
                Err(e) => {
                    first_error.get_or_insert(format!("{}: {e}", sheet.title()));
                }
            }
        }
        let state = State {
            order: self.sheets.iter().map(|s| s.id.clone()).collect(),
            active: self.sheets[self.active].id.clone(),
            cursors: self
                .sheets
                .iter()
                .map(|s| (s.id.clone(), s.cursor))
                .collect(),
            sidebar: Some(self.sidebar),
        };
        if let Ok(json) = serde_json::to_vec_pretty(&state)
            && let Err(e) = write_atomic(&dir.with_file_name("state.json"), &json)
        {
            first_error.get_or_insert(format!("state.json: {e}"));
        }
        self.state_dirty = false;
        first_error.map_or(Ok(()), Err)
    }
}

/// Write to a temp file and rename, so a crash never leaves a half-written sheet.
/// Write to a temporary file, flush it to disk, then rename it over the original,
/// so a crash or power cut leaves either the old file or the new one. The temporary
/// name is unique, so two Omacus windows never write the same one.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{}-{n}.tmp", std::process::id()));
    let tmp = PathBuf::from(name);
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    #[cfg(unix)]
    if let Some(dir) = path.parent()
        && let Ok(dir) = std::fs::File::open(dir)
    {
        let _ = dir.sync_all();
    }
    result
}

/// Earlier versions kept a single `sheet.txt`.
fn migrate_single_sheet(root: &Path, dir: &Path) {
    let old = root.join("sheet.txt");
    if old.exists() {
        let _ = std::fs::rename(&old, dir.join(format!("{}.txt", new_id())));
    }
}

/// Time plus a per-process counter. Time alone isn't unique: macOS's clock has
/// microsecond resolution, so two sheets made back to back got the same id.
fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{n:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("omacus-test-{name}-{}", new_id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn sheets_persist_across_restarts() {
        let root = temp_root("persist");
        let mut store = Store::load_in(Some(root.clone()), "# Welcome");
        assert_eq!(store.sheets.len(), 1);

        store.new_sheet("");
        store.current().text = "# Groceries\nmilk $3".into();
        store.current().mark_dirty();
        store.set_cursor(5);
        store.toggle_sidebar();
        store.save().unwrap();

        let mut again = Store::load_in(Some(root.clone()), "# Welcome");
        assert_eq!(again.sheets.len(), 2);
        assert_eq!(again.active, 0);
        assert_eq!(again.current().title(), "Groceries");
        assert_eq!(again.current().cursor, 5);
        assert!(!again.sidebar);
        assert_eq!(again.sheets[1].title(), "Welcome");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delete_moves_to_trash_and_reorder_sticks() {
        let root = temp_root("trash");
        let mut store = Store::load_in(Some(root.clone()), "# A");
        store.new_sheet("# B");
        store.new_sheet("# C");
        store.save().unwrap();
        // Order is C, B, A. Move C down, then delete B.
        store.move_active(1);
        assert_eq!(
            store.sheets.iter().map(Sheet::title).collect::<Vec<_>>(),
            ["B", "C", "A"]
        );
        store.delete(0);
        store.save().unwrap();

        let again = Store::load_in(Some(root.clone()), "");
        assert_eq!(
            again.sheets.iter().map(Sheet::title).collect::<Vec<_>>(),
            ["C", "A"]
        );
        assert_eq!(std::fs::read_dir(root.join("trash")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_deleted_sheet_can_come_back_with_unsaved_edits() {
        let root = temp_root("undo");
        let mut store = Store::load_in(Some(root.clone()), "# A");
        store.new_sheet("# B");
        store.save().unwrap();
        // An edit that hasn't been saved yet still reaches the trash.
        store.sheets[0].text = "# B\nlate edit".into();
        store.sheets[0].mark_dirty();
        store.delete(0);
        let trashed = std::fs::read_to_string(
            root.join("trash")
                .join(format!("{}.txt", store.deleted.as_ref().unwrap().1.id)),
        )
        .unwrap();
        assert_eq!(trashed, "# B\nlate edit");

        assert_eq!(store.undo_delete().as_deref(), Some("B"));
        assert_eq!(
            store.sheets.iter().map(Sheet::title).collect::<Vec<_>>(),
            ["B", "A"]
        );
        store.save().unwrap();
        let again = Store::load_in(Some(root.clone()), "");
        assert_eq!(again.sheets[0].text, "# B\nlate edit");
        assert_eq!(std::fs::read_dir(root.join("trash")).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_save_says_so() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_root("readonly");
        let mut store = Store::load_in(Some(root.clone()), "# A");
        store.save().unwrap();
        let sheets = root.join("sheets");
        std::fs::set_permissions(&sheets, std::fs::Permissions::from_mode(0o500)).unwrap();
        store.sheets[0].text = "# A\nmore".into();
        store.sheets[0].mark_dirty();
        let result = store.save();
        std::fs::set_permissions(&sheets, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err(), "{result:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sheets_made_back_to_back_never_collide() {
        let root = temp_root("burst");
        let mut store = Store::load_in(Some(root.clone()), "# 0");
        for i in 1..200 {
            store.new_sheet(&format!("# {i}"));
        }
        store.save().unwrap();
        let again = Store::load_in(Some(root.clone()), "");
        assert_eq!(again.sheets.len(), 200);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_the_old_single_sheet() {
        let root = temp_root("migrate");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("sheet.txt"), "# Old notes\n1 + 1").unwrap();
        let mut store = Store::load_in(Some(root.clone()), "# Welcome");
        assert_eq!(store.sheets.len(), 1);
        assert_eq!(store.current().title(), "Old notes");
        assert!(!root.join("sheet.txt").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
