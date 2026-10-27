//! The desktop binary against a Secret Service stand-in, and a live daemon
//! when this machine has one. No collection, a locked collection, and an
//! unlocked collection.
#![cfg(all(unix, not(target_os = "macos")))]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn root(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("omacus-keyring-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn tools(dir: &Path) -> (PathBuf, PathBuf) {
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let dbus = bin.join("dbus-send");
    executable(
        &dbus,
        r#"#!/bin/sh
mode="${OMACUS_E2E_COLLECTION:-none}"
if printf '%s\n' "$@" | grep -q ReadAlias; then
  if [ "$mode" = "healthy" ] || [ "$mode" = "locked" ]; then
    if printf '%s\n' "$@" | grep -q 'string:default'; then
      printf '%s\n' 'object path "/org/freedesktop/secrets/collection/login"'
      exit 0
    fi
  fi
  printf '%s\n' 'object path "/"'
  exit 0
fi
if printf '%s\n' "$@" | grep -q Properties.Get; then
  if [ "$mode" = "locked" ]; then
    printf '%s\n' 'variant boolean true'
  else
    printf '%s\n' 'variant boolean false'
  fi
  exit 0
fi
echo 'unexpected call' >&2
exit 1
"#,
    );
    let secret = bin.join("secret-tool");
    executable(
        &secret,
        r#"#!/bin/sh
if [ "${OMACUS_E2E_SECRET_SLEEP:-}" = "1" ]; then
  printf '%s\n' "$$" > "$OMACUS_E2E_PID"
  sleep 30
  exit 0
fi
if [ "$1" = "store" ]; then
  cat >/dev/null
  if [ "${OMACUS_E2E_COLLECTION:-}" = "healthy" ]; then
    exit 0
  fi
  exit 1
fi
exit 1
"#,
    );
    (dbus, secret)
}

fn run_binary(dir: &Path, mode: &str, sleep: bool) -> (std::process::Output, Duration) {
    let (dbus, secret) = tools(dir);
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let pid = dir.join("sleeper.pid");
    let mut command = Command::new(env!("CARGO_BIN_EXE_omacus"));
    command
        .env("OMACUS_E2E_KEYRING", "1")
        .env("OMACUS_E2E_DIR", &data)
        .env("OMACUS_E2E_COLLECTION", mode)
        .env("OMACUS_DBUS_SEND", &dbus)
        .env("OMACUS_SECRET_TOOL", &secret)
        .env("OMACUS_E2E_PID", &pid)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if sleep {
        command.env("OMACUS_E2E_SECRET_SLEEP", "1");
    }
    let start = Instant::now();
    let output = command.output().unwrap();
    (output, start.elapsed())
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

#[test]
fn the_desktop_binary_falls_back_without_a_default_collection() {
    let dir = root("none");
    let (output, elapsed) = run_binary(&dir, "none", false);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "fallback");
    assert!(elapsed < Duration::from_secs(12));
}

#[test]
fn the_desktop_binary_falls_back_when_the_collection_is_locked() {
    let dir = root("locked");
    let (output, elapsed) = run_binary(&dir, "locked", false);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "fallback");
    assert!(elapsed < Duration::from_secs(12));
}

#[test]
fn the_desktop_binary_stores_when_the_collection_is_unlocked() {
    let dir = root("healthy");
    let (output, elapsed) = run_binary(&dir, "healthy", false);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "stored");
    assert!(elapsed < Duration::from_secs(12));
}

#[test]
fn the_desktop_binary_kills_a_stuck_secret_tool() {
    let dir = root("stuck");
    let (output, elapsed) = run_binary(&dir, "healthy", true);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "fallback");
    assert!(elapsed < Duration::from_secs(12));
    let pid: i32 = fs::read_to_string(dir.join("sleeper.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let alive = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "secret-tool was still running");
}

#[test]
fn a_live_secret_service_without_a_default_collection_falls_back() {
    if !Path::new("/usr/bin/gnome-keyring-daemon").exists()
        || !Path::new("/usr/bin/dbus-daemon").exists()
        || !Path::new("/usr/bin/dbus-send").exists()
    {
        return;
    }
    let dir = root("live");
    let runtime = dir.join("run");
    fs::create_dir_all(&runtime).unwrap();
    let bus = Command::new("dbus-daemon")
        .args([
            "--session",
            "--nofork",
            "--nopidfile",
            "--address",
            &format!("unix:path={}", runtime.join("bus").display()),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut bus) = bus else {
        return;
    };
    let address = format!("unix:path={}", runtime.join("bus").display());
    let data_home = dir.join("data-home");
    fs::create_dir_all(&data_home).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let keyring = Command::new("gnome-keyring-daemon")
        .args(["--components=secrets", "--foreground"])
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_DATA_HOME", &data_home)
        .env_remove("DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut keyring) = keyring else {
        let _ = bus.kill();
        return;
    };
    std::thread::sleep(Duration::from_millis(400));
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let start = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_omacus"))
        .env("OMACUS_E2E_KEYRING", "1")
        .env("OMACUS_E2E_DIR", &data)
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_DATA_HOME", &data_home)
        .env_remove("DISPLAY")
        .env_remove("OMACUS_DBUS_SEND")
        .env_remove("OMACUS_SECRET_TOOL")
        .output()
        .unwrap();
    let _ = keyring.kill();
    let _ = bus.kill();
    assert!(start.elapsed() < Duration::from_secs(12));
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "fallback");
}
