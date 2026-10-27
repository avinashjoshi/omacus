//! Sidebar, trash, handles, and preferences inside the library document.
//!
//! The bytes are sealed before they leave the device. A merge keeps the later
//! edit of each item. The same revision prefers a page that sits in a folder
//! over a copy of that page left at the root, then a stable order.

use std::collections::{HashMap, HashSet};

use automerge::transaction::Transactable;
use automerge::{ROOT, ReadDoc, ScalarValue, Value};
use serde_json::{Map, Value as Json, json};

use crate::Library;

/// Not `vault`: that key is the library id (`personal`). This one is the sidebar,
/// trash, handles, and preferences. It is sealed with the rest of the document.
const VAULT_KEY: &str = "shared";

#[derive(Clone, Debug)]
struct Place {
    id: String,
    kind: String,
    name: String,
    parent: String,
    index: u64,
    rev: u64,
    collapsed: bool,
    gone: bool,
    lock: Option<Json>,
}

#[derive(Clone, Debug)]
struct Handle {
    id: String,
    handle: String,
    rev: u64,
    gone: bool,
}

#[derive(Clone, Debug)]
struct Trashed {
    id: String,
    text: String,
    handle: String,
    rev: u64,
    gone: bool,
}

#[derive(Clone, Debug)]
struct Pref {
    value: Json,
    rev: u64,
}

#[derive(Clone, Debug, Default)]
struct Vault {
    clock: u64,
    sidebar: Vec<Place>,
    handles: Vec<Handle>,
    trash: Vec<Trashed>,
    prefs: HashMap<String, Pref>,
}

impl Library {
    /// The merged library state, if any device has written one.
    pub fn vault_json(&self) -> Option<String> {
        let blobs = self.vault_blobs();
        if blobs.is_empty() {
            None
        } else {
            Some(merge_vaults(&blobs))
        }
    }

    /// Fold `structural` into the saved state. Unchanged state is left alone.
    pub fn note_vault(&mut self, structural: &str) -> bool {
        let current = self.vault_json().unwrap_or_else(empty_vault);
        let Some(next) = note_structural(&current, structural) else {
            return false;
        };
        let _ = self.doc.put(ROOT, VAULT_KEY, next);
        self.doc.commit();
        true
    }

    /// What the sidebar, trash, handles, and preferences should show.
    pub fn vault_present(&self) -> Option<String> {
        self.vault_json().map(|json| present_vault(&json))
    }

    /// A second document still carries the sidebar, trash, handles, and preferences.
    /// Folding its pages in used to leave those behind.
    pub(crate) fn keep_shared_state(&mut self, other: &Library) {
        let mut blobs = self.vault_blobs();
        let before = blobs.len();
        for blob in other.vault_blobs() {
            if !blobs.iter().any(|have| have == &blob) {
                blobs.push(blob);
            }
        }
        if blobs.len() == before {
            return;
        }
        let merged = merge_vaults(&blobs);
        if self.vault_json().as_deref() == Some(merged.as_str()) {
            return;
        }
        let _ = self.doc.put(ROOT, VAULT_KEY, merged);
        self.doc.commit();
    }

    /// Fold a stored copy of the sidebar, trash, handles, and preferences
    /// into this document. A later revision of each item wins.
    pub fn adopt_vault(&mut self, stored: &str) {
        if stored.trim().is_empty() {
            return;
        }
        let mut blobs = self.vault_blobs();
        if blobs.iter().any(|have| have == stored) {
            return;
        }
        blobs.push(stored.to_string());
        let merged = merge_vaults(&blobs);
        if self.vault_json().as_deref() == Some(merged.as_str()) {
            return;
        }
        let _ = self.doc.put(ROOT, VAULT_KEY, merged);
        self.doc.commit();
    }

    fn vault_blobs(&self) -> Vec<String> {
        let Ok(all) = self.doc.get_all(ROOT, VAULT_KEY) else {
            return Vec::new();
        };
        all.into_iter()
            .filter_map(|(value, _)| match value {
                Value::Scalar(v) => match v.as_ref() {
                    ScalarValue::Str(text) => Some(text.to_string()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }
}

fn empty_vault() -> String {
    r#"{"clock":0,"sidebar":[],"handles":[],"trash":[],"prefs":{}}"#.to_string()
}

/// Merge every concurrent copy. A later revision wins. A tie keeps the page
/// that is inside a folder.
pub fn merge_vaults(blobs: &[String]) -> String {
    let vaults: Vec<Vault> = blobs.iter().map(|blob| parse_vault(blob)).collect();
    if vaults.is_empty() {
        return empty_vault();
    }
    let mut clock = vaults.iter().map(|vault| vault.clock).max().unwrap_or(0);
    let sidebar = merge_places(vaults.iter().flat_map(|vault| vault.sidebar.iter()));
    let handles = merge_handles(vaults.iter().flat_map(|vault| vault.handles.iter()));
    let trash = merge_trash(vaults.iter().flat_map(|vault| vault.trash.iter()));
    let mut prefs: HashMap<String, Pref> = HashMap::new();
    for vault in &vaults {
        for (key, pref) in &vault.prefs {
            match prefs.get(key) {
                Some(have) if !beats_pref(pref, have) => {}
                _ => {
                    prefs.insert(key.clone(), pref.clone());
                }
            }
        }
    }
    clock = clock
        .max(sidebar.iter().map(|row| row.rev).max().unwrap_or(0))
        .max(handles.iter().map(|row| row.rev).max().unwrap_or(0))
        .max(trash.iter().map(|row| row.rev).max().unwrap_or(0))
        .max(prefs.values().map(|row| row.rev).max().unwrap_or(0));
    write_vault(&Vault {
        clock,
        sidebar,
        handles,
        trash,
        prefs,
    })
}

/// Apply a device's current tree, trash, handles, and preferences.
/// `None` means nothing moved.
pub fn note_structural(current: &str, structural: &str) -> Option<String> {
    let vault = parse_vault(current);
    let incoming = parse_structural(structural);
    let mut clock = vault.clock;
    let mut changed = false;
    let sidebar = note_places(&vault.sidebar, &incoming.sidebar, &mut clock, &mut changed);
    let handles = note_handles(&vault.handles, &incoming.handles, &mut clock, &mut changed);
    let trash = note_trash(&vault.trash, &incoming.trash, &mut clock, &mut changed);
    let mut prefs = vault.prefs.clone();
    for (key, value) in &incoming.prefs {
        match prefs.get(key) {
            Some(have) if have.value == *value => {}
            _ => {
                clock += 1;
                changed = true;
                prefs.insert(
                    key.clone(),
                    Pref {
                        value: value.clone(),
                        rev: clock,
                    },
                );
            }
        }
    }
    if !changed {
        return None;
    }
    Some(write_vault(&Vault {
        clock,
        sidebar,
        handles,
        trash,
        prefs,
    }))
}

/// Nested sidebar plus the other shared pieces, with revisions removed.
pub fn present_vault(stored: &str) -> String {
    let vault = parse_vault(stored);
    let trashed: HashSet<String> = vault
        .trash
        .iter()
        .filter(|row| !row.gone)
        .map(|row| row.id.clone())
        .collect();
    let sidebar = nest(&vault.sidebar, "", 0, &trashed);
    let mut handles: Vec<_> = vault
        .handles
        .iter()
        .filter(|row| row.gone || !row.handle.is_empty())
        .map(|row| {
            json!({
                "id": row.id,
                "handle": if row.gone { "" } else { row.handle.as_str() },
            })
        })
        .collect();
    handles.sort_by(|a, b| {
        a.get("id")
            .and_then(Json::as_str)
            .unwrap_or("")
            .cmp(b.get("id").and_then(Json::as_str).unwrap_or(""))
    });
    let mut trash: Vec<_> = vault
        .trash
        .iter()
        .filter(|row| !row.gone)
        .map(|row| json!({"id": row.id, "text": row.text, "handle": row.handle}))
        .collect();
    trash.sort_by(|a, b| {
        a.get("id")
            .and_then(Json::as_str)
            .unwrap_or("")
            .cmp(b.get("id").and_then(Json::as_str).unwrap_or(""))
    });
    let mut prefs = Map::new();
    let mut keys: Vec<_> = vault.prefs.keys().cloned().collect();
    keys.sort();
    for key in keys {
        if let Some(pref) = vault.prefs.get(&key) {
            prefs.insert(key, pref.value.clone());
        }
    }
    json!({
        "sidebar": sidebar,
        "handles": handles,
        "trash": trash,
        "prefs": Json::Object(prefs),
    })
    .to_string()
}

fn nest(nodes: &[Place], parent: &str, depth: usize, trashed: &HashSet<String>) -> Vec<Json> {
    let mut kids: Vec<&Place> = nodes
        .iter()
        .filter(|node| !node.gone && node.parent == parent)
        .filter(|node| node.kind == "folder" || !trashed.contains(&node.id))
        .collect();
    kids.sort_by(|a, b| a.index.cmp(&b.index).then(a.id.cmp(&b.id)));
    let mut out = Vec::new();
    for node in kids {
        if node.kind == "folder" {
            if depth >= 2 {
                continue;
            }
            let mut row = json!({
                "type": "folder",
                "id": node.id,
                "name": node.name,
                "collapsed": node.collapsed,
                "children": nest(nodes, &node.id, depth + 1, trashed),
            });
            if let Some(lock) = &node.lock {
                row["lock"] = lock.clone();
            }
            out.push(row);
        } else if node.kind == "page" {
            out.push(json!({"type": "page", "id": node.id}));
        }
    }
    out
}

fn note_places(
    current: &[Place],
    incoming: &[Place],
    clock: &mut u64,
    changed: &mut bool,
) -> Vec<Place> {
    let mut next = Vec::new();
    let mut seen = HashSet::new();
    for node in incoming {
        if node.id.is_empty() {
            continue;
        }
        seen.insert(node.id.clone());
        if let Some(prev) = current.iter().find(|row| row.id == node.id)
            && !prev.gone
            && same_place(prev, node)
        {
            next.push(prev.clone());
            continue;
        }
        *clock += 1;
        *changed = true;
        let mut row = node.clone();
        row.rev = *clock;
        row.gone = false;
        next.push(row);
    }
    for prev in current {
        if seen.contains(&prev.id) {
            continue;
        }
        if prev.gone {
            next.push(prev.clone());
            continue;
        }
        *clock += 1;
        *changed = true;
        let mut gone = prev.clone();
        gone.gone = true;
        gone.rev = *clock;
        next.push(gone);
    }
    next
}

fn note_handles(
    current: &[Handle],
    incoming: &[Handle],
    clock: &mut u64,
    changed: &mut bool,
) -> Vec<Handle> {
    let mut next = Vec::new();
    let mut seen = HashSet::new();
    for node in incoming {
        if node.id.is_empty() {
            continue;
        }
        seen.insert(node.id.clone());
        if let Some(prev) = current.iter().find(|row| row.id == node.id)
            && !prev.gone
            && prev.handle == node.handle
        {
            next.push(prev.clone());
            continue;
        }
        *clock += 1;
        *changed = true;
        next.push(Handle {
            id: node.id.clone(),
            handle: node.handle.clone(),
            rev: *clock,
            gone: false,
        });
    }
    for prev in current {
        if seen.contains(&prev.id) {
            continue;
        }
        if prev.gone {
            next.push(prev.clone());
            continue;
        }
        *clock += 1;
        *changed = true;
        let mut gone = prev.clone();
        gone.gone = true;
        gone.rev = *clock;
        next.push(gone);
    }
    next
}

fn note_trash(
    current: &[Trashed],
    incoming: &[Trashed],
    clock: &mut u64,
    changed: &mut bool,
) -> Vec<Trashed> {
    let mut next = Vec::new();
    let mut seen = HashSet::new();
    for node in incoming {
        if node.id.is_empty() {
            continue;
        }
        seen.insert(node.id.clone());
        if let Some(prev) = current.iter().find(|row| row.id == node.id)
            && !prev.gone
            && prev.text == node.text
            && prev.handle == node.handle
        {
            next.push(prev.clone());
            continue;
        }
        *clock += 1;
        *changed = true;
        next.push(Trashed {
            id: node.id.clone(),
            text: node.text.clone(),
            handle: node.handle.clone(),
            rev: *clock,
            gone: false,
        });
    }
    for prev in current {
        if seen.contains(&prev.id) {
            continue;
        }
        if prev.gone {
            next.push(prev.clone());
            continue;
        }
        *clock += 1;
        *changed = true;
        let mut gone = prev.clone();
        gone.gone = true;
        gone.rev = *clock;
        next.push(gone);
    }
    next
}

fn same_place(prev: &Place, next: &Place) -> bool {
    prev.kind == next.kind
        && prev.name == next.name
        && prev.parent == next.parent
        && prev.index == next.index
        && prev.collapsed == next.collapsed
        && prev.lock == next.lock
}

fn merge_places<'a>(rows: impl Iterator<Item = &'a Place>) -> Vec<Place> {
    let mut best: HashMap<String, Place> = HashMap::new();
    for row in rows {
        if row.id.is_empty() {
            continue;
        }
        match best.get(&row.id) {
            Some(have) if !beats_place(row, have) => {}
            _ => {
                best.insert(row.id.clone(), row.clone());
            }
        }
    }
    let mut out: Vec<_> = best.into_values().collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn merge_handles<'a>(rows: impl Iterator<Item = &'a Handle>) -> Vec<Handle> {
    let mut best: HashMap<String, Handle> = HashMap::new();
    for row in rows {
        if row.id.is_empty() {
            continue;
        }
        match best.get(&row.id) {
            Some(have) if !beats_rev(row.rev, &row.handle, have.rev, &have.handle) => {}
            _ => {
                best.insert(row.id.clone(), row.clone());
            }
        }
    }
    let mut out: Vec<_> = best.into_values().collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn merge_trash<'a>(rows: impl Iterator<Item = &'a Trashed>) -> Vec<Trashed> {
    let mut best: HashMap<String, Trashed> = HashMap::new();
    for row in rows {
        if row.id.is_empty() {
            continue;
        }
        match best.get(&row.id) {
            Some(have) if row.rev < have.rev || (row.rev == have.rev && row.text < have.text) => {}
            _ => {
                best.insert(row.id.clone(), row.clone());
            }
        }
    }
    let mut out: Vec<_> = best.into_values().collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn beats_place(row: &Place, have: &Place) -> bool {
    if row.rev != have.rev {
        return row.rev > have.rev;
    }
    let nested = !row.parent.is_empty() && !row.gone;
    let other = !have.parent.is_empty() && !have.gone;
    if nested != other {
        return nested;
    }
    (row.parent.as_str(), row.index, row.name.as_str(), !row.gone)
        > (
            have.parent.as_str(),
            have.index,
            have.name.as_str(),
            !have.gone,
        )
}

fn beats_rev(rev: u64, name: &str, have_rev: u64, have_name: &str) -> bool {
    rev > have_rev || (rev == have_rev && name > have_name)
}

fn beats_pref(row: &Pref, have: &Pref) -> bool {
    if row.rev != have.rev {
        return row.rev > have.rev;
    }
    // Value has no order of its own. The same revision keeps the greater JSON text,
    // so a named theme beats "auto".
    #[allow(clippy::cmp_owned)]
    {
        row.value.to_string() > have.value.to_string()
    }
}

fn parse_vault(text: &str) -> Vault {
    let Ok(value) = serde_json::from_str::<Json>(text) else {
        return Vault::default();
    };
    let mut vault = Vault {
        clock: value.get("clock").and_then(Json::as_u64).unwrap_or(0),
        ..Vault::default()
    };
    if let Some(rows) = value.get("sidebar").and_then(Json::as_array) {
        for row in rows {
            let Some(id) = row.get("id").and_then(Json::as_str) else {
                continue;
            };
            if id.is_empty() {
                continue;
            }
            vault.sidebar.push(Place {
                id: id.to_string(),
                kind: row
                    .get("kind")
                    .and_then(Json::as_str)
                    .unwrap_or("page")
                    .to_string(),
                name: row
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                parent: row
                    .get("parent")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                index: row.get("index").and_then(Json::as_u64).unwrap_or(0),
                rev: row.get("rev").and_then(Json::as_u64).unwrap_or(0),
                collapsed: row
                    .get("collapsed")
                    .and_then(Json::as_bool)
                    .unwrap_or(false),
                gone: row.get("gone").and_then(Json::as_bool).unwrap_or(false),
                lock: row.get("lock").filter(|lock| lock.is_object()).cloned(),
            });
        }
    }
    if let Some(rows) = value.get("handles").and_then(Json::as_array) {
        for row in rows {
            let Some(id) = row.get("id").and_then(Json::as_str) else {
                continue;
            };
            vault.handles.push(Handle {
                id: id.to_string(),
                handle: row
                    .get("handle")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                rev: row.get("rev").and_then(Json::as_u64).unwrap_or(0),
                gone: row.get("gone").and_then(Json::as_bool).unwrap_or(false),
            });
        }
    }
    if let Some(rows) = value.get("trash").and_then(Json::as_array) {
        for row in rows {
            let Some(id) = row.get("id").and_then(Json::as_str) else {
                continue;
            };
            vault.trash.push(Trashed {
                id: id.to_string(),
                text: row
                    .get("text")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                handle: row
                    .get("handle")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                rev: row.get("rev").and_then(Json::as_u64).unwrap_or(0),
                gone: row.get("gone").and_then(Json::as_bool).unwrap_or(false),
            });
        }
    }
    if let Some(prefs) = value.get("prefs").and_then(Json::as_object) {
        for (key, row) in prefs {
            let (pref_value, rev) = if let Some(obj) = row.as_object() {
                if obj.contains_key("v") || obj.contains_key("rev") {
                    (
                        obj.get("v").cloned().unwrap_or(Json::Null),
                        obj.get("rev").and_then(Json::as_u64).unwrap_or(0),
                    )
                } else {
                    (row.clone(), 0)
                }
            } else {
                (row.clone(), 0)
            };
            vault.prefs.insert(
                key.clone(),
                Pref {
                    value: pref_value,
                    rev,
                },
            );
        }
    }
    vault
}

struct Structural {
    sidebar: Vec<Place>,
    handles: Vec<Handle>,
    trash: Vec<Trashed>,
    prefs: HashMap<String, Json>,
}

fn parse_structural(text: &str) -> Structural {
    let Ok(value) = serde_json::from_str::<Json>(text) else {
        return Structural {
            sidebar: Vec::new(),
            handles: Vec::new(),
            trash: Vec::new(),
            prefs: HashMap::new(),
        };
    };
    let mut structural = Structural {
        sidebar: Vec::new(),
        handles: Vec::new(),
        trash: Vec::new(),
        prefs: HashMap::new(),
    };
    if let Some(rows) = value.get("sidebar").and_then(Json::as_array) {
        for row in rows {
            let Some(id) = row.get("id").and_then(Json::as_str) else {
                continue;
            };
            structural.sidebar.push(Place {
                id: id.to_string(),
                kind: row
                    .get("kind")
                    .and_then(Json::as_str)
                    .unwrap_or("page")
                    .to_string(),
                name: row
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                parent: row
                    .get("parent")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                index: row.get("index").and_then(Json::as_u64).unwrap_or(0),
                collapsed: row
                    .get("collapsed")
                    .and_then(Json::as_bool)
                    .unwrap_or(false),
                gone: false,
                rev: 0,
                lock: row.get("lock").filter(|lock| lock.is_object()).cloned(),
            });
        }
    }
    if let Some(rows) = value.get("handles").and_then(Json::as_array) {
        for row in rows {
            let Some(id) = row.get("id").and_then(Json::as_str) else {
                continue;
            };
            structural.handles.push(Handle {
                id: id.to_string(),
                handle: row
                    .get("handle")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                rev: 0,
                gone: false,
            });
        }
    }
    if let Some(rows) = value.get("trash").and_then(Json::as_array) {
        for row in rows {
            let Some(id) = row.get("id").and_then(Json::as_str) else {
                continue;
            };
            structural.trash.push(Trashed {
                id: id.to_string(),
                text: row
                    .get("text")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                handle: row
                    .get("handle")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                rev: 0,
                gone: false,
            });
        }
    }
    if let Some(prefs) = value.get("prefs").and_then(Json::as_object) {
        for (key, value) in prefs {
            if matches!(
                key.as_str(),
                "theme" | "hideAnswers" | "lockMinutes" | "replaceExports" | "checkForUpdates"
            ) {
                structural.prefs.insert(key.clone(), value.clone());
            }
        }
    }
    structural
}

fn write_vault(vault: &Vault) -> String {
    let sidebar: Vec<_> = vault
        .sidebar
        .iter()
        .map(|row| {
            let mut value = json!({
                "id": row.id,
                "kind": row.kind,
                "name": row.name,
                "parent": row.parent,
                "index": row.index,
                "rev": row.rev,
                "collapsed": row.collapsed,
                "gone": row.gone,
            });
            if let Some(lock) = &row.lock {
                value["lock"] = lock.clone();
            }
            value
        })
        .collect();
    let handles: Vec<_> = vault
        .handles
        .iter()
        .map(|row| {
            json!({
                "id": row.id,
                "handle": row.handle,
                "rev": row.rev,
                "gone": row.gone,
            })
        })
        .collect();
    let trash: Vec<_> = vault
        .trash
        .iter()
        .map(|row| {
            json!({
                "id": row.id,
                "text": row.text,
                "handle": row.handle,
                "rev": row.rev,
                "gone": row.gone,
            })
        })
        .collect();
    let mut prefs = Map::new();
    let mut keys: Vec<_> = vault.prefs.keys().cloned().collect();
    keys.sort();
    for key in keys {
        if let Some(pref) = vault.prefs.get(&key) {
            prefs.insert(key, json!({"v": pref.value, "rev": pref.rev}));
        }
    }
    json!({
        "clock": vault.clock,
        "sidebar": sidebar,
        "handles": handles,
        "trash": trash,
        "prefs": Json::Object(prefs),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(id: &str, kind: &str, name: &str, parent: &str, index: u64, rev: u64) -> Json {
        json!({
            "id": id,
            "kind": kind,
            "name": name,
            "parent": parent,
            "index": index,
            "rev": rev,
            "collapsed": false,
            "gone": false,
        })
    }

    #[test]
    fn a_folder_and_the_third_page_win_over_a_flat_list() {
        let desktop = json!({
            "clock": 6,
            "sidebar": [
                place("pantry", "folder", "Pantry", "", 0, 6),
                place("oats", "page", "", "pantry", 0, 6),
                place("ferry", "page", "", "pantry", 1, 6),
                place("bread", "page", "", "pantry", 2, 6),
                place("cake", "page", "", "pantry", 3, 6),
            ],
            "handles": [{"id": "ferry", "handle": "pier", "rev": 4, "gone": false}],
            "trash": [{"id": "kept", "text": "Kept\nold\n", "handle": "kept", "rev": 5, "gone": false}],
            "prefs": {"theme": {"v": "dusk", "rev": 3}, "hideAnswers": {"v": false, "rev": 1}}
        });
        let web = json!({
            "clock": 1,
            "sidebar": [
                place("oats", "page", "", "", 0, 1),
                place("ferry", "page", "", "", 1, 1),
                place("cake", "page", "", "", 2, 1),
                place("bread", "page", "", "", 3, 1),
            ],
            "handles": [{"id": "ferry", "handle": "dock", "rev": 1, "gone": false}],
            "trash": [],
            "prefs": {"theme": {"v": "light", "rev": 1}}
        });
        let merged = merge_vaults(&[desktop.to_string(), web.to_string()]);
        let shown: Json = serde_json::from_str(&present_vault(&merged)).unwrap();
        let sidebar = shown.get("sidebar").and_then(Json::as_array).unwrap();
        assert_eq!(sidebar.len(), 1, "{shown}");
        assert_eq!(
            sidebar[0].get("name").and_then(Json::as_str),
            Some("Pantry")
        );
        let children = sidebar[0].get("children").and_then(Json::as_array).unwrap();
        let ids: Vec<_> = children
            .iter()
            .map(|row| row.get("id").and_then(Json::as_str).unwrap_or(""))
            .collect();
        assert_eq!(ids, vec!["oats", "ferry", "bread", "cake"]);
        let handles = shown.get("handles").and_then(Json::as_array).unwrap();
        assert_eq!(
            handles[0].get("handle").and_then(Json::as_str),
            Some("pier")
        );
        let trash = shown.get("trash").and_then(Json::as_array).unwrap();
        assert_eq!(trash[0].get("id").and_then(Json::as_str), Some("kept"));
        assert_eq!(
            shown.pointer("/prefs/theme").and_then(Json::as_str),
            Some("dusk")
        );
    }

    #[test]
    fn the_same_revision_keeps_the_page_inside_the_folder() {
        let nested = json!({
            "clock": 1,
            "sidebar": [
                place("pantry", "folder", "Pantry", "", 0, 1),
                place("oats", "page", "", "pantry", 0, 1),
            ],
        });
        let flat = json!({
            "clock": 1,
            "sidebar": [place("oats", "page", "", "", 0, 1)],
        });
        let merged = merge_vaults(&[flat.to_string(), nested.to_string()]);
        let shown: Json = serde_json::from_str(&present_vault(&merged)).unwrap();
        let sidebar = shown.get("sidebar").and_then(Json::as_array).unwrap();
        assert_eq!(
            sidebar[0].get("type").and_then(Json::as_str),
            Some("folder")
        );
        let child = &sidebar[0].get("children").and_then(Json::as_array).unwrap()[0];
        assert_eq!(child.get("id").and_then(Json::as_str), Some("oats"));
    }

    #[test]
    fn restore_removes_the_page_from_recently_deleted() {
        let stored = json!({
            "clock": 2,
            "trash": [{"id": "oats", "text": "Oats\n1\n", "handle": "oats", "rev": 1, "gone": false}],
        })
        .to_string();
        let structural =
            r#"{"trash":[],"sidebar":[{"id":"oats","kind":"page","parent":"","index":0}]}"#;
        let next = note_structural(&stored, structural).unwrap();
        let shown: Json = serde_json::from_str(&present_vault(&next)).unwrap();
        assert!(
            shown
                .get("trash")
                .and_then(Json::as_array)
                .unwrap()
                .is_empty()
        );
        let again = note_structural(&next, structural);
        assert!(again.is_none(), "an unchanged library was written again");
    }

    #[test]
    fn a_later_theme_wins_and_a_lock_is_kept() {
        let structural = json!({
            "sidebar": [{
                "id": "pantry",
                "kind": "folder",
                "name": "Pantry",
                "parent": "",
                "index": 0,
                "collapsed": true,
                "lock": {"salt": "s", "nonce": "n", "wrapped": "w", "memory_kib": 19456, "iterations": 2, "lanes": 1}
            }],
            "prefs": {"theme": "paper", "hideAnswers": true, "lockMinutes": 15}
        });
        let next = note_structural(&empty_vault(), &structural.to_string()).unwrap();
        let shown: Json = serde_json::from_str(&present_vault(&next)).unwrap();
        assert_eq!(
            shown.pointer("/prefs/theme").and_then(Json::as_str),
            Some("paper")
        );
        assert_eq!(
            shown.pointer("/prefs/hideAnswers").and_then(Json::as_bool),
            Some(true)
        );
        assert_eq!(
            shown.pointer("/prefs/lockMinutes").and_then(Json::as_u64),
            Some(15)
        );
        assert_eq!(
            shown
                .pointer("/sidebar/0/lock/wrapped")
                .and_then(Json::as_str),
            Some("w")
        );
        assert_eq!(
            shown
                .pointer("/sidebar/0/collapsed")
                .and_then(Json::as_bool),
            Some(true)
        );
        let later = json!({"prefs": {"theme": "dark"}}).to_string();
        let updated = note_structural(&next, &later).unwrap();
        let shown: Json = serde_json::from_str(&present_vault(&updated)).unwrap();
        assert_eq!(
            shown.pointer("/prefs/theme").and_then(Json::as_str),
            Some("dark")
        );
        assert_eq!(
            shown.pointer("/prefs/hideAnswers").and_then(Json::as_bool),
            Some(true)
        );
    }

    #[test]
    fn a_cleared_handle_is_shown_as_empty_so_the_old_name_leaves() {
        let stored = json!({
            "clock": 2,
            "handles": [{"id": "ferry", "handle": "dock", "rev": 1, "gone": false}],
        })
        .to_string();
        let structural = r#"{"handles":[]}"#;
        let next = note_structural(&stored, structural).unwrap();
        let shown: Json = serde_json::from_str(&present_vault(&next)).unwrap();
        let handles = shown.get("handles").and_then(Json::as_array).unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].get("id").and_then(Json::as_str), Some("ferry"));
        assert_eq!(handles[0].get("handle").and_then(Json::as_str), Some(""));
    }

    #[test]
    fn a_blank_copy_keeps_the_shared_sidebar() {
        let mut founder = crate::Library::new();
        assert!(founder.note_vault(
            r#"{"sidebar":[{"id":"pantry","kind":"folder","name":"Pantry","parent":"","index":0},{"id":"oats","kind":"page","parent":"pantry","index":0}],"trash":[{"id":"kept","text":"old loaf"}],"prefs":{"theme":"dusk"}}"#
        ));
        founder.reconcile("oats", "# Oats\nbowl 2\n", None);
        let mut blank = crate::Library::new();
        let founder_bytes = founder.save();
        let blank_bytes = blank.save();
        let (doc, _) = crate::merge_libraries(
            &blank_bytes,
            &[],
            &founder_bytes,
            &[("oats".to_string(), "# Oats\nbowl 2\n".to_string())],
        );
        let shown = crate::Library::load(&doc).unwrap();
        let present = shown.vault_present().unwrap_or_default();
        assert!(present.contains("Pantry"), "{present}");
        assert!(present.contains("kept"), "{present}");
        assert!(present.contains("dusk"), "{present}");
    }
}
