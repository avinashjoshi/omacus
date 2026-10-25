//! A mailbox request is signed by a device on the newest list.
//!
//! This crate does not open a page. It only checks the device list and the
//! signature that are already in the sealed record.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

pub const WINDOW_MS: u64 = 5 * 60 * 1000;
pub const RATE_WINDOW_MS: u64 = 60_000;
pub const ACCOUNT_LIMIT: u32 = 240;
pub const IP_LIMIT: u32 = 800;
pub const GUESS_LIMIT: u32 = 10;
pub const JOIN_LIMIT: u32 = 10;
pub const READ_ACCOUNT_LIMIT: u32 = 180;
pub const READ_IP_LIMIT: u32 = 500;
const MAX_NONCES: usize = 64;
const RECOVERY_ID: [u8; 16] = *b"recovery-signer\0";

pub struct Auth {
    pub device: [u8; 16],
    pub time: u64,
    pub nonce: [u8; 16],
    pub signature: [u8; 64],
}

#[derive(Clone, PartialEq, Eq)]
struct Dev {
    id: [u8; 16],
    sign: [u8; 32],
    receive: [u8; 32],
}

#[derive(Clone, PartialEq, Eq)]
struct List {
    version: u64,
    prev: [u8; 32],
    signer: [u8; 16],
    devices: Vec<Dev>,
}

struct Opened {
    account: [u8; 32],
    lists: Vec<List>,
    signed: bool,
}

pub fn read_auth(headers: &BTreeMap<String, String>) -> Option<Auth> {
    let device = unhex_exact(headers.get("x-omacus-device")?)?;
    let time = headers.get("x-omacus-time")?.parse().ok()?;
    let nonce = unhex_exact(headers.get("x-omacus-nonce")?)?;
    let signature = unhex_exact(headers.get("x-omacus-signature")?)?;
    Some(Auth {
        device,
        time,
        nonce,
        signature,
    })
}

pub fn read_proof(headers: &BTreeMap<String, String>) -> Option<Vec<u8>> {
    let text = headers.get("x-omacus-proof")?;
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, text.trim()).ok()
}

/// `Some(true)` when the recovery signing key signed this request.
pub fn accept_request(
    method: &str,
    path: &str,
    body: &[u8],
    auth: &Auth,
    now: u64,
    record: &[u8],
    nonces: &mut BTreeMap<String, u64>,
) -> Option<bool> {
    if auth.time.saturating_add(WINDOW_MS) < now || now.saturating_add(WINDOW_MS) < auth.time {
        return None;
    }
    let opened = open_record(record)?;
    let recovery = auth.device == RECOVERY_ID;
    let key = verify_key(&opened, &auth.device)?;
    let message = auth_message(method, path, auth.time, &auth.nonce, body);
    let signature = Signature::from_slice(&auth.signature).ok()?;
    let verifying = VerifyingKey::from_bytes(&key).ok()?;
    verifying.verify(&message, &signature).ok()?;
    let nonce = hex(&auth.nonce);
    prune_nonces(nonces, now);
    if nonces.contains_key(&nonce) {
        return None;
    }
    if nonces.len() >= MAX_NONCES {
        return None;
    }
    nonces.insert(nonce, auth.time);
    Some(recovery)
}

pub fn put_allowed(stored: &[u8], incoming: &[u8], recovery: bool) -> bool {
    let Some(next) = open_record(incoming) else {
        return false;
    };
    if !next.signed {
        return false;
    }
    if recovery {
        return true;
    }
    let Some(previous) = open_record(stored) else {
        return false;
    };
    let shared = next.lists.len().min(previous.lists.len());
    if next.lists[..shared] != previous.lists[..shared] {
        return false;
    }
    next.lists.len() >= previous.lists.len()
}

fn verify_key(opened: &Opened, device: &[u8; 16]) -> Option<[u8; 32]> {
    let latest = opened.lists.last()?;
    if *device == RECOVERY_ID {
        let on_list = latest
            .devices
            .iter()
            .find(|row| row.id == RECOVERY_ID)
            .map(|row| row.sign);
        if let Some(sign) = on_list {
            if sign != opened.account {
                return None;
            }
            return Some(sign);
        }
        return Some(opened.account);
    }
    latest
        .devices
        .iter()
        .find(|row| row.id == *device)
        .map(|row| row.sign)
}

fn auth_message(method: &str, path: &str, time_ms: u64, nonce: &[u8; 16], body: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"omacus-auth/1\n");
    message.extend_from_slice(method.to_ascii_uppercase().as_bytes());
    message.push(b'\n');
    message.extend_from_slice(path.as_bytes());
    message.push(b'\n');
    message.extend_from_slice(&time_ms.to_be_bytes());
    message.extend_from_slice(nonce);
    message.extend_from_slice(&Sha256::digest(body));
    message
}

fn prune_nonces(nonces: &mut BTreeMap<String, u64>, now: u64) {
    nonces.retain(|_, time| time.saturating_add(WINDOW_MS) >= now);
}

struct Cursor<'a> {
    rest: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn new(rest: &'a [u8]) -> Self {
        Self { rest }
    }

    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        if self.rest.len() < len {
            return None;
        }
        let (head, tail) = self.rest.split_at(len);
        self.rest = tail;
        Some(head)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Some(out)
    }

    fn u16(&mut self) -> Option<usize> {
        let bytes = self.array::<2>()?;
        Some(usize::from(u16::from_be_bytes(bytes)))
    }

    fn u32(&mut self) -> Option<usize> {
        let bytes = self.array::<4>()?;
        usize::try_from(u32::from_be_bytes(bytes)).ok()
    }

    fn u64(&mut self) -> Option<u64> {
        let bytes = self.array::<8>()?;
        Some(u64::from_be_bytes(bytes))
    }

    fn skip(&mut self, len: usize) -> Option<()> {
        self.take(len).map(|_| ())
    }
}

fn open_record(bytes: &[u8]) -> Option<Opened> {
    if bytes.len() < 14 {
        return None;
    }
    let magic = &bytes[..14];
    if magic != b"OMACUS-SYNC/1\n" && magic != b"OMACUS-SYNC/2\n" {
        return None;
    }
    let mut cursor = Cursor::new(&bytes[14..]);
    let vault_len = usize::from(*cursor.take(1)?.first()?);
    if cursor.take(vault_len)? != b"personal" {
        return None;
    }
    cursor.u64()?;
    let account = cursor.array()?;
    cursor.skip(16)?;
    let chain_len = cursor.u32()?;
    let chain = cursor.take(chain_len)?;
    let phrase_len = cursor.u32()?;
    cursor.skip(phrase_len)?;
    let wraps = cursor.u16()?;
    for _ in 0..wraps {
        let wrap_len = cursor.u32()?;
        cursor.skip(wrap_len)?;
    }
    let page_len = cursor.u32()?;
    cursor.skip(page_len)?;
    let (signed, signer, signature) = if cursor.rest.is_empty() {
        (false, None, None)
    } else if cursor.rest.len() == 80 {
        let signer = cursor.array()?;
        let signature = cursor.array()?;
        (true, Some(signer), Some(signature))
    } else {
        return None;
    };
    let lists = open_chain(chain)?;
    if signed {
        let signer = signer?;
        let signature = signature?;
        let prefix = &bytes[..bytes.len() - 80];
        if !record_signature_ok(&lists, account, signer, signature, prefix) {
            return None;
        }
    }
    Some(Opened {
        account,
        lists,
        signed,
    })
}

fn record_signature_ok(
    lists: &[List],
    account: [u8; 32],
    signer: [u8; 16],
    signature: [u8; 64],
    prefix: &[u8],
) -> bool {
    let mut message = Vec::with_capacity(16 + prefix.len());
    message.extend_from_slice(b"omacus-record/1");
    message.extend_from_slice(prefix);
    let Some(latest) = lists.last() else {
        return false;
    };
    let key = if signer == RECOVERY_ID {
        match latest.devices.iter().find(|row| row.id == RECOVERY_ID) {
            Some(row) if row.sign == account => account,
            Some(_) => return false,
            None => account,
        }
    } else {
        match latest.devices.iter().find(|row| row.id == signer) {
            Some(row) => row.sign,
            None => return false,
        }
    };
    let Ok(verifying) = VerifyingKey::from_bytes(&key) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&signature) else {
        return false;
    };
    verifying.verify(&message, &signature).is_ok()
}

fn open_chain(bytes: &[u8]) -> Option<Vec<List>> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(16)? != b"OMACUS-DEVICES/1" {
        return None;
    }
    let count = cursor.u16()?;
    if count == 0 || count > 64 {
        return None;
    }
    let mut lists = Vec::with_capacity(count);
    for place in 0..count {
        let version = cursor.u64()?;
        let prev = cursor.array::<32>()?;
        let signer = cursor.array::<16>()?;
        let signature = cursor.array::<64>()?;
        let devices_len = cursor.u16()?;
        if devices_len == 0 || devices_len > 64 {
            return None;
        }
        let mut devices = Vec::with_capacity(devices_len);
        for _ in 0..devices_len {
            devices.push(Dev {
                id: cursor.array()?,
                sign: cursor.array()?,
                receive: cursor.array()?,
            });
        }
        if version != place as u64 + 1 || !unique_ids(&devices) {
            return None;
        }
        if place == 0 {
            if prev != [0u8; 32] || devices.len() != 1 || signer != devices[0].id {
                return None;
            }
        } else {
            let previous = &lists[place - 1];
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&Sha256::digest(list_message(previous)));
            if prev != hash || !changed_by_one(&previous.devices, &devices) {
                return None;
            }
            if !previous.devices.iter().any(|device| device.id == signer) {
                return None;
            }
        }
        let sign = devices.iter().find(|device| device.id == signer)?.sign;
        let list = List {
            version,
            prev,
            signer,
            devices,
        };
        let verifying = VerifyingKey::from_bytes(&sign).ok()?;
        let signature = Signature::from_slice(&signature).ok()?;
        verifying.verify(&list_message(&list), &signature).ok()?;
        lists.push(list);
    }
    if !cursor.rest.is_empty() {
        return None;
    }
    Some(lists)
}

fn list_message(list: &List) -> Vec<u8> {
    list_message_raw(list.version, &list.prev, &list.signer, &list.devices)
}

fn list_message_raw(version: u64, prev: &[u8; 32], signer: &[u8; 16], devices: &[Dev]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"omacus-devices/1");
    message.extend_from_slice(&version.to_be_bytes());
    message.extend_from_slice(prev);
    message.extend_from_slice(signer);
    message.extend_from_slice(&(devices.len() as u16).to_be_bytes());
    for device in devices {
        message.extend_from_slice(&device.id);
        message.extend_from_slice(&device.sign);
        message.extend_from_slice(&device.receive);
    }
    message
}

fn unique_ids(devices: &[Dev]) -> bool {
    for (index, device) in devices.iter().enumerate() {
        if devices[..index]
            .iter()
            .any(|earlier| earlier.id == device.id)
        {
            return false;
        }
    }
    true
}

fn changed_by_one(previous: &[Dev], next: &[Dev]) -> bool {
    appended_one(previous, next) || removed_one(previous, next)
}

fn appended_one(previous: &[Dev], next: &[Dev]) -> bool {
    next.len() == previous.len() + 1
        && next[..previous.len()] == previous[..]
        && !previous
            .iter()
            .any(|device| device.id == next[previous.len()].id)
}

fn removed_one(previous: &[Dev], next: &[Dev]) -> bool {
    if previous.len() < 2 || next.len() + 1 != previous.len() {
        return false;
    }
    let mut skipped = false;
    let mut index = 0;
    for device in previous {
        if index < next.len() && next[index] == *device {
            index += 1;
        } else if !skipped {
            skipped = true;
        } else {
            return false;
        }
    }
    skipped && index == next.len()
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn unhex_exact<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (slot, chunk) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        let text = std::str::from_utf8(chunk).ok()?;
        *slot = u8::from_str_radix(text, 16).ok()?;
    }
    Some(out)
}
