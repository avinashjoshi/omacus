//! An invite code that may create one new set of pages.
//!
//! The code is checked with a secret that stays in the process. What gets
//! stored is a hash of the code and how many uses are left, never the code.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const MARK: &[u8] = b"omacus-access/1";
const ID_LEN: usize = 6;
const TAG_LEN: usize = 8;
const RAW_LEN: usize = ID_LEN + 1 + TAG_LEN;
const CHAR_LEN: usize = 24;
pub const MIN_SECRET: usize = 16;
pub const MAX_USES: u8 = 20;

type HmacSha256 = Hmac<Sha256>;

pub struct Opened {
    pub hash: [u8; 32],
    pub uses: u8,
}

pub fn mint(secret: &[u8], uses: u8, id: &[u8; ID_LEN]) -> Option<String> {
    if secret.len() < MIN_SECRET || !(1..=MAX_USES).contains(&uses) {
        return None;
    }
    let tag = tag(secret, id, uses)?;
    let mut raw = [0u8; RAW_LEN];
    raw[..ID_LEN].copy_from_slice(id);
    raw[ID_LEN] = uses;
    raw[ID_LEN + 1..].copy_from_slice(&tag);
    Some(group(&encode(&raw)))
}

pub fn open(secret: &[u8], code: &str) -> Option<Opened> {
    if secret.len() < MIN_SECRET {
        return None;
    }
    let raw = decode(code)?;
    let mut id = [0u8; ID_LEN];
    id.copy_from_slice(&raw[..ID_LEN]);
    let uses = raw[ID_LEN];
    if !(1..=MAX_USES).contains(&uses) {
        return None;
    }
    let tag = tag(secret, &id, uses)?;
    if tag != raw[ID_LEN + 1..] {
        return None;
    }
    let normalized = normalize(code)?;
    Some(Opened {
        hash: Sha256::digest(normalized.as_bytes()).into(),
        uses,
    })
}

fn tag(secret: &[u8], id: &[u8], uses: u8) -> Option<[u8; TAG_LEN]> {
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(MARK);
    mac.update(id);
    mac.update(&[uses]);
    let full = mac.finalize().into_bytes();
    let mut out = [0u8; TAG_LEN];
    out.copy_from_slice(&full[..TAG_LEN]);
    Some(out)
}

fn encode(bytes: &[u8; RAW_LEN]) -> String {
    let mut bits = 0u128;
    for byte in bytes {
        bits = (bits << 8) | u128::from(*byte);
    }
    let mut chars = ['0'; CHAR_LEN];
    for slot in (0..CHAR_LEN).rev() {
        chars[slot] = ALPHABET[(bits & 31) as usize] as char;
        bits >>= 5;
    }
    chars.iter().collect()
}

fn decode(text: &str) -> Option<[u8; RAW_LEN]> {
    let cleaned = normalize(text)?;
    let mut bits = 0u128;
    for ch in cleaned.chars() {
        let index = ALPHABET.iter().position(|byte| *byte as char == ch)?;
        bits = (bits << 5) | index as u128;
    }
    let mut out = [0u8; RAW_LEN];
    for slot in (0..RAW_LEN).rev() {
        out[slot] = (bits & 0xff) as u8;
        bits >>= 8;
    }
    Some(out)
}

fn normalize(text: &str) -> Option<String> {
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
    if cleaned.len() != CHAR_LEN {
        return None;
    }
    Some(cleaned)
}

fn group(raw: &str) -> String {
    raw.chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"omacus-access-test-secret";

    #[test]
    fn a_code_round_trips_and_a_changed_use_does_not() {
        let id = [1u8, 2, 3, 4, 5, 6];
        let code = mint(SECRET, 1, &id).unwrap();
        assert_eq!(code, "0410-6105-0R0N-S1ZK-97T3-78B1");
        let opened = open(SECRET, &code).unwrap();
        assert_eq!(opened.uses, 1);
        let hash: String = opened
            .hash
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            hash,
            "171c9e446cd1b267ec7309325edc0cefa1f9f169f869a0aee36a2de0f00f3502"
        );
        assert!(open(SECRET, "nope").is_none());
        assert!(open(b"short", &code).is_none());
        let mut chars: Vec<char> = code.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == '0' { '1' } else { '0' };
        let wrong: String = chars.into_iter().collect();
        assert!(open(SECRET, &wrong).is_none());
    }
}
