#!/usr/bin/env python3
"""The desktop app, the shell, and both web engines carry one release version.

A page can show a new HTML version while a cached script is still the previous
one. The release script bumps these together, and this check fails when they drift.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CRATES = {
    "omacus": ROOT / "Cargo.toml",
    "omacus-engine": ROOT / "engine" / "Cargo.toml",
    "omacus-web": ROOT / "web" / "Cargo.toml",
    "omacus-sync-web": ROOT / "sync-web" / "Cargo.toml",
    "omacus-doc": ROOT / "doc" / "Cargo.toml",
    "omacus-relay": ROOT / "relay" / "Cargo.toml",
}
LOCK_NAMES = (
    "omacus",
    "omacus-doc",
    "omacus-engine",
    "omacus-relay",
    "omacus-sync-web",
    "omacus-web",
)


def fail(message: str) -> None:
    print(message)
    sys.exit(1)


def main() -> None:
    cargo = (ROOT / "Cargo.toml").read_text()
    found = re.search(r'(?m)^version = "([^"]+)"', cargo)
    if not found:
        fail("Cargo.toml has no workspace version")
    version = found.group(1)
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        fail(f"workspace version is not major.minor.patch: {version}")

    for name, path in CRATES.items():
        text = path.read_text()
        if name == "omacus":
            if "version.workspace = true" not in text:
                fail("the desktop package does not use the workspace version")
            continue
        if "version.workspace = true" not in text:
            fail(f"{name} does not use the workspace version")
        if re.search(r'(?m)^version = "', text):
            fail(f"{name} has its own version")

    lock = (ROOT / "Cargo.lock").read_text()
    for name in LOCK_NAMES:
        needle = f'name = "{name}"\nversion = "{version}"\n'
        if lock.count(needle) != 1:
            fail(f"Cargo.lock entry for {name} is not {version}")

    html = (ROOT / "site/app/index.html").read_text()
    if f'name="omacus-version" content="{version}"' not in html:
        fail("index.html omacus-version does not match the desktop app")
    if f">omacus v{version}<" not in html:
        fail("the sidebar version does not match the desktop app")

    version_js = (ROOT / "site/app/version.js").read_text()
    if f'export const APP_VERSION = "{version}"' not in version_js:
        fail("version.js does not match the desktop app")

    sw = (ROOT / "site/app/sw.js").read_text()
    if f'const APP_VERSION = "{version}"' not in sw:
        fail("the service worker version does not match the desktop app")
    if "omacus-app-${APP_VERSION}" not in sw:
        fail("the service worker cache is not named from the release version")

    raw_probe = (ROOT / "site/app/version.json").read_text()
    probe = json.loads(raw_probe)
    if probe.get("version") != version:
        fail("version.json does not match the desktop app")
    if f'"version": "{version}"' not in raw_probe:
        fail("version.json is not the text the release script updates")

    deploy = (ROOT / ".github/actions/deploy-app/action.yml").read_text()
    if 'json.load(open("dist/version.json"))' not in deploy:
        fail("the web deploy does not read version.json as JSON")

    for path in (ROOT / "web/src/lib.rs", ROOT / "sync-web/src/lib.rs"):
        source = path.read_text()
        if 'env!("CARGO_PKG_VERSION")' not in source or "fn app_version" not in source:
            fail(f"{path.relative_to(ROOT)} does not report the crate version")

    prepare = (ROOT / ".github/scripts/prepare-release.sh").read_text()
    for token in ("version.js", "version.json", "sw.js", "omacus-web", "omacus-sync-web", "omacus-engine"):
        if token not in prepare:
            fail(f"prepare-release.sh does not bump {token}")

    print(f"versions match {version}")


if __name__ == "__main__":
    main()
