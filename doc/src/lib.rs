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

    /// The page the first time it was saved. Later edits do not move this.
    pub fn page_origin(&mut self, page_id: &str) -> Option<String> {
        self.doc.commit();
        let hashes: Vec<ChangeHash> = self
            .doc
            .get_changes(&[])
            .into_iter()
            .map(|change| change.hash())
            .collect();
        for hash in hashes {
            let head = [hash];
            if self.written_at(page_id, &head).is_some() {
                return self.page_text_at(page_id, &head);
            }
        }
        self.snapshot(page_id)
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
    /// A different document: the same page id stays one page.
    /// Two pages that only share a title stay two pages, because their ids differ.
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
                let combined = union_lines(&mine, &text);
                self.reconcile(&id, &combined, None);
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

    fn written_at(&self, page_id: &str, heads: &[ChangeHash]) -> Option<String> {
        let pages = self.child(&ROOT, "pages")?;
        let n = self.doc.length_at(&pages, heads);
        for i in 0..n {
            let (_, page) = self.doc.get_at(&pages, i, heads).ok().flatten()?;
            if self.str_at_heads(&page, "id", heads).as_deref() != Some(page_id) {
                continue;
            }
            return self.str_at_heads(&page, "written", heads);
        }
        None
    }

    /// Bring another device's pages into this one.
    ///
    /// The same page id stays one page. A deletion on one side and a new line
    /// on the other both remain when the two copies share a start. A locked
    /// page stays on that same id. Two pages that only share a title stay two
    /// pages, because their ids differ.
    fn fold_other(&mut self, mut other: Library) {
        let mut ids = self.page_ids();
        for id in other.page_ids() {
            if !ids.iter().any(|have| have == &id) {
                ids.push(id);
            }
        }
        for id in ids {
            let ours = self.snapshot(&id);
            let theirs = other.snapshot(&id);
            match (ours, theirs) {
                (None, Some(text)) => {
                    self.reconcile(&id, &text, None);
                    self.note_written(&id, &text);
                }
                (Some(_), None) | (None, None) => {}
                (Some(ours), Some(theirs)) if ours == theirs => {}
                (Some(ours), Some(theirs)) if is_sealed(&ours) || is_sealed(&theirs) => {
                    self.reconcile(&id, &ours, Some(&theirs));
                }
                (Some(ours), Some(theirs)) => {
                    let origin_ours = self.page_origin(&id);
                    let origin_theirs = other.page_origin(&id);
                    if let (Some(start_ours), Some(start_theirs)) = (origin_ours, origin_theirs)
                        && start_ours == start_theirs
                    {
                        if self.snapshot(&id).as_deref() != Some(start_ours.as_str()) {
                            self.reconcile(&id, &start_ours, None);
                        }
                        self.reconcile(&id, &ours, Some(&theirs));
                    } else {
                        let combined = union_lines(&ours, &theirs);
                        self.reconcile(&id, &combined, None);
                    }
                }
            }
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

/// Merge two devices' page sets.
///
/// `pages` are the copies each device is showing. They are folded into that
/// device's document first. The result is one document and the text it shows.
/// A page that only one device has is kept. A deleted line is not put back.
pub fn merge_libraries(
    local_doc: &[u8],
    local_pages: &[(String, String)],
    remote_doc: &[u8],
    remote_pages: &[(String, String)],
) -> (Vec<u8>, Vec<(String, String)>) {
    let mut local = prepare_library(local_doc, local_pages);
    let mut remote = prepare_library(remote_doc, remote_pages);
    if remote.doc_id() < local.doc_id() {
        std::mem::swap(&mut local, &mut remote);
    }
    let merged = if local.doc_id() == remote.doc_id() {
        local.union(remote)
    } else {
        local.fold_other(remote);
        local
    };
    finish_library(merged, local_pages, remote_pages)
}

fn prepare_library(doc: &[u8], pages: &[(String, String)]) -> Library {
    let mut lib = Library::load(doc).unwrap_or_else(|_| Library::new());
    for (id, text) in pages {
        if id.is_empty() {
            continue;
        }
        if lib.snapshot(id).as_deref() != Some(text.as_str()) {
            lib.reconcile(id, text, None);
            lib.note_written(id, text);
        }
    }
    lib
}

fn finish_library(
    mut lib: Library,
    local_pages: &[(String, String)],
    remote_pages: &[(String, String)],
) -> (Vec<u8>, Vec<(String, String)>) {
    for (id, text) in local_pages.iter().chain(remote_pages.iter()) {
        if id.is_empty() {
            continue;
        }
        if lib.snapshot(id).is_none() {
            lib.reconcile(id, text, None);
            lib.note_written(id, text);
        }
    }
    let pages = lib
        .page_ids()
        .into_iter()
        .filter_map(|id| lib.snapshot(&id).map(|text| (id, text)))
        .collect();
    (lib.save(), pages)
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

/// Lines from either copy, in order, without a second page.
fn union_lines(ours: &str, theirs: &str) -> String {
    let mut lines = Vec::new();
    for line in ours.lines().chain(theirs.lines()) {
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    let mut text = lines.join("\n");
    if ours.ends_with('\n') || theirs.ends_with('\n') {
        text.push('\n');
    }
    text
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
        // Both sides added these lines. The same line on both sides is one
        // line: a copy that already arrived must not be inserted again.
        return merge_inserts(ours, theirs);
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

/// Lines both sides added, in order. A line that shows up on each side once
/// stays once. A line only one side added stays too.
fn merge_inserts(ours: &[String], theirs: &[String]) -> Vec<Piece> {
    let keep = |text: &String| Piece::Keep {
        id: None,
        text: text.clone(),
    };
    if ours.is_empty() {
        return theirs.iter().map(keep).collect();
    }
    if theirs.is_empty() {
        return ours.iter().map(keep).collect();
    }
    let matches = lcs(ours, theirs);
    let mut out = Vec::new();
    let mut oi = 0;
    let mut ti = 0;
    for (om, tm) in matches {
        while oi < om {
            out.push(keep(&ours[oi]));
            oi += 1;
        }
        while ti < tm {
            out.push(keep(&theirs[ti]));
            ti += 1;
        }
        out.push(keep(&ours[om]));
        oi = om + 1;
        ti = tm + 1;
    }
    while oi < ours.len() {
        out.push(keep(&ours[oi]));
        oi += 1;
    }
    while ti < theirs.len() {
        out.push(keep(&theirs[ti]));
        ti += 1;
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
        assert_eq!(union.page_ids(), vec!["trip".to_string()], "{texts:?}");
        let text = texts.into_iter().next().unwrap();
        assert!(text.contains("ferry = 40"), "{text}");
        assert!(text.contains("ferry = 60"), "{text}");
        assert!(!text.contains("460"), "{text}");
        assert_eq!(union.vault(), "personal");
    }

    #[test]
    fn the_same_page_is_not_copied_when_a_device_rejoins() {
        let start = "Trip Budget\nferry = 40\n";
        let mut a = Library::new();
        a.reconcile("trip", start, None);
        a.note_written("trip", start);
        a.reconcile("trip", "Trip Budget\nferry = 40\nmilk $3\n", None);
        let mut b = Library::new();
        b.reconcile("trip", start, None);
        b.note_written("trip", start);
        b.reconcile("trip", "Trip Budget\nferry = 40\nbread $2\n", None);
        let (_doc, pages) = merge_libraries(
            &a.save(),
            &[("trip".into(), "Trip Budget\nferry = 40\nmilk $3\n".into())],
            &b.save(),
            &[("trip".into(), "Trip Budget\nferry = 40\nbread $2\n".into())],
        );
        let trips: Vec<_> = pages
            .iter()
            .filter(|(id, _)| id == "trip" || id.starts_with("trip-"))
            .collect();
        assert_eq!(trips.len(), 1, "{pages:?}");
        let text = &trips[0].1;
        assert!(text.contains("milk $3"), "{text}");
        assert!(text.contains("bread $2"), "{text}");
        assert_eq!(text.matches("ferry = 40").count(), 1, "{text}");
    }

    #[test]
    fn a_page_is_not_copied_when_the_first_saved_text_differs() {
        let mut a = Library::new();
        a.reconcile("trip", "ferry = 40\n", None);
        a.note_written("trip", "ferry = 40\n");
        let mut b = Library::new();
        b.reconcile("trip", "ferry = 40\nbread $2\n", None);
        b.note_written("trip", "ferry = 40\nbread $2\n");
        let (_doc, pages) = merge_libraries(
            &a.save(),
            &[("trip".into(), "ferry = 40\n".into())],
            &b.save(),
            &[("trip".into(), "ferry = 40\nbread $2\n".into())],
        );
        assert_eq!(pages.len(), 1, "{pages:?}");
        assert_eq!(pages[0].0, "trip");
        assert!(pages[0].1.contains("ferry = 40"), "{pages:?}");
        assert!(pages[0].1.contains("bread $2"), "{pages:?}");
    }

    #[test]
    fn two_pages_with_the_same_title_both_stay() {
        let trip = "# Trip Budget\nferry = 40\n";
        let groceries = "# Trip Budget\nmilk $3\n";
        let mut a = Library::new();
        a.reconcile("ex1", trip, None);
        a.note_written("ex1", trip);
        let mut b = Library::new();
        b.reconcile("ex2", groceries, None);
        b.note_written("ex2", groceries);
        let (_doc, pages) = merge_libraries(
            &a.save(),
            &[("ex1".into(), trip.into())],
            &b.save(),
            &[("ex2".into(), groceries.into())],
        );
        assert_eq!(pages.len(), 2, "{pages:?}");
        assert!(pages.iter().any(|(id, text)| id == "ex1" && text == trip));
        assert!(
            pages
                .iter()
                .any(|(id, text)| id == "ex2" && text == groceries)
        );
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

    fn device(text: &str) -> (Vec<u8>, Vec<(String, String)>) {
        let mut lib = Library::new();
        lib.reconcile("trip", text, None);
        lib.note_written("trip", text);
        let pages = vec![("trip".to_string(), text.to_string())];
        (lib.save(), pages)
    }

    fn edit(
        doc: &[u8],
        pages: &[(String, String)],
        text: &str,
    ) -> (Vec<u8>, Vec<(String, String)>) {
        let mut lib = Library::load(doc).unwrap();
        lib.reconcile("trip", text, None);
        lib.note_written("trip", text);
        let mut pages = pages.to_vec();
        pages[0].1 = text.to_string();
        (lib.save(), pages)
    }

    fn merged_trip(
        left: &(Vec<u8>, Vec<(String, String)>),
        right: &(Vec<u8>, Vec<(String, String)>),
    ) -> String {
        let (_doc, pages) = merge_libraries(&left.0, &left.1, &right.0, &right.1);
        pages.into_iter().find(|(id, _)| id == "trip").unwrap().1
    }

    #[test]
    fn a_replaced_page_does_not_bring_the_old_lines_back() {
        let sample = "# Trip Budget\nflights = $640\nhotel = 4 nights\n";
        let replaced = "# Trip Budget\nferry = 40\n";
        let a = device(sample);
        let b = device(sample);
        let a = edit(&a.0, &a.1, replaced);
        let text = merged_trip(&a, &b);
        assert!(text.contains("ferry = 40"), "{text}");
        assert!(!text.contains("flights"), "{text}");
        assert!(!text.contains("hotel"), "{text}");
        let other_way = merged_trip(&b, &a);
        assert_eq!(other_way, text);
    }

    #[test]
    fn a_delete_and_a_new_line_both_stay() {
        let sample = "ferry = 40\nmilk $3\n";
        let a = device(sample);
        let b = device(sample);
        let a = edit(&a.0, &a.1, "milk $3\n");
        let b = edit(&b.0, &b.1, "ferry = 40\nmilk $3\nbread $2\n");
        let text = merged_trip(&a, &b);
        assert!(text.contains("milk $3"), "{text}");
        assert!(text.contains("bread $2"), "{text}");
        assert!(!text.contains("ferry"), "{text}");
        let other_way = merged_trip(&b, &a);
        assert!(other_way.contains("milk $3"), "{other_way}");
        assert!(other_way.contains("bread $2"), "{other_way}");
        assert!(!other_way.contains("ferry"), "{other_way}");
    }

    #[test]
    fn the_same_document_drops_a_deleted_line_and_keeps_a_new_one() {
        let mut origin = Library::new();
        let sample = "ferry = 40\nmilk $3\n";
        origin.reconcile("trip", sample, None);
        origin.note_written("trip", sample);
        let bytes = origin.save();
        let pages = vec![("trip".to_string(), sample.to_string())];
        let a = edit(&bytes, &pages, "milk $3\n");
        let b = edit(&bytes, &pages, "ferry = 40\nmilk $3\nbread $2\n");
        assert_eq!(
            Library::load(&a.0).unwrap().doc_id(),
            Library::load(&b.0).unwrap().doc_id()
        );
        let text = merged_trip(&a, &b);
        assert!(text.contains("milk $3"), "{text}");
        assert!(text.contains("bread $2"), "{text}");
        assert!(!text.contains("ferry"), "{text}");
    }

    #[test]
    fn concurrent_appends_on_one_document_each_appear_once() {
        let mut origin = Library::new();
        let sample = "# Trip Budget\nferry = 40\n";
        origin.reconcile("trip", sample, None);
        origin.note_written("trip", sample);
        let bytes = origin.save();
        let pages = vec![("trip".to_string(), sample.to_string())];
        let a = edit(&bytes, &pages, "# Trip Budget\nferry = 40\ncake $5\n");
        let b = edit(&bytes, &pages, "# Trip Budget\nferry = 40\nbread $2\n");
        let text = merged_trip(&a, &b);
        assert_eq!(text.matches("cake $5").count(), 1, "{text}");
        assert_eq!(text.matches("bread $2").count(), 1, "{text}");
        assert_eq!(text.matches("ferry = 40").count(), 1, "{text}");
        let other = merged_trip(&b, &a);
        assert_eq!(other.matches("cake $5").count(), 1, "{other}");
        assert_eq!(other.matches("bread $2").count(), 1, "{other}");
    }

    #[test]
    fn an_append_after_a_blank_line_is_not_duplicated() {
        let sample = "# Trip Budget\n// Four nights, three currencies.\nflights = $640\nhotel = 4 nights * €120 in USD\nfood = ¥8,000 in USD\ntotal = sum\n";
        let mut origin = Library::new();
        origin.reconcile("trip", sample, None);
        origin.note_written("trip", sample);
        origin.reconcile("trip", "# Trip Budget\nferry = 40\n", None);
        origin.note_written("trip", "# Trip Budget\nferry = 40\n");
        let bytes = origin.save();
        let base = "# Trip Budget\nferry = 40\n";
        let pages = vec![("trip".to_string(), base.to_string())];
        // Control+End on a page that already ends with a newline, then a new line.
        let a = edit(&bytes, &pages, "# Trip Budget\nferry = 40\n\ncake $5\n");
        let b = edit(&bytes, &pages, "# Trip Budget\nferry = 40\n\nbread $2\n");
        let text = merged_trip(&a, &b);
        assert_eq!(text.matches("cake $5").count(), 1, "{text:?}");
        assert_eq!(text.matches("bread $2").count(), 1, "{text:?}");
        assert!(!text.contains("flights"), "{text:?}");

        // The other device then merges that result with its own copy.
        let (doc, pages) = merge_libraries(&b.0, &b.1, &a.0, &a.1);
        let again = merge_libraries(&a.0, &a.1, &doc, &pages);
        let text = again.1.into_iter().find(|(id, _)| id == "trip").unwrap().1;
        assert_eq!(text.matches("cake $5").count(), 1, "{text:?}");
        assert_eq!(text.matches("bread $2").count(), 1, "{text:?}");
    }

    #[test]
    fn merging_again_does_not_duplicate_a_line_already_brought_across() {
        let sample = "# Trip Budget\n// Four nights, three currencies.\nflights = $640\nhotel = 4 nights * €120 in USD\nfood = ¥8,000 in USD\ntotal = sum\n";
        let ferry = "# Trip Budget\nferry = 40\n";
        let a = device(sample);
        let b = device(sample);
        let a = edit(&a.0, &a.1, ferry);
        let b = edit(&b.0, &b.1, ferry);
        let a = edit(&a.0, &a.1, "# Trip Budget\nferry = 40\n\ncake $5\n");
        let b = edit(&b.0, &b.1, "# Trip Budget\nferry = 40\n\nbread $2\n");
        let (doc, pages) = merge_libraries(&b.0, &b.1, &a.0, &a.1);
        let once = pages.iter().find(|(id, _)| id == "trip").unwrap().1.clone();
        assert_eq!(once.matches("cake $5").count(), 1, "{once:?}");
        assert_eq!(once.matches("bread $2").count(), 1, "{once:?}");
        assert!(!once.contains("flights"), "{once:?}");
        let (_doc, again_pages) = merge_libraries(&a.0, &a.1, &doc, &pages);
        let again = again_pages
            .into_iter()
            .find(|(id, _)| id == "trip")
            .unwrap()
            .1;
        assert_eq!(again.matches("cake $5").count(), 1, "{again:?}");
        assert_eq!(again.matches("bread $2").count(), 1, "{again:?}");
        assert!(again.contains("ferry = 40"), "{again:?}");
        assert!(!again.contains("flights"), "{again:?}");
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
