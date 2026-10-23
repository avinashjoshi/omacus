//! Pages live as plain text files in `~/.local/share/omacus/pages/`, Sublime-style:
//! every edit is saved, nothing asks to be saved, and closing the window loses nothing.
//! `state.json` remembers order, the active page, cursors and the sidebar.
//!
//! Older versions stored those files in `sheets/`. The first launch that sees only
//! that folder renames it to `pages/`; if both exist, files still only in `sheets/`
//! are moved across. The text format and `state.json` keys are unchanged.
//!
//! Two windows can be open on the same sheets. Each remembers every file's size and
//! modification time as it last read or wrote it, and `sync` picks up what the other
//! window changed. When both changed a sheet, this window's text is saved and the
//! other's becomes a new sheet, so neither is lost.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use omacus_engine::lock::{self, ContentKey, Wrap};
use omacus_engine::xref;
use serde::{Deserialize, Serialize};

pub struct Sheet {
    pub id: String,
    pub text: String,
    pub cursor: usize,
    /// Short name for `@handle.variable`. Renaming the page does not change it.
    pub handle: String,
    dirty: bool,
    /// The file as this window last read or wrote it; `None` until it's on disk.
    stamp: Option<Stamp>,
    /// Ciphertext file body. `text` is plaintext only while `unlocked` is set.
    sealed: Option<String>,
    /// The password has been entered this session, so `text` is the page.
    unlocked: bool,
}

/// A file's modification time and size, to notice another window's writes.
type Stamp = (SystemTime, u64);

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// What `sync` found.
#[derive(Default, Debug)]
pub struct Synced {
    /// The open sheet's text changed (or another sheet is open now).
    pub active_changed: bool,
    pub changed: bool,
}

impl Sheet {
    /// The first line, with a `#` or `//` stripped off. Not shortened.
    /// A locked page that isn't open is [`LOCKED_TITLE`], so the sidebar and
    /// the switcher never show its first line.
    pub fn heading(&self) -> String {
        if self.sealed.is_some() && !self.unlocked {
            return LOCKED_TITLE.into();
        }
        heading_of(&self.text)
    }

    pub fn locked(&self) -> bool {
        self.sealed.is_some()
    }

    /// The password has been entered, so `text` is the page.
    pub fn revealed(&self) -> bool {
        self.sealed.is_none() || self.unlocked
    }

    /// What the page file holds: ciphertext while locked, otherwise the text.
    pub fn stored(&self) -> &str {
        self.sealed.as_deref().unwrap_or(&self.text)
    }

    pub fn title(&self) -> String {
        let title = self.heading();
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

/// What the library shows for a locked page that is not open.
pub const LOCKED_TITLE: &str = "Locked page";

/// A sheet's name: its first non-empty line, without a leading `#` or `//`.
pub fn heading_of(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let title = line.trim_start_matches(['#', '/', ' ']).trim();
    if title.is_empty() {
        "Untitled".into()
    } else {
        title.to_string()
    }
}

/// A folder may contain another folder. Anything deeper is refused.
pub const MAX_FOLDER_DEPTH: usize = 2;

/// One sidebar entry. Pages and folders share this tree; `state.json` from before
/// folders existed has no `tree`, and those pages load at the root in `order`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    Page {
        id: String,
    },
    Folder {
        id: String,
        name: String,
        #[serde(default)]
        collapsed: bool,
        #[serde(default)]
        children: Vec<Node>,
        /// Set when the folder has a password. Pages inside are encrypted with it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lock: Option<FolderSeal>,
    },
}

/// The wrapped key for a locked folder. Stored on the folder, and copied into
/// each page file so a page can still be opened after the folder is gone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderSeal {
    pub salt: String,
    pub nonce: String,
    pub wrapped: String,
    #[serde(default = "seal_memory")]
    pub memory_kib: u32,
    #[serde(default = "seal_iterations")]
    pub iterations: u32,
    #[serde(default = "seal_lanes")]
    pub lanes: u32,
}

fn seal_memory() -> u32 {
    19 * 1024
}
fn seal_iterations() -> u32 {
    2
}
fn seal_lanes() -> u32 {
    1
}

impl From<&Wrap> for FolderSeal {
    fn from(wrap: &Wrap) -> Self {
        Self {
            salt: wrap.salt.clone(),
            nonce: wrap.nonce.clone(),
            wrapped: wrap.wrapped.clone(),
            memory_kib: wrap.memory_kib,
            iterations: wrap.iterations,
            lanes: wrap.lanes,
        }
    }
}

impl FolderSeal {
    fn wrap(&self) -> Wrap {
        Wrap {
            salt: self.salt.clone(),
            nonce: self.nonce.clone(),
            wrapped: self.wrapped.clone(),
            memory_kib: self.memory_kib,
            iterations: self.iterations,
            lanes: self.lanes,
        }
    }
}

/// A row the sidebar draws. Children of a collapsed folder are left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideEntry {
    pub depth: usize,
    pub kind: SideKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideKind {
    Page {
        id: String,
    },
    Folder {
        id: String,
        name: String,
        collapsed: bool,
    },
}

/// Where a drag (or Move to folder) puts an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// Same parent as this page or folder, just before it.
    Before(String),
    /// Same parent as this page or folder, just after it.
    After(String),
    /// First child of this folder.
    Into(String),
    /// Last child of the root.
    RootEnd,
}

/// A folder in the Move-to list, parents before their children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderInfo {
    pub id: String,
    pub name: String,
    /// 1 is a top-level folder, 2 is nested inside one.
    pub depth: usize,
}

/// Missing fields keep their defaults, so a `state.json` from an older version
/// (no `sidebar_width`, for example) still loads the sheet order and cursors.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    order: Vec<String>,
    active: String,
    cursors: HashMap<String, usize>,
    sidebar: Option<bool>,
    /// Width of the expanded sidebar, in points. Absent in older `state.json`.
    sidebar_width: Option<f32>,
    /// Absent until the first save that knows about folders.
    tree: Option<Vec<Node>>,
    /// Page id to handle. Kept for pages in Recently deleted, so a reference
    /// still names them. Absent in older `state.json`; those pages get one.
    handles: HashMap<String, String>,
    /// The first-run examples have been offered. Stays set if they're deleted,
    /// so a later launch does not put them back. Absent in older `state.json`.
    seeded: bool,
}

/// Expanded sidebar width, and the range a drag is allowed to land in.
pub const SIDEBAR_WIDTH: f32 = 210.0;
pub const SIDEBAR_MIN: f32 = 168.0;
pub const SIDEBAR_MAX: f32 = 480.0;

/// A sheet sitting in `trash/`, ready to be put back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trashed {
    pub id: String,
    pub title: String,
    pub text: String,
    pub handle: String,
    /// The file is encrypted. `text` is empty so a preview can't show it.
    pub locked: bool,
}

/// The last delete, so Undo can put the page or the folder back where it was.
struct Deleted {
    parent: Option<String>,
    at: usize,
    item: DeletedItem,
}

enum DeletedItem {
    Page(Sheet),
    /// `node` still holds the folder's children. `sheets` are those pages' text.
    Folder {
        node: Node,
        sheets: Vec<Sheet>,
    },
}

pub struct Store {
    dir: Option<PathBuf>,
    pub sheets: Vec<Sheet>,
    /// Sidebar order. Pages not listed here are appended at the root on load.
    tree: Vec<Node>,
    pub active: usize,
    pub sidebar: bool,
    pub sidebar_width: f32,
    handles: HashMap<String, String>,
    /// See `State::seeded`. An older library is marked on its next launch, and
    /// nothing is added to it.
    seeded: bool,
    state_dirty: bool,
    deleted: Option<Deleted>,
    /// Titles of sheets that two windows changed at once, to tell the user.
    conflicts: Vec<String>,
    /// Content keys for this session only. Never written to `state.json`.
    page_keys: HashMap<String, ContentKey>,
    folder_keys: HashMap<String, ContentKey>,
}

/// `OMACUS_DATA_DIR` when it is set, otherwise the per-user data folder
/// (`~/.local/share/omacus`, and the equivalents on macOS and Windows).
/// The window, the command line and the MCP server all use this.
pub fn data_root() -> Option<PathBuf> {
    match std::env::var_os("OMACUS_DATA_DIR") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => dirs::data_dir().map(|d| d.join("omacus")),
    }
}

impl Store {
    pub fn load(welcome: &str) -> Self {
        let mut store = Self::load_in(data_root(), welcome);
        store.seed_first_run();
        store
    }

    /// Pages already on disk. An empty library stays empty, so a command that
    /// only lists pages does not create one.
    pub fn open_library() -> Self {
        Self::load_pages(data_root(), None)
    }

    /// Open the pages under `root` (a temp directory in tests).
    #[cfg(test)]
    pub(crate) fn open_at(root: PathBuf) -> Self {
        Self::load_pages(Some(root), None)
    }

    pub fn directory(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The folder that holds `pages/`, `trash/`, and `state.json`.
    pub fn library_root(&self) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .and_then(|pages| pages.parent().map(Path::to_path_buf))
    }

    /// A page as `@handle.name` may see it. A closed lock contributes no text.
    pub fn as_reference(
        &self,
        sheet: &Sheet,
        deleted: bool,
        elsewhere: bool,
    ) -> omacus_engine::xref::Page {
        let open = sheet.revealed();
        omacus_engine::xref::Page {
            id: sheet.id.clone(),
            handle: sheet.handle.clone(),
            title: sheet.heading(),
            text: if open {
                sheet.text.clone()
            } else {
                String::new()
            },
            locked: !open,
            deleted,
            elsewhere,
        }
    }

    fn load_in(root: Option<PathBuf>, welcome: &str) -> Self {
        Self::load_pages(root, Some(welcome))
    }

    fn load_pages(root: Option<PathBuf>, welcome: Option<&str>) -> Self {
        let dir = root.as_ref().map(|r| pages_dir(r));
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
                if let Some(mut sheet) = read_sheet(dir, &id) {
                    if let Some(&cursor) = state.cursors.get(&id) {
                        sheet.cursor = cursor;
                    }
                    sheets.push(sheet);
                }
            }
        }

        let mut tree = match state.tree {
            Some(raw) => {
                let ids: HashSet<String> = sheets.iter().map(|s| s.id.clone()).collect();
                sanitize_tree(raw, &ids)
            }
            // No tree yet: the saved order is the root, top to bottom.
            None => sheets
                .iter()
                .map(|s| Node::Page { id: s.id.clone() })
                .collect(),
        };
        let placed = page_ids(&tree);
        for sheet in &sheets {
            if !placed.iter().any(|id| id == &sheet.id) {
                tree.push(Node::Page {
                    id: sheet.id.clone(),
                });
            }
        }

        let mut store = Self {
            dir,
            active: 0,
            sidebar: state.sidebar.unwrap_or(true),
            sidebar_width: state
                .sidebar_width
                .unwrap_or(SIDEBAR_WIDTH)
                .clamp(SIDEBAR_MIN, SIDEBAR_MAX),
            sheets,
            tree,
            handles: state.handles,
            seeded: state.seeded,
            state_dirty: false,
            deleted: None,
            conflicts: Vec::new(),
            page_keys: HashMap::new(),
            folder_keys: HashMap::new(),
        };
        store.align_sheets();
        store.ensure_handles();
        if store.sheets.is_empty()
            && let Some(welcome) = welcome
        {
            store.new_sheet(welcome);
        }
        store.active = store
            .sheets
            .iter()
            .position(|s| s.id == state.active)
            .unwrap_or(0);
        store
    }

    /// Offer the example pages once, on a library that has never had any.
    /// An existing library is only marked, so its pages stay as they are.
    /// Deleting the examples does not bring them back.
    fn seed_first_run(&mut self) {
        if self.seeded {
            return;
        }
        if self.pages_on_disk() || self.trash_on_disk() {
            self.seeded = true;
            self.state_dirty = true;
            let _ = self.save();
            return;
        }
        let welcome_id = self.sheets.first().map(|s| s.id.clone());
        let Some(folder) = self.create_folder(None) else {
            return;
        };
        self.rename_folder(&folder, "Examples");
        for text in bundled_examples().iter().rev() {
            self.new_sheet_in(Some(&folder), text);
        }
        if let Some(id) = welcome_id
            && let Some(i) = self.sheets.iter().position(|s| s.id == id)
        {
            self.active = i;
        }
        self.seeded = true;
        self.state_dirty = true;
        let _ = self.save();
    }

    fn pages_on_disk(&self) -> bool {
        self.dir.as_ref().is_some_and(|dir| dir_has_page(dir))
    }

    fn trash_on_disk(&self) -> bool {
        self.trash_dir().is_some_and(|dir| dir_has_page(&dir))
    }

    pub fn side_entries(&self) -> Vec<SideEntry> {
        side_entries(&self.tree)
    }

    /// Folders a page can move into, in sidebar order.
    pub fn folders(&self) -> Vec<FolderInfo> {
        let mut out = Vec::new();
        fn walk(nodes: &[Node], depth: usize, out: &mut Vec<FolderInfo>) {
            for node in nodes {
                if let Node::Folder {
                    id, name, children, ..
                } = node
                {
                    out.push(FolderInfo {
                        id: id.clone(),
                        name: name.clone(),
                        depth: depth + 1,
                    });
                    walk(children, depth + 1, out);
                }
            }
        }
        walk(&self.tree, 0, &mut out);
        out
    }

    pub fn folder_of(&self, page_id: &str) -> Option<String> {
        locate(&self.tree, page_id).and_then(|(parent, _, _)| parent)
    }

    pub fn folder_name(&self, id: &str) -> Option<String> {
        find_folder(&self.tree, id).map(|(name, _, _)| name)
    }

    /// Whether a new folder may be created inside `parent` (`None` is the root).
    pub fn can_nest_folder(&self, parent: Option<&str>) -> bool {
        match parent {
            None => true,
            Some(id) => folder_depth(&self.tree, id).is_some_and(|d| d < MAX_FOLDER_DEPTH),
        }
    }

    /// True when this page is the first or last among its siblings.
    pub fn at_sibling_edge(&self, index: usize) -> (bool, bool) {
        let Some(id) = self.sheets.get(index).map(|s| s.id.as_str()) else {
            return (true, true);
        };
        let Some((parent, at, _)) = locate(&self.tree, id) else {
            return (true, true);
        };
        let len = children_len(&self.tree, parent.as_deref());
        (at == 0, at + 1 >= len)
    }

    pub fn current(&mut self) -> &mut Sheet {
        &mut self.sheets[self.active]
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.sheets.len() && idx != self.active {
            self.active = idx;
            let id = self.sheets[idx].id.clone();
            self.expand_to(&id);
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

    /// Open a shared sheet: the copy already here if there is one, else a new sheet.
    /// Returns whether it was new.
    pub fn open(&mut self, text: &str) -> bool {
        match self.sheets.iter().position(|s| s.text == text) {
            Some(i) => {
                self.select(i);
                false
            }
            None => {
                self.new_sheet(text);
                true
            }
        }
    }

    /// New sheets go to the top of the open page's folder (the root, when that
    /// page isn't in one), like a notes app.
    /// `false` when the open page's folder is locked and this session doesn't
    /// have its key: the page is not created, so it can't be written in the clear.
    pub fn new_sheet(&mut self, text: &str) -> bool {
        let parent = self
            .sheets
            .get(self.active)
            .and_then(|s| self.folder_of(&s.id));
        self.new_sheet_in(parent.as_deref(), text)
    }

    /// A new page at the top of `parent` (`None` is the root). `false` when that
    /// folder is locked and not unlocked.
    pub fn new_sheet_in(&mut self, parent: Option<&str>, text: &str) -> bool {
        let id = self.unused_id();
        let parent = parent.filter(|id| self.folder_name(id).is_some());
        let mut sheet = plain_sheet(id.clone(), text.to_string());
        let handle = self.fresh_handle(&sheet.heading());
        self.handles.insert(id.clone(), handle.clone());
        sheet.handle = handle;
        if sheet.sealed.is_none()
            && let Some(folder) = parent.and_then(|id| self.covering_folder(id))
        {
            let Some(key) = self.folder_keys.get(&folder).cloned() else {
                return false;
            };
            let Some(wrap) = self.folder_seal(&folder).map(|s| s.wrap()) else {
                return false;
            };
            let Ok(sealed) = lock::seal(&key, text, &wrap, id.as_bytes()) else {
                return false;
            };
            sheet.sealed = Some(sealed);
            sheet.unlocked = true;
        }
        if insert_node(&mut self.tree, parent, 0, Node::Page { id: id.clone() }).is_err() {
            self.tree.insert(0, Node::Page { id: id.clone() });
        }
        self.sheets.push(sheet);
        self.align_sheets();
        self.active = self.sheets.iter().position(|s| s.id == id).unwrap_or(0);
        self.state_dirty = true;
        true
    }

    /// A folder at the top of `parent`. `None` when that parent is already nested.
    pub fn create_folder(&mut self, parent: Option<&str>) -> Option<String> {
        let parent = parent.filter(|id| self.folder_name(id).is_some());
        if !self.can_nest_folder(parent) {
            return None;
        }
        let id = format!("f{}", new_id());
        let node = Node::Folder {
            id: id.clone(),
            name: fresh_folder_name(&self.tree),
            collapsed: false,
            children: Vec::new(),
            lock: None,
        };
        if insert_node(&mut self.tree, parent, 0, node).is_err() {
            return None;
        }
        self.state_dirty = true;
        Some(id)
    }

    pub fn rename_folder(&mut self, id: &str, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let name = name.to_string();
        let renamed = rename_folder(&mut self.tree, id, &name);
        if renamed {
            self.state_dirty = true;
        }
        renamed
    }

    pub fn toggle_folder(&mut self, id: &str) {
        if set_collapsed(&mut self.tree, id, None) {
            self.state_dirty = true;
        }
    }

    /// Open a folder, including when a drag hovers it.
    pub fn expand_folder(&mut self, id: &str) {
        if set_collapsed(&mut self.tree, id, Some(false)) {
            self.state_dirty = true;
        }
    }

    /// Delete a folder. Every page inside it, including in a nested folder, moves
    /// to Recently deleted. Returns the folder's name.
    pub fn delete_folder(&mut self, id: &str) -> Option<String> {
        let active_id = self.sheets.get(self.active).map(|s| s.id.clone());
        let (parent, at, node) = take_node(&mut self.tree, id)?;
        let Node::Folder { name, .. } = &node else {
            let _ = insert_node(&mut self.tree, parent.as_deref(), at, node);
            return None;
        };
        let name = name.clone();
        let ids = page_ids_in(&node);
        for id in &ids {
            self.refresh_seal(id);
        }
        let mut kept = Vec::new();
        let mut removed = Vec::new();
        for sheet in self.sheets.drain(..) {
            if ids.iter().any(|id| id == &sheet.id) {
                removed.push(sheet);
            } else {
                kept.push(sheet);
            }
        }
        removed.sort_by_key(|sheet| ids.iter().position(|id| id == &sheet.id).unwrap_or(0));
        for sheet in &removed {
            // `refresh_seal` already ran while the pages were still in `self.sheets`.
            self.trash_sheet(sheet);
        }
        self.sheets = kept;
        self.deleted = Some(Deleted {
            parent,
            at,
            item: DeletedItem::Folder {
                node,
                sheets: removed,
            },
        });
        if self.sheets.is_empty() {
            self.new_sheet("");
        } else if let Some(id) = active_id
            && let Some(idx) = self.sheets.iter().position(|s| s.id == id)
        {
            self.active = idx;
        } else {
            self.active = 0;
        }
        self.state_dirty = true;
        Some(name)
    }

    /// Move a page or folder. `index` is among the destination's children after
    /// this item has been taken out. Returns whether it landed there.
    pub fn move_item(&mut self, id: &str, parent: Option<&str>, index: usize) -> bool {
        let is_page = self.sheets.iter().any(|s| s.id == id);
        let moving: Vec<String> = if is_page {
            vec![id.to_string()]
        } else {
            self.pages_under(id)
        };
        if moving.iter().any(|pid| !self.can_move_page(pid, parent)) {
            return false;
        }
        let old_covers: Vec<(String, Option<String>)> = moving
            .iter()
            .map(|pid| (pid.clone(), self.covering_folder(pid)))
            .collect();
        let Some((old_parent, old_at, node)) = take_node(&mut self.tree, id) else {
            return false;
        };
        let parent_exists = parent.is_none_or(|p| folder_depth(&self.tree, p).is_some());
        if !parent_exists || !can_place(&self.tree, &node, parent) {
            let _ = insert_node(&mut self.tree, old_parent.as_deref(), old_at, node);
            return false;
        }
        if old_parent.as_deref() == parent && old_at == index.min(children_len(&self.tree, parent))
        {
            let _ = insert_node(&mut self.tree, old_parent.as_deref(), old_at, node);
            return true;
        }
        if let Err(node) = insert_node(&mut self.tree, parent, index, node) {
            let _ = insert_node(&mut self.tree, old_parent.as_deref(), old_at, node);
            return false;
        }
        for (pid, old) in &old_covers {
            self.rebind_page(pid, old.as_deref());
        }
        self.align_sheets();
        self.state_dirty = true;
        true
    }

    /// Drop `id` at `spot`. A no-op drop (onto itself) still returns true.
    pub fn place(&mut self, id: &str, spot: Place) -> bool {
        let Some((src_parent, src_at, _)) = locate(&self.tree, id) else {
            return false;
        };
        let (parent, mut index) = match &spot {
            Place::Before(sib) | Place::After(sib) => {
                if sib == id {
                    return true;
                }
                let Some((p, i, _)) = locate(&self.tree, sib) else {
                    return false;
                };
                let index = if matches!(spot, Place::After(_)) {
                    i + 1
                } else {
                    i
                };
                (p, index)
            }
            Place::Into(folder) => {
                if folder == id {
                    return false;
                }
                (Some(folder.clone()), 0)
            }
            Place::RootEnd => (None, usize::MAX),
        };
        if src_parent == parent && src_at < index {
            index -= 1;
        }
        self.move_item(id, parent.as_deref(), index)
    }

    /// Deleted sheets are moved to `trash/`, never destroyed.
    pub fn delete(&mut self, idx: usize) {
        if idx >= self.sheets.len() {
            return;
        }
        let id = self.sheets[idx].id.clone();
        let (parent, at, _) =
            take_node(&mut self.tree, &id).unwrap_or((None, idx, Node::Page { id: id.clone() }));
        self.refresh_seal(&id);
        let sheet = self.sheets.remove(idx);
        self.trash_sheet(&sheet);
        self.deleted = Some(Deleted {
            parent,
            at,
            item: DeletedItem::Page(sheet),
        });
        if self.sheets.is_empty() {
            self.new_sheet("");
        } else if self.active > idx || self.active >= self.sheets.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.state_dirty = true;
    }

    /// Move the open page up or down among the pages and folders beside it.
    pub fn move_active(&mut self, step: isize) {
        let Some(id) = self.sheets.get(self.active).map(|s| s.id.clone()) else {
            return;
        };
        let Some((parent, at, _)) = locate(&self.tree, &id) else {
            return;
        };
        let dest = at as isize + step;
        let len = children_len(&self.tree, parent.as_deref());
        if dest < 0 || dest as usize >= len {
            return;
        }
        if swap_siblings(&mut self.tree, parent.as_deref(), at, dest as usize) {
            self.align_sheets();
            self.active = self
                .sheets
                .iter()
                .position(|s| s.id == id)
                .unwrap_or(self.active);
            self.state_dirty = true;
        }
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar = !self.sidebar;
        self.state_dirty = true;
    }

    pub fn set_sidebar_width(&mut self, width: f32) {
        let width = width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        if (self.sidebar_width - width).abs() > 1.0 {
            self.sidebar_width = width;
            self.state_dirty = true;
        }
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

    /// Titles of sheets that another window changed while this one was editing them.
    pub fn take_conflicts(&mut self) -> Vec<String> {
        std::mem::take(&mut self.conflicts)
    }

    /// Pick up what another window did: reload sheets it changed (unless they have
    /// edits here, which `save` sorts out), drop ones it deleted, and add new ones.
    pub fn sync(&mut self) -> Synced {
        let mut synced = Synced::default();
        let Some(dir) = self.dir.clone() else {
            return synced;
        };
        let active = self.sheets[self.active].id.clone();
        let mut i = 0;
        while i < self.sheets.len() {
            let sheet = &mut self.sheets[i];
            let path = dir.join(format!("{}.txt", sheet.id));
            let now = stamp(&path);
            if sheet.dirty || sheet.stamp.is_none() || now == sheet.stamp {
                i += 1;
                continue;
            }
            synced.changed = true;
            match now.and_then(|_| read_sheet(&dir, &sheet.id)) {
                Some(fresh) => {
                    let changed = fresh.sealed != sheet.sealed
                        || (fresh.sealed.is_none() && fresh.text != sheet.text);
                    if changed {
                        synced.active_changed |= sheet.id == active;
                        let cursor = sheet.cursor;
                        *sheet = fresh;
                        sheet.cursor = cursor;
                    }
                    sheet.stamp = stamp(&path);
                    i += 1;
                }
                // Deleted in the other window.
                None => {
                    let id = self.sheets[i].id.clone();
                    self.sheets.remove(i);
                    let _ = take_node(&mut self.tree, &id);
                    synced.active_changed |= self.sheets.iter().all(|s| s.id != active);
                }
            }
        }
        // Sheets made in the other window go to the top, like new ones here.
        // A page this window just deleted is not one of them, even if its file
        // is still in `pages/` because the trash move hasn't finished.
        let skip = self
            .deleted
            .as_ref()
            .map(Deleted::page_ids)
            .unwrap_or_default();
        let new: Vec<Sheet> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .into_string()
                    .ok()?
                    .strip_suffix(".txt")
                    .map(String::from)
            })
            .filter(|id| !skip.iter().any(|s| s == id) && self.sheets.iter().all(|s| &s.id != id))
            .filter_map(|id| read_sheet(&dir, &id))
            .collect();
        if !new.is_empty() {
            synced.changed = true;
            for sheet in new.iter().rev() {
                self.tree.insert(
                    0,
                    Node::Page {
                        id: sheet.id.clone(),
                    },
                );
            }
            self.sheets.splice(0..0, new);
            self.align_sheets();
            self.ensure_handles();
        }
        if self.sheets.is_empty() {
            self.new_sheet("");
        }
        self.active = self.sheets.iter().position(|s| s.id == active).unwrap_or(0);
        self.reveal_all();
        synced
    }

    fn trash_dir(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.with_file_name("trash"))
    }

    fn trash_sheet(&self, sheet: &Sheet) {
        let Some(dir) = &self.dir else {
            return;
        };
        // Ciphertext when the page is locked, so Recently deleted has no plaintext.
        let Some(body) = stored_body(sheet) else {
            return;
        };
        let trash = dir.with_file_name("trash");
        let _ = std::fs::create_dir_all(&trash);
        if write_atomic(&trash.join(format!("{}.txt", sheet.id)), body.as_bytes()).is_ok() {
            let _ = std::fs::remove_file(dir.join(format!("{}.txt", sheet.id)));
        }
    }

    fn untrash(&self, id: &str) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_file(dir.with_file_name("trash").join(format!("{id}.txt")));
        }
    }

    /// A deleted last page is replaced by an empty one. Undo drops that again.
    fn drop_placeholder(&mut self) {
        if self.sheets.len() == 1 && self.sheets[0].text.is_empty() {
            let id = self.sheets[0].id.clone();
            self.sheets.clear();
            let _ = take_node(&mut self.tree, &id);
        }
    }

    /// Open every folder between the root and this page, so a jump can show it.
    fn expand_to(&mut self, page_id: &str) {
        fn walk(nodes: &mut [Node], page_id: &str) -> bool {
            for node in nodes {
                if let Node::Folder {
                    collapsed,
                    children,
                    ..
                } = node
                    && page_ids_in_nodes(children).iter().any(|id| id == page_id)
                {
                    *collapsed = false;
                    walk(children, page_id);
                    return true;
                }
            }
            false
        }
        walk(&mut self.tree, page_id);
    }

    /// `sheets` follows the tree, depth-first, so indexes stay the sidebar's page order.
    fn align_sheets(&mut self) {
        let order = page_ids(&self.tree);
        let active_id = self.sheets.get(self.active).map(|s| s.id.clone());
        let mut by_id: HashMap<String, Sheet> =
            self.sheets.drain(..).map(|s| (s.id.clone(), s)).collect();
        let mut ordered = Vec::new();
        for id in &order {
            if let Some(sheet) = by_id.remove(id) {
                ordered.push(sheet);
            }
        }
        for (id, sheet) in by_id {
            self.tree.push(Node::Page { id });
            ordered.push(sheet);
        }
        self.sheets = ordered;
        self.active = active_id
            .and_then(|id| self.sheets.iter().position(|s| s.id == id))
            .unwrap_or(0);
    }

    /// Sheets in `trash/`, newest first. Reading the directory is the caller's
    /// choice of how often; this does not cache.
    pub fn trashed(&self) -> Vec<Trashed> {
        let Some(dir) = self.trash_dir() else {
            return Vec::new();
        };
        let mut items: Vec<(SystemTime, Trashed)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let id = name.strip_suffix(".txt")?.to_string();
                let text = std::fs::read_to_string(e.path()).ok()?;
                let modified = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH);
                let sheet = sheet_from_stored(id.clone(), text, false, None);
                let locked = sheet.locked();
                let title = sheet.title();
                let handle = self
                    .handles
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| xref::allocate_handle(&title, &self.taken_handles()));
                let text = if locked { String::new() } else { sheet.text };
                Some((
                    modified,
                    Trashed {
                        id,
                        title,
                        text,
                        handle,
                        locked,
                    },
                ))
            })
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.0));
        items.into_iter().map(|(_, t)| t).collect()
    }

    /// Put a trashed sheet back at the top of the list. Works after a restart,
    /// when the in-memory undo is gone. Returns its title.
    pub fn restore(&mut self, id: &str) -> Option<String> {
        let path = self.trash_dir()?.join(format!("{id}.txt"));
        let text = std::fs::read_to_string(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        if self.deleted.as_ref().is_some_and(|d| d.contains_page(id)) {
            self.deleted = None;
        }
        self.drop_placeholder();
        let mut sheet = sheet_from_stored(id.to_string(), text, true, None);
        let handle = self
            .handles
            .get(id)
            .cloned()
            .unwrap_or_else(|| self.fresh_handle(&sheet.heading()));
        self.handles.insert(id.to_string(), handle.clone());
        sheet.handle = handle;
        let title = sheet.title();
        self.tree.insert(0, Node::Page { id: id.to_string() });
        self.sheets.push(sheet);
        self.align_sheets();
        self.active = self.sheets.iter().position(|s| s.id == id).unwrap_or(0);
        self.state_dirty = true;
        Some(title)
    }

    /// Remove a trashed sheet for good. Undo can no longer bring it back.
    pub fn purge(&mut self, id: &str) -> bool {
        let Some(path) = self.trash_dir().map(|d| d.join(format!("{id}.txt"))) else {
            return false;
        };
        if std::fs::remove_file(&path).is_err() {
            return false;
        }
        if self.deleted.as_ref().is_some_and(|d| d.contains_page(id)) {
            self.deleted = None;
        }
        self.handles.remove(id);
        self.state_dirty = true;
        true
    }

    /// Put back the page or folder deleted last, where it was. Returns its name.
    pub fn undo_delete(&mut self) -> Option<String> {
        let deleted = self.deleted.take()?;
        self.drop_placeholder();
        match deleted.item {
            DeletedItem::Page(mut sheet) => {
                self.untrash(&sheet.id);
                sheet.dirty = true;
                sheet.stamp = None;
                let title = sheet.title();
                let id = sheet.id.clone();
                let _ = insert_node(
                    &mut self.tree,
                    deleted.parent.as_deref(),
                    deleted.at,
                    Node::Page { id: id.clone() },
                );
                self.sheets.push(sheet);
                self.align_sheets();
                self.active = self.sheets.iter().position(|s| s.id == id).unwrap_or(0);
                self.state_dirty = true;
                Some(title)
            }
            DeletedItem::Folder { node, sheets } => {
                let name = match &node {
                    Node::Folder { name, .. } => name.clone(),
                    Node::Page { .. } => "Folder".into(),
                };
                let first = sheets.first().map(|s| s.id.clone());
                for mut sheet in sheets {
                    self.untrash(&sheet.id);
                    sheet.dirty = true;
                    sheet.stamp = None;
                    self.sheets.push(sheet);
                }
                let _ = insert_node(&mut self.tree, deleted.parent.as_deref(), deleted.at, node);
                self.align_sheets();
                if let Some(id) = first {
                    self.active = self.sheets.iter().position(|s| s.id == id).unwrap_or(0);
                }
                self.state_dirty = true;
                Some(name)
            }
        }
    }

    /// `Some(true)` is a closed lock, `Some(false)` an open one, `None` if the page
    /// has no password of its own and isn't inside a locked folder.
    pub fn page_lock(&self, id: &str) -> Option<bool> {
        let sheet = self.sheets.iter().find(|s| s.id == id)?;
        let locked = sheet.sealed.is_some() || self.covering_folder(id).is_some();
        if !locked {
            return None;
        }
        let open = sheet.unlocked || (sheet.sealed.is_none() && self.folder_is_open(id));
        Some(!open)
    }

    /// `Some(true)` when the folder is locked and this session doesn't have the key.
    pub fn folder_lock(&self, id: &str) -> Option<bool> {
        self.folder_seal(id)?;
        Some(!self.folder_keys.contains_key(id))
    }

    pub fn concealed(&self, id: &str) -> bool {
        self.page_lock(id) == Some(true)
    }

    /// The page is encrypted by a folder password rather than its own.
    pub fn folder_covers_page(&self, id: &str) -> bool {
        self.covering_folder(id).is_some() && !self.individual_lock(id)
    }

    /// A locked folder that still has to be unlocked before a page can be added.
    pub fn needs_unlock(&self, parent: Option<&str>) -> Option<String> {
        let folder = parent.and_then(|id| self.covering_folder(id))?;
        (!self.folder_keys.contains_key(&folder)).then_some(folder)
    }

    pub fn any_unlocked(&self) -> bool {
        self.sheets.iter().any(|s| s.unlocked) || !self.folder_keys.is_empty()
    }

    pub fn lock_page(&mut self, id: &str, password: &str) -> Result<(), String> {
        if self.covering_folder(id).is_some() {
            return Err("This page is inside a locked folder.".into());
        }
        let idx = self.idx(id).ok_or("That page is gone.")?;
        if self.sheets[idx].sealed.is_some() {
            return Err("This page is already locked.".into());
        }
        let (key, wrap) = lock::create(password).map_err(|e| e.message().to_string())?;
        let text = self.sheets[idx].text.clone();
        let sealed =
            lock::seal(&key, &text, &wrap, id.as_bytes()).map_err(|e| e.message().to_string())?;
        self.sheets[idx].sealed = Some(sealed);
        self.sheets[idx].unlocked = true;
        self.sheets[idx].dirty = true;
        self.page_keys.insert(id.to_string(), key);
        self.persist_index(idx);
        Ok(())
    }

    pub fn unlock_page(&mut self, id: &str, password: &str) -> Result<(), String> {
        if let Some(folder) = self.covering_folder(id)
            && (self.encrypted_by(id, Some(&folder)) || self.page_wrap(id).is_none())
        {
            return self.unlock_folder(&folder, password);
        }
        let idx = self.idx(id).ok_or("That page is gone.")?;
        let sealed = self.sheets[idx]
            .sealed
            .clone()
            .ok_or("This page isn't locked.")?;
        let (text, key) = lock::open_password(password, &sealed, id.as_bytes())
            .map_err(|e| e.message().to_string())?;
        self.sheets[idx].text = text;
        self.sheets[idx].unlocked = true;
        self.page_keys.insert(id.to_string(), key);
        Ok(())
    }

    pub fn change_page_password(
        &mut self,
        id: &str,
        current: &str,
        new: &str,
    ) -> Result<(), String> {
        let idx = self.idx(id).ok_or("That page is gone.")?;
        if !self.sheets[idx].unlocked {
            return Err("Unlock this page first.".into());
        }
        let wrap = self.page_wrap(id).ok_or("This page isn't locked.")?;
        lock::unwrap_key(current, &wrap).map_err(|e| e.message().to_string())?;
        let key = self
            .page_keys
            .get(id)
            .cloned()
            .ok_or("Unlock this page first.")?;
        let wrap = lock::wrap_key(new, &key).map_err(|e| e.message().to_string())?;
        let text = self.sheets[idx].text.clone();
        let sealed =
            lock::seal(&key, &text, &wrap, id.as_bytes()).map_err(|e| e.message().to_string())?;
        self.sheets[idx].sealed = Some(sealed);
        self.sheets[idx].dirty = true;
        self.persist_index(idx);
        Ok(())
    }

    /// Check `password`, then store the page as plaintext. A wrong password
    /// changes nothing. An edit already on screen is kept once the password matches.
    pub fn remove_page_lock(&mut self, id: &str, password: &str) -> Result<(), String> {
        let idx = self.idx(id).ok_or("That page is gone.")?;
        if self.sheets[idx].sealed.is_none() {
            return Err("This page isn't locked.".into());
        }
        let cover = self.covering_folder(id);
        if self.encrypted_by(id, cover.as_deref()) {
            return Err("The folder is locked. Remove that lock instead.".into());
        }
        let sealed = self.sheets[idx].sealed.clone().unwrap();
        let (opened, _) = lock::open_password(password, &sealed, id.as_bytes())
            .map_err(|e| e.message().to_string())?;
        let text = if self.sheets[idx].unlocked {
            self.sheets[idx].text.clone()
        } else {
            opened
        };
        let idx = self.idx(id).unwrap();
        self.sheets[idx].text = text;
        self.sheets[idx].sealed = None;
        self.sheets[idx].unlocked = false;
        self.sheets[idx].dirty = true;
        self.page_keys.remove(id);
        self.persist_index(idx);
        Ok(())
    }

    pub fn lock_folder(&mut self, id: &str, password: &str) -> Result<(), String> {
        if self.folder_name(id).is_none() {
            return Err("That folder is gone.".into());
        }
        if self.folder_seal(id).is_some() {
            return Err("This folder is already locked.".into());
        }
        if self.ancestor_lock(id).is_some() {
            return Err("A folder around this one is already locked.".into());
        }
        let (key, wrap) = lock::create(password).map_err(|e| e.message().to_string())?;
        let seal = FolderSeal::from(&wrap);
        if !self.set_folder_seal(id, Some(seal)) {
            return Err("That folder is gone.".into());
        }
        self.folder_keys.insert(id.to_string(), key.clone());
        for pid in self.pages_under(id) {
            if self.individual_lock(&pid) {
                continue;
            }
            let Some(idx) = self.idx(&pid) else { continue };
            if self.sheets[idx].sealed.is_some() && !self.sheets[idx].unlocked {
                continue;
            }
            let text = self.sheets[idx].text.clone();
            let sealed = lock::seal(&key, &text, &wrap, pid.as_bytes())
                .map_err(|e| e.message().to_string())?;
            let idx = self.idx(&pid).unwrap();
            self.sheets[idx].sealed = Some(sealed);
            self.sheets[idx].unlocked = true;
            self.sheets[idx].dirty = true;
            self.page_keys.remove(&pid);
            self.persist_index(idx);
        }
        Ok(())
    }

    pub fn unlock_folder(&mut self, id: &str, password: &str) -> Result<(), String> {
        let seal = self
            .folder_seal(id)
            .cloned()
            .ok_or("This folder isn't locked.")?;
        let key = lock::unwrap_key(password, &seal.wrap()).map_err(|e| e.message().to_string())?;
        self.folder_keys.insert(id.to_string(), key.clone());
        for pid in self.pages_under(id) {
            if self.individual_lock(&pid) {
                continue;
            }
            self.reveal_with(&pid, &key);
        }
        Ok(())
    }

    pub fn change_folder_password(
        &mut self,
        id: &str,
        current: &str,
        new: &str,
    ) -> Result<(), String> {
        let seal = self
            .folder_seal(id)
            .cloned()
            .ok_or("This folder isn't locked.")?;
        lock::unwrap_key(current, &seal.wrap()).map_err(|e| e.message().to_string())?;
        let key = self
            .folder_keys
            .get(id)
            .cloned()
            .ok_or("Unlock this folder first.")?;
        let wrap = lock::wrap_key(new, &key).map_err(|e| e.message().to_string())?;
        self.set_folder_seal(id, Some(FolderSeal::from(&wrap)));
        for pid in self.pages_under(id) {
            if self.individual_lock(&pid) {
                continue;
            }
            let Some(idx) = self.idx(&pid) else { continue };
            if !self.sheets[idx].unlocked {
                continue;
            }
            let text = self.sheets[idx].text.clone();
            let sealed = lock::seal(&key, &text, &wrap, pid.as_bytes())
                .map_err(|e| e.message().to_string())?;
            let idx = self.idx(&pid).unwrap();
            self.sheets[idx].sealed = Some(sealed);
            self.sheets[idx].dirty = true;
            self.persist_index(idx);
        }
        Ok(())
    }

    pub fn remove_folder_lock(&mut self, id: &str) -> Result<(), String> {
        if self.folder_seal(id).is_none() {
            return Err("This folder isn't locked.".into());
        }
        if !self.folder_keys.contains_key(id) {
            return Err("Unlock this folder first.".into());
        }
        for pid in self.pages_under(id) {
            if !self.encrypted_by(&pid, Some(id)) {
                continue;
            }
            let Some(idx) = self.idx(&pid) else { continue };
            if !self.sheets[idx].unlocked {
                continue;
            }
            self.sheets[idx].sealed = None;
            self.sheets[idx].unlocked = false;
            self.sheets[idx].dirty = true;
            self.persist_index(idx);
        }
        self.folder_keys.remove(id);
        self.set_folder_seal(id, None);
        Ok(())
    }

    /// Drop every unlocked key and clear plaintext from memory. Files stay encrypted.
    pub fn lock_now(&mut self) {
        let ids: Vec<String> = self.sheets.iter().map(|s| s.id.clone()).collect();
        for id in ids {
            self.close_page(&id);
        }
        if let Some(deleted) = self.deleted.as_mut() {
            match &mut deleted.item {
                DeletedItem::Page(sheet) => close_detached(sheet),
                DeletedItem::Folder { sheets, .. } => {
                    for sheet in sheets {
                        close_detached(sheet);
                    }
                }
            }
        }
        self.page_keys.clear();
        self.folder_keys.clear();
    }

    /// Write changed sheets and the state. Keeps going past a failure, and returns
    /// the first error so it can be shown. A locked page is written as ciphertext.
    pub fn save(&mut self) -> Result<(), String> {
        let Some(dir) = self.dir.clone() else {
            return Ok(());
        };
        let mut first_error = None;
        let mut theirs = Vec::new();
        let dirty: Vec<usize> = self
            .sheets
            .iter()
            .enumerate()
            .filter(|(_, s)| s.dirty)
            .map(|(i, _)| i)
            .collect();
        for idx in dirty {
            let id = self.sheets[idx].id.clone();
            self.refresh_seal(&id);
            let Some(body) = self.body_for_disk(idx) else {
                first_error.get_or_insert("A locked page was left unsaved.".into());
                continue;
            };
            let path = dir.join(format!("{}.txt", self.sheets[idx].id));
            let title = self.sheets[idx].title();
            if self.sheets[idx].stamp.is_some()
                && stamp(&path) != self.sheets[idx].stamp
                && let Ok(text) = std::fs::read_to_string(&path)
                && text != body
            {
                self.conflicts.push(title.clone());
                theirs.push(text);
            }
            match write_atomic(&path, body.as_bytes()) {
                Ok(()) => {
                    self.sheets[idx].dirty = false;
                    self.sheets[idx].stamp = stamp(&path);
                }
                Err(e) => {
                    first_error.get_or_insert(format!("{title}: {e}"));
                }
            }
        }
        if !theirs.is_empty() {
            let active = self.sheets[self.active].id.clone();
            for text in theirs {
                self.new_sheet(&text);
            }
            self.active = self.sheets.iter().position(|s| s.id == active).unwrap_or(0);
            return self.save().and(first_error.map_or(Ok(()), Err));
        }
        let state = State {
            order: page_ids(&self.tree),
            active: self.sheets[self.active].id.clone(),
            cursors: self
                .sheets
                .iter()
                .map(|s| (s.id.clone(), s.cursor))
                .collect(),
            sidebar: Some(self.sidebar),
            sidebar_width: Some(self.sidebar_width),
            tree: Some(self.tree.clone()),
            handles: self.handles.clone(),
            seeded: self.seeded,
        };
        if let Ok(json) = serde_json::to_vec_pretty(&state)
            && let Err(e) = write_atomic(&dir.with_file_name("state.json"), &json)
        {
            first_error.get_or_insert(format!("state.json: {e}"));
        }
        self.state_dirty = false;
        first_error.map_or(Ok(()), Err)
    }

    fn idx(&self, id: &str) -> Option<usize> {
        self.sheets.iter().position(|s| s.id == id)
    }

    fn folder_is_open(&self, page_id: &str) -> bool {
        self.covering_folder(page_id)
            .is_some_and(|id| self.folder_keys.contains_key(&id))
    }

    /// Nearest locked folder at or above `id` (a page or a folder).
    fn covering_folder(&self, id: &str) -> Option<String> {
        fn walk(nodes: &[Node], id: &str, locked: Option<&str>) -> Option<String> {
            for node in nodes {
                match node {
                    Node::Page { id: pid } if pid == id => {
                        return locked.map(str::to_string);
                    }
                    Node::Folder {
                        id: fid,
                        children,
                        lock,
                        ..
                    } => {
                        let here = if lock.is_some() {
                            Some(fid.as_str())
                        } else {
                            locked
                        };
                        if fid == id {
                            return here.map(str::to_string);
                        }
                        if let Some(found) = walk(children, id, here) {
                            return Some(found);
                        }
                    }
                    Node::Page { .. } => {}
                }
            }
            None
        }
        walk(&self.tree, id, None)
    }

    /// A locked folder strictly above `id`, ignoring a lock on `id` itself.
    fn ancestor_lock(&self, id: &str) -> Option<String> {
        fn walk(nodes: &[Node], id: &str, locked: Option<&str>) -> Option<String> {
            for node in nodes {
                if let Node::Folder {
                    id: fid,
                    children,
                    lock,
                    ..
                } = node
                {
                    if fid == id {
                        return locked.map(str::to_string);
                    }
                    let here = if lock.is_some() {
                        Some(fid.as_str())
                    } else {
                        locked
                    };
                    if let Some(found) = walk(children, id, here) {
                        return Some(found);
                    }
                }
            }
            None
        }
        walk(&self.tree, id, None)
    }

    fn folder_seal(&self, id: &str) -> Option<&FolderSeal> {
        fn walk<'a>(nodes: &'a [Node], id: &str) -> Option<&'a FolderSeal> {
            for node in nodes {
                if let Node::Folder {
                    id: fid,
                    children,
                    lock,
                    ..
                } = node
                {
                    if fid == id {
                        return lock.as_ref();
                    }
                    if let Some(found) = walk(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        walk(&self.tree, id)
    }

    fn set_folder_seal(&mut self, id: &str, seal: Option<FolderSeal>) -> bool {
        fn walk(nodes: &mut [Node], id: &str, seal: &mut Option<FolderSeal>) -> bool {
            for node in nodes {
                if let Node::Folder {
                    id: fid,
                    children,
                    lock,
                    ..
                } = node
                {
                    if fid == id {
                        *lock = seal.take();
                        return true;
                    }
                    if walk(children, id, seal) {
                        return true;
                    }
                }
            }
            false
        }
        let mut seal = seal;
        let ok = walk(&mut self.tree, id, &mut seal);
        if ok {
            self.state_dirty = true;
        }
        ok
    }

    fn pages_under(&self, folder: &str) -> Vec<String> {
        fn walk(nodes: &[Node], folder: &str, inside: bool, out: &mut Vec<String>) {
            for node in nodes {
                match node {
                    Node::Page { id } if inside => out.push(id.clone()),
                    Node::Folder { id, children, .. } => {
                        walk(children, folder, inside || id == folder, out);
                    }
                    Node::Page { .. } => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.tree, folder, false, &mut out);
        out
    }

    fn page_wrap(&self, id: &str) -> Option<Wrap> {
        let sealed = self.sheets.iter().find(|s| s.id == id)?.sealed.as_deref()?;
        lock::wrap_of(sealed).ok()
    }

    fn encrypted_by(&self, page: &str, folder: Option<&str>) -> bool {
        let (Some(folder), Some(wrap)) = (folder, self.page_wrap(page)) else {
            return false;
        };
        self.folder_seal(folder)
            .is_some_and(|seal| seal.wrap() == wrap)
    }

    /// A password on the page itself, not the folder around it.
    fn individual_lock(&self, id: &str) -> bool {
        let Some(wrap) = self.page_wrap(id) else {
            return false;
        };
        match self.covering_folder(id) {
            Some(folder) => self
                .folder_seal(&folder)
                .is_none_or(|seal| seal.wrap() != wrap),
            None => !self.tree_has_wrap(&wrap),
        }
    }

    fn tree_has_wrap(&self, wrap: &Wrap) -> bool {
        fn walk(nodes: &[Node], wrap: &Wrap) -> bool {
            for node in nodes {
                if let Node::Folder { children, lock, .. } = node {
                    if lock.as_ref().is_some_and(|seal| seal.wrap() == *wrap) {
                        return true;
                    }
                    if walk(children, wrap) {
                        return true;
                    }
                }
            }
            false
        }
        walk(&self.tree, wrap)
    }

    fn can_move_page(&self, id: &str, parent: Option<&str>) -> bool {
        if self.individual_lock(id) {
            return true;
        }
        let Some(dest) = parent.and_then(|p| self.covering_folder(p)) else {
            return true;
        };
        if self.folder_keys.contains_key(&dest) {
            return true;
        }
        self.sheets
            .iter()
            .any(|s| s.id == id && s.sealed.is_some() && !s.unlocked)
    }

    fn rebind_page(&mut self, id: &str, old_cover: Option<&str>) {
        let new_cover = self.covering_folder(id);
        if new_cover.as_deref() == old_cover {
            return;
        }
        let Some(idx) = self.idx(id) else { return };
        let individual = self.sheets[idx].sealed.is_some() && !self.encrypted_by(id, old_cover);
        if individual {
            return;
        }
        if let Some(folder) = new_cover {
            let Some(key) = self.folder_keys.get(&folder).cloned() else {
                return;
            };
            let Some(wrap) = self.folder_seal(&folder).map(|s| s.wrap()) else {
                return;
            };
            if self.sheets[idx].unlocked || self.sheets[idx].sealed.is_none() {
                let text = self.sheets[idx].text.clone();
                if let Ok(sealed) = lock::seal(&key, &text, &wrap, id.as_bytes()) {
                    let idx = self.idx(id).unwrap();
                    self.sheets[idx].sealed = Some(sealed);
                    self.sheets[idx].unlocked = true;
                    self.sheets[idx].dirty = true;
                    self.page_keys.remove(id);
                    self.persist_index(idx);
                }
            }
        } else if self.sheets[idx].unlocked {
            self.sheets[idx].sealed = None;
            self.sheets[idx].unlocked = false;
            self.sheets[idx].dirty = true;
            self.page_keys.remove(id);
            self.persist_index(idx);
        }
    }

    fn key_for(&self, id: &str) -> Option<ContentKey> {
        if !self.individual_lock(id)
            && let Some(folder) = self.covering_folder(id)
            && let Some(key) = self.folder_keys.get(&folder)
        {
            return Some(key.clone());
        }
        self.page_keys.get(id).cloned()
    }

    fn wrap_for(&self, id: &str) -> Option<Wrap> {
        if !self.individual_lock(id)
            && let Some(folder) = self.covering_folder(id)
        {
            return self.folder_seal(&folder).map(|s| s.wrap());
        }
        self.page_wrap(id)
    }

    fn refresh_seal(&mut self, id: &str) {
        let Some(idx) = self.idx(id) else { return };
        if !self.sheets[idx].unlocked {
            return;
        }
        let Some(key) = self.key_for(id) else { return };
        let Some(wrap) = self.wrap_for(id) else {
            return;
        };
        let text = self.sheets[idx].text.clone();
        if let Ok(sealed) = lock::seal(&key, &text, &wrap, id.as_bytes()) {
            let idx = self.idx(id).unwrap();
            self.sheets[idx].sealed = Some(sealed);
        }
    }

    fn reveal_with(&mut self, id: &str, key: &ContentKey) {
        let Some(idx) = self.idx(id) else { return };
        let Some(sealed) = self.sheets[idx].sealed.clone() else {
            self.sheets[idx].unlocked = true;
            return;
        };
        if let Ok(text) = lock::open_key(key, &sealed, id.as_bytes()) {
            self.sheets[idx].text = text;
            self.sheets[idx].unlocked = true;
        }
    }

    fn reveal_all(&mut self) {
        let ids: Vec<String> = self.sheets.iter().map(|s| s.id.clone()).collect();
        for id in ids {
            if self.sheets.iter().any(|s| s.id == id && s.unlocked) {
                continue;
            }
            let Some(key) = self.key_for(&id) else {
                continue;
            };
            self.reveal_with(&id, &key);
        }
    }

    fn close_page(&mut self, id: &str) {
        let Some(idx) = self.idx(id) else { return };
        if self.sheets[idx].sealed.is_none() && self.covering_folder(id).is_none() {
            return;
        }
        self.refresh_seal(id);
        let Some(idx) = self.idx(id) else { return };
        if self.sheets[idx].sealed.is_none() {
            return;
        }
        self.sheets[idx].text.clear();
        self.sheets[idx].unlocked = false;
        self.sheets[idx].cursor = 0;
        self.sheets[idx].dirty = true;
        self.persist_index(idx);
    }

    fn body_for_disk(&self, idx: usize) -> Option<String> {
        let sheet = &self.sheets[idx];
        if let Some(sealed) = &sheet.sealed {
            return Some(sealed.clone());
        }
        if self.covering_folder(&sheet.id).is_some() {
            return None;
        }
        Some(sheet.text.clone())
    }

    fn persist_index(&mut self, idx: usize) {
        let Some(dir) = self.dir.clone() else {
            self.sheets[idx].dirty = true;
            return;
        };
        let Some(body) = self.body_for_disk(idx) else {
            self.sheets[idx].dirty = true;
            return;
        };
        let path = dir.join(format!("{}.txt", self.sheets[idx].id));
        if write_atomic(&path, body.as_bytes()).is_ok() {
            self.sheets[idx].dirty = false;
            self.sheets[idx].stamp = stamp(&path);
        } else {
            self.sheets[idx].dirty = true;
        }
    }
}

fn stored_body(sheet: &Sheet) -> Option<String> {
    if let Some(sealed) = &sheet.sealed {
        return Some(sealed.clone());
    }
    Some(sheet.text.clone())
}

fn close_detached(sheet: &mut Sheet) {
    if sheet.sealed.is_some() {
        sheet.text.clear();
        sheet.unlocked = false;
        sheet.cursor = 0;
    }
}

impl Deleted {
    fn contains_page(&self, id: &str) -> bool {
        self.page_ids().iter().any(|page| page == id)
    }

    fn page_ids(&self) -> Vec<String> {
        match &self.item {
            DeletedItem::Page(sheet) => vec![sheet.id.clone()],
            DeletedItem::Folder { sheets, .. } => sheets.iter().map(|s| s.id.clone()).collect(),
        }
    }
}

fn page_ids(nodes: &[Node]) -> Vec<String> {
    page_ids_in_nodes(nodes)
}

fn page_ids_in(node: &Node) -> Vec<String> {
    match node {
        Node::Page { id } => vec![id.clone()],
        Node::Folder { children, .. } => page_ids_in_nodes(children),
    }
}

fn page_ids_in_nodes(nodes: &[Node]) -> Vec<String> {
    let mut ids = Vec::new();
    for node in nodes {
        match node {
            Node::Page { id } => ids.push(id.clone()),
            Node::Folder { children, .. } => ids.extend(page_ids_in_nodes(children)),
        }
    }
    ids
}

fn side_entries(nodes: &[Node]) -> Vec<SideEntry> {
    fn walk(nodes: &[Node], depth: usize, out: &mut Vec<SideEntry>) {
        for node in nodes {
            match node {
                Node::Page { id } => out.push(SideEntry {
                    depth,
                    kind: SideKind::Page { id: id.clone() },
                }),
                Node::Folder {
                    id,
                    name,
                    collapsed,
                    children,
                    ..
                } => {
                    out.push(SideEntry {
                        depth,
                        kind: SideKind::Folder {
                            id: id.clone(),
                            name: name.clone(),
                            collapsed: *collapsed,
                        },
                    });
                    if !*collapsed {
                        walk(children, depth + 1, out);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(nodes, 0, &mut out);
    out
}

/// Drop pages that aren't on disk, and folders nested past [`MAX_FOLDER_DEPTH`].
/// Pages inside a too-deep folder move up to the deepest folder still allowed.
fn sanitize_tree(nodes: Vec<Node>, known: &HashSet<String>) -> Vec<Node> {
    fn walk(
        nodes: Vec<Node>,
        depth: usize,
        known: &HashSet<String>,
        seen: &mut HashSet<String>,
    ) -> Vec<Node> {
        let mut out = Vec::new();
        for node in nodes {
            match node {
                Node::Page { id } => {
                    if known.contains(&id) && seen.insert(id.clone()) {
                        out.push(Node::Page { id });
                    }
                }
                Node::Folder {
                    id,
                    name,
                    collapsed,
                    children,
                    lock,
                } => {
                    if depth >= MAX_FOLDER_DEPTH {
                        out.extend(walk(children, depth, known, seen));
                    } else if seen.insert(id.clone()) {
                        out.push(Node::Folder {
                            id,
                            name,
                            collapsed,
                            lock,
                            children: walk(children, depth + 1, known, seen),
                        });
                    } else {
                        out.extend(walk(children, depth, known, seen));
                    }
                }
            }
        }
        out
    }
    walk(nodes, 0, known, &mut HashSet::new())
}

fn folder_depth(nodes: &[Node], id: &str) -> Option<usize> {
    fn walk(nodes: &[Node], id: &str, depth: usize) -> Option<usize> {
        for node in nodes {
            if let Node::Folder {
                id: fid, children, ..
            } = node
            {
                if fid == id {
                    return Some(depth + 1);
                }
                if let Some(found) = walk(children, id, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(nodes, id, 0)
}

fn find_folder(nodes: &[Node], id: &str) -> Option<(String, bool, usize)> {
    fn walk(nodes: &[Node], id: &str, depth: usize) -> Option<(String, bool, usize)> {
        for node in nodes {
            if let Node::Folder {
                id: fid,
                name,
                collapsed,
                children,
                ..
            } = node
            {
                if fid == id {
                    return Some((name.clone(), *collapsed, depth + 1));
                }
                if let Some(found) = walk(children, id, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(nodes, id, 0)
}

fn children_len(nodes: &[Node], parent: Option<&str>) -> usize {
    match parent {
        None => nodes.len(),
        Some(id) => {
            fn walk(nodes: &[Node], id: &str) -> Option<usize> {
                for node in nodes {
                    if let Node::Folder {
                        id: fid, children, ..
                    } = node
                    {
                        if fid == id {
                            return Some(children.len());
                        }
                        if let Some(n) = walk(children, id) {
                            return Some(n);
                        }
                    }
                }
                None
            }
            walk(nodes, id).unwrap_or(0)
        }
    }
}

fn child_list<'a>(nodes: &'a mut Vec<Node>, parent: Option<&str>) -> Option<&'a mut Vec<Node>> {
    let Some(parent) = parent else {
        return Some(nodes);
    };
    for node in nodes.iter_mut() {
        if let Node::Folder { id, children, .. } = node {
            if id == parent {
                return Some(children);
            }
            if let Some(found) = child_list(children, Some(parent)) {
                return Some(found);
            }
        }
    }
    None
}

// A failed insert hands the node back so the caller can put it where it was.
#[allow(clippy::result_large_err)]
fn insert_node(
    nodes: &mut Vec<Node>,
    parent: Option<&str>,
    index: usize,
    node: Node,
) -> Result<(), Node> {
    let Some(list) = child_list(nodes, parent) else {
        return Err(node);
    };
    let index = index.min(list.len());
    list.insert(index, node);
    Ok(())
}

/// `(parent folder id, index among that parent's children, the node)`.
fn locate(nodes: &[Node], id: &str) -> Option<(Option<String>, usize, ())> {
    fn walk(nodes: &[Node], id: &str, parent: Option<&str>) -> Option<(Option<String>, usize)> {
        for (i, node) in nodes.iter().enumerate() {
            match node {
                Node::Page { id: pid } if pid == id => {
                    return Some((parent.map(str::to_string), i));
                }
                Node::Folder {
                    id: fid, children, ..
                } => {
                    if fid == id {
                        return Some((parent.map(str::to_string), i));
                    }
                    if let Some(found) = walk(children, id, Some(fid)) {
                        return Some(found);
                    }
                }
                Node::Page { .. } => {}
            }
        }
        None
    }
    walk(nodes, id, None).map(|(p, i)| (p, i, ()))
}

fn take_node(nodes: &mut Vec<Node>, id: &str) -> Option<(Option<String>, usize, Node)> {
    fn walk(
        nodes: &mut Vec<Node>,
        id: &str,
        parent: Option<&str>,
    ) -> Option<(Option<String>, usize, Node)> {
        let mut i = 0;
        while i < nodes.len() {
            let hit = match &nodes[i] {
                Node::Page { id: pid } => pid == id,
                Node::Folder { id: fid, .. } => fid == id,
            };
            if hit {
                return Some((parent.map(str::to_string), i, nodes.remove(i)));
            }
            if let Node::Folder {
                id: fid, children, ..
            } = &mut nodes[i]
            {
                let fid = fid.clone();
                if let Some(found) = walk(children, id, Some(&fid)) {
                    return Some(found);
                }
            }
            i += 1;
        }
        None
    }
    walk(nodes, id, None)
}

fn node_contains(node: &Node, id: &str) -> bool {
    match node {
        Node::Page { id: pid } => pid == id,
        Node::Folder {
            id: fid, children, ..
        } => fid == id || children.iter().any(|child| node_contains(child, id)),
    }
}

fn has_subfolder(node: &Node) -> bool {
    let Node::Folder { children, .. } = node else {
        return false;
    };
    children
        .iter()
        .any(|child| matches!(child, Node::Folder { .. }))
}

fn can_place(nodes: &[Node], node: &Node, parent: Option<&str>) -> bool {
    match node {
        Node::Page { .. } => match parent {
            None => true,
            Some(id) => folder_depth(nodes, id).is_some_and(|d| d <= MAX_FOLDER_DEPTH),
        },
        Node::Folder { id, .. } => {
            if parent.is_some_and(|p| p == id || node_contains(node, p)) {
                return false;
            }
            let depth = match parent {
                None => 1,
                Some(p) => match folder_depth(nodes, p) {
                    Some(d) => d + 1,
                    None => return false,
                },
            };
            depth <= MAX_FOLDER_DEPTH && (depth < MAX_FOLDER_DEPTH || !has_subfolder(node))
        }
    }
}

fn rename_folder(nodes: &mut [Node], id: &str, name: &str) -> bool {
    for node in nodes {
        if let Node::Folder {
            id: fid,
            name: current,
            children,
            ..
        } = node
        {
            if fid == id {
                *current = name.to_string();
                return true;
            }
            if rename_folder(children, id, name) {
                return true;
            }
        }
    }
    false
}

/// `collapsed = None` toggles. Returns whether a folder changed.
fn set_collapsed(nodes: &mut [Node], id: &str, collapsed: Option<bool>) -> bool {
    for node in nodes {
        if let Node::Folder {
            id: fid,
            collapsed: flag,
            children,
            ..
        } = node
        {
            if fid == id {
                let next = collapsed.unwrap_or(!*flag);
                if *flag == next {
                    return false;
                }
                *flag = next;
                return true;
            }
            if set_collapsed(children, id, collapsed) {
                return true;
            }
        }
    }
    false
}

fn swap_siblings(nodes: &mut Vec<Node>, parent: Option<&str>, a: usize, b: usize) -> bool {
    let Some(list) = child_list(nodes, parent) else {
        return false;
    };
    if a >= list.len() || b >= list.len() || a == b {
        return false;
    }
    list.swap(a, b);
    true
}

fn folder_names(nodes: &[Node]) -> Vec<String> {
    let mut names = Vec::new();
    fn walk(nodes: &[Node], names: &mut Vec<String>) {
        for node in nodes {
            if let Node::Folder { name, children, .. } = node {
                names.push(name.clone());
                walk(children, names);
            }
        }
    }
    walk(nodes, &mut names);
    names
}

fn fresh_folder_name(nodes: &[Node]) -> String {
    let names = folder_names(nodes);
    if !names.iter().any(|n| n == "New folder") {
        return "New folder".into();
    }
    let mut n = 2;
    loop {
        let candidate = format!("New folder {n}");
        if !names.iter().any(|name| name == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

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

fn read_sheet(dir: &Path, id: &str) -> Option<Sheet> {
    let path = dir.join(format!("{id}.txt"));
    let stamp = stamp(&path);
    let text = std::fs::read_to_string(&path).ok()?;
    Some(sheet_from_stored(id.to_string(), text, false, stamp))
}

fn plain_sheet(id: String, text: String) -> Sheet {
    sheet_from_stored(id, text, true, None)
}

/// A page file is either plain text or a lock envelope (`OMACUS-LOCK`, or a
/// future sync wrap around one). The envelope never becomes `text` until a
/// password opens it. The handle is filled in after load.
fn sheet_from_stored(id: String, raw: String, dirty: bool, stamp: Option<Stamp>) -> Sheet {
    if lock::is_envelope(&raw) {
        Sheet {
            id,
            text: String::new(),
            cursor: 0,
            handle: String::new(),
            dirty,
            stamp,
            sealed: Some(raw),
            unlocked: false,
        }
    } else {
        Sheet {
            id,
            cursor: raw.chars().count(),
            text: raw,
            handle: String::new(),
            dirty,
            stamp,
            sealed: None,
            unlocked: false,
        }
    }
}

impl Store {
    /// Give every page a handle, including ones already in Recently deleted.
    /// A title change later does not call this again.
    fn ensure_handles(&mut self) {
        let mut missing: Vec<(String, String)> = Vec::new();
        for sheet in &self.sheets {
            if sheet.handle.is_empty() && !self.handles.contains_key(&sheet.id) {
                missing.push((sheet.id.clone(), heading_of(&sheet.text)));
            }
        }
        if let Some(dir) = self.trash_dir()
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for entry in entries.flatten() {
                let Some(name) = entry.file_name().into_string().ok() else {
                    continue;
                };
                let Some(id) = name.strip_suffix(".txt") else {
                    continue;
                };
                if self.handles.contains_key(id) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                missing.push((id.to_string(), heading_of(&text)));
            }
        }
        if missing.is_empty() {
            for sheet in &mut self.sheets {
                if sheet.handle.is_empty()
                    && let Some(handle) = self.handles.get(&sheet.id)
                {
                    sheet.handle = handle.clone();
                }
            }
            return;
        }
        for (id, title) in missing {
            let handle = self.fresh_handle(&title);
            self.handles.insert(id, handle);
        }
        for sheet in &mut self.sheets {
            if let Some(handle) = self.handles.get(&sheet.id).cloned() {
                sheet.handle = handle;
            }
        }
        self.state_dirty = true;
    }

    fn taken_handles(&self) -> HashSet<String> {
        let mut taken: HashSet<String> = self.handles.values().cloned().collect();
        for sheet in &self.sheets {
            if !sheet.handle.is_empty() {
                taken.insert(sheet.handle.clone());
            }
        }
        taken
    }

    fn fresh_handle(&self, title: &str) -> String {
        xref::allocate_handle(title, &self.taken_handles())
    }

    pub fn remember_handle(&mut self, id: &str, handle: &str) {
        if let Some(sheet) = self.sheets.iter_mut().find(|s| s.id == id) {
            sheet.handle = handle.to_string();
        }
        self.handles.insert(id.to_string(), handle.to_string());
        self.state_dirty = true;
    }

    /// Write `text` back over a page that is not in the open list: the one just
    /// deleted, or a file in Recently deleted. Used when a handle or variable
    /// rename reaches a page that is not on screen.
    pub fn rewrite_saved(&mut self, id: &str, text: &str, handle: &str) -> bool {
        let mut wrote = false;
        if let Some(deleted) = &mut self.deleted {
            let sheets = match &mut deleted.item {
                DeletedItem::Page(sheet) => std::slice::from_mut(sheet),
                DeletedItem::Folder { sheets, .. } => sheets.as_mut_slice(),
            };
            if let Some(sheet) = sheets.iter_mut().find(|sheet| sheet.id == id) {
                if sheet.revealed() {
                    sheet.text = text.to_string();
                    wrote = true;
                }
                sheet.handle = handle.to_string();
            }
        }
        if let Some(path) = self.trash_dir().map(|d| d.join(format!("{id}.txt")))
            && path.exists()
        {
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            if !lock::is_envelope(&existing) {
                wrote = write_atomic(&path, text.as_bytes()).is_ok() || wrote;
            }
        }
        if wrote {
            self.handles.insert(id.to_string(), handle.to_string());
            self.state_dirty = true;
        }
        wrote
    }

    /// Pages deleted a moment ago, still available to undo, before the trash
    /// listing is read again.
    pub fn pending_sheets(&self) -> &[Sheet] {
        let Some(deleted) = &self.deleted else {
            return &[];
        };
        match &deleted.item {
            DeletedItem::Page(sheet) => std::slice::from_ref(sheet),
            DeletedItem::Folder { sheets, .. } => sheets,
        }
    }

    /// Another window wrote this file since we read it. A rename skips the page.
    pub fn opened_elsewhere(&self, sheet: &Sheet) -> bool {
        let Some(dir) = &self.dir else {
            return false;
        };
        let path = dir.join(format!("{}.txt", sheet.id));
        match (sheet.stamp, stamp(&path)) {
            (Some(saved), Some(now)) => saved != now,
            _ => false,
        }
    }
}

/// `pages/`, or the older `sheets/` folder when it can't be renamed.
fn pages_dir(root: &Path) -> PathBuf {
    let pages = root.join("pages");
    let legacy = root.join("sheets");
    if legacy.is_dir() && !pages.exists() {
        if std::fs::rename(&legacy, &pages).is_ok() {
            return pages;
        }
        return legacy;
    }
    if legacy.is_dir() && pages.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&legacy) {
            for entry in entries.flatten() {
                let dest = pages.join(entry.file_name());
                if !dest.exists() {
                    let _ = std::fs::rename(entry.path(), dest);
                }
            }
        }
        let _ = std::fs::remove_dir(&legacy);
    }
    pages
}

/// Earlier versions kept a single `sheet.txt`.
fn migrate_single_sheet(root: &Path, dir: &Path) {
    let old = root.join("sheet.txt");
    if old.exists() {
        let _ = std::fs::rename(&old, dir.join(format!("{}.txt", new_id())));
    }
}

/// The example pages, in sidebar order. The text lives in `site/app/examples.js`,
/// which the web app imports, so the two can't drift.
fn bundled_examples() -> Vec<String> {
    let src = include_str!("../site/app/examples.js");
    let mut pages = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find("text: `") {
        rest = &rest[i + "text: `".len()..];
        let Some(end) = rest.find('`') else {
            break;
        };
        pages.push(rest[..end].to_string());
        rest = &rest[end + 1..];
    }
    pages
}

fn dir_has_page(dir: &Path) -> bool {
    std::fs::read_dir(dir).ok().is_some_and(|rd| {
        rd.flatten()
            .any(|e| e.path().extension().is_some_and(|ext| ext == "txt"))
    })
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
    fn sidebar_width_comes_back_and_stays_in_range() {
        let root = temp_root("width");
        let mut store = Store::load_in(Some(root.clone()), "# Welcome");
        assert_eq!(store.sidebar_width, SIDEBAR_WIDTH);
        store.set_sidebar_width(10.0);
        assert_eq!(store.sidebar_width, SIDEBAR_MIN);
        store.set_sidebar_width(900.0);
        assert_eq!(store.sidebar_width, SIDEBAR_MAX);
        store.set_sidebar_width(280.0);
        store.toggle_sidebar();
        store.save().unwrap();

        let again = Store::load_in(Some(root.clone()), "# Welcome");
        assert!(!again.sidebar);
        assert_eq!(again.sidebar_width, 280.0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn old_state_without_a_sidebar_width_still_loads() {
        let root = temp_root("old-state");
        let sheets = root.join("sheets");
        std::fs::create_dir_all(&sheets).unwrap();
        std::fs::write(sheets.join("abc.txt"), "# Weekend Plans\n1+1").unwrap();
        std::fs::write(sheets.join("def.txt"), "# Other\n2").unwrap();
        // The shape written before the sidebar could be resized.
        std::fs::write(
            root.join("state.json"),
            r#"{"order":["def","abc"],"active":"def","cursors":{"def":3},"sidebar":false}"#,
        )
        .unwrap();
        let store = Store::load_in(Some(root.clone()), "# Welcome");
        assert_eq!(
            store.sheets.iter().map(Sheet::heading).collect::<Vec<_>>(),
            ["Other", "Weekend Plans"]
        );
        assert!(!store.sidebar);
        assert_eq!(store.sidebar_width, SIDEBAR_WIDTH);
        assert_eq!(store.sheets[0].cursor, 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn deleting_forever_leaves_nothing_to_restore() {
        let root = temp_root("purge");
        let mut store = Store::load_in(Some(root.clone()), "# Weekend Plans");
        store.delete(0);
        store.save().unwrap();
        let id = store.trashed()[0].id.clone();
        assert!(store.purge(&id));
        assert!(store.trashed().is_empty());
        assert!(store.restore(&id).is_none());
        assert!(store.undo_delete().is_none());
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
        let deleted_id = store.deleted.as_ref().unwrap().page_ids()[0].clone();
        let trashed =
            std::fs::read_to_string(root.join("trash").join(format!("{deleted_id}.txt"))).unwrap();
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
        let pages = root.join("pages");
        std::fs::set_permissions(&pages, std::fs::Permissions::from_mode(0o500)).unwrap();
        store.sheets[0].text = "# A\nmore".into();
        store.sheets[0].mark_dirty();
        let result = store.save();
        std::fs::set_permissions(&pages, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err(), "{result:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn two_windows_see_each_others_changes() {
        let root = temp_root("windows");
        let mut a = Store::load_in(Some(root.clone()), "# Shared\n1 + 1");
        a.save().unwrap();
        let mut b = Store::load_in(Some(root.clone()), "");
        // Some filesystems only keep whole seconds; make sure the next write looks new.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        // B edits; A, with nothing unsaved, picks it up.
        b.sheets[0].text = "# Shared\n2 + 2".into();
        b.sheets[0].mark_dirty();
        b.save().unwrap();
        let synced = a.sync();
        assert!(synced.active_changed);
        assert_eq!(a.sheets[0].text, "# Shared\n2 + 2");

        // B makes a sheet; A lists it.
        b.new_sheet("# From B");
        b.save().unwrap();
        a.sync();
        assert!(a.sheets.iter().any(|s| s.title() == "From B"));

        // B deletes it; A drops it.
        let idx = b.sheets.iter().position(|s| s.title() == "From B").unwrap();
        b.delete(idx);
        b.save().unwrap();
        a.sync();
        assert!(a.sheets.iter().all(|s| s.title() != "From B"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn editing_the_same_sheet_in_two_windows_keeps_both() {
        let root = temp_root("conflict");
        let mut a = Store::load_in(Some(root.clone()), "# Shared");
        a.save().unwrap();
        let mut b = Store::load_in(Some(root.clone()), "");
        std::thread::sleep(std::time::Duration::from_millis(1100));

        b.sheets[0].text = "# Shared\nfrom B".into();
        b.sheets[0].mark_dirty();
        b.save().unwrap();
        // A edited too, before it noticed.
        a.sheets[0].text = "# Shared\nfrom A".into();
        a.sheets[0].mark_dirty();
        a.save().unwrap();

        assert_eq!(a.take_conflicts(), ["Shared"]);
        let texts: Vec<_> = a.sheets.iter().map(|s| s.text.as_str()).collect();
        assert!(texts.contains(&"# Shared\nfrom A"), "{texts:?}");
        assert!(texts.contains(&"# Shared\nfrom B"), "{texts:?}");
        assert_eq!(a.current().text, "# Shared\nfrom A");
        let again = Store::load_in(Some(root.clone()), "");
        assert_eq!(again.sheets.len(), 2);
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
    fn an_old_sheets_folder_becomes_pages() {
        let root = temp_root("pages-dir");
        let legacy = root.join("sheets");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("abc.txt"), "# Weekend Plans\n1+1").unwrap();
        std::fs::write(
            root.join("state.json"),
            r#"{"order":["abc"],"active":"abc","cursors":{"abc":4}}"#,
        )
        .unwrap();
        let mut store = Store::load_in(Some(root.clone()), "# Welcome");
        assert_eq!(store.current().title(), "Weekend Plans");
        assert_eq!(store.current().cursor, 4);
        assert!(root.join("pages").join("abc.txt").is_file());
        assert!(!legacy.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pages_already_there_keep_a_leftover_sheets_file() {
        let root = temp_root("pages-both");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("sheets")).unwrap();
        std::fs::write(root.join("pages").join("new.txt"), "# New\n1").unwrap();
        std::fs::write(root.join("sheets").join("old.txt"), "# Old\n2").unwrap();
        let store = Store::load_in(Some(root.clone()), "");
        let titles: Vec<_> = store.sheets.iter().map(Sheet::title).collect();
        assert!(titles.contains(&"New".to_string()), "{titles:?}");
        assert!(titles.contains(&"Old".to_string()), "{titles:?}");
        assert!(root.join("pages").join("old.txt").is_file());
        assert!(!root.join("sheets").exists());
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

    #[test]
    fn an_empty_library_stays_empty() {
        let root = temp_root("empty-lib");
        let store = Store::open_at(root.clone());
        assert!(store.sheets.is_empty());
        assert_eq!(std::fs::read_dir(root.join("pages")).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_trashed_sheet_can_be_restored_after_reload() {
        let root = temp_root("restore");
        let mut store = Store::load_in(Some(root.clone()), "# Kept");
        store.new_sheet("# Gone\nmilk $3");
        store.save().unwrap();
        let id = store.sheets[0].id.clone();
        store.delete(0);
        store.save().unwrap();

        let mut again = Store::load_in(Some(root.clone()), "");
        assert!(again.undo_delete().is_none());
        let trash = again.trashed();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].id, id);
        assert_eq!(trash[0].title, "Gone");
        assert_eq!(again.restore(&id).as_deref(), Some("Gone"));
        assert_eq!(again.sheets[0].text, "# Gone\nmilk $3");
        assert!(again.trashed().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pages_without_a_tree_stay_at_the_root_in_order() {
        let root = temp_root("root-order");
        let pages = root.join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("trip.txt"), "# Trip Budget\nferry $40").unwrap();
        std::fs::write(pages.join("groceries.txt"), "# Groceries\nmilk $3").unwrap();
        std::fs::write(pages.join("weekend.txt"), "# Weekend Plans\nhike 6 km").unwrap();
        std::fs::write(
            root.join("state.json"),
            r#"{"order":["weekend","trip","groceries"],"active":"trip","cursors":{}}"#,
        )
        .unwrap();
        let store = Store::load_in(Some(root.clone()), "");
        assert_eq!(
            store.sheets.iter().map(Sheet::heading).collect::<Vec<_>>(),
            ["Weekend Plans", "Trip Budget", "Groceries"]
        );
        assert!(store.folders().is_empty());
        assert!(store.folder_of("trip").is_none());
        assert_eq!(store.sheets[store.active].heading(), "Trip Budget");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_keeps_its_pages_and_comes_back_collapsed() {
        let root = temp_root("folder-round");
        let mut store = Store::load_in(Some(root.clone()), "# Groceries\nmilk $3");
        store.new_sheet("# Trip Budget\nferry $40");
        store.new_sheet("# Weekend Plans\nhike 6 km");
        let folder = store.create_folder(None).unwrap();
        assert_eq!(store.folder_name(&folder).as_deref(), Some("New folder"));
        assert!(store.rename_folder(&folder, "Trip"));
        // Newest page is active and at the root. Move Weekend Plans into the folder.
        let weekend = store.sheets[0].id.clone();
        assert!(store.place(&weekend, Place::Into(folder.clone())));
        store.toggle_folder(&folder);
        store.save().unwrap();

        let again = Store::load_in(Some(root.clone()), "");
        let entries = again.side_entries();
        assert!(
            entries.iter().any(|e| matches!(
                &e.kind,
                SideKind::Folder { name, collapsed, .. } if name == "Trip" && *collapsed
            )),
            "{entries:?}"
        );
        // Collapsed, so the page inside is not a visible row, but the text is still there.
        assert!(
            again
                .side_entries()
                .iter()
                .all(|e| { !matches!(&e.kind, SideKind::Page { id } if id == &weekend) })
        );
        assert!(again.sheets.iter().any(|s| s.heading() == "Weekend Plans"));
        assert_eq!(again.folder_of(&weekend).as_deref(), Some(folder.as_str()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_page_keeps_its_handle_when_the_title_changes() {
        let root = temp_root("handles");
        let mut store = Store::load_in(Some(root.clone()), "# Trip Budget\ntotal = 5");
        assert_eq!(store.sheets[0].handle, "trip");
        store.sheets[0].text = "# Weekend Plans\ntotal = 5".into();
        store.sheets[0].mark_dirty();
        store.save().unwrap();
        let again = Store::load_in(Some(root.clone()), "");
        assert_eq!(again.sheets[0].handle, "trip");
        assert_eq!(again.sheets[0].title(), "Weekend Plans");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn deleting_a_folder_moves_its_pages_to_recently_deleted() {
        let root = temp_root("folder-trash");
        let mut store = Store::load_in(Some(root.clone()), "# Groceries");
        let folder = store.create_folder(None).unwrap();
        store.rename_folder(&folder, "Weekend");
        store.new_sheet_in(Some(&folder), "# Weekend Plans");
        store.new_sheet_in(Some(&folder), "# Trip Budget");
        let nested = store.create_folder(Some(&folder)).unwrap();
        store.new_sheet_in(Some(&nested), "# Groceries\ninside");
        store.save().unwrap();
        let inside = store
            .sheets
            .iter()
            .find(|s| s.text.contains("inside"))
            .unwrap()
            .id
            .clone();

        assert_eq!(store.delete_folder(&folder).as_deref(), Some("Weekend"));
        store.save().unwrap();
        let mut again = Store::load_in(Some(root.clone()), "");
        assert!(again.folders().is_empty());
        assert!(again.sheets.iter().all(|s| s.id != inside));
        let trash = again.trashed();
        let titles: Vec<_> = trash.iter().map(|t| t.title.as_str()).collect();
        assert!(titles.contains(&"Weekend Plans"), "{titles:?}");
        assert!(titles.contains(&"Trip Budget"), "{titles:?}");
        assert!(titles.contains(&"Groceries"), "{titles:?}");
        // Restore puts the page back at the root. The folder itself is gone.
        assert_eq!(again.restore(&inside).as_deref(), Some("Groceries"));
        assert!(again.folder_of(&inside).is_none());
        assert_eq!(again.sheets[0].text, "# Groceries\ninside");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn undo_puts_a_deleted_folder_back() {
        let root = temp_root("folder-undo");
        let mut store = Store::load_in(Some(root.clone()), "# Groceries");
        let folder = store.create_folder(None).unwrap();
        store.rename_folder(&folder, "Weekend");
        store.new_sheet_in(Some(&folder), "# Weekend Plans\ntrain $20");
        store.delete_folder(&folder);
        assert_eq!(store.undo_delete().as_deref(), Some("Weekend"));
        assert_eq!(store.folder_name(&folder).as_deref(), Some("Weekend"));
        let page = store
            .sheets
            .iter()
            .find(|s| s.heading() == "Weekend Plans")
            .unwrap();
        assert_eq!(store.folder_of(&page.id).as_deref(), Some(folder.as_str()));
        assert!(store.trashed().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_cannot_be_nested_three_deep() {
        let root = temp_root("folder-depth");
        let mut store = Store::load_in(Some(root.clone()), "# Trip Budget");
        let outer = store.create_folder(None).unwrap();
        let inner = store.create_folder(Some(&outer)).unwrap();
        assert!(store.create_folder(Some(&inner)).is_none());
        // A folder that already holds a folder cannot move inside another folder.
        let other = store.create_folder(None).unwrap();
        assert!(!store.place(&outer, Place::Into(other.clone())));
        assert!(store.place(&inner, Place::Into(other)));
        let page = store.sheets[0].id.clone();
        assert!(store.place(&page, Place::Into(inner.clone())));
        store.save().unwrap();
        let again = Store::load_in(Some(root.clone()), "");
        assert_eq!(again.folder_of(&page).as_deref(), Some(inner.as_str()));
        assert_eq!(again.folders().len(), 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn moving_a_page_out_of_a_folder_persists() {
        let root = temp_root("folder-move");
        let mut store = Store::load_in(Some(root.clone()), "# Groceries");
        let groceries = store.sheets[0].id.clone();
        store.new_sheet("# Weekend Plans");
        let weekend = store.sheets[0].id.clone();
        let folder = store.create_folder(None).unwrap();
        assert!(store.place(&groceries, Place::Into(folder.clone())));
        assert!(store.place(&weekend, Place::Before(groceries.clone())));
        assert_eq!(store.folder_of(&weekend).as_deref(), Some(folder.as_str()));
        assert!(store.place(&groceries, Place::RootEnd));
        assert!(store.folder_of(&groceries).is_none());
        store.save().unwrap();
        let again = Store::load_in(Some(root.clone()), "");
        assert!(again.folder_of(&groceries).is_none());
        assert_eq!(again.folder_of(&weekend).as_deref(), Some(folder.as_str()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_new_page_lands_in_the_open_pages_folder() {
        let root = temp_root("folder-new");
        let mut store = Store::load_in(Some(root.clone()), "# Groceries");
        let folder = store.create_folder(None).unwrap();
        store.new_sheet_in(Some(&folder), "# Trip Budget");
        store.select(
            store
                .sheets
                .iter()
                .position(|s| s.heading() == "Trip Budget")
                .unwrap(),
        );
        store.new_sheet("# Weekend Plans");
        let weekend = store
            .sheets
            .iter()
            .find(|s| s.heading() == "Weekend Plans")
            .unwrap()
            .id
            .clone();
        assert_eq!(store.folder_of(&weekend).as_deref(), Some(folder.as_str()));
        // It sits above the page that was open, still inside the folder.
        let ids = page_ids(&store.tree);
        let folder_pages: Vec<_> = store
            .tree
            .iter()
            .find_map(|n| match n {
                Node::Folder { id, children, .. } if id == &folder => Some(page_ids(children)),
                _ => None,
            })
            .unwrap();
        assert_eq!(folder_pages[0], weekend);
        assert!(ids.contains(&weekend));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_next_page_with_the_same_title_gets_another_handle() {
        let root = temp_root("handle-collision");
        let mut store = Store::load_in(Some(root.clone()), "# Trip Budget");
        store.new_sheet("# Trip Budget");
        assert_eq!(store.sheets[0].handle, "tripbudget");
        assert_eq!(store.sheets[1].handle, "trip");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locking_a_page_hides_it_until_the_password_and_comes_back() {
        let root = temp_root("lock-page");
        let mut store = Store::load_in(Some(root.clone()), "# Groceries\nmilk $3");
        let id = store.sheets[0].id.clone();
        store.lock_page(&id, "correct horse").unwrap();
        store.save().unwrap();
        let file = std::fs::read_to_string(root.join("pages").join(format!("{id}.txt"))).unwrap();
        assert!(file.starts_with("OMACUS-LOCK/1\n"), "{file}");
        assert!(!file.contains("Groceries"), "{file}");
        assert!(!file.contains("milk"), "{file}");

        store.lock_now();
        assert_eq!(store.sheets[0].heading(), LOCKED_TITLE);
        assert!(store.concealed(&id));
        assert_eq!(
            store.unlock_page(&id, "nope").unwrap_err(),
            "Wrong password."
        );
        assert!(store.concealed(&id));
        store.unlock_page(&id, "correct horse").unwrap();
        assert_eq!(store.sheets[0].text, "# Groceries\nmilk $3");

        let again = Store::load_in(Some(root.clone()), "");
        assert_eq!(again.sheets[0].heading(), LOCKED_TITLE);
        assert!(again.concealed(&id));
        let mut again = again;
        again.unlock_page(&id, "correct horse").unwrap();
        assert_eq!(again.sheets[0].text, "# Groceries\nmilk $3");
        again.remove_page_lock(&id, "correct horse").unwrap();
        again.save().unwrap();
        let plain = std::fs::read_to_string(root.join("pages").join(format!("{id}.txt"))).unwrap();
        assert_eq!(plain, "# Groceries\nmilk $3");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn removing_a_lock_needs_the_password_and_keeps_the_words() {
        let root = temp_root("lock-remove");
        let mut store = Store::load_in(Some(root.clone()), "# Groceries\nmilk $3");
        let id = store.sheets[0].id.clone();
        store.lock_page(&id, "correct horse").unwrap();
        store.lock_now();
        assert!(store.concealed(&id));
        assert_eq!(
            store.remove_page_lock(&id, "nope").unwrap_err(),
            "Wrong password."
        );
        assert!(store.concealed(&id));
        assert!(store.sheets[0].sealed.is_some());
        let sealed = std::fs::read_to_string(root.join("pages").join(format!("{id}.txt"))).unwrap();
        assert!(sealed.starts_with("OMACUS-LOCK/1\n"), "{sealed}");

        store.remove_page_lock(&id, "correct horse").unwrap();
        assert!(!store.concealed(&id));
        assert!(store.sheets[0].sealed.is_none());
        assert_eq!(store.sheets[0].text, "# Groceries\nmilk $3");
        store.save().unwrap();
        let plain = std::fs::read_to_string(root.join("pages").join(format!("{id}.txt"))).unwrap();
        assert_eq!(plain, "# Groceries\nmilk $3");

        store.lock_page(&id, "correct horse").unwrap();
        store.sheets[0].text = "# Groceries\nmilk $4".into();
        store.sheets[0].dirty = true;
        assert_eq!(
            store.remove_page_lock(&id, "nope").unwrap_err(),
            "Wrong password."
        );
        assert_eq!(store.sheets[0].text, "# Groceries\nmilk $4");
        assert!(store.sheets[0].sealed.is_some());
        store.remove_page_lock(&id, "correct horse").unwrap();
        assert_eq!(store.sheets[0].text, "# Groceries\nmilk $4");
        assert!(store.sheets[0].sealed.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_locked_folder_encrypts_pages_inside_including_a_new_one() {
        let root = temp_root("lock-folder");
        let mut store = Store::load_in(Some(root.clone()), "# Groceries\nmilk $3");
        let page = store.sheets[0].id.clone();
        let folder = store.create_folder(None).unwrap();
        store.rename_folder(&folder, "Trip");
        assert!(store.place(&page, Place::Into(folder.clone())));
        store.lock_folder(&folder, "folder-secret").unwrap();
        store.save().unwrap();
        let file = std::fs::read_to_string(root.join("pages").join(format!("{page}.txt"))).unwrap();
        assert!(
            !file.contains("Groceries") && !file.contains("milk"),
            "{file}"
        );

        store.new_sheet_in(Some(&folder), "# Weekend Plans\nhike 6 km");
        let weekend = store
            .sheets
            .iter()
            .find(|s| s.text.contains("Weekend"))
            .unwrap()
            .id
            .clone();
        store.save().unwrap();
        let nested =
            std::fs::read_to_string(root.join("pages").join(format!("{weekend}.txt"))).unwrap();
        assert!(
            !nested.contains("Weekend") && !nested.contains("hike"),
            "{nested}"
        );

        store.lock_now();
        assert!(store.concealed(&page));
        assert!(!store.new_sheet_in(Some(&folder), "# Trip Budget"));
        store.unlock_folder(&folder, "folder-secret").unwrap();
        assert_eq!(
            store.sheets.iter().find(|s| s.id == page).unwrap().text,
            "# Groceries\nmilk $3"
        );
        store.delete(store.sheets.iter().position(|s| s.id == page).unwrap());
        let trashed = store.trashed();
        let item = trashed.iter().find(|t| t.id == page).unwrap();
        assert!(item.locked);
        assert!(!item.text.contains("milk"), "{}", item.text);
        assert_eq!(item.title, LOCKED_TITLE);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_closed_folder_refuses_plaintext_moved_in_until_it_is_open() {
        let root = temp_root("lock-move");
        let mut store = Store::load_in(Some(root.clone()), "# Groceries\nmilk $3");
        let page = store.sheets[0].id.clone();
        let trip = store.create_folder(None).unwrap();
        store.rename_folder(&trip, "Trip");
        store.lock_folder(&trip, "folder-secret").unwrap();
        store.lock_now();
        assert!(!store.place(&page, Place::Into(trip.clone())));
        store.save().unwrap();
        let path = root.join("pages").join(format!("{page}.txt"));
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.contains("Groceries"), "{file}");

        store.new_sheet("# Weekend Plans\nhike");
        let weekend = store.sheets[0].id.clone();
        let bag = store.create_folder(None).unwrap();
        store.rename_folder(&bag, "Bag");
        assert!(store.place(&weekend, Place::Into(bag.clone())));
        assert!(!store.place(&bag, Place::Into(trip.clone())));
        store.save().unwrap();
        let weekend_file =
            std::fs::read_to_string(root.join("pages").join(format!("{weekend}.txt"))).unwrap();
        assert!(weekend_file.contains("Weekend"), "{weekend_file}");

        store.unlock_folder(&trip, "folder-secret").unwrap();
        assert!(store.place(&page, Place::Into(trip.clone())));
        store.save().unwrap();
        let locked = std::fs::read_to_string(&path).unwrap();
        assert!(locked.starts_with("OMACUS-LOCK/1\n"), "{locked}");
        assert!(!locked.contains("Groceries"), "{locked}");
        let _ = std::fs::remove_dir_all(root);
    }

    fn open_seeded(root: PathBuf, welcome: &str) -> Store {
        let mut store = Store::load_in(Some(root), welcome);
        store.seed_first_run();
        store
    }

    #[test]
    fn first_run_seeds_examples_beside_welcome_and_only_once() {
        let root = temp_root("seed-once");
        let welcome = "# Welcome to Omacus\n1 + 1";
        let mut store = open_seeded(root.clone(), welcome);
        assert!(store.seeded);
        assert_eq!(store.current().heading(), "Welcome to Omacus");
        let welcome_id = store.current().id.clone();
        assert!(store.folder_of(&welcome_id).is_none());
        let names: Vec<_> = store.folders().into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["Examples"]);
        let titles: Vec<_> = store.sheets.iter().map(Sheet::heading).collect::<Vec<_>>();
        for title in [
            "Welcome to Omacus",
            "Trip Budget",
            "Groceries",
            "Monthly budget",
            "Cooking",
            "Time zones",
        ] {
            assert!(titles.contains(&title.to_string()), "{titles:?}");
        }
        assert_eq!(titles.len(), 6);
        let trip = store
            .sheets
            .iter()
            .find(|s| s.heading() == "Trip Budget")
            .map(|s| (s.id.clone(), s.handle.clone()))
            .unwrap();
        let groceries = store
            .sheets
            .iter()
            .find(|s| s.heading() == "Groceries")
            .map(|s| s.handle.clone())
            .unwrap();
        assert_eq!(trip.1, "trip");
        assert_eq!(groceries, "groceries");
        assert!(store.folder_of(&trip.0).is_some());
        let monthly = store
            .sheets
            .iter()
            .find(|s| s.heading() == "Monthly budget")
            .map(|s| s.text.clone())
            .unwrap();
        assert!(monthly.contains("@trip.total"));
        assert!(monthly.contains("@groceries.total"));

        let again = open_seeded(root.clone(), welcome);
        assert_eq!(again.sheets.len(), 6);
        assert_eq!(
            again
                .sheets
                .iter()
                .map(|s| s.id.clone())
                .collect::<Vec<_>>(),
            store
                .sheets
                .iter()
                .map(|s| s.id.clone())
                .collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn deleted_examples_are_not_seeded_again() {
        let root = temp_root("seed-delete");
        let mut store = open_seeded(root.clone(), "# Welcome to Omacus");
        let folder = store.folders()[0].id.clone();
        store.delete_folder(&folder);
        while store
            .sheets
            .iter()
            .any(|s| s.heading() == "Welcome to Omacus")
        {
            let i = store
                .sheets
                .iter()
                .position(|s| s.heading() == "Welcome to Omacus")
                .unwrap();
            store.delete(i);
        }
        store.save().unwrap();
        let ids: Vec<_> = store.trashed().into_iter().map(|t| t.id).collect();
        for id in ids {
            store.purge(&id);
        }
        store.save().unwrap();

        let again = open_seeded(root.clone(), "# Welcome to Omacus");
        let titles: Vec<_> = again.sheets.iter().map(Sheet::heading).collect::<Vec<_>>();
        assert!(again.folders().is_empty(), "{titles:?}");
        assert!(!titles.iter().any(|t| t == "Trip Budget"), "{titles:?}");
        assert!(!titles.iter().any(|t| t == "Groceries"), "{titles:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_existing_library_is_marked_and_not_overwritten() {
        let root = temp_root("seed-keep");
        let pages = root.join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        let body = "# Mine\n2 + 2\n";
        std::fs::write(pages.join("mine.txt"), body).unwrap();
        std::fs::write(
            root.join("state.json"),
            r#"{"order":["mine"],"active":"mine","cursors":{}}"#,
        )
        .unwrap();
        let mut store = open_seeded(root.clone(), "# Welcome to Omacus");
        assert!(store.seeded);
        assert!(store.folders().is_empty());
        assert_eq!(store.sheets.len(), 1);
        assert_eq!(store.current().heading(), "Mine");
        assert_eq!(
            std::fs::read_to_string(pages.join("mine.txt")).unwrap(),
            body
        );

        store.delete(0);
        store.save().unwrap();
        for id in store
            .trashed()
            .into_iter()
            .map(|t| t.id)
            .collect::<Vec<_>>()
        {
            store.purge(&id);
        }
        store.save().unwrap();
        let again = open_seeded(root.clone(), "# Welcome to Omacus");
        let titles: Vec<_> = again.sheets.iter().map(Sheet::heading).collect::<Vec<_>>();
        assert!(!titles.iter().any(|t| t == "Trip Budget"), "{titles:?}");
        assert!(again.folders().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
