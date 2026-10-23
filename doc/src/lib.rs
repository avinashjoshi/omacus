//! A local library document.
//!
//! Pages are lists of `{id, text}` lines. The same line changed two ways stays
//! as two choices until someone picks one. A number is never merged character
//! by character. Two libraries that were never the same document are kept side
//! by side. Nothing here opens a socket.

use std::collections::HashSet;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ChangeHash, ObjType, ROOT, ReadDoc, ScalarValue, Value};
use sha2::{Digest, Sha256};

/// Every library belongs to this vault until a person has more than one.
pub const VAULT_ID: &str = "personal";

/// A folder may contain another folder. Anything deeper is refused.
pub const MAX_FOLDER_DEPTH: usize = 2;

const LOCK_PREFIX: &str = "OMACUS-LOCK/";

/// One line that was edited two ways.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineConflict {
    pub page_id: String,
    pub line_id: String,
    pub ours: String,
    pub theirs: String,
}

/// A folder as a parent pointer. `parent` is another folder, or the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
}

/// What to store when a library is first built from pages that already exist.
#[derive(Clone, Debug)]
pub struct FolderSeed {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceLine {
    id: String,
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Keep {
        id: Option<String>,
        text: String,
    },
    Conflict {
        id: Option<String>,
        ours: String,
        theirs: String,
    },
}

/// The library document. The bytes from [`Library::save`] are the source of
/// truth; page files are a snapshot written out from it.
pub struct Library {
    doc: AutoCommit,
}

impl Library {
    pub fn new() -> Self {
        let mut doc = AutoCommit::new();
        let actor = doc.get_actor().to_string();
        let _ = doc.put(ROOT, "doc", actor.as_str());
        let _ = doc.put(ROOT, "vault", VAULT_ID);
        let _ = doc.put_object(ROOT, "pages", ObjType::List);
        let _ = doc.put_object(ROOT, "folders", ObjType::List);
        let _ = doc.put(ROOT, "next", 1_i64);
        doc.commit();
        Self { doc }
    }

    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        let doc = AutoCommit::load(bytes).map_err(|e| e.to_string())?;
        let mut lib = Self { doc };
        lib.ensure_root();
        lib.break_folder_problems();
        Ok(lib)
    }

    pub fn save(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    pub fn doc_id(&self) -> String {
        self.root_str("doc")
            .unwrap_or_else(|| self.doc.get_actor().to_string())
    }

    pub fn vault(&self) -> String {
        self.root_str("vault")
            .unwrap_or_else(|| VAULT_ID.to_string())
    }

    /// Heads after the changes so far, so an older line can still be read.
    pub fn heads(&mut self) -> Vec<ChangeHash> {
        self.doc.commit();
        self.doc.get_heads()
    }

    /// The page as it was at `heads`. Missing pages return `None`.
    pub fn page_text_at(&mut self, page_id: &str, heads: &[ChangeHash]) -> Option<String> {
        self.doc.commit();
        let pages = self.child(&ROOT, "pages")?;
        let n = self.doc.length_at(&pages, heads);
        for i in 0..n {
            let (_, page) = self.doc.get_at(&pages, i, heads).ok().flatten()?;
            if self.str_at_heads(&page, "id", heads).as_deref() != Some(page_id) {
                continue;
            }
            if bool_at_heads(&self.doc, &page, "sealed", heads) {
                return self.str_at_heads(&page, "body", heads);
            }
            let lines = object_at(&self.doc, &page, "lines", heads)?;
            let ln = self.doc.length_at(&lines, heads);
            let mut parts = Vec::with_capacity(ln);
            for j in 0..ln {
                let (_, line) = self.doc.get_at(&lines, j, heads).ok().flatten()?;
                parts.push(self.str_at_heads(&line, "text", heads).unwrap_or_default());
            }
            return Some(parts.join("\n"));
        }
        None
    }

    pub fn page_ids(&self) -> Vec<String> {
        let Some(pages) = self.child(&ROOT, "pages") else {
            return Vec::new();
        };
        let n = self.doc.length(&pages);
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let Some((_, page)) = self.doc.get(&pages, i).ok().flatten() else {
                continue;
            };
            if let Some(id) = self.str_at(&page, "id") {
                out.push(id);
            }
        }
        out
    }

    /// Current text only. Deleted lines are not part of this.
    pub fn snapshot(&self, page_id: &str) -> Option<String> {
        let (_, page) = self.find_page(page_id)?;
        if self.bool_at(&page, "sealed") {
            return self.str_at(&page, "body");
        }
        let lines = self.child(&page, "lines")?;
        let n = self.doc.length(&lines);
        let mut parts = Vec::with_capacity(n);
        for i in 0..n {
            let Some((_, line)) = self.doc.get(&lines, i).ok().flatten() else {
                continue;
            };
            parts.push(self.str_at(&line, "text").unwrap_or_default());
        }
        Some(parts.join("\n"))
    }

    pub fn written_hash(&self, page_id: &str) -> Option<String> {
        let (_, page) = self.find_page(page_id)?;
        self.str_at(&page, "written")
    }

    pub fn note_written(&mut self, page_id: &str, body: &str) {
        self.doc.commit();
        let Some((_, page)) = self.find_page(page_id) else {
            return;
        };
        let _ = self.doc.put(&page, "written", content_hash(body).as_str());
        self.doc.commit();
    }

    /// Fold `local` and an optional on-disk copy into the page.
    ///
    /// The lines already stored are the last snapshot. A line changed on only
    /// one side is kept. The same line changed on both sides becomes a conflict
    /// and the page keeps `local` until [`Library::pick`].
    pub fn reconcile(
        &mut self,
        page_id: &str,
        local: &str,
        disk: Option<&str>,
    ) -> Vec<LineConflict> {
        self.doc.commit();
        if is_sealed(local) || disk.is_some_and(is_sealed) {
            return self.reconcile_sealed(page_id, local, disk);
        }
        let base = self.base_lines(page_id);
        let ours = split_lines(local);
        let pieces = if let Some(disk) = disk {
            merge_pieces(&base, &ours, &split_lines(disk))
        } else {
            let base_text: Vec<String> = base.iter().map(|l| l.text.clone()).collect();
            merge_pieces(&base, &ours, &base_text)
        };
        self.install(page_id, &pieces)
    }

    pub fn conflicts(&self) -> Vec<LineConflict> {
        let mut out = Vec::new();
        for id in self.page_ids() {
            let Some((_, page)) = self.find_page(&id) else {
                continue;
            };
            for (line_id, ours, theirs) in self.read_conflicts(&page) {
                out.push(LineConflict {
                    page_id: id.clone(),
                    line_id,
                    ours,
                    theirs,
                });
            }
        }
        out
    }

    /// `theirs` chooses the other copy. Otherwise the line stays as it is.
    pub fn pick(&mut self, page_id: &str, line_id: &str, theirs: bool) -> bool {
        self.doc.commit();
        let Some((_, page)) = self.find_page(page_id) else {
            return false;
        };
        let pending = self.read_conflicts(&page);
        let Some((_, ours, other)) = pending.iter().find(|(id, _, _)| id == line_id) else {
            return false;
        };
        let chosen = if theirs { other.clone() } else { ours.clone() };
        if line_id == "body" || self.bool_at(&page, "sealed") {
            let _ = self.doc.put(&page, "body", chosen.as_str());
            let _ = self.doc.put(&page, "sealed", true);
        } else if let Some(lines) = self.child(&page, "lines") {
            let n = self.doc.length(&lines);
            for i in 0..n {
                let Some((_, line)) = self.doc.get(&lines, i).ok().flatten() else {
                    continue;
                };
                if self.str_at(&line, "id").as_deref() == Some(line_id) {
                    let _ = self.doc.put(&line, "text", chosen.as_str());
                    break;
                }
            }
        }
        let rest: Vec<_> = pending
            .into_iter()
            .filter(|(id, _, _)| id != line_id)
            .collect();
        self.write_conflicts(&page, &rest);
        self.doc.commit();
        true
    }

    pub fn remove_page(&mut self, page_id: &str) -> bool {
        self.doc.commit();
        let Some((idx, _)) = self.find_page(page_id) else {
            return false;
        };
        let Some(pages) = self.child(&ROOT, "pages") else {
            return false;
        };
        if self.doc.delete(&pages, idx).is_err() {
            return false;
        }
        self.doc.commit();
        true
    }

    pub fn folders(&self) -> Vec<Folder> {
        let Some(list) = self.child(&ROOT, "folders") else {
            return Vec::new();
        };
        let n = self.doc.length(&list);
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let Some((_, folder)) = self.doc.get(&list, i).ok().flatten() else {
                continue;
            };
            let Some(id) = self.str_at(&folder, "id") else {
                continue;
            };
            out.push(Folder {
                id,
                name: self.str_at(&folder, "name").unwrap_or_default(),
                parent: self.str_at(&folder, "parent"),
            });
        }
        out
    }

    pub fn parent_of(&self, id: &str) -> Option<String> {
        self.folders()
            .into_iter()
            .find(|f| f.id == id)
            .and_then(|f| f.parent)
    }

    /// Point `id` at `parent`. A loop is broken by clearing the pointer.
    /// A folder nested deeper than [`MAX_FOLDER_DEPTH`] is left where it was.
    pub fn set_parent(&mut self, id: &str, parent: Option<&str>) -> bool {
        self.doc.commit();
        if self.find_folder(id).is_none() {
            return false;
        }
        let Some(parent) = parent else {
            self.write_parent(id, None);
            return true;
        };
        if parent == id || self.find_folder(parent).is_none() || self.chain_contains(parent, id) {
            self.write_parent(id, None);
            return false;
        }
        if self.depth_of(parent) + 1 > MAX_FOLDER_DEPTH {
            return false;
        }
        self.write_parent(id, Some(parent));
        true
    }

    pub fn set_folders(&mut self, folders: &[FolderSeed]) {
        self.doc.commit();
        self.clear_list(&ROOT, "folders");
        for folder in folders {
            self.insert_folder(&folder.id, &folder.name);
        }
        for folder in folders {
            let _ = self.set_parent(&folder.id, folder.parent.as_deref());
        }
    }

    /// Same document: merge history, and surface same-line conflicts.
    /// A different document: keep every page, and mint a new id when one collides.
    /// Unrelated pages are never merged character by character.
    pub fn union(mut self, mut other: Self) -> Self {
        self.doc.commit();
        other.doc.commit();
        if self.doc_id() == other.doc_id() {
            let _ = self.doc.merge(&mut other.doc);
            self.doc.commit();
            self.harvest_conflicts();
            self.ensure_root();
            self
        } else {
            self.absorb(&other);
            self
        }
    }

    /// A conflict for screenshots and tests. The page shows `line_text`.
    pub fn add_conflict_on(&mut self, page_id: &str, line_text: &str, theirs: &str) -> bool {
        self.doc.commit();
        let Some((_, page)) = self.find_page(page_id) else {
            return false;
        };
        let Some(lines) = self.child(&page, "lines") else {
            return false;
        };
        let n = self.doc.length(&lines);
        for i in 0..n {
            let Some((_, line)) = self.doc.get(&lines, i).ok().flatten() else {
                continue;
            };
            if self.str_at(&line, "text").as_deref() != Some(line_text) {
                continue;
            }
            let Some(id) = self.str_at(&line, "id") else {
                continue;
            };
            let mut pending = self.read_conflicts(&page);
            pending.push((id, line_text.to_string(), theirs.to_string()));
            self.write_conflicts(&page, &pending);
            self.doc.commit();
            return true;
        }
        false
    }

    fn ensure_root(&mut self) {
        if self.root_str("doc").is_none() {
            let actor = self.doc.get_actor().to_string();
            let _ = self.doc.put(ROOT, "doc", actor.as_str());
        }
        if self.root_str("vault").is_none() {
            let _ = self.doc.put(ROOT, "vault", VAULT_ID);
        }
        if self.child(&ROOT, "pages").is_none() {
            let _ = self.doc.put_object(ROOT, "pages", ObjType::List);
        }
        if self.child(&ROOT, "folders").is_none() {
            let _ = self.doc.put_object(ROOT, "folders", ObjType::List);
        }
        self.doc.commit();
    }

    fn reconcile_sealed(
        &mut self,
        page_id: &str,
        local: &str,
        disk: Option<&str>,
    ) -> Vec<LineConflict> {
        let base = self.snapshot(page_id).unwrap_or_default();
        let Some(disk) = disk else {
            self.write_sealed(page_id, local, &[]);
            return Vec::new();
        };
        if disk == local || disk == base {
            self.write_sealed(page_id, local, &[]);
            return Vec::new();
        }
        if local == base {
            self.write_sealed(page_id, disk, &[]);
            return Vec::new();
        }
        let conflict = LineConflict {
            page_id: page_id.to_string(),
            line_id: "body".into(),
            ours: local.to_string(),
            theirs: disk.to_string(),
        };
        self.write_sealed(
            page_id,
            local,
            &[(
                conflict.line_id.clone(),
                conflict.ours.clone(),
                conflict.theirs.clone(),
            )],
        );
        vec![conflict]
    }

    fn write_sealed(&mut self, page_id: &str, body: &str, conflicts: &[(String, String, String)]) {
        let page = self.ensure_page(page_id);
        let _ = self.doc.put(&page, "sealed", true);
        let _ = self.doc.put(&page, "body", body);
        if let Some(lines) = self.child(&page, "lines") {
            self.clear_obj_list(&lines);
        } else {
            let _ = self.doc.put_object(&page, "lines", ObjType::List);
        }
        self.write_conflicts(&page, conflicts);
        self.doc.commit();
    }

    fn base_lines(&self, page_id: &str) -> Vec<SourceLine> {
        let Some((_, page)) = self.find_page(page_id) else {
            return Vec::new();
        };
        if self.bool_at(&page, "sealed") {
            return Vec::new();
        }
        let Some(lines) = self.child(&page, "lines") else {
            return Vec::new();
        };
        let n = self.doc.length(&lines);
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let Some((_, line)) = self.doc.get(&lines, i).ok().flatten() else {
                continue;
            };
            let Some(id) = self.str_at(&line, "id") else {
                continue;
            };
            out.push(SourceLine {
                id,
                text: self.str_at(&line, "text").unwrap_or_default(),
            });
        }
        out
    }

    fn install(&mut self, page_id: &str, pieces: &[Piece]) -> Vec<LineConflict> {
        let page = self.ensure_page(page_id);
        let _ = self.doc.put(&page, "sealed", false);
        let _ = self.doc.delete(&page, "body");
        let list = if let Some(lines) = self.child(&page, "lines") {
            lines
        } else {
            self.doc
                .put_object(&page, "lines", ObjType::List)
                .unwrap_or_else(|_| self.child(&page, "lines").unwrap_or(ROOT))
        };
        self.doc.commit();
        let mut have = self.line_refs(&list);
        let keep: HashSet<String> = pieces.iter().filter_map(piece_id).collect();
        let drop_at: Vec<usize> = have
            .iter()
            .enumerate()
            .filter(|(_, line)| !keep.contains(&line.id))
            .map(|(i, _)| i)
            .collect();
        for idx in drop_at.into_iter().rev() {
            let _ = self.doc.delete(&list, idx);
        }
        self.doc.commit();
        have = self.line_refs(&list);

        let mut conflicts = Vec::new();
        let mut at = 0usize;
        for piece in pieces {
            let (id_opt, text, theirs) = match piece {
                Piece::Keep { id, text } => (id.clone(), text.clone(), None),
                Piece::Conflict { id, ours, theirs } => {
                    (id.clone(), ours.clone(), Some(theirs.clone()))
                }
            };
            let id = if let Some(id) = id_opt.clone() {
                if have.iter().any(|line| line.id == id) {
                    id
                } else {
                    self.mint()
                }
            } else {
                self.mint()
            };
            if let Some(pos) = have.iter().position(|line| line.id == id) {
                if pos != at {
                    let _ = self.doc.delete(&list, pos);
                    self.doc.commit();
                    let ex = self
                        .doc
                        .insert_object(&list, at, ObjType::Map)
                        .unwrap_or(ROOT);
                    let _ = self.doc.put(&ex, "id", id.as_str());
                    let _ = self.doc.put(&ex, "text", text.as_str());
                } else if have[pos].text != text {
                    let _ = self.doc.put(&have[pos].exid, "text", text.as_str());
                }
            } else {
                let ex = self
                    .doc
                    .insert_object(&list, at, ObjType::Map)
                    .unwrap_or(ROOT);
                let _ = self.doc.put(&ex, "id", id.as_str());
                let _ = self.doc.put(&ex, "text", text.as_str());
            }
            self.doc.commit();
            have = self.line_refs(&list);
            if let Some(theirs) = theirs {
                conflicts.push((id, text, theirs));
            }
            at += 1;
        }
        let mut guard = self.doc.length(&list);
        while at < self.doc.length(&list) && guard > 0 {
            guard -= 1;
            let last = self.doc.length(&list) - 1;
            if self.doc.delete(&list, last).is_err() {
                break;
            }
            self.doc.commit();
        }
        let page = self.ensure_page(page_id);
        self.write_conflicts(&page, &conflicts);
        self.doc.commit();
        conflicts
            .into_iter()
            .map(|(line_id, ours, theirs)| LineConflict {
                page_id: page_id.to_string(),
                line_id,
                ours,
                theirs,
            })
            .collect()
    }

    fn line_refs(&self, list: &automerge::ObjId) -> Vec<LineRef> {
        let n = self.doc.length(list);
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let Some((_, exid)) = self.doc.get(list, i).ok().flatten() else {
                continue;
            };
            let Some(id) = self.str_at(&exid, "id") else {
                continue;
            };
            out.push(LineRef {
                id,
                text: self.str_at(&exid, "text").unwrap_or_default(),
                exid,
            });
        }
        out
    }

    fn harvest_conflicts(&mut self) {
        let ids = self.page_ids();
        for id in ids {
            let Some((_, page)) = self.find_page(&id) else {
                continue;
            };
            if self.bool_at(&page, "sealed") {
                let all = self.all_str(&page, "body");
                if all.len() > 1 {
                    self.write_conflicts(&page, &[("body".into(), all[0].clone(), all[1].clone())]);
                }
                continue;
            }
            let Some(lines) = self.child(&page, "lines") else {
                continue;
            };
            let n = self.doc.length(&lines);
            let mut found = Vec::new();
            for i in 0..n {
                let Some((_, line)) = self.doc.get(&lines, i).ok().flatten() else {
                    continue;
                };
                let all = self.all_str(&line, "text");
                if all.len() > 1 {
                    let lid = self.str_at(&line, "id").unwrap_or_else(|| format!("c{i}"));
                    found.push((lid, all[0].clone(), all[1].clone()));
                }
            }
            if !found.is_empty() {
                let mut pending = self.read_conflicts(&page);
                for item in found {
                    if !pending.iter().any(|(id, _, _)| id == &item.0) {
                        pending.push(item);
                    }
                }
                self.write_conflicts(&page, &pending);
            }
        }
        self.doc.commit();
    }

    fn absorb(&mut self, other: &Library) {
        for id in other.page_ids() {
            let Some(text) = other.snapshot(&id) else {
                continue;
            };
            if let Some(mine) = self.snapshot(&id) {
                if mine == text {
                    continue;
                }
                let fresh = self.fresh_page_id(&id);
                self.reconcile(&fresh, &text, None);
            } else {
                self.reconcile(&id, &text, None);
            }
        }
        for folder in other.folders() {
            if self.find_folder(&folder.id).is_none() {
                self.insert_folder(&folder.id, &folder.name);
                let _ = self.set_parent(&folder.id, folder.parent.as_deref());
            }
        }
    }

    fn fresh_page_id(&self, id: &str) -> String {
        let mut n = 2u32;
        loop {
            let candidate = format!("{id}-{n}");
            if self.find_page(&candidate).is_none() {
                return candidate;
            }
            n += 1;
        }
    }

    fn ensure_page(&mut self, page_id: &str) -> automerge::ObjId {
        self.doc.commit();
        if let Some((_, page)) = self.find_page(page_id) {
            return page;
        }
        let pages = if let Some(pages) = self.child(&ROOT, "pages") {
            pages
        } else {
            self.doc
                .put_object(ROOT, "pages", ObjType::List)
                .unwrap_or(ROOT)
        };
        let at = self.doc.length(&pages);
        let page = self
            .doc
            .insert_object(&pages, at, ObjType::Map)
            .unwrap_or(ROOT);
        let _ = self.doc.put(&page, "id", page_id);
        let _ = self.doc.put(&page, "sealed", false);
        let _ = self.doc.put_object(&page, "lines", ObjType::List);
        let _ = self.doc.put_object(&page, "conflicts", ObjType::List);
        self.doc.commit();
        page
    }

    fn find_page(&self, page_id: &str) -> Option<(usize, automerge::ObjId)> {
        let pages = self.child(&ROOT, "pages")?;
        let n = self.doc.length(&pages);
        for i in 0..n {
            let (_, page) = self.doc.get(&pages, i).ok().flatten()?;
            if self.str_at(&page, "id").as_deref() == Some(page_id) {
                return Some((i, page));
            }
        }
        None
    }

    fn find_folder(&self, id: &str) -> Option<automerge::ObjId> {
        let list = self.child(&ROOT, "folders")?;
        let n = self.doc.length(&list);
        for i in 0..n {
            let (_, folder) = self.doc.get(&list, i).ok().flatten()?;
            if self.str_at(&folder, "id").as_deref() == Some(id) {
                return Some(folder);
            }
        }
        None
    }

    fn insert_folder(&mut self, id: &str, name: &str) {
        if self.find_folder(id).is_some() {
            if let Some(folder) = self.find_folder(id) {
                let _ = self.doc.put(&folder, "name", name);
                self.doc.commit();
            }
            return;
        }
        let list = if let Some(list) = self.child(&ROOT, "folders") {
            list
        } else {
            self.doc
                .put_object(ROOT, "folders", ObjType::List)
                .unwrap_or(ROOT)
        };
        let at = self.doc.length(&list);
        let folder = self
            .doc
            .insert_object(&list, at, ObjType::Map)
            .unwrap_or(ROOT);
        let _ = self.doc.put(&folder, "id", id);
        let _ = self.doc.put(&folder, "name", name);
        self.doc.commit();
    }

    fn write_parent(&mut self, id: &str, parent: Option<&str>) {
        let Some(folder) = self.find_folder(id) else {
            return;
        };
        if let Some(parent) = parent {
            let _ = self.doc.put(&folder, "parent", parent);
        } else {
            let _ = self.doc.delete(&folder, "parent");
        }
        self.doc.commit();
    }

    fn chain_contains(&self, start: &str, target: &str) -> bool {
        let mut cur = Some(start.to_string());
        let mut guard = 0;
        while let Some(id) = cur {
            if id == target {
                return true;
            }
            cur = self.parent_of(&id);
            guard += 1;
            if guard > 16 {
                return true;
            }
        }
        false
    }

    fn depth_of(&self, id: &str) -> usize {
        let mut depth = 1usize;
        let mut cur = self.parent_of(id);
        let mut guard = 0;
        while let Some(parent) = cur {
            depth += 1;
            cur = self.parent_of(&parent);
            guard += 1;
            if guard > 16 {
                break;
            }
        }
        depth
    }

    fn break_folder_problems(&mut self) {
        let folders = self.folders();
        for folder in &folders {
            if let Some(parent) = &folder.parent
                && (parent == &folder.id || self.chain_contains(parent, &folder.id))
            {
                self.write_parent(&folder.id, None);
            }
        }
        let folders = self.folders();
        for folder in folders {
            if let Some(parent) = &folder.parent
                && self.depth_of(parent) + 1 > MAX_FOLDER_DEPTH
            {
                self.write_parent(&folder.id, None);
            }
        }
    }

    fn clear_list(&mut self, parent: &automerge::ObjId, key: &str) {
        let Some(list) = self.child(parent, key) else {
            return;
        };
        self.clear_obj_list(&list);
    }

    fn clear_obj_list(&mut self, list: &automerge::ObjId) {
        let mut guard = self.doc.length(list);
        while self.doc.length(list) > 0 && guard > 0 {
            guard -= 1;
            let last = self.doc.length(list) - 1;
            if self.doc.delete(list, last).is_err() {
                break;
            }
        }
        self.doc.commit();
    }

    fn read_conflicts(&self, page: &automerge::ObjId) -> Vec<(String, String, String)> {
        let Some(list) = self.child(page, "conflicts") else {
            return Vec::new();
        };
        let n = self.doc.length(&list);
        let mut out = Vec::new();
        for i in 0..n {
            let Some((_, item)) = self.doc.get(&list, i).ok().flatten() else {
                continue;
            };
            let Some(line_id) = self.str_at(&item, "line") else {
                continue;
            };
            out.push((
                line_id,
                self.str_at(&item, "ours").unwrap_or_default(),
                self.str_at(&item, "theirs").unwrap_or_default(),
            ));
        }
        out
    }

    fn write_conflicts(&mut self, page: &automerge::ObjId, conflicts: &[(String, String, String)]) {
        let list = if let Some(list) = self.child(page, "conflicts") {
            list
        } else {
            match self.doc.put_object(page, "conflicts", ObjType::List) {
                Ok(list) => list,
                Err(_) => return,
            }
        };
        self.clear_obj_list(&list);
        for (line, ours, theirs) in conflicts {
            let at = self.doc.length(&list);
            let Ok(item) = self.doc.insert_object(&list, at, ObjType::Map) else {
                continue;
            };
            let _ = self.doc.put(&item, "line", line.as_str());
            let _ = self.doc.put(&item, "ours", ours.as_str());
            let _ = self.doc.put(&item, "theirs", theirs.as_str());
        }
        self.doc.commit();
    }

    fn mint(&mut self) -> String {
        let n = match self.doc.get(ROOT, "next").ok().flatten().map(|(v, _)| v) {
            Some(Value::Scalar(v)) => match v.as_ref() {
                ScalarValue::Int(i) => *i,
                ScalarValue::Uint(u) => *u as i64,
                _ => 1,
            },
            _ => 1,
        };
        let _ = self.doc.put(ROOT, "next", n.saturating_add(1));
        format!("l{n:x}")
    }

    fn child(&self, parent: &automerge::ObjId, key: &str) -> Option<automerge::ObjId> {
        let (value, id) = self.doc.get(parent, key).ok().flatten()?;
        match value {
            Value::Object(_) => Some(id),
            _ => None,
        }
    }

    fn root_str(&self, key: &str) -> Option<String> {
        self.str_at(&ROOT, key)
    }

    fn str_at(&self, obj: &automerge::ObjId, key: &str) -> Option<String> {
        let (value, _) = self.doc.get(obj, key).ok().flatten()?;
        match value {
            Value::Scalar(v) => match v.as_ref() {
                ScalarValue::Str(s) => Some(s.to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    fn str_at_heads(
        &self,
        obj: &automerge::ObjId,
        key: &str,
        heads: &[ChangeHash],
    ) -> Option<String> {
        let (value, _) = self.doc.get_at(obj, key, heads).ok().flatten()?;
        match value {
            Value::Scalar(v) => match v.as_ref() {
                ScalarValue::Str(s) => Some(s.to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    fn bool_at(&self, obj: &automerge::ObjId, key: &str) -> bool {
        match self.doc.get(obj, key).ok().flatten().map(|(v, _)| v) {
            Some(Value::Scalar(v)) => matches!(v.as_ref(), ScalarValue::Boolean(true)),
            _ => false,
        }
    }

    fn all_str(&self, obj: &automerge::ObjId, key: &str) -> Vec<String> {
        let Ok(all) = self.doc.get_all(obj, key) else {
            return Vec::new();
        };
        all.into_iter()
            .filter_map(|(value, _)| match value {
                Value::Scalar(v) => match v.as_ref() {
                    ScalarValue::Str(s) => Some(s.to_string()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

struct LineRef {
    id: String,
    text: String,
    exid: automerge::ObjId,
}

fn piece_id(piece: &Piece) -> Option<String> {
    match piece {
        Piece::Keep { id, .. } | Piece::Conflict { id, .. } => id.clone(),
    }
}

fn is_sealed(text: &str) -> bool {
    text.starts_with(LOCK_PREFIX)
}

pub fn content_hash(text: &str) -> String {
    let dig = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(dig.len() * 2);
    for byte in dig {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(str::to_string).collect()
    }
}

fn merge_pieces(base: &[SourceLine], ours: &[String], theirs: &[String]) -> Vec<Piece> {
    let base_text: Vec<String> = base.iter().map(|l| l.text.clone()).collect();
    let ma = lcs(&base_text, ours);
    let mb = lcs(&base_text, theirs);
    let mut ia = 0;
    let mut ib = 0;
    let mut bp = 0;
    let mut op = 0;
    let mut tp = 0;
    let mut out = Vec::new();
    let mut guard = base.len() + ours.len() + theirs.len() + 2;
    while bp < base.len() || op < ours.len() || tp < theirs.len() {
        if guard == 0 {
            break;
        }
        guard -= 1;
        let a_here = ma.get(ia).copied().filter(|&(b, o)| b == bp && o == op);
        let t_here = mb.get(ib).copied().filter(|&(b, t)| b == bp && t == tp);
        if a_here.is_some() && t_here.is_some() && bp < base.len() {
            out.push(Piece::Keep {
                id: Some(base[bp].id.clone()),
                text: base[bp].text.clone(),
            });
            bp += 1;
            op += 1;
            tp += 1;
            ia += 1;
            ib += 1;
            continue;
        }
        let mut look_a = ia;
        let mut look_b = ib;
        let mut anchor = None;
        while look_a < ma.len() && look_b < mb.len() {
            let (ba, oa) = ma[look_a];
            let (bb, tb) = mb[look_b];
            if ba < bp || oa < op {
                look_a += 1;
                continue;
            }
            if bb < bp || tb < tp {
                look_b += 1;
                continue;
            }
            if ba == bb {
                anchor = Some((ba, oa, tb, look_a, look_b));
                break;
            } else if ba < bb {
                look_a += 1;
            } else {
                look_b += 1;
            }
        }
        let (b_end, o_end, t_end, next_ia, next_ib) = match anchor {
            Some(found) => found,
            None => (base.len(), ours.len(), theirs.len(), ma.len(), mb.len()),
        };
        if b_end == bp && o_end == op && t_end == tp {
            let o = ours.get(op).cloned().unwrap_or_default();
            let t = theirs.get(tp).cloned().unwrap_or_default();
            let id = base.get(bp).map(|l| l.id.clone());
            if bp < base.len() && o == base[bp].text {
                if tp < theirs.len() {
                    out.push(Piece::Keep { id, text: t });
                }
            } else if bp < base.len() && t == base[bp].text {
                if op < ours.len() {
                    out.push(Piece::Keep { id, text: o });
                }
            } else if bp >= base.len() {
                if op < ours.len() {
                    out.push(Piece::Keep {
                        id: None,
                        text: o.clone(),
                    });
                }
                if tp < theirs.len() && t != o {
                    out.push(Piece::Keep {
                        id: None,
                        text: theirs[tp].clone(),
                    });
                }
            } else if op < ours.len() || tp < theirs.len() {
                out.push(Piece::Conflict {
                    id,
                    ours: o,
                    theirs: t,
                });
            }
            if bp < base.len() {
                bp += 1;
            }
            if op < ours.len() {
                op += 1;
            }
            if tp < theirs.len() {
                tp += 1;
            }
            continue;
        }
        out.extend(merge_hunk(
            &base[bp..b_end],
            &ours[op..o_end],
            &theirs[tp..t_end],
        ));
        bp = b_end;
        op = o_end;
        tp = t_end;
        ia = next_ia;
        ib = next_ib;
    }
    out
}

fn merge_hunk(base: &[SourceLine], ours: &[String], theirs: &[String]) -> Vec<Piece> {
    if ours == theirs {
        return adopt(base, ours);
    }
    if chunk_eq(base, ours) {
        return adopt(base, theirs);
    }
    if chunk_eq(base, theirs) {
        return adopt(base, ours);
    }
    if base.is_empty() {
        let mut out = adopt(base, ours);
        out.extend(adopt(base, theirs));
        return out;
    }
    if base.len() == 1 && ours.len() == 1 && theirs.len() == 1 {
        return vec![Piece::Conflict {
            id: Some(base[0].id.clone()),
            ours: ours[0].clone(),
            theirs: theirs[0].clone(),
        }];
    }
    let n = ours.len().max(theirs.len()).max(base.len());
    let mut out = Vec::new();
    for i in 0..n {
        let o = ours.get(i).cloned();
        let t = theirs.get(i).cloned();
        let id = base.get(i).map(|l| l.id.clone());
        match (o, t) {
            (Some(o), Some(t)) if o == t => out.push(Piece::Keep { id, text: o }),
            (Some(o), Some(t)) => {
                let base_text = base.get(i).map(|l| l.text.as_str());
                if base_text == Some(o.as_str()) {
                    out.push(Piece::Keep { id, text: t });
                } else if base_text == Some(t.as_str()) {
                    out.push(Piece::Keep { id, text: o });
                } else {
                    out.push(Piece::Conflict {
                        id,
                        ours: o,
                        theirs: t,
                    });
                }
            }
            (Some(o), None) => out.push(Piece::Keep { id: None, text: o }),
            (None, Some(t)) => out.push(Piece::Keep { id: None, text: t }),
            (None, None) => {}
        }
    }
    out
}

fn chunk_eq(base: &[SourceLine], side: &[String]) -> bool {
    base.len() == side.len() && base.iter().zip(side).all(|(line, text)| line.text == *text)
}

fn adopt(base: &[SourceLine], side: &[String]) -> Vec<Piece> {
    if side.is_empty() {
        return Vec::new();
    }
    let base_text: Vec<String> = base.iter().map(|l| l.text.clone()).collect();
    let matches = lcs(&base_text, side);
    let mut out = Vec::new();
    let mut bi = 0;
    let mut si = 0;
    let mut anchors = matches;
    anchors.push((base.len(), side.len()));
    for (bm, sm) in anchors {
        let base_gap = bm.saturating_sub(bi);
        let side_gap = sm.saturating_sub(si);
        if base_gap == side_gap {
            for k in 0..side_gap {
                let id = base.get(bi + k).map(|l| l.id.clone());
                out.push(Piece::Keep {
                    id,
                    text: side[si + k].clone(),
                });
            }
        } else if base_gap == 0 {
            for k in 0..side_gap {
                out.push(Piece::Keep {
                    id: None,
                    text: side[si + k].clone(),
                });
            }
        } else if side_gap != 0 {
            let n = base_gap.min(side_gap);
            for k in 0..n {
                out.push(Piece::Keep {
                    id: base.get(bi + k).map(|l| l.id.clone()),
                    text: side[si + k].clone(),
                });
            }
            for k in n..side_gap {
                out.push(Piece::Keep {
                    id: None,
                    text: side[si + k].clone(),
                });
            }
        }
        if bm < base.len() && sm < side.len() {
            out.push(Piece::Keep {
                id: Some(base[bm].id.clone()),
                text: side[sm].clone(),
            });
            bi = bm + 1;
            si = sm + 1;
        } else {
            bi = bm;
            si = sm;
        }
    }
    out
}

fn lcs(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let n = a.len();
    let m = b.len();
    if n == 0 || m == 0 {
        return Vec::new();
    }
    if n.saturating_mul(m) > 250_000 {
        return greedy_match(a, b);
    }
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn greedy_match(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut j = 0;
    for (i, line) in a.iter().enumerate() {
        while j < b.len() && &b[j] != line {
            j += 1;
        }
        if j < b.len() {
            out.push((i, j));
            j += 1;
        }
    }
    out
}

fn bool_at_heads(
    doc: &AutoCommit,
    obj: &automerge::ObjId,
    key: &str,
    heads: &[ChangeHash],
) -> bool {
    match doc.get_at(obj, key, heads).ok().flatten().map(|(v, _)| v) {
        Some(Value::Scalar(v)) => matches!(v.as_ref(), ScalarValue::Boolean(true)),
        _ => false,
    }
}

fn object_at(
    doc: &AutoCommit,
    obj: &automerge::ObjId,
    key: &str,
    heads: &[ChangeHash],
) -> Option<automerge::ObjId> {
    let (value, id) = doc.get_at(obj, key, heads).ok().flatten()?;
    match value {
        Value::Object(_) => Some(id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrating_pages_round_trips_and_names_the_vault() {
        let mut lib = Library::new();
        lib.reconcile("trip", "# Trip Budget\nferry = 40\n", None);
        lib.note_written("trip", "# Trip Budget\nferry = 40\n");
        lib.reconcile("groceries", "milk $3\n", None);
        lib.set_folders(&[FolderSeed {
            id: "trips".into(),
            name: "Trips".into(),
            parent: None,
        }]);
        assert_eq!(lib.vault(), VAULT_ID);
        assert_eq!(
            lib.snapshot("trip").as_deref(),
            Some("# Trip Budget\nferry = 40\n")
        );
        assert_eq!(lib.snapshot("groceries").as_deref(), Some("milk $3\n"));
        assert_eq!(
            lib.written_hash("trip").as_deref(),
            Some(content_hash("# Trip Budget\nferry = 40\n").as_str())
        );
        let bytes = lib.save();
        let loaded = Library::load(&bytes).unwrap();
        assert_eq!(loaded.doc_id(), lib.doc_id());
        assert_eq!(loaded.vault(), "personal");
        assert_eq!(
            loaded.snapshot("trip").as_deref(),
            Some("# Trip Budget\nferry = 40\n")
        );
        assert_eq!(loaded.folders()[0].name, "Trips");
    }

    #[test]
    fn an_outside_edit_on_another_line_merges() {
        let mut lib = Library::new();
        lib.reconcile("trip", "# Trip Budget\nferry = 40", None);
        let conflicts = lib.reconcile(
            "trip",
            "# Trip Budget\nferry = 50",
            Some("# Trip Budget\nferry = 40\nmilk $3"),
        );
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert_eq!(
            lib.snapshot("trip").as_deref(),
            Some("# Trip Budget\nferry = 50\nmilk $3")
        );
    }

    #[test]
    fn the_same_line_keeps_both_until_someone_picks() {
        let mut lib = Library::new();
        lib.reconcile("trip", "ferry = 40", None);
        let conflicts = lib.reconcile("trip", "ferry = 50", Some("ferry = 60"));
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].ours, "ferry = 50");
        assert_eq!(conflicts[0].theirs, "ferry = 60");
        assert_eq!(lib.snapshot("trip").as_deref(), Some("ferry = 50"));
        let again = lib.conflicts();
        assert_eq!(again.len(), 1);
        assert!(lib.pick("trip", &again[0].line_id, true));
        assert_eq!(lib.snapshot("trip").as_deref(), Some("ferry = 60"));
        assert!(lib.conflicts().is_empty());
    }

    #[test]
    fn one_side_matching_the_last_write_takes_the_other() {
        let mut lib = Library::new();
        lib.reconcile("trip", "ferry = 40", None);
        let conflicts = lib.reconcile("trip", "ferry = 40", Some("ferry = 60"));
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert_eq!(lib.snapshot("trip").as_deref(), Some("ferry = 60"));
        let conflicts = lib.reconcile("trip", "milk $3", Some("ferry = 60"));
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert_eq!(lib.snapshot("trip").as_deref(), Some("milk $3"));
    }

    #[test]
    fn inserts_from_both_sides_stay_on_the_page() {
        let mut lib = Library::new();
        lib.reconcile("trip", "# Shared", None);
        let conflicts = lib.reconcile("trip", "# Shared\nfrom A", Some("# Shared\nfrom B"));
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert_eq!(
            lib.snapshot("trip").as_deref(),
            Some("# Shared\nfrom A\nfrom B")
        );
    }

    #[test]
    fn a_locked_page_is_not_split_into_lines() {
        let mut lib = Library::new();
        let base = "OMACUS-LOCK/1\na=1\n";
        lib.reconcile("secret", base, None);
        assert_eq!(lib.snapshot("secret").as_deref(), Some(base));
        let ours = "OMACUS-LOCK/1\na=2\n";
        let theirs = "OMACUS-LOCK/1\na=3\n";
        let conflicts = lib.reconcile("secret", ours, Some(theirs));
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].line_id, "body");
        assert_eq!(lib.snapshot("secret").as_deref(), Some(ours));
        assert!(!lib.snapshot("secret").unwrap().contains("a=3"));
        assert!(lib.pick("secret", "body", true));
        assert_eq!(lib.snapshot("secret").as_deref(), Some(theirs));
    }

    #[test]
    fn a_deleted_line_leaves_the_snapshot_and_stays_in_history() {
        let mut lib = Library::new();
        lib.reconcile("trip", "ferry = 40\nmilk $3", None);
        let heads = lib.heads();
        let conflicts = lib.reconcile("trip", "milk $3", Some("ferry = 40\nmilk $3"));
        assert!(conflicts.is_empty(), "{conflicts:?}");
        let snap = lib.snapshot("trip").unwrap();
        assert_eq!(snap, "milk $3");
        assert!(!snap.contains("ferry"));
        let old = lib.page_text_at("trip", &heads).unwrap();
        assert!(old.contains("ferry = 40"), "{old}");
        let bytes = lib.save();
        let mut loaded = Library::load(&bytes).unwrap();
        assert_eq!(loaded.snapshot("trip").as_deref(), Some("milk $3"));
        let old = loaded.page_text_at("trip", &heads).unwrap();
        assert!(old.contains("ferry = 40"), "{old}");
    }

    #[test]
    fn unrelated_documents_keep_both_pages() {
        let a = Library::new();
        let mut a = a;
        a.reconcile("trip", "ferry = 40", None);
        let mut b = Library::new();
        b.reconcile("trip", "ferry = 60", None);
        assert_ne!(a.doc_id(), b.doc_id());
        let union = a.union(b);
        let texts: Vec<_> = union
            .page_ids()
            .iter()
            .map(|id| union.snapshot(id).unwrap())
            .collect();
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert!(texts.iter().any(|t| t == "ferry = 40"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "ferry = 60"), "{texts:?}");
        assert!(
            texts.iter().all(|t| !(t.contains('4') && t.contains('6'))),
            "{texts:?}"
        );
        assert_eq!(union.vault(), "personal");
    }

    #[test]
    fn the_same_document_merges_different_lines_and_conflicts_one_line() {
        let mut origin = Library::new();
        origin.reconcile("trip", "ferry = 40", None);
        let bytes = origin.save();
        let mut a = Library::load(&bytes).unwrap();
        let mut b = Library::load(&bytes).unwrap();
        a.reconcile("trip", "ferry = 40\nmilk $3", None);
        b.reconcile("trip", "ferry = 50", None);
        let union = a.union(b);
        let text = union.snapshot("trip").unwrap();
        assert!(text.contains("milk $3"), "{text}");
        assert!(text.contains("ferry = 50"), "{text}");
        assert!(!text.contains("ferry = 40"), "{text}");

        let mut a = Library::load(&bytes).unwrap();
        let mut b = Library::load(&bytes).unwrap();
        a.reconcile("trip", "ferry = 50", None);
        b.reconcile("trip", "ferry = 60", None);
        let union = a.union(b);
        let conflicts = union.conflicts();
        assert_eq!(conflicts.len(), 1, "{conflicts:?}");
        let mut sides = [conflicts[0].ours.clone(), conflicts[0].theirs.clone()];
        sides.sort();
        assert_eq!(sides, ["ferry = 50", "ferry = 60"]);
        let snap = union.snapshot("trip").unwrap();
        assert!(snap == "ferry = 50" || snap == "ferry = 60", "{snap}");
        assert!(!snap.contains("460"), "{snap}");
    }

    #[test]
    fn a_folder_loop_is_broken_and_a_third_level_is_refused() {
        let mut lib = Library::new();
        lib.set_folders(&[
            FolderSeed {
                id: "a".into(),
                name: "A".into(),
                parent: None,
            },
            FolderSeed {
                id: "b".into(),
                name: "B".into(),
                parent: None,
            },
            FolderSeed {
                id: "c".into(),
                name: "C".into(),
                parent: None,
            },
        ]);
        assert!(lib.set_parent("a", Some("b")));
        assert!(!lib.set_parent("b", Some("a")));
        assert_eq!(lib.parent_of("b"), None);
        assert_eq!(lib.parent_of("a").as_deref(), Some("b"));
        // `a` is already nested, so `c` inside `a` would be a third level.
        assert!(!lib.set_parent("c", Some("a")));
        assert_eq!(lib.parent_of("c"), None);
        assert!(lib.set_parent("c", Some("b")));
        assert_eq!(lib.parent_of("c").as_deref(), Some("b"));
        assert!(lib.depth_of("c") <= MAX_FOLDER_DEPTH);
    }

    #[test]
    fn loading_a_loop_clears_it() {
        let mut lib = Library::new();
        lib.insert_folder("a", "A");
        lib.insert_folder("b", "B");
        lib.write_parent("a", Some("b"));
        lib.write_parent("b", Some("a"));
        let bytes = lib.save();
        let loaded = Library::load(&bytes).unwrap();
        let a = loaded.parent_of("a");
        let b = loaded.parent_of("b");
        assert!(a.is_none() || b.is_none(), "a={a:?} b={b:?}");
    }
}
