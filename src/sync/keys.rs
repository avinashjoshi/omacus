//! The device key stays off the page files.
//!
//! macOS uses the Keychain for this device only, not iCloud. Windows wraps the
//! key with DPAPI for this user. Linux uses Secret Service when a default or
//! login collection is unlocked. If the key store is running but nothing is
//! unlocked, setup creates the passwordless default keyring once and checks it
//! through Secret Service. A passphrase file is only used when someone chooses
//! it: Argon2id at 64 MiB and 3 passes, then XChaCha20-Poly1305. That file is
//! created mode 0600 and renamed into place, so a crash leaves the previous
//! key. The passphrase is not written. A pointer file in the data folder
//! records which store holds the key. On Windows that file is the DPAPI
//! ciphertext. A plain key file is removed if one is found.

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

/// The desktop binary uses this to show whether the key store accepted a write.
/// The bytes are a marker, not a device key, and they are not printed.
pub fn e2e_store(dir: &Path) -> i32 {
    if std::env::var("OMACUS_E2E_PREPARE").ok().as_deref() == Some("1")
        && let Err(error) = prepare_keyring()
    {
        println!("{}", e2e_label(&error));
        return 0;
    }
    match save(dir, b"omacus-e2e") {
        Ok(()) => {
            println!("stored");
            0
        }
        Err(error) => {
            println!("{}", e2e_label(&error));
            0
        }
    }
}

fn e2e_label(error: &str) -> &'static str {
    if wants_key_setup(error) {
        "setup"
    } else if error == "The key store took too long." {
        "timeout"
    } else {
        "fallback"
    }
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
    if !collection_unlocked() {
        return Err(SETUP_REASON.into());
    }
    secret_put(plain)?;
    let mut out = MARK.to_vec();
    out.extend_from_slice(b"secret\n");
    Ok(out)
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

/// The key store is up, but no default or login collection is unlocked.
pub const SETUP_REASON: &str = "The key store on this computer is not unlocked.";

const PREPARE_MISSING: &str =
    "The key store is not running. Start it when you sign in, then try again.";
const PREPARE_LOCKED: &str = "The key store has a password, so it stays locked. A password asks again every time you sign in. Use a passphrase on this computer, or leave the key store without a password.";
const PREPARE_FAILED: &str =
    "The key store is still not unlocked. Try again, or use a passphrase on this computer.";

/// The key file is already a passphrase file. A missing key store is not this.
pub fn needs_passphrase(dir: &Path) -> bool {
    has_pass_file(dir)
}

/// A saved key that is not the passphrase file. Opening it can wait on a
/// prompt, so startup does that on a background step instead of while the
/// window is opening.
pub fn defer_secret_load(dir: &Path) -> bool {
    #[cfg(test)]
    {
        let _ = dir;
        false
    }
    #[cfg(not(test))]
    {
        fs::read(path(dir)).ok().is_some_and(|bytes| {
            bytes.starts_with(MARK)
                && bytes.strip_prefix(MARK).is_some_and(|rest| {
                    !rest.starts_with(b"pass\n") && !rest.starts_with(b"file\n")
                })
        })
    }
}

pub fn wants_key_setup(error: &str) -> bool {
    error == SETUP_REASON
        || error == PREPARE_MISSING
        || error == PREPARE_LOCKED
        || error == PREPARE_FAILED
}

/// Secret Service has no unlocked default or login collection.
/// The test build never asks the real key store.
pub fn key_store_needs_setup() -> bool {
    #[cfg(all(not(test), unix, not(target_os = "macos")))]
    {
        !collection_unlocked()
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
    // A temp file in the same directory, created private, then renamed. A crash
    // while writing leaves the previous device.bin in place.
    let tmp = file.with_file_name("device.bin.tmp");
    let _ = fs::remove_file(&tmp);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut out = options.open(&tmp).map_err(|e| e.to_string())?;
    out.write_all(sealed).map_err(|e| e.to_string())?;
    out.sync_all().map_err(|e| e.to_string())?;
    drop(out);
    fs::rename(&tmp, &file).map_err(|e| e.to_string())
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
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
    };
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
            CRYPTPROTECT_UI_FORBIDDEN,
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
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
    };
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
            CRYPTPROTECT_UI_FORBIDDEN,
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
const KEYRING_WAIT: Duration = Duration::from_secs(5);

#[cfg(all(unix, not(target_os = "macos")))]
fn dbus_bin() -> String {
    std::env::var("OMACUS_DBUS_SEND").unwrap_or_else(|_| "dbus-send".to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn secret_bin() -> String {
    std::env::var("OMACUS_SECRET_TOOL").unwrap_or_else(|_| "secret-tool".to_string())
}

/// Wait for a child, and kill it if it is still running after `timeout`.
#[cfg(all(unix, not(target_os = "macos")))]
fn finish_child(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    use std::io::Read;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("The key store took too long.".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(error.to_string()),
        }
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut stderr);
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn parse_object_path(text: &str) -> Option<String> {
    parse_object_paths(text).into_iter().next()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn parse_object_paths(text: &str) -> Vec<String> {
    let marker = "object path \"";
    let mut rest = text;
    let mut paths = Vec::new();
    while let Some(start) = rest.find(marker) {
        rest = &rest[start + marker.len()..];
        let Some(end) = rest.find('"') else {
            break;
        };
        paths.push(rest[..end].to_string());
        rest = &rest[end + 1..];
    }
    paths
}

#[cfg(all(unix, not(target_os = "macos")))]
fn parse_locked(text: &str) -> Option<bool> {
    if text.contains("boolean true") {
        Some(true)
    } else if text.contains("boolean false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn path_is_collection(path: &str) -> bool {
    !path.is_empty() && path != "/" && path != "/org/freedesktop/secrets"
}

/// Secret Service is up, and the default or login collection exists and is unlocked.
/// A locked collection gets one Secret Service unlock. An empty password opens
/// with no prompt. A passworded collection stays locked, and its prompt is dismissed.
#[cfg(all(unix, not(target_os = "macos")))]
fn collection_unlocked() -> bool {
    if alias_open("default") || alias_open("login") {
        return true;
    }
    #[cfg(not(test))]
    if collection_path().is_some() {
        unlock_empty();
        return alias_open("default") || alias_open("login");
    }
    false
}

/// Create or unlock the default collection once. A passworded collection is
/// left alone. The check afterwards is ReadAlias plus the Locked property.
#[cfg(all(not(test), unix, not(target_os = "macos")))]
pub fn prepare_keyring() -> Result<(), String> {
    if collection_unlocked() {
        return Ok(());
    }
    if read_alias("default").is_none() {
        return Err(PREPARE_MISSING.into());
    }
    if collection_path().is_some() {
        return Err(PREPARE_LOCKED.into());
    }
    create_empty_collection()?;
    if collection_unlocked() {
        Ok(())
    } else {
        Err(PREPARE_FAILED.into())
    }
}

#[cfg(any(test, not(unix), target_os = "macos"))]
pub fn prepare_keyring() -> Result<(), String> {
    Err(PREPARE_FAILED.into())
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn collection_path() -> Option<String> {
    for alias in ["default", "login"] {
        let Some(path) = read_alias(alias) else {
            continue;
        };
        if path_is_collection(&path) {
            return Some(path);
        }
    }
    None
}

/// `Unlock` on the default and login collections. Secret Service tries an empty
/// password inside that call and returns no prompt when it works. A passworded
/// collection returns a prompt; that prompt is dismissed, never shown.
#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn unlock_empty() {
    let mut seen = Vec::new();
    for alias in ["default", "login"] {
        let Some(path) = read_alias(alias) else {
            continue;
        };
        if path_is_collection(&path) && !seen.contains(&path) {
            seen.push(path);
        }
    }
    for path in seen {
        unlock_collection(&path);
    }
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn unlock_collection(path: &str) {
    let arg = format!("array:objpath:{path}");
    let Ok(out) = dbus_call(&[
        "--session",
        "--print-reply",
        "--dest=org.freedesktop.secrets",
        "/org/freedesktop/secrets",
        "org.freedesktop.Secret.Service.Unlock",
        &arg,
    ]) else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let paths = parse_object_paths(&text);
    if let Some(prompt) = paths.last()
        && prompt != "/"
        && prompt != path
    {
        let _ = dbus_call(&[
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.secrets",
            prompt,
            "org.freedesktop.Secret.Prompt.Dismiss",
        ]);
    }
}

/// One bus connection: OpenSession and CreateWithMasterPassword have to share
/// it. The collection has no password, so a later Unlock opens it with no prompt.
#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn create_empty_collection() -> Result<(), String> {
    use std::io::Write;
    let mut stream = session_socket()?;
    stream
        .set_read_timeout(Some(KEYRING_WAIT))
        .map_err(|_| PREPARE_FAILED.to_string())?;
    stream
        .set_write_timeout(Some(KEYRING_WAIT))
        .map_err(|_| PREPARE_FAILED.to_string())?;
    let uid = unsafe { libc::getuid() };
    let hex: String = uid
        .to_string()
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    stream
        .write_all(format!("\0AUTH EXTERNAL {hex}\r\n").as_bytes())
        .map_err(|_| PREPARE_FAILED.to_string())?;
    let line = read_dbus_line(&mut stream)?;
    if !line.starts_with("OK") {
        return Err(PREPARE_FAILED.into());
    }
    stream
        .write_all(b"BEGIN\r\n")
        .map_err(|_| PREPARE_FAILED.to_string())?;
    let hello = bus_send(
        &mut stream,
        1,
        &BusCall {
            dest: "org.freedesktop.DBus",
            path: "/org/freedesktop/DBus",
            interface: "org.freedesktop.DBus",
            member: "Hello",
            signature: "",
            body: &[],
        },
    )?;
    if hello.kind == 3 {
        return Err(PREPARE_FAILED.into());
    }
    let open_body = body_open_session();
    let open = bus_send(
        &mut stream,
        2,
        &BusCall {
            dest: "org.freedesktop.secrets",
            path: "/org/freedesktop/secrets",
            interface: "org.freedesktop.Secret.Service",
            member: "OpenSession",
            signature: "sv",
            body: &open_body,
        },
    )?;
    if open.kind == 3 {
        return Err(PREPARE_FAILED.into());
    }
    let session = session_path(&open.body).ok_or_else(|| PREPARE_FAILED.to_string())?;
    let created_body = body_empty_collection(&session);
    let created = bus_send(
        &mut stream,
        3,
        &BusCall {
            dest: "org.freedesktop.secrets",
            path: "/org/freedesktop/secrets",
            interface: "org.gnome.keyring.InternalUnsupportedGuiltRiddenInterface",
            member: "CreateWithMasterPassword",
            signature: "a{sv}(oayays)",
            body: &created_body,
        },
    )?;
    if created.kind == 3 {
        return Err(PREPARE_FAILED.into());
    }
    Ok(())
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn session_socket() -> Result<std::os::unix::net::UnixStream, String> {
    use std::os::unix::net::UnixStream;
    let address =
        std::env::var("DBUS_SESSION_BUS_ADDRESS").map_err(|_| PREPARE_MISSING.to_string())?;
    for piece in address.split(';') {
        let Some(rest) = piece.strip_prefix("unix:") else {
            continue;
        };
        for item in rest.split(',') {
            let Some(path) = item.strip_prefix("path=") else {
                continue;
            };
            if let Ok(stream) = UnixStream::connect(path) {
                return Ok(stream);
            }
        }
    }
    Err(PREPARE_MISSING.into())
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn read_dbus_line(stream: &mut std::os::unix::net::UnixStream) -> Result<String, String> {
    use std::io::Read;
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream
            .read_exact(&mut byte)
            .map_err(|_| PREPARE_FAILED.to_string())?;
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
        if buf.len() > 512 {
            return Err(PREPARE_FAILED.into());
        }
    }
    Ok(String::from_utf8_lossy(&buf).trim().to_string())
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
struct DbusReply {
    kind: u8,
    body: Vec<u8>,
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
struct BusCall<'a> {
    dest: &'a str,
    path: &'a str,
    interface: &'a str,
    member: &'a str,
    signature: &'a str,
    body: &'a [u8],
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn bus_send(
    stream: &mut std::os::unix::net::UnixStream,
    serial: u32,
    call: &BusCall<'_>,
) -> Result<DbusReply, String> {
    use std::io::Write;
    let message = dbus_message(
        serial,
        call.dest,
        call.path,
        call.interface,
        call.member,
        call.signature,
        call.body,
    );
    stream
        .write_all(&message)
        .map_err(|_| PREPARE_FAILED.to_string())?;
    loop {
        let reply = read_dbus_message(stream)?;
        if reply.kind == 4 {
            continue;
        }
        return Ok(reply);
    }
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn read_dbus_message(stream: &mut std::os::unix::net::UnixStream) -> Result<DbusReply, String> {
    use std::io::Read;
    let mut head = [0u8; 16];
    stream
        .read_exact(&mut head)
        .map_err(|_| PREPARE_FAILED.to_string())?;
    if head[0] != b'l' {
        return Err(PREPARE_FAILED.into());
    }
    let body_len = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
    let fields_len = u32::from_le_bytes(head[12..16].try_into().unwrap()) as usize;
    if body_len > 1_000_000 || fields_len > 1_000_000 {
        return Err(PREPARE_FAILED.into());
    }
    let mut fields = vec![0u8; fields_len];
    stream
        .read_exact(&mut fields)
        .map_err(|_| PREPARE_FAILED.to_string())?;
    let header_len = 16 + fields_len;
    let pad = (8 - (header_len % 8)) % 8;
    if pad > 0 {
        let mut padding = vec![0u8; pad];
        stream
            .read_exact(&mut padding)
            .map_err(|_| PREPARE_FAILED.to_string())?;
    }
    let mut body = vec![0u8; body_len];
    if body_len > 0 {
        stream
            .read_exact(&mut body)
            .map_err(|_| PREPARE_FAILED.to_string())?;
    }
    Ok(DbusReply {
        kind: head[1],
        body,
    })
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn dbus_message(
    serial: u32,
    dest: &str,
    path: &str,
    interface: &str,
    member: &str,
    signature: &str,
    body: &[u8],
) -> Vec<u8> {
    let mut buf = vec![0u8; 16];
    buf[0] = b'l';
    buf[1] = 1;
    buf[3] = 1;
    header_path(&mut buf, path);
    header_string(&mut buf, 6, dest);
    header_string(&mut buf, 2, interface);
    header_string(&mut buf, 3, member);
    if !signature.is_empty() {
        header_signature(&mut buf, signature);
    }
    let fields_len = (buf.len() - 16) as u32;
    while !buf.len().is_multiple_of(8) {
        buf.push(0);
    }
    buf.extend_from_slice(body);
    buf[4..8].copy_from_slice(&(body.len() as u32).to_le_bytes());
    buf[8..12].copy_from_slice(&serial.to_le_bytes());
    buf[12..16].copy_from_slice(&fields_len.to_le_bytes());
    buf
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn pad_to(buf: &mut Vec<u8>, align: usize) {
    while !buf.len().is_multiple_of(align) {
        buf.push(0);
    }
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn write_len_string(buf: &mut Vec<u8>, text: &str) {
    pad_to(buf, 4);
    buf.extend_from_slice(&(text.len() as u32).to_le_bytes());
    buf.extend_from_slice(text.as_bytes());
    buf.push(0);
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn write_signature(buf: &mut Vec<u8>, text: &str) {
    buf.push(text.len() as u8);
    buf.extend_from_slice(text.as_bytes());
    buf.push(0);
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn header_string(buf: &mut Vec<u8>, code: u8, text: &str) {
    pad_to(buf, 8);
    buf.push(code);
    write_signature(buf, "s");
    write_len_string(buf, text);
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn header_path(buf: &mut Vec<u8>, path: &str) {
    pad_to(buf, 8);
    buf.push(1);
    write_signature(buf, "o");
    write_len_string(buf, path);
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn header_signature(buf: &mut Vec<u8>, signature: &str) {
    pad_to(buf, 8);
    buf.push(8);
    write_signature(buf, "g");
    write_signature(buf, signature);
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn body_open_session() -> Vec<u8> {
    let mut body = Vec::new();
    write_len_string(&mut body, "plain");
    write_signature(&mut body, "s");
    write_len_string(&mut body, "");
    body
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn body_empty_collection(session: &str) -> Vec<u8> {
    let mut body = Vec::new();
    pad_to(&mut body, 4);
    let len_at = body.len();
    body.extend_from_slice(&0u32.to_le_bytes());
    pad_to(&mut body, 8);
    let start = body.len();
    write_len_string(&mut body, "org.freedesktop.Secret.Collection.Label");
    write_signature(&mut body, "s");
    write_len_string(&mut body, "login");
    let array_len = (body.len() - start) as u32;
    body[len_at..len_at + 4].copy_from_slice(&array_len.to_le_bytes());
    pad_to(&mut body, 8);
    write_len_string(&mut body, session);
    pad_to(&mut body, 4);
    body.extend_from_slice(&0u32.to_le_bytes());
    pad_to(&mut body, 4);
    body.extend_from_slice(&0u32.to_le_bytes());
    write_len_string(&mut body, "");
    body
}

#[cfg(all(not(test), unix, not(target_os = "macos")))]
fn session_path(body: &[u8]) -> Option<String> {
    let marker = b"/org/freedesktop/secrets/session/";
    let start = body
        .windows(marker.len())
        .position(|window| window == marker)?;
    let rest = &body[start..];
    let end = rest.iter().position(|byte| *byte == 0)?;
    let path = std::str::from_utf8(&rest[..end]).ok()?;
    if path.starts_with("/org/freedesktop/secrets/session/") {
        Some(path.to_string())
    } else {
        None
    }
}

/// The passwordless default keyring Omarchy writes at install. An existing
/// keyring file is left in place. Returns whether a new file was written.
#[cfg(all(test, unix, not(target_os = "macos")))]
fn install_passwordless_keyring_at(dir: &Path) -> Result<bool, String> {
    const BODY: &str = "\
[keyring]
display-name=Default keyring
ctime=1700000000
mtime=0
lock-on-idle=false
lock-after=false
";
    if dir.exists() {
        let meta = fs::symlink_metadata(dir).map_err(|_| PREPARE_FAILED.to_string())?;
        if meta.file_type().is_symlink() {
            return Err(PREPARE_FAILED.into());
        }
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().ends_with(".keyring") {
                    return Ok(false);
                }
            }
        }
    } else {
        fs::create_dir_all(dir).map_err(|_| PREPARE_FAILED.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        }
    }
    let keyring = dir.join("Default_keyring.keyring");
    write_new(&keyring, BODY.as_bytes(), 0o600)?;
    let pointer = dir.join("default");
    fs::write(&pointer, b"Default_keyring\n").map_err(|_| PREPARE_FAILED.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pointer, fs::Permissions::from_mode(0o644))
            .map_err(|_| PREPARE_FAILED.to_string())?;
    }
    Ok(true)
}

#[cfg(all(test, unix, not(target_os = "macos")))]
fn write_new(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut file = options.open(path).map_err(|_| PREPARE_FAILED.to_string())?;
    file.write_all(bytes)
        .map_err(|_| PREPARE_FAILED.to_string())?;
    file.sync_all().map_err(|_| PREPARE_FAILED.to_string())?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn alias_open(alias: &str) -> bool {
    let Some(path) = read_alias(alias) else {
        return false;
    };
    if !path_is_collection(&path) {
        return false;
    }
    locked_flag(&path) == Some(false)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn dbus_call(args: &[&str]) -> Result<std::process::Output, String> {
    let mut command = std::process::Command::new(dbus_bin());
    command.args(args);
    let child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    finish_child(child, KEYRING_WAIT)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read_alias(alias: &str) -> Option<String> {
    let alias_arg = format!("string:{alias}");
    let out = dbus_call(&[
        "--session",
        "--print-reply",
        "--dest=org.freedesktop.secrets",
        "/org/freedesktop/secrets",
        "org.freedesktop.Secret.Service.ReadAlias",
        &alias_arg,
    ])
    .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_object_path(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn locked_flag(path: &str) -> Option<bool> {
    let out = dbus_call(&[
        "--session",
        "--print-reply",
        "--dest=org.freedesktop.secrets",
        path,
        "org.freedesktop.DBus.Properties.Get",
        "string:org.freedesktop.Secret.Collection",
        "string:Locked",
    ])
    .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_locked(&String::from_utf8_lossy(&out.stdout))
}

#[cfg_attr(test, allow(dead_code))]
#[cfg(all(unix, not(target_os = "macos")))]
fn secret_put(plain: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(secret_bin())
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(plain).map_err(|error| error.to_string())?;
    }
    let out = finish_child(child, KEYRING_WAIT)?;
    if out.status.success() {
        Ok(())
    } else {
        Err("The keyring did not keep the key.".into())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn secret_get() -> Result<Vec<u8>, String> {
    if !collection_unlocked() {
        return Err(SETUP_REASON.into());
    }
    let mut command = std::process::Command::new(secret_bin());
    command.args(["lookup", "app", "omacus", "kind", "device"]);
    let child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let out = finish_child(child, KEYRING_WAIT)?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err("That copy could not be read.".into())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn secret_delete() -> Result<(), String> {
    if !collection_unlocked() {
        return Ok(());
    }
    let mut command = std::process::Command::new(secret_bin());
    command.args(["clear", "app", "omacus", "kind", "device"]);
    let child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let out = finish_child(child, KEYRING_WAIT)?;
    if out.status.success() {
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
    fn a_half_written_temp_file_does_not_replace_the_key() {
        let dir = std::env::temp_dir().join(format!("omacus-key-tmp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let first = b"OMACUS-DEVICE/1\nfirst-key";
        save(&dir, first).unwrap();
        assert!(is_private(&dir));
        let file = path(&dir);
        let tmp = file.with_file_name("device.bin.tmp");
        let kept = fs::read(&file).unwrap();
        fs::write(&tmp, b"partial").unwrap();
        assert_eq!(fs::read(&file).unwrap(), kept);
        assert_eq!(load(&dir).unwrap().unwrap(), first);
        let second = b"OMACUS-DEVICE/1\nsecond-key";
        save(&dir, second).unwrap();
        assert!(!tmp.exists());
        assert!(is_private(&dir));
        assert_eq!(load(&dir).unwrap().unwrap(), second);
        remove(&dir);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn an_unlock_reply_names_the_collection_and_the_prompt() {
        let opened = "\
array [
   object path \"/org/freedesktop/secrets/collection/login\"
]
object path \"/\"
";
        assert_eq!(
            parse_object_paths(opened),
            vec![
                "/org/freedesktop/secrets/collection/login".to_string(),
                "/".to_string()
            ]
        );
        let locked = "\
array [
]
object path \"/org/freedesktop/secrets/prompt/u1\"
";
        assert_eq!(
            parse_object_paths(locked),
            vec!["/org/freedesktop/secrets/prompt/u1".to_string()]
        );
        assert_eq!(
            parse_object_path("object path \"/org/freedesktop/secrets/collection/login\""),
            Some("/org/freedesktop/secrets/collection/login".to_string())
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_stuck_keyring_call_is_killed() {
        assert!(KEYRING_WAIT >= Duration::from_secs(5));
        assert!(KEYRING_WAIT <= Duration::from_secs(6));
        let mut command = std::process::Command::new("sleep");
        command.arg("30");
        let child = command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let error = finish_child(child, Duration::from_millis(200)).unwrap_err();
        assert_eq!(error, "The key store took too long.");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_missing_or_locked_collection_is_not_usable() {
        assert_eq!(
            parse_object_path("   object path \"/\"\n"),
            Some("/".to_string())
        );
        assert_eq!(
            parse_object_path("object path \"/org/freedesktop/secrets/collection/login\""),
            Some("/org/freedesktop/secrets/collection/login".to_string())
        );
        assert_eq!(parse_object_path("no path"), None);
        assert_eq!(parse_locked("variant boolean false\n"), Some(false));
        assert_eq!(parse_locked("variant boolean true\n"), Some(true));
        assert_eq!(parse_locked("variant string \"no\""), None);
        assert!(!path_is_collection("/"));
        assert!(!path_is_collection(""));
        assert!(!path_is_collection("/org/freedesktop/secrets"));
        assert!(path_is_collection(
            "/org/freedesktop/secrets/collection/login"
        ));
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
        let payload = stored
            .strip_prefix(MARK)
            .unwrap()
            .strip_prefix(b"pass\n")
            .unwrap();
        assert_eq!(
            u32::from_be_bytes(payload[0..4].try_into().unwrap()),
            64 * 1024
        );
        assert_eq!(u32::from_be_bytes(payload[4..8].try_into().unwrap()), 3);
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

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_passwordless_keyring_is_written_once() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("omacus-keyring-file-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(install_passwordless_keyring_at(&dir).unwrap());
        let body = fs::read_to_string(dir.join("Default_keyring.keyring")).unwrap();
        assert!(body.contains("display-name=Default keyring"));
        assert!(body.contains("lock-on-idle=false"));
        assert!(body.contains("lock-after=false"));
        assert!(!body.contains("password"));
        assert_eq!(fs::read(dir.join("default")).unwrap(), b"Default_keyring\n");
        let key_mode = fs::metadata(dir.join("Default_keyring.keyring"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let pointer_mode = fs::metadata(dir.join("default"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(key_mode, 0o600);
        assert_eq!(pointer_mode, 0o644);
        assert!(!install_passwordless_keyring_at(&dir).unwrap());
        assert_eq!(
            fs::read_to_string(dir.join("Default_keyring.keyring")).unwrap(),
            body
        );

        let other =
            std::env::temp_dir().join(format!("omacus-keyring-keep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&other);
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("login.keyring"), b"keep-me").unwrap();
        assert!(!install_passwordless_keyring_at(&other).unwrap());
        assert_eq!(fs::read(other.join("login.keyring")).unwrap(), b"keep-me");
        assert!(!other.join("Default_keyring.keyring").exists());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&other);
    }
}
