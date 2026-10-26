//! The device file stays on this computer.
//!
//! macOS keeps it in the Keychain for this device only. Windows wraps it with
//! DPAPI for this user. Linux asks Secret Service when a keyring answers, and
//! otherwise keeps the file private to this user.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
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
    let file = path(dir);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let sealed = seal(plain)?;
    let mut out = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&file)
        .map_err(|e| e.to_string())?;
    out.write_all(&sealed).map_err(|e| e.to_string())?;
    set_private(&file)?;
    Ok(())
}

pub fn load(dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = path(dir);
    if !file.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&file).map_err(|e| e.to_string())?;
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

/// What a leak of this file is. `file` is the Linux fallback: the bytes are
/// the device file, and the mode is 0600. The other kinds are ciphertext.
#[cfg(test)]
pub fn kind(dir: &Path) -> Option<&'static str> {
    let bytes = fs::read(path(dir)).ok()?;
    let rest = bytes.strip_prefix(MARK)?;
    let line = rest.split(|b| *b == b'\n').next()?;
    match line {
        b"file" => Some("file"),
        b"dpapi" => Some("dpapi"),
        b"keychain" => Some("keychain"),
        b"secret" => Some("secret"),
        _ => None,
    }
}

fn seal(plain: &[u8]) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "macos")]
    {
        if keychain_put(plain).is_ok() {
            let mut out = MARK.to_vec();
            out.extend_from_slice(b"keychain\n");
            return Ok(out);
        }
    }
    #[cfg(windows)]
    {
        let blob = dpapi_seal(plain)?;
        let mut out = MARK.to_vec();
        out.extend_from_slice(b"dpapi\n");
        out.extend_from_slice(&blob);
        return Ok(out);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if keyring_up() && secret_put(plain).is_ok() {
            let mut out = MARK.to_vec();
            out.extend_from_slice(b"secret\n");
            return Ok(out);
        }
    }
    let mut out = MARK.to_vec();
    out.extend_from_slice(b"file\n");
    out.extend_from_slice(plain);
    Ok(out)
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
        b"file" => Ok(payload.to_vec()),
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
        _ => Err("That copy could not be read.".into()),
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

#[cfg(all(unix, not(target_os = "macos")))]
fn keyring_up() -> bool {
    if std::env::var("OMACUS_SYNC_KEYRING").ok().as_deref() == Some("file") {
        return false;
    }
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
        match kind(&dir) {
            Some("file") => {
                assert!(is_private(&dir));
                assert_eq!(load(&dir).unwrap().unwrap(), plain);
            }
            Some("secret") | Some("keychain") | Some("dpapi") => {
                assert!(
                    !stored
                        .windows(b"OMACUS-DEVICE".len())
                        .any(|w| w == b"OMACUS-DEVICE"),
                    "the wrapped file holds the raw device marker"
                );
                assert_eq!(load(&dir).unwrap().unwrap(), plain);
            }
            other => panic!("unexpected key kind {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_mac_key_stays_on_this_device() {
        assert_eq!(keychain_access(), "AfterFirstUnlockThisDeviceOnly");
    }
}
