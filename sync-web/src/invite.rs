//! The same short code the relay checks. The page keeps the code. The relay keeps a hash.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

type HmacSha256 = Hmac<Sha256>;

pub fn encode_code(secret: &[u8; 5]) -> String {
    let mut value = 0u64;
    for byte in secret {
        value = (value << 8) | u64::from(*byte);
    }
    let mut chars = ['0'; 8];
    for slot in (0..8).rev() {
        chars[slot] = ALPHABET[(value & 31) as usize] as char;
        value >>= 5;
    }
    format!(
        "{}-{}",
        chars[..4].iter().collect::<String>(),
        chars[4..].iter().collect::<String>()
    )
}

pub fn decode_code(text: &str) -> Option<[u8; 5]> {
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
    if cleaned.len() != 8 {
        return None;
    }
    let mut value = 0u64;
    for ch in cleaned.chars() {
        let index = ALPHABET.iter().position(|byte| *byte as char == ch)?;
        value = (value << 5) | index as u64;
    }
    let mut out = [0u8; 5];
    for slot in (0..5).rev() {
        out[slot] = (value & 0xff) as u8;
        value >>= 8;
    }
    Some(out)
}

pub fn hash_hex(secret: &[u8]) -> String {
    hex(&Sha256::digest(secret))
}

pub fn proof(secret: &[u8], device: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts this key");
    mac.update(device);
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}

pub fn proof_matches(secret: &[u8], device: &[u8], given: &[u8]) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
        return false;
    };
    mac.update(device);
    mac.verify_slice(given).is_ok()
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
        let secret = [1u8, 2, 3, 4, 5];
        assert_eq!(encode_code(&secret), "0410-6105");
        assert_eq!(
            hash_hex(&secret),
            "74f81fe167d99b4cb41d6d0ccda82278caee9f3e2f25d5e5a3936ff3dcec60d0"
        );
        let mac = proof(&secret, &[7u8; 80]);
        assert_eq!(
            hex(&mac),
            "afafdd003f0810b9ea25a10e555b13e163697cec5710fa8cc39f9f497b9f6b4a"
        );
        assert!(proof_matches(&secret, &[7u8; 80], &mac));
    }
}
