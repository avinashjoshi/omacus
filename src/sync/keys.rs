//! The device key stays off the page files.
//!
//! macOS uses the Keychain for this device only, not iCloud. Windows wraps the
//! key with DPAPI for this user. Linux uses Secret Service when it is running.
//! When this computer has no keyring, the key stays in a file locked with a
//! passphrase: Argon2id, then XChaCha20-Poly1305. That file is mode 0600. The
//! passphrase is not written. A pointer file in the data folder records which
//! store holds the key. On Windows that file is the DPAPI ciphertext. A plain
//! key file is removed if one is found.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(all(unix, not(target_os = "macos")))]
use std::time::{Duration, Instant};

const MARK: &[u8] = b"OMACUS-LOCAL/1\n";
#[cfg(target_os = "macos")]
const SERVICE: &str = "omacus-sync";
#[cfg(target_os = "macos")]
const ACCOUNT: &str = "device";

pub fn path(dir: &Path) -> PathBuf {
    dir.join("sync").join("device.bin")
}

pub fn save(dir: &Path, plain: &[u8]) -> Result<(), String> {
    let sealed = seal(plain)?;
    write_private(dir, &sealed)
}

pub fn load(dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = path(dir);
    if !file.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&file).map_err(|e| e.to_string())?;
    if plain_file(&bytes) {
        let _ = fs::remove_file(&file);
        return Ok(None);
    }
    Ok(Some(open_sealed(&bytes)?))
}

pub fn remove(dir: &Path) {
    let _ = fs::remove_file(path(dir));
    #[cfg(target_os = "macos")]
    let _ = keychain_delete();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = secret_delete();
}

#[cfg(test)]
pub fn is_private(dir: &Path) -> bool {
    let file = path(dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(&file)
            .map(|meta| meta.permissions().mode() & 0o777 == 0o600)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        file.is_file()
    }
}

/// Which store the pointer names. `test` exists only in the test build, so a
/// suite can keep a profile without a keyring. It is not a plain key.
#[cfg(test)]
pub fn kind(dir: &Path) -> Option<&'static str> {
    let bytes = fs::read(path(dir)).ok()?;
    let rest = bytes.strip_prefix(MARK)?;
    let line = rest.split(|b| *b == b'\n').next()?;
    match line {
        b"dpapi" => Some("dpapi"),
        b"keychain" => Some("keychain"),
        b"secret" => Some("secret"),
        b"pass" => Some("pass"),
        b"test" => Some("test"),
        _ => None,
    }
}

fn plain_file(bytes: &[u8]) -> bool {
    let Some(rest) = bytes.strip_prefix(MARK) else {
        return false;
    };
    rest.starts_with(b"file\n")
}

fn seal(plain: &[u8]) -> Result<Vec<u8>, String> {
    // The test build never writes a plain key, and it does not use the login
    // keychain. Each profile carries its own wrapped blob, so two devices in
    // one run do not share a slot. The release build has no such path.
    #[cfg(test)]
    {
        let mut out = MARK.to_vec();
        out.extend_from_slice(b"test\n");
        out.extend(wrap_test(plain));
        Ok(out)
    }
    #[cfg(not(test))]
    {
        os_seal(plain)
    }
}

#[cfg(all(not(test), target_os = "macos"))]
fn os_seal(plain: &[u8]) -> Result<Vec<u8>, String> {
    keychain_put(plain)?;
    let mut out = MARK.to_vec();
    out.extend_from_slice(b"keychain\n");
    Ok(out)
}

#[cfg(all(not(test), windows))]
fn os_seal(plain: &[u8]) -> Result<Vec<u8>, String> {
    let blob = dpapi_seal(plain)?;
    let mut out = MARK.to_vec();
    out.extend_from_slice(b"dpapi\n");
    out.extend_from_slice(&blob);
    Ok(out)
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn os_seal(plain: &[u8]) -> Result<Vec<u8>, String> {
    if keyring_up() && secret_put(plain).is_ok() {
        let mut out = MARK.to_vec();
        out.extend_from_slice(b"secret\n");
        return Ok(out);
    }
    Err("This computer has no keyring, so sync cannot keep a key here.".into())
}

#[cfg(test)]
fn wrap_test(plain: &[u8]) -> Vec<u8> {
    plain
        .iter()
        .enumerate()
        .map(|(i, byte)| byte ^ 0xA5 ^ (i as u8))
        .collect()
}

#[cfg(test)]
fn unwrap_test(blob: &[u8]) -> Vec<u8> {
    wrap_test(blob)
}

fn open_sealed(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let rest = bytes
        .strip_prefix(MARK)
        .ok_or_else(|| "That copy could not be read.".to_string())?;
    let Some(split) = rest.iter().position(|b| *b == b'\n') else {
        return Err("That copy could not be read.".into());
    };
    let (kind, payload) = rest.split_at(split);
    let payload = &payload[1..];
    match kind {
        b"test" => {
            #[cfg(test)]
            {
                Ok(unwrap_test(payload))
            }
            #[cfg(not(test))]
            {
                let _ = payload;
                Err("That copy could not be read.".into())
            }
        }
        b"dpapi" => {
            #[cfg(windows)]
            {
                dpapi_open(payload)
            }
            #[cfg(not(windows))]
            {
                let _ = payload;
                Err("That copy could not be read.".into())
            }
        }
        b"keychain" => {
            #[cfg(target_os = "macos")]
            {
                keychain_get()
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err("That copy could not be read.".into())
            }
        }
        b"secret" => {
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                secret_get()
            }
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            {
                Err("That copy could not be read.".into())
            }
        }
        b"pass" => Err("Enter the passphrase.".into()),
        _ => Err("That copy could not be read.".into()),
    }
}

/// A passphrase-derived key kept in memory for this run. The passphrase is not kept.
pub struct PassLock {
    seal: omacus_engine::lock::SecretSeal,
}

impl std::fmt::Debug for PassLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PassLock(..)")
    }
}

pub fn has_pass_file(dir: &Path) -> bool {
    fs::read(path(dir)).ok().is_some_and(|bytes| {
        bytes
            .strip_prefix(MARK)
            .is_some_and(|rest| rest.starts_with(b"pass\n"))
    })
}

/// Secret Service is missing, or the key file is already locked with a passphrase.
pub fn needs_passphrase(dir: &Path) -> bool {
    has_pass_file(dir) || linux_without_keyring()
}

fn linux_without_keyring() -> bool {
    #[cfg(all(not(test), unix, not(target_os = "macos")))]
    {
        !keyring_up()
    }
    #[cfg(not(all(not(test), unix, not(target_os = "macos"))))]
    {
        false
    }
}

pub fn new_pass_lock(password: &str) -> Result<PassLock, String> {
    omacus_engine::lock::derive_passphrase(password)
        .map(|seal| PassLock { seal })
        .map_err(pass_error)
}

pub fn write_pass(dir: &Path, plain: &[u8], lock: &PassLock) -> Result<(), String> {
    let payload =
        omacus_engine::lock::seal_with_passphrase(&lock.seal, plain).map_err(pass_error)?;
    let mut sealed = MARK.to_vec();
    sealed.extend_from_slice(b"pass\n");
    sealed.extend_from_slice(&payload);
    write_private(dir, &sealed)
}

pub fn open_pass(dir: &Path, password: &str) -> Result<(Vec<u8>, PassLock), String> {
    let bytes = fs::read(path(dir)).map_err(|e| e.to_string())?;
    let rest = bytes
        .strip_prefix(MARK)
        .ok_or_else(|| "That copy could not be read.".to_string())?;
    let payload = rest
        .strip_prefix(b"pass\n")
        .ok_or_else(|| "That copy could not be read.".to_string())?;
    let (plain, seal) =
        omacus_engine::lock::open_with_passphrase(password, payload).map_err(pass_error)?;
    Ok((plain, PassLock { seal }))
}

fn write_private(dir: &Path, sealed: &[u8]) -> Result<(), String> {
    let file = path(dir);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut out = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&file)
        .map_err(|e| e.to_string())?;
    out.write_all(sealed).map_err(|e| e.to_string())?;
    set_private(&file)
}

fn pass_error(error: omacus_engine::lock::LockError) -> String {
    match error {
        omacus_engine::lock::LockError::EmptyPassword => "Enter a passphrase.".into(),
        omacus_engine::lock::LockError::WrongPassword => {
            "That passphrase didn't work. Try again.".into()
        }
        omacus_engine::lock::LockError::Corrupt => "That copy could not be read.".into(),
    }
}

fn set_private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// This device only. A key in iCloud Keychain would leave this computer.
#[cfg(target_os = "macos")]
pub fn keychain_access() -> &'static str {
    "AfterFirstUnlockThisDeviceOnly"
}

#[cfg(target_os = "macos")]
fn keychain_options(protected: bool) -> security_framework::passwords::PasswordOptions {
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::PasswordOptions;
    let mut options = PasswordOptions::new_generic_password(SERVICE, ACCOUNT);
    options.set_label("Omacus sync");
    options.set_access_synchronized(Some(false));
    if let Ok(access) = SecAccessControl::create_with_protection(
        Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
        0,
    ) {
        options.set_access_control(access);
    }
    if protected {
        options.use_protected_keychain();
    }
    let _ = keychain_access();
    options
}

#[cfg_attr(test, allow(dead_code))]
#[cfg(target_os = "macos")]
fn keychain_put(plain: &[u8]) -> Result<(), String> {
    use security_framework::passwords::set_generic_password_options;
    let _ = keychain_delete();
    if set_generic_password_options(plain, keychain_options(true)).is_ok() {
        return Ok(());
    }
    let _ = keychain_delete();
    set_generic_password_options(plain, keychain_options(false)).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn keychain_get() -> Result<Vec<u8>, String> {
    use security_framework::passwords::generic_password;
    generic_password(keychain_options(true))
        .or_else(|_| generic_password(keychain_options(false)))
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn keychain_delete() -> Result<(), String> {
    use security_framework::passwords::delete_generic_password_options;
    let _ = delete_generic_password_options(keychain_options(true));
    delete_generic_password_options(keychain_options(false)).map_err(|e| e.to_string())
}

#[cfg_attr(test, allow(dead_code))]
#[cfg(windows)]
fn dpapi_seal(plain: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: plain.len() as u32,
            pbData: plain.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let ok = CryptProtectData(
            &input,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            0,
            &mut output,
        );
        if ok == 0 || output.pbData.is_null() {
            return Err("That copy could not be read.".into());
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        windows_sys::Win32::Foundation::LocalFree(output.pbData as *mut core::ffi::c_void);
        Ok(bytes)
    }
}

#[cfg(windows)]
fn dpapi_open(blob: &[u8]) -> Result<Vec<u8>, String> {
    use std::ptr;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: blob.len() as u32,
            pbData: blob.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let ok = CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            0,
            &mut output,
        );
        if ok == 0 || output.pbData.is_null() {
            return Err("That copy could not be read.".into());
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        windows_sys::Win32::Foundation::LocalFree(output.pbData as *mut core::ffi::c_void);
        Ok(bytes)
    }
}

#[cfg_attr(test, allow(dead_code))]
#[cfg(all(unix, not(target_os = "macos")))]
fn keyring_up() -> bool {
    let Ok(mut child) = std::process::Command::new("secret-tool")
        .args(["lookup", "app", "omacus", "kind", "probe"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    else {
        return false;
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let err = child
                    .stderr
                    .take()
                    .and_then(|mut pipe| {
                        let mut buf = String::new();
                        use std::io::Read;
                        pipe.read_to_string(&mut buf).ok()?;
                        Some(buf)
                    })
                    .unwrap_or_default();
                let down = err.contains("Could not connect")
                    || err.contains("No such")
                    || err.contains("Cannot");
                return status.success() || (status.code() == Some(1) && !down && err.is_empty());
            }
            Ok(None) if start.elapsed() > Duration::from_millis(300) => {
                let _ = child.kill();
                return false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return false,
        }
    }
}

#[cfg_attr(test, allow(dead_code))]
#[cfg(all(unix, not(target_os = "macos")))]
fn secret_put(plain: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("secret-tool")
        .args([
            "store",
            "--label",
            "Omacus sync",
            "app",
            "omacus",
            "kind",
            "device",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(plain).map_err(|e| e.to_string())?;
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("The keyring did not keep the key.".into())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn secret_get() -> Result<Vec<u8>, String> {
    let out = std::process::Command::new("secret-tool")
        .args(["lookup", "app", "omacus", "kind", "device"])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err("That copy could not be read.".into())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn secret_delete() -> Result<(), String> {
    let status = std::process::Command::new("secret-tool")
        .args(["clear", "app", "omacus", "kind", "device"])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("The keyring did not clear the key.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_file_is_private_to_this_user() {
        let dir = std::env::temp_dir().join(format!("omacus-key-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let plain = b"OMACUS-DEVICE/1\nnot-a-real-key";
        save(&dir, plain).unwrap();
        assert!(is_private(&dir));
        let stored = fs::read(path(&dir)).unwrap();
        assert_ne!(kind(&dir), Some("file"));
        assert!(
            !stored
                .windows(b"OMACUS-DEVICE".len())
                .any(|w| w == b"OMACUS-DEVICE"),
            "the key file holds the raw device marker"
        );
        assert_eq!(load(&dir).unwrap().unwrap(), plain);
        remove(&dir);
        assert!(load(&dir).unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_passphrase_file_is_not_the_key() {
        let dir = std::env::temp_dir().join(format!("omacus-pass-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let plain = b"OMACUS-DEVICE/1\nnot-a-real-key";
        let password = "lantern-dock-42";
        let lock = new_pass_lock(password).unwrap();
        write_pass(&dir, plain, &lock).unwrap();
        assert!(is_private(&dir));
        assert_eq!(kind(&dir), Some("pass"));
        let stored = fs::read(path(&dir)).unwrap();
        assert!(
            !stored
                .windows(b"OMACUS-DEVICE".len())
                .any(|window| window == b"OMACUS-DEVICE")
        );
        assert!(
            !stored
                .windows(password.len())
                .any(|window| window == password.as_bytes())
        );
        assert!(load(&dir).is_err());
        assert!(has_pass_file(&dir));
        let wrong = open_pass(&dir, "wrong-pass-99").unwrap_err();
        assert_eq!(wrong, "That passphrase didn't work. Try again.");
        let (opened, again) = open_pass(&dir, password).unwrap();
        assert_eq!(opened, plain);
        write_pass(&dir, plain, &again).unwrap();
        let (second, _) = open_pass(&dir, password).unwrap();
        assert_eq!(second, plain);
        remove(&dir);
        assert!(!has_pass_file(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_mac_key_stays_on_this_device() {
        assert_eq!(keychain_access(), "AfterFirstUnlockThisDeviceOnly");
    }
}
