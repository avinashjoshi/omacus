#!/usr/bin/env python3
"""omacus Test stays a separate app and is published as a prerelease."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
plist = (ROOT / "relay/desktop-test/Info.plist").read_text()
swift = (ROOT / "relay/desktop-test/main.swift").read_text()
script = (ROOT / "relay/desktop-test/build-mac.sh").read_text()
workflow = (ROOT / ".github/workflows/desktop-test.yml").read_text()
cargo = (ROOT / "Cargo.toml").read_text()

errors = []


def need(cond, message):
    if not cond:
        errors.append(message)


need('>omacus Test<' in plist, "app name")
need(">dev.omacus.OmacusTest<" in plist, "bundle id")
need(">dev.omacus.Omacus<" not in plist, "real bundle id")
need("omacus://" not in plist and "CFBundleURLTypes" not in plist, "url scheme")
need("https://sync-test.omacus.com/" in swift, "sync test host")
need("sync.omacus.com" not in swift.replace("sync-test.omacus.com", ""), "production sync host")
need("dev.omacus.OmacusTest" in swift, "runtime id check")
need("Application Support" not in swift, "real data folder")
need("Right-click" in (ROOT / "relay/desktop-test/NOTES.txt").read_text(), "mac install step")
need("omacus Test.app" in script, "app bundle name")
need("arm64-apple-macosx12.0" in script and "x86_64-apple-macosx12.0" in script and "lipo -create" in script, "both mac chips")
need("codesign --force --sign -" in script, "ad-hoc signature only")
need("prerelease" in workflow, "prerelease")
need("omacus-test-desktop" in workflow, "prerelease tag")
need("branches: [main]" not in workflow and "v${{" not in workflow, "not a normal release")
need('identifier = "dev.omacus.Omacus"' in cargo, "real app id unchanged")
need("OmacusTest" not in cargo, "cargo bundle id unchanged")

if errors:
    raise SystemExit("\n".join(errors))
print("desktop test ok")
