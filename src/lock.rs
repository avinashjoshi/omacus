//! Opening a locked page from the command line and the MCP server.
//!
//! The page file is the lock. Argon2id derives a key from the password and
//! XChaCha20-Poly1305 opens the text. A password, or a hash of one, is never
//! stored. A future sync layer may wrap that file a second time; each
//! envelope is peeled, and a layer this build cannot open is not printed.

use omacus_engine::lock::{self, LockError};

/// A page lock, then a sync wrap around it. Anything still sealed after that
/// is not page text.
const MAX_LAYERS: usize = 2;

pub fn is_locked(body: &str) -> bool {
    lock::is_envelope(body)
}

/// The page text. The error never includes the locked bytes.
pub fn reveal(id: &str, body: &str, password: Option<&str>) -> Result<String, String> {
    let mut current = body.to_string();
    for _ in 0..MAX_LAYERS {
        if !lock::is_envelope(&current) {
            return Ok(current);
        }
        let Some(password) = password else {
            return Err(refuse(false));
        };
        current = peel(&current, password, id)?;
    }
    if lock::is_envelope(&current) {
        Err(refuse(password.is_some()))
    } else {
        Ok(current)
    }
}

/// Message when a locked page is not opened. `had_password` is the attempt.
pub fn refuse(had_password: bool) -> String {
    if had_password {
        "This page is locked. That password did not open it.".into()
    } else {
        "This page is locked. Pass --password to open it.".into()
    }
}

fn peel(body: &str, password: &str, id: &str) -> Result<String, String> {
    if lock::is_lock(body) {
        return lock::open_password(password, body, id.as_bytes())
            .map(|(text, _key)| text)
            .map_err(error);
    }
    // `OMACUS-SYNC/` wraps an already-encrypted page. Peel that outer layer
    // here once the format exists. Until then the file stays closed, so
    // neither the wrapper nor the page lock inside it is printed.
    Err(refuse(true))
}

fn error(err: LockError) -> String {
    match err {
        LockError::EmptyPassword => "Enter a password.".into(),
        LockError::WrongPassword => refuse(true),
        LockError::Corrupt => "That lock could not be read.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seal(password: &str, id: &str, text: &str) -> String {
        let (key, wrap) = lock::create(password).unwrap();
        lock::seal(&key, text, &wrap, id.as_bytes()).unwrap()
    }

    #[test]
    fn the_file_opens_only_with_the_password() {
        let stored = seal("open-sesame", "page-1", "# Groceries\nmilk $3\n");
        assert!(is_locked(&stored));
        assert!(!stored.contains("Groceries"), "{stored}");
        assert!(!stored.contains("milk"), "{stored}");
        let err = reveal("page-1", &stored, None).unwrap_err();
        assert!(err.contains("locked"), "{err}");
        assert!(!err.contains("milk") && !err.contains("Groceries"), "{err}");
        let err = reveal("page-1", &stored, Some("nope")).unwrap_err();
        assert!(err.contains("locked"), "{err}");
        assert!(!err.contains("milk"), "{err}");
        let text = reveal("page-1", &stored, Some("open-sesame")).unwrap();
        assert_eq!(text, "# Groceries\nmilk $3\n");
    }

    #[test]
    fn a_second_lock_layer_is_not_printed() {
        let inner = seal("open-sesame", "page-1", "# Weekend Plans\nhike\n");
        let outer = seal("open-sesame", "page-1", &inner);
        assert!(outer.starts_with("OMACUS-LOCK/1\n"), "{outer}");
        assert!(!outer.contains("Weekend"), "{outer}");
        assert!(!outer.contains("hike"), "{outer}");
        let err = reveal("page-1", &outer, None).unwrap_err();
        assert!(
            !err.contains("Weekend") && !err.contains("OMACUS-LOCK"),
            "{err}"
        );
        let text = reveal("page-1", &outer, Some("open-sesame")).unwrap();
        assert_eq!(text, "# Weekend Plans\nhike\n");
        let err = reveal("page-1", &outer, Some("nope")).unwrap_err();
        assert!(!err.contains("Weekend") && !err.contains("hike"), "{err}");
    }

    #[test]
    fn a_sync_wrap_stays_closed() {
        let inner = seal("open-sesame", "page-1", "# Trip Budget\nferry $40\n");
        let outer = format!("{}1\n{inner}", lock::SYNC_HEADER);
        for password in [None, Some("open-sesame")] {
            let err = reveal("page-1", &outer, password).unwrap_err();
            assert!(err.contains("locked"), "{err}");
            assert!(!err.contains("ferry") && !err.contains("Trip"), "{err}");
            assert!(!err.contains("OMACUS-LOCK"), "{err}");
        }
    }
}
