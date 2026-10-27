//! One sealed object per page, plus the shared sidebar when there is one,
//! and a small sealed manifest.
//!
//! The manifest names the pages, their object ids, and the key for each object.
//! A save seals only the pages whose text changed. Object ids are random.
//! Page ids stay inside the manifest.

use std::collections::HashSet;

use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::sync_keys::{self, PageSet, SyncKeyError, SyncMeta, VaultKey};

pub const MANIFEST_MAGIC: &[u8] = b"OMACUS-MANIFEST/1\n";
const VAULT_MARK: &[u8] = b"VAULT";
/// `VAULT` || object id || content key || revision || sha256.
const SHARED_TAIL: usize = 5 + 16 + 32 + 8 + 32;
pub const PAGE_MAGIC: &[u8] = b"OMACUS-PAGE/1\n";
pub const MANIFEST_ID: &[u8] = b"manifest";
pub const MAX_MANIFEST: usize = 1024 * 1024;
pub const MAX_OBJECT: usize = 4 * 1024 * 1024 + 64;
pub const MAX_PAGE_COUNT: usize = 100_000;

const BUCKETS: &[usize] = &[256, 1024, 4096, 16384, 65536, 262144, 1_048_576, 4_194_304];

#[derive(Clone)]
pub struct Slot {
    pub page_id: String,
    pub object_id: [u8; 16],
    pub hash: [u8; 32],
    pub content_key: [u8; 32],
    pub vault_id: [u8; 16],
    pub revision: u64,
    pub server_rev: u64,
    pub doc: Vec<u8>,
}

pub struct Outgoing {
    pub id: [u8; 16],
    pub body: Vec<u8>,
    pub base: u64,
}

pub struct Incoming {
    pub id: [u8; 16],
    pub revision: u64,
    pub body: Vec<u8>,
}

pub struct Planned {
    pub manifest: Vec<u8>,
    pub sealed_manifest: Vec<u8>,
    pub objects: Vec<Outgoing>,
    pub drop_ids: Vec<[u8; 16]>,
    pub slots: Vec<Slot>,
}

pub struct Assembled {
    pub packed: Vec<u8>,
    pub slots: Vec<Slot>,
}

struct Entry {
    object_id: [u8; 16],
    page_id: String,
    revision: u64,
    hash: [u8; 32],
    content_key: [u8; 32],
}

struct FolderRow {
    id: String,
    name: String,
    parent: Option<String>,
}

pub fn is_v2(bytes: &[u8]) -> bool {
    bytes.starts_with(b"OMACUS-SYNC/2\n")
}

pub fn content_hash(text: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher.finalize().into()
}

pub fn bucket_for(len: usize) -> Result<usize, SyncKeyError> {
    BUCKETS
        .iter()
        .copied()
        .find(|bucket| *bucket >= len)
        .ok_or(SyncKeyError::Corrupt)
}

fn object_aad(object_id: &[u8; 16]) -> Vec<u8> {
    let mut aad = b"omacus-object-v1".to_vec();
    aad.extend_from_slice(object_id);
    aad
}

pub fn seal_object(
    key: &[u8; 32],
    object_id: &[u8; 16],
    doc: &[u8],
) -> Result<Vec<u8>, SyncKeyError> {
    let mut plain = Vec::new();
    plain.extend_from_slice(PAGE_MAGIC);
    let len = u32::try_from(doc.len()).map_err(|_| SyncKeyError::Corrupt)?;
    plain.extend_from_slice(&len.to_be_bytes());
    plain.extend_from_slice(doc);
    let bucket = bucket_for(plain.len())?;
    plain.resize(bucket, 0);
    sync_keys::seal_with_key(key, &object_aad(object_id), &plain)
}

pub fn open_object(
    key: &[u8; 32],
    object_id: &[u8; 16],
    stored: &[u8],
) -> Result<Vec<u8>, SyncKeyError> {
    if stored.len() > MAX_OBJECT {
        return Err(SyncKeyError::Corrupt);
    }
    let plain = sync_keys::open_with_key(key, &object_aad(object_id), stored)?;
    if plain.len() < PAGE_MAGIC.len() + 4 || !plain.starts_with(PAGE_MAGIC) {
        return Err(SyncKeyError::Corrupt);
    }
    let mut at = PAGE_MAGIC.len();
    let len = u32::from_be_bytes(
        plain
            .get(at..at + 4)
            .ok_or(SyncKeyError::Corrupt)?
            .try_into()
            .map_err(|_| SyncKeyError::Corrupt)?,
    );
    at += 4;
    let len = usize::try_from(len).map_err(|_| SyncKeyError::Corrupt)?;
    let end = at.checked_add(len).ok_or(SyncKeyError::Corrupt)?;
    if end > plain.len() {
        return Err(SyncKeyError::Corrupt);
    }
    Ok(plain[at..end].to_vec())
}

pub fn page_document(previous: &[u8], page_id: &str, text: &str) -> Result<Vec<u8>, SyncKeyError> {
    let mut lib = if previous.is_empty() {
        omacus_doc::Library::new()
    } else {
        omacus_doc::Library::load(previous).map_err(|_| SyncKeyError::Corrupt)?
    };
    if lib.snapshot(page_id).as_deref() != Some(text) {
        lib.reconcile(page_id, text, None);
        lib.note_written(page_id, text);
    }
    if lib.snapshot(page_id).as_deref() != Some(text) {
        return Err(SyncKeyError::Corrupt);
    }
    Ok(lib.save())
}

pub fn page_text(doc: &[u8], page_id: &str) -> Result<String, SyncKeyError> {
    let lib = omacus_doc::Library::load(doc).map_err(|_| SyncKeyError::Corrupt)?;
    lib.snapshot(page_id).ok_or(SyncKeyError::Corrupt)
}

pub fn merge_page(
    page_id: &str,
    local_doc: &[u8],
    local_text: &str,
    remote_doc: &[u8],
    remote_text: &str,
) -> Result<(Vec<u8>, String), SyncKeyError> {
    let local_pages = vec![(page_id.to_string(), local_text.to_string())];
    let remote_pages = vec![(page_id.to_string(), remote_text.to_string())];
    let (doc, pages) =
        omacus_doc::merge_libraries(local_doc, &local_pages, remote_doc, &remote_pages);
    let text = pages
        .into_iter()
        .find(|(id, _)| id == page_id)
        .map(|(_, text)| text)
        .unwrap_or_default();
    Ok((doc, text))
}

/// Fold two copies of one page that both started from `ancestor`.
/// A copy that still matches `ancestor` takes the other one, so a deleted line
/// stays deleted. A line added on one side stays.
pub fn three_way(
    page_id: &str,
    ancestor: &str,
    local: &str,
    remote: &str,
) -> Result<String, SyncKeyError> {
    if local == ancestor {
        return Ok(remote.to_string());
    }
    if remote == ancestor || local == remote {
        return Ok(local.to_string());
    }
    let base = page_document(b"", page_id, ancestor)?;
    let ours = page_document(&base, page_id, local)?;
    let theirs = page_document(&base, page_id, remote)?;
    let (_doc, text) = merge_page(page_id, &ours, local, &theirs, remote)?;
    Ok(text)
}

fn place_in(doc: &[u8], page_id: &str) -> Option<String> {
    omacus_doc::Library::load(doc).ok()?.place_of(page_id)
}

fn place_of_doc(doc: &[u8], page_id: &str) -> Option<String> {
    if doc.is_empty() {
        return None;
    }
    place_in(doc, page_id)
}

fn with_place(doc: &[u8], page_id: &str, folder: Option<&str>) -> Result<Vec<u8>, SyncKeyError> {
    let mut lib = omacus_doc::Library::load(doc).map_err(|_| SyncKeyError::Corrupt)?;
    if lib.place_of(page_id).as_deref() != folder {
        lib.set_place(page_id, folder);
    }
    Ok(lib.save())
}

fn folders_of(doc: &[u8]) -> Vec<FolderRow> {
    if doc.is_empty() {
        return Vec::new();
    }
    let Ok(lib) = omacus_doc::Library::load(doc) else {
        return Vec::new();
    };
    lib.folders()
        .into_iter()
        .map(|folder| FolderRow {
            id: folder.id,
            name: folder.name,
            parent: folder.parent,
        })
        .collect()
}

fn push_str(out: &mut Vec<u8>, text: &str) -> Result<(), SyncKeyError> {
    let bytes = text.as_bytes();
    if bytes.len() > 200 {
        return Err(SyncKeyError::Corrupt);
    }
    let len = u16::try_from(bytes.len()).map_err(|_| SyncKeyError::Corrupt)?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

struct SharedSeal {
    object_id: [u8; 16],
    content_key: [u8; 32],
    revision: u64,
    hash: [u8; 32],
}

fn encode_manifest(
    meta: &SyncMeta,
    slots: &[Slot],
    folders: &[FolderRow],
    generation: u64,
    shared: Option<&Slot>,
) -> Result<Vec<u8>, SyncKeyError> {
    if slots.len() > MAX_PAGE_COUNT || folders.len() > 4096 {
        return Err(SyncKeyError::Corrupt);
    }
    let meta_bytes = sync_keys::meta_to_bytes(meta)?;
    let mut out = Vec::new();
    out.extend_from_slice(MANIFEST_MAGIC);
    let meta_len = u32::try_from(meta_bytes.len()).map_err(|_| SyncKeyError::Corrupt)?;
    out.extend_from_slice(&meta_len.to_be_bytes());
    out.extend_from_slice(&meta_bytes);
    let folders_len = u16::try_from(folders.len()).map_err(|_| SyncKeyError::Corrupt)?;
    out.extend_from_slice(&folders_len.to_be_bytes());
    for folder in folders {
        push_str(&mut out, &folder.id)?;
        push_str(&mut out, &folder.name)?;
        match &folder.parent {
            Some(parent) => {
                out.push(1);
                push_str(&mut out, parent)?;
            }
            None => out.push(0),
        }
    }
    let count = u32::try_from(slots.len()).map_err(|_| SyncKeyError::Corrupt)?;
    out.extend_from_slice(&count.to_be_bytes());
    for slot in slots {
        out.extend_from_slice(&slot.object_id);
        push_str(&mut out, &slot.page_id)?;
        out.extend_from_slice(&slot.revision.to_be_bytes());
        out.extend_from_slice(&slot.hash);
        out.extend_from_slice(&slot.content_key);
    }
    out.extend_from_slice(&generation.to_be_bytes());
    if let Some(slot) = shared {
        out.extend_from_slice(VAULT_MARK);
        out.extend_from_slice(&slot.object_id);
        out.extend_from_slice(&slot.content_key);
        out.extend_from_slice(&slot.revision.to_be_bytes());
        out.extend_from_slice(&slot.hash);
    }
    if out.len() > MAX_MANIFEST {
        return Err(SyncKeyError::Corrupt);
    }
    Ok(out)
}

struct Decoded {
    meta: SyncMeta,
    folders: Vec<FolderRow>,
    entries: Vec<Entry>,
    generation: u64,
    shared: Option<SharedSeal>,
}

fn take_str(bytes: &[u8], at: &mut usize) -> Result<String, SyncKeyError> {
    if *at + 2 > bytes.len() {
        return Err(SyncKeyError::Corrupt);
    }
    let len = u16::from_be_bytes([bytes[*at], bytes[*at + 1]]) as usize;
    *at += 2;
    if len > 200 || *at + len > bytes.len() {
        return Err(SyncKeyError::Corrupt);
    }
    let text = std::str::from_utf8(&bytes[*at..*at + len]).map_err(|_| SyncKeyError::Corrupt)?;
    *at += len;
    if text.chars().any(|ch| ch.is_control()) {
        return Err(SyncKeyError::Corrupt);
    }
    Ok(text.to_string())
}

fn decode_manifest(bytes: &[u8]) -> Result<Decoded, SyncKeyError> {
    if bytes.len() > MAX_MANIFEST || !bytes.starts_with(MANIFEST_MAGIC) {
        return Err(SyncKeyError::Corrupt);
    }
    let mut at = MANIFEST_MAGIC.len();
    if at + 4 > bytes.len() {
        return Err(SyncKeyError::Corrupt);
    }
    let meta_len = u32::from_be_bytes(
        bytes[at..at + 4]
            .try_into()
            .map_err(|_| SyncKeyError::Corrupt)?,
    ) as usize;
    at += 4;
    if at + meta_len > bytes.len() {
        return Err(SyncKeyError::Corrupt);
    }
    let meta = sync_keys::meta_from_bytes(&bytes[at..at + meta_len])?;
    at += meta_len;
    if at + 2 > bytes.len() {
        return Err(SyncKeyError::Corrupt);
    }
    let folder_count = u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
    at += 2;
    if folder_count > 4096 {
        return Err(SyncKeyError::Corrupt);
    }
    let mut folders = Vec::with_capacity(folder_count);
    for _ in 0..folder_count {
        let id = take_str(bytes, &mut at)?;
        let name = take_str(bytes, &mut at)?;
        if at >= bytes.len() {
            return Err(SyncKeyError::Corrupt);
        }
        let flag = bytes[at];
        at += 1;
        let parent = if flag == 0 {
            None
        } else if flag == 1 {
            Some(take_str(bytes, &mut at)?)
        } else {
            return Err(SyncKeyError::Corrupt);
        };
        if id.is_empty() {
            return Err(SyncKeyError::Corrupt);
        }
        folders.push(FolderRow { id, name, parent });
    }
    if at + 4 > bytes.len() {
        return Err(SyncKeyError::Corrupt);
    }
    let count = u32::from_be_bytes(
        bytes[at..at + 4]
            .try_into()
            .map_err(|_| SyncKeyError::Corrupt)?,
    ) as usize;
    at += 4;
    if count > MAX_PAGE_COUNT {
        return Err(SyncKeyError::Corrupt);
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if at + 16 > bytes.len() {
            return Err(SyncKeyError::Corrupt);
        }
        let mut object_id = [0u8; 16];
        object_id.copy_from_slice(&bytes[at..at + 16]);
        at += 16;
        let page_id = take_str(bytes, &mut at)?;
        if page_id.is_empty() || at + 8 + 32 + 32 > bytes.len() {
            return Err(SyncKeyError::Corrupt);
        }
        let revision = u64::from_be_bytes(
            bytes[at..at + 8]
                .try_into()
                .map_err(|_| SyncKeyError::Corrupt)?,
        );
        at += 8;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&bytes[at..at + 32]);
        at += 32;
        let mut content_key = [0u8; 32];
        content_key.copy_from_slice(&bytes[at..at + 32]);
        at += 32;
        entries.push(Entry {
            object_id,
            page_id,
            revision,
            hash,
            content_key,
        });
    }
    let generation = if at == bytes.len() {
        0
    } else {
        if at + 8 > bytes.len() {
            return Err(SyncKeyError::Corrupt);
        }
        let generation = u64::from_be_bytes(
            bytes[at..at + 8]
                .try_into()
                .map_err(|_| SyncKeyError::Corrupt)?,
        );
        at += 8;
        generation
    };
    let shared = if at == bytes.len() {
        None
    } else {
        Some(decode_shared(bytes, &mut at)?)
    };
    if at != bytes.len() {
        return Err(SyncKeyError::Corrupt);
    }
    Ok(Decoded {
        meta,
        folders,
        entries,
        generation,
        shared,
    })
}

fn decode_shared(bytes: &[u8], at: &mut usize) -> Result<SharedSeal, SyncKeyError> {
    if *at + SHARED_TAIL != bytes.len() || !bytes[*at..].starts_with(VAULT_MARK) {
        return Err(SyncKeyError::Corrupt);
    }
    *at += VAULT_MARK.len();
    let mut object_id = [0u8; 16];
    object_id.copy_from_slice(&bytes[*at..*at + 16]);
    *at += 16;
    let mut content_key = [0u8; 32];
    content_key.copy_from_slice(&bytes[*at..*at + 32]);
    *at += 32;
    let revision = u64::from_be_bytes(
        bytes[*at..*at + 8]
            .try_into()
            .map_err(|_| SyncKeyError::Corrupt)?,
    );
    *at += 8;
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes[*at..*at + 32]);
    *at += 32;
    Ok(SharedSeal {
        object_id,
        content_key,
        revision,
        hash,
    })
}

fn shared_json(doc: &[u8]) -> String {
    omacus_doc::Library::load(doc)
        .ok()
        .and_then(|lib| lib.vault_json())
        .unwrap_or_default()
}

/// The sidebar, trash, handles, and preferences travel as their own object.
/// An empty page id marks that slot. It is not a page.
fn seal_shared(
    previous: &[Slot],
    json: &str,
    vault: &VaultKey,
    objects: &mut Vec<Outgoing>,
) -> Result<Slot, SyncKeyError> {
    let hash = content_hash(json);
    if let Some(prev) = previous.iter().find(|slot| slot.page_id.is_empty()) {
        if prev.hash == hash && !prev.doc.is_empty() {
            let mut kept = prev.clone();
            kept.vault_id = *vault.id();
            return Ok(kept);
        }
        let mut key = sync_keys::random_bytes::<32>()?;
        let body = seal_object(&key, &prev.object_id, json.as_bytes())?;
        objects.push(Outgoing {
            id: prev.object_id,
            body,
            base: prev.server_rev,
        });
        let slot = Slot {
            page_id: String::new(),
            object_id: prev.object_id,
            hash,
            content_key: key,
            vault_id: *vault.id(),
            revision: prev.revision.saturating_add(1),
            server_rev: prev.server_rev,
            doc: json.as_bytes().to_vec(),
        };
        key.zeroize();
        return Ok(slot);
    }
    let mut key = sync_keys::random_bytes::<32>()?;
    let object_id = sync_keys::random_bytes::<16>()?;
    let body = seal_object(&key, &object_id, json.as_bytes())?;
    objects.push(Outgoing {
        id: object_id,
        body,
        base: 0,
    });
    let slot = Slot {
        page_id: String::new(),
        object_id,
        hash,
        content_key: key,
        vault_id: *vault.id(),
        revision: 1,
        server_rev: 0,
        doc: json.as_bytes().to_vec(),
    };
    key.zeroize();
    Ok(slot)
}

/// More than two signed deletes at once waits for a person to confirm.
/// One or two still apply. A page that is only missing is not a delete.
/// `local_pages` is the count on this device; the guard is the delete count.
pub fn mass_delete(local_pages: usize, doomed: usize) -> bool {
    let _ = local_pages;
    doomed > 2
}

pub fn manifest_generation(bytes: &[u8]) -> Result<u64, SyncKeyError> {
    Ok(decode_manifest(bytes)?.generation)
}

pub fn plan(
    vault: &VaultKey,
    epoch: u64,
    set: &PageSet,
    previous: &[Slot],
    generation: u64,
) -> Result<Planned, SyncKeyError> {
    if set.pages.len() > MAX_PAGE_COUNT {
        return Err(SyncKeyError::Corrupt);
    }
    let mut slots = Vec::new();
    let mut objects = Vec::new();
    let mut seen = HashSet::new();
    for (page_id, text) in &set.pages {
        if !set.meta.in_vault(page_id) {
            continue;
        }
        if page_id.is_empty() || !seen.insert(page_id.clone()) {
            return Err(SyncKeyError::Corrupt);
        }
        let hash = content_hash(text);
        let wanted_place = place_in(&set.doc, page_id);
        if let Some(prev) = previous.iter().find(|slot| slot.page_id == *page_id) {
            if prev.hash == hash
                && !prev.doc.is_empty()
                && place_of_doc(&prev.doc, page_id) == wanted_place
            {
                let mut kept = prev.clone();
                kept.vault_id = *vault.id();
                slots.push(kept);
                continue;
            }
            let doc = page_document(&prev.doc, page_id, text)?;
            let doc = with_place(&doc, page_id, wanted_place.as_deref())?;
            let mut key = sync_keys::random_bytes::<32>()?;
            let body = seal_object(&key, &prev.object_id, &doc)?;
            objects.push(Outgoing {
                id: prev.object_id,
                body,
                base: prev.server_rev,
            });
            slots.push(Slot {
                page_id: page_id.clone(),
                object_id: prev.object_id,
                hash,
                content_key: key,
                vault_id: *vault.id(),
                revision: prev.revision.saturating_add(1),
                server_rev: prev.server_rev,
                doc,
            });
            key.zeroize();
        } else {
            let doc = page_document(b"", page_id, text)?;
            let doc = with_place(&doc, page_id, wanted_place.as_deref())?;
            let mut key = sync_keys::random_bytes::<32>()?;
            let object_id = sync_keys::random_bytes::<16>()?;
            let body = seal_object(&key, &object_id, &doc)?;
            objects.push(Outgoing {
                id: object_id,
                body,
                base: 0,
            });
            slots.push(Slot {
                page_id: page_id.clone(),
                object_id,
                hash,
                content_key: key,
                vault_id: *vault.id(),
                revision: 1,
                server_rev: 0,
                doc,
            });
            key.zeroize();
        }
    }
    let doomed: Vec<&str> = set
        .meta
        .tombstones
        .iter()
        .map(|tomb| tomb.id.as_str())
        .collect();
    let named = previous
        .iter()
        .filter(|prev| doomed.iter().any(|id| *id == prev.page_id))
        .count();
    // A page that is only absent is not deleted. A signed tombstone is.
    // More than two at once drops nothing, until this device confirms.
    let mut drop_ids = if mass_delete(previous.len(), named) {
        Vec::new()
    } else {
        previous
            .iter()
            .filter(|prev| !prev.page_id.is_empty())
            .filter(|prev| doomed.iter().any(|id| *id == prev.page_id))
            .map(|prev| prev.object_id)
            .collect()
    };
    let folders = folders_of(&set.doc);
    let shared_json = shared_json(&set.doc);
    let shared = if shared_json.is_empty() {
        if let Some(prev) = previous.iter().find(|slot| slot.page_id.is_empty()) {
            drop_ids.push(prev.object_id);
        }
        None
    } else {
        Some(seal_shared(previous, &shared_json, vault, &mut objects)?)
    };
    let manifest = encode_manifest(&set.meta, &slots, &folders, generation, shared.as_ref())?;
    if let Some(slot) = shared {
        slots.push(slot);
    }
    let sealed_manifest =
        sync_keys::seal_manifest(vault, epoch, MANIFEST_ID, &manifest, generation)?;
    Ok(Planned {
        manifest,
        sealed_manifest,
        objects,
        drop_ids,
        slots,
    })
}

pub fn manifest_wants(manifest: &[u8], previous: &[Slot]) -> Result<Vec<[u8; 16]>, SyncKeyError> {
    let decoded = decode_manifest(manifest)?;
    let mut want = Vec::new();
    for entry in decoded.entries {
        let have = previous.iter().any(|slot| {
            slot.object_id == entry.object_id
                && slot.hash == entry.hash
                && slot.revision == entry.revision
                && !slot.doc.is_empty()
        });
        if !have {
            want.push(entry.object_id);
        }
    }
    if let Some(shared) = &decoded.shared {
        let have = previous.iter().any(|slot| {
            slot.page_id.is_empty()
                && slot.object_id == shared.object_id
                && slot.hash == shared.hash
                && slot.revision == shared.revision
                && !slot.doc.is_empty()
        });
        if !have {
            want.push(shared.object_id);
        }
    }
    Ok(want)
}

pub fn assemble(
    manifest: &[u8],
    incoming: &[Incoming],
    previous: &[Slot],
    base_doc: &[u8],
) -> Result<Assembled, SyncKeyError> {
    let decoded = decode_manifest(manifest)?;
    let mut pages = Vec::new();
    let mut slots = Vec::new();
    let mut seen_pages = HashSet::new();
    let mut seen_ids = HashSet::new();
    for entry in decoded.entries {
        if !seen_pages.insert(entry.page_id.clone()) || !seen_ids.insert(entry.object_id) {
            return Err(SyncKeyError::Corrupt);
        }
        let prev = previous.iter().find(|slot| slot.page_id == entry.page_id);
        let found = incoming.iter().find(|item| item.id == entry.object_id);
        let (doc, text, server_rev) = if let Some(item) = found {
            let doc = open_object(&entry.content_key, &entry.object_id, &item.body)?;
            let remote_text = page_text(&doc, &entry.page_id)?;
            if content_hash(&remote_text) != entry.hash {
                return Err(SyncKeyError::Corrupt);
            }
            if let Some(prev) = prev {
                if !prev.doc.is_empty() && prev.hash != entry.hash {
                    let local_text = page_text(&prev.doc, &entry.page_id).unwrap_or_default();
                    if local_text != remote_text {
                        let (merged_doc, merged_text) =
                            merge_page(&entry.page_id, &prev.doc, &local_text, &doc, &remote_text)?;
                        (merged_doc, merged_text, item.revision)
                    } else {
                        (doc, remote_text, item.revision)
                    }
                } else {
                    (doc, remote_text, item.revision)
                }
            } else {
                (doc, remote_text, item.revision)
            }
        } else if let Some(prev) = prev {
            if prev.object_id != entry.object_id || prev.hash != entry.hash || prev.doc.is_empty() {
                return Err(SyncKeyError::Corrupt);
            }
            let text = page_text(&prev.doc, &entry.page_id)?;
            if content_hash(&text) != entry.hash {
                return Err(SyncKeyError::Corrupt);
            }
            (prev.doc.clone(), text, prev.server_rev)
        } else {
            return Err(SyncKeyError::Corrupt);
        };
        let hash = content_hash(&text);
        let vault_id = prev.map(|slot| slot.vault_id).unwrap_or([0u8; 16]);
        let sealed = prev.and_then(|slot| page_text(&slot.doc, &entry.page_id).ok());
        pages.push((entry.page_id.clone(), text, sealed));
        slots.push(Slot {
            page_id: entry.page_id,
            object_id: entry.object_id,
            hash,
            content_key: entry.content_key,
            vault_id,
            revision: entry.revision,
            server_rev,
            doc,
        });
    }
    let mut lib = if base_doc.is_empty() {
        omacus_doc::Library::new()
    } else {
        omacus_doc::Library::load(base_doc).unwrap_or_else(|_| omacus_doc::Library::new())
    };
    let mut folded = Vec::with_capacity(pages.len());
    for (id, incoming, sealed) in &pages {
        let current = lib.snapshot(id);
        let text = match (current.as_deref(), sealed.as_deref()) {
            (None, _) => incoming.clone(),
            (Some(current), _) if current == incoming => current.to_string(),
            // This phone has not changed the page since the last sync, so the
            // incoming copy wins. A line deleted there stays deleted.
            (Some(current), Some(ancestor)) if current == ancestor => incoming.clone(),
            (Some(current), Some(ancestor)) => three_way(id, ancestor, current, incoming)?,
            (Some(current), None) => three_way(id, "", current, incoming)?,
        };
        if lib.snapshot(id).as_deref() != Some(text.as_str()) {
            lib.reconcile(id, &text, None);
            lib.note_written(id, &text);
        }
        folded.push((id.clone(), text));
    }
    if !decoded.folders.is_empty() {
        let seeds: Vec<omacus_doc::FolderSeed> = decoded
            .folders
            .iter()
            .map(|folder| omacus_doc::FolderSeed {
                id: folder.id.clone(),
                name: folder.name.clone(),
                parent: folder.parent.clone(),
            })
            .collect();
        lib.set_folders(&seeds);
    }
    for slot in &slots {
        // An older page object has no place. Leave the one already in this library.
        if let Some(folder) = place_of_doc(&slot.doc, &slot.page_id) {
            lib.set_place(&slot.page_id, Some(&folder));
        }
    }
    if let Some(shared) = decoded.shared {
        let prev = previous.iter().find(|slot| slot.page_id.is_empty());
        let found = incoming.iter().find(|item| item.id == shared.object_id);
        let (doc, server_rev) = if let Some(item) = found {
            let doc = open_object(&shared.content_key, &shared.object_id, &item.body)?;
            (doc, item.revision)
        } else if let Some(prev) = prev {
            if prev.object_id != shared.object_id
                || prev.hash != shared.hash
                || prev.revision != shared.revision
                || prev.doc.is_empty()
            {
                return Err(SyncKeyError::Corrupt);
            }
            (prev.doc.clone(), prev.server_rev)
        } else {
            return Err(SyncKeyError::Corrupt);
        };
        let json = std::str::from_utf8(&doc).map_err(|_| SyncKeyError::Corrupt)?;
        if content_hash(json) != shared.hash {
            return Err(SyncKeyError::Corrupt);
        }
        lib.adopt_vault(json);
        let vault_id = prev.map(|slot| slot.vault_id).unwrap_or([0u8; 16]);
        slots.push(Slot {
            page_id: String::new(),
            object_id: shared.object_id,
            hash: shared.hash,
            content_key: shared.content_key,
            vault_id,
            revision: shared.revision,
            server_rev,
            doc,
        });
    }
    let packed = PageSet {
        doc: lib.save(),
        pages: folded,
        meta: decoded.meta,
    }
    .to_bytes()?;
    Ok(Assembled { packed, slots })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_set(pages: Vec<(String, String)>) -> PageSet {
        PageSet {
            doc: Vec::new(),
            pages,
            meta: SyncMeta::default(),
        }
    }

    #[test]
    fn a_manifest_seal_binds_its_generation_and_refuses_an_older_one() {
        let vault = VaultKey::generate().unwrap();
        let set = sample_set(vec![("pantry".into(), "oats\n".into())]);
        let first = plan(&vault, 1, &set, &[], 1).unwrap();
        let second = plan(&vault, 1, &set, &first.slots, 2).unwrap();
        assert_eq!(
            sync_keys::manifest_seal_generation(&first.sealed_manifest),
            Some(1)
        );
        let opened =
            sync_keys::open_manifest_seal(&vault, 1, MANIFEST_ID, &second.sealed_manifest, 2)
                .unwrap();
        assert_eq!(manifest_generation(&opened).unwrap(), 2);
        let err = sync_keys::open_manifest_seal(&vault, 1, MANIFEST_ID, &first.sealed_manifest, 2)
            .unwrap_err();
        assert_eq!(err, SyncKeyError::Older);
        let mut claimed = first.sealed_manifest.clone();
        let at = b"OMACUS-SEAL/2\n".len();
        claimed[at..at + 8].copy_from_slice(&2u64.to_be_bytes());
        assert_eq!(
            sync_keys::open_manifest_seal(&vault, 1, MANIFEST_ID, &claimed, 0).unwrap_err(),
            SyncKeyError::Sealed
        );
        let legacy = sync_keys::seal_page(&vault, 1, MANIFEST_ID, &first.manifest).unwrap();
        let plain = sync_keys::open_manifest_seal(&vault, 1, MANIFEST_ID, &legacy, 0).unwrap();
        assert_eq!(plain, first.manifest);
    }

    #[test]
    fn a_changed_page_is_the_only_upload_and_rotation_skips_objects() {
        let vault = VaultKey::generate().unwrap();
        let set = sample_set(vec![
            ("pantry".into(), "oats\n".into()),
            ("groceries".into(), "lentils\n".into()),
        ]);
        let first = plan(&vault, 1, &set, &[], 1).unwrap();
        assert_eq!(manifest_generation(&first.manifest).unwrap(), 1);
        assert_eq!(first.objects.len(), 2);
        assert!(first.manifest.len() < 8 * 1024);
        let marker = b"lentils";
        assert!(
            !first
                .sealed_manifest
                .windows(marker.len())
                .any(|w| w == marker)
        );
        for object in &first.objects {
            assert!(!object.body.windows(marker.len()).any(|w| w == marker));
            assert_ne!(&object.id, b"groceries\0\0\0\0\0\0\0");
        }
        let mut slots = first.slots.clone();
        for slot in &mut slots {
            slot.server_rev = 1;
        }
        let mut edited = set.clone();
        edited.pages[0].1 = "oats\nmustard\n".into();
        let second = plan(&vault, 1, &edited, &slots, 1).unwrap();
        assert_eq!(second.objects.len(), 1);
        assert_eq!(second.objects[0].base, 1);
        assert_eq!(second.objects[0].id, slots[0].object_id);

        let rotated = VaultKey::generate().unwrap();
        let third = plan(&rotated, 1, &edited, &second.slots, 2).unwrap();
        assert_eq!(manifest_generation(&third.manifest).unwrap(), 2);
        let mut without_counter = first.manifest.clone();
        without_counter.truncate(without_counter.len() - 8);
        assert_eq!(
            manifest_generation(&without_counter).unwrap(),
            0,
            "a manifest from before the counter is generation zero"
        );
        assert!(
            third.objects.is_empty(),
            "rotation reseals the manifest only"
        );
        let only_pantry = sample_set(vec![("pantry".into(), "oats\n".into())]);
        let narrowed = plan(&vault, 1, &only_pantry, &first.slots, 2).unwrap();
        assert!(
            narrowed.drop_ids.is_empty(),
            "a page missing from this copy was dropped"
        );
        let incoming: Vec<Incoming> = second
            .objects
            .iter()
            .map(|object| Incoming {
                id: object.id,
                revision: 2,
                body: object.body.clone(),
            })
            .collect();
        let mut cached = slots;
        cached[0] = second.slots[0].clone();
        let opened = assemble(&third.manifest, &incoming, &cached, &[]).unwrap();
        let set = PageSet::from_bytes(&opened.packed).unwrap();
        assert_eq!(set.pages.len(), 2);
        assert!(set.pages.iter().any(|(_, text)| text.contains("mustard")));
        assert!(set.pages.iter().any(|(_, text)| text.contains("lentils")));
    }

    #[test]
    fn a_line_typed_since_the_last_sync_survives_the_other_copy() {
        let vault = VaultKey::generate().unwrap();
        let ferry = "Harbor ledger\nferry = 40\n";
        let bread = "Harbor ledger\nferry = 40\nbread $2\n";
        let set = sample_set(vec![("harbor".into(), ferry.into())]);
        let planned = plan(&vault, 1, &set, &[], 1).unwrap();
        let mut slots = planned.slots.clone();
        for slot in &mut slots {
            slot.server_rev = 1;
        }
        let mut lib = omacus_doc::Library::new();
        lib.reconcile("harbor", ferry, None);
        lib.note_written("harbor", ferry);
        lib.reconcile("harbor", bread, None);
        lib.note_written("harbor", bread);
        let incoming = vec![Incoming {
            id: planned.objects[0].id,
            revision: 1,
            body: planned.objects[0].body.clone(),
        }];
        let opened = assemble(&planned.manifest, &incoming, &slots, &lib.save()).unwrap();
        let round = PageSet::from_bytes(&opened.packed).unwrap();
        let text = round
            .pages
            .iter()
            .find(|(id, _)| id == "harbor")
            .unwrap()
            .1
            .clone();
        assert!(text.contains("bread $2"), "{text}");
        assert!(text.contains("ferry = 40"), "{text}");
        let shown = omacus_doc::Library::load(&round.doc)
            .unwrap()
            .snapshot("harbor")
            .unwrap();
        assert!(shown.contains("bread $2"), "{shown}");
    }

    #[test]
    fn an_unchanged_page_keeps_a_line_deleted_on_the_other_phone() {
        let vault = VaultKey::generate().unwrap();
        let ferry = "Harbor ledger\nferry = 40\n";
        let with_bread = "Harbor ledger\nferry = 40\nbread $2\n";
        let set = sample_set(vec![("harbor".into(), with_bread.into())]);
        let first = plan(&vault, 1, &set, &[], 1).unwrap();
        let mut slots = first.slots.clone();
        for slot in &mut slots {
            slot.server_rev = 1;
        }
        let edited = sample_set(vec![("harbor".into(), ferry.into())]);
        let second = plan(&vault, 1, &edited, &slots, 2).unwrap();
        let mut lib = omacus_doc::Library::new();
        lib.reconcile("harbor", with_bread, None);
        lib.note_written("harbor", with_bread);
        let incoming = vec![Incoming {
            id: second.objects[0].id,
            revision: 2,
            body: second.objects[0].body.clone(),
        }];
        let opened = assemble(&second.manifest, &incoming, &slots, &lib.save()).unwrap();
        let round = PageSet::from_bytes(&opened.packed).unwrap();
        let text = &round.pages.iter().find(|(id, _)| id == "harbor").unwrap().1;
        assert!(!text.contains("bread"), "{text}");
        assert!(text.contains("ferry = 40"), "{text}");
    }

    #[test]
    fn an_unsent_line_merges_with_a_line_from_the_other_phone() {
        let vault = VaultKey::generate().unwrap();
        let ferry = "Harbor ledger\nferry = 40\n";
        let bread = "Harbor ledger\nferry = 40\nbread $2\n";
        let cake = "Harbor ledger\nferry = 40\ncake $5\n";
        let set = sample_set(vec![("harbor".into(), ferry.into())]);
        let first = plan(&vault, 1, &set, &[], 1).unwrap();
        let mut slots = first.slots.clone();
        for slot in &mut slots {
            slot.server_rev = 1;
        }
        let edited = sample_set(vec![("harbor".into(), bread.into())]);
        let second = plan(&vault, 1, &edited, &slots, 2).unwrap();
        let mut lib = omacus_doc::Library::new();
        lib.reconcile("harbor", ferry, None);
        lib.note_written("harbor", ferry);
        lib.reconcile("harbor", cake, None);
        lib.note_written("harbor", cake);
        let incoming = vec![Incoming {
            id: second.objects[0].id,
            revision: 2,
            body: second.objects[0].body.clone(),
        }];
        let opened = assemble(&second.manifest, &incoming, &slots, &lib.save()).unwrap();
        let text = &PageSet::from_bytes(&opened.packed)
            .unwrap()
            .pages
            .into_iter()
            .find(|(id, _)| id == "harbor")
            .unwrap()
            .1;
        assert!(text.contains("bread $2"), "{text}");
        assert!(text.contains("cake $5"), "{text}");
        assert_eq!(text.matches("ferry = 40").count(), 1, "{text}");
    }

    #[test]
    fn two_edits_of_one_page_merge() {
        let base = page_document(b"", "pantry", "oats\nlentils\n").unwrap();
        let ours = page_document(&base, "pantry", "oats\nlentils\nmustard\n").unwrap();
        let theirs = page_document(&base, "pantry", "oats\nlentils\nsesame\n").unwrap();
        let (_doc, text) = merge_page(
            "pantry",
            &ours,
            "oats\nlentils\nmustard\n",
            &theirs,
            "oats\nlentils\nsesame\n",
        )
        .unwrap();
        assert!(text.contains("mustard"), "{text}");
        assert!(text.contains("sesame"), "{text}");
        assert!(text.contains("oats"), "{text}");
    }

    #[test]
    fn five_hundred_pages_seal_a_small_manifest() {
        let vault = VaultKey::generate().unwrap();
        let marker = "lentils-oats-mustard-sesame-pantry";
        let mut bulky = String::with_capacity(80_000);
        bulky.push_str(marker);
        bulky.push('\n');
        let mut state = 0x1234_5678_9abc_def0u64;
        while bulky.len() < 80_000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            bulky.push(char::from(b'a' + (state % 26) as u8));
        }
        let mut pages = Vec::with_capacity(500);
        for n in 0..500 {
            let id = format!("page-{n:04}");
            let text = if n == 250 {
                bulky.clone()
            } else {
                format!("oats {n}\n")
            };
            pages.push((id, text));
        }
        let set = sample_set(pages);
        let started = std::time::Instant::now();
        let first = plan(&vault, 1, &set, &[], 1).unwrap();
        let seal_ms = started.elapsed().as_millis();
        assert_eq!(first.objects.len(), 500);
        assert!(
            first.manifest.len() < 256 * 1024,
            "manifest {}",
            first.manifest.len()
        );
        assert!(first.sealed_manifest.len() < 256 * 1024);
        let large = first
            .objects
            .iter()
            .max_by_key(|object| object.body.len())
            .unwrap();
        let small = first
            .objects
            .iter()
            .min_by_key(|object| object.body.len())
            .unwrap();
        assert!(
            large.body.len() > small.body.len(),
            "small {} large {}",
            small.body.len(),
            large.body.len()
        );
        let large_bucket = large.body.len() - 24 - 16;
        assert!(
            BUCKETS.contains(&large_bucket),
            "large body {} is not a bucket",
            large.body.len()
        );
        let blob = {
            let mut all = first.sealed_manifest.clone();
            for object in &first.objects {
                all.extend_from_slice(&object.body);
                all.extend_from_slice(&object.id);
            }
            all
        };
        assert!(
            !blob
                .windows(marker.len())
                .any(|window| window == marker.as_bytes())
        );
        assert!(
            !blob
                .windows(b"page-0250".len())
                .any(|window| window == b"page-0250")
        );
        let mut slots = first.slots.clone();
        for slot in &mut slots {
            slot.server_rev = 4;
        }
        let mut edited = set.clone();
        edited.pages[3].1 = "sesame\n".into();
        let second = plan(&vault, 1, &edited, &slots, 1).unwrap();
        assert_eq!(second.objects.len(), 1, "one changed page");
        let rotated = VaultKey::generate().unwrap();
        let third = plan(&rotated, 1, &edited, &second.slots, 2).unwrap();
        assert_eq!(third.objects.len(), 0, "rotation uploads no page");
        let incoming: Vec<Incoming> = first
            .objects
            .iter()
            .enumerate()
            .map(|(index, object)| Incoming {
                id: object.id,
                revision: 4,
                body: if index == 3 {
                    second.objects[0].body.clone()
                } else {
                    object.body.clone()
                },
            })
            .collect();
        let open_started = std::time::Instant::now();
        let opened = assemble(&third.manifest, &incoming, &[], &[]).unwrap();
        let open_ms = open_started.elapsed().as_millis();
        let round = PageSet::from_bytes(&opened.packed).unwrap();
        let warm_started = std::time::Instant::now();
        let _warm = assemble(&third.manifest, &incoming, &[], &round.doc).unwrap();
        let warm_ms = warm_started.elapsed().as_millis();
        assert_eq!(round.pages.len(), 500);
        let ids: HashSet<_> = round.pages.iter().map(|(id, _)| id.clone()).collect();
        assert_eq!(ids.len(), 500, "no duplicate pages");
        assert!(round.pages.iter().any(|(_, text)| text.contains(marker)));
        assert!(round.pages.iter().any(|(_, text)| text == "sesame\n"));
        let line = format!(
            "pages=500 manifest={} sealed_manifest={} objects={} small_body={} large_body={} changed_upload=1 rotate_upload=0 seal_ms={seal_ms} open_ms={open_ms} warm_ms={warm_ms}",
            first.manifest.len(),
            first.sealed_manifest.len(),
            first.objects.len(),
            small.body.len(),
            large.body.len(),
        );
        eprintln!("{line}");
        let _ = std::fs::write("/tmp/omacus-sync-500.txt", &line);
    }

    #[test]
    fn a_missing_page_is_kept_and_a_tombstone_is_a_delete() {
        use crate::sync_keys::Tombstone;
        let vault = VaultKey::generate().unwrap();
        let both = sample_set(vec![
            ("pantry".into(), "oats\n".into()),
            ("kept".into(), "ferry = 40\n".into()),
        ]);
        let first = plan(&vault, 1, &both, &[], 1).unwrap();
        assert!(first.drop_ids.is_empty());
        let mut only_kept = sample_set(vec![("kept".into(), "ferry = 40\n".into())]);
        let omitted = plan(&vault, 1, &only_kept, &first.slots, 2).unwrap();
        assert!(omitted.drop_ids.is_empty(), "a missing page was dropped");
        only_kept.meta.tombstones.push(Tombstone {
            id: "pantry".into(),
            rev: 1,
        });
        let deleted = plan(&vault, 1, &only_kept, &first.slots, 3).unwrap();
        let pantry = first
            .slots
            .iter()
            .find(|slot| slot.page_id == "pantry")
            .unwrap();
        assert_eq!(deleted.drop_ids, vec![pantry.object_id]);
        let round = PageSet::from_bytes(&only_kept.to_bytes().unwrap()).unwrap();
        assert_eq!(round.meta.tombstones[0].id, "pantry");
        let old = PageSet::from_bytes(&both.to_bytes().unwrap()).unwrap();
        assert!(
            old.meta.tombstones.is_empty(),
            "an older copy gained a delete"
        );
        only_kept.meta.tombstones.push(Tombstone {
            id: "kept".into(),
            rev: 2,
        });
        let two = plan(&vault, 1, &only_kept, &first.slots, 4).unwrap();
        assert_eq!(two.drop_ids.len(), 2, "two signed deletes were held back");
        let trio = sample_set(vec![
            ("pantry".into(), "oats\n".into()),
            ("kept".into(), "ferry = 40\n".into()),
            ("harbor".into(), "cake\n".into()),
        ]);
        let sealed = plan(&vault, 1, &trio, &[], 5).unwrap();
        let mut wiped = trio;
        wiped.pages.clear();
        wiped.meta.tombstones = vec![
            Tombstone {
                id: "pantry".into(),
                rev: 1,
            },
            Tombstone {
                id: "kept".into(),
                rev: 2,
            },
            Tombstone {
                id: "harbor".into(),
                rev: 3,
            },
        ];
        let mass = plan(&vault, 1, &wiped, &sealed.slots, 6).unwrap();
        assert!(
            mass.drop_ids.is_empty(),
            "more than two deletes dropped an object"
        );
    }

    #[test]
    fn a_joiner_keeps_the_sidebar_trash_and_settings() {
        let vault = VaultKey::generate().unwrap();
        let mut lib = omacus_doc::Library::new();
        assert!(lib.note_vault(
            r#"{"sidebar":[{"id":"pantry","kind":"folder","name":"Pantry","parent":"","index":0},{"id":"oats","kind":"page","parent":"pantry","index":0},{"id":"ferry","kind":"page","parent":"pantry","index":1},{"id":"bread","kind":"page","parent":"pantry","index":2},{"id":"cake","kind":"page","parent":"pantry","index":3}],"handles":[{"id":"ferry","handle":"pier"}],"trash":[{"id":"kept","text":"old loaf"}],"prefs":{"theme":"dusk","hideAnswers":true,"lockMinutes":15}}"#
        ));
        lib.reconcile("oats", "# Oats\nbowl 2\n", None);
        lib.reconcile("ferry", "# Ferry\n", None);
        lib.reconcile("bread", "# Bread\n", None);
        lib.reconcile("cake", "# Cake\n", None);
        let set = PageSet {
            doc: lib.save(),
            pages: vec![
                ("oats".into(), "# Oats\nbowl 2\n".into()),
                ("ferry".into(), "# Ferry\n".into()),
                ("bread".into(), "# Bread\n".into()),
                ("cake".into(), "# Cake\n".into()),
            ],
            meta: SyncMeta::default(),
        };
        let planned = plan(&vault, 1, &set, &[], 1).unwrap();
        assert!(
            planned.slots.iter().any(|slot| slot.page_id.is_empty()),
            "the shared sidebar was not sealed"
        );
        let incoming: Vec<Incoming> = planned
            .objects
            .iter()
            .map(|object| Incoming {
                id: object.id,
                revision: 1,
                body: object.body.clone(),
            })
            .collect();
        let want = manifest_wants(&planned.manifest, &[]).unwrap();
        assert!(
            planned
                .slots
                .iter()
                .filter(|slot| slot.page_id.is_empty())
                .all(|slot| want.contains(&slot.object_id)),
            "a joiner was not told to fetch the sidebar"
        );
        let opened = assemble(&planned.manifest, &incoming, &[], &[]).unwrap();
        let round = PageSet::from_bytes(&opened.packed).unwrap();
        let present = omacus_doc::Library::load(&round.doc)
            .unwrap()
            .vault_present()
            .unwrap_or_default();
        assert!(present.contains("Pantry"), "{present}");
        assert!(present.contains("kept"), "{present}");
        assert!(present.contains("pier"), "{present}");
        assert!(present.contains("dusk"), "{present}");
        assert!(round.pages.iter().any(|(id, _)| id == "bread"));
        let mut slots = opened.slots.clone();
        for slot in &mut slots {
            slot.server_rev = 1;
        }
        let again = plan(&vault, 1, &set, &slots, 2).unwrap();
        assert!(
            again.objects.is_empty(),
            "an unchanged sidebar was uploaded again"
        );
    }
}
