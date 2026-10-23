//! Whether a page may be shown.
//!
//! The desktop app may mark a page locked. Until that format is the one this
//! module recognizes, a lock we do not know how to open stays closed: the
//! command line and the MCP server never print the page text.
//!
//! Recognized records, beside the page or under `locks` / `locked` /
//! `passwords` in `state.json`:
//! - an object with `salt` and `hash` (hex SHA-256 of the salt bytes followed
//!   by the password bytes)
//! - a string, the password itself
//!
//! Anything else, including a lock with no secret, does not open.

use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::sheets::heading_of;

#[derive(Default)]
struct Record {
    salt: Option<String>,
    hash: Option<String>,
    /// A password stored as itself. Only used when `hash` is absent.
    password: Option<String>,
}

impl Record {
    fn opens(&self, password: &str) -> bool {
        if let (Some(salt), Some(hash)) = (&self.salt, &self.hash) {
            let mut hasher = Sha256::new();
            hasher.update(salt.as_bytes());
            hasher.update(password.as_bytes());
            return hex(&hasher.finalize()) == hash.to_ascii_lowercase();
        }
        self.password.as_deref().is_some_and(|p| p == password)
    }

    fn from_value(value: &Value) -> Self {
        match value {
            Value::String(password) => Self {
                password: Some(password.clone()),
                ..Self::default()
            },
            Value::Object(_) => Self {
                salt: text_field(value, &["salt"]),
                hash: text_field(value, &["hash", "password_hash", "sha256"]),
                password: None,
            },
            _ => Self::default(),
        }
    }
}

fn text_field(value: &Value, keys: &[&str]) -> Option<String> {
    let obj = value.as_object()?;
    keys.iter().find_map(|key| {
        obj.get(*key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.is_empty())
    })
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

/// `Some(message)` when `text` must not be printed. `password` opens the page
/// only when the lock record can be checked and it matches.
pub fn refusal(pages_dir: &Path, id: &str, text: &str, password: Option<&str>) -> Option<String> {
    let record = find(pages_dir, id)?;
    if password.is_some_and(|p| record.opens(p)) {
        return None;
    }
    let title = heading_of(text);
    Some(if password.is_some() {
        format!("\"{title}\" is locked. That password did not open it.")
    } else {
        format!("\"{title}\" is locked. Pass --password to open it.")
    })
}

/// Whether a lock record names this page. Listing may still show the title.
pub fn is_locked(pages_dir: &Path, id: &str) -> bool {
    find(pages_dir, id).is_some()
}

fn find(pages_dir: &Path, id: &str) -> Option<Record> {
    for name in [format!("{id}.lock"), format!("{id}.txt.lock")] {
        let path = pages_dir.join(name);
        if path.is_file() {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            return Some(parse_record(&text));
        }
    }
    from_state(&pages_dir.with_file_name("state.json"), id)
}

fn parse_record(text: &str) -> Record {
    serde_json::from_str::<Value>(text)
        .map(|value| Record::from_value(&value))
        .unwrap_or_default()
}

fn from_state(path: &Path, id: &str) -> Option<Record> {
    let bytes = std::fs::read(path).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    for key in ["locks", "locked", "passwords"] {
        match &value[key] {
            Value::Object(map) => {
                if let Some(item) = map.get(id) {
                    return Some(Record::from_value(item));
                }
            }
            Value::Array(items) => {
                for item in items {
                    let named = item.as_str() == Some(id)
                        || item.get("id").and_then(Value::as_str) == Some(id);
                    if named {
                        return Some(if item.is_object() {
                            Record::from_value(item)
                        } else {
                            Record::default()
                        });
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Hex SHA-256 a lock file can store. Tests use it to build a record.
#[cfg(test)]
pub fn password_hash(salt: &str, password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(password.as_bytes());
    hex(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_opens_only_with_the_password() {
        let record = Record {
            salt: Some("s".into()),
            hash: Some(password_hash("s", "correct")),
            password: None,
        };
        assert!(record.opens("correct"));
        assert!(!record.opens("nope"));
    }
}
