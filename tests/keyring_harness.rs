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
  if [ "${OMACUS_E2E_COLLECTION:-}" = "healthy" ]; then
    # Like the real secret-tool: it reads at most 8192 bytes and keeps them.
    head -c 8192 > "$OMACUS_E2E_SLOT"
    cat >/dev/null
    exit 0
  fi
  cat >/dev/null
  exit 1
fi
if [ "$1" = "lookup" ] && [ -f "$OMACUS_E2E_SLOT" ]; then
  cat "$OMACUS_E2E_SLOT"
  exit 0
fi
exit 1
"#,
    );
    (dbus, secret)
}

fn run_binary(dir: &Path, mode: &str, sleep: bool) -> (std::process::Output, Duration) {
    run_with(dir, mode, sleep, &[])
}

fn run_with(
    dir: &Path,
    mode: &str,
    sleep: bool,
    extra: &[(&str, &str)],
) -> (std::process::Output, Duration) {
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
        .env("OMACUS_E2E_SLOT", dir.join("slot"))
        .envs(extra.iter().copied())
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

fn assert_no_pass_file(dir: &Path) {
    let file = dir.join("sync").join("device.bin");
    if let Ok(bytes) = fs::read(&file) {
        assert!(
            !bytes.windows(5).any(|window| window == b"pass\n"),
            "a passphrase file was written"
        );
    }
}

#[test]
fn the_desktop_binary_asks_for_setup_without_a_default_collection() {
    let dir = root("none");
    let (output, elapsed) = run_binary(&dir, "none", false);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "setup");
    assert!(elapsed < Duration::from_secs(12));
    assert_no_pass_file(&dir.join("data"));
}

#[test]
fn the_desktop_binary_asks_for_setup_when_the_collection_is_locked() {
    let dir = root("locked");
    let (output, elapsed) = run_binary(&dir, "locked", false);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "setup");
    assert!(elapsed < Duration::from_secs(12));
    assert_no_pass_file(&dir.join("data"));
}

#[test]
fn the_desktop_binary_stores_when_the_collection_is_unlocked() {
    let dir = root("healthy");
    let (output, elapsed) = run_binary(&dir, "healthy", false);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "stored");
    assert!(elapsed < Duration::from_secs(12));
}

/// secret-tool keeps only the first 8192 bytes it is given. A sync key with
/// many pages behind it is longer than that, and must still open.
#[test]
fn the_desktop_binary_reopens_a_key_longer_than_secret_tool_keeps() {
    let dir = root("large");
    let large = [("OMACUS_E2E_LARGE", "1")];
    let (output, _) = run_with(&dir, "healthy", false, &large);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "stored");
    let read = [("OMACUS_E2E_LARGE", "1"), ("OMACUS_E2E_KEYRING_READ", "1")];
    let (output, _) = run_with(&dir, "healthy", false, &read);
    assert_eq!(text(&output.stdout), "reopened", "{}", text(&output.stderr));
    let slot = fs::metadata(dir.join("slot")).unwrap().len();
    assert!(slot < 200, "the keyring holds {slot} bytes, not just a key");
}

#[test]
fn the_desktop_binary_kills_a_stuck_secret_tool() {
    let dir = root("stuck");
    let (output, elapsed) = run_binary(&dir, "healthy", true);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "timeout");
    assert!(elapsed < Duration::from_secs(12));
    assert_no_pass_file(&dir.join("data"));
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

fn live_store(name: &str, prepare: bool) -> Option<(std::process::Output, PathBuf)> {
    if !Path::new("/usr/bin/gnome-keyring-daemon").exists()
        || !Path::new("/usr/bin/dbus-daemon").exists()
        || !Path::new("/usr/bin/dbus-send").exists()
    {
        return None;
    }
    let dir = root(name);
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
        return None;
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
        return None;
    };
    std::thread::sleep(Duration::from_millis(400));
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_omacus"));
    command
        .env("OMACUS_E2E_KEYRING", "1")
        .env("OMACUS_E2E_DIR", &data)
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_DATA_HOME", &data_home)
        .env_remove("DISPLAY")
        .env_remove("OMACUS_DBUS_SEND")
        .env_remove("OMACUS_SECRET_TOOL");
    if prepare {
        command.env("OMACUS_E2E_PREPARE", "1");
    }
    let output = command.output().unwrap();
    let _ = keyring.kill();
    let _ = bus.kill();
    Some((output, data))
}

#[test]
fn a_live_secret_service_without_a_default_collection_asks_for_setup() {
    let Some((output, data)) = live_store("live", false) else {
        return;
    };
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "setup");
    assert_no_pass_file(&data);
}

#[test]
fn a_live_secret_service_setup_stores_without_a_passphrase() {
    let Some((output, data)) = live_store("live-prepare", true) else {
        return;
    };
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        text(&output.stderr),
        text(&output.stdout)
    );
    assert_eq!(
        text(&output.stdout),
        "stored",
        "stderr: {}",
        text(&output.stderr)
    );
    let stored = fs::read(data.join("sync").join("device.bin")).unwrap();
    assert!(stored.windows(8).any(|window| window == b"wrapped\n"));
    assert_no_pass_file(&data);
}

#[test]
fn a_live_secret_service_reopens_without_asking_for_a_passphrase() {
    if !Path::new("/usr/bin/gnome-keyring-daemon").exists()
        || !Path::new("/usr/bin/dbus-daemon").exists()
        || !Path::new("/usr/bin/dbus-send").exists()
    {
        return;
    }
    let dir = root("live-reopen");
    let runtime = dir.join("run");
    fs::create_dir_all(&runtime).unwrap();
    let mut bus = Command::new("dbus-daemon")
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
        .spawn()
        .unwrap();
    let address = format!("unix:path={}", runtime.join("bus").display());
    let data_home = dir.join("data-home");
    fs::create_dir_all(&data_home).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let mut keyring = Command::new("gnome-keyring-daemon")
        .args(["--components=secrets", "--foreground"])
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_DATA_HOME", &data_home)
        .env_remove("DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let first = dir.join("first");
    fs::create_dir_all(&first).unwrap();
    let setup = Command::new(env!("CARGO_BIN_EXE_omacus"))
        .env("OMACUS_E2E_KEYRING", "1")
        .env("OMACUS_E2E_DIR", &first)
        .env("OMACUS_E2E_PREPARE", "1")
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_DATA_HOME", &data_home)
        .env_remove("DISPLAY")
        .output()
        .unwrap();
    assert_eq!(text(&setup.stdout), "stored", "{}", text(&setup.stderr));
    let _ = keyring.kill();
    let _ = keyring.wait();
    std::thread::sleep(Duration::from_millis(200));
    let mut again = Command::new("gnome-keyring-daemon")
        .args(["--components=secrets", "--foreground"])
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_DATA_HOME", &data_home)
        .env_remove("DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let opened = Command::new(env!("CARGO_BIN_EXE_omacus"))
        .env("OMACUS_E2E_KEYRING", "1")
        .env("OMACUS_E2E_DIR", &first)
        .env("OMACUS_E2E_KEYRING_READ", "1")
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_DATA_HOME", &data_home)
        .env_remove("DISPLAY")
        .env_remove("OMACUS_E2E_PREPARE")
        .output()
        .unwrap();
    let _ = again.kill();
    let _ = again.wait();
    let _ = bus.kill();
    let _ = bus.wait();
    assert_eq!(
        text(&opened.stdout),
        "reopened",
        "stderr: {}",
        text(&opened.stderr)
    );
    assert_no_pass_file(&first);
}
