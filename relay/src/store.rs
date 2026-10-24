//! In-memory mailbox. Records are kept as the bytes that arrived.
//! Nothing here is read as a page.

use std::collections::BTreeMap;

const MAX_RECORD: usize = 256 * 1024;
const MAX_CHAIN: usize = 64 * 1024;
const MAX_WRAP: usize = 1024;
const MAX_MAILBOXES: usize = 32;
const MAX_REQUESTS: usize = 8;
const RECORD_MAGIC: &[u8] = b"OMACUS-SYNC/1\n";
const CHAIN_MAGIC: &[u8] = b"OMACUS-DEVICES/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    Unknown,
    Conflict,
    Revision,
    Full,
    Unreadable,
}

impl StoreError {
    pub fn status(self) -> u16 {
        match self {
            StoreError::Unknown => 404,
            StoreError::Conflict | StoreError::Revision => 409,
            StoreError::Full => 429,
            StoreError::Unreadable => 400,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            StoreError::Unknown => "This test server has no pages for that yet.",
            StoreError::Conflict => "This test server already has that copy.",
            StoreError::Revision => "That copy is out of date. Try again.",
            StoreError::Full => "This test server is full.",
            StoreError::Unreadable => "That copy could not be read.",
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Pending,
    Approved,
    Rejected,
}

struct Mailbox {
    revision: u64,
    record: Vec<u8>,
    requests: BTreeMap<String, Request>,
}

pub struct Store {
    mailboxes: BTreeMap<String, Mailbox>,
}

#[derive(Clone)]
pub struct Pending {
    pub id: String,
    pub device: Vec<u8>,
    pub name: String,
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
        }
    }

    /// Every byte this server is keeping. A test scans it for keys and pages.
    pub fn stored_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (account, mailbox) in &self.mailboxes {
            out.extend_from_slice(account.as_bytes());
            out.extend_from_slice(&mailbox.record);
            for (id, request) in &mailbox.requests {
                out.extend_from_slice(id.as_bytes());
                out.extend_from_slice(&request.device);
                out.extend_from_slice(request.name.as_bytes());
                out.extend_from_slice(&request.chain);
                out.extend_from_slice(&request.wrap);
            }
        }
        out
    }

    pub fn list(&self) -> Vec<(String, u64)> {
        self.mailboxes
            .iter()
            .map(|(account, mailbox)| (account.clone(), mailbox.revision))
            .collect()
    }

    pub fn create(&mut self, account: &str, record: Vec<u8>) -> Result<u64, StoreError> {
        let account = account_id(account)?;
        sealed_record(&record)?;
        if self.mailboxes.contains_key(&account) {
            return Err(StoreError::Conflict);
        }
        if self.mailboxes.len() >= MAX_MAILBOXES {
            return Err(StoreError::Full);
        }
        self.mailboxes.insert(
            account,
            Mailbox {
                revision: 1,
                record,
                requests: BTreeMap::new(),
            },
        );
        Ok(1)
    }

    pub fn get(&self, account: &str) -> Result<(u64, Vec<u8>), StoreError> {
        let account = account_id(account)?;
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
        let mailbox = self
            .mailboxes
            .get_mut(&account)
            .ok_or(StoreError::Unknown)?;
        if mailbox.revision != revision {
            return Err(StoreError::Revision);
        }
        mailbox.revision += 1;
        mailbox.record = record;
        Ok(mailbox.revision)
    }

    pub fn ask(
        &mut self,
        account: &str,
        device: Vec<u8>,
        name: &str,
    ) -> Result<String, StoreError> {
        if device.len() != 80 {
            return Err(StoreError::Unreadable);
        }
        let account = account_id(account)?;
        let mailbox = self
            .mailboxes
            .get_mut(&account)
            .ok_or(StoreError::Unknown)?;
        let pending = mailbox
            .requests
            .values()
            .filter(|request| request.state == State::Pending)
            .count();
        if pending >= MAX_REQUESTS {
            return Err(StoreError::Full);
        }
        let id = fresh_id()?;
        mailbox.requests.insert(
            id.clone(),
            Request {
                device,
                name: clean_name(name),
                state: State::Pending,
                chain: Vec::new(),
                wrap: Vec::new(),
            },
        );
        Ok(id)
    }

    pub fn pending(&self, account: &str) -> Result<Vec<Pending>, StoreError> {
        let account = account_id(account)?;
        let mailbox = self.mailboxes.get(&account).ok_or(StoreError::Unknown)?;
        Ok(mailbox
            .requests
            .iter()
            .filter(|(_, request)| request.state == State::Pending)
            .map(|(id, request)| Pending {
                id: id.clone(),
                device: request.device.clone(),
                name: request.name.clone(),
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

    pub fn request(&self, account: &str, id: &str) -> Result<RequestView, StoreError> {
        let account = account_id(account)?;
        let mailbox = self.mailboxes.get(&account).ok_or(StoreError::Unknown)?;
        let request = mailbox.requests.get(id).ok_or(StoreError::Unknown)?;
        Ok(RequestView {
            state: match request.state {
                State::Pending => "pending",
                State::Approved => "approved",
                State::Rejected => "rejected",
            },
            device: request.device.clone(),
            name: request.name.clone(),
            chain: request.chain.clone(),
            wrap: request.wrap.clone(),
        })
    }

    fn request_mut(&mut self, account: &str, id: &str) -> Result<&mut Request, StoreError> {
        let account = account_id(account)?;
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

fn account_id(account: &str) -> Result<String, StoreError> {
    let account = account.trim().to_ascii_lowercase();
    if account.len() == 64 && account.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(account)
    } else {
        Err(StoreError::Unreadable)
    }
}

fn sealed_record(bytes: &[u8]) -> Result<(), StoreError> {
    if bytes.len() <= MAX_RECORD && bytes.starts_with(RECORD_MAGIC) {
        Ok(())
    } else {
        Err(StoreError::Unreadable)
    }
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

    fn account() -> String {
        "ab".repeat(32)
    }

    fn record() -> Vec<u8> {
        b"OMACUS-SYNC/1\nsealed".to_vec()
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
        let id = store.ask(&account, vec![7u8; 80], "This phone").unwrap();
        store.reject(&account, &id).unwrap();
        assert_eq!(store.request(&account, &id).unwrap().state, "rejected");
        assert!(store.pending(&account).unwrap().is_empty());
    }
}
