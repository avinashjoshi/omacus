//! In-memory mailbox. Records are kept as the bytes that arrived.
//! Nothing here is read as a page.

use std::collections::{BTreeMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde_json::{Value, json};

use crate::adapter::{FileKv, Kv};
use crate::invite::{self, INVITE_MS};

const MAX_RECORD: usize = 256 * 1024;
const MAX_MANIFEST: usize = 1024 * 1024;
const MAX_OBJECT: usize = 4 * 1024 * 1024 + 64;
const MAX_OBJECTS: usize = 100_000;
const MAX_CHAIN: usize = 64 * 1024;
const MAX_WRAP: usize = 1024;
const MAX_REQUESTS: usize = 8;
const RECORD_MAGIC: &[u8] = b"OMACUS-SYNC/1\n";
const RECORD_MAGIC_V2: &[u8] = b"OMACUS-SYNC/2\n";
const CHAIN_MAGIC: &[u8] = b"OMACUS-DEVICES/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    Unknown,
    Conflict,
    Revision,
    Full,
    Unreadable,
    BadInvite,
    BadAccess,
    Limited,
}

impl StoreError {
    pub fn status(self) -> u16 {
        match self {
            StoreError::Unknown => 404,
            StoreError::Conflict | StoreError::Revision => 409,
            StoreError::Full | StoreError::Limited => 429,
            StoreError::Unreadable | StoreError::BadInvite | StoreError::BadAccess => 400,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            StoreError::Unknown => "No pages for that yet.",
            StoreError::Conflict => "That copy is already here.",
            StoreError::Revision => "That copy is out of date. Try again.",
            StoreError::Full => "Sync is full.",
            StoreError::Unreadable => "That copy could not be read.",
            StoreError::BadInvite => "That code didn't work. Check it on the other device.",
            StoreError::BadAccess => "That invite code didn't work. Check it and try again.",
            StoreError::Limited => "Try again in a moment.",
        }
    }
}

#[derive(Clone)]
struct Request {
    device: Vec<u8>,
    name: String,
    state: State,
    chain: Vec<u8>,
    wrap: Vec<u8>,
    proof: Vec<u8>,
    /// When the request arrived. A decision can be read once. After that,
    /// or once the invite window has passed, the request is removed.
    created: u64,
}

struct Invite {
    hash: [u8; 32],
    expires: u64,
    used: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Pending,
    Approved,
    Rejected,
}

struct StoredObject {
    revision: u64,
    body: Vec<u8>,
}

struct Hit {
    start: u64,
    count: u32,
}

struct Mailbox {
    revision: u64,
    record: Vec<u8>,
    requests: BTreeMap<String, Request>,
    invite: Option<Invite>,
    objects: BTreeMap<String, StoredObject>,
    nonces: BTreeMap<String, u64>,
}

pub struct ObjectWrite {
    pub id: String,
    pub base: u64,
    pub body: Vec<u8>,
}

pub struct ObjectHead {
    pub id: String,
    pub revision: u64,
    pub bytes: usize,
}

pub struct Store {
    mailboxes: BTreeMap<String, Mailbox>,
    /// Live invite hash → account. A join reads this map. It does not walk the accounts.
    invites: BTreeMap<[u8; 32], String>,
    access_left: BTreeMap<[u8; 32], u8>,
    access_secret: Vec<u8>,
    now_ms: Option<u64>,
    backend: Option<Box<dyn Kv>>,
    account_hits: BTreeMap<String, Hit>,
    ip_hits: BTreeMap<String, Hit>,
    guess_hits: BTreeMap<String, Hit>,
    join_hits: BTreeMap<String, Hit>,
    read_account_hits: BTreeMap<String, Hit>,
    read_ip_hits: BTreeMap<String, Hit>,
}

#[derive(Clone)]
pub struct Pending {
    pub id: String,
    pub device: Vec<u8>,
    pub name: String,
    pub proof: Vec<u8>,
}

#[derive(Clone)]
pub struct RequestView {
    pub state: &'static str,
    pub device: Vec<u8>,
    pub name: String,
    pub chain: Vec<u8>,
    pub wrap: Vec<u8>,
}

impl Store {
    pub fn new() -> Self {
        Self {
            mailboxes: BTreeMap::new(),
            invites: BTreeMap::new(),
            access_left: BTreeMap::new(),
            access_secret: Vec::new(),
            now_ms: None,
            backend: None,
            account_hits: BTreeMap::new(),
            ip_hits: BTreeMap::new(),
            guess_hits: BTreeMap::new(),
            join_hits: BTreeMap::new(),
            read_account_hits: BTreeMap::new(),
            read_ip_hits: BTreeMap::new(),
        }
    }

    /// Load whatever the adapter already has, then keep writing through it.
    pub fn with_backend(backend: Box<dyn Kv>) -> Self {
        let (mailboxes, access_left) = load_backend(backend.as_ref());
        let invites = index_invites(&mailboxes);
        Self {
            mailboxes,
            invites,
            access_left,
            access_secret: Vec::new(),
            now_ms: None,
            backend: Some(backend),
            account_hits: BTreeMap::new(),
            ip_hits: BTreeMap::new(),
            guess_hits: BTreeMap::new(),
            join_hits: BTreeMap::new(),
            read_account_hits: BTreeMap::new(),
            read_ip_hits: BTreeMap::new(),
        }
    }

    pub fn open_dir(dir: &std::path::Path) -> Self {
        Self::with_backend(Box::new(FileKv::open(dir)))
    }

    pub fn into_backend(self) -> Option<Box<dyn Kv>> {
        self.backend
    }

    /// The secret checks invite codes. It is not part of the stored mailbox.
    pub fn set_access_secret(&mut self, secret: &[u8]) {
        if secret.len() >= crate::access::MIN_SECRET {
            self.access_secret = secret.to_vec();
        }
    }

    /// Tests pin the clock so a code's lifetime does not depend on the wall time.
    pub fn set_now(&mut self, ms: u64) {
        self.now_ms = Some(ms);
    }

    pub fn charge_ip(&mut self, ip: &str) -> Result<(), StoreError> {
        let now = self.now();
        self.ip_hits
            .retain(|_, hit| now.saturating_sub(hit.start) < crate::auth::RATE_WINDOW_MS);
        bump(&mut self.ip_hits, ip, now, crate::auth::IP_LIMIT)
    }

    pub fn charge_account(&mut self, account: &str) -> Result<(), StoreError> {
        let account = account_id(account)?;
        let now = self.now();
        self.account_hits
            .retain(|_, hit| now.saturating_sub(hit.start) < crate::auth::RATE_WINDOW_MS);
        bump(
            &mut self.account_hits,
            &account,
            now,
            crate::auth::ACCOUNT_LIMIT,
        )
    }

    pub fn charge_guess(&mut self, ip: &str) -> Result<(), StoreError> {
        let now = self.now();
        fresh(&mut self.guess_hits, now);
        bump(&mut self.guess_hits, ip, now, crate::auth::GUESS_LIMIT)
    }

    pub fn charge_join(&mut self, ip: &str) -> Result<(), StoreError> {
        let now = self.now();
        fresh(&mut self.join_hits, now);
        bump(&mut self.join_hits, ip, now, crate::auth::JOIN_LIMIT)
    }

    pub fn charge_read(&mut self, ip: &str, account: &str) -> Result<(), StoreError> {
        let account = account_id(account)?;
        let now = self.now();
        fresh(&mut self.read_ip_hits, now);
        fresh(&mut self.read_account_hits, now);
        bump(&mut self.read_ip_hits, ip, now, crate::auth::READ_IP_LIMIT)?;
        bump(
            &mut self.read_account_hits,
            &account,
            now,
            crate::auth::READ_ACCOUNT_LIMIT,
        )
    }

    /// Check a device signature and remember its nonce. A failure is the same
    /// as a missing mailbox.
    pub fn authorize(
        &mut self,
        account: &str,
        method: &str,
        path: &str,
        body: &[u8],
        headers: &BTreeMap<String, String>,
    ) -> Result<bool, StoreError> {
        let account = account_id(account)?;
        let auth = crate::auth::read_auth(headers).ok_or(StoreError::Unknown)?;
        let now = self.now();
        let recovery = {
            let mailbox = self
                .mailboxes
                .get_mut(&account)
                .ok_or(StoreError::Unknown)?;
            crate::auth::accept_request(
                method,
                path,
                body,
                &auth,
                now,
                &mailbox.record,
                &mut mailbox.nonces,
            )
            .ok_or(StoreError::Unknown)?
        };
        self.checkpoint();
        Ok(recovery)
    }

    pub fn allow_put(
        &self,
        account: &str,
        record: &[u8],
        recovery: bool,
    ) -> Result<(), StoreError> {
        let account = account_id(account)?;
        let mailbox = self.mailboxes.get(&account).ok_or(StoreError::Unknown)?;
        if crate::auth::put_allowed(&mailbox.record, record, recovery) {
            Ok(())
        } else {
            Err(StoreError::Unreadable)
        }
    }

    /// The joining device may read only its own request, and only with the
    /// same check value it used to create that request.
    pub fn request_with_proof(
        &mut self,
        account: &str,
        id: &str,
        proof: &[u8],
    ) -> Result<RequestView, StoreError> {
        let account_key = account_id(account)?;
        self.drop_expired(&account_key);
        let mailbox = self
            .mailboxes
            .get(&account_key)
            .ok_or(StoreError::Unknown)?;
        let request = mailbox.requests.get(id).ok_or(StoreError::Unknown)?;
        if !same_bytes(&request.proof, proof) {
            return Err(StoreError::Unknown);
        }
        self.request(&account_key, id)
    }

    fn now(&self) -> u64 {
        self.now_ms.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or(0)
        })
    }

    /// Every byte this server is keeping. A test scans it for keys and pages.
    pub fn stored_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (account, mailbox) in &self.mailboxes {
            out.extend_from_slice(account.as_bytes());
            out.extend_from_slice(&mailbox.record);
            if let Some(invite) = &mailbox.invite {
                out.extend_from_slice(&invite.hash);
            }
            for (id, request) in &mailbox.requests {
                out.extend_from_slice(id.as_bytes());
                out.extend_from_slice(&request.device);
                out.extend_from_slice(request.name.as_bytes());
                out.extend_from_slice(&request.chain);
                out.extend_from_slice(&request.wrap);
                out.extend_from_slice(&request.proof);
            }
            for (id, object) in &mailbox.objects {
                out.extend_from_slice(id.as_bytes());
                out.extend_from_slice(&object.body);
            }
        }
        for (hash, left) in &self.access_left {
            out.extend_from_slice(hash);
            out.push(*left);
        }
        out
    }

    /// A code can be looked at without using it up.
    pub fn peek_access(&self, code: &str) -> Result<(), StoreError> {
        let _ = self.access_room(code)?;
        Ok(())
    }

    /// Creating a set of pages spends one use. The code itself is not kept.
    pub fn take_access(&mut self, code: &str) -> Result<(), StoreError> {
        let (hash, left) = self.access_room(code)?;
        self.access_left.insert(hash, left - 1);
        Ok(())
    }

    fn access_room(&self, code: &str) -> Result<([u8; 32], u8), StoreError> {
        let opened = crate::access::open(&self.access_secret, code).ok_or(StoreError::BadAccess)?;
        let left = self
            .access_left
            .get(&opened.hash)
            .copied()
            .unwrap_or(opened.uses);
        if left == 0 {
            return Err(StoreError::BadAccess);
        }
        Ok((opened.hash, left))
    }

    /// Checks that a new mailbox would be accepted, without writing it.
    pub fn reserve(&self, account: &str, record: &[u8]) -> Result<(), StoreError> {
        let account = account_id(account)?;
        sealed_record(record)?;
        if self.mailboxes.contains_key(&account) {
            return Err(StoreError::Conflict);
        }
        Ok(())
    }

    pub fn create(&mut self, account: &str, record: Vec<u8>) -> Result<u64, StoreError> {
        let account = account_id(account)?;
        sealed_record(&record)?;
        if self.mailboxes.contains_key(&account) {
            return Err(StoreError::Conflict);
        }
        self.mailboxes.insert(
            account.clone(),
            Mailbox {
                revision: 1,
                record,
                requests: BTreeMap::new(),
                invite: None,
                objects: BTreeMap::new(),
                nonces: BTreeMap::new(),
            },
        );
        self.checkpoint();
        Ok(1)
    }

    pub fn create_with(
        &mut self,
        account: &str,
        record: Vec<u8>,
        writes: Vec<ObjectWrite>,
    ) -> Result<(u64, Vec<(String, u64)>), StoreError> {
        let account = account_id(account)?;
        sealed_record(&record)?;
        if self.mailboxes.contains_key(&account) {
            return Err(StoreError::Conflict);
        }
        let mut objects = BTreeMap::new();
        let mut written = Vec::new();
        let mut seen = HashSet::new();
        for write in writes {
            let id = object_id(&write.id)?;
            if write.base != 0
                || !seen.insert(id.clone())
                || write.body.len() < 40
                || write.body.len() > MAX_OBJECT
            {
                return Err(StoreError::Unreadable);
            }
            objects.insert(
                id.clone(),
                StoredObject {
                    revision: 1,
                    body: write.body,
                },
            );
            written.push((id, 1));
        }
        if objects.len() > MAX_OBJECTS {
            return Err(StoreError::Full);
        }
        self.mailboxes.insert(
            account,
            Mailbox {
                revision: 1,
                record,
                requests: BTreeMap::new(),
                invite: None,
                objects,
                nonces: BTreeMap::new(),
            },
        );
        self.checkpoint();
        Ok((1, written))
    }

    pub fn get(&mut self, account: &str) -> Result<(u64, Vec<u8>), StoreError> {
        let account = account_id(account)?;
        self.drop_expired(&account);
        let mailbox = self.mailboxes.get(&account).ok_or(StoreError::Unknown)?;
        Ok((mailbox.revision, mailbox.record.clone()))
    }

    pub fn put(
        &mut self,
        account: &str,
        revision: u64,
        record: Vec<u8>,
    ) -> Result<u64, StoreError> {
        let account = account_id(account)?;
        sealed_record(&record)?;
        self.drop_expired(&account);
        let mailbox = self
            .mailboxes
            .get_mut(&account)
            .ok_or(StoreError::Unknown)?;
        if mailbox.revision != revision {
            return Err(StoreError::Revision);
        }
        mailbox.revision += 1;
        mailbox.record = record;
        let revision = mailbox.revision;
        self.checkpoint();
        Ok(revision)
    }

    pub fn object_index(&mut self, account: &str) -> Result<Vec<ObjectHead>, StoreError> {
        let account = account_id(account)?;
        let mailbox = self.mailboxes.get(&account).ok_or(StoreError::Unknown)?;
        let mut rows: Vec<_> = mailbox
            .objects
            .iter()
            .map(|(id, object)| ObjectHead {
                id: id.clone(),
                revision: object.revision,
                bytes: object.body.len(),
            })
            .collect();
        rows.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(rows)
    }

    pub fn read_objects(
        &self,
        account: &str,
        ids: &[String],
    ) -> Result<Vec<(String, u64, Vec<u8>)>, StoreError> {
        let account = account_id(account)?;
        if ids.len() > 2000 {
            return Err(StoreError::Unreadable);
        }
        let mailbox = self.mailboxes.get(&account).ok_or(StoreError::Unknown)?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let id = object_id(id)?;
            let object = mailbox.objects.get(&id).ok_or(StoreError::Unknown)?;
            out.push((id, object.revision, object.body.clone()));
        }
        Ok(out)
    }

    /// Compare-and-swap the manifest and the named objects together.
    /// A stale base writes nothing.
    pub fn put_objects(
        &mut self,
        account: &str,
        revision: u64,
        record: Vec<u8>,
        writes: Vec<ObjectWrite>,
        drop_ids: &[String],
    ) -> Result<(u64, Vec<(String, u64)>), StoreError> {
        let account = account_id(account)?;
        sealed_record(&record)?;
        if writes.len() > 2000 || drop_ids.len() > 2000 {
            return Err(StoreError::Unreadable);
        }
        self.drop_expired(&account);
        let mailbox = self.mailboxes.get(&account).ok_or(StoreError::Unknown)?;
        if mailbox.revision != revision {
            return Err(StoreError::Revision);
        }
        let mut seen = HashSet::new();
        let mut planned = Vec::with_capacity(writes.len());
        for write in &writes {
            let id = object_id(&write.id)?;
            if !seen.insert(id.clone()) || write.body.len() < 40 || write.body.len() > MAX_OBJECT {
                return Err(StoreError::Unreadable);
            }
            match mailbox.objects.get(&id) {
                Some(_) if write.base == 0 => return Err(StoreError::Revision),
                Some(current) if current.revision != write.base => {
                    return Err(StoreError::Revision);
                }
                Some(current) => planned.push((id, current.revision + 1, write.body.clone())),
                None if write.base == 0 => planned.push((id, 1, write.body.clone())),
                None => return Err(StoreError::Revision),
            }
        }
        let mut dropping = Vec::new();
        for id in drop_ids {
            let id = object_id(id)?;
            if seen.contains(&id) {
                return Err(StoreError::Unreadable);
            }
            dropping.push(id);
        }
        let added = planned
            .iter()
            .filter(|(id, _, _)| !mailbox.objects.contains_key(id))
            .count();
        if mailbox.objects.len() + added > MAX_OBJECTS {
            return Err(StoreError::Full);
        }
        let mailbox = self
            .mailboxes
            .get_mut(&account)
            .ok_or(StoreError::Unknown)?;
        mailbox.revision += 1;
        mailbox.record = record;
        let mut written = Vec::with_capacity(planned.len());
        for (id, rev, body) in planned {
            mailbox.objects.insert(
                id.clone(),
                StoredObject {
                    revision: rev,
                    body,
                },
            );
            written.push((id, rev));
        }
        for id in dropping {
            mailbox.objects.remove(&id);
        }
        let revision = mailbox.revision;
        self.checkpoint();
        Ok((revision, written))
    }

    /// Remember a hash for a code this account is showing. The code itself stays on the device.
    pub fn open_invite(&mut self, account: &str, hash_hex: &str) -> Result<(), StoreError> {
        let account = account_id(account)?;
        let hash = invite::parse_hash(hash_hex).ok_or(StoreError::Unreadable)?;
        let expires = self.now().saturating_add(INVITE_MS);
        let previous = self
            .mailboxes
            .get(&account)
            .ok_or(StoreError::Unknown)?
            .invite
            .as_ref()
            .map(|invite| invite.hash);
        if let Some(previous) = previous
            && self.invites.get(&previous) == Some(&account)
        {
            self.invites.remove(&previous);
        }
        self.mailboxes
            .get_mut(&account)
            .ok_or(StoreError::Unknown)?
            .invite = Some(Invite {
            hash,
            expires,
            used: false,
        });
        self.invites.insert(hash, account);
        Ok(())
    }

    /// Add a device only when the code matches one live invite and the check value matches.
    pub fn join(
        &mut self,
        code: &str,
        device: Vec<u8>,
        name: &str,
        given: &[u8],
    ) -> Result<(String, String), StoreError> {
        if device.len() != 80 || given.len() != 32 {
            return Err(StoreError::Unreadable);
        }
        let secret = invite::decode_code(code).ok_or(StoreError::BadInvite)?;
        let hash = invite::hash_secret(&secret);
        let now = self.now();
        let account = self
            .invites
            .get(&hash)
            .cloned()
            .ok_or(StoreError::BadInvite)?;
        if !invite::proof_matches(&secret, &device, given) {
            return Err(StoreError::BadInvite);
        }
        self.drop_expired(&account);
        let matches = self
            .mailboxes
            .get(&account)
            .and_then(|mailbox| mailbox.invite.as_ref())
            .is_some_and(|invite| !invite.used && invite.expires > now && invite.hash == hash);
        if !matches {
            let drop_index = match self
                .mailboxes
                .get(&account)
                .and_then(|mailbox| mailbox.invite.as_ref())
            {
                Some(invite) => invite.used || invite.hash != hash,
                None => true,
            };
            if drop_index {
                self.invites.remove(&hash);
            }
            return Err(StoreError::BadInvite);
        }
        let id = fresh_id()?;
        {
            let mailbox = self
                .mailboxes
                .get_mut(&account)
                .ok_or(StoreError::BadInvite)?;
            let pending = mailbox
                .requests
                .values()
                .filter(|request| request.state == State::Pending)
                .count();
            if pending >= MAX_REQUESTS {
                return Err(StoreError::Full);
            }
            if let Some(invite) = mailbox.invite.as_mut() {
                invite.used = true;
            }
            mailbox.requests.insert(
                id.clone(),
                Request {
                    device,
                    name: clean_name(name),
                    state: State::Pending,
                    chain: Vec::new(),
                    wrap: Vec::new(),
                    proof: given.to_vec(),
                    created: now,
                },
            );
        }
        self.invites.remove(&hash);
        Ok((account, id))
    }

    pub fn pending(&mut self, account: &str) -> Result<Vec<Pending>, StoreError> {
        let account = account_id(account)?;
        self.drop_expired(&account);
        let mailbox = self.mailboxes.get(&account).ok_or(StoreError::Unknown)?;
        Ok(mailbox
            .requests
            .iter()
            .filter(|(_, request)| request.state == State::Pending)
            .map(|(id, request)| Pending {
                id: id.clone(),
                device: request.device.clone(),
                name: request.name.clone(),
                proof: request.proof.clone(),
            })
            .collect())
    }

    /// Pending requests, and approvals the other phone has not read yet.
    /// An approval stays here until that phone reads it, or until it expires.
    /// The third value is when the request was created, so a phone can stop
    /// waiting at the same moment the request expires.
    pub fn open_requests(
        &mut self,
        account: &str,
    ) -> Result<Vec<(Pending, &'static str, u64)>, StoreError> {
        let account = account_id(account)?;
        self.drop_expired(&account);
        let mailbox = self.mailboxes.get(&account).ok_or(StoreError::Unknown)?;
        Ok(mailbox
            .requests
            .iter()
            .filter_map(|(id, request)| {
                let state = match request.state {
                    State::Pending => "pending",
                    State::Approved => "approved",
                    State::Rejected => return None,
                };
                Some((
                    Pending {
                        id: id.clone(),
                        device: request.device.clone(),
                        name: request.name.clone(),
                        proof: request.proof.clone(),
                    },
                    state,
                    request.created,
                ))
            })
            .collect())
    }

    pub fn approve(
        &mut self,
        account: &str,
        id: &str,
        chain: Vec<u8>,
        wrap: Vec<u8>,
    ) -> Result<(), StoreError> {
        if chain.len() > MAX_CHAIN
            || !chain.starts_with(CHAIN_MAGIC)
            || wrap.is_empty()
            || wrap.len() > MAX_WRAP
        {
            return Err(StoreError::Unreadable);
        }
        let request = self.request_mut(account, id)?;
        if request.state != State::Pending {
            return Err(StoreError::Conflict);
        }
        request.state = State::Approved;
        request.chain = chain;
        request.wrap = wrap;
        Ok(())
    }

    pub fn reject(&mut self, account: &str, id: &str) -> Result<(), StoreError> {
        let request = self.request_mut(account, id)?;
        if request.state != State::Pending {
            return Err(StoreError::Conflict);
        }
        request.state = State::Rejected;
        Ok(())
    }

    /// The waiting phone reads a decision once. That read removes it.
    pub fn request(&mut self, account: &str, id: &str) -> Result<RequestView, StoreError> {
        let account = account_id(account)?;
        self.drop_expired(&account);
        let mailbox = self
            .mailboxes
            .get_mut(&account)
            .ok_or(StoreError::Unknown)?;
        let request = mailbox.requests.get(id).ok_or(StoreError::Unknown)?;
        let decided = request.state != State::Pending;
        let view = RequestView {
            state: match request.state {
                State::Pending => "pending",
                State::Approved => "approved",
                State::Rejected => "rejected",
            },
            device: request.device.clone(),
            name: request.name.clone(),
            chain: request.chain.clone(),
            wrap: request.wrap.clone(),
        };
        if decided {
            mailbox.requests.remove(id);
        }
        Ok(view)
    }

    fn drop_expired(&mut self, account: &str) {
        let now = self.now();
        let Some(mailbox) = self.mailboxes.get_mut(account) else {
            return;
        };
        mailbox
            .requests
            .retain(|_, request| request.created.saturating_add(INVITE_MS) > now);
    }

    fn request_mut(&mut self, account: &str, id: &str) -> Result<&mut Request, StoreError> {
        let account = account_id(account)?;
        self.drop_expired(&account);
        let mailbox = self
            .mailboxes
            .get_mut(&account)
            .ok_or(StoreError::Unknown)?;
        mailbox.requests.get_mut(id).ok_or(StoreError::Unknown)
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

fn fresh(hits: &mut BTreeMap<String, Hit>, now: u64) {
    hits.retain(|_, hit| now.saturating_sub(hit.start) < crate::auth::RATE_WINDOW_MS);
}

fn bump(
    hits: &mut BTreeMap<String, Hit>,
    key: &str,
    now: u64,
    limit: u32,
) -> Result<(), StoreError> {
    let hit = hits.entry(key.to_string()).or_insert(Hit {
        start: now,
        count: 0,
    });
    if now.saturating_sub(hit.start) >= crate::auth::RATE_WINDOW_MS {
        hit.start = now;
        hit.count = 0;
    }
    if hit.count >= limit {
        return Err(StoreError::Limited);
    }
    hit.count += 1;
    Ok(())
}

fn same_bytes(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (one, other) in left.iter().zip(right) {
        diff |= one ^ other;
    }
    diff == 0
}

fn account_id(account: &str) -> Result<String, StoreError> {
    let account = account.trim().to_ascii_lowercase();
    if account.len() == 64 && account.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(account)
    } else {
        Err(StoreError::Unreadable)
    }
}

fn sealed_record(bytes: &[u8]) -> Result<(), StoreError> {
    let v1 = bytes.starts_with(RECORD_MAGIC) && bytes.len() <= MAX_RECORD;
    let v2 = bytes.starts_with(RECORD_MAGIC_V2) && bytes.len() <= MAX_MANIFEST;
    if bytes.len() > 14 && (v1 || v2) {
        Ok(())
    } else {
        Err(StoreError::Unreadable)
    }
}

fn object_id(id: &str) -> Result<String, StoreError> {
    let id = id.trim().to_ascii_lowercase();
    if id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(id)
    } else {
        Err(StoreError::Unreadable)
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn from_b64(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .ok()
}

impl Store {
    fn checkpoint(&mut self) {
        let Some(mut backend) = self.backend.take() else {
            return;
        };
        save_backend(&mut *backend, &self.mailboxes, &self.access_left);
        self.backend = Some(backend);
    }
}

fn save_backend(
    kv: &mut dyn Kv,
    mailboxes: &BTreeMap<String, Mailbox>,
    access: &BTreeMap<[u8; 32], u8>,
) {
    let mut live_boxes = HashSet::new();
    let mut live_objects = HashSet::new();
    for (account, mailbox) in mailboxes {
        let key = format!("box/{account}");
        live_boxes.insert(key.clone());
        kv.put(&key, mailbox_bytes(mailbox));
        for (id, object) in &mailbox.objects {
            let obj_key = format!("obj/{account}/{id}");
            live_objects.insert(obj_key.clone());
            let mut bytes = object.revision.to_be_bytes().to_vec();
            bytes.extend_from_slice(&object.body);
            kv.put(&obj_key, bytes);
        }
    }
    for key in kv.list_prefix("box/") {
        if !live_boxes.contains(&key) {
            kv.delete(&key);
        }
    }
    for key in kv.list_prefix("obj/") {
        if !live_objects.contains(&key) {
            kv.delete(&key);
        }
    }
    let rows: Vec<_> = access
        .iter()
        .map(|(hash, left)| json!({"hash": hex_bytes(hash), "left": left}))
        .collect();
    kv.put("access", serde_json::to_vec(&rows).unwrap_or_default());
}

fn index_invites(mailboxes: &BTreeMap<String, Mailbox>) -> BTreeMap<[u8; 32], String> {
    let mut invites = BTreeMap::new();
    for (account, mailbox) in mailboxes {
        if let Some(invite) = &mailbox.invite
            && !invite.used
        {
            invites.insert(invite.hash, account.clone());
        }
    }
    invites
}

fn load_backend(kv: &dyn Kv) -> (BTreeMap<String, Mailbox>, BTreeMap<[u8; 32], u8>) {
    let mut mailboxes = BTreeMap::new();
    for key in kv.list_prefix("box/") {
        let Some(account) = key.strip_prefix("box/") else {
            continue;
        };
        if account.contains('/') {
            continue;
        }
        let Some(bytes) = kv.get(&key) else {
            continue;
        };
        if let Some(mailbox) = mailbox_from(&bytes) {
            mailboxes.insert(account.to_string(), mailbox);
        }
    }
    for key in kv.list_prefix("obj/") {
        let Some(rest) = key.strip_prefix("obj/") else {
            continue;
        };
        let Some((account, id)) = rest.split_once('/') else {
            continue;
        };
        if id.contains('/') || object_id(id).is_err() {
            continue;
        }
        let Some(mailbox) = mailboxes.get_mut(account) else {
            continue;
        };
        let Some(bytes) = kv.get(&key) else {
            continue;
        };
        if bytes.len() < 8 {
            continue;
        }
        let revision = u64::from_be_bytes(bytes[..8].try_into().unwrap_or([0; 8]));
        mailbox.objects.insert(
            id.to_string(),
            StoredObject {
                revision,
                body: bytes[8..].to_vec(),
            },
        );
    }
    let mut access = BTreeMap::new();
    if let Some(bytes) = kv.get("access")
        && let Ok(rows) = serde_json::from_slice::<Vec<Value>>(&bytes)
    {
        for row in rows {
            let Some(hash) = row.get("hash").and_then(Value::as_str) else {
                continue;
            };
            let Some(left) = row.get("left").and_then(Value::as_u64) else {
                continue;
            };
            if let Ok(hash) = hex_array(hash) {
                access.insert(hash, left as u8);
            }
        }
    }
    (mailboxes, access)
}

fn mailbox_bytes(mailbox: &Mailbox) -> Vec<u8> {
    let requests: Vec<_> = mailbox
        .requests
        .iter()
        .map(|(id, request)| {
            json!({
                "id": id,
                "device": b64(&request.device),
                "name": request.name,
                "state": match request.state {
                    State::Pending => "pending",
                    State::Approved => "approved",
                    State::Rejected => "rejected",
                },
                "chain": b64(&request.chain),
                "wrap": b64(&request.wrap),
                "proof": b64(&request.proof),
                "created": request.created,
            })
        })
        .collect();
    let invite = mailbox.invite.as_ref().map(|invite| {
        json!({
            "hash": hex_bytes(&invite.hash),
            "expires": invite.expires,
            "used": invite.used,
        })
    });
    serde_json::to_vec(&json!({
        "revision": mailbox.revision,
        "record": b64(&mailbox.record),
        "requests": requests,
        "invite": invite,
        "nonces": mailbox.nonces.iter().map(|(nonce, time)| json!({"nonce": nonce, "time": time})).collect::<Vec<_>>(),
    }))
    .unwrap_or_default()
}

fn mailbox_from(bytes: &[u8]) -> Option<Mailbox> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let revision = value.get("revision")?.as_u64()?;
    let record = from_b64(value.get("record")?.as_str()?)?;
    let mut requests = BTreeMap::new();
    for row in value.get("requests")?.as_array()? {
        let id = row.get("id")?.as_str()?.to_string();
        let state = match row.get("state").and_then(Value::as_str)? {
            "pending" => State::Pending,
            "approved" => State::Approved,
            "rejected" => State::Rejected,
            _ => return None,
        };
        requests.insert(
            id,
            Request {
                device: from_b64(row.get("device")?.as_str()?)?,
                name: row.get("name")?.as_str()?.to_string(),
                state,
                chain: from_b64(row.get("chain")?.as_str()?)?,
                wrap: from_b64(row.get("wrap")?.as_str()?)?,
                proof: from_b64(row.get("proof")?.as_str()?)?,
                created: row.get("created")?.as_u64()?,
            },
        );
    }
    let invite = match value.get("invite") {
        None | Some(Value::Null) => None,
        Some(row) => Some(Invite {
            hash: hex_array(row.get("hash")?.as_str()?).ok()?,
            expires: row.get("expires")?.as_u64()?,
            used: row.get("used")?.as_bool()?,
        }),
    };
    let mut nonces = BTreeMap::new();
    if let Some(rows) = value.get("nonces").and_then(Value::as_array) {
        for row in rows {
            let Some(nonce) = row.get("nonce").and_then(Value::as_str) else {
                continue;
            };
            let Some(time) = row.get("time").and_then(Value::as_u64) else {
                continue;
            };
            nonces.insert(nonce.to_string(), time);
        }
    }
    Some(Mailbox {
        revision,
        record,
        requests,
        invite,
        objects: BTreeMap::new(),
        nonces,
    })
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_array(text: &str) -> Result<[u8; 32], ()> {
    if text.len() != 64 {
        return Err(());
    }
    let mut out = [0u8; 32];
    for (slot, chunk) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        let text = std::str::from_utf8(chunk).map_err(|_| ())?;
        *slot = u8::from_str_radix(text, 16).map_err(|_| ())?;
    }
    Ok(out)
}

pub fn clean_name(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == ' ' || ch == '-' {
            out.push(ch);
        }
        if out.len() >= 24 {
            break;
        }
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        "New device".to_string()
    } else {
        out
    }
}

fn fresh_id() -> Result<String, StoreError> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|_| StoreError::Unreadable)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invite::{self, INVITE_MS};

    fn account() -> String {
        "ab".repeat(32)
    }

    fn record() -> Vec<u8> {
        b"OMACUS-SYNC/1\nsealed".to_vec()
    }

    fn arrive(
        store: &mut Store,
        account: &str,
        when: u64,
        secret: [u8; 5],
        device: &[u8],
        name: &str,
    ) -> String {
        store.set_now(when);
        let code = invite::encode_code(&secret);
        store
            .open_invite(account, &invite::hash_hex(&secret))
            .unwrap();
        let mac = invite::proof(&secret, device);
        let (found, id) = store.join(&code, device.to_vec(), name, &mac).unwrap();
        assert_eq!(found, account);
        id
    }

    #[test]
    fn the_33rd_account_is_kept_and_a_join_follows_the_invite_index() {
        let mut store = Store::new();
        for n in 0..33u32 {
            store.create(&format!("{n:064x}"), record()).unwrap();
        }
        let first = format!("{:064x}", 0);
        let second = format!("{:064x}", 32);
        let secret = [4u8, 5, 6, 7, 8];
        store.set_now(50);
        store
            .open_invite(&second, &invite::hash_hex(&secret))
            .unwrap();
        let hash = invite::hash_secret(&secret);
        // The live invite sits on the 33rd account. Pointing the index elsewhere
        // must miss. A walk of every account would still find it.
        store.invites.insert(hash, first);
        let device = vec![3u8; 80];
        let code = invite::encode_code(&secret);
        let mac = invite::proof(&secret, &device);
        assert_eq!(
            store
                .join(&code, device.clone(), "Kitchen", &mac)
                .unwrap_err(),
            StoreError::BadInvite
        );
        store.invites.insert(hash, second.clone());
        let (found, _) = store.join(&code, device, "Kitchen", &mac).unwrap();
        assert_eq!(found, second);
        assert!(!store.invites.contains_key(&hash));
    }

    #[test]
    fn a_paste_of_words_is_not_stored() {
        let mut store = Store::new();
        let words =
            b"river apple garden window yellow mountain coffee pencil summer bridge orange candle";
        assert_eq!(
            store.create(&account(), words.to_vec()).unwrap_err(),
            StoreError::Unreadable
        );
        assert!(store.stored_bytes().is_empty());
        store.create(&account(), record()).unwrap();
        assert!(
            !store
                .stored_bytes()
                .windows(words.len())
                .any(|window| window == words)
        );
    }

    #[test]
    fn a_revision_must_match_and_a_request_can_be_refused() {
        let mut store = Store::new();
        let account = account();
        assert_eq!(store.create(&account, record()).unwrap(), 1);
        assert_eq!(
            store.put(&account, 0, record()).unwrap_err(),
            StoreError::Revision
        );
        assert_eq!(store.put(&account, 1, record()).unwrap(), 2);
        let secret = [1u8, 2, 3, 4, 5];
        let code = invite::encode_code(&secret);
        store
            .open_invite(&account, &invite::hash_hex(&secret))
            .unwrap();
        let device = vec![7u8; 80];
        let mac = invite::proof(&secret, &device);
        let (_, id) = store.join(&code, device, "This phone", &mac).unwrap();
        store.reject(&account, &id).unwrap();
        assert_eq!(store.request(&account, &id).unwrap().state, "rejected");
        assert!(store.pending(&account).unwrap().is_empty());
    }

    #[test]
    fn a_join_without_a_live_code_is_dropped_and_the_code_is_not_kept() {
        let mut store = Store::new();
        let account = account();
        store.create(&account, record()).unwrap();
        let device = vec![7u8; 80];
        let secret = [9u8, 8, 7, 6, 5];
        let code = invite::encode_code(&secret);
        let mac = invite::proof(&secret, &device);
        assert_eq!(
            store
                .join(&code, device.clone(), "Phone", &mac)
                .unwrap_err(),
            StoreError::BadInvite
        );
        assert!(store.pending(&account).unwrap().is_empty());

        store.set_now(1_000);
        store
            .open_invite(&account, &invite::hash_hex(&secret))
            .unwrap();
        assert_eq!(
            store
                .join("0000-0001", device.clone(), "Phone", &mac)
                .unwrap_err(),
            StoreError::BadInvite
        );
        assert!(store.pending(&account).unwrap().is_empty());
        let kept = store.stored_bytes();
        assert!(!kept.windows(secret.len()).any(|window| window == secret));
        assert!(
            !kept
                .windows(code.len())
                .any(|window| window == code.as_bytes())
        );

        store.set_now(1_000 + INVITE_MS);
        assert_eq!(
            store
                .join(&code, device.clone(), "Phone", &mac)
                .unwrap_err(),
            StoreError::BadInvite
        );

        store.set_now(1_000);
        let (found, id) = store.join(&code, device.clone(), "Phone", &mac).unwrap();
        assert_eq!(found, account);
        assert_eq!(store.pending(&account).unwrap()[0].id, id);
        assert_eq!(
            store.join(&code, device, "Phone", &mac).unwrap_err(),
            StoreError::BadInvite
        );
    }

    #[test]
    fn a_join_request_is_dropped_once_it_is_approved_declined_or_expired() {
        let mut store = Store::new();
        let account = account();
        store.create(&account, record()).unwrap();
        let chain = b"OMACUS-DEVICES/1porch".to_vec();
        let wrap = b"porch-wrap".to_vec();

        let gone = |store: &Store, needle: &[u8]| {
            let kept = store.stored_bytes();
            !kept.windows(needle.len()).any(|window| window == needle)
        };

        let device = vec![3u8; 80];
        let id = arrive(
            &mut store,
            &account,
            10_000,
            [3, 1, 4, 1, 5],
            &device,
            "Porch lamp",
        );
        store.reject(&account, &id).unwrap();
        assert_eq!(store.request(&account, &id).unwrap().state, "rejected");
        assert!(matches!(
            store.request(&account, &id),
            Err(StoreError::Unknown)
        ));
        assert!(gone(&store, &device));
        assert!(gone(&store, b"Porch lamp"));

        let device = vec![4u8; 80];
        let id = arrive(
            &mut store,
            &account,
            20_000,
            [2, 7, 1, 8, 2],
            &device,
            "Cabin shelf",
        );
        store
            .approve(&account, &id, chain.clone(), wrap.clone())
            .unwrap();
        let view = store.request(&account, &id).unwrap();
        assert_eq!(view.state, "approved");
        assert_eq!(view.chain, chain);
        assert_eq!(view.wrap, wrap);
        assert!(matches!(
            store.request(&account, &id),
            Err(StoreError::Unknown)
        ));
        assert!(gone(&store, &chain));
        assert!(gone(&store, &wrap));
        assert!(gone(&store, &device));
        assert!(gone(&store, b"Cabin shelf"));

        let device = vec![5u8; 80];
        let id = arrive(
            &mut store,
            &account,
            30_000,
            [8, 6, 7, 5, 3],
            &device,
            "Shed light",
        );
        assert_eq!(store.pending(&account).unwrap()[0].id, id);
        store.set_now(30_000 + INVITE_MS);
        store.get(&account).unwrap();
        assert!(store.pending(&account).unwrap().is_empty());
        assert!(matches!(
            store.request(&account, &id),
            Err(StoreError::Unknown)
        ));
        assert_eq!(
            store
                .approve(&account, &id, chain.clone(), wrap.clone())
                .unwrap_err(),
            StoreError::Unknown
        );
        assert!(gone(&store, &device));
        assert!(gone(&store, b"Shed light"));

        let device = vec![6u8; 80];
        let unread = b"unread-wrap".to_vec();
        let id = arrive(
            &mut store,
            &account,
            40_000,
            [1, 1, 2, 3, 5],
            &device,
            "Hall table",
        );
        store
            .approve(
                &account,
                &id,
                b"OMACUS-DEVICES/1hall".to_vec(),
                unread.clone(),
            )
            .unwrap();
        store.set_now(40_000 + INVITE_MS);
        store.get(&account).unwrap();
        assert!(matches!(
            store.request(&account, &id),
            Err(StoreError::Unknown)
        ));
        assert!(gone(&store, &unread));
        assert!(gone(&store, &device));
        assert!(gone(&store, b"Hall table"));
    }

    fn sample_object(id: &str, body: Vec<u8>) -> ObjectWrite {
        ObjectWrite {
            id: id.to_string(),
            base: 0,
            body,
        }
    }

    fn round_trip(mut store: Store) -> Store {
        let account = "ab".repeat(32);
        let record = b"OMACUS-SYNC/2\nmanifest".to_vec();
        let id = "0123456789abcdef0123456789abcdef";
        let body = vec![9u8; 64];
        let (revision, written) = store
            .create_with(
                &account,
                record.clone(),
                vec![sample_object(id, body.clone())],
            )
            .unwrap();
        assert_eq!(revision, 1);
        assert_eq!(written, vec![(id.to_string(), 1)]);
        let stale = store.put_objects(
            &account,
            1,
            record.clone(),
            vec![ObjectWrite {
                id: id.into(),
                base: 0,
                body: vec![3u8; 64],
            }],
            &[],
        );
        assert_eq!(stale.unwrap_err(), StoreError::Revision);
        assert_eq!(
            store.read_objects(&account, &[id.into()]).unwrap()[0].2,
            body
        );
        let (revision, written) = store
            .put_objects(
                &account,
                1,
                b"OMACUS-SYNC/2\nnext".to_vec(),
                vec![ObjectWrite {
                    id: id.into(),
                    base: 1,
                    body: vec![4u8; 80],
                }],
                &[],
            )
            .unwrap();
        assert_eq!(revision, 2);
        assert_eq!(written[0].1, 2);
        let kept = store.stored_bytes();
        assert!(
            !kept
                .windows(b"page-lentils".len())
                .any(|w| w == b"page-lentils")
        );
        assert!(!kept.windows(b"lentils".len()).any(|w| w == b"lentils"));
        store
    }

    #[test]
    fn objects_survive_a_new_memory_adapter_and_a_new_directory() {
        let store = round_trip(Store::with_backend(Box::new(
            crate::adapter::MemoryKv::new(),
        )));
        let backend = store.into_backend().unwrap();
        let account = "ab".repeat(32);
        let id = "0123456789abcdef0123456789abcdef";
        let again = Store::with_backend(backend);
        let bodies = again.read_objects(&account, &[id.into()]).unwrap();
        assert_eq!(bodies[0].1, 2);
        assert_eq!(bodies[0].2, vec![4u8; 80]);
        let heads = {
            let mut again = again;
            again.object_index(&account).unwrap()
        };
        assert_eq!(heads[0].bytes, 80);

        let dir = std::env::temp_dir().join(format!("omacus-kv-{id}"));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let _ = round_trip(Store::open_dir(&dir));
        }
        let names: Vec<_> = std::fs::read_dir(dir.join("obj").join(&account))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec![id.to_string()]);
        assert!(
            !names
                .iter()
                .any(|name| name.contains("lentils") || name.contains("page"))
        );
        let raw = std::fs::read(dir.join("obj").join(&account).join(id)).unwrap();
        assert!(
            !raw.windows(b"lentils".len())
                .any(|window| window == b"lentils")
        );
        let reopened = Store::open_dir(&dir);
        assert_eq!(
            reopened.read_objects(&account, &[id.into()]).unwrap()[0].2,
            vec![4u8; 80]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_burst_is_limited_without_naming_a_missing_account() {
        let mut store = Store::new();
        store.set_now(1_000);
        let missing = "cd".repeat(32);
        for _ in 0..crate::auth::ACCOUNT_LIMIT {
            store.charge_account(&missing).unwrap();
        }
        assert_eq!(
            store.charge_account(&missing).unwrap_err(),
            StoreError::Limited
        );
        assert_eq!(StoreError::Limited.message(), "Try again in a moment.");
        let other = "ce".repeat(32);
        store.charge_account(&other).unwrap();
        store.set_now(1_000 + crate::auth::RATE_WINDOW_MS);
        store.charge_account(&missing).unwrap();

        let ip = "203.0.113.9";
        for _ in 0..crate::auth::IP_LIMIT {
            store.charge_ip(ip).unwrap();
        }
        assert_eq!(store.charge_ip(ip).unwrap_err(), StoreError::Limited);
        store.charge_ip("203.0.113.10").unwrap();
        store.set_now(1_000 + crate::auth::RATE_WINDOW_MS + crate::auth::RATE_WINDOW_MS);
        store.charge_ip(ip).unwrap();
    }
}
