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
}

pub struct Approval {
    pub chain: Vec<u8>,
    pub wrap: Vec<u8>,
    pub record: Vec<u8>,
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
            "That's 1 word. A recovery key is 12.".to_string()
        } else {
            format!("That's {count} words. A recovery key is 12.")
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
        qr_svg(&payload)
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
        let (account, phrase, vault) = {
            let root = confirmed.root("")?;
            let account = root.account_public()?;
            let vault = VaultKey::generate()?;
            let phrase = sync_keys::wrap_vault_for_phrase(&root, &vault, self.epoch)?;
            (account, phrase, vault)
        };
        let chain = sync_keys::found_devices(&self.device)?;
        let wrap = sync_keys::wrap_vault_for_device(&self.device.public(), &vault, self.epoch)?;
        let sealed = sync_keys::seal_page(&vault, self.epoch, PAGE_ID, SAMPLE_PAGE.as_bytes())?;
        self.account = Some(account);
        self.vault = Some(vault);
        self.chain = Some(chain);
        self.phrase_wrap = phrase;
        self.wraps = vec![wrap];
        self.sealed_page = sealed;
        self.record()
    }

    pub fn account_hex(&self) -> Option<String> {
        self.account.map(|account| hex(&account))
    }

    pub fn pairing_code(&self) -> String {
        sync_keys::pairing_code(&self.device.public())
    }

    pub fn public_device(&self) -> Vec<u8> {
        encode_device(&self.device.public())
    }

    pub fn approve_new(&mut self, device: &[u8]) -> Result<Approval, SyncKeyError> {
        let newcomer = decode_device(device)?;
        let chain = self.chain.as_ref().ok_or(SyncKeyError::NotApproved)?;
        let vault = self.vault.as_ref().ok_or(SyncKeyError::Sealed)?;
        let (chain, wrap) = sync_keys::approve(&self.device, chain, &newcomer, vault, self.epoch)?;
        self.chain = Some(chain);
        self.wraps.push(wrap.clone());
        let record = self.record()?;
        let chain_bytes = self.chain.as_ref().ok_or(SyncKeyError::Corrupt)?.to_bytes();
        Ok(Approval {
            chain: chain_bytes,
            wrap,
            record,
        })
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

    pub fn recovery_open(&mut self, record: &[u8]) -> Result<String, SyncKeyError> {
        let page = self.unlock_phrase(record)?;
        String::from_utf8(page).map_err(|_| SyncKeyError::Corrupt)
    }

    /// Same as [`Desk::recovery_open`], returning the sealed pages as bytes.
    pub fn recovery_open_pages(&mut self, record: &[u8]) -> Result<Vec<u8>, SyncKeyError> {
        self.unlock_phrase(record)
    }

    pub fn export_local(&self) -> Result<Vec<u8>, SyncKeyError> {
        let vault = self.vault.as_ref().ok_or(SyncKeyError::Sealed)?;
        let account = self.account.ok_or(SyncKeyError::Corrupt)?;
        let chain = self
            .chain
            .as_ref()
            .map(DeviceChain::to_bytes)
            .unwrap_or_default();
        sync_keys::store_device(&self.device, vault, &account, self.epoch, &chain)
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
        })
    }

    pub fn seal_pages(&mut self, packed: &[u8]) -> Result<Vec<u8>, SyncKeyError> {
        let vault = self.vault.as_ref().ok_or(SyncKeyError::Sealed)?;
        self.sealed_page = sync_keys::seal_page(vault, self.epoch, PAGES_ID, packed)?;
        self.record()
    }

    pub fn read_pages(&self) -> Result<Vec<u8>, SyncKeyError> {
        let vault = self.vault.as_ref().ok_or(SyncKeyError::Sealed)?;
        open_sealed(vault, self.epoch, &self.sealed_page)
    }

    /// Open a newer record. A chain that drops a removal we already have is not adopted.
    /// The opened pages come back either way, so a merge can keep every page.
    pub fn absorb_record(&mut self, record: &[u8]) -> Result<(Vec<u8>, bool), SyncKeyError> {
        let view = ServerView::parse(record)?;
        view.chain.verify()?;
        if let Some(account) = self.account
            && account != view.account
        {
            return Err(SyncKeyError::Sealed);
        }
        let opened = if let Some(vault) = &self.vault {
            if vault.id() != &view.key_id {
                return Err(SyncKeyError::Sealed);
            }
            open_sealed(vault, view.epoch, &view.page)?
        } else {
            let mut found = None;
            for wrap in &view.device_wraps {
                if let Ok(vault) =
                    sync_keys::open_device_wrap(&self.device, wrap, view.epoch, &view.key_id)
                {
                    let pages = open_sealed(&vault, view.epoch, &view.page)?;
                    found = Some((vault, pages));
                    break;
                }
            }
            let (vault, pages) = found.ok_or(SyncKeyError::Sealed)?;
            self.vault = Some(vault);
            pages
        };
        let take_chain = match &self.chain {
            None => true,
            Some(local) => {
                matches!(
                    view.chain.compared_with(local),
                    sync_keys::ChainOrder::Newer | sync_keys::ChainOrder::Same
                )
            }
        };
        if take_chain {
            self.chain = Some(view.chain);
            self.phrase_wrap = view.phrase_wrap;
            self.wraps = view.device_wraps;
            self.sealed_page = view.page;
            self.epoch = view.epoch;
        }
        self.account = Some(view.account);
        Ok((opened, self.member()))
    }

    pub fn member(&self) -> bool {
        let mine = *self.device.public().id();
        self.chain
            .as_ref()
            .and_then(|chain| chain.devices().ok())
            .is_some_and(|devices| devices.iter().any(|device| *device.id() == mine))
    }

    pub fn listed_devices(&self) -> Result<Vec<(String, String, bool)>, SyncKeyError> {
        let chain = self.chain.as_ref().ok_or(SyncKeyError::NotApproved)?;
        let mine = *self.device.public().id();
        let mut rows = Vec::new();
        for device in chain.devices()? {
            let code = sync_keys::pairing_code(&device);
            rows.push((hex(device.id()), code, *device.id() == mine));
        }
        Ok(rows)
    }

    pub fn remove_listed(&mut self, id: &[u8; 16]) -> Result<Vec<u8>, SyncKeyError> {
        let chain = self.chain.clone().ok_or(SyncKeyError::NotApproved)?;
        self.chain = Some(sync_keys::remove_device(&self.device, &chain, id)?);
        self.record()
    }

    fn unlock_approval(
        &mut self,
        chain: &[u8],
        wrap: &[u8],
        record: &[u8],
    ) -> Result<Vec<u8>, SyncKeyError> {
        let chain = DeviceChain::from_bytes(chain)?;
        chain.verify()?;
        let view = ServerView::parse(record)?;
        view.chain.verify()?;
        let vault = sync_keys::open_device_wrap(&self.device, wrap, view.epoch, &view.key_id)?;
        let pages = open_sealed(&vault, view.epoch, &view.page)?;
        self.keep_opened(view, vault, chain);
        Ok(pages)
    }

    fn unlock_phrase(&mut self, record: &[u8]) -> Result<Vec<u8>, SyncKeyError> {
        let view = ServerView::parse(record)?;
        if self.account != Some(view.account) {
            return Err(SyncKeyError::Sealed);
        }
        let root = self.root.take().ok_or(SyncKeyError::NotConfirmed)?;
        view.chain.verify()?;
        let vault =
            sync_keys::open_phrase_wrap(&root, &view.phrase_wrap, view.epoch, &view.key_id)?;
        let pages = open_sealed(&vault, view.epoch, &view.page)?;
        let chain = view.chain.clone();
        self.keep_opened(view, vault, chain);
        Ok(pages)
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

    fn keep_opened(&mut self, view: ServerView, vault: VaultKey, chain: DeviceChain) {
        self.epoch = view.epoch;
        self.account = Some(view.account);
        self.phrase_wrap = view.phrase_wrap;
        self.wraps = view.device_wraps;
        self.sealed_page = view.page;
        self.chain = Some(chain);
        self.vault = Some(vault);
    }

    fn record(&self) -> Result<Vec<u8>, SyncKeyError> {
        let view = ServerView {
            epoch: self.epoch,
            account: self.account.ok_or(SyncKeyError::Corrupt)?,
            chain: self.chain.clone().ok_or(SyncKeyError::Corrupt)?,
            phrase_wrap: self.phrase_wrap.clone(),
            device_wraps: self.wraps.clone(),
            key_id: *self.vault.as_ref().ok_or(SyncKeyError::Corrupt)?.id(),
            page: self.sealed_page.clone(),
        };
        Ok(view.stored())
    }
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

fn qr_svg(payload: &str) -> Result<String, SyncKeyError> {
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
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" role=\"img\" aria-label=\"Recovery sheet\"><path fill=\"currentColor\" d=\"{path}\"/></svg>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "That's 2 words. A recovery key is 12."
        );
        assert_eq!(
            check_phrase("one").message,
            "That's 1 word. A recovery key is 12."
        );
    }
}
