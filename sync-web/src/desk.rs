//! One device in the private sync test.
//!
//! The words and the page stay here. What leaves is a sealed record, a public
//! device, or a sealed copy for a device that was just approved.

use base64::Engine;
use omacus_engine::sync_keys::{
    self, DeviceChain, DeviceKeys, DevicePublic, RecoveryKey, Root, ServerView, SyncKeyError,
    VaultKey,
};
use qrcode::QrCode;

#[cfg(target_arch = "wasm32")]
mod clock {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        type Date;

        #[wasm_bindgen(static_method_of = Date)]
        fn now() -> f64;
    }

    pub fn unix_ms() -> u64 {
        Date::now() as u64
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod clock {
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn unix_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0)
    }
}

const EPOCH: u64 = 1;
const PAGE_ID: &[u8] = b"trip";
/// The sealed field that holds every page, not the sample page.
pub const PAGES_ID: &[u8] = b"pages";
/// The sample page. Made up, same shape as the editor behind the test screens.
pub const SAMPLE_PAGE: &str = "Trip Budget\nferry = 40\nmilk $3";
/// Word numbers the confirm screen asks for. 1-based, as the sheet numbers them.
pub const CONFIRM_AT: [u8; 3] = [3, 7, 11];

pub struct Desk {
    device: DeviceKeys,
    vault: Option<VaultKey>,
    chain: Option<DeviceChain>,
    account: Option<[u8; 32]>,
    epoch: u64,
    phrase_wrap: Vec<u8>,
    wraps: Vec<Vec<u8>>,
    sealed_page: Vec<u8>,
    recovery: Option<RecoveryKey>,
    root: Option<Root>,
    phrase_kek: Option<[u8; 32]>,
    /// The part of the recovery words that can seal a new key. Not a decryption key.
    recovery_public: Option<[u8; 32]>,
    /// Phones still on the list when the words could not open the newest seal.
    prior_devices: Vec<DevicePublic>,
    /// This device has opened a signed record. An unsigned copy is then refused.
    signed_seen: bool,
    /// Highest seal counter opened on this phone. An older seal is refused.
    seen_gen: u64,
    /// The six numbers were checked on this phone before the first list was stored.
    link_confirmed: bool,
    /// The next record is signed by the account key. Used for one recovery fork.
    sign_recovery: bool,
}

/// What the recovery words could open from one record.
pub struct PhraseOpen {
    pub pages: Vec<u8>,
    /// The newest seal did not open. `pages` is an older copy, or empty.
    pub partial: bool,
}

/// What [`Desk::absorb_record`] opened.
pub struct OpenedRecord {
    pub pages: Vec<u8>,
    pub member: bool,
    /// False when the pages already on this device should stay as they are.
    pub replace: bool,
    /// True when this device should publish its own sealed pages again.
    pub republish: bool,
    /// True when the pages already here must be combined with `pages`.
    pub combine: bool,
}

pub struct Approval {
    pub chain: Vec<u8>,
    pub wrap: Vec<u8>,
    pub record: Vec<u8>,
}

/// Headers for one mailbox request. The signing seed is not here.
pub struct RequestAuth {
    pub device: String,
    pub time: String,
    pub nonce: String,
    pub signature: String,
}

impl RequestAuth {
    pub fn headers(&self) -> String {
        format!(
            "X-Omacus-Device: {}\r\nX-Omacus-Time: {}\r\nX-Omacus-Nonce: {}\r\nX-Omacus-Signature: {}\r\n",
            self.device, self.time, self.nonce, self.signature
        )
    }
}

pub struct PhraseCheck {
    pub ok: bool,
    pub count: usize,
    pub message: String,
    pub normalized: String,
}

pub fn check_phrase(text: &str) -> PhraseCheck {
    let normalized = sync_keys::normalize_phrase(text);
    let count = if normalized.is_empty() {
        0
    } else {
        normalized.split_whitespace().count()
    };
    if count != 12 {
        let message = if count == 1 {
            "That's 1 word. Recovery words are 12.".to_string()
        } else {
            format!("That's {count} words. Recovery words are 12.")
        };
        return PhraseCheck {
            ok: false,
            count,
            message,
            normalized,
        };
    }
    match RecoveryKey::parse(&normalized) {
        Ok(_) => PhraseCheck {
            ok: true,
            count,
            message: "Looks right. 12 words.".to_string(),
            normalized,
        },
        Err(error) => PhraseCheck {
            ok: false,
            count,
            message: error.message().to_string(),
            normalized,
        },
    }
}

impl Desk {
    pub fn start_new() -> Result<Self, SyncKeyError> {
        Ok(Self {
            device: DeviceKeys::generate()?,
            vault: None,
            chain: None,
            account: None,
            epoch: EPOCH,
            phrase_wrap: Vec::new(),
            wraps: Vec::new(),
            sealed_page: Vec::new(),
            recovery: Some(RecoveryKey::generate()?),
            root: None,
            phrase_kek: None,
            recovery_public: None,
            prior_devices: Vec::new(),
            signed_seen: false,
            seen_gen: 0,
            link_confirmed: false,
            sign_recovery: false,
        })
    }

    pub fn start_join() -> Result<Self, SyncKeyError> {
        Ok(Self {
            device: DeviceKeys::generate()?,
            vault: None,
            chain: None,
            account: None,
            epoch: EPOCH,
            phrase_wrap: Vec::new(),
            wraps: Vec::new(),
            sealed_page: Vec::new(),
            recovery: None,
            root: None,
            phrase_kek: None,
            recovery_public: None,
            prior_devices: Vec::new(),
            signed_seen: false,
            seen_gen: 0,
            link_confirmed: false,
            sign_recovery: false,
        })
    }

    pub fn start_recovery(text: &str) -> Result<Self, SyncKeyError> {
        let check = check_phrase(text);
        if !check.ok {
            return Err(SyncKeyError::NotAKey);
        }
        Ok(Self {
            device: DeviceKeys::generate()?,
            vault: None,
            chain: None,
            account: None,
            epoch: EPOCH,
            phrase_wrap: Vec::new(),
            wraps: Vec::new(),
            sealed_page: Vec::new(),
            recovery: Some(RecoveryKey::parse(&check.normalized)?),
            root: None,
            phrase_kek: None,
            recovery_public: None,
            prior_devices: Vec::new(),
            signed_seen: false,
            seen_gen: 0,
            link_confirmed: false,
            sign_recovery: false,
        })
    }

    pub fn words(&self) -> Option<[&'static str; 12]> {
        self.recovery.as_ref().map(RecoveryKey::words)
    }

    pub fn sheet_svg(&self) -> Result<String, SyncKeyError> {
        let words = self.words().ok_or(SyncKeyError::NotAKey)?;
        let mut payload = String::from("omacus-recovery\n");
        for (index, word) in words.iter().enumerate() {
            payload.push_str(&format!("{}. {word}\n", index + 1));
        }
        qr_svg(&payload, "Recovery sheet")
    }

    pub fn confirm_saved(&mut self, picks: &[(u8, &str)]) -> Result<Vec<u8>, SyncKeyError> {
        let recovery = self.recovery.take().ok_or(SyncKeyError::NotAKey)?;
        let confirmed = match recovery.confirm(picks) {
            Ok(confirmed) => confirmed,
            Err(error) => {
                self.recovery = Some(recovery);
                return Err(error);
            }
        };
        let (account, phrase, vault, kek, public) = {
            let root = confirmed.root("")?;
            let account = root.account_public()?;
            let vault = VaultKey::generate()?;
            let kek = sync_keys::phrase_kek(&root)?;
            let public = sync_keys::recovery_public_key(&root)?;
            let phrase = sync_keys::wrap_vault_for_recovery(&public, &vault, self.epoch)?;
            (account, phrase, vault, kek, public)
        };
        let chain = sync_keys::found_devices(&self.device)?;
        let wrap = sync_keys::wrap_vault_for_device(
            &self.device.public(),
            &vault,
            self.epoch,
            Some(&public),
        )?;
        let sealed = sync_keys::seal_page(&vault, self.epoch, PAGE_ID, SAMPLE_PAGE.as_bytes())?;
        self.account = Some(account);
        self.vault = Some(vault);
        self.chain = Some(chain);
        self.phrase_wrap = phrase;
        self.wraps = vec![wrap];
        self.sealed_page = sealed;
        self.phrase_kek = Some(kek);
        self.recovery_public = Some(public);
        self.ensure_recovery()?;
        self.signed_seen = true;
        self.record()
    }

    pub fn account_hex(&self) -> Option<String> {
        self.account.map(|account| hex(&account))
    }

    pub fn pairing_code(&self) -> String {
        sync_keys::pairing_code(&self.device.public())
    }

    /// The six numbers on this phone match the other phone or computer.
    pub fn confirm_link(&mut self) {
        self.link_confirmed = true;
    }

    /// Four words for this account, from the public key.
    pub fn account_words(&self) -> Result<String, SyncKeyError> {
        let account = self.account.ok_or(SyncKeyError::Corrupt)?;
        Ok(sync_keys::account_fingerprint(&account).join(" "))
    }

    pub fn public_device(&self) -> Vec<u8> {
        encode_device(&self.device.public())
    }

    pub fn approve_new(&mut self, device: &[u8]) -> Result<Approval, SyncKeyError> {
        let newcomer = decode_device(device)?;
        let chain = self
            .chain
            .as_ref()
            .ok_or(SyncKeyError::NotApproved(sync_keys::NO_LIST))?;
        let vault = self.vault.as_ref().ok_or(SyncKeyError::Sealed)?;
        let (chain, wrap) = sync_keys::approve(
            &self.device,
            chain,
            &newcomer,
            vault,
            self.epoch,
            self.recovery_public.as_ref(),
        )?;
        self.chain = Some(chain);
        self.wraps.push(wrap.clone());
        self.ensure_recovery()?;
        self.signed_seen = true;
        let record = self.record()?;
        let chain_bytes = self.chain.as_ref().ok_or(SyncKeyError::Corrupt)?.to_bytes();
        Ok(Approval {
            chain: chain_bytes,
            wrap,
            record,
        })
    }

    /// Put this phone on the list the words just opened. The recovery signer
    /// adds it. The vault key does not change, so the other phones keep opening
    /// the same pages and then see this phone on the longer list.
    pub fn claim_listed(&mut self) -> Result<(), SyncKeyError> {
        if self.member() {
            return Ok(());
        }
        let root = self.root.as_ref().ok_or(SyncKeyError::NotConfirmed)?;
        let chain = self
            .chain
            .clone()
            .ok_or(SyncKeyError::NotApproved(sync_keys::NO_LIST))?;
        let vault = self.vault.as_ref().ok_or(SyncKeyError::Sealed)?;
        let (chain, wrap) = sync_keys::admit_with_recovery(
            root,
            &chain,
            &self.device.public(),
            vault,
            self.epoch,
            self.recovery_public.as_ref(),
        )?;
        self.chain = Some(chain);
        self.wraps.push(wrap);
        self.ensure_recovery()?;
        self.signed_seen = true;
        Ok(())
    }

    pub fn open_approval(
        &mut self,
        chain: &[u8],
        wrap: &[u8],
        record: &[u8],
    ) -> Result<String, SyncKeyError> {
        let page = self.unlock_approval(chain, wrap, record)?;
        String::from_utf8(page).map_err(|_| SyncKeyError::Corrupt)
    }

    /// Same as [`Desk::open_approval`], returning the sealed pages as bytes.
    pub fn open_approval_pages(
        &mut self,
        chain: &[u8],
        wrap: &[u8],
        record: &[u8],
    ) -> Result<Vec<u8>, SyncKeyError> {
        self.unlock_approval(chain, wrap, record)
    }

    /// Confirm the three words and remember the account they open.
    pub fn recovery_account(&mut self, picks: &[(u8, &str)]) -> Result<String, SyncKeyError> {
        let recovery = self.recovery.take().ok_or(SyncKeyError::NotAKey)?;
        let confirmed = match recovery.confirm(picks) {
            Ok(confirmed) => confirmed,
            Err(error) => {
                self.recovery = Some(recovery);
                return Err(error);
            }
        };
        let root = confirmed.root("")?;
        let account = root.account_public()?;
        self.account = Some(account);
        self.root = Some(root);
        self.account_hex().ok_or(SyncKeyError::Corrupt)
    }

    /// The 12 words were typed. Remember the account they open, without a second check.
    pub fn accept_phrase(&mut self) -> Result<String, SyncKeyError> {
        let recovery = self.recovery.take().ok_or(SyncKeyError::NotAKey)?;
        let confirmed = match recovery.accept() {
            Ok(confirmed) => confirmed,
            Err(error) => {
                self.recovery = Some(recovery);
                return Err(error);
            }
        };
        let root = confirmed.root("")?;
        let account = root.account_public()?;
        let public = sync_keys::recovery_public_key(&root)?;
        let kek = sync_keys::phrase_kek(&root)?;
        self.account = Some(account);
        self.recovery_public = Some(public);
        self.phrase_kek = Some(kek);
        self.root = Some(root);
        self.account_hex().ok_or(SyncKeyError::Corrupt)
    }

    pub fn recovery_open(&mut self, record: &[u8]) -> Result<String, SyncKeyError> {
        let opened = self.unlock_phrase(record)?;
        String::from_utf8(opened.pages).map_err(|_| SyncKeyError::Corrupt)
    }

    /// Same as [`Desk::recovery_open`], returning the sealed pages as bytes.
    /// `partial` means the newest seal did not open. `pages` is the newest copy
    /// the words could open, which may be empty.
    pub fn recovery_open_pages(&mut self, record: &[u8]) -> Result<PhraseOpen, SyncKeyError> {
        self.unlock_phrase(record)
    }

    /// Seal `packed` for these words and for this phone, and for any phone that
    /// was still on the list. Later recovery opens this seal. A phone from that
    /// list can open it too, and merge what it still has.
    pub fn reestablish_pages(&mut self, packed: &[u8]) -> Result<Vec<u8>, SyncKeyError> {
        let public = self.recovery_public.ok_or(SyncKeyError::NotConfirmed)?;
        let kek = self.phrase_kek.ok_or(SyncKeyError::NotConfirmed)?;
        let vault = VaultKey::generate()?;
        let epoch = self.epoch;
        let mine = *self.device.public().id();
        let me = self.device.public();
        let (chain, wraps) = {
            let root = self.root.as_ref().ok_or(SyncKeyError::NotConfirmed)?;
            let mut chain = sync_keys::found_recovery(root, &public)?;
            let mut wraps = Vec::new();
            let (next, wrap) =
                sync_keys::admit_with_recovery(root, &chain, &me, &vault, epoch, Some(&public))?;
            chain = next;
            wraps.push(wrap);
            for device in &self.prior_devices {
                if *device.id() == mine {
                    continue;
                }
                let (next, wrap) = sync_keys::admit_with_recovery(
                    root,
                    &chain,
                    device,
                    &vault,
                    epoch,
                    Some(&public),
                )?;
                chain = next;
                wraps.push(wrap);
            }
            (chain, wraps)
        };
        let phrase = sync_keys::wrap_vault_for_recovery(&public, &vault, self.epoch)?;
        let sealed = sync_keys::seal_page(&vault, self.epoch, PAGES_ID, packed)?;
        self.vault = Some(vault);
        self.chain = Some(chain);
        self.phrase_wrap = phrase;
        self.wraps = wraps;
        self.sealed_page = sealed;
        self.phrase_kek = Some(kek);
        self.recovery_public = Some(public);
        self.prior_devices.clear();
        self.ensure_recovery()?;
        self.signed_seen = true;
        self.sign_recovery = true;
        let record = self.record();
        self.sign_recovery = false;
        record
    }

    pub fn export_local(&self) -> Result<Vec<u8>, SyncKeyError> {
        let vault = self.vault.as_ref().ok_or(SyncKeyError::Sealed)?;
        let account = self.account.ok_or(SyncKeyError::Corrupt)?;
        let chain = self
            .chain
            .as_ref()
            .map(DeviceChain::to_bytes)
            .unwrap_or_default();
        let kek = self.phrase_kek.unwrap_or([0u8; 32]);
        let stored_kek: &[u8] = if self.phrase_kek.is_some() { &kek } else { &[] };
        let public = self.recovery_public.unwrap_or([0u8; 32]);
        let stored_public: &[u8] = if self.recovery_public.is_some() {
            &public
        } else {
            &[]
        };
        sync_keys::store_device(
            &self.device,
            vault,
            &account,
            self.epoch,
            &chain,
            stored_kek,
            stored_public,
            self.signed_seen,
            self.seen_gen,
        )
    }

    pub fn import_local(bytes: &[u8]) -> Result<Self, SyncKeyError> {
        let file = sync_keys::open_device_store(bytes)?;
        let chain = if file.chain.is_empty() {
            None
        } else {
            let chain = DeviceChain::from_bytes(&file.chain)?;
            chain.verify()?;
            Some(chain)
        };
        let link_confirmed = chain.is_some();
        Ok(Self {
            device: file.device,
            vault: Some(file.vault),
            chain,
            account: Some(file.account),
            epoch: file.epoch,
            phrase_wrap: Vec::new(),
            wraps: Vec::new(),
            sealed_page: Vec::new(),
            recovery: None,
            root: None,
            phrase_kek: if file.kek.len() == 32 {
                let mut kek = [0u8; 32];
                kek.copy_from_slice(&file.kek);
                Some(kek)
            } else {
                None
            },
            recovery_public: if file.recovery_public.len() == 32 {
                let mut public = [0u8; 32];
                public.copy_from_slice(&file.recovery_public);
                Some(public)
            } else {
                None
            },
            prior_devices: Vec::new(),
            signed_seen: file.signed_seen,
            seen_gen: file.seen_gen,
            link_confirmed,
            sign_recovery: false,
        })
    }

    pub fn seal_pages(&mut self, packed: &[u8]) -> Result<Vec<u8>, SyncKeyError> {
        let vault = self.vault.as_ref().ok_or(SyncKeyError::Sealed)?;
        self.sealed_page = sync_keys::seal_page(vault, self.epoch, PAGES_ID, packed)?;
        self.ensure_recovery()?;
        self.signed_seen = true;
        self.record()
    }

    pub fn read_pages(&self) -> Result<Vec<u8>, SyncKeyError> {
        let vault = self.vault.as_ref().ok_or(SyncKeyError::Sealed)?;
        open_sealed(vault, self.epoch, &self.sealed_page)
    }

    /// Open a record. A chain that drops a removal we already have is not adopted.
    /// A new seal for the remaining phones is opened from this device's wrap.
    /// A phone that was removed keeps the pages it already has and is no longer a member.
    pub fn absorb_record(&mut self, record: &[u8]) -> Result<OpenedRecord, SyncKeyError> {
        self.absorb_with(record, open_sealed)
    }

    fn absorb_with(
        &mut self,
        record: &[u8],
        open_blob: OpenBlob,
    ) -> Result<OpenedRecord, SyncKeyError> {
        let view = ServerView::parse(record)?;
        view.chain.verify()?;
        if let Some(account) = self.account
            && account != view.account
        {
            return Err(SyncKeyError::Sealed);
        }
        let mine = *self.device.public().id();
        let on_list = view
            .chain
            .devices()?
            .iter()
            .any(|device| *device.id() == mine);
        let order = self
            .chain
            .as_ref()
            .map(|local| view.chain.compared_with(local));
        // A different list is followed only when it starts at the account key.
        // A new vault key needs that fork, or a list that extends the one here.
        // Both are decided before the wrap is opened and before a signature
        // on the forged list is remembered.
        let adopt = self.chain_decision(&view, order)?;
        let unsigned = self.note_signature(&view)?;
        let diverged = matches!(order, Some(sync_keys::ChainOrder::Diverge));
        let opened = if let Some(vault) = &self.vault {
            if vault.id() == &view.key_id {
                Some(Opened::Current(open_blob(vault, view.epoch, &view.page)?))
            } else {
                open_wrapped(&self.device, &view, open_blob)?.map(Opened::Wrapped)
            }
        } else {
            open_wrapped(&self.device, &view, open_blob)?.map(Opened::Wrapped)
        };
        match opened {
            Some(Opened::Current(pages)) => {
                if adopt {
                    self.note_public_from_wraps(&view.device_wraps, view.epoch, &view.key_id);
                    self.adopt_view(view);
                } else {
                    self.account = Some(view.account);
                }
                let repaired = self.repair_phrase();
                Ok(OpenedRecord {
                    pages,
                    member: self.member(),
                    replace: true,
                    republish: repaired || unsigned,
                    combine: false,
                })
            }
            Some(Opened::Wrapped(WrappedPages {
                vault,
                pages,
                recovery_public: public,
            })) => {
                self.note_recovery(public);
                let republish_older = !adopt && !diverged && self.member();
                if adopt {
                    self.adopt_view(view);
                    self.vault = Some(vault);
                } else {
                    self.account = Some(view.account);
                }
                let repaired = self.repair_phrase();
                Ok(OpenedRecord {
                    pages,
                    member: self.member(),
                    replace: true,
                    republish: republish_older || repaired || diverged || unsigned,
                    combine: diverged,
                })
            }
            None if !on_list => {
                if adopt {
                    self.chain = Some(view.chain);
                    self.epoch = view.epoch;
                }
                self.account = Some(view.account);
                Ok(OpenedRecord {
                    pages: Vec::new(),
                    member: false,
                    replace: false,
                    republish: false,
                    combine: false,
                })
            }
            None if self.member() => {
                let _ = self.repair_phrase();
                Ok(OpenedRecord {
                    pages: Vec::new(),
                    member: true,
                    replace: false,
                    republish: true,
                    combine: false,
                })
            }
            None => Err(SyncKeyError::Sealed),
        }
    }

    /// `true` means this record's list replaces the one stored here.
    fn chain_decision(
        &self,
        view: &ServerView,
        order: Option<sync_keys::ChainOrder>,
    ) -> Result<bool, SyncKeyError> {
        let new_key = self
            .vault
            .as_ref()
            .is_some_and(|vault| vault.id() != &view.key_id);
        let recovery = self
            .account
            .is_some_and(|account| sync_keys::recovery_signed_record(view, &account));
        if new_key {
            let extends = matches!(order, Some(sync_keys::ChainOrder::Newer));
            let first = order.is_none();
            if !extends && !recovery && !first {
                return Err(SyncKeyError::Sealed);
            }
        }
        match order {
            None => {
                if !self.link_confirmed {
                    return Err(SyncKeyError::NotApproved(sync_keys::CONFIRM_LINK));
                }
                Ok(true)
            }
            Some(sync_keys::ChainOrder::Newer | sync_keys::ChainOrder::Same) => Ok(true),
            Some(sync_keys::ChainOrder::Older) => Ok(false),
            Some(sync_keys::ChainOrder::Diverge) => {
                if recovery {
                    Ok(true)
                } else {
                    Err(SyncKeyError::Sealed)
                }
            }
        }
    }

    fn accept_incoming_chain(&self, chain: &DeviceChain) -> Result<(), SyncKeyError> {
        if let Some(local) = &self.chain {
            let recovery = self
                .account
                .is_some_and(|account| chain.recovery_fork(&account));
            match chain.compared_with(local) {
                sync_keys::ChainOrder::Newer | sync_keys::ChainOrder::Same => Ok(()),
                sync_keys::ChainOrder::Older => Err(SyncKeyError::Sealed),
                sync_keys::ChainOrder::Diverge => {
                    if recovery {
                        Ok(())
                    } else {
                        Err(SyncKeyError::Sealed)
                    }
                }
            }
        } else if self.link_confirmed {
            Ok(())
        } else {
            Err(SyncKeyError::NotApproved(sync_keys::CONFIRM_LINK))
        }
    }

    pub fn member(&self) -> bool {
        let mine = *self.device.public().id();
        self.chain
            .as_ref()
            .and_then(|chain| chain.devices().ok())
            .is_some_and(|devices| devices.iter().any(|device| *device.id() == mine))
    }

    fn require_self(&self, chain: &DeviceChain) -> Result<(), SyncKeyError> {
        let mine = *self.device.public().id();
        if chain.devices()?.iter().any(|device| *device.id() == mine) {
            Ok(())
        } else {
            Err(SyncKeyError::NotApproved(sync_keys::NOT_ON_LIST))
        }
    }

    /// This phone was on the list and then dropped. Opening the latest pages
    /// with the words adds this phone, so that path is on the list.
    pub fn formerly_listed(&self) -> bool {
        if self.member() {
            return false;
        }
        let mine = *self.device.public().id();
        self.chain
            .as_ref()
            .is_some_and(|chain| chain.contained(&mine))
    }

    pub fn listed_devices(&self) -> Result<Vec<(String, String, bool)>, SyncKeyError> {
        let chain = self
            .chain
            .as_ref()
            .ok_or(SyncKeyError::NotApproved(sync_keys::NO_LIST))?;
        let mine = *self.device.public().id();
        let mut rows = Vec::new();
        for device in chain.devices()? {
            if *device.id() == sync_keys::RECOVERY_ID {
                continue;
            }
            let code = sync_keys::pairing_code(&device);
            rows.push((hex(device.id()), code, *device.id() == mine));
        }
        Ok(rows)
    }

    pub fn remove_listed(&mut self, id: &[u8; 16]) -> Result<Vec<u8>, SyncKeyError> {
        let chain = self
            .chain
            .clone()
            .ok_or(SyncKeyError::NotApproved(sync_keys::NO_LIST))?;
        let chain = sync_keys::remove_device(&self.device, &chain, id)?;
        let vault = VaultKey::generate()?;
        let mut wraps = Vec::new();
        for device in chain.devices()? {
            if *device.id() == sync_keys::RECOVERY_ID {
                continue;
            }
            wraps.push(sync_keys::wrap_vault_for_device(
                &device,
                &vault,
                self.epoch,
                self.recovery_public.as_ref(),
            )?);
        }
        if let Some(phrase) = self.reseal_phrase(&vault) {
            self.phrase_wrap = phrase;
        }
        self.vault = Some(vault);
        self.wraps = wraps;
        self.chain = Some(chain);
        self.ensure_recovery()?;
        self.signed_seen = true;
        self.record()
    }

    fn unlock_approval(
        &mut self,
        chain: &[u8],
        wrap: &[u8],
        record: &[u8],
    ) -> Result<Vec<u8>, SyncKeyError> {
        let chain = DeviceChain::from_bytes(chain)?;
        chain
            .verify()
            .map_err(|error| sync_keys::scope_list_error(error, false))?;
        self.require_self(&chain)?;
        self.accept_incoming_chain(&chain)?;
        let view = ServerView::parse(record)?;
        view.chain
            .verify()
            .map_err(|error| sync_keys::scope_list_error(error, true))?;
        self.note_signature(&view)?;
        let opened =
            sync_keys::open_device_wrap_parts(&self.device, wrap, view.epoch, &view.key_id)?;
        let pages = open_sealed(&opened.vault, view.epoch, &view.page)?;
        self.note_recovery(opened.recovery_public);
        self.keep_opened(view, opened.vault, chain, false)?;
        let _ = self.repair_phrase();
        Ok(pages)
    }

    fn unlock_phrase(&mut self, record: &[u8]) -> Result<PhraseOpen, SyncKeyError> {
        let view = ServerView::parse(record)?;
        if self.account != Some(view.account) {
            return Err(SyncKeyError::Sealed);
        }
        view.chain
            .verify()
            .map_err(|error| sync_keys::scope_list_error(error, true))?;
        self.note_signature(&view)?;
        let root = self.root.take().ok_or(SyncKeyError::NotConfirmed)?;
        let kek = match sync_keys::phrase_kek(&root) {
            Ok(kek) => kek,
            Err(error) => {
                self.root = Some(root);
                return Err(error);
            }
        };
        let public = match sync_keys::recovery_public_key(&root) {
            Ok(public) => public,
            Err(error) => {
                self.root = Some(root);
                return Err(error);
            }
        };
        let vault = phrase_vault(&root, &view);
        let opened = match vault {
            Some(vault) => match open_sealed(&vault, view.epoch, &view.page) {
                Ok(pages) if vault.id() == &view.key_id => {
                    self.phrase_kek = Some(kek);
                    self.recovery_public = Some(public);
                    let chain = view.chain.clone();
                    self.keep_opened(view, vault, chain, true)?;
                    PhraseOpen {
                        pages,
                        partial: false,
                    }
                }
                Ok(pages) => {
                    self.remember_partial(&view, kek, public);
                    PhraseOpen {
                        pages,
                        partial: true,
                    }
                }
                Err(_) => {
                    self.remember_partial(&view, kek, public);
                    PhraseOpen {
                        pages: Vec::new(),
                        partial: true,
                    }
                }
            },
            None => {
                self.remember_partial(&view, kek, public);
                PhraseOpen {
                    pages: Vec::new(),
                    partial: true,
                }
            }
        };
        self.root = Some(root);
        if !opened.partial {
            let _ = self.claim_listed();
        }
        Ok(opened)
    }

    fn remember_partial(&mut self, view: &ServerView, kek: [u8; 32], public: [u8; 32]) {
        self.epoch = view.epoch;
        self.account = Some(view.account);
        self.phrase_kek = Some(kek);
        self.recovery_public = Some(public);
        self.prior_devices = view
            .chain
            .devices()
            .unwrap_or_default()
            .into_iter()
            .filter(|device| *device.id() != sync_keys::RECOVERY_ID)
            .collect();
    }

    pub fn device(&self) -> &DeviceKeys {
        &self.device
    }

    pub fn vault(&self) -> Option<&VaultKey> {
        self.vault.as_ref()
    }

    pub fn root(&self) -> Option<&Root> {
        self.root.as_ref()
    }

    fn adopt_view(&mut self, view: ServerView) {
        self.epoch = view.epoch;
        self.account = Some(view.account);
        self.phrase_wrap = view.phrase_wrap;
        self.wraps = view.device_wraps;
        self.sealed_page = view.page;
        self.chain = Some(view.chain);
    }

    fn note_recovery(&mut self, public: Option<[u8; 32]>) {
        if self.recovery_public.is_none() {
            self.recovery_public = public;
        }
    }

    fn note_public_from_wraps(&mut self, wraps: &[Vec<u8>], epoch: u64, key_id: &[u8; 16]) {
        if self.recovery_public.is_some() {
            return;
        }
        for wrap in wraps {
            if let Ok(opened) = sync_keys::open_device_wrap_parts(&self.device, wrap, epoch, key_id)
            {
                self.note_recovery(opened.recovery_public);
                if self.recovery_public.is_some() {
                    return;
                }
            }
        }
    }

    fn phrase_needs_repair(&self, vault: &VaultKey) -> bool {
        if self.recovery_public.is_none() && self.phrase_kek.is_none() {
            return false;
        }
        if let Some(id) = sync_keys::recovery_wrap_key_id(&self.phrase_wrap) {
            return id != *vault.id();
        }
        if self.recovery_public.is_some() {
            return true;
        }
        let Some(kek) = self.phrase_kek else {
            return false;
        };
        sync_keys::open_phrase_wrap_with_kek(&kek, &self.phrase_wrap, self.epoch, vault.id())
            .is_err()
    }

    fn reseal_phrase(&self, vault: &VaultKey) -> Option<Vec<u8>> {
        if let Some(public) = self.recovery_public {
            return sync_keys::wrap_vault_for_recovery(&public, vault, self.epoch).ok();
        }
        if let Some(kek) = self.phrase_kek {
            return sync_keys::wrap_vault_with_kek(&kek, vault, self.epoch).ok();
        }
        None
    }

    /// A linked device that can seal for the words replaces a wrap that cannot open them.
    fn repair_phrase(&mut self) -> bool {
        let Some(phrase) = self.fresh_phrase() else {
            return false;
        };
        self.phrase_wrap = phrase;
        true
    }

    fn fresh_phrase(&self) -> Option<Vec<u8>> {
        if !self.member() {
            return None;
        }
        let vault = self.vault.as_ref()?;
        if !self.phrase_needs_repair(vault) {
            return None;
        }
        self.reseal_phrase(vault)
    }

    fn keep_opened(
        &mut self,
        view: ServerView,
        vault: VaultKey,
        chain: DeviceChain,
        from_words: bool,
    ) -> Result<(), SyncKeyError> {
        if !from_words {
            self.accept_incoming_chain(&chain)?;
        }
        if let Some(local) = &self.chain
            && self
                .vault
                .as_ref()
                .is_some_and(|have| have.id() != vault.id())
        {
            let extends = matches!(chain.compared_with(local), sync_keys::ChainOrder::Newer);
            let recovery = self
                .account
                .is_some_and(|account| chain.recovery_fork(&account));
            if !extends && !recovery {
                return Err(SyncKeyError::Sealed);
            }
        }
        self.epoch = view.epoch;
        self.account = Some(view.account);
        self.phrase_wrap = view.phrase_wrap;
        self.wraps = view.device_wraps;
        self.sealed_page = view.page;
        self.chain = Some(chain);
        self.vault = Some(vault);
        Ok(())
    }

    fn record(&self) -> Result<Vec<u8>, SyncKeyError> {
        self.record_at(1)
    }

    fn record_at(&self, version: u8) -> Result<Vec<u8>, SyncKeyError> {
        let view = ServerView {
            epoch: self.epoch,
            account: self.account.ok_or(SyncKeyError::Corrupt)?,
            chain: self.chain.clone().ok_or(SyncKeyError::Corrupt)?,
            phrase_wrap: self.phrase_wrap.clone(),
            device_wraps: self.wraps.clone(),
            key_id: *self.vault.as_ref().ok_or(SyncKeyError::Corrupt)?.id(),
            page: self.sealed_page.clone(),
            version,
            signer: None,
            signature: None,
        };
        let prefix = view.to_bytes(version);
        let (signer, mut secret) = self.record_secret()?;
        let signed = sync_keys::append_record_signature(&prefix, &signer, &secret);
        secret.fill(0);
        Ok(signed)
    }

    fn record_secret(&self) -> Result<([u8; 16], [u8; 32]), SyncKeyError> {
        if self.sign_recovery
            && let Some(root) = &self.root
        {
            return Ok((sync_keys::RECOVERY_ID, root.account_signing_secret()?));
        }
        if self.member() {
            return Ok((*self.device.public().id(), *self.device.signing_secret()));
        }
        if let Some(root) = &self.root {
            return Ok((sync_keys::RECOVERY_ID, root.account_signing_secret()?));
        }
        Ok((*self.device.public().id(), *self.device.signing_secret()))
    }

    fn ensure_recovery(&mut self) -> Result<(), SyncKeyError> {
        if !self.member() {
            return Ok(());
        }
        let Some(account) = self.account else {
            return Ok(());
        };
        let Some(chain) = self.chain.clone() else {
            return Ok(());
        };
        let receive = self.recovery_public.unwrap_or([0u8; 32]);
        self.chain = Some(sync_keys::admit_recovery(
            &self.device,
            &chain,
            &account,
            &receive,
        )?);
        Ok(())
    }

    /// `Ok(true)` when the record is an older unsigned copy.
    fn note_signature(&mut self, view: &ServerView) -> Result<bool, SyncKeyError> {
        let unsigned = sync_keys::trust_signature(view, self.signed_seen)?;
        if !unsigned {
            self.signed_seen = true;
        }
        Ok(unsigned)
    }

    pub fn sign_request(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<RequestAuth, SyncKeyError> {
        let time = clock::unix_ms();
        let nonce = sync_keys::fresh_nonce()?;
        let (signer, mut secret) = if let Some(root) = &self.root {
            (sync_keys::RECOVERY_ID, root.account_signing_secret()?)
        } else {
            (*self.device.public().id(), *self.device.signing_secret())
        };
        let message = sync_keys::auth_message(method, path, time, &nonce, body);
        let signature = sync_keys::sign_auth(&secret, &message);
        secret.fill(0);
        Ok(RequestAuth {
            device: hex(&signer),
            time: time.to_string(),
            nonce: hex(&nonce),
            signature: hex(&signature),
        })
    }

    pub fn seal_library(
        &mut self,
        packed: &[u8],
        slots: &[omacus_engine::sync_objects::Slot],
    ) -> Result<omacus_engine::sync_objects::Planned, SyncKeyError> {
        let set = omacus_engine::sync_keys::PageSet::from_bytes(packed)?;
        let planned = {
            let vault = self.vault.as_ref().ok_or(SyncKeyError::Sealed)?;
            omacus_engine::sync_objects::plan(
                vault,
                self.epoch,
                &set,
                slots,
                self.seen_gen.saturating_add(1),
            )?
        };
        self.sealed_page = planned.sealed_manifest.clone();
        self.ensure_recovery()?;
        self.signed_seen = true;
        self.seen_gen = self.seen_gen.saturating_add(1);
        let _ = self.record_at(2)?;
        Ok(planned)
    }

    pub fn library_record(&self) -> Result<Vec<u8>, SyncKeyError> {
        self.record_at(2)
    }

    pub fn wanted_objects(
        &self,
        record: &[u8],
        slots: &[omacus_engine::sync_objects::Slot],
    ) -> Result<(bool, Vec<[u8; 16]>), SyncKeyError> {
        if !omacus_engine::sync_objects::is_v2(record) {
            return Ok((true, Vec::new()));
        }
        // A recovery-signed list can carry an older manifest generation.
        // The objects in that manifest still have to be fetched before it opens.
        let floor = if self.incoming_list_wins(record) {
            0
        } else {
            self.seen_gen
        };
        let plain = match self.peek_manifest_floor(record, floor) {
            Ok(plain) => plain,
            Err(_) => return Ok((false, Vec::new())),
        };
        let ids = omacus_engine::sync_objects::manifest_wants(&plain, slots)?;
        Ok((false, ids))
    }

    pub fn open_library(
        &mut self,
        record: &[u8],
        incoming: &[omacus_engine::sync_objects::Incoming],
        slots: &[omacus_engine::sync_objects::Slot],
        base_doc: &[u8],
    ) -> Result<LibraryOpened, SyncKeyError> {
        if !omacus_engine::sync_objects::is_v2(record) {
            let opened = self.absorb_record(record)?;
            return Ok(LibraryOpened {
                packed: opened.pages,
                slots: slots.to_vec(),
                legacy: true,
                member: opened.member,
                replace: opened.replace,
                republish: opened.republish,
                combine: opened.combine,
            });
        }
        // A recovery signer can add a phone and publish a longer list whose
        // manifest generation is still lower than the one this phone already
        // saved. That list has to be adopted before the next save. An older
        // generation on the same list is still refused, so a replay does not
        // roll the pages back.
        let wins = self.incoming_list_wins(record);
        let floor = if wins { 0 } else { self.seen_gen };
        let peeked = match self.peek_manifest_floor(record, floor) {
            Err(SyncKeyError::Older) if !wins => return Err(SyncKeyError::Older),
            other => other,
        };
        let generation = match &peeked {
            Ok(plain) => {
                let generation = omacus_engine::sync_objects::manifest_generation(plain)?;
                if generation < self.seen_gen && !wins {
                    return Err(SyncKeyError::Older);
                }
                Some(generation)
            }
            Err(_) => None,
        };
        let assembled = match peeked {
            Ok(plain) => Some(omacus_engine::sync_objects::assemble(
                &plain, incoming, slots, base_doc,
            )?),
            Err(_) => None,
        };
        let opened = self.absorb_with(record, open_manifest)?;
        if let Some(generation) = generation {
            self.seen_gen = self.seen_gen.max(generation);
        }
        if !opened.replace {
            return Ok(LibraryOpened {
                packed: Vec::new(),
                slots: slots.to_vec(),
                legacy: false,
                member: opened.member,
                replace: opened.replace,
                republish: opened.republish,
                combine: opened.combine,
            });
        }
        let assembled = assembled.ok_or(SyncKeyError::Sealed)?;
        Ok(LibraryOpened {
            packed: assembled.packed,
            slots: assembled.slots,
            legacy: false,
            member: opened.member,
            replace: opened.replace,
            republish: opened.republish,
            combine: opened.combine,
        })
    }

    /// A longer list, or a recovery-signed fork, replaces the list stored here
    /// even when its manifest generation is lower.
    fn incoming_list_wins(&self, record: &[u8]) -> bool {
        let Ok(view) = ServerView::parse(record) else {
            return false;
        };
        if view.chain.verify().is_err() {
            return false;
        }
        let order = self
            .chain
            .as_ref()
            .map(|local| view.chain.compared_with(local));
        matches!(
            (self.chain_decision(&view, order), order),
            (
                Ok(true),
                Some(sync_keys::ChainOrder::Newer | sync_keys::ChainOrder::Diverge)
            )
        )
    }

    fn peek_manifest(&self, record: &[u8]) -> Result<Vec<u8>, SyncKeyError> {
        self.peek_manifest_floor(record, self.seen_gen)
    }

    fn peek_manifest_floor(&self, record: &[u8], floor: u64) -> Result<Vec<u8>, SyncKeyError> {
        let view = ServerView::parse(record)?;
        view.chain.verify()?;
        sync_keys::trust_signature(&view, self.signed_seen)?;
        if let Some(vault) = &self.vault
            && vault.id() == &view.key_id
        {
            return open_manifest_at(vault, view.epoch, &view.page, floor);
        }
        for wrap in &view.device_wraps {
            if let Ok(opened) =
                sync_keys::open_device_wrap_parts(&self.device, wrap, view.epoch, &view.key_id)
            {
                return open_manifest_at(&opened.vault, view.epoch, &view.page, floor);
            }
        }
        Err(SyncKeyError::Sealed)
    }
}

impl Desk {
    pub fn recovery_peek(&mut self, record: &[u8]) -> Result<RecoveryPeeked, SyncKeyError> {
        let resign = ServerView::parse(record)
            .ok()
            .is_some_and(|view| view.signer.is_none());
        if !omacus_engine::sync_objects::is_v2(record) {
            let opened = self.recovery_open_pages(record)?;
            return Ok(RecoveryPeeked {
                packed: opened.pages,
                partial: opened.partial,
                legacy: true,
                want: Vec::new(),
                resign,
            });
        }
        let view = ServerView::parse(record)?;
        if self.account != Some(view.account) {
            return Err(SyncKeyError::Sealed);
        }
        view.chain
            .verify()
            .map_err(|error| sync_keys::scope_list_error(error, true))?;
        self.note_signature(&view)?;
        let root = self.root.take().ok_or(SyncKeyError::NotConfirmed)?;
        let kek = match sync_keys::phrase_kek(&root) {
            Ok(kek) => kek,
            Err(error) => {
                self.root = Some(root);
                return Err(error);
            }
        };
        let public = match sync_keys::recovery_public_key(&root) {
            Ok(public) => public,
            Err(error) => {
                self.root = Some(root);
                return Err(error);
            }
        };
        let Some(vault) = phrase_vault(&root, &view) else {
            self.remember_partial(&view, kek, public);
            self.root = Some(root);
            return Ok(RecoveryPeeked {
                packed: Vec::new(),
                partial: true,
                legacy: false,
                want: Vec::new(),
                resign,
            });
        };
        if vault.id() != &view.key_id {
            self.remember_partial(&view, kek, public);
            self.root = Some(root);
            return Ok(RecoveryPeeked {
                packed: Vec::new(),
                partial: true,
                legacy: false,
                want: Vec::new(),
                resign,
            });
        }
        let plain = match open_manifest_at(&vault, view.epoch, &view.page, self.seen_gen) {
            Ok(plain) => plain,
            Err(_) => {
                self.remember_partial(&view, kek, public);
                self.root = Some(root);
                return Ok(RecoveryPeeked {
                    packed: Vec::new(),
                    partial: true,
                    legacy: false,
                    want: Vec::new(),
                    resign,
                });
            }
        };
        let want = match omacus_engine::sync_objects::manifest_wants(&plain, &[]) {
            Ok(want) => want,
            Err(error) => {
                self.root = Some(root);
                return Err(error);
            }
        };
        self.phrase_kek = Some(kek);
        self.recovery_public = Some(public);
        let chain = view.chain.clone();
        self.keep_opened(view, vault, chain, true)?;
        self.root = Some(root);
        let _ = self.claim_listed();
        Ok(RecoveryPeeked {
            packed: Vec::new(),
            partial: false,
            legacy: false,
            want,
            resign,
        })
    }

    pub fn recovery_apply(
        &mut self,
        record: &[u8],
        incoming: &[omacus_engine::sync_objects::Incoming],
    ) -> Result<(Vec<u8>, Vec<omacus_engine::sync_objects::Slot>), SyncKeyError> {
        let plain = self.peek_manifest(record)?;
        let assembled = omacus_engine::sync_objects::assemble(&plain, incoming, &[], &[])?;
        Ok((assembled.packed, assembled.slots))
    }

    pub fn approval_plan(
        &mut self,
        chain: &[u8],
        wrap: &[u8],
        record: &[u8],
    ) -> Result<ApprovalPlan, SyncKeyError> {
        if !omacus_engine::sync_objects::is_v2(record) {
            let packed = self.unlock_approval(chain, wrap, record)?;
            return Ok(ApprovalPlan {
                packed,
                want: Vec::new(),
                legacy: true,
            });
        }
        let chain = DeviceChain::from_bytes(chain)?;
        chain
            .verify()
            .map_err(|error| sync_keys::scope_list_error(error, false))?;
        self.require_self(&chain)?;
        self.accept_incoming_chain(&chain)?;
        let view = ServerView::parse(record)?;
        view.chain
            .verify()
            .map_err(|error| sync_keys::scope_list_error(error, true))?;
        self.note_signature(&view)?;
        let opened =
            sync_keys::open_device_wrap_parts(&self.device, wrap, view.epoch, &view.key_id)?;
        let plain = open_manifest_at(&opened.vault, view.epoch, &view.page, self.seen_gen)?;
        let want = omacus_engine::sync_objects::manifest_wants(&plain, &[])?;
        self.note_recovery(opened.recovery_public);
        self.keep_opened(view, opened.vault, chain, false)?;
        let _ = self.repair_phrase();
        Ok(ApprovalPlan {
            packed: Vec::new(),
            want,
            legacy: false,
        })
    }

    pub fn approval_finish(
        &mut self,
        record: &[u8],
        incoming: &[omacus_engine::sync_objects::Incoming],
    ) -> Result<(Vec<u8>, Vec<omacus_engine::sync_objects::Slot>), SyncKeyError> {
        let plain = self.peek_manifest(record)?;
        let assembled = omacus_engine::sync_objects::assemble(&plain, incoming, &[], &[])?;
        Ok((assembled.packed, assembled.slots))
    }
}

pub struct RecoveryPeeked {
    pub packed: Vec<u8>,
    pub partial: bool,
    pub legacy: bool,
    pub want: Vec<[u8; 16]>,
    /// The record had no signature. The device should publish a signed copy.
    pub resign: bool,
}

pub struct ApprovalPlan {
    pub packed: Vec<u8>,
    pub want: Vec<[u8; 16]>,
    pub legacy: bool,
}

pub struct LibraryOpened {
    pub packed: Vec<u8>,
    pub slots: Vec<omacus_engine::sync_objects::Slot>,
    pub legacy: bool,
    pub member: bool,
    pub replace: bool,
    pub republish: bool,
    pub combine: bool,
}

struct WrappedPages {
    vault: VaultKey,
    pages: Vec<u8>,
    recovery_public: Option<[u8; 32]>,
}

enum Opened {
    Current(Vec<u8>),
    Wrapped(WrappedPages),
}

type OpenBlob = fn(&VaultKey, u64, &[u8]) -> Result<Vec<u8>, SyncKeyError>;

fn open_wrapped(
    device: &DeviceKeys,
    view: &ServerView,
    open_blob: OpenBlob,
) -> Result<Option<WrappedPages>, SyncKeyError> {
    for wrap in &view.device_wraps {
        if let Ok(opened) =
            sync_keys::open_device_wrap_parts(device, wrap, view.epoch, &view.key_id)
        {
            let pages = open_blob(&opened.vault, view.epoch, &view.page)?;
            return Ok(Some(WrappedPages {
                vault: opened.vault,
                pages,
                recovery_public: opened.recovery_public,
            }));
        }
    }
    Ok(None)
}

/// The key the words can open, even when the record names a newer one.
fn phrase_vault(root: &Root, view: &ServerView) -> Option<VaultKey> {
    if let Some(id) = sync_keys::recovery_wrap_key_id(&view.phrase_wrap) {
        return sync_keys::open_recovery_wrap(root, &view.phrase_wrap, view.epoch, &id).ok();
    }
    sync_keys::open_phrase_wrap(root, &view.phrase_wrap, view.epoch, &view.key_id).ok()
}

fn open_manifest(vault: &VaultKey, epoch: u64, stored: &[u8]) -> Result<Vec<u8>, SyncKeyError> {
    open_manifest_at(vault, epoch, stored, 0)
}

fn open_manifest_at(
    vault: &VaultKey,
    epoch: u64,
    stored: &[u8],
    floor: u64,
) -> Result<Vec<u8>, SyncKeyError> {
    sync_keys::open_manifest_seal(
        vault,
        epoch,
        omacus_engine::sync_objects::MANIFEST_ID,
        stored,
        floor,
    )
}

fn open_sealed(vault: &VaultKey, epoch: u64, stored: &[u8]) -> Result<Vec<u8>, SyncKeyError> {
    match sync_keys::open_page(vault, epoch, PAGES_ID, stored) {
        Ok(pages) => Ok(pages),
        Err(_) => sync_keys::open_page(vault, epoch, PAGE_ID, stored),
    }
}

pub fn decode_hex_id(text: &str) -> Result<[u8; 16], SyncKeyError> {
    let text = text.trim();
    if text.len() != 32 {
        return Err(SyncKeyError::Corrupt);
    }
    let mut out = [0u8; 16];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| SyncKeyError::Corrupt)?;
    }
    Ok(out)
}

pub fn code_for_device(bytes: &[u8]) -> Result<String, SyncKeyError> {
    Ok(sync_keys::pairing_code(&decode_device(bytes)?))
}

pub fn encode_device(device: &DevicePublic) -> Vec<u8> {
    let mut out = Vec::with_capacity(80);
    out.extend_from_slice(device.id());
    out.extend_from_slice(device.signing_key());
    out.extend_from_slice(device.receive_key());
    out
}

pub fn decode_device(bytes: &[u8]) -> Result<DevicePublic, SyncKeyError> {
    if bytes.len() != 80 {
        return Err(SyncKeyError::Corrupt);
    }
    let mut id = [0u8; 16];
    let mut sign = [0u8; 32];
    let mut receive = [0u8; 32];
    id.copy_from_slice(&bytes[..16]);
    sign.copy_from_slice(&bytes[16..48]);
    receive.copy_from_slice(&bytes[48..]);
    Ok(DevicePublic::from_parts(id, sign, receive))
}

pub fn b64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn b64_decode(text: &str) -> Result<Vec<u8>, SyncKeyError> {
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|_| SyncKeyError::Corrupt)
}

pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

pub(crate) fn qr_svg(payload: &str, label: &str) -> Result<String, SyncKeyError> {
    let code = QrCode::new(payload.as_bytes()).map_err(|_| SyncKeyError::Unavailable)?;
    let modules = code.width();
    let quiet = 2;
    let size = modules + quiet * 2;
    let mut path = String::new();
    for y in 0..modules {
        let mut x = 0;
        while x < modules {
            if code[(x, y)] != qrcode::Color::Dark {
                x += 1;
                continue;
            }
            let start = x;
            while x < modules && code[(x, y)] == qrcode::Color::Dark {
                x += 1;
            }
            path.push_str(&format!(
                "M{} {}h{}v1h-{}z",
                start + quiet,
                y + quiet,
                x - start,
                x - start
            ));
        }
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" role=\"img\" aria-label=\"{label}\"><path fill=\"currentColor\" d=\"{path}\"/></svg>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_join_confirmed() -> Desk {
        let mut desk = Desk::start_join().unwrap();
        desk.confirm_link();
        desk
    }

    fn confirmed_desk() -> (Desk, Vec<u8>) {
        let mut desk = Desk::start_new().unwrap();
        let words = desk.words().unwrap();
        let picks: Vec<(u8, &str)> = CONFIRM_AT
            .into_iter()
            .map(|n| (n, words[n as usize - 1]))
            .collect();
        let record = desk.confirm_saved(&picks).unwrap();
        (desk, record)
    }

    #[test]
    fn a_reloaded_device_can_approve_a_new_one() {
        let (founder, first) = confirmed_desk();
        let file = founder.export_local().unwrap();
        let mut founder = Desk::import_local(&file).unwrap();
        assert!(
            founder.wraps.is_empty(),
            "a reload forgets the sealed copies"
        );
        let _ = founder.absorb_record(&first).unwrap();
        let joiner = start_join_confirmed();
        let device = joiner.public_device();
        let approval = founder.approve_new(&device).unwrap();
        let published = founder.seal_pages(b"Trip Budget\nferry = 40\n").unwrap();
        let mut joiner = joiner;
        joiner
            .open_approval_pages(&approval.chain, &approval.wrap, &published)
            .expect("reloaded device approval should open");
    }

    #[test]
    fn a_reloaded_device_approves_before_it_has_pulled() {
        let (founder, first) = confirmed_desk();
        let file = founder.export_local().unwrap();
        // Upgraded in place: the device file has the chain and the vault, and
        // nothing else. A slow reload has not pulled the mailbox yet.
        let mut founder = Desk::import_local(&file).unwrap();
        assert!(founder.wraps.is_empty());
        let mut joiner = start_join_confirmed();
        let approval = founder.approve_new(&joiner.public_device()).unwrap();
        let published = founder.seal_pages(b"Trip Budget\nferry = 40\n").unwrap();
        let err = joiner
            .open_approval_pages(&approval.chain, &approval.wrap, &published)
            .err();
        assert!(
            err.is_none(),
            "pulled-nothing approval failed: {}",
            err.unwrap().message()
        );
        // The copy still on the server from before this reload.
        joiner
            .open_approval_pages(&approval.chain, &approval.wrap, &first)
            .expect("old copy");
    }

    #[test]
    fn a_stale_mailbox_still_opens_after_approve() {
        let (founder, first) = confirmed_desk();
        let file = founder.export_local().unwrap();
        let mut founder = Desk::import_local(&file).unwrap();
        founder.absorb_record(&first).unwrap();
        let mut joiner = start_join_confirmed();
        let approval = founder.approve_new(&joiner.public_device()).unwrap();
        // A slow tick published nothing new: the joiner still sees the previous copy.
        joiner
            .open_approval_pages(&approval.chain, &approval.wrap, &first)
            .expect("previous copy should still open");
    }

    #[test]
    fn a_numbered_paste_looks_right_when_the_words_match() {
        let key = RecoveryKey::from_entropy([0x11; 16]);
        let words = key.words();
        let pasted = format!(
            "1. {}\n2. {}, {}",
            words[0],
            words[1],
            words[2..].join("\n")
        );
        let check = check_phrase(&pasted);
        assert!(check.ok, "{}", check.message);
        assert_eq!(check.message, "Looks right. 12 words.");
        assert_eq!(
            check_phrase("river apple").message,
            "That's 2 words. Recovery words are 12."
        );
        assert_eq!(
            check_phrase("one").message,
            "That's 1 word. Recovery words are 12."
        );
    }

    #[test]
    fn a_second_approve_needs_the_list_from_before_the_first_tap() {
        let (mut founder, _) = confirmed_desk();
        let file = founder.export_local().unwrap();
        let joiner = start_join_confirmed();
        let device = joiner.public_device();
        founder.approve_new(&device).unwrap();
        let Err(error) = founder.approve_new(&device) else {
            panic!("the phone was approved twice");
        };
        assert_eq!(error.message(), "That device is already on the list.");
        let mut restored = Desk::import_local(&file).unwrap();
        restored
            .approve_new(&device)
            .expect("the saved file can approve");
    }

    #[test]
    fn a_bad_approval_says_which_list_failed() {
        let (mut founder, record) = confirmed_desk();
        let mut joiner = start_join_confirmed();
        let approval = founder.approve_new(&joiner.public_device()).unwrap();
        let mut forged = approval.chain.clone();
        let last = forged.len() - 1;
        forged[last] ^= 0xff;
        let err = joiner
            .open_approval_pages(&forged, &approval.wrap, &record)
            .unwrap_err();
        assert_eq!(err.message(), "The approval's list didn't check out.");
        joiner
            .open_approval_pages(&approval.chain, &approval.wrap, &record)
            .expect("a real approval still opens");
    }

    #[test]
    fn the_list_shows_this_computer_and_leaves_a_removed_one_out() {
        let (mut founder, _) = confirmed_desk();
        let alone = founder.listed_devices().unwrap();
        assert_eq!(alone.len(), 1);
        assert!(alone[0].2);
        let self_id = alone[0].0.clone();

        let phone = start_join_confirmed();
        founder.approve_new(&phone.public_device()).unwrap();
        let two = founder.listed_devices().unwrap();
        assert_eq!(two.len(), 2);
        assert_eq!(two.iter().filter(|(_, _, mine)| *mine).count(), 1);
        assert!(two.iter().any(|(id, _, mine)| *mine && id == &self_id));
        let other = two.iter().find(|(_, _, mine)| !mine).unwrap().0.clone();

        let extra = start_join_confirmed();
        founder.approve_new(&extra.public_device()).unwrap();
        let three = founder.listed_devices().unwrap();
        assert_eq!(three.len(), 3);
        let extra_id = three
            .iter()
            .find(|(id, _, _)| id != &self_id && id != &other)
            .unwrap()
            .0
            .clone();
        founder
            .remove_listed(&decode_hex_id(&extra_id).unwrap())
            .unwrap();
        let left = founder.listed_devices().unwrap();
        assert_eq!(left.len(), 2);
        assert!(!left.iter().any(|(id, _, _)| id == &extra_id));
        assert!(left.iter().any(|(id, _, mine)| *mine && id == &self_id));
        assert_eq!(left.iter().filter(|(_, _, mine)| *mine).count(), 1);
    }

    fn trip_pages() -> Vec<u8> {
        omacus_engine::sync_keys::PageSet {
            doc: Vec::new(),
            pages: vec![("trip".into(), "ferry = 40\n".into())],
            meta: omacus_engine::sync_keys::SyncMeta::default(),
        }
        .to_bytes()
        .unwrap()
    }

    fn listed_ids(desk: &Desk) -> Vec<(String, bool)> {
        desk.listed_devices()
            .unwrap()
            .into_iter()
            .map(|(id, _, mine)| (id, mine))
            .collect()
    }

    /// Linking shows this computer and the other one, on both sides, including
    /// after the joiner opens the copy that was saved before the approval and
    /// after both phones reload.
    #[test]
    fn linking_a_second_device_lists_it_on_both_sides() {
        let (mut founder, _) = confirmed_desk();
        let before_plan = founder.seal_library(&trip_pages(), &[]).unwrap();
        let before = founder.library_record().unwrap();
        let before_objects: Vec<_> = before_plan
            .objects
            .iter()
            .map(|object| omacus_engine::sync_objects::Incoming {
                id: object.id,
                revision: 1,
                body: object.body.clone(),
            })
            .collect();
        let founder_id = listed_ids(&founder)[0].0.clone();
        assert!(listed_ids(&founder)[0].1);

        let mut joiner = start_join_confirmed();
        let approval = founder.approve_new(&joiner.public_device()).unwrap();
        let published_plan = founder
            .seal_library(&trip_pages(), &before_plan.slots)
            .unwrap();
        let published = founder.library_record().unwrap();
        let published_chain = sync_keys::ServerView::parse(&published)
            .unwrap()
            .chain
            .to_bytes();
        assert_eq!(
            published_chain,
            approval.chain,
            "the saved copy's list is not the approval list ({} vs {} bytes)",
            published_chain.len(),
            approval.chain.len()
        );
        let published_objects: Vec<_> = published_plan
            .objects
            .iter()
            .map(|object| omacus_engine::sync_objects::Incoming {
                id: object.id,
                revision: object.base.saturating_add(1),
                body: object.body.clone(),
            })
            .collect();
        assert_eq!(listed_ids(&founder).len(), 2, "the approver lost a phone");
        assert_eq!(
            listed_ids(&founder)
                .iter()
                .filter(|(_, mine)| *mine)
                .count(),
            1
        );

        let mut skipped = start_join_confirmed();
        let stale = sync_keys::ServerView::parse(&before)
            .unwrap()
            .chain
            .to_bytes();
        match skipped.approval_plan(&stale, &approval.wrap, &before) {
            Err(error) => assert_eq!(error.message(), sync_keys::NOT_ON_LIST),
            Ok(_) => panic!("an approval that leaves this phone off the list still opened"),
        }
        assert!(!skipped.member());

        joiner
            .approval_plan(&approval.chain, &approval.wrap, &before)
            .expect("the copy from before approval still opens");
        let joiner_rows = listed_ids(&joiner);
        assert_eq!(
            joiner_rows.len(),
            2,
            "the new phone's list after approval: {joiner_rows:?}"
        );
        assert_eq!(
            joiner_rows.iter().filter(|(_, mine)| *mine).count(),
            1,
            "the new phone is not on its own list: {joiner_rows:?}"
        );
        assert!(
            joiner_rows.iter().any(|(id, _)| id == &founder_id),
            "the first computer is missing on the new phone"
        );

        // A later sync still has the older copy. It must not drop this phone.
        joiner
            .open_library(&before, &before_objects, &[], &[])
            .expect("the older copy opens");
        let after_old = listed_ids(&joiner);
        assert_eq!(
            after_old.len(),
            2,
            "an older copy dropped a phone: {after_old:?}"
        );
        assert_eq!(after_old.iter().filter(|(_, mine)| *mine).count(), 1);

        founder
            .open_library(&published, &published_objects, &before_plan.slots, &[])
            .expect("the approver reopens the saved copy");
        assert_eq!(listed_ids(&founder).len(), 2);
        assert_eq!(listed_ids(&joiner).len(), 2);

        let founder = Desk::import_local(&founder.export_local().unwrap()).unwrap();
        let joiner = Desk::import_local(&joiner.export_local().unwrap()).unwrap();
        assert_eq!(
            listed_ids(&founder).len(),
            2,
            "reload dropped the approver's list"
        );
        assert_eq!(
            listed_ids(&joiner).len(),
            2,
            "reload dropped the new phone's list"
        );
        assert_eq!(
            listed_ids(&founder)
                .iter()
                .filter(|(_, mine)| *mine)
                .count(),
            1
        );
        assert_eq!(
            listed_ids(&joiner).iter().filter(|(_, mine)| *mine).count(),
            1
        );
        assert_ne!(
            listed_ids(&founder)
                .iter()
                .find(|(_, mine)| *mine)
                .unwrap()
                .0,
            listed_ids(&joiner)
                .iter()
                .find(|(_, mine)| *mine)
                .unwrap()
                .0
        );
    }

    #[test]
    fn opening_with_the_words_lists_this_computer_on_both_sides() {
        let mut founder = Desk::start_new().unwrap();
        let words = founder.words().unwrap();
        let phrase = words.join(" ");
        let picks: Vec<(u8, &str)> = CONFIRM_AT
            .into_iter()
            .map(|n| (n, words[n as usize - 1]))
            .collect();
        founder.confirm_saved(&picks).unwrap();
        let record = founder.seal_pages(b"ferry = 40\n").unwrap();
        let founder_id = listed_ids(&founder)[0].0.clone();
        assert_eq!(listed_ids(&founder).len(), 1);
        assert!(listed_ids(&founder)[0].1);

        let mut phone = Desk::start_recovery(&phrase).unwrap();
        phone.accept_phrase().unwrap();
        let opened = phone.recovery_open_pages(&record).unwrap();
        assert!(!opened.partial);
        assert_eq!(opened.pages, b"ferry = 40\n");
        let phone_rows = listed_ids(&phone);
        assert_eq!(
            phone_rows.len(),
            2,
            "the words left this computer off: {phone_rows:?}"
        );
        assert_eq!(phone_rows.iter().filter(|(_, mine)| *mine).count(), 1);
        assert!(
            phone_rows
                .iter()
                .any(|(id, mine)| id == &founder_id && !mine)
        );
        let phone_id = phone_rows.iter().find(|(_, mine)| *mine).unwrap().0.clone();

        let published = phone.seal_pages(b"ferry = 40\n").unwrap();
        founder.absorb_record(&published).unwrap();
        let founder_rows = listed_ids(&founder);
        assert_eq!(
            founder_rows.len(),
            2,
            "the first computer never saw the new one: {founder_rows:?}"
        );
        assert_eq!(founder_rows.iter().filter(|(_, mine)| *mine).count(), 1);
        assert!(
            founder_rows
                .iter()
                .any(|(id, mine)| id == &phone_id && !mine)
        );
        assert_ne!(
            founder_rows.iter().find(|(_, mine)| *mine).unwrap().0,
            phone_id
        );

        let plan = founder.seal_library(&trip_pages(), &[]).unwrap();
        let library = founder.library_record().unwrap();
        let mut second = Desk::start_recovery(&phrase).unwrap();
        second.accept_phrase().unwrap();
        let peeked = second.recovery_peek(&library).unwrap();
        assert!(!peeked.partial);
        assert!(!peeked.legacy);
        let second_rows = listed_ids(&second);
        assert_eq!(
            second_rows.len(),
            3,
            "a current copy left this computer off: {second_rows:?}"
        );
        assert_eq!(second_rows.iter().filter(|(_, mine)| *mine).count(), 1);
        assert!(second_rows.iter().any(|(id, _)| id == &founder_id));
        assert!(second_rows.iter().any(|(id, _)| id == &phone_id));
        let _ = plan;
    }

    fn other_id(desk: &Desk) -> [u8; 16] {
        let listed = desk.listed_devices().unwrap();
        let (id, _, mine) = listed.iter().find(|(_, _, mine)| !mine).unwrap();
        assert!(!mine);
        decode_hex_id(id).unwrap()
    }

    #[test]
    fn a_removed_device_cannot_open_pages_sealed_afterward() {
        let mut founder = Desk::start_new().unwrap();
        let words = founder.words().unwrap();
        let phrase = words.join(" ");
        let picks: Vec<(u8, &str)> = CONFIRM_AT
            .into_iter()
            .map(|n| (n, words[n as usize - 1]))
            .collect();
        founder.confirm_saved(&picks).unwrap();

        let mut laptop = start_join_confirmed();
        let approval = founder.approve_new(&laptop.public_device()).unwrap();
        let first = founder.seal_pages(b"ferry = 40\n").unwrap();
        laptop
            .open_approval_pages(&approval.chain, &approval.wrap, &first)
            .unwrap();

        let mut phone = start_join_confirmed();
        let approval = founder.approve_new(&phone.public_device()).unwrap();
        let shared = founder.seal_pages(b"ferry = 40\n").unwrap();
        phone
            .open_approval_pages(&approval.chain, &approval.wrap, &shared)
            .unwrap();

        let phone_id = *phone.device().public().id();
        founder.remove_listed(&phone_id).unwrap();
        let later = founder.seal_pages(b"bread $2\n").unwrap();

        let kept = laptop.absorb_record(&later).unwrap();
        assert!(kept.member);
        assert!(kept.replace);
        assert!(!kept.combine);
        assert_eq!(kept.pages, b"bread $2\n");

        let dropped = phone.absorb_record(&later).unwrap();
        assert!(!dropped.member);
        assert!(!dropped.replace);
        assert!(phone.formerly_listed());
        let view = ServerView::parse(&later).unwrap();
        assert!(view.device_wraps.iter().all(|wrap| {
            sync_keys::open_device_wrap(phone.device(), wrap, view.epoch, &view.key_id).is_err()
        }));

        let mut recovered = Desk::start_recovery(&phrase).unwrap();
        recovered.recovery_account(&picks).unwrap();
        let opened = recovered.recovery_open_pages(&later).unwrap();
        assert!(!opened.partial);
        assert_eq!(opened.pages, b"bread $2\n");
        assert!(recovered.member());
        assert!(!recovered.formerly_listed());
        assert!(
            recovered
                .listed_devices()
                .unwrap()
                .iter()
                .any(|(_, _, mine)| *mine)
        );
    }

    #[test]
    fn a_beta1_file_can_approve_and_removal_still_locks_new_pages() {
        let mut founder = Desk::start_new().unwrap();
        let words = founder.words().unwrap();
        let phrase = words.join(" ");
        let picks: Vec<(u8, &str)> = CONFIRM_AT
            .into_iter()
            .map(|n| (n, words[n as usize - 1]))
            .collect();
        let first = founder.confirm_saved(&picks).unwrap();
        let file = founder.export_local().unwrap();
        let stored = sync_keys::open_device_store(&file).unwrap();
        let mut kek = [0u8; 32];
        kek.copy_from_slice(&stored.kek);
        let view = ServerView::parse(&first).unwrap();
        let mut wraps = Vec::new();
        for device in view.chain.devices().unwrap() {
            wraps.push(
                sync_keys::wrap_vault_for_device(
                    &device,
                    founder.vault().unwrap(),
                    view.epoch,
                    None,
                )
                .unwrap(),
            );
        }
        let legacy_phrase =
            sync_keys::wrap_vault_with_kek(&kek, founder.vault().unwrap(), view.epoch).unwrap();
        let legacy_record = ServerView {
            epoch: view.epoch,
            account: view.account,
            chain: view.chain,
            phrase_wrap: legacy_phrase,
            device_wraps: wraps,
            key_id: view.key_id,
            page: view.page,
            version: 1,
            signer: None,
            signature: None,
        }
        .stored();
        let legacy = sync_keys::store_device(
            &stored.device,
            &stored.vault,
            &stored.account,
            stored.epoch,
            &stored.chain,
            &[],
            &[],
            false,
            0,
        )
        .unwrap();
        let mut founder = Desk::import_local(&legacy).unwrap();
        founder.absorb_record(&legacy_record).unwrap();

        let mut phone = start_join_confirmed();
        let approval = founder.approve_new(&phone.public_device()).unwrap();
        let shared = founder.seal_pages(b"ferry = 40\n").unwrap();
        phone
            .open_approval_pages(&approval.chain, &approval.wrap, &shared)
            .expect("beta.1 device file can approve");

        founder.remove_listed(&other_id(&founder)).unwrap();
        let later = founder.seal_pages(b"sesame = 12 g\n").unwrap();
        let dropped = phone.absorb_record(&later).unwrap();
        assert!(!dropped.member && !dropped.replace);
        let mut recovered = Desk::start_recovery(&phrase).unwrap();
        recovered.accept_phrase().unwrap();
        let opened = recovered.recovery_open_pages(&later).unwrap();
        assert!(opened.partial);
        assert!(opened.pages.is_empty());
        let restored = recovered.reestablish_pages(b"lentils = 180 g\n").unwrap();
        let root = RecoveryKey::parse(&phrase)
            .unwrap()
            .accept()
            .unwrap()
            .root("")
            .unwrap();
        let secret = sync_keys::recovery_secret(&root).unwrap();
        assert!(
            !restored
                .windows(secret.len())
                .any(|window| window == secret),
            "the fresh seal holds the recovery secret"
        );
        if let Ok(path) = std::env::var("OMACUS_STALE_FIXTURE") {
            let record = b64_encode(&later);
            let account = founder.account_hex().unwrap();
            std::fs::write(
                path,
                format!(
                    "{{\"words\":\"{phrase}\",\"account\":\"{account}\",\"record\":\"{record}\"}}\n"
                ),
            )
            .unwrap();
        }
        let mut again = Desk::start_recovery(&phrase).unwrap();
        again.accept_phrase().unwrap();
        let second = again.recovery_open_pages(&restored).unwrap();
        assert!(!second.partial);
        assert_eq!(second.pages, b"lentils = 180 g\n");
        let brought = founder.absorb_record(&restored).unwrap();
        assert!(brought.member);
        assert!(brought.replace);
        assert!(brought.combine);
        assert_eq!(brought.pages, b"lentils = 180 g\n");
        let still_out = phone.absorb_record(&restored).unwrap();
        assert!(!still_out.member && !still_out.replace);
    }

    #[test]
    fn recovery_opens_an_older_seal_when_the_newest_key_is_only_named() {
        let mut founder = Desk::start_new().unwrap();
        let words = founder.words().unwrap();
        let phrase = words.join(" ");
        let picks: Vec<(u8, &str)> = CONFIRM_AT
            .into_iter()
            .map(|n| (n, words[n as usize - 1]))
            .collect();
        let first = founder.confirm_saved(&picks).unwrap();
        let old = ServerView::parse(&first).unwrap();
        let decoy = VaultKey::generate().unwrap();
        let stale = ServerView {
            epoch: old.epoch,
            account: old.account,
            chain: old.chain,
            phrase_wrap: old.phrase_wrap,
            device_wraps: old.device_wraps,
            key_id: *decoy.id(),
            page: old.page,
            version: 1,
            signer: None,
            signature: None,
        }
        .stored();
        let mut recovered = Desk::start_recovery(&phrase).unwrap();
        recovered.accept_phrase().unwrap();
        let opened = recovered.recovery_open_pages(&stale).unwrap();
        assert!(opened.partial);
        assert_eq!(opened.pages, SAMPLE_PAGE.as_bytes());
        let restored = recovered.reestablish_pages(b"lentils = 180 g\n").unwrap();
        let mut again = Desk::start_recovery(&phrase).unwrap();
        again.accept_phrase().unwrap();
        let second = again.recovery_open_pages(&restored).unwrap();
        assert!(!second.partial);
        assert_eq!(second.pages, b"lentils = 180 g\n");
    }

    #[test]
    fn recovery_opens_the_latest_pages_after_several_removals() {
        let mut founder = Desk::start_new().unwrap();
        let words = founder.words().unwrap();
        let phrase = words.join(" ");
        let picks: Vec<(u8, &str)> = CONFIRM_AT
            .into_iter()
            .map(|n| (n, words[n as usize - 1]))
            .collect();
        founder.confirm_saved(&picks).unwrap();

        let mut laptop = start_join_confirmed();
        let approval = founder.approve_new(&laptop.public_device()).unwrap();
        let shared = founder.seal_pages(b"ferry = 40\n").unwrap();
        laptop
            .open_approval_pages(&approval.chain, &approval.wrap, &shared)
            .unwrap();
        let stored = sync_keys::open_device_store(&laptop.export_local().unwrap()).unwrap();
        assert!(
            stored.kek.is_empty(),
            "a joined phone does not keep the wrapping secret"
        );
        assert_eq!(stored.recovery_public.len(), 32);

        let mut phone = start_join_confirmed();
        let approval = laptop.approve_new(&phone.public_device()).unwrap();
        let shared = laptop.seal_pages(b"ferry = 40\n").unwrap();
        phone
            .open_approval_pages(&approval.chain, &approval.wrap, &shared)
            .unwrap();

        let mut tablet = start_join_confirmed();
        let approval = laptop.approve_new(&tablet.public_device()).unwrap();
        let shared = laptop.seal_pages(b"milk $3\n").unwrap();
        tablet
            .open_approval_pages(&approval.chain, &approval.wrap, &shared)
            .unwrap();

        let phone_id = *phone.device().public().id();
        laptop.remove_listed(&phone_id).unwrap();
        let after_phone = laptop.seal_pages(b"milk $3\n").unwrap();
        assert!(founder.absorb_record(&after_phone).unwrap().member);
        assert!(tablet.absorb_record(&after_phone).unwrap().member);
        assert!(!phone.absorb_record(&after_phone).unwrap().member);

        let tablet_id = *tablet.device().public().id();
        laptop.remove_listed(&tablet_id).unwrap();
        let after_tablet = laptop.seal_pages(b"bread $2\n").unwrap();
        assert!(founder.absorb_record(&after_tablet).unwrap().member);

        let mut extra = start_join_confirmed();
        let approval = founder.approve_new(&extra.public_device()).unwrap();
        let shared = founder.seal_pages(b"bread $2\n").unwrap();
        extra
            .open_approval_pages(&approval.chain, &approval.wrap, &shared)
            .unwrap();
        let extra_id = *extra.device().public().id();
        founder.remove_listed(&extra_id).unwrap();
        let latest = founder.seal_pages(b"bread $2\npark = 2\n").unwrap();
        let dropped = phone.absorb_record(&latest).unwrap();
        assert!(!dropped.member && !dropped.replace);

        let view = ServerView::parse(&latest).unwrap();
        let root = RecoveryKey::parse(&phrase)
            .unwrap()
            .accept()
            .unwrap()
            .root("")
            .unwrap();
        let secret = sync_keys::recovery_secret(&root).unwrap();
        assert!(
            !view
                .stored()
                .windows(secret.len())
                .any(|window| window == secret),
            "the server copy holds the recovery secret"
        );

        let mut recovered = Desk::start_recovery(&phrase).unwrap();
        recovered.accept_phrase().unwrap();
        let opened = recovered.recovery_open_pages(&latest).unwrap();
        assert!(!opened.partial);
        assert_eq!(opened.pages, b"bread $2\npark = 2\n");
    }

    #[test]
    fn a_new_key_on_the_same_list_is_refused() {
        let mut founder = Desk::start_new().unwrap();
        let words = founder.words().unwrap();
        let phrase = words.join(" ");
        let picks: Vec<(u8, &str)> = CONFIRM_AT
            .into_iter()
            .map(|n| (n, words[n as usize - 1]))
            .collect();
        let first = founder.confirm_saved(&picks).unwrap();
        let exported = founder.export_local().unwrap();
        let stored = sync_keys::open_device_store(&exported).unwrap();
        let mut kek = [0u8; 32];
        kek.copy_from_slice(&stored.kek);
        let file = sync_keys::store_device(
            &stored.device,
            &stored.vault,
            &stored.account,
            stored.epoch,
            &stored.chain,
            &kek,
            &[],
            false,
            0,
        )
        .unwrap();
        let mut legacy = Desk::import_local(&file).unwrap();
        let old = ServerView::parse(&first).unwrap();
        let new_vault = VaultKey::generate().unwrap();
        let mut wraps = Vec::new();
        for device in old.chain.devices().unwrap() {
            wraps.push(
                sync_keys::wrap_vault_for_device(&device, &new_vault, old.epoch, None).unwrap(),
            );
        }
        let page = sync_keys::seal_page(&new_vault, old.epoch, PAGES_ID, b"bread $2\n").unwrap();
        let stale = ServerView {
            epoch: old.epoch,
            account: old.account,
            chain: old.chain,
            phrase_wrap: old.phrase_wrap,
            device_wraps: wraps,
            key_id: *new_vault.id(),
            page,
            version: old.version,
            signer: None,
            signature: None,
        }
        .stored();
        let kept = *legacy.vault().unwrap().id();
        let err = match legacy.absorb_record(&stale) {
            Err(error) => error,
            Ok(_) => panic!("a new key on the same list was adopted"),
        };
        assert_eq!(err, SyncKeyError::Sealed);
        assert_eq!(legacy.vault().unwrap().id(), &kept);

        let mut recovered = Desk::start_recovery(&phrase).unwrap();
        recovered.accept_phrase().unwrap();
        let opened = recovered.recovery_open_pages(&first).unwrap();
        assert!(!opened.partial);
        assert_eq!(opened.pages, SAMPLE_PAGE.as_bytes());
    }

    #[test]
    fn a_server_minted_fork_with_a_valid_wrap_is_refused() {
        let (mut founder, _) = confirmed_desk();
        let account = founder.account.unwrap();
        let epoch = founder.epoch;
        let vault_before = *founder.vault().unwrap().id();
        let listed = founder.listed_devices().unwrap();
        let stranger = DeviceKeys::generate().unwrap();
        let chain = sync_keys::found_devices(&stranger).unwrap();
        let vault = VaultKey::generate().unwrap();
        let wrap =
            sync_keys::wrap_vault_for_device(&founder.device().public(), &vault, epoch, None)
                .unwrap();
        let page = sync_keys::seal_page(&vault, epoch, PAGES_ID, b"forged lentils\n").unwrap();
        let view = ServerView {
            epoch,
            account,
            chain,
            phrase_wrap: Vec::new(),
            device_wraps: vec![wrap],
            key_id: *vault.id(),
            page,
            version: 1,
            signer: None,
            signature: None,
        };
        let record = sync_keys::append_record_signature(
            &view.to_bytes(1),
            stranger.public().id(),
            stranger.signing_secret(),
        );
        let parsed = ServerView::parse(&record).unwrap();
        assert!(
            sync_keys::trust_signature(&parsed, true).is_ok(),
            "the forged list's own signature checks out"
        );
        let err = match founder.absorb_record(&record) {
            Err(error) => error,
            Ok(_) => panic!("a server-minted fork was adopted"),
        };
        assert_eq!(err, SyncKeyError::Sealed);
        assert_eq!(founder.vault().unwrap().id(), &vault_before);
        assert_eq!(founder.listed_devices().unwrap(), listed);
        assert!(founder.member());
    }

    #[test]
    fn a_first_link_waits_until_the_numbers_are_checked() {
        let (mut founder, _) = confirmed_desk();
        let mut joiner = Desk::start_join().unwrap();
        let approval = founder.approve_new(&joiner.public_device()).unwrap();
        let record = founder.seal_pages(b"lentils = 180 g\n").unwrap();
        let err = joiner
            .open_approval_pages(&approval.chain, &approval.wrap, &record)
            .unwrap_err();
        assert_eq!(err.message(), sync_keys::CONFIRM_LINK);
        assert!(joiner.vault().is_none());
        assert!(!joiner.member());
        joiner.confirm_link();
        joiner
            .open_approval_pages(&approval.chain, &approval.wrap, &record)
            .unwrap();
        assert!(joiner.member());
        assert_eq!(joiner.listed_devices().unwrap().len(), 2);
    }

    #[test]
    fn an_old_sealed_copy_splits_into_one_object_per_page() {
        let (mut desk, _) = confirmed_desk();
        let set = omacus_engine::sync_keys::PageSet {
            doc: Vec::new(),
            pages: vec![
                ("pantry".into(), "oats\n".into()),
                ("groceries".into(), "lentils\n".into()),
            ],
            meta: omacus_engine::sync_keys::SyncMeta::default(),
        };
        let packed = set.to_bytes().unwrap();
        let legacy = desk.seal_pages(&packed).unwrap();
        let opened = desk.open_library(&legacy, &[], &[], &[]).unwrap();
        assert!(opened.legacy);
        let planned = desk.seal_library(&opened.packed, &[]).unwrap();
        assert_eq!(planned.objects.len(), 2);
        let record = desk.library_record().unwrap();
        assert!(omacus_engine::sync_objects::is_v2(&record));
        let incoming: Vec<_> = planned
            .objects
            .iter()
            .map(|object| omacus_engine::sync_objects::Incoming {
                id: object.id,
                revision: 1,
                body: object.body.clone(),
            })
            .collect();
        let again = desk.open_library(&record, &incoming, &[], &[]).unwrap();
        assert!(!again.legacy);
        let round = omacus_engine::sync_keys::PageSet::from_bytes(&again.packed).unwrap();
        assert_eq!(round.pages.len(), 2);
        let ids: std::collections::HashSet<_> =
            round.pages.iter().map(|(id, _)| id.clone()).collect();
        assert_eq!(ids.len(), 2);
        assert!(round.pages.iter().any(|(_, text)| text.contains("lentils")));
        let kept = desk.seal_library(&again.packed, &again.slots).unwrap();
        assert!(
            kept.objects.is_empty(),
            "an unchanged page is not uploaded again"
        );
    }

    #[test]
    fn a_forged_record_is_refused_and_an_unsigned_copy_still_opens() {
        let (founder, signed) = confirmed_desk();
        assert!(signed.len() > 80);
        let file = founder.export_local().unwrap();
        let stored = sync_keys::open_device_store(&file).unwrap();
        let old_file = sync_keys::store_device(
            &stored.device,
            &stored.vault,
            &stored.account,
            stored.epoch,
            &stored.chain,
            &stored.kek,
            &stored.recovery_public,
            false,
            0,
        )
        .unwrap();
        let mut old = Desk::import_local(&old_file).unwrap();
        let unsigned = &signed[..signed.len() - 80];
        let migrated = old.absorb_record(unsigned).unwrap();
        assert!(migrated.member);
        assert!(
            migrated.republish,
            "an older copy is published with a signature"
        );
        let opened = old.absorb_record(&signed).unwrap();
        assert!(opened.member);
        assert_eq!(opened.pages, SAMPLE_PAGE.as_bytes());
        assert!(matches!(
            old.absorb_record(unsigned),
            Err(SyncKeyError::Sealed)
        ));

        let mut flipped = signed.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0xff;
        assert!(matches!(
            old.absorb_record(&flipped),
            Err(SyncKeyError::Sealed)
        ));
        let mut body = signed.clone();
        let inside = body.len() - 81;
        body[inside] ^= 0xff;
        assert!(matches!(
            old.absorb_record(&body),
            Err(SyncKeyError::Sealed)
        ));

        let stranger = DeviceKeys::generate().unwrap();
        let forged = sync_keys::append_record_signature(
            unsigned,
            stranger.public().id(),
            stranger.signing_secret(),
        );
        assert!(matches!(
            old.absorb_record(&forged),
            Err(SyncKeyError::Sealed)
        ));
    }

    #[test]
    fn a_beta_account_opens_once_and_seals_one_object_per_page() {
        let (mut founder, _) = confirmed_desk();
        let set = omacus_engine::sync_keys::PageSet {
            doc: Vec::new(),
            pages: vec![
                ("groceries".into(), "lentils\n".into()),
                ("pantry".into(), "oats\n".into()),
            ],
            meta: omacus_engine::sync_keys::SyncMeta::default(),
        };
        let packed = set.to_bytes().unwrap();
        let signed = founder.seal_pages(&packed).unwrap();
        assert!(signed.starts_with(b"OMACUS-SYNC/1\n"));
        assert!(!omacus_engine::sync_objects::is_v2(&signed));
        let unsigned = &signed[..signed.len() - 80];

        let file = founder.export_local().unwrap();
        let stored = sync_keys::open_device_store(&file).unwrap();
        let old_file = sync_keys::store_device(
            &stored.device,
            &stored.vault,
            &stored.account,
            stored.epoch,
            &stored.chain,
            &stored.kek,
            &stored.recovery_public,
            false,
            0,
        )
        .unwrap();
        let mut old = Desk::import_local(&old_file).unwrap();
        let opened = old.open_library(unsigned, &[], &[], &[]).unwrap();
        assert!(opened.legacy, "a version 1 copy is still one sealed blob");
        assert!(opened.republish, "the phone publishes the current format");
        assert!(opened.member);

        let planned = old.seal_library(&opened.packed, &[]).unwrap();
        assert_eq!(planned.objects.len(), 2, "each page is its own object");
        let record = old.library_record().unwrap();
        assert!(omacus_engine::sync_objects::is_v2(&record));
        assert!(record.len() > unsigned.len());
        assert!(matches!(
            old.open_library(unsigned, &[], &[], &[]),
            Err(SyncKeyError::Sealed)
        ));

        let incoming: Vec<_> = planned
            .objects
            .iter()
            .map(|object| omacus_engine::sync_objects::Incoming {
                id: object.id,
                revision: 1,
                body: object.body.clone(),
            })
            .collect();
        let again = old.open_library(&record, &incoming, &[], &[]).unwrap();
        assert!(!again.legacy);
        let round = omacus_engine::sync_keys::PageSet::from_bytes(&again.packed).unwrap();
        assert_eq!(round.pages.len(), 2);
        assert!(round.pages.iter().any(|(_, text)| text.contains("lentils")));
        assert!(round.pages.iter().any(|(_, text)| text.contains("oats")));
    }

    #[test]
    fn an_older_seal_is_refused_after_a_newer_one_is_saved() {
        let (mut desk, _) = confirmed_desk();
        let set = omacus_engine::sync_keys::PageSet {
            doc: Vec::new(),
            pages: vec![
                ("groceries".into(), "lentils\n".into()),
                ("pantry".into(), "oats\n".into()),
            ],
            meta: omacus_engine::sync_keys::SyncMeta::default(),
        };
        let legacy = desk.seal_pages(&set.to_bytes().unwrap()).unwrap();
        let opened = desk.open_library(&legacy, &[], &[], &[]).unwrap();
        let first = desk.seal_library(&opened.packed, &[]).unwrap();
        let first_record = desk.library_record().unwrap();
        let mut edited = omacus_engine::sync_keys::PageSet::from_bytes(&opened.packed).unwrap();
        for page in &mut edited.pages {
            if page.1.contains("oats") {
                page.1 = "oats\nsesame\n".into();
            }
        }
        let second = desk
            .seal_library(&edited.to_bytes().unwrap(), &first.slots)
            .unwrap();
        let second_record = desk.library_record().unwrap();
        let newer: Vec<_> = second
            .objects
            .iter()
            .map(|object| omacus_engine::sync_objects::Incoming {
                id: object.id,
                revision: object.base.saturating_add(1),
                body: object.body.clone(),
            })
            .collect();
        let opened = desk
            .open_library(&second_record, &newer, &first.slots, &[])
            .unwrap();
        let round = omacus_engine::sync_keys::PageSet::from_bytes(&opened.packed).unwrap();
        assert!(round.pages.iter().any(|(_, text)| text.contains("sesame")));
        assert!(matches!(
            desk.open_library(&first_record, &[], &opened.slots, &[]),
            Err(SyncKeyError::Older)
        ));

        let mut restored = Desk::import_local(&desk.export_local().unwrap()).unwrap();
        assert!(matches!(
            restored.open_library(&first_record, &[], &[], &[]),
            Err(SyncKeyError::Older)
        ));
        let mut both = newer;
        for object in &first.objects {
            if both.iter().any(|item| item.id == object.id) {
                continue;
            }
            both.push(omacus_engine::sync_objects::Incoming {
                id: object.id,
                revision: 1,
                body: object.body.clone(),
            });
        }
        let still = restored
            .open_library(&second_record, &both, &[], &[])
            .unwrap();
        let kept = omacus_engine::sync_keys::PageSet::from_bytes(&still.packed).unwrap();
        assert!(kept.pages.iter().any(|(_, text)| text.contains("sesame")));
        assert!(kept.pages.iter().any(|(_, text)| text.contains("lentils")));
    }
}
