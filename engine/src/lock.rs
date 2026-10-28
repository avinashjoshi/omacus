//! Password locks for pages and folders.
//!
//! A random content key is wrapped with a key derived by Argon2id (a fresh salt
//! per lock) and the page text is encrypted with XChaCha20-Poly1305. The same
//! content key can wrap again under a new password without a second derivation
//! of the old one. Plaintext is not part of the stored form.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine, engine::general_purpose::STANDARD};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroize;

/// Interactive Argon2id: 19 MiB, 2 iterations, one lane (OWASP's lower bound).
const MEMORY_KIB: u32 = 19 * 1024;
const ITERATIONS: u32 = 2;
const LANES: u32 = 1;

/// The device-key file. Heavier than a page lock: 64 MiB and 3 passes.
const DEVICE_MEMORY_KIB: u32 = 64 * 1024;
const DEVICE_ITERATIONS: u32 = 3;

const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

pub const HEADER: &str = "OMACUS-LOCK/1";

/// Reserved for a future sync wrap around an already-encrypted page file.
/// The line is not page text, so a file that starts with it stays closed.
pub const SYNC_HEADER: &str = "OMACUS-SYNC/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockError {
    EmptyPassword,
    WrongPassword,
    Corrupt,
}

impl LockError {
    pub fn message(self) -> &'static str {
        match self {
            LockError::EmptyPassword => "Enter a password.",
            LockError::WrongPassword => "Wrong password.",
            LockError::Corrupt => "That lock could not be read.",
        }
    }
}

/// A content key. Debug output does not include the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct ContentKey([u8; KEY_LEN]);

impl std::fmt::Debug for ContentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ContentKey(..)")
    }
}

impl ContentKey {
    pub fn encode(&self) -> String {
        STANDARD.encode(self.0)
    }

    pub fn decode(text: &str) -> Result<Self, LockError> {
        let bytes = STANDARD
            .decode(text.trim())
            .map_err(|_| LockError::Corrupt)?;
        let bytes: [u8; KEY_LEN] = bytes.try_into().map_err(|_| LockError::Corrupt)?;
        Ok(ContentKey(bytes))
    }
}

/// The Argon2 salt and the wrapped content key. Pages in one locked folder share it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wrap {
    pub salt: String,
    pub nonce: String,
    pub wrapped: String,
    pub memory_kib: u32,
    pub iterations: u32,
    pub lanes: u32,
}

/// A new content key, wrapped for `password`.
pub fn create(password: &str) -> Result<(ContentKey, Wrap), LockError> {
    let key = ContentKey(random_array()?);
    let wrap = wrap_key(password, &key)?;
    Ok((key, wrap))
}

/// Wrap an existing content key for a new password.
pub fn wrap_key(password: &str, key: &ContentKey) -> Result<Wrap, LockError> {
    let salt = random_vec(SALT_LEN)?;
    let kek = derive(password, &salt, MEMORY_KIB, ITERATIONS, LANES)?;
    let nonce = random_vec(NONCE_LEN)?;
    let wrapped = xcrypt(true, &kek, &nonce, b"omacus-key", &key.0)?;
    Ok(Wrap {
        salt: STANDARD.encode(salt),
        nonce: STANDARD.encode(nonce),
        wrapped: STANDARD.encode(wrapped),
        memory_kib: MEMORY_KIB,
        iterations: ITERATIONS,
        lanes: LANES,
    })
}

/// Encrypt `plaintext`. `aad` binds the ciphertext to the page id so two pages
/// that share a folder key cannot be swapped.
pub fn seal(
    key: &ContentKey,
    plaintext: &str,
    wrap: &Wrap,
    aad: &[u8],
) -> Result<String, LockError> {
    let nonce = random_vec(NONCE_LEN)?;
    let data = xcrypt(true, &key.0, &nonce, aad, plaintext.as_bytes())?;
    Ok(format!(
        "{HEADER}\nm={}\nt={}\np={}\nsalt={}\nwrap={}\nkey={}\nnonce={}\ndata={}\n",
        wrap.memory_kib,
        wrap.iterations,
        wrap.lanes,
        wrap.salt,
        wrap.nonce,
        wrap.wrapped,
        STANDARD.encode(&nonce),
        STANDARD.encode(&data),
    ))
}

pub fn open_key(key: &ContentKey, stored: &str, aad: &[u8]) -> Result<String, LockError> {
    let parsed = parse(stored)?;
    let plain = xcrypt(false, &key.0, &parsed.nonce, aad, &parsed.data)?;
    String::from_utf8(plain).map_err(|_| LockError::Corrupt)
}

pub fn unwrap_key(password: &str, wrap: &Wrap) -> Result<ContentKey, LockError> {
    let salt = STANDARD
        .decode(&wrap.salt)
        .map_err(|_| LockError::Corrupt)?;
    let kek = derive(
        password,
        &salt,
        wrap.memory_kib,
        wrap.iterations,
        wrap.lanes,
    )?;
    let nonce = STANDARD
        .decode(&wrap.nonce)
        .map_err(|_| LockError::Corrupt)?;
    let wrapped = STANDARD
        .decode(&wrap.wrapped)
        .map_err(|_| LockError::Corrupt)?;
    let plain = xcrypt(false, &kek, &nonce, b"omacus-key", &wrapped)?;
    let bytes: [u8; KEY_LEN] = plain.try_into().map_err(|_| LockError::Corrupt)?;
    Ok(ContentKey(bytes))
}

/// Unlock a stored page: Argon2 once, then the content key for later saves.
pub fn open_password(
    password: &str,
    stored: &str,
    aad: &[u8],
) -> Result<(String, ContentKey), LockError> {
    let wrap = wrap_of(stored)?;
    let key = unwrap_key(password, &wrap)?;
    let text = open_key(&key, stored, aad)?;
    Ok((text, key))
}

const PASS_AAD: &[u8] = b"omacus-device-pass/1";

/// A key derived from a passphrase. The passphrase is not kept.
pub struct SecretSeal {
    salt: [u8; SALT_LEN],
    kek: [u8; KEY_LEN],
}

impl Drop for SecretSeal {
    fn drop(&mut self) {
        self.kek.zeroize();
    }
}

impl std::fmt::Debug for SecretSeal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretSeal(..)")
    }
}

/// Argon2id once, at the device-key settings. Later seals reuse this key and salt.
pub fn derive_passphrase(password: &str) -> Result<SecretSeal, LockError> {
    let salt = random_array()?;
    let kek = derive_device(password, &salt, DEVICE_MEMORY_KIB, DEVICE_ITERATIONS, LANES)?;
    Ok(SecretSeal { salt, kek })
}

/// Encrypt `plain`. A new nonce each time. The salt and the device-key parameters travel with the ciphertext.
pub fn seal_with_passphrase(seal: &SecretSeal, plain: &[u8]) -> Result<Vec<u8>, LockError> {
    let nonce = random_array::<NONCE_LEN>()?;
    let data = xcrypt(true, &seal.kek, &nonce, PASS_AAD, plain)?;
    let mut out = Vec::with_capacity(12 + SALT_LEN + NONCE_LEN + data.len());
    out.extend_from_slice(&DEVICE_MEMORY_KIB.to_be_bytes());
    out.extend_from_slice(&DEVICE_ITERATIONS.to_be_bytes());
    out.extend_from_slice(&LANES.to_be_bytes());
    out.extend_from_slice(&seal.salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&data);
    Ok(out)
}

/// Open a passphrase seal. A weaker parameter set is refused.
pub fn open_with_passphrase(
    password: &str,
    stored: &[u8],
) -> Result<(Vec<u8>, SecretSeal), LockError> {
    let header = 12 + SALT_LEN + NONCE_LEN;
    if stored.len() < header + 16 {
        return Err(LockError::Corrupt);
    }
    let memory = u32::from_be_bytes(stored[0..4].try_into().map_err(|_| LockError::Corrupt)?);
    let iterations = u32::from_be_bytes(stored[4..8].try_into().map_err(|_| LockError::Corrupt)?);
    let lanes = u32::from_be_bytes(stored[8..12].try_into().map_err(|_| LockError::Corrupt)?);
    let salt: [u8; SALT_LEN] = stored[12..12 + SALT_LEN]
        .try_into()
        .map_err(|_| LockError::Corrupt)?;
    let nonce_at = 12 + SALT_LEN;
    let nonce: [u8; NONCE_LEN] = stored[nonce_at..nonce_at + NONCE_LEN]
        .try_into()
        .map_err(|_| LockError::Corrupt)?;
    let data = &stored[header..];
    let mut kek = derive_device(password, &salt, memory, iterations, lanes)?;
    let plain = match xcrypt(false, &kek, &nonce, PASS_AAD, data) {
        Ok(plain) => plain,
        Err(error) => {
            kek.zeroize();
            return Err(error);
        }
    };
    Ok((plain, SecretSeal { salt, kek }))
}

pub fn is_lock(text: &str) -> bool {
    text.starts_with(HEADER) && text.as_bytes().get(HEADER.len()) == Some(&b'\n')
}

/// A page lock, or an outer sync envelope around one. Either way the bytes
/// are not the page, and they must not be shown as the page.
pub fn is_envelope(text: &str) -> bool {
    is_lock(text)
        || text
            .lines()
            .next()
            .is_some_and(|line| line.starts_with(SYNC_HEADER))
}

pub fn wrap_of(stored: &str) -> Result<Wrap, LockError> {
    let parsed = parse(stored)?;
    Ok(parsed.wrap)
}

struct Parsed {
    wrap: Wrap,
    nonce: Vec<u8>,
    data: Vec<u8>,
}

fn parse(stored: &str) -> Result<Parsed, LockError> {
    if !is_lock(stored) {
        return Err(LockError::Corrupt);
    }
    let mut memory_kib = 0u32;
    let mut iterations = 0u32;
    let mut lanes = 0u32;
    let mut salt = None;
    let mut wrap_nonce = None;
    let mut wrapped = None;
    let mut nonce = None;
    let mut data = None;
    for line in stored.lines().skip(1) {
        if line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(LockError::Corrupt);
        };
        match k {
            "m" => memory_kib = v.parse().map_err(|_| LockError::Corrupt)?,
            "t" => iterations = v.parse().map_err(|_| LockError::Corrupt)?,
            "p" => lanes = v.parse().map_err(|_| LockError::Corrupt)?,
            "salt" => salt = Some(v.to_string()),
            "wrap" => wrap_nonce = Some(v.to_string()),
            "key" => wrapped = Some(v.to_string()),
            "nonce" => nonce = Some(STANDARD.decode(v).map_err(|_| LockError::Corrupt)?),
            "data" => data = Some(STANDARD.decode(v).map_err(|_| LockError::Corrupt)?),
            _ => {}
        }
    }
    if memory_kib != MEMORY_KIB || iterations != ITERATIONS || lanes != LANES {
        return Err(LockError::Corrupt);
    }
    let (Some(salt), Some(wrap_nonce), Some(wrapped), Some(nonce), Some(data)) =
        (salt, wrap_nonce, wrapped, nonce, data)
    else {
        return Err(LockError::Corrupt);
    };
    if STANDARD
        .decode(&salt)
        .map(|s| s.len() != SALT_LEN)
        .unwrap_or(true)
        || STANDARD
            .decode(&wrap_nonce)
            .map(|n| n.len() != NONCE_LEN)
            .unwrap_or(true)
    {
        return Err(LockError::Corrupt);
    }
    Ok(Parsed {
        wrap: Wrap {
            salt,
            nonce: wrap_nonce,
            wrapped,
            memory_kib,
            iterations,
            lanes,
        },
        nonce,
        data,
    })
}

fn derive(
    password: &str,
    salt: &[u8],
    memory_kib: u32,
    iterations: u32,
    lanes: u32,
) -> Result<[u8; KEY_LEN], LockError> {
    if memory_kib != MEMORY_KIB || iterations != ITERATIONS || lanes != LANES {
        return Err(LockError::Corrupt);
    }
    hash_password(password, salt, memory_kib, iterations, lanes)
}

/// The device-key file only. Page-lock parameters, and anything weaker, are refused.
fn derive_device(
    password: &str,
    salt: &[u8],
    memory_kib: u32,
    iterations: u32,
    lanes: u32,
) -> Result<[u8; KEY_LEN], LockError> {
    if memory_kib != DEVICE_MEMORY_KIB || iterations != DEVICE_ITERATIONS || lanes != LANES {
        return Err(LockError::Corrupt);
    }
    hash_password(password, salt, memory_kib, iterations, lanes)
}

fn hash_password(
    password: &str,
    salt: &[u8],
    memory_kib: u32,
    iterations: u32,
    lanes: u32,
) -> Result<[u8; KEY_LEN], LockError> {
    if password.is_empty() {
        return Err(LockError::EmptyPassword);
    }
    if salt.len() != SALT_LEN {
        return Err(LockError::Corrupt);
    }
    let params = Params::new(memory_kib, iterations, lanes, Some(KEY_LEN))
        .map_err(|_| LockError::Corrupt)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|_| LockError::Corrupt)?;
    Ok(out)
}

fn xcrypt(
    encrypting: bool,
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    input: &[u8],
) -> Result<Vec<u8>, LockError> {
    if key.len() != KEY_LEN || nonce.len() != NONCE_LEN {
        return Err(LockError::Corrupt);
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| LockError::Corrupt)?;
    let nonce = XNonce::try_from(nonce).map_err(|_| LockError::Corrupt)?;
    let payload = Payload { msg: input, aad };
    let result = if encrypting {
        cipher.encrypt(&nonce, payload)
    } else {
        cipher.decrypt(&nonce, payload)
    };
    result.map_err(|_| {
        if encrypting {
            LockError::Corrupt
        } else {
            LockError::WrongPassword
        }
    })
}

fn random_vec(len: usize) -> Result<Vec<u8>, LockError> {
    let mut buf = vec![0u8; len];
    getrandom::getrandom(&mut buf).map_err(|_| LockError::Corrupt)?;
    Ok(buf)
}

fn random_array<const N: usize>() -> Result<[u8; N], LockError> {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).map_err(|_| LockError::Corrupt)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ciphertext from chacha20poly1305 0.10.1 for this key, nonce, and page.
    #[test]
    fn old_xchacha_bytes_still_seal_and_open() {
        let key = [0x11u8; KEY_LEN];
        let nonce = [0x22u8; NONCE_LEN];
        let aad = b"omacus-page";
        let plain = b"ferry = 40\n";
        let old = "e502957e2599d9b4d17d1ee14d784ab0abd92acb006af361a7dbd9";
        let old: Vec<u8> = (0..old.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&old[index..index + 2], 16).unwrap())
            .collect();
        assert_eq!(xcrypt(true, &key, &nonce, aad, plain).unwrap(), old);
        assert_eq!(xcrypt(false, &key, &nonce, aad, &old).unwrap(), plain);
    }

    #[test]
    fn a_page_round_trips_and_the_file_has_no_plaintext() {
        let (stored, key) = {
            let (key, wrap) = create("correct horse").unwrap();
            let stored = seal(&key, "# Groceries\nmilk $3", &wrap, b"page-1").unwrap();
            (stored, key)
        };
        assert!(is_lock(&stored));
        assert!(!stored.contains("Groceries"), "{stored}");
        assert!(!stored.contains("milk"), "{stored}");
        let opened = open_key(&key, &stored, b"page-1").unwrap();
        assert_eq!(opened, "# Groceries\nmilk $3");
        let (text, again) = open_password("correct horse", &stored, b"page-1").unwrap();
        assert_eq!(text, "# Groceries\nmilk $3");
        assert_eq!(again, key);
    }

    #[test]
    fn a_wrong_password_does_not_open_the_page() {
        let (key, wrap) = create("right").unwrap();
        let stored = seal(&key, "Weekend Plans", &wrap, b"page").unwrap();
        assert_eq!(
            open_password("wrong", &stored, b"page").unwrap_err(),
            LockError::WrongPassword
        );
        assert_eq!(
            open_password("", &stored, b"page").unwrap_err(),
            LockError::EmptyPassword
        );
    }

    #[test]
    fn a_tampered_page_does_not_open() {
        let (key, wrap) = create("trip").unwrap();
        let stored = seal(&key, "Trip Budget\nferry $40", &wrap, b"page").unwrap();
        let mut chars: Vec<u8> = stored.into_bytes();
        let at = chars.len() - 4;
        chars[at] ^= 0x1;
        let bad = String::from_utf8(chars).unwrap();
        assert!(open_password("trip", &bad, b"page").is_err());
    }

    #[test]
    fn the_same_folder_key_opens_a_page_made_later() {
        let (key, wrap) = create("folder-secret").unwrap();
        let stored = seal(&key, "Groceries\neggs $2", &wrap, b"eggs").unwrap();
        let again = unwrap_key("folder-secret", &wrap).unwrap();
        assert_eq!(
            open_key(&again, &stored, b"eggs").unwrap(),
            "Groceries\neggs $2"
        );
        assert_eq!(
            unwrap_key("nope", &wrap).unwrap_err(),
            LockError::WrongPassword
        );
        let changed = wrap_key("new-secret", &key).unwrap();
        assert!(unwrap_key("folder-secret", &changed).is_err());
        assert_eq!(unwrap_key("new-secret", &changed).unwrap(), key);
    }

    #[test]
    fn a_sync_envelope_is_not_page_text() {
        let (key, wrap) = create("pw").unwrap();
        let inner = seal(&key, "# Groceries\nmilk $3", &wrap, b"page-a").unwrap();
        let outer = format!("{SYNC_HEADER}1\n{inner}");
        assert!(is_envelope(&outer));
        assert!(!is_lock(&outer));
        assert!(is_envelope(&inner));
    }

    #[test]
    fn a_weaker_device_seal_is_refused() {
        let mut stored = vec![0u8; 12 + 16 + 24 + 16];
        stored[0..4].copy_from_slice(&MEMORY_KIB.to_be_bytes());
        stored[4..8].copy_from_slice(&ITERATIONS.to_be_bytes());
        stored[8..12].copy_from_slice(&LANES.to_be_bytes());
        assert_eq!(
            open_with_passphrase("lantern-dock-42", &stored).unwrap_err(),
            LockError::Corrupt
        );
    }

    #[test]
    fn ciphertext_does_not_open_under_a_different_page_id() {
        let (key, wrap) = create("pw").unwrap();
        let stored = seal(&key, "Weekend Plans", &wrap, b"page-a").unwrap();
        assert!(open_key(&key, &stored, b"page-b").is_err());
    }
}
