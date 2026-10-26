//! New releases: checking for them, and installing them.
//!
//! On launch, and every few hours while it stays open, Omacus downloads the latest
//! signed update manifest on a background thread. The answer is cached, so reopening
//! the app within the hour makes no request. An unsigned release tag is not a version.
//! Nothing about the user or their pages is sent. Turn it off in Settings,
//! or with `check_for_updates = false` in `settings.toml` in the config folder.
//!
//! On Linux and macOS, installing reuses the one-line installer, so an update is
//! exactly a fresh install of the latest release: same files, same places, sheets
//! untouched. On Windows, where Omacus is a single portable .exe, the new one is
//! swapped in next to the running one (Windows allows renaming a running .exe).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

const REPO: &str = "avinashjoshi/omacus";
/// Minisign public key for update manifests. The matching secret is not in the repository.
const UPDATE_PUBLIC_KEY: &str = include_str!("../keys/omacus-update.pub");
/// Legacy minisign signs the file itself. `ED` would be the prehashed algorithm, which this is not.
const MINISIGN_ED: &[u8] = b"Ed";
/// Copies from before this version check a SHA-256 on the same GitHub release.
/// They do not check the minisign signature. This build checks the signature first.
#[cfg(test)]
const MINISIGN_SINCE: &str = "1.0.0";
pub const RELEASES_URL: &str = "https://github.com/avinashjoshi/omacus/releases";
/// The site's changelog. What's new in the app opens this, at the new version.
pub const UPDATES_PAGE: &str = "https://omacus.com/updates/";
/// How old the last answer may be at launch, so reopening the app often doesn't ask again.
const AT_LAUNCH: Duration = Duration::from_secs(60 * 60);
/// How often a window that stays open asks.
const WHILE_OPEN: Duration = Duration::from_secs(4 * 60 * 60);
/// How soon to try again after a failed check (offline, say).
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
/// A check the user started. After this, the button works again even if the
/// request never comes back.
const MANUAL_CHECK_TIMEOUT: Duration = Duration::from_secs(20);

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The notes for a version, like "0.1.3", on the site's updates page.
pub fn notes_url(version: &str) -> String {
    let version = version.trim_start_matches('v');
    format!("{UPDATES_PAGE}#v{version}")
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    checked_at: u64,
    latest: Option<String>,
}

#[derive(Deserialize)]
struct UpdateManifest {
    version: String,
    files: BTreeMap<String, String>,
}

/// A check started from About, so its result can be shown there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Check {
    Idle,
    Checking,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Installing,
    Installed,
    Failed(String),
}

/// How this copy can be updated.
#[derive(Clone, Copy, PartialEq)]
pub enum Method {
    /// Installed by the one-line installer (or a tarball or .dmg): run it again.
    Installer,
    /// The portable Windows .exe: swap in the new one.
    Replace,
    /// Installed by a package manager (.deb or .rpm): download the new package.
    Download,
}

/// Where this executable was started from. Captured at launch, because after a
/// Windows update the running file has been renamed to `.old`.
static EXE: OnceLock<Option<PathBuf>> = OnceLock::new();

fn exe() -> Option<PathBuf> {
    EXE.get_or_init(|| std::env::current_exe().ok()).clone()
}

/// Call first thing in main: remembers the executable's path and, on Windows,
/// removes the copy an update left behind.
pub fn init() {
    if let Some(exe) = exe() {
        let _ = std::fs::remove_file(sibling(&exe, "old"));
    }
}

/// `omacus.exe` -> `omacus.exe.old`.
fn sibling(exe: &Path, ext: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(format!(".{ext}"));
    PathBuf::from(name)
}

#[derive(Clone)]
pub struct Updates {
    latest: Arc<Mutex<Option<String>>>,
    state: Arc<Mutex<State>>,
    check: Arc<Mutex<Check>>,
    /// When GitHub last answered, and when it was last asked.
    checked_at: Arc<Mutex<u64>>,
    tried_at: Arc<Mutex<u64>>,
    /// Bumps each time a manual check starts, so a late answer can't overwrite a newer one.
    generation: Arc<Mutex<u64>>,
    /// When the manual check that's showing "Checking…" began.
    started: Arc<Mutex<Option<Instant>>>,
}

impl Updates {
    /// Use the cached answer now, and ask GitHub in the background if it's over an hour old.
    pub fn check(on_update: impl Fn() + Send + 'static) -> Self {
        let updates = Self::from_cache(read_cache());
        updates.check_older_than(AT_LAUNCH, on_update);
        updates
    }

    fn from_cache(cache: Cache) -> Self {
        Self {
            latest: Arc::new(Mutex::new(cache.latest)),
            state: Arc::new(Mutex::new(State::Idle)),
            check: Arc::new(Mutex::new(Check::Idle)),
            checked_at: Arc::new(Mutex::new(cache.checked_at)),
            tried_at: Arc::new(Mutex::new(0)),
            generation: Arc::new(Mutex::new(0)),
            started: Arc::new(Mutex::new(None)),
        }
    }

    /// Called on every frame (cheaply), so a window that stays open still hears
    /// about new versions.
    pub fn check_if_due(&self, on_update: impl Fn() + Send + 'static) {
        self.check_older_than(WHILE_OPEN, on_update);
    }

    /// Ask GitHub in the background if its last answer is older than `age`, and no
    /// try was made in the last half hour.
    fn check_older_than(&self, age: Duration, on_update: impl Fn() + Send + 'static) {
        let now = now_secs();
        {
            let checked_at = *self.checked_at.lock().unwrap();
            let mut tried_at = self.tried_at.lock().unwrap();
            if !is_due(now, checked_at, *tried_at, age) || !enabled() {
                return;
            }
            *tried_at = now;
        }
        let (latest, checked_at) = (self.latest.clone(), self.checked_at.clone());
        std::thread::spawn(move || {
            if let Ok(version) = fetch_latest() {
                write_cache(&version);
                *checked_at.lock().unwrap() = now_secs();
                *latest.lock().unwrap() = Some(version);
                on_update();
            }
        });
    }

    /// A newer version than this one, if there is one. With checking turned off,
    /// only a check the user asked for counts.
    pub fn available(&self) -> Option<String> {
        if !enabled() && self.checked() != Check::Done {
            return None;
        }
        self.latest
            .lock()
            .unwrap()
            .clone()
            .filter(|v| is_newer(v, current()))
    }

    pub fn checked(&self) -> Check {
        let check = *self.check.lock().unwrap();
        if check == Check::Checking && self.manual_timed_out() {
            // The request is still out, but it no longer owns the button.
            Check::Failed
        } else {
            check
        }
    }

    /// Ask GitHub now (the About pane's button), whatever the schedule says.
    /// A click while one is already running does nothing. `false` means that.
    pub fn check_now(&self, on_done: impl Fn() + Send + 'static) -> bool {
        let Some(token) = self.start_manual() else {
            return false;
        };
        let updates = self.clone();
        std::thread::spawn(move || {
            let result = fetch_latest().map_err(|e| e.to_string());
            if let Some(version) = updates.finish_manual(token, result) {
                write_cache(&version);
            }
            on_done();
        });
        true
    }

    /// Start a manual check. `None` while one is already in flight.
    fn start_manual(&self) -> Option<u64> {
        if self.manual_in_flight() {
            return None;
        }
        let mut generation = self.generation.lock().unwrap();
        *generation = generation.saturating_add(1);
        *self.check.lock().unwrap() = Check::Checking;
        *self.started.lock().unwrap() = Some(Instant::now());
        Some(*generation)
    }

    /// Record `result` if `token` is still the check on screen. The version, when
    /// this was the current check and GitHub answered.
    fn finish_manual(&self, token: u64, result: Result<String, String>) -> Option<String> {
        if *self.generation.lock().unwrap() != token {
            return None;
        }
        let version = match result {
            Ok(version) => {
                *self.checked_at.lock().unwrap() = now_secs();
                *self.latest.lock().unwrap() = Some(version.clone());
                *self.check.lock().unwrap() = Check::Done;
                Some(version)
            }
            Err(_) => {
                *self.check.lock().unwrap() = Check::Failed;
                None
            }
        };
        *self.started.lock().unwrap() = None;
        version
    }

    fn manual_in_flight(&self) -> bool {
        *self.check.lock().unwrap() == Check::Checking && !self.manual_timed_out()
    }

    fn manual_timed_out(&self) -> bool {
        self.started
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() >= MANUAL_CHECK_TIMEOUT)
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    /// Install the available version on a background thread; `on_done` wakes the UI.
    pub fn install(&self, on_done: impl Fn() + Send + 'static) {
        let Some(version) = self.available() else {
            return;
        };
        *self.state.lock().unwrap() = State::Installing;
        let state = self.state.clone();
        std::thread::spawn(move || {
            let result = match method() {
                Method::Replace => replace_exe(&version),
                _ => run_installer(&version, false),
            };
            *state.lock().unwrap() = match result {
                Ok(()) => State::Installed,
                Err(e) => State::Failed(e),
            };
            on_done();
        });
    }
}

pub fn method() -> Method {
    if cfg!(windows) {
        return Method::Replace;
    }
    let Some(exe) = exe() else {
        return Method::Download;
    };
    // The installer puts Omacus in the home folder (~/.local/bin) on Linux, and an
    // Omacus.app bundle on macOS. A copy in /usr/bin came from a package manager.
    let in_home = dirs::home_dir().is_some_and(|home| exe.starts_with(home));
    if in_home || app_bundle(&exe).is_some() {
        Method::Installer
    } else {
        Method::Download
    }
}

/// Start the freshly installed copy. The running one closes itself after.
pub fn relaunch() -> std::io::Result<()> {
    let exe = exe().ok_or_else(|| std::io::Error::other("unknown executable path"))?;
    if let Some(bundle) = app_bundle(&exe) {
        Command::new("open").arg("-n").arg(bundle).spawn()?;
        return Ok(());
    }
    // After an update on Linux, /proc/self/exe points at the replaced file
    // ("omacus (deleted)"); start the one now at the same path.
    let path = PathBuf::from(exe.to_string_lossy().trim_end_matches(" (deleted)"));
    Command::new(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

/// `omacus update`.
pub fn cli() -> i32 {
    println!("Checking for a new version of Omacus…");
    let latest = match fetch_latest() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("omacus: couldn't reach GitHub: {e}");
            return 1;
        }
    };
    write_cache(&latest);
    if !is_newer(&latest, current()) {
        println!("Omacus {} is the latest version.", current());
        return 0;
    }
    if method() == Method::Download {
        println!(
            "Omacus {latest} is available (you have {}). Download it from {RELEASES_URL}/latest",
            current()
        );
        return 0;
    }
    println!("Updating Omacus {} → {latest}…", current());
    let result = match method() {
        Method::Replace => replace_exe(&latest),
        _ => run_installer(&latest, true),
    };
    match result {
        Ok(()) => {
            println!("Updated. If Omacus is open, restart it to use {latest}.");
            0
        }
        Err(e) => {
            eprintln!("omacus: update failed: {e}");
            1
        }
    }
}

/// A file from a release, checked against the signed update manifest. The manifest
/// and its minisign signature come from GitHub over HTTPS for that exact version.
/// A damaged download, or a release that was not signed with the update key, is
/// refused rather than run.
fn release_file(version: &str, name: &str) -> Result<Vec<u8>, String> {
    let version = version.trim_start_matches('v');
    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    let manifest = signed_manifest_at(&base).map_err(|e| e.to_string())?;
    let expected = expected_checksum(&manifest, version, name)?;
    let bytes = download(&format!("{base}/{name}"), 64 * 1024 * 1024).map_err(|e| e.to_string())?;
    if sha256_hex(&bytes) != expected {
        return Err(format!(
            "{name} didn't match its checksum; not installing it"
        ));
    }
    Ok(bytes)
}

fn expected_checksum(
    manifest: &UpdateManifest,
    version: &str,
    name: &str,
) -> Result<String, String> {
    if manifest.version != version {
        return Err("the signed update is for a different version".into());
    }
    let hash = manifest
        .files
        .get(name)
        .ok_or_else(|| format!("{name} isn't in the signed update"))?
        .to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{name} has no checksum in the signed update"));
    }
    Ok(hash)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A new folder only this user can read, for running the installer from.
fn private_dir() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("omacus-update-{}-{nanos}", std::process::id()));
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder
    };
    #[cfg(not(unix))]
    let builder = std::fs::DirBuilder::new();
    builder.create(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Linux and macOS: the release's own install.sh, checked, without the Hyprland
/// questions (an update keeps the existing keybind and window rule). `show` passes
/// its output through.
fn run_installer(version: &str, show: bool) -> Result<(), String> {
    let script = release_file(version, "install.sh")?;
    let dir = private_dir()?;
    let result = (|| {
        let path = dir.join("install.sh");
        std::fs::write(&path, &script).map_err(|e| e.to_string())?;
        let mut cmd = Command::new("bash");
        cmd.arg(&path)
            .arg("--no-hyprland")
            .current_dir(&dir)
            .stdin(Stdio::null())
            // Install exactly this version, the one whose checksums were checked,
            // even if a newer one came out since.
            .env("OMACUS_VERSION", version);
        // On macOS, replace this exact app bundle, wherever it is and whatever it's called.
        if let Some(bundle) = exe().as_deref().and_then(app_bundle) {
            cmd.env("OMACUS_APP", bundle);
        }
        if show {
            let status = cmd.status().map_err(|e| e.to_string())?;
            return status
                .success()
                .then_some(())
                .ok_or_else(|| format!("the installer exited with {status}"));
        }
        let out = cmd.output().map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("the installer failed")
            .trim()
            .to_string())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Windows: the release's .exe, checked, swapped in for this one.
#[cfg(windows)]
fn replace_exe(version: &str) -> Result<(), String> {
    let exe = exe().ok_or("couldn't find the running executable")?;
    let zip = release_file(version, "omacus-windows-x86_64.zip")?;
    let new = extract_exe(&zip)?;
    swap_in(&exe, &new)
}

#[cfg(not(windows))]
fn replace_exe(_version: &str) -> Result<(), String> {
    Err("only Windows updates by replacing the executable".into())
}

/// The `omacus.exe` inside a release zip.
#[cfg(any(windows, test))]
fn extract_exe(zip: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip))
        .map_err(|e| format!("the download isn't a valid zip: {e}"))?;
    let mut file = archive
        .by_name("omacus.exe")
        .map_err(|_| "the download has no omacus.exe".to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Write the new executable next to `exe`, move the running one aside to `.old`,
/// and move the new one into place. If the last step fails, put the old one back.
#[cfg(any(windows, test))]
fn swap_in(exe: &Path, new: &[u8]) -> Result<(), String> {
    let (staged, old) = (sibling(exe, "new"), sibling(exe, "old"));
    let explain = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::PermissionDenied => format!(
            "no permission to replace {} (is another Omacus window open?); download the new version instead",
            exe.display()
        ),
        _ => e.to_string(),
    };
    if let Err(e) = std::fs::write(&staged, new) {
        let _ = std::fs::remove_file(&staged);
        return Err(explain(e));
    }
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&staged);
        return Err(explain(e));
    }
    if let Err(e) = std::fs::rename(&staged, exe) {
        let _ = std::fs::remove_file(&staged);
        return Err(match std::fs::rename(&old, exe) {
            Ok(()) => e.to_string(),
            Err(_) => format!("{e}; the previous version is at {}", old.display()),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

fn app_bundle(exe: &Path) -> Option<&Path> {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
}

/// Whether checking is on. Settings keeps this in memory, so a check on every
/// frame does not read the file. `OMACUS_NO_UPDATE_CHECK` forces it off.
fn enabled() -> bool {
    crate::settings::updates_enabled()
}

/// The latest version, from the signed manifest on the latest release.
/// An unsigned tag is not a version.
fn fetch_latest() -> Result<String, Box<dyn std::error::Error>> {
    Ok(signed_manifest_at(&format!("{RELEASES_URL}/latest/download"))?.version)
}

fn signed_manifest_at(base: &str) -> Result<UpdateManifest, Box<dyn std::error::Error>> {
    let body = download(&format!("{base}/omacus-update.json"), 64 * 1024)?;
    let sig = download(&format!("{base}/omacus-update.json.minisig"), 8 * 1024)?;
    let sig = String::from_utf8(sig).map_err(|_| fail("unreadable update signature"))?;
    verify_minisign(UPDATE_PUBLIC_KEY, &body, &sig).map_err(fail)?;
    let manifest: UpdateManifest =
        serde_json::from_slice(&body).map_err(|_| fail("unreadable update manifest"))?;
    if !version_ok(&manifest.version) {
        return Err(fail("the signed update names no version"));
    }
    Ok(manifest)
}

fn version_ok(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    crate::rates::http()
        .get(url)
        .header("User-Agent", concat!("omacus/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| fail(format!("download failed: {e}")))?
        .into_body()
        .with_config()
        .limit(limit)
        .reader()
        .read_to_end(&mut bytes)
        .map_err(|e| fail(format!("download failed: {e}")))?;
    Ok(bytes)
}

fn fail(msg: impl Into<String>) -> Box<dyn std::error::Error> {
    std::io::Error::other(msg.into()).into()
}

/// A minisign legacy (Ed) signature over `message`, including the trusted comment.
fn verify_minisign(public_key: &str, message: &[u8], signature: &str) -> Result<(), String> {
    let (key_id, pk) = parse_public_key(public_key)?;
    let parsed = parse_signature(signature)?;
    if parsed.key_id != key_id {
        return Err("the update signature is for a different key".into());
    }
    let vk = VerifyingKey::from_bytes(&pk).map_err(|_| "the update key is not a public key")?;
    let sig = Signature::from_slice(&parsed.sig).map_err(|_| "unreadable update signature")?;
    vk.verify(message, &sig)
        .map_err(|_| "the update signature doesn't match")?;
    let mut global_msg = parsed.sig.to_vec();
    global_msg.extend_from_slice(parsed.trusted.as_bytes());
    let global =
        Signature::from_slice(&parsed.global).map_err(|_| "unreadable update signature")?;
    vk.verify(&global_msg, &global)
        .map_err(|_| "the update signature's trusted comment doesn't match")?;
    Ok(())
}

struct ParsedSignature {
    key_id: [u8; 8],
    sig: [u8; 64],
    global: [u8; 64],
    trusted: String,
}

fn parse_public_key(text: &str) -> Result<([u8; 8], [u8; 32]), String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("untrusted comment:"))
        .ok_or("unreadable update key")?;
    let raw = decode_b64(line)?;
    if raw.len() != 42 || &raw[..2] != MINISIGN_ED {
        return Err("the update key is not a minisign Ed key".into());
    }
    let mut id = [0u8; 8];
    let mut pk = [0u8; 32];
    id.copy_from_slice(&raw[2..10]);
    pk.copy_from_slice(&raw[10..42]);
    Ok((id, pk))
}

fn parse_signature(text: &str) -> Result<ParsedSignature, String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() != 4 || !lines[0].starts_with("untrusted comment:") {
        return Err("unreadable update signature".into());
    }
    let Some(trusted) = lines[2].strip_prefix("trusted comment: ") else {
        return Err("unreadable update signature".into());
    };
    if trusted.is_empty() {
        return Err("unreadable update signature".into());
    }
    let raw = decode_b64(lines[1])?;
    let global = decode_b64(lines[3])?;
    if raw.len() != 74 || &raw[..2] != MINISIGN_ED || global.len() != 64 {
        return Err("the update signature is not a minisign Ed signature".into());
    }
    let mut key_id = [0u8; 8];
    let mut sig = [0u8; 64];
    let mut global_sig = [0u8; 64];
    key_id.copy_from_slice(&raw[2..10]);
    sig.copy_from_slice(&raw[10..74]);
    global_sig.copy_from_slice(&global);
    Ok(ParsedSignature {
        key_id,
        sig,
        global: global_sig,
        trusted: trusted.to_string(),
    })
}

fn decode_b64(text: &str) -> Result<Vec<u8>, String> {
    STANDARD
        .decode(text.trim().as_bytes())
        .map_err(|_| "unreadable update signature".to_string())
}

/// The release page, for the ignored network test. The app does not trust it.
#[cfg(test)]
fn from_latest_page() -> Result<String, Box<dyn std::error::Error>> {
    use ureq::ResponseExt;
    let response = crate::rates::http()
        .get(format!("{RELEASES_URL}/latest"))
        .header("User-Agent", concat!("omacus/", env!("CARGO_PKG_VERSION")))
        .call()?;
    let path = response.get_uri().path();
    let tag = path
        .rsplit_once("/tag/")
        .map(|(_, tag)| tag)
        .ok_or_else(|| format!("unexpected release page: {path}"))?;
    Ok(tag.trim_start_matches('v').to_string())
}

/// Whether to ask now: the last answer is older than `age`, and the last try (which
/// may have failed) was over `RETRY_AFTER` ago.
fn is_due(now: u64, checked_at: u64, tried_at: u64, age: Duration) -> bool {
    // A time in the future means the clock moved back; don't wait for it.
    (checked_at > now || now - checked_at > age.as_secs())
        && now.saturating_sub(tried_at) > RETRY_AFTER.as_secs()
}

/// Whether `a` is a later version than `b` ("0.1.10" > "0.1.9").
fn is_newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parse(a) > parse(b)
}

fn cache_path() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("omacus").join("update.json"))
}

fn read_cache() -> Cache {
    cache_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_cache(latest: &str) {
    let Some(path) = cache_path() else { return };
    let cache = Cache {
        checked_at: now_secs(),
        latest: Some(latest.to_string()),
    };
    if let (Some(dir), Ok(json)) = (path.parent(), serde_json::to_vec(&cache)) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, json);
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "network"]
    fn reads_latest_from_the_release_page() {
        let v = from_latest_page().unwrap();
        assert!(v.split('.').count() == 3, "{v}");
    }

    #[test]
    fn checks_on_launch_and_while_open() {
        let (hour, now) = (60 * 60, 100 * 24 * 60 * 60);
        // Launch: ask unless the last answer is under an hour old.
        assert!(is_due(now, 0, 0, AT_LAUNCH));
        assert!(is_due(now, now - 2 * hour, 0, AT_LAUNCH));
        assert!(!is_due(now, now - hour / 2, 0, AT_LAUNCH));
        // Open: every four hours.
        assert!(!is_due(now, now - 3 * hour, 0, WHILE_OPEN));
        assert!(is_due(now, now - 5 * hour, 0, WHILE_OPEN));
        // Offline: try again after half an hour, not every frame.
        assert!(!is_due(now, 0, now - 60, WHILE_OPEN));
        assert!(is_due(now, 0, now - hour, WHILE_OPEN));
        // A last check "in the future" doesn't block checking.
        assert!(is_due(now, now + 24 * hour, 0, WHILE_OPEN));
    }

    #[test]
    fn whats_new_opens_the_updates_page() {
        assert_eq!(notes_url("0.2.7"), "https://omacus.com/updates/#v0.2.7");
        assert_eq!(notes_url("v0.2.7"), "https://omacus.com/updates/#v0.2.7");
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.1.3", "0.1.2"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
    }

    #[test]
    fn swaps_in_a_new_executable() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("omacus-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("omacus.exe");
        std::fs::write(&exe, b"old build").unwrap();

        // A release zip holds omacus.exe and the README.
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("README.md", options).unwrap();
        zip.write_all(b"readme").unwrap();
        zip.start_file("omacus.exe", options).unwrap();
        zip.write_all(b"new build").unwrap();
        let zip = zip.finish().unwrap().into_inner();

        swap_in(&exe, &extract_exe(&zip).unwrap()).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new build");
        assert_eq!(std::fs::read(sibling(&exe, "old")).unwrap(), b"old build");
        assert!(!sibling(&exe, "new").exists());

        // A second update replaces the leftover .old.
        swap_in(&exe, b"newer build").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"newer build");
        assert_eq!(std::fs::read(sibling(&exe, "old")).unwrap(), b"new build");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "network"]
    fn refuses_a_release_without_a_signature() {
        let err = release_file("0.1.5", "install.sh").unwrap_err();
        assert!(
            err.contains("download failed")
                || err.contains("signature")
                || err.contains("manifest"),
            "{err}"
        );
    }

    #[test]
    fn a_signed_manifest_verifies_and_a_changed_byte_does_not() {
        let public_key = include_str!("../tests/fixtures/omacus-update-test.pub");
        let signature = include_str!("../tests/fixtures/omacus-update-test.minisig");
        verify_minisign(public_key, b"omacus-update-test\n", signature).unwrap();
        assert!(verify_minisign(public_key, b"omacus-update-tesT\n", signature).is_err());
        let bad = signature.replacen("Y9nzBA==", "Y9nzBB==", 1);
        assert_ne!(bad, signature);
        assert!(verify_minisign(public_key, b"omacus-update-test\n", &bad).is_err());
    }

    #[test]
    fn copies_before_1_0_only_check_sha256() {
        assert_eq!(MINISIGN_SINCE, "1.0.0");
        assert!(version_ok(MINISIGN_SINCE));
    }

    #[test]
    fn the_shipped_update_key_is_a_minisign_ed_key() {
        let (id, pk) = parse_public_key(UPDATE_PUBLIC_KEY).unwrap();
        assert_ne!(id, [0u8; 8]);
        let fixture = include_str!("../tests/fixtures/omacus-update-test.pub");
        assert_ne!(id, parse_public_key(fixture).unwrap().0);
        VerifyingKey::from_bytes(&pk).unwrap();
    }

    #[test]
    fn a_checksum_has_to_be_for_this_version() {
        let hash = "ab".repeat(32);
        let manifest = UpdateManifest {
            version: "0.2.21".into(),
            files: BTreeMap::from([("install.sh".into(), hash.clone())]),
        };
        assert_eq!(
            expected_checksum(&manifest, "0.2.21", "install.sh").unwrap(),
            hash
        );
        assert!(expected_checksum(&manifest, "0.2.20", "install.sh").is_err());
        assert!(expected_checksum(&manifest, "0.2.21", "missing").is_err());
        let short = UpdateManifest {
            version: "0.2.21".into(),
            files: BTreeMap::from([("install.sh".into(), "abcd".into())]),
        };
        assert!(expected_checksum(&short, "0.2.21", "install.sh").is_err());
        assert!(version_ok("0.2.21"));
        assert!(!version_ok("0.2"));
        assert!(!version_ok("1.0.0-beta"));
    }

    #[test]
    fn hashes_like_sha256sum() {
        // printf omacus | sha256sum
        assert_eq!(
            sha256_hex(b"omacus"),
            "35c9293164625a75db5ee2dd57387bff729e1b6d28994eabd316f86104b366c5"
        );
    }

    #[test]
    fn a_manual_check_can_run_again_after_it_finishes() {
        let updates = Updates::from_cache(Cache::default());
        let first = updates.start_manual().unwrap();
        assert!(
            updates.start_manual().is_none(),
            "a second click while checking does nothing"
        );
        assert_eq!(updates.checked(), Check::Checking);
        assert_eq!(
            updates.finish_manual(first, Ok("9.9.9".into())).as_deref(),
            Some("9.9.9")
        );
        assert_eq!(updates.checked(), Check::Done);
        let second = updates.start_manual().unwrap();
        assert_ne!(first, second);
        // A late answer from the first check can't overwrite the one on screen.
        assert!(updates.finish_manual(first, Ok("0.0.1".into())).is_none());
        updates.finish_manual(second, Err("offline".into()));
        assert_eq!(updates.checked(), Check::Failed);
        assert!(
            updates.start_manual().is_some(),
            "an error doesn't lock the button"
        );
    }

    #[test]
    fn a_hung_check_unlocks_after_the_timeout() {
        let updates = Updates::from_cache(Cache::default());
        let token = updates.start_manual().unwrap();
        let earlier = Instant::now()
            .checked_sub(MANUAL_CHECK_TIMEOUT)
            .expect("the clock has been running longer than the check timeout");
        *updates.started.lock().unwrap() = Some(earlier);
        assert_eq!(updates.checked(), Check::Failed);
        let again = updates.start_manual().unwrap();
        assert!(updates.finish_manual(token, Ok("9.9.9".into())).is_none());
        assert_eq!(updates.checked(), Check::Checking);
        updates.finish_manual(again, Ok("0.0.1".into()));
        assert_eq!(updates.checked(), Check::Done);
    }

    #[test]
    fn a_zip_without_the_exe_is_an_error() {
        let zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()))
            .finish()
            .unwrap()
            .into_inner();
        assert!(extract_exe(&zip).is_err());
        assert!(extract_exe(b"not a zip").is_err());
    }
}
