//! The private sync test, for a browser pointed at a test server.
//!
//! Nothing here is the app at app.omacus.com. The page keeps the words. The
//! server receives a sealed record and public device bytes.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use omacus_engine::sync_keys::SyncKeyError;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

pub mod desk;
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
pub fn start_new() -> String {
    call(|| {
        let desk = Desk::start_new()?;
        let words = desk.words().ok_or(SyncKeyError::NotAKey)?;
        let qr = desk.sheet_svg()?;
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
