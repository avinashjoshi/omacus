//! Keys for syncing pages between devices.
//!
//! Nothing in this module is stored on a server. A recovery key is 12 everyday
//! words. They stand for 128 random bits and a checksum. The root is Argon2id
//! over those bits and an optional extra word, with the salt `omacus-v1`, and
//! it is wiped after the device keys exist. Each device keeps its own signing
//! key and its own receive key. A vault key is sealed to a device with HPKE
//! (RFC 9180, base mode) and, separately, to the recovery key. A signed device
//! list says who may receive a seal. The signature covers the list, not the
//! vault key.
//!
//! The published app does not call this. The private test keeps a device file
//! on that device and sends only a sealed record.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce, XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The vault these keys open. The same id the page library uses.
pub const VAULT_ID: &str = "personal";

/// Protocol version mixed into every seal.
pub const PROTOCOL: u16 = 1;

const WORD_COUNT: usize = 12;
const CONFIRM_COUNT: usize = 3;
const MEMORY_KIB: u32 = 19 * 1024;
const ITERATIONS: u32 = 2;
const LANES: u32 = 1;
const ROOT_SALT: &[u8] = b"omacus-v1";

/// DHKEM(X25519, HKDF-SHA256), then HKDF-SHA256 and ChaCha20Poly1305.
const KEM_SUITE: &[u8] = b"KEM\x00\x20";
const HPKE_SUITE: &[u8] = b"HPKE\x00\x20\x00\x01\x00\x03";

const KEY_LEN: usize = 32;
const X_NONCE: usize = 24;
const DEVICE_CAP: usize = 64;
const MAX_DOC: usize = 1024 * 1024;
const MAX_PAGES: usize = 4096;
const MAX_PAGE_ID: usize = 128;
const MAX_PAGE_TEXT: usize = 1024 * 1024;
const MAX_CHAIN_BYTES: usize = 64 * 1024;
/// Bytes that stay on the device. A mailbox rejects them.
pub const DEVICE_MAGIC: &[u8] = b"OMACUS-DEVICE/1\n";
/// A sealed snapshot of every page, inside a mailbox record.
pub const PAGES_MAGIC: &[u8] = b"OMACUS-PAGES/1\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncKeyError {
    Unavailable,
    NotAKey,
    NotConfirmed,
    NotApproved,
    Sealed,
    Corrupt,
}

impl SyncKeyError {
    pub fn message(self) -> &'static str {
        match self {
            SyncKeyError::Unavailable => "Try again in a moment.",
            SyncKeyError::NotAKey => "Those words are not a recovery key.",
            SyncKeyError::NotConfirmed => "Tap the three words first.",
            SyncKeyError::NotApproved => "That device was not approved.",
            SyncKeyError::Sealed => "Those pages could not be opened.",
            SyncKeyError::Corrupt => "That copy could not be read.",
        }
    }
}

fn words() -> &'static [&'static str] {
    static WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
        include_str!("recovery_words.txt")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect()
    });
    WORDS.as_slice()
}

fn random_bytes<const N: usize>() -> Result<[u8; N], SyncKeyError> {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).map_err(|_| SyncKeyError::Unavailable)?;
    Ok(buf)
}

#[cfg(test)]
fn holds(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|window| window == needle)
}

/// Twelve words. Debug output does not include them.
pub struct RecoveryKey {
    index: [u16; WORD_COUNT],
}

impl std::fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryKey(12 words)")
    }
}

impl Drop for RecoveryKey {
    fn drop(&mut self) {
        self.index.zeroize();
    }
}

impl Zeroize for RecoveryKey {
    fn zeroize(&mut self) {
        self.index.zeroize();
    }
}

impl ZeroizeOnDrop for RecoveryKey {}

impl RecoveryKey {
    /// A new key. The words are the only copy of the random bits.
    pub fn generate() -> Result<Self, SyncKeyError> {
        let mut entropy = random_bytes::<16>()?;
        let key = Self::from_entropy(entropy);
        entropy.zeroize();
        Ok(key)
    }

    pub fn from_entropy(entropy: [u8; 16]) -> Self {
        let mut bits = [0u8; 17];
        bits[..16].copy_from_slice(&entropy);
        bits[16] = checksum_nibble(&entropy) << 4;
        let mut index = [0u16; WORD_COUNT];
        for (n, slot) in index.iter_mut().enumerate() {
            let mut value = 0u16;
            for bit in 0..11 {
                let at = n * 11 + bit;
                let byte = bits[at / 8];
                let offset = 7 - (at % 8);
                value = (value << 1) | u16::from((byte >> offset) & 1);
            }
            *slot = value;
        }
        Self { index }
    }

    pub fn words(&self) -> [&'static str; WORD_COUNT] {
        let mut out = [""; WORD_COUNT];
        for (slot, index) in out.iter_mut().zip(self.index) {
            *slot = words().get(usize::from(index)).copied().unwrap_or("");
        }
        out
    }

    /// The words, with capitals, extra spaces, and one small typo forgiven.
    /// A typo is kept only when it is one edit from exactly one word on the list.
    pub fn parse(text: &str) -> Result<Self, SyncKeyError> {
        let tokens: Vec<&str> = text.split_whitespace().collect();
        if tokens.len() != WORD_COUNT {
            return Err(SyncKeyError::NotAKey);
        }
        let mut index = [0u16; WORD_COUNT];
        for (slot, token) in index.iter_mut().zip(tokens) {
            *slot = resolve_word(token)?;
        }
        let key = Self { index };
        key.entropy().map_err(|_| SyncKeyError::NotAKey)?;
        Ok(key)
    }

    /// The phrase can derive a root only after three of its words are tapped.
    /// `picks` are 1-based word numbers, the way the screen numbers them.
    pub fn confirm(&self, picks: &[(u8, &str)]) -> Result<Confirmed, SyncKeyError> {
        if picks.len() != CONFIRM_COUNT {
            return Err(SyncKeyError::NotConfirmed);
        }
        let mut seen = [false; WORD_COUNT];
        for &(position, tapped) in picks {
            if !(1..=WORD_COUNT as u8).contains(&position) {
                return Err(SyncKeyError::NotConfirmed);
            }
            let at = usize::from(position - 1);
            if seen[at] {
                return Err(SyncKeyError::NotConfirmed);
            }
            seen[at] = true;
            let want = words()
                .get(usize::from(self.index[at]))
                .copied()
                .ok_or(SyncKeyError::Corrupt)?;
            if !same_word(tapped, want) {
                return Err(SyncKeyError::NotConfirmed);
            }
        }
        Ok(Confirmed {
            entropy: self.entropy()?,
        })
    }

    fn entropy(&self) -> Result<[u8; 16], SyncKeyError> {
        if self
            .index
            .iter()
            .any(|index| usize::from(*index) >= words().len())
        {
            return Err(SyncKeyError::Corrupt);
        }
        // 12 words are 132 bits: 16 bytes of entropy, then 4 checksum bits.
        let mut bits = [0u8; 17];
        for (n, index) in self.index.iter().enumerate() {
            for bit in 0..11 {
                let at = n * 11 + bit;
                let value = (index >> (10 - bit)) & 1;
                bits[at / 8] |= (value as u8) << (7 - (at % 8));
            }
        }
        let mut entropy = [0u8; 16];
        entropy.copy_from_slice(&bits[..16]);
        if bits[16] >> 4 != checksum_nibble(&entropy) {
            return Err(SyncKeyError::NotAKey);
        }
        Ok(entropy)
    }
}

/// A recovery key whose three words were tapped. The words themselves are gone.
pub struct Confirmed {
    entropy: [u8; 16],
}

impl std::fmt::Debug for Confirmed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Confirmed(..)")
    }
}

impl Drop for Confirmed {
    fn drop(&mut self) {
        self.entropy.zeroize();
    }
}

impl Zeroize for Confirmed {
    fn zeroize(&mut self) {
        self.entropy.zeroize();
    }
}

impl ZeroizeOnDrop for Confirmed {}

impl Confirmed {
    #[doc(hidden)]
    pub fn entropy_bytes(&self) -> &[u8; 16] {
        &self.entropy
    }

    /// `extra` is the optional word mixed into the root. Empty means there is none.
    /// Forgetting a word that was set cannot be repaired.
    pub fn root(&self, extra: &str) -> Result<Root, SyncKeyError> {
        let mut input = Vec::with_capacity(17 + extra.len());
        input.extend_from_slice(&self.entropy);
        input.push(0);
        input.extend_from_slice(extra.as_bytes());
        let params = Params::new(MEMORY_KIB, ITERATIONS, LANES, Some(KEY_LEN))
            .map_err(|_| SyncKeyError::Corrupt)?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut out = [0u8; KEY_LEN];
        let derived = argon.hash_password_into(&input, ROOT_SALT, &mut out);
        input.zeroize();
        derived.map_err(|_| SyncKeyError::Corrupt)?;
        Ok(Root(out))
    }
}

/// Words that start with `prefix`, for the entry screen. At most eight.
pub fn suggest(prefix: &str) -> Vec<&'static str> {
    let prefix = prefix.trim().to_lowercase();
    if prefix.is_empty() {
        return Vec::new();
    }
    words()
        .iter()
        .copied()
        .filter(|word| word.starts_with(&prefix))
        .take(8)
        .collect()
}

/// A pasted sheet: drop numbering, commas, and extra space, and ignore case.
/// `1. River` and `river` are the same word. The check that it is a real key
/// is still [`RecoveryKey::parse`].
pub fn normalize_phrase(text: &str) -> String {
    let mut words = Vec::new();
    for raw in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '|') {
        let token = raw.trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '“' | '”' | '‘' | '’' | '(' | ')' | '[' | ']' | '.' | ':' | '!' | '?'
            )
        });
        if token.is_empty() {
            continue;
        }
        let token = strip_index(token);
        let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if token.is_empty() || token.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        words.push(token.to_lowercase());
    }
    words.join(" ")
}

fn strip_index(token: &str) -> &str {
    let bytes = token.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index > 0 && index < bytes.len() && (bytes[index] == b'.' || bytes[index] == b')') {
        return &token[index + 1..];
    }
    token
}

fn checksum_nibble(entropy: &[u8; 16]) -> u8 {
    Sha256::digest(entropy)[0] >> 4
}

fn same_word(tapped: &str, want: &str) -> bool {
    let tapped = tapped.trim();
    tapped.len() == want.len()
        && tapped
            .bytes()
            .zip(want.bytes())
            .all(|(got, expect)| got.to_ascii_lowercase() == expect)
}

fn resolve_word(token: &str) -> Result<u16, SyncKeyError> {
    let token = token.trim().to_lowercase();
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_lowercase()) {
        return Err(SyncKeyError::NotAKey);
    }
    if let Some(index) = words().iter().position(|word| *word == token) {
        return Ok(index as u16);
    }
    let mut found = None;
    for (index, word) in words().iter().enumerate() {
        if edit_distance_1(&token, word) {
            if found.is_some() {
                return Err(SyncKeyError::NotAKey);
            }
            found = Some(index as u16);
        }
    }
    found.ok_or(SyncKeyError::NotAKey)
}

/// One insert, delete, substitution, or swap of two neighbouring letters.
fn edit_distance_1(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() == right.len() {
        let mut diffs = [0usize; 2];
        let mut count = 0;
        for (index, (a, b)) in left.iter().zip(right.iter()).enumerate() {
            if a != b {
                if count == 2 {
                    return false;
                }
                diffs[count] = index;
                count += 1;
            }
        }
        if count == 1 {
            return true;
        }
        count == 2
            && diffs[1] == diffs[0] + 1
            && left[diffs[0]] == right[diffs[1]]
            && left[diffs[1]] == right[diffs[0]]
    } else if left.len() + 1 == right.len() {
        one_extra(left, right)
    } else if right.len() + 1 == left.len() {
        one_extra(right, left)
    } else {
        false
    }
}

fn one_extra(shorter: &[u8], longer: &[u8]) -> bool {
    let mut short_at = 0;
    let mut long_at = 0;
    let mut skipped = false;
    while short_at < shorter.len() && long_at < longer.len() {
        if shorter[short_at] == longer[long_at] {
            short_at += 1;
            long_at += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
            long_at += 1;
        }
    }
    true
}

/// The root. Debug output does not include the bytes, and drop wipes them.
pub struct Root([u8; KEY_LEN]);

impl std::fmt::Debug for Root {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Root(..)")
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Zeroize for Root {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl ZeroizeOnDrop for Root {}

impl Root {
    #[doc(hidden)]
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    fn expand(&self, info: &[u8], out: &mut [u8]) -> Result<(), SyncKeyError> {
        let hk = Hkdf::<Sha256>::new(Some(ROOT_SALT), &self.0);
        hk.expand(info, out).map_err(|_| SyncKeyError::Corrupt)
    }

    pub fn account_public(&self) -> Result<[u8; KEY_LEN], SyncKeyError> {
        let mut secret = [0u8; KEY_LEN];
        self.expand(b"omacus/v1/account-sign", &mut secret)?;
        let key = SigningKey::from_bytes(&secret);
        secret.zeroize();
        Ok(key.verifying_key().to_bytes())
    }
}

/// Four everyday words for checking an account, made from the public key only.
pub fn account_fingerprint(account: &[u8; KEY_LEN]) -> [&'static str; 4] {
    let digest = Sha256::digest([b"omacus/v1/fingerprint".as_slice(), account].concat());
    let mut acc = 0u32;
    let mut have = 0;
    let mut byte_at = 0;
    let mut out = [""; 4];
    for slot in &mut out {
        while have < 11 {
            acc = (acc << 8) | u32::from(digest[byte_at]);
            byte_at += 1;
            have += 8;
        }
        have -= 11;
        let index = ((acc >> have) & 0x7ff) as usize;
        *slot = words()[index];
    }
    out
}

/// A vault key. The id can be stored in the open. The key cannot.
pub struct VaultKey {
    id: [u8; 16],
    key: [u8; KEY_LEN],
}

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultKey(..)")
    }
}

impl Drop for VaultKey {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl Zeroize for VaultKey {
    fn zeroize(&mut self) {
        self.key.zeroize();
    }
}

impl ZeroizeOnDrop for VaultKey {}

impl VaultKey {
    pub fn generate() -> Result<Self, SyncKeyError> {
        Ok(Self {
            id: random_bytes()?,
            key: random_bytes()?,
        })
    }

    pub fn id(&self) -> &[u8; 16] {
        &self.id
    }

    #[doc(hidden)]
    pub fn secret(&self) -> &[u8; KEY_LEN] {
        &self.key
    }
}

/// One device's keys. They never leave the device.
pub struct DeviceKeys {
    id: [u8; 16],
    sign: [u8; KEY_LEN],
    receive: [u8; KEY_LEN],
}

impl std::fmt::Debug for DeviceKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeviceKeys(..)")
    }
}

impl Drop for DeviceKeys {
    fn drop(&mut self) {
        self.sign.zeroize();
        self.receive.zeroize();
    }
}

impl Zeroize for DeviceKeys {
    fn zeroize(&mut self) {
        self.sign.zeroize();
        self.receive.zeroize();
    }
}

impl ZeroizeOnDrop for DeviceKeys {}

/// What another device, and the server, are allowed to see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DevicePublic {
    id: [u8; 16],
    sign: [u8; KEY_LEN],
    receive: [u8; KEY_LEN],
}

impl DeviceKeys {
    pub fn generate() -> Result<Self, SyncKeyError> {
        Ok(Self {
            id: random_bytes()?,
            sign: random_bytes()?,
            receive: random_bytes()?,
        })
    }

    #[doc(hidden)]
    pub fn signing_secret(&self) -> &[u8; KEY_LEN] {
        &self.sign
    }

    #[doc(hidden)]
    pub fn receive_secret(&self) -> &[u8; KEY_LEN] {
        &self.receive
    }

    pub fn public(&self) -> DevicePublic {
        DevicePublic {
            id: self.id,
            sign: ed25519_public(&self.sign),
            receive: x25519_public(&self.receive),
        }
    }

    fn signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.sign)
    }
}

fn ed25519_public(secret: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    SigningKey::from_bytes(secret).verifying_key().to_bytes()
}

fn x25519_public(secret: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
}

fn dh(secret: &[u8; KEY_LEN], public: &[u8; KEY_LEN]) -> Result<[u8; KEY_LEN], SyncKeyError> {
    let scalar = StaticSecret::from(*secret);
    let point = PublicKey::from(*public);
    let shared = scalar.diffie_hellman(&point);
    if !shared.was_contributory() {
        return Err(SyncKeyError::Sealed);
    }
    Ok(shared.to_bytes())
}

impl DevicePublic {
    /// A device another screen already checked. The bytes are the id, then the
    /// signing key, then the receive key, each as they are shown in the open.
    pub fn from_parts(id: [u8; 16], sign: [u8; KEY_LEN], receive: [u8; KEY_LEN]) -> Self {
        Self { id, sign, receive }
    }

    pub fn id(&self) -> &[u8; 16] {
        &self.id
    }

    pub fn signing_key(&self) -> &[u8; KEY_LEN] {
        &self.sign
    }

    pub fn receive_key(&self) -> &[u8; KEY_LEN] {
        &self.receive
    }
}

/// The same six digits on both screens. It is made from the new device's
/// public keys, so it is not a key and a swapped device shows a different code.
pub fn pairing_code(device: &DevicePublic) -> String {
    let digest =
        Sha256::digest([b"omacus/v1/pair".as_slice(), &device.sign, &device.receive].concat());
    let number = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{:03} {:03}", number / 1000, number % 1000)
}

#[derive(Clone, PartialEq, Eq)]
struct SignedDevices {
    version: u64,
    prev: [u8; KEY_LEN],
    signer: [u8; 16],
    devices: Vec<DevicePublic>,
    signature: [u8; 64],
}

impl SignedDevices {
    /// The bytes the signature covers. The vault key is not one of them.
    fn message(&self) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend_from_slice(b"omacus-devices/1");
        message.extend_from_slice(&self.version.to_be_bytes());
        message.extend_from_slice(&self.prev);
        message.extend_from_slice(&self.signer);
        message.extend_from_slice(&(self.devices.len() as u16).to_be_bytes());
        for device in &self.devices {
            message.extend_from_slice(&device.id);
            message.extend_from_slice(&device.sign);
            message.extend_from_slice(&device.receive);
        }
        message
    }
}

/// The devices allowed to hold the vault key, oldest list first.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceChain {
    lists: Vec<SignedDevices>,
}

impl DeviceChain {
    pub fn verify(&self) -> Result<(), SyncKeyError> {
        if self.lists.is_empty() {
            return Err(SyncKeyError::Corrupt);
        }
        for (place, list) in self.lists.iter().enumerate() {
            if list.version != place as u64 + 1
                || list.devices.is_empty()
                || list.devices.len() > DEVICE_CAP
            {
                return Err(SyncKeyError::NotApproved);
            }
            if !unique_ids(&list.devices) {
                return Err(SyncKeyError::NotApproved);
            }
            if place == 0 {
                if list.prev != [0u8; KEY_LEN]
                    || list.devices.len() != 1
                    || list.signer != list.devices[0].id
                {
                    return Err(SyncKeyError::NotApproved);
                }
            } else {
                let previous = &self.lists[place - 1];
                let mut previous_hash = [0u8; KEY_LEN];
                previous_hash.copy_from_slice(&Sha256::digest(previous.message()));
                if list.prev != previous_hash || !changed_by_one(&previous.devices, &list.devices) {
                    return Err(SyncKeyError::NotApproved);
                }
                if !previous
                    .devices
                    .iter()
                    .any(|device| device.id == list.signer)
                {
                    return Err(SyncKeyError::NotApproved);
                }
            }
            let signer = list
                .devices
                .iter()
                .find(|device| device.id == list.signer)
                .ok_or(SyncKeyError::NotApproved)?;
            let key =
                VerifyingKey::from_bytes(&signer.sign).map_err(|_| SyncKeyError::NotApproved)?;
            let signature =
                Signature::from_slice(&list.signature).map_err(|_| SyncKeyError::Corrupt)?;
            key.verify(&list.message(), &signature)
                .map_err(|_| SyncKeyError::NotApproved)?;
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"OMACUS-DEVICES/1");
        out.extend_from_slice(&(self.lists.len() as u16).to_be_bytes());
        for list in &self.lists {
            out.extend_from_slice(&list.version.to_be_bytes());
            out.extend_from_slice(&list.prev);
            out.extend_from_slice(&list.signer);
            out.extend_from_slice(&list.signature);
            out.extend_from_slice(&(list.devices.len() as u16).to_be_bytes());
            for device in &list.devices {
                out.extend_from_slice(&device.id);
                out.extend_from_slice(&device.sign);
                out.extend_from_slice(&device.receive);
            }
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SyncKeyError> {
        let mut reader = Reader::new(bytes);
        if reader.take(16)? != b"OMACUS-DEVICES/1" {
            return Err(SyncKeyError::Corrupt);
        }
        let count = usize::from(reader.u16()?);
        if count == 0 || count > DEVICE_CAP {
            return Err(SyncKeyError::Corrupt);
        }
        let mut lists = Vec::with_capacity(count);
        for _ in 0..count {
            let version = reader.u64()?;
            let prev = reader.array()?;
            let signer = reader.array()?;
            let signature = reader.array()?;
            let devices_len = usize::from(reader.u16()?);
            if devices_len == 0 || devices_len > DEVICE_CAP {
                return Err(SyncKeyError::Corrupt);
            }
            let mut devices = Vec::with_capacity(devices_len);
            for _ in 0..devices_len {
                devices.push(DevicePublic {
                    id: reader.array()?,
                    sign: reader.array()?,
                    receive: reader.array()?,
                });
            }
            lists.push(SignedDevices {
                version,
                prev,
                signer,
                devices,
                signature,
            });
        }
        reader.finish()?;
        Ok(Self { lists })
    }

    fn latest(&self) -> Result<&SignedDevices, SyncKeyError> {
        self.lists.last().ok_or(SyncKeyError::Corrupt)
    }

    /// The devices on the newest signed list.
    pub fn devices(&self) -> Result<Vec<DevicePublic>, SyncKeyError> {
        Ok(self.latest()?.devices.clone())
    }

    /// How this chain sits against `earlier`. Newer means this one extends it.
    pub fn compared_with(&self, earlier: &DeviceChain) -> ChainOrder {
        let shared = self.lists.len().min(earlier.lists.len());
        if self.lists[..shared] != earlier.lists[..shared] {
            return ChainOrder::Diverge;
        }
        match self.lists.len().cmp(&earlier.lists.len()) {
            std::cmp::Ordering::Equal => ChainOrder::Same,
            std::cmp::Ordering::Greater => ChainOrder::Newer,
            std::cmp::Ordering::Less => ChainOrder::Older,
        }
    }
}

/// Whether one device list extends another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainOrder {
    Same,
    Newer,
    Older,
    Diverge,
}

fn unique_ids(devices: &[DevicePublic]) -> bool {
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

fn appended_one(previous: &[DevicePublic], next: &[DevicePublic]) -> bool {
    next.len() == previous.len() + 1
        && next[..previous.len()] == previous[..]
        && !previous
            .iter()
            .any(|device| device.id == next[previous.len()].id)
}

/// One device left the list. The rest stay, in the same order, with the same keys.
fn removed_one(previous: &[DevicePublic], next: &[DevicePublic]) -> bool {
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

fn changed_by_one(previous: &[DevicePublic], next: &[DevicePublic]) -> bool {
    appended_one(previous, next) || removed_one(previous, next)
}

/// The first device signs a list that contains only itself.
pub fn found_devices(device: &DeviceKeys) -> Result<DeviceChain, SyncKeyError> {
    let public = device.public();
    let mut list = SignedDevices {
        version: 1,
        prev: [0u8; KEY_LEN],
        signer: public.id,
        devices: vec![public],
        signature: [0u8; 64],
    };
    list.signature = sign_list(device, &list)?;
    let chain = DeviceChain { lists: vec![list] };
    chain.verify()?;
    Ok(chain)
}

fn sign_list(device: &DeviceKeys, list: &SignedDevices) -> Result<[u8; 64], SyncKeyError> {
    if device.id != list.signer {
        return Err(SyncKeyError::NotApproved);
    }
    Ok(device.signing_key().sign(&list.message()).to_bytes())
}

/// An approved device adds one new device and seals the vault key to it.
/// The new list is signed by the device that already has the pages.
pub fn approve(
    approver: &DeviceKeys,
    chain: &DeviceChain,
    newcomer: &DevicePublic,
    vault: &VaultKey,
    epoch: u64,
) -> Result<(DeviceChain, Vec<u8>), SyncKeyError> {
    chain.verify()?;
    let latest = chain.latest()?;
    if !latest.devices.iter().any(|device| device.id == approver.id) {
        return Err(SyncKeyError::NotApproved);
    }
    if latest.devices.iter().any(|device| device.id == newcomer.id) {
        return Err(SyncKeyError::NotApproved);
    }
    let mut devices = latest.devices.clone();
    devices.push(*newcomer);
    let mut next = SignedDevices {
        version: latest.version + 1,
        prev: {
            let mut hash = [0u8; KEY_LEN];
            hash.copy_from_slice(&Sha256::digest(latest.message()));
            hash
        },
        signer: approver.id,
        devices,
        signature: [0u8; 64],
    };
    next.signature = sign_list(approver, &next)?;
    let mut lists = chain.lists.clone();
    lists.push(next);
    let approved = DeviceChain { lists };
    approved.verify()?;
    let wrap = wrap_vault_for_device(newcomer, vault, epoch)?;
    Ok((approved, wrap))
}

/// An approved device removes one other device from the list.
/// The removed device is no longer approved to add or remove devices.
pub fn remove_device(
    approver: &DeviceKeys,
    chain: &DeviceChain,
    target: &[u8; 16],
) -> Result<DeviceChain, SyncKeyError> {
    chain.verify()?;
    let latest = chain.latest()?;
    if approver.id == *target || latest.devices.len() < 2 {
        return Err(SyncKeyError::NotApproved);
    }
    if !latest.devices.iter().any(|device| device.id == approver.id) {
        return Err(SyncKeyError::NotApproved);
    }
    if !latest.devices.iter().any(|device| device.id == *target) {
        return Err(SyncKeyError::NotApproved);
    }
    let devices: Vec<_> = latest
        .devices
        .iter()
        .copied()
        .filter(|device| device.id != *target)
        .collect();
    let mut next = SignedDevices {
        version: latest.version + 1,
        prev: {
            let mut hash = [0u8; KEY_LEN];
            hash.copy_from_slice(&Sha256::digest(latest.message()));
            hash
        },
        signer: approver.id,
        devices,
        signature: [0u8; 64],
    };
    next.signature = sign_list(approver, &next)?;
    let mut lists = chain.lists.clone();
    lists.push(next);
    let removed = DeviceChain { lists };
    removed.verify()?;
    Ok(removed)
}

fn bind(kind: &[u8], epoch: u64, key_id: &[u8; 16], recipient: Option<&[u8; 16]>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"omacus/v1");
    out.extend_from_slice(&PROTOCOL.to_be_bytes());
    out.extend_from_slice(VAULT_ID.as_bytes());
    out.extend_from_slice(&epoch.to_be_bytes());
    out.extend_from_slice(key_id);
    out.extend_from_slice(kind);
    if let Some(recipient) = recipient {
        out.extend_from_slice(recipient);
    }
    out
}

pub fn wrap_vault_for_phrase(
    root: &Root,
    vault: &VaultKey,
    epoch: u64,
) -> Result<Vec<u8>, SyncKeyError> {
    let mut kek = [0u8; KEY_LEN];
    root.expand(b"omacus/v1/vault-wrap/0", &mut kek)?;
    let mut nonce = random_bytes::<X_NONCE>()?;
    let aad = bind(b"phrase-key", epoch, &vault.id, None);
    let ciphertext = xchacha(true, &kek, &nonce, &aad, &vault.key)?;
    kek.zeroize();
    let mut out = Vec::with_capacity(X_NONCE + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    nonce.zeroize();
    Ok(out)
}

pub fn open_phrase_wrap(
    root: &Root,
    stored: &[u8],
    epoch: u64,
    key_id: &[u8; 16],
) -> Result<VaultKey, SyncKeyError> {
    if stored.len() < X_NONCE + 16 {
        return Err(SyncKeyError::Corrupt);
    }
    let mut kek = [0u8; KEY_LEN];
    root.expand(b"omacus/v1/vault-wrap/0", &mut kek)?;
    let (nonce, ciphertext) = stored.split_at(X_NONCE);
    let nonce: [u8; X_NONCE] = nonce.try_into().map_err(|_| SyncKeyError::Corrupt)?;
    let aad = bind(b"phrase-key", epoch, key_id, None);
    let plain = xchacha(false, &kek, &nonce, &aad, ciphertext)?;
    kek.zeroize();
    let key: [u8; KEY_LEN] = plain.try_into().map_err(|_| SyncKeyError::Sealed)?;
    Ok(VaultKey { id: *key_id, key })
}

pub fn wrap_vault_for_device(
    recipient: &DevicePublic,
    vault: &VaultKey,
    epoch: u64,
) -> Result<Vec<u8>, SyncKeyError> {
    let aad = bind(b"vault-key", epoch, &vault.id, Some(&recipient.id));
    let info = aad.clone();
    let (enc, ciphertext) = hpke_seal(&recipient.receive, &info, &aad, &vault.key, None)?;
    let mut out = Vec::with_capacity(16 + KEY_LEN + ciphertext.len());
    out.extend_from_slice(&recipient.id);
    out.extend_from_slice(&enc);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn open_device_wrap(
    device: &DeviceKeys,
    stored: &[u8],
    epoch: u64,
    key_id: &[u8; 16],
) -> Result<VaultKey, SyncKeyError> {
    if stored.len() < 16 + KEY_LEN + 16 {
        return Err(SyncKeyError::Corrupt);
    }
    let (id, rest) = stored.split_at(16);
    if id != device.id {
        return Err(SyncKeyError::Sealed);
    }
    let (enc, ciphertext) = rest.split_at(KEY_LEN);
    let enc: [u8; KEY_LEN] = enc.try_into().map_err(|_| SyncKeyError::Corrupt)?;
    let aad = bind(b"vault-key", epoch, key_id, Some(&device.id));
    let plain = hpke_open(&device.receive, &enc, &aad, &aad, ciphertext)?;
    let key: [u8; KEY_LEN] = plain.try_into().map_err(|_| SyncKeyError::Sealed)?;
    Ok(VaultKey { id: *key_id, key })
}

pub fn seal_page(
    vault: &VaultKey,
    epoch: u64,
    page_id: &[u8],
    page: &[u8],
) -> Result<Vec<u8>, SyncKeyError> {
    let mut nonce = random_bytes::<X_NONCE>()?;
    let mut aad = bind(b"page", epoch, &vault.id, None);
    aad.extend_from_slice(page_id);
    let ciphertext = xchacha(true, &vault.key, &nonce, &aad, page)?;
    let mut out = Vec::with_capacity(X_NONCE + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    nonce.zeroize();
    Ok(out)
}

pub fn open_page(
    vault: &VaultKey,
    epoch: u64,
    page_id: &[u8],
    stored: &[u8],
) -> Result<Vec<u8>, SyncKeyError> {
    if stored.len() < X_NONCE + 16 {
        return Err(SyncKeyError::Corrupt);
    }
    let (nonce, ciphertext) = stored.split_at(X_NONCE);
    let nonce: [u8; X_NONCE] = nonce.try_into().map_err(|_| SyncKeyError::Corrupt)?;
    let mut aad = bind(b"page", epoch, &vault.id, None);
    aad.extend_from_slice(page_id);
    xchacha(false, &vault.key, &nonce, &aad, ciphertext)
}

/// What a server would store. Public keys, signatures, and ciphertext.
pub struct ServerView {
    pub epoch: u64,
    pub account: [u8; KEY_LEN],
    pub chain: DeviceChain,
    pub phrase_wrap: Vec<u8>,
    pub device_wraps: Vec<Vec<u8>>,
    pub key_id: [u8; 16],
    pub page: Vec<u8>,
}

impl ServerView {
    pub fn stored(&self) -> Vec<u8> {
        let chain = self.chain.to_bytes();
        let mut out = Vec::new();
        out.extend_from_slice(b"OMACUS-SYNC/1\n");
        let vault = VAULT_ID.as_bytes();
        out.push(vault.len() as u8);
        out.extend_from_slice(vault);
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.extend_from_slice(&self.account);
        out.extend_from_slice(&self.key_id);
        push_chunk(&mut out, &chain);
        push_chunk(&mut out, &self.phrase_wrap);
        out.extend_from_slice(&(self.device_wraps.len() as u16).to_be_bytes());
        for wrap in &self.device_wraps {
            push_chunk(&mut out, wrap);
        }
        push_chunk(&mut out, &self.page);
        out
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, SyncKeyError> {
        let mut reader = Reader::new(bytes);
        if reader.take(14)? != b"OMACUS-SYNC/1\n" {
            return Err(SyncKeyError::Corrupt);
        }
        let vault_len = usize::from(reader.byte()?);
        if reader.take(vault_len)? != VAULT_ID.as_bytes() {
            return Err(SyncKeyError::Corrupt);
        }
        let epoch = reader.u64()?;
        let account = reader.array()?;
        let key_id = reader.array()?;
        let chain = DeviceChain::from_bytes(&reader.chunk()?)?;
        let phrase_wrap = reader.chunk()?;
        let wraps = usize::from(reader.u16()?);
        let mut device_wraps = Vec::with_capacity(wraps);
        for _ in 0..wraps {
            device_wraps.push(reader.chunk()?);
        }
        let page = reader.chunk()?;
        reader.finish()?;
        Ok(Self {
            epoch,
            account,
            chain,
            phrase_wrap,
            device_wraps,
            key_id,
            page,
        })
    }
}

/// What one device remembers so a reload stays linked. It is not a mailbox record.
pub struct DeviceStore {
    pub device: DeviceKeys,
    pub vault: VaultKey,
    pub account: [u8; 32],
    pub epoch: u64,
    pub chain: Vec<u8>,
}

impl std::fmt::Debug for DeviceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeviceStore(..)")
    }
}

pub fn store_device(
    device: &DeviceKeys,
    vault: &VaultKey,
    account: &[u8; 32],
    epoch: u64,
    chain: &[u8],
) -> Result<Vec<u8>, SyncKeyError> {
    if chain.len() > MAX_CHAIN_BYTES {
        return Err(SyncKeyError::Corrupt);
    }
    let mut out = Vec::new();
    out.extend_from_slice(DEVICE_MAGIC);
    out.extend_from_slice(&device.id);
    out.extend_from_slice(&device.sign);
    out.extend_from_slice(&device.receive);
    out.extend_from_slice(&vault.id);
    out.extend_from_slice(&vault.key);
    out.extend_from_slice(account);
    out.extend_from_slice(&epoch.to_be_bytes());
    push_chunk(&mut out, chain);
    let digest = Sha256::digest(&out);
    out.extend_from_slice(&digest);
    Ok(out)
}

pub fn open_device_store(bytes: &[u8]) -> Result<DeviceStore, SyncKeyError> {
    if bytes.len() < DEVICE_MAGIC.len() + 16 + KEY_LEN + KEY_LEN + 16 + KEY_LEN + 32 + 8 + 32 {
        return Err(SyncKeyError::Corrupt);
    }
    let (body, digest) = bytes.split_at(bytes.len() - 32);
    if Sha256::digest(body).as_slice() != digest {
        return Err(SyncKeyError::Corrupt);
    }
    let mut reader = Reader::new(body);
    if reader.take(DEVICE_MAGIC.len())? != DEVICE_MAGIC {
        return Err(SyncKeyError::Corrupt);
    }
    let store = DeviceStore {
        device: DeviceKeys {
            id: reader.array()?,
            sign: reader.array()?,
            receive: reader.array()?,
        },
        vault: VaultKey {
            id: reader.array()?,
            key: reader.array()?,
        },
        account: reader.array()?,
        epoch: reader.u64()?,
        chain: reader.chunk()?,
    };
    if store.chain.len() > MAX_CHAIN_BYTES {
        return Err(SyncKeyError::Corrupt);
    }
    reader.finish()?;
    Ok(store)
}

/// Every page, plus the library document, before it is sealed.
#[derive(Clone)]
pub struct PageSet {
    pub doc: Vec<u8>,
    pub pages: Vec<(String, String)>,
}

impl PageSet {
    pub fn to_bytes(&self) -> Result<Vec<u8>, SyncKeyError> {
        if self.doc.len() > MAX_DOC || self.pages.len() > MAX_PAGES {
            return Err(SyncKeyError::Corrupt);
        }
        let mut out = Vec::new();
        out.extend_from_slice(PAGES_MAGIC);
        push_chunk(&mut out, &self.doc);
        out.extend_from_slice(
            &(u32::try_from(self.pages.len()).map_err(|_| SyncKeyError::Corrupt)?).to_be_bytes(),
        );
        for (id, text) in &self.pages {
            let id_bytes = id.as_bytes();
            let text_bytes = text.as_bytes();
            if id_bytes.is_empty()
                || id_bytes.len() > MAX_PAGE_ID
                || text_bytes.len() > MAX_PAGE_TEXT
            {
                return Err(SyncKeyError::Corrupt);
            }
            let id_len = u16::try_from(id_bytes.len()).map_err(|_| SyncKeyError::Corrupt)?;
            out.extend_from_slice(&id_len.to_be_bytes());
            out.extend_from_slice(id_bytes);
            push_chunk(&mut out, text_bytes);
        }
        Ok(out)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SyncKeyError> {
        let mut reader = Reader::new(bytes);
        if reader.take(PAGES_MAGIC.len())? != PAGES_MAGIC {
            return Err(SyncKeyError::Corrupt);
        }
        let doc = reader.chunk()?;
        if doc.len() > MAX_DOC {
            return Err(SyncKeyError::Corrupt);
        }
        let count = usize::try_from(reader.u32()?).map_err(|_| SyncKeyError::Corrupt)?;
        if count > MAX_PAGES {
            return Err(SyncKeyError::Corrupt);
        }
        let mut pages = Vec::with_capacity(count);
        for _ in 0..count {
            let id_len = usize::from(reader.u16()?);
            if id_len == 0 || id_len > MAX_PAGE_ID {
                return Err(SyncKeyError::Corrupt);
            }
            let id = std::str::from_utf8(reader.take(id_len)?)
                .map_err(|_| SyncKeyError::Corrupt)?
                .to_string();
            let text_bytes = reader.chunk()?;
            if text_bytes.len() > MAX_PAGE_TEXT {
                return Err(SyncKeyError::Corrupt);
            }
            let text = String::from_utf8(text_bytes).map_err(|_| SyncKeyError::Corrupt)?;
            pages.push((id, text));
        }
        reader.finish()?;
        Ok(Self { doc, pages })
    }

    /// Keep this document. Page text is the union: lines are added, never dropped.
    pub fn merge(self, other: Self) -> Self {
        Self {
            doc: self.doc,
            pages: merge_page_texts(&self.pages, &other.pages),
        }
    }
}

pub fn merge_page_texts(
    local: &[(String, String)],
    remote: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out = local.to_vec();
    for (id, remote_text) in remote {
        if let Some(index) = out.iter().position(|(have, _)| have == id) {
            let local_text = out[index].1.clone();
            if local_text == *remote_text {
                continue;
            }
            if sealed_page(&local_text) || sealed_page(remote_text) {
                let copy = fresh_page_id(&out, id);
                out.push((copy, remote_text.clone()));
            } else {
                out[index].1 = merge_lines(&local_text, remote_text);
            }
        } else {
            out.push((id.clone(), remote_text.clone()));
        }
    }
    out
}

fn sealed_page(text: &str) -> bool {
    text.starts_with("OMACUS-LOCK/1\n")
}

fn fresh_page_id(pages: &[(String, String)], id: &str) -> String {
    for n in 2..1000 {
        let next = format!("{id}~{n}");
        if !pages.iter().any(|(have, _)| have == &next) {
            return next;
        }
    }
    format!("{id}~extra")
}

fn merge_lines(local: &str, remote: &str) -> String {
    if local == remote || remote.is_empty() {
        return local.to_string();
    }
    if local.is_empty() {
        return remote.to_string();
    }
    if local.contains(remote) {
        return local.to_string();
    }
    if remote.contains(local) {
        return remote.to_string();
    }
    let lines: Vec<&str> = local.split('\n').collect();
    let have: std::collections::HashSet<&str> = lines.iter().copied().collect();
    let mut extra = Vec::new();
    for line in remote.split('\n') {
        if !have.contains(line) {
            extra.push(line);
        }
    }
    if extra.iter().all(|line| line.is_empty()) {
        return if local.len() >= remote.len() {
            local.to_string()
        } else {
            remote.to_string()
        };
    }
    let mut merged = local.trim_end_matches('\n').to_string();
    for line in extra {
        if line.is_empty() {
            continue;
        }
        merged.push('\n');
        merged.push_str(line);
    }
    if local.ends_with('\n') || remote.ends_with('\n') {
        merged.push('\n');
    }
    merged
}

fn push_chunk(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(rest: &'a [u8]) -> Self {
        Self { rest }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], SyncKeyError> {
        if self.rest.len() < len {
            return Err(SyncKeyError::Corrupt);
        }
        let (head, tail) = self.rest.split_at(len);
        self.rest = tail;
        Ok(head)
    }

    fn byte(&mut self) -> Result<u8, SyncKeyError> {
        Ok(self.take(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], SyncKeyError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u16(&mut self) -> Result<u16, SyncKeyError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, SyncKeyError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, SyncKeyError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn chunk(&mut self) -> Result<Vec<u8>, SyncKeyError> {
        let len = usize::try_from(self.u32()?).map_err(|_| SyncKeyError::Corrupt)?;
        Ok(self.take(len)?.to_vec())
    }

    fn finish(self) -> Result<(), SyncKeyError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(SyncKeyError::Corrupt)
        }
    }
}

fn xchacha(
    encrypting: bool,
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    input: &[u8],
) -> Result<Vec<u8>, SyncKeyError> {
    if key.len() != KEY_LEN || nonce.len() != X_NONCE {
        return Err(SyncKeyError::Corrupt);
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| SyncKeyError::Corrupt)?;
    let nonce = XNonce::from_slice(nonce);
    let payload = Payload { msg: input, aad };
    if encrypting {
        cipher
            .encrypt(nonce, payload)
            .map_err(|_| SyncKeyError::Corrupt)
    } else {
        cipher
            .decrypt(nonce, payload)
            .map_err(|_| SyncKeyError::Sealed)
    }
}

fn labeled_extract(suite: &[u8], salt: &[u8], label: &[u8], ikm: &[u8]) -> [u8; KEY_LEN] {
    let mut labeled = Vec::with_capacity(7 + suite.len() + label.len() + ikm.len());
    labeled.extend_from_slice(b"HPKE-v1");
    labeled.extend_from_slice(suite);
    labeled.extend_from_slice(label);
    labeled.extend_from_slice(ikm);
    let salt = if salt.is_empty() { None } else { Some(salt) };
    let (prk, _) = Hkdf::<Sha256>::extract(salt, &labeled);
    labeled.zeroize();
    let mut out = [0u8; KEY_LEN];
    out.copy_from_slice(prk.as_ref());
    out
}

fn labeled_expand(
    suite: &[u8],
    prk: &[u8],
    label: &[u8],
    info: &[u8],
    out: &mut [u8],
) -> Result<(), SyncKeyError> {
    let hk = Hkdf::<Sha256>::from_prk(prk).map_err(|_| SyncKeyError::Corrupt)?;
    let len = u16::try_from(out.len())
        .map_err(|_| SyncKeyError::Corrupt)?
        .to_be_bytes();
    hk.expand_multi_info(&[&len, b"HPKE-v1", suite, label, info], out)
        .map_err(|_| SyncKeyError::Corrupt)
}

fn extract_and_expand(dh_secret: &[u8], context: &[u8]) -> Result<[u8; KEY_LEN], SyncKeyError> {
    let mut extracted = labeled_extract(KEM_SUITE, b"", b"eae_prk", dh_secret);
    let mut shared = [0u8; KEY_LEN];
    labeled_expand(
        KEM_SUITE,
        &extracted,
        b"shared_secret",
        context,
        &mut shared,
    )?;
    extracted.zeroize();
    Ok(shared)
}

fn key_schedule(shared: &[u8], info: &[u8]) -> Result<([u8; KEY_LEN], [u8; 12]), SyncKeyError> {
    let psk_id_hash = labeled_extract(HPKE_SUITE, b"", b"psk_id_hash", b"");
    let info_hash = labeled_extract(HPKE_SUITE, b"", b"info_hash", info);
    let mut context = Vec::with_capacity(1 + KEY_LEN + KEY_LEN);
    context.push(0);
    context.extend_from_slice(&psk_id_hash);
    context.extend_from_slice(&info_hash);
    let mut secret = labeled_extract(HPKE_SUITE, shared, b"secret", b"");
    let mut key = [0u8; KEY_LEN];
    let mut nonce = [0u8; 12];
    labeled_expand(HPKE_SUITE, &secret, b"key", &context, &mut key)?;
    labeled_expand(HPKE_SUITE, &secret, b"base_nonce", &context, &mut nonce)?;
    secret.zeroize();
    context.zeroize();
    Ok((key, nonce))
}

fn hpke_seal(
    recipient: &[u8; KEY_LEN],
    info: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    ephemeral: Option<[u8; KEY_LEN]>,
) -> Result<([u8; KEY_LEN], Vec<u8>), SyncKeyError> {
    let mut secret = match ephemeral {
        Some(secret) => secret,
        None => random_bytes()?,
    };
    let enc = x25519_public(&secret);
    let shared = encap_shared(&secret, recipient)?;
    secret.zeroize();
    let (mut key, nonce) = key_schedule(&shared, info)?;
    let ciphertext = aead_seal(&key, &nonce, aad, plaintext)?;
    key.zeroize();
    Ok((enc, ciphertext))
}

fn hpke_open(
    recipient_secret: &[u8; KEY_LEN],
    enc: &[u8; KEY_LEN],
    info: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, SyncKeyError> {
    let shared = decap_shared(recipient_secret, enc)?;
    let (mut key, nonce) = key_schedule(&shared, info)?;
    let plain = aead_open(&key, &nonce, aad, ciphertext)?;
    key.zeroize();
    Ok(plain)
}

fn encap_shared(
    ephemeral: &[u8; KEY_LEN],
    recipient: &[u8; KEY_LEN],
) -> Result<[u8; KEY_LEN], SyncKeyError> {
    let mut dh_secret = dh(ephemeral, recipient)?;
    let enc = x25519_public(ephemeral);
    let mut context = [0u8; 64];
    context[..32].copy_from_slice(&enc);
    context[32..].copy_from_slice(recipient);
    let shared = extract_and_expand(&dh_secret, &context)?;
    dh_secret.zeroize();
    Ok(shared)
}

fn decap_shared(
    secret: &[u8; KEY_LEN],
    enc: &[u8; KEY_LEN],
) -> Result<[u8; KEY_LEN], SyncKeyError> {
    let mut dh_secret = dh(secret, enc)?;
    let public = x25519_public(secret);
    let mut context = [0u8; 64];
    context[..32].copy_from_slice(enc);
    context[32..].copy_from_slice(&public);
    let shared = extract_and_expand(&dh_secret, &context)?;
    dh_secret.zeroize();
    Ok(shared)
}

fn aead_seal(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, SyncKeyError> {
    let cipher = ChaCha20Poly1305::new_from_slice(key).map_err(|_| SyncKeyError::Corrupt)?;
    let nonce = Nonce::from_slice(nonce);
    cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| SyncKeyError::Corrupt)
}

fn aead_open(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, SyncKeyError> {
    let cipher = ChaCha20Poly1305::new_from_slice(key).map_err(|_| SyncKeyError::Corrupt)?;
    let nonce = Nonce::from_slice(nonce);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| SyncKeyError::Sealed)
}

#[cfg(test)]
fn derive_x25519(ikm: &[u8]) -> [u8; KEY_LEN] {
    let extracted = labeled_extract(KEM_SUITE, b"", b"dkp_prk", ikm);
    let mut secret = [0u8; KEY_LEN];
    labeled_expand(KEM_SUITE, &extracted, b"sk", b"", &mut secret).unwrap();
    secret
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
            .collect()
    }

    fn array<const N: usize>(text: &str) -> [u8; N] {
        unhex(text).try_into().unwrap()
    }

    #[test]
    fn the_word_list_is_2048_easy_words_and_one_typo_is_unique() {
        let list = words();
        assert_eq!(list.len(), 2048);
        assert!(list.iter().all(|word| {
            (5..=8).contains(&word.len()) && word.bytes().all(|byte| byte.is_ascii_lowercase())
        }));
        for (index, word) in list.iter().enumerate() {
            for other in &list[index + 1..] {
                if word.len().abs_diff(other.len()) >= 3 {
                    continue;
                }
                assert!(
                    levenshtein(word, other) >= 3,
                    "{word} and {other} are too close"
                );
            }
        }
    }

    fn levenshtein(left: &str, right: &str) -> usize {
        let right = right.as_bytes();
        let mut previous: Vec<usize> = (0..=right.len()).collect();
        for (i, byte) in left.bytes().enumerate() {
            let mut current = vec![i + 1];
            for (j, other) in right.iter().enumerate() {
                let cost = usize::from(byte != *other);
                let insert = current[j] + 1;
                let delete = previous[j + 1] + 1;
                let substitute = previous[j] + cost;
                current.push(insert.min(delete).min(substitute));
            }
            previous = current;
        }
        previous[right.len()]
    }

    #[test]
    fn words_round_trip_and_a_small_typo_is_forgiven() {
        let entropy = [0x42u8; 16];
        let key = RecoveryKey::from_entropy(entropy);
        let shown = key.words();
        assert!(shown.iter().all(|word| !word.is_empty()));
        let parsed = RecoveryKey::parse(&shown.join(" ")).unwrap();
        assert_eq!(parsed.words(), shown);
        assert_eq!(parsed.entropy().unwrap(), entropy);

        let mut parts: Vec<String> = shown.iter().map(|word| (*word).to_string()).collect();
        let mut typo = parts[4].as_bytes().to_vec();
        let last = typo.len() - 1;
        typo[last] = if typo[last] == b'a' { b'b' } else { b'a' };
        parts[4] = String::from_utf8(typo).unwrap();
        parts[0] = parts[0].to_uppercase();
        let sloppy = format!("  {}   {} ", parts[0], parts[1..].join("  "));
        assert_eq!(RecoveryKey::parse(&sloppy).unwrap().words(), shown);

        let replacement = words()
            .iter()
            .copied()
            .find(|word| *word != shown[0] && !edit_distance_1(shown[0], word))
            .unwrap();
        parts[0] = replacement.to_string();
        parts[4] = shown[4].to_string();
        assert_eq!(
            RecoveryKey::parse(&parts.join(" ")).unwrap_err(),
            SyncKeyError::NotAKey
        );
        assert_eq!(
            RecoveryKey::parse("one two").unwrap_err(),
            SyncKeyError::NotAKey
        );
        assert_eq!(suggest(&shown[0].to_uppercase()), vec![shown[0]]);
        assert!(suggest("").is_empty());
        assert!(suggest("   ").is_empty());
        assert!(suggest("zzzzzz").is_empty());

        let third = shown[2].to_uppercase();
        let seventh = shown[6].to_uppercase();
        let eleventh = shown[10].to_uppercase();
        let confirmed = key
            .confirm(&[
                (3, third.as_str()),
                (7, seventh.as_str()),
                (11, eleventh.as_str()),
            ])
            .unwrap();
        assert_eq!(confirmed.entropy, entropy);
        assert!(
            key.confirm(&[(3, shown[0]), (7, shown[6]), (11, shown[10])])
                .is_err()
        );
        assert!(
            key.confirm(&[(3, shown[2]), (3, shown[2]), (7, shown[6])])
                .is_err()
        );
        assert!(key.confirm(&[(3, shown[2])]).is_err());
        assert_eq!(format!("{key:?}"), "RecoveryKey(12 words)");
        assert!(!format!("{key:?}").contains(shown[0]));

        let mut wipe = RecoveryKey::from_entropy(entropy);
        wipe.zeroize();
        assert_eq!(wipe.index, [0u16; WORD_COUNT]);
    }

    #[test]
    fn a_pasted_sheet_drops_numbering_commas_and_case() {
        let key = RecoveryKey::from_entropy([0x42; 16]);
        let shown = key.words();
        let pasted = format!(
            "1. {}\n2. {}, {}\r\n{}\n{} {}\n6. {}\n{}\n{}\n{}\n10. {}\n{}",
            shown[0].to_uppercase(),
            capitalize(shown[1]),
            shown[2],
            shown[3],
            shown[4],
            shown[5],
            shown[6],
            shown[7],
            shown[8],
            shown[9],
            shown[10],
            shown[11],
        );
        let normalized = normalize_phrase(&pasted);
        assert_eq!(normalized, shown.join(" "));
        assert_eq!(RecoveryKey::parse(&normalized).unwrap().words(), shown);

        let stuck = format!("1.{} 2){}", shown[0], shown[1]);
        assert_eq!(
            normalize_phrase(&stuck),
            format!("{} {}", shown[0], shown[1])
        );
        assert_eq!(normalize_phrase("1. 2. 3."), "");
    }

    fn capitalize(word: &str) -> String {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }

    #[test]
    fn hpke_matches_the_published_base_mode_vector() {
        // RFC 9180 appendix A.2.1: DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20Poly1305.
        let ikm_e: [u8; 32] =
            array("909a9b35d3dc4713a5e72a4da274b55d3d3821a37e5d099e74a647db583a904b");
        let secret = derive_x25519(&ikm_e);
        assert_eq!(
            secret,
            array::<32>("f4ec9b33b792c372c1d2c2063507b684ef925b8c75a42dbcbf57d63ccd381600")
        );
        assert_eq!(
            x25519_public(&secret),
            array::<32>("1afa08d3dec047a643885163f1180476fa7ddb54c6a8029ea33f95796bf2ac4a")
        );
        let recipient_secret: [u8; 32] =
            array("8057991eef8f1f1af18f4a9491d16a1ce333f695d4db8e38da75975c4478e0fb");
        let recipient = x25519_public(&recipient_secret);
        assert_eq!(
            recipient,
            array::<32>("4310ee97d88cc1f088a5576c77ab0cf5c3ac797f3d95139c6c84b5429c59662a")
        );
        let info = unhex("4f6465206f6e2061204772656369616e2055726e");
        let shared = encap_shared(&secret, &recipient).unwrap();
        assert_eq!(
            shared,
            array::<32>("0bbe78490412b4bbea4812666f7916932b828bba79942424abb65244930d69a7")
        );
        let (key, nonce) = key_schedule(&shared, &info).unwrap();
        assert_eq!(
            key,
            array::<32>("ad2744de8e17f4ebba575b3f5f5a8fa1f69c2a07f6e7500bc60ca6e3e3ec1c91")
        );
        assert_eq!(nonce, array::<12>("5c4d98150661b848853b547f"));
        let plaintext = unhex("4265617574792069732074727574682c20747275746820626561757479");
        let aad = unhex("436f756e742d30");
        let (enc, ciphertext) =
            hpke_seal(&recipient, &info, &aad, &plaintext, Some(secret)).unwrap();
        assert_eq!(
            enc,
            array::<32>("1afa08d3dec047a643885163f1180476fa7ddb54c6a8029ea33f95796bf2ac4a")
        );
        assert_eq!(
            ciphertext,
            unhex(
                "1c5250d8034ec2b784ba2cfd69dbdb8af406cfe3ff938e131f0def8c8b\
                 60b4db21993c62ce81883d2dd1b51a28"
            )
        );
        let opened = hpke_open(&recipient_secret, &enc, &info, &aad, &ciphertext).unwrap();
        assert_eq!(opened, plaintext);
        let mut wrong = recipient_secret;
        wrong[0] ^= 0xff;
        assert!(hpke_open(&wrong, &enc, &info, &aad, &ciphertext).is_err());
    }

    #[test]
    fn a_server_copy_does_not_hold_the_key_or_the_page() {
        let recovery = RecoveryKey::from_entropy([0x42; 16]);
        let shown = recovery.words();
        let confirmed = recovery
            .confirm(&[(3, shown[2]), (7, shown[6]), (11, shown[10])])
            .unwrap();
        let mut root = confirmed.root("").unwrap();
        let other = confirmed.root("blue notebook").unwrap();
        assert_ne!(root.0, other.0);
        let account = root.account_public().unwrap();
        assert_ne!(
            account_fingerprint(&account),
            account_fingerprint(&other.account_public().unwrap())
        );
        assert_eq!(account_fingerprint(&account), account_fingerprint(&account));

        let laptop = DeviceKeys::generate().unwrap();
        let chain = found_devices(&laptop).unwrap();
        let code = pairing_code(&laptop.public());
        assert_eq!(code.len(), 7);
        assert_eq!(code.as_bytes()[3], b' ');
        assert_eq!(code, pairing_code(&chain.lists[0].devices[0]));
        let mut swapped = laptop.public();
        swapped.receive[0] ^= 1;
        assert_ne!(code, pairing_code(&swapped));

        let vault = VaultKey::generate().unwrap();
        let epoch = 1;
        let phrase = wrap_vault_for_phrase(&root, &vault, epoch).unwrap();
        let laptop_wrap = wrap_vault_for_device(&laptop.public(), &vault, epoch).unwrap();
        let page = b"Trip Budget\nferry = 40\nmilk $3";
        let sealed = seal_page(&vault, epoch, b"trip", page).unwrap();

        let phone = DeviceKeys::generate().unwrap();
        let (with_phone, phone_wrap) =
            approve(&laptop, &chain, &phone.public(), &vault, epoch).unwrap();
        let (again, again_wrap) = approve(&laptop, &chain, &phone.public(), &vault, epoch).unwrap();
        assert_eq!(with_phone.lists[1].signature, again.lists[1].signature);
        assert_ne!(phone_wrap, again_wrap);
        let other_vault = VaultKey::generate().unwrap();
        let (same_list, other_wrap) =
            approve(&laptop, &chain, &phone.public(), &other_vault, epoch).unwrap();
        assert_eq!(with_phone.lists[1].signature, same_list.lists[1].signature);
        assert_ne!(phone_wrap, other_wrap);
        assert!(!holds(&with_phone.lists[1].message(), &vault.key));
        assert!(!holds(&with_phone.lists[1].message(), &root.0));

        let opened = open_device_wrap(&phone, &phone_wrap, epoch, vault.id()).unwrap();
        assert_eq!(opened.key, vault.key);
        assert_eq!(open_page(&opened, epoch, b"trip", &sealed).unwrap(), page);
        assert!(open_page(&opened, epoch, b"other", &sealed).is_err());
        assert_eq!(
            open_page(
                &open_device_wrap(&laptop, &laptop_wrap, epoch, vault.id()).unwrap(),
                epoch,
                b"trip",
                &sealed
            )
            .unwrap(),
            page
        );
        assert!(open_device_wrap(&phone, &laptop_wrap, epoch, vault.id()).is_err());
        assert!(open_device_wrap(&phone, &phone_wrap, epoch + 1, vault.id()).is_err());
        assert_eq!(
            open_phrase_wrap(&root, &phrase, epoch, vault.id())
                .unwrap()
                .key,
            vault.key
        );
        assert!(open_phrase_wrap(&other, &phrase, epoch, vault.id()).is_err());

        let mut forged = with_phone.clone();
        forged.lists[1].signature[0] ^= 0xff;
        assert_eq!(forged.verify().unwrap_err(), SyncKeyError::NotApproved);
        let mut raw = with_phone.to_bytes();
        let parsed = DeviceChain::from_bytes(&raw).unwrap();
        parsed.verify().unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        assert!(DeviceChain::from_bytes(&raw).unwrap().verify().is_err());
        let mut sneaky = chain.clone();
        sneaky.lists[0].devices.push(phone.public());
        assert!(sneaky.verify().is_err());

        let view = ServerView {
            epoch,
            account,
            chain: with_phone,
            phrase_wrap: phrase,
            device_wraps: vec![laptop_wrap, phone_wrap],
            key_id: *vault.id(),
            page: sealed,
        };
        let stored = view.stored();
        let read = ServerView::parse(&stored).unwrap();
        read.chain.verify().unwrap();
        assert_eq!(read.account, account);
        assert_eq!(read.key_id, *vault.id());
        for secret in [
            root.0.as_slice(),
            other.0.as_slice(),
            vault.key.as_slice(),
            other_vault.key.as_slice(),
            laptop.sign.as_slice(),
            laptop.receive.as_slice(),
            phone.sign.as_slice(),
            phone.receive.as_slice(),
            confirmed.entropy.as_slice(),
            page.as_slice(),
            b"blue notebook".as_slice(),
        ] {
            assert!(!holds(&stored, secret));
        }
        for word in shown {
            assert!(!holds(&stored, word.as_bytes()), "{word}");
        }
        assert_eq!(format!("{root:?}"), "Root(..)");
        assert_eq!(format!("{laptop:?}"), "DeviceKeys(..)");
        assert_eq!(format!("{vault:?}"), "VaultKey(..)");
        assert!(!format!("{root:?}").contains("42"));
        assert_ne!(root.0, [0u8; KEY_LEN]);
        root.zeroize();
        assert_eq!(root.0, [0u8; KEY_LEN]);
    }

    #[test]
    fn a_device_file_round_trips_and_is_not_a_mailbox_record() {
        let device = DeviceKeys::generate().unwrap();
        let vault = VaultKey::generate().unwrap();
        let account = [0x11u8; 32];
        let chain = found_devices(&device).unwrap().to_bytes();
        let stored = store_device(&device, &vault, &account, 1, &chain).unwrap();
        assert!(stored.starts_with(DEVICE_MAGIC));
        assert!(!stored.starts_with(b"OMACUS-SYNC/1\n"));
        let opened = open_device_store(&stored).unwrap();
        assert_eq!(opened.device.id, device.id);
        assert_eq!(opened.device.sign, device.sign);
        assert_eq!(opened.device.receive, device.receive);
        assert_eq!(opened.vault.id, vault.id);
        assert_eq!(opened.vault.key, vault.key);
        assert_eq!(opened.account, account);
        assert_eq!(opened.epoch, 1);
        assert_eq!(opened.chain, chain);
        let mut torn = stored.clone();
        let last = torn.len() - 1;
        torn[last] ^= 0xff;
        assert!(open_device_store(&torn).is_err());
    }

    #[test]
    fn removing_a_device_keeps_the_rest_and_a_bad_list_fails() {
        let laptop = DeviceKeys::generate().unwrap();
        let phone = DeviceKeys::generate().unwrap();
        let tablet = DeviceKeys::generate().unwrap();
        let vault = VaultKey::generate().unwrap();
        let chain = found_devices(&laptop).unwrap();
        let (with_phone, _) = approve(&laptop, &chain, &phone.public(), &vault, 1).unwrap();
        let (with_both, _) = approve(&phone, &with_phone, &tablet.public(), &vault, 1).unwrap();
        assert_eq!(with_both.compared_with(&with_phone), ChainOrder::Newer);
        let removed = remove_device(&laptop, &with_both, phone.public().id()).unwrap();
        removed.verify().unwrap();
        let ids: Vec<_> = removed
            .devices()
            .unwrap()
            .iter()
            .map(|device| device.id)
            .collect();
        assert!(ids.contains(&laptop.id));
        assert!(ids.contains(&tablet.id));
        assert!(!ids.contains(&phone.id));
        assert_eq!(removed.compared_with(&with_both), ChainOrder::Newer);
        assert!(remove_device(&phone, &removed, tablet.public().id()).is_err());
        assert!(remove_device(&laptop, &removed, laptop.public().id()).is_err());
        assert!(
            approve(
                &phone,
                &removed,
                &DeviceKeys::generate().unwrap().public(),
                &vault,
                1
            )
            .is_err()
        );
    }

    #[test]
    fn merging_pages_adds_and_never_drops_a_line() {
        let local = PageSet {
            doc: b"local-doc".to_vec(),
            pages: vec![
                ("trip".to_string(), "Trip Budget\nferry = 40\n".to_string()),
                ("note".to_string(), "cake $5\n".to_string()),
            ],
        };
        let remote = PageSet {
            doc: b"remote-doc".to_vec(),
            pages: vec![
                ("trip".to_string(), "Trip Budget\nmilk $3\n".to_string()),
                ("market".to_string(), "bread $2\n".to_string()),
            ],
        };
        let merged = local.merge(remote);
        assert_eq!(merged.doc, b"local-doc");
        let trip = merged
            .pages
            .iter()
            .find(|(id, _)| id == "trip")
            .unwrap()
            .1
            .as_str();
        assert!(trip.contains("ferry = 40"), "{trip}");
        assert!(trip.contains("milk $3"), "{trip}");
        assert!(merged.pages.iter().any(|(id, _)| id == "note"));
        assert!(merged.pages.iter().any(|(id, _)| id == "market"));
        let kept = merge_page_texts(
            &[("trip".to_string(), "ferry = 40\nmilk $3\n".to_string())],
            &[("trip".to_string(), "ferry = 40\n".to_string())],
        );
        assert!(kept[0].1.contains("milk $3"));
        let locked = merge_page_texts(
            &[("trip".to_string(), "OMACUS-LOCK/1\nlocal\n".to_string())],
            &[("trip".to_string(), "OMACUS-LOCK/1\nremote\n".to_string())],
        );
        assert_eq!(locked[0].1, "OMACUS-LOCK/1\nlocal\n");
        assert!(locked.iter().any(|(_, text)| text.contains("remote")));
        let bytes = merged.to_bytes().unwrap();
        assert_eq!(
            PageSet::from_bytes(&bytes).unwrap().pages.len(),
            merged.pages.len()
        );
        assert!(bytes.starts_with(PAGES_MAGIC));
    }
}
