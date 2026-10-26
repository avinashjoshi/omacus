//! The same short code the relay checks. The page keeps the code. The relay keeps a slow hash.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

pub const SECRET_LEN: usize = 8;
pub const CODE_CHARS: usize = 12;
pub const CODE_KDF_ITERS: u32 = 20_000;
const CODE_KDF_SALT: &[u8] = b"omacus-join-v1";

type HmacSha256 = Hmac<Sha256>;

pub fn canonical(secret: &[u8; SECRET_LEN]) -> [u8; SECRET_LEN] {
    let mut out = *secret;
    out[0] &= 0x0f;
    out
}

pub fn encode_code(secret: &[u8; SECRET_LEN]) -> String {
    let secret = canonical(secret);
    let mut value = 0u64;
    for byte in secret {
        value = (value << 8) | u64::from(byte);
    }
    let mut chars = ['0'; CODE_CHARS];
    for slot in (0..CODE_CHARS).rev() {
        chars[slot] = ALPHABET[(value & 31) as usize] as char;
        value >>= 5;
    }
    format!(
        "{}-{}-{}",
        chars[..4].iter().collect::<String>(),
        chars[4..8].iter().collect::<String>(),
        chars[8..].iter().collect::<String>()
    )
}

pub fn decode_code(text: &str) -> Option<[u8; SECRET_LEN]> {
    let mut cleaned = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() || ch == '-' {
            continue;
        }
        let mapped = match ch.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            other => other,
        };
        cleaned.push(mapped);
    }
    if cleaned.len() != CODE_CHARS {
        return None;
    }
    let mut value = 0u64;
    for ch in cleaned.chars() {
        let index = ALPHABET.iter().position(|byte| *byte as char == ch)?;
        value = (value << 5) | index as u64;
    }
    let mut out = [0u8; SECRET_LEN];
    for slot in (0..SECRET_LEN).rev() {
        out[slot] = (value & 0xff) as u8;
        value >>= 8;
    }
    Some(canonical(&out))
}

pub fn hash_hex(secret: &[u8]) -> String {
    let mut owned = [0u8; SECRET_LEN];
    let bytes = if secret.len() == SECRET_LEN {
        owned.copy_from_slice(secret);
        owned = canonical(&owned);
        pbkdf2_sha256(&owned, CODE_KDF_SALT, CODE_KDF_ITERS)
    } else {
        pbkdf2_sha256(secret, CODE_KDF_SALT, CODE_KDF_ITERS)
    };
    hex(&bytes)
}

pub fn proof(secret: &[u8], device: &[u8]) -> [u8; 32] {
    hmac_sha256(secret, &[device])
}

pub fn proof_matches(secret: &[u8], device: &[u8], given: &[u8]) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
        return false;
    };
    mac.update(device);
    mac.verify_slice(given).is_ok()
}

fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac accepts this key");
    for part in parts {
        mac.update(part);
    }
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}

fn pbkdf2_sha256(password: &[u8], salt: &[u8], iters: u32) -> [u8; 32] {
    let mut block = Vec::with_capacity(salt.len() + 4);
    block.extend_from_slice(salt);
    block.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_sha256(password, &[&block]);
    let mut t = u;
    for _ in 1..iters.max(1) {
        u = hmac_sha256(password, &[&u]);
        for (slot, byte) in t.iter_mut().enumerate() {
            *byte ^= u[slot];
        }
    }
    t
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_code_matches_the_relay() {
        let secret = [1u8, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(encode_code(&secret), "20G3-0G2G-C1R8");
        assert_eq!(
            hash_hex(&secret),
            "6afc99534f22743635cd2fef5b3220f2193a8b75f15fe3601bc9266859f9f1a6"
        );
        let mac = proof(&secret, &[7u8; 80]);
        assert_eq!(
            hex(&mac),
            "20553d58135273ffe333faffbc1632d38acb36a703c76f8c2ae3af0f470d0404"
        );
        assert!(proof_matches(&secret, &[7u8; 80], &mac));
    }
}
