//! A one-time code that points a new device at one account.
//!
//! The code is the only copy of the secret. What gets stored is a hash of
//! those bytes. A join also carries a check value made from the new device's
//! public bytes, so a request with no live code never becomes a pending device.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

pub const INVITE_MS: u64 = 5 * 60 * 1000;
pub const SECRET_LEN: usize = 5;

type HmacSha256 = Hmac<Sha256>;

pub fn encode_code(secret: &[u8; SECRET_LEN]) -> String {
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
    if cleaned.len() != 8 {
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
    Some(out)
}

pub fn hash_secret(secret: &[u8]) -> [u8; 32] {
    Sha256::digest(secret).into()
}

pub fn hash_hex(secret: &[u8]) -> String {
    hex(&hash_secret(secret))
}

pub fn parse_hash(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (slot, chunk) in text.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).ok()?;
        out[slot] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
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
    fn a_short_code_round_trips_and_matches_the_check_value() {
        let secret = [1u8, 2, 3, 4, 5];
        assert_eq!(encode_code(&secret), "0410-6105");
        assert_eq!(decode_code("0410-6105"), Some(secret));
        assert_eq!(decode_code("04106105"), Some(secret));
        assert_eq!(decode_code("o410-6105"), Some(secret));
        assert_eq!(
            hash_hex(&secret),
            "74f81fe167d99b4cb41d6d0ccda82278caee9f3e2f25d5e5a3936ff3dcec60d0"
        );
        let device = [7u8; 80];
        let mac = proof(&secret, &device);
        assert_eq!(
            hex(&mac),
            "afafdd003f0810b9ea25a10e555b13e163697cec5710fa8cc39f9f497b9f6b4a"
        );
        assert!(proof_matches(&secret, &device, &mac));
        assert!(!proof_matches(&secret, &device, &[0u8; 32]));
        assert!(decode_code("nope").is_none());
        let rfc = proof(&[0x0b; 20], b"Hi There");
        assert_eq!(
            hex(&rfc),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }
}
