//! New releases: a daily check, and installing them.
//!
//! At most once a day, Omacus asks GitHub for the latest release tag, on a
//! background thread; the answer is cached so most launches make no request.
//! Nothing about the user or their sheets is sent. Turn it off with
//! `check_for_updates = false` in `settings.toml` in the config folder.
//!
//! Installing reuses the one-line installer, so an update is exactly a fresh
//! install of the latest release: same files, same places, sheets untouched.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const LATEST_URL: &str = "https://api.github.com/repos/avinashjoshi/omacus/releases/latest";
const INSTALLER_URL: &str = "https://omacus.com/install";
pub const RELEASES_URL: &str = "https://github.com/avinashjoshi/omacus/releases";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The notes for a version, like "0.1.3".
pub fn notes_url(version: &str) -> String {
    format!("{RELEASES_URL}/tag/v{version}")
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    checked_at: u64,
    latest: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
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
    /// Installed by the one-line installer (or a tarball or .dmg): it can update itself.
    Installer,
    /// Installed some other way (a .deb or .rpm, or Windows): download it again.
    Download,
}

#[derive(Clone)]
pub struct Updates {
    latest: Arc<Mutex<Option<String>>>,
    state: Arc<Mutex<State>>,
}

impl Updates {
    /// Use the cached answer now, and ask GitHub in the background if it's a day old.
    pub fn check(on_update: impl Fn() + Send + 'static) -> Self {
        let cache = read_cache();
        let updates = Self {
            latest: Arc::new(Mutex::new(cache.latest)),
            state: Arc::new(Mutex::new(State::Idle)),
        };
        let stale = now_secs().saturating_sub(cache.checked_at) > CHECK_EVERY.as_secs();
        if enabled() && stale {
            let latest = updates.latest.clone();
            std::thread::spawn(move || {
                if let Ok(version) = fetch_latest() {
                    write_cache(&version);
                    *latest.lock().unwrap() = Some(version);
                    on_update();
                }
            });
        }
        updates
    }

    /// A newer version than this one, if there is one.
    pub fn available(&self) -> Option<String> {
        if !enabled() {
            return None;
        }
        self.latest
            .lock()
            .unwrap()
            .clone()
            .filter(|v| is_newer(v, current()))
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    /// Run the installer on a background thread; `on_done` wakes the UI.
    pub fn install(&self, on_done: impl Fn() + Send + 'static) {
        *self.state.lock().unwrap() = State::Installing;
        let state = self.state.clone();
        std::thread::spawn(move || {
            *state.lock().unwrap() = match run_installer(false) {
                Ok(()) => State::Installed,
                Err(e) => State::Failed(e),
            };
            on_done();
        });
    }
}

pub fn method() -> Method {
    if cfg!(windows) {
        return Method::Download;
    }
    let Ok(exe) = std::env::current_exe() else {
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
    let exe = std::env::current_exe()?;
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
    match run_installer(true) {
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

/// The one-line installer, without the Hyprland questions (an update keeps the
/// existing keybind and window rule). `show` passes its output through.
fn run_installer(show: bool) -> Result<(), String> {
    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg(format!(
            "set -o pipefail; curl -fsSL {INSTALLER_URL} | bash -s -- --no-hyprland"
        ))
        // The installer treats a folder with Cargo.toml as a source checkout.
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null());
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
}

fn app_bundle(exe: &Path) -> Option<&Path> {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
}

fn enabled() -> bool {
    if std::env::var_os("OMACUS_NO_UPDATE_CHECK").is_some() {
        return false;
    }
    let Some(path) = dirs::config_dir().map(|d| d.join("omacus").join("settings.toml")) else {
        return true;
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.parse::<toml::Table>().ok())
        .and_then(|t| t.get("check_for_updates").and_then(toml::Value::as_bool))
        .unwrap_or(true)
}

fn fetch_latest() -> Result<String, Box<dyn std::error::Error>> {
    let body = ureq::get(LATEST_URL)
        .header("User-Agent", concat!("omacus/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_to_string()?;
    let tag = serde_json::from_str::<Release>(&body)?.tag_name;
    Ok(tag.trim_start_matches('v').to_string())
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
    fn compares_versions() {
        assert!(is_newer("0.1.3", "0.1.2"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
    }
}
