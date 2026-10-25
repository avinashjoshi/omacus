//! Sync for the browser.
//!
//! The page keeps the words. The server receives a sealed record and public device bytes.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use omacus_engine::sync_keys::SyncKeyError;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

pub mod desk;
mod invite;
use desk::{Desk, b64_decode, b64_encode};

thread_local! {
    static DESKS: RefCell<HashMap<u32, Desk>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u32> = const { Cell::new(1) };
}

fn store(desk: Desk) -> u32 {
    let id = NEXT.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1).max(1));
        id
    });
    DESKS.with(|desks| desks.borrow_mut().insert(id, desk));
    id
}

fn with_desk<T>(id: u32, body: impl FnOnce(&mut Desk) -> T) -> Result<T, SyncKeyError> {
    DESKS.with(|desks| {
        let mut desks = desks.borrow_mut();
        let desk = desks.get_mut(&id).ok_or(SyncKeyError::Corrupt)?;
        Ok(body(desk))
    })
}

fn fail(error: SyncKeyError) -> String {
    json!({ "ok": false, "message": error.message() }).to_string()
}

fn picks(text: &str) -> Result<Vec<(u8, String)>, SyncKeyError> {
    let value: Value = serde_json::from_str(text).map_err(|_| SyncKeyError::Corrupt)?;
    let list = value.as_array().ok_or(SyncKeyError::Corrupt)?;
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let pair = item.as_array().ok_or(SyncKeyError::Corrupt)?;
        if pair.len() != 2 {
            return Err(SyncKeyError::Corrupt);
        }
        let position = pair[0].as_u64().ok_or(SyncKeyError::Corrupt)?;
        let word = pair[1].as_str().ok_or(SyncKeyError::Corrupt)?;
        if position > u64::from(u8::MAX) {
            return Err(SyncKeyError::NotConfirmed);
        }
        out.push((position as u8, word.to_string()));
    }
    Ok(out)
}

fn call(body: impl FnOnce() -> Result<Value, SyncKeyError>) -> String {
    match body() {
        Ok(value) => value.to_string(),
        Err(error) => fail(error),
    }
}

#[wasm_bindgen(js_name = checkPhrase)]
pub fn check_phrase(text: &str) -> String {
    let check = desk::check_phrase(text);
    json!({
        "ok": check.ok,
        "count": check.count,
        "message": check.message,
        "normalized": check.normalized,
    })
    .to_string()
}

#[wasm_bindgen(js_name = startNew)]
pub fn start_new(page: &str) -> String {
    call(|| {
        let desk = Desk::start_new()?;
        let words = desk.words().ok_or(SyncKeyError::NotAKey)?;
        let qr = desk.sheet_svg(page)?;
        let id = store(desk);
        Ok(json!({
            "ok": true,
            "id": id,
            "words": words,
            "qr": qr,
            "asks": desk::CONFIRM_AT,
        }))
    })
}

#[wasm_bindgen(js_name = confirmSaved)]
pub fn confirm_saved(id: u32, picks_json: &str) -> String {
    call(|| {
        let owned = picks(picks_json)?;
        let refs: Vec<(u8, &str)> = owned.iter().map(|(n, word)| (*n, word.as_str())).collect();
        let (record, account) = with_desk(id, |desk| {
            let record = desk.confirm_saved(&refs)?;
            let account = desk.account_hex().ok_or(SyncKeyError::Corrupt)?;
            Ok((record, account))
        })??;
        Ok(json!({
            "ok": true,
            "account": account,
            "record": b64_encode(&record),
        }))
    })
}

#[wasm_bindgen(js_name = makeInvite)]
pub fn make_invite(account: &str, origin: &str, page: &str, secret_b64: &str) -> String {
    let account = account.trim().to_ascii_lowercase();
    let origin = origin.trim();
    if account.len() != 64
        || !account.bytes().all(|byte| byte.is_ascii_hexdigit())
        || origin.is_empty()
        || origin.contains(['\n', '\r'])
        || account.contains(['\n', '\r'])
    {
        return json!({
            "ok": false,
            "message": "That code didn't work. Check it on the other device.",
        })
        .to_string();
    }
    let Ok(secret_bytes) = b64_decode(secret_b64) else {
        return json!({ "ok": false, "message": "Try again in a moment." }).to_string();
    };
    if secret_bytes.len() != 5 {
        return json!({ "ok": false, "message": "Try again in a moment." }).to_string();
    }
    let mut secret = [0u8; 5];
    secret.copy_from_slice(&secret_bytes);
    let code = invite::encode_code(&secret);
    let Ok(payload) = desk::join_link(page, origin, &account, &code) else {
        return json!({ "ok": false, "message": "Try again in a moment." }).to_string();
    };
    let Ok(qr) = desk::qr_svg(&payload, "Join code") else {
        return json!({ "ok": false, "message": "Try again in a moment." }).to_string();
    };
    json!({
        "ok": true,
        "code": code,
        "qr": qr,
        "hash": invite::hash_hex(&secret),
    })
    .to_string()
}

#[wasm_bindgen(js_name = joinProof)]
pub fn join_proof(device_b64: &str, code: &str) -> String {
    let Ok(device) = b64_decode(device_b64) else {
        return json!({
            "ok": false,
            "message": "That code didn't work. Check it on the other device.",
        })
        .to_string();
    };
    let Some(secret) = invite::decode_code(code) else {
        return json!({
            "ok": false,
            "message": "That code didn't work. Check it on the other device.",
        })
        .to_string();
    };
    let mac = invite::proof(&secret, &device);
    json!({ "ok": true, "proof": b64_encode(&mac) }).to_string()
}

#[wasm_bindgen(js_name = checkProof)]
pub fn check_proof(code: &str, device_b64: &str, proof_b64: &str) -> String {
    let Ok(device) = b64_decode(device_b64) else {
        return json!({ "ok": false }).to_string();
    };
    let Ok(given) = b64_decode(proof_b64) else {
        return json!({ "ok": false }).to_string();
    };
    let Some(secret) = invite::decode_code(code) else {
        return json!({ "ok": false }).to_string();
    };
    json!({ "ok": invite::proof_matches(&secret, &device, &given) }).to_string()
}

#[wasm_bindgen(js_name = startJoin)]
pub fn start_join() -> String {
    call(|| {
        let desk = Desk::start_join()?;
        let code = desk.pairing_code();
        let device = b64_encode(&desk.public_device());
        let id = store(desk);
        Ok(json!({
            "ok": true,
            "id": id,
            "code": code,
            "device": device,
        }))
    })
}

#[wasm_bindgen(js_name = confirmLink)]
pub fn confirm_link(id: u32) -> String {
    call(|| {
        with_desk(id, |desk| desk.confirm_link())?;
        Ok(json!({ "ok": true }))
    })
}

#[wasm_bindgen(js_name = accountWords)]
pub fn account_words(id: u32) -> String {
    call(|| {
        let words = with_desk(id, |desk| desk.account_words())??;
        Ok(json!({ "ok": true, "words": words }))
    })
}

#[wasm_bindgen(js_name = recordWords)]
pub fn record_words(record_b64: &str) -> String {
    call(|| {
        let record = b64_decode(record_b64)?;
        let view = omacus_engine::sync_keys::ServerView::parse(&record)?;
        let words = omacus_engine::sync_keys::account_fingerprint(&view.account).join(" ");
        Ok(json!({ "ok": true, "words": words }))
    })
}

#[wasm_bindgen(js_name = codeFor)]
pub fn code_for(device_b64: &str) -> String {
    call(|| {
        let bytes = b64_decode(device_b64)?;
        Ok(json!({
            "ok": true,
            "code": desk::code_for_device(&bytes)?,
        }))
    })
}

#[wasm_bindgen(js_name = approveDevice)]
pub fn approve_device(id: u32, device_b64: &str) -> String {
    call(|| {
        let device = b64_decode(device_b64)?;
        let approval = with_desk(id, |desk| desk.approve_new(&device))??;
        Ok(json!({
            "ok": true,
            "chain": b64_encode(&approval.chain),
            "wrap": b64_encode(&approval.wrap),
            "record": b64_encode(&approval.record),
        }))
    })
}

#[wasm_bindgen(js_name = openApproval)]
pub fn open_approval(id: u32, chain_b64: &str, wrap_b64: &str, record_b64: &str) -> String {
    call(|| {
        let chain = b64_decode(chain_b64)?;
        let wrap = b64_decode(wrap_b64)?;
        let record = b64_decode(record_b64)?;
        let page = with_desk(id, |desk| desk.open_approval(&chain, &wrap, &record))??;
        Ok(json!({ "ok": true, "page": page }))
    })
}

#[wasm_bindgen(js_name = startRecovery)]
pub fn start_recovery(text: &str) -> String {
    call(|| {
        let check = desk::check_phrase(text);
        if !check.ok {
            return Ok(json!({
                "ok": false,
                "message": check.message,
                "count": check.count,
            }));
        }
        let desk = Desk::start_recovery(text)?;
        let words = desk.words().ok_or(SyncKeyError::NotAKey)?;
        let id = store(desk);
        Ok(json!({
            "ok": true,
            "id": id,
            "words": words,
            "asks": desk::CONFIRM_AT,
        }))
    })
}

#[wasm_bindgen(js_name = acceptRecovery)]
pub fn accept_recovery(id: u32) -> String {
    call(|| {
        let account = with_desk(id, |desk| desk.accept_phrase())??;
        Ok(json!({ "ok": true, "account": account }))
    })
}

#[wasm_bindgen(js_name = recoveryAccount)]
pub fn recovery_account(id: u32, picks_json: &str) -> String {
    call(|| {
        let owned = picks(picks_json)?;
        let refs: Vec<(u8, &str)> = owned.iter().map(|(n, word)| (*n, word.as_str())).collect();
        let account = with_desk(id, |desk| desk.recovery_account(&refs))??;
        Ok(json!({ "ok": true, "account": account }))
    })
}

#[wasm_bindgen(js_name = recoveryOpen)]
pub fn recovery_open(id: u32, record_b64: &str) -> String {
    call(|| {
        let record = b64_decode(record_b64)?;
        let page = with_desk(id, |desk| desk.recovery_open(&record))??;
        Ok(json!({ "ok": true, "page": page }))
    })
}

#[wasm_bindgen(js_name = exportLocal)]
pub fn export_local(id: u32) -> String {
    call(|| {
        let file = with_desk(id, |desk| desk.export_local())??;
        Ok(json!({ "ok": true, "file": b64_encode(&file) }))
    })
}

#[wasm_bindgen(js_name = importLocal)]
pub fn import_local(file_b64: &str) -> String {
    call(|| {
        let bytes = b64_decode(file_b64)?;
        let desk = Desk::import_local(&bytes)?;
        let account = desk.account_hex().ok_or(SyncKeyError::Corrupt)?;
        let id = store(desk);
        Ok(json!({ "ok": true, "id": id, "account": account }))
    })
}

#[wasm_bindgen(js_name = sealPages)]
pub fn seal_pages(id: u32, packed_b64: &str) -> String {
    call(|| {
        let packed = b64_decode(packed_b64)?;
        let record = with_desk(id, |desk| desk.seal_pages(&packed))??;
        Ok(json!({ "ok": true, "record": b64_encode(&record) }))
    })
}

#[wasm_bindgen(js_name = absorbRecord)]
pub fn absorb_record(id: u32, record_b64: &str) -> String {
    call(|| {
        let record = b64_decode(record_b64)?;
        let opened = with_desk(id, |desk| desk.absorb_record(&record))??;
        Ok(json!({
            "ok": true,
            "packed": b64_encode(&opened.pages),
            "member": opened.member,
            "replace": opened.replace,
            "republish": opened.republish,
            "combine": opened.combine,
        }))
    })
}

#[wasm_bindgen(js_name = openApprovalPages)]
pub fn open_approval_pages(id: u32, chain_b64: &str, wrap_b64: &str, record_b64: &str) -> String {
    call(|| {
        let chain = b64_decode(chain_b64)?;
        let wrap = b64_decode(wrap_b64)?;
        let record = b64_decode(record_b64)?;
        let packed = with_desk(id, |desk| desk.open_approval_pages(&chain, &wrap, &record))??;
        Ok(json!({ "ok": true, "packed": b64_encode(&packed) }))
    })
}

#[wasm_bindgen(js_name = recoveryOpenPages)]
pub fn recovery_open_pages(id: u32, record_b64: &str) -> String {
    call(|| {
        let record = b64_decode(record_b64)?;
        let opened = with_desk(id, |desk| desk.recovery_open_pages(&record))??;
        Ok(json!({
            "ok": true,
            "packed": b64_encode(&opened.pages),
            "partial": opened.partial,
        }))
    })
}

#[wasm_bindgen(js_name = reestablishPages)]
pub fn reestablish_pages(id: u32, packed_b64: &str) -> String {
    call(|| {
        let packed = b64_decode(packed_b64)?;
        let record = with_desk(id, |desk| desk.reestablish_pages(&packed))??;
        Ok(json!({ "ok": true, "record": b64_encode(&record) }))
    })
}

#[wasm_bindgen(js_name = deviceList)]
pub fn device_list(id: u32) -> String {
    call(|| {
        let (rows, former) = with_desk(id, |desk| {
            let former = desk.formerly_listed();
            desk.listed_devices().map(|rows| (rows, former))
        })??;
        let devices: Vec<_> = rows
            .into_iter()
            .map(|(id, code, mine)| json!({ "id": id, "code": code, "self": mine }))
            .collect();
        Ok(json!({ "ok": true, "devices": devices, "former": former }))
    })
}

#[wasm_bindgen(js_name = signRequest)]
pub fn sign_request(id: u32, method: &str, path: &str, body: &str) -> String {
    call(|| {
        let auth = with_desk(id, |desk| desk.sign_request(method, path, body.as_bytes()))??;
        Ok(json!({
            "ok": true,
            "device": auth.device,
            "time": auth.time,
            "nonce": auth.nonce,
            "signature": auth.signature,
        }))
    })
}

#[wasm_bindgen(js_name = removeDevice)]
pub fn remove_device(id: u32, device_id_hex: &str) -> String {
    call(|| {
        let device_id = desk::decode_hex_id(device_id_hex)?;
        let record = with_desk(id, |desk| desk.remove_listed(&device_id))??;
        Ok(json!({ "ok": true, "record": b64_encode(&record) }))
    })
}

fn page_pairs(text: &str) -> Result<Vec<(String, String)>, SyncKeyError> {
    let value: Value = serde_json::from_str(text).map_err(|_| SyncKeyError::Corrupt)?;
    let list = value.as_array().ok_or(SyncKeyError::Corrupt)?;
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let pair = item.as_array().ok_or(SyncKeyError::Corrupt)?;
        if pair.len() != 2 {
            return Err(SyncKeyError::Corrupt);
        }
        let id = pair[0].as_str().ok_or(SyncKeyError::Corrupt)?;
        let page = pair[1].as_str().ok_or(SyncKeyError::Corrupt)?;
        out.push((id.to_string(), page.to_string()));
    }
    Ok(out)
}

#[wasm_bindgen(js_name = packPages)]
pub fn pack_pages(doc_b64: &str, pages_json: &str, meta_json: &str) -> String {
    call(|| {
        let doc = if doc_b64.is_empty() {
            Vec::new()
        } else {
            b64_decode(doc_b64)?
        };
        let pages = page_pairs(pages_json)?;
        let packed = sync_keys_pack(&doc, &pages, meta_json)?;
        Ok(json!({ "ok": true, "packed": b64_encode(&packed) }))
    })
}

#[wasm_bindgen(js_name = unpackPages)]
pub fn unpack_pages(packed_b64: &str) -> String {
    call(|| {
        let packed = b64_decode(packed_b64)?;
        let set = omacus_engine::sync_keys::PageSet::from_bytes(&packed)?;
        let pages: Vec<_> = set
            .pages
            .iter()
            .map(|(id, text)| json!([id, text]))
            .collect();
        Ok(json!({
            "ok": true,
            "doc": b64_encode(&set.doc),
            "pages": pages,
            "meta": meta_value(&set.meta),
        }))
    })
}

#[wasm_bindgen(js_name = mergePacked)]
pub fn merge_packed(local_b64: &str, remote_b64: &str) -> String {
    call(|| {
        let local = omacus_engine::sync_keys::PageSet::from_bytes(&b64_decode(local_b64)?)?;
        let remote = omacus_engine::sync_keys::PageSet::from_bytes(&b64_decode(remote_b64)?)?;
        let merged = local.merge(remote);
        let packed = merged.to_bytes()?;
        Ok(json!({ "ok": true, "packed": b64_encode(&packed) }))
    })
}

fn sync_keys_pack(
    doc: &[u8],
    pages: &[(String, String)],
    meta_json: &str,
) -> Result<Vec<u8>, SyncKeyError> {
    let mut set = omacus_engine::sync_keys::PageSet {
        doc: doc.to_vec(),
        pages: pages.to_vec(),
        meta: parse_meta(meta_json)?,
    };
    set.retain_vault();
    set.to_bytes()
}

fn parse_meta(text: &str) -> Result<omacus_engine::sync_keys::SyncMeta, SyncKeyError> {
    use omacus_engine::sync_keys::{DeviceLabel, PageMember, SyncMeta};
    if text.trim().is_empty() {
        return Ok(SyncMeta::default());
    }
    let value: Value = serde_json::from_str(text).map_err(|_| SyncKeyError::Corrupt)?;
    let vault_name = value
        .get("vaultName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let name_rev = value.get("nameRev").and_then(Value::as_u64).unwrap_or(0);
    let mut devices = Vec::new();
    if let Some(rows) = value.get("devices").and_then(Value::as_array) {
        for row in rows {
            let id = row.get("id").and_then(Value::as_str).unwrap_or("");
            let name = row
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if id.is_empty() || name.is_empty() {
                continue;
            }
            devices.push(DeviceLabel {
                id: desk::decode_hex_id(id)?,
                rev: row.get("rev").and_then(Value::as_u64).unwrap_or(0),
                name,
            });
        }
    }
    let mut membership = Vec::new();
    if let Some(rows) = value.get("membership").and_then(Value::as_array) {
        for row in rows {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            membership.push(PageMember {
                id,
                rev: row.get("rev").and_then(Value::as_u64).unwrap_or(0),
                in_vault: row.get("inVault").and_then(Value::as_bool).unwrap_or(true),
            });
        }
    }
    Ok(SyncMeta {
        vault_name,
        name_rev,
        devices,
        membership,
    })
}

fn decode_hex_n<const N: usize>(text: &str) -> Result<[u8; N], SyncKeyError> {
    if text.len() != N * 2 {
        return Err(SyncKeyError::Corrupt);
    }
    let mut out = [0u8; N];
    for (slot, chunk) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        let text = std::str::from_utf8(chunk).map_err(|_| SyncKeyError::Corrupt)?;
        *slot = u8::from_str_radix(text, 16).map_err(|_| SyncKeyError::Corrupt)?;
    }
    Ok(out)
}

fn parse_slots(text: &str) -> Result<Vec<omacus_engine::sync_objects::Slot>, SyncKeyError> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(text).map_err(|_| SyncKeyError::Corrupt)?;
    let list = value.as_array().ok_or(SyncKeyError::Corrupt)?;
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let page_id = item
            .get("pageId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let doc = item.get("doc").and_then(Value::as_str).unwrap_or("");
        out.push(omacus_engine::sync_objects::Slot {
            page_id,
            object_id: decode_hex_n(item.get("objectId").and_then(Value::as_str).unwrap_or(""))?,
            hash: decode_hex_n(item.get("hash").and_then(Value::as_str).unwrap_or(""))?,
            content_key: {
                let key = b64_decode(item.get("key").and_then(Value::as_str).unwrap_or(""))?;
                let key: [u8; 32] = key.try_into().map_err(|_| SyncKeyError::Corrupt)?;
                key
            },
            vault_id: decode_hex_n(item.get("vaultId").and_then(Value::as_str).unwrap_or(""))?,
            revision: item.get("revision").and_then(Value::as_u64).unwrap_or(0),
            server_rev: item.get("serverRev").and_then(Value::as_u64).unwrap_or(0),
            doc: if doc.is_empty() {
                Vec::new()
            } else {
                b64_decode(doc)?
            },
        });
    }
    Ok(out)
}

fn slots_json(slots: &[omacus_engine::sync_objects::Slot]) -> String {
    let rows: Vec<_> = slots
        .iter()
        .map(|slot| {
            json!({
                "pageId": slot.page_id,
                "objectId": desk::hex(&slot.object_id),
                "hash": desk::hex(&slot.hash),
                "key": b64_encode(&slot.content_key),
                "vaultId": desk::hex(&slot.vault_id),
                "revision": slot.revision,
                "serverRev": slot.server_rev,
                "doc": b64_encode(&slot.doc),
            })
        })
        .collect();
    Value::Array(rows).to_string()
}

fn parse_incoming(text: &str) -> Result<Vec<omacus_engine::sync_objects::Incoming>, SyncKeyError> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(text).map_err(|_| SyncKeyError::Corrupt)?;
    let list = value.as_array().ok_or(SyncKeyError::Corrupt)?;
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        out.push(omacus_engine::sync_objects::Incoming {
            id: decode_hex_n(item.get("id").and_then(Value::as_str).unwrap_or(""))?,
            revision: item.get("revision").and_then(Value::as_u64).unwrap_or(0),
            body: b64_decode(item.get("body").and_then(Value::as_str).unwrap_or(""))?,
        });
    }
    Ok(out)
}

fn objects_json(objects: &[omacus_engine::sync_objects::Outgoing]) -> Vec<Value> {
    objects
        .iter()
        .map(|object| {
            json!({
                "id": desk::hex(&object.id),
                "body": b64_encode(&object.body),
                "base": object.base,
            })
        })
        .collect()
}

#[wasm_bindgen(js_name = sealLibrary)]
pub fn seal_library(id: u32, packed_b64: &str, slots_json_text: &str) -> String {
    call(|| {
        let packed = b64_decode(packed_b64)?;
        let slots = parse_slots(slots_json_text)?;
        let (planned, record) = with_desk(id, |desk| {
            let planned = desk.seal_library(&packed, &slots)?;
            let record = desk.library_record()?;
            Ok::<_, SyncKeyError>((planned, record))
        })??;
        let drop_ids: Vec<_> = planned.drop_ids.iter().map(|id| desk::hex(id)).collect();
        Ok(json!({
            "ok": true,
            "record": b64_encode(&record),
            "objects": objects_json(&planned.objects),
            "drop": drop_ids,
            "slots": slots_json(&planned.slots),
        }))
    })
}

#[wasm_bindgen(js_name = wantedObjects)]
pub fn wanted_objects(id: u32, record_b64: &str, slots_json_text: &str) -> String {
    call(|| {
        let record = b64_decode(record_b64)?;
        let slots = parse_slots(slots_json_text)?;
        let (legacy, ids) = with_desk(id, |desk| desk.wanted_objects(&record, &slots))??;
        let want: Vec<_> = ids.iter().map(|id| desk::hex(id)).collect();
        Ok(json!({ "ok": true, "legacy": legacy, "want": want }))
    })
}

#[wasm_bindgen(js_name = openLibrary)]
pub fn open_library(
    id: u32,
    record_b64: &str,
    objects_json: &str,
    slots_json_text: &str,
    doc_b64: &str,
) -> String {
    call(|| {
        let record = b64_decode(record_b64)?;
        let incoming = parse_incoming(objects_json)?;
        let slots = parse_slots(slots_json_text)?;
        let doc = if doc_b64.is_empty() {
            Vec::new()
        } else {
            b64_decode(doc_b64)?
        };
        let opened = with_desk(id, |desk| {
            desk.open_library(&record, &incoming, &slots, &doc)
        })??;
        Ok(json!({
            "ok": true,
            "packed": b64_encode(&opened.packed),
            "slots": slots_json(&opened.slots),
            "legacy": opened.legacy,
            "member": opened.member,
            "replace": opened.replace,
            "republish": opened.republish,
            "combine": opened.combine,
        }))
    })
}

#[wasm_bindgen(js_name = recoveryPeek)]
pub fn recovery_peek(id: u32, record_b64: &str) -> String {
    call(|| {
        let record = b64_decode(record_b64)?;
        let peeked = with_desk(id, |desk| desk.recovery_peek(&record))??;
        let want: Vec<_> = peeked.want.iter().map(|id| desk::hex(id)).collect();
        Ok(json!({
            "ok": true,
            "packed": b64_encode(&peeked.packed),
            "partial": peeked.partial,
            "legacy": peeked.legacy,
            "want": want,
            "resign": peeked.resign,
        }))
    })
}

#[wasm_bindgen(js_name = recoveryApply)]
pub fn recovery_apply(id: u32, record_b64: &str, objects_json: &str) -> String {
    call(|| {
        let record = b64_decode(record_b64)?;
        let incoming = parse_incoming(objects_json)?;
        let (packed, slots) = with_desk(id, |desk| desk.recovery_apply(&record, &incoming))??;
        Ok(json!({
            "ok": true,
            "packed": b64_encode(&packed),
            "slots": slots_json(&slots),
        }))
    })
}

#[wasm_bindgen(js_name = approvalPlan)]
pub fn approval_plan(id: u32, chain_b64: &str, wrap_b64: &str, record_b64: &str) -> String {
    call(|| {
        let chain = b64_decode(chain_b64)?;
        let wrap = b64_decode(wrap_b64)?;
        let record = b64_decode(record_b64)?;
        let plan = with_desk(id, |desk| desk.approval_plan(&chain, &wrap, &record))??;
        let want: Vec<_> = plan.want.iter().map(|id| desk::hex(id)).collect();
        Ok(json!({
            "ok": true,
            "packed": b64_encode(&plan.packed),
            "want": want,
            "legacy": plan.legacy,
        }))
    })
}

#[wasm_bindgen(js_name = approvalFinish)]
pub fn approval_finish(id: u32, record_b64: &str, objects_json: &str) -> String {
    call(|| {
        let record = b64_decode(record_b64)?;
        let incoming = parse_incoming(objects_json)?;
        let (packed, slots) = with_desk(id, |desk| desk.approval_finish(&record, &incoming))??;
        Ok(json!({
            "ok": true,
            "packed": b64_encode(&packed),
            "slots": slots_json(&slots),
        }))
    })
}

fn meta_value(meta: &omacus_engine::sync_keys::SyncMeta) -> Value {
    json!({
        "vaultName": meta.vault_name,
        "nameRev": meta.name_rev,
        "devices": meta.devices.iter().map(|device| json!({
            "id": desk::hex(&device.id),
            "rev": device.rev,
            "name": device.name,
        })).collect::<Vec<_>>(),
        "membership": meta.membership.iter().map(|member| json!({
            "id": member.id,
            "rev": member.rev,
            "inVault": member.in_vault,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn the_invite_picture_is_the_fragment_link() {
        let account = "ab".repeat(32);
        let secret = b64_encode(&[7, 7, 7, 7, 7]);
        let page = "https://app.omacus.com";
        let relay = "https://sync.omacus.com";
        let made: serde_json::Value =
            serde_json::from_str(&make_invite(&account, relay, page, &secret)).unwrap();
        assert_eq!(made["ok"], true);
        let code = made["code"].as_str().unwrap();
        let link = desk::join_link(page, relay, &account, code).unwrap();
        let qr = made["qr"].as_str().unwrap();
        assert_eq!(qr, desk::qr_svg(&link, "Join code").unwrap());
        let before = link.split_once('#').unwrap().0;
        assert_eq!(before, "https://app.omacus.com/");
        assert!(!before.contains(code));
        assert!(!qr.contains(code));
        assert!(!qr.contains(&account));
        let test_page = "https://sync-test.omacus.com";
        let test_made: serde_json::Value =
            serde_json::from_str(&make_invite(&account, relay, test_page, &secret)).unwrap();
        let test_link = desk::join_link(test_page, relay, &account, code).unwrap();
        assert_eq!(
            test_made["qr"].as_str().unwrap(),
            desk::qr_svg(&test_link, "Join code").unwrap()
        );
    }
}
