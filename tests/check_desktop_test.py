#!/usr/bin/env python3
"""omacus Test stays a separate app and is published as a prerelease."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
plist = (ROOT / "relay/desktop-test/Info.plist").read_text()
swift = (ROOT / "relay/desktop-test/main.swift").read_text()
script = (ROOT / "relay/desktop-test/build-mac.sh").read_text()
linux = (ROOT / "relay/desktop-test/main-linux.c").read_text()
linux_script = (ROOT / "relay/desktop-test/build-linux.sh").read_text()
desktop = (ROOT / "relay/desktop-test/dev.omacus.OmacusTest.desktop").read_text()
notes = (ROOT / "relay/desktop-test/NOTES.txt").read_text()
linux_notes = (ROOT / "relay/desktop-test/LINUX.txt").read_text()
ops = (ROOT / "docs/sync-ops.md").read_text()
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
need("Right-click" in notes, "mac install step")
need("omacus Test.app" in script, "app bundle name")
need("arm64-apple-macosx12.0" in script and "x86_64-apple-macosx12.0" in script and "lipo -create" in script, "both mac chips")
need("codesign --force --sign -" in script, "ad-hoc signature only")
need("https://sync-test.omacus.com/" in linux, "linux sync test host")
need("sync.omacus.com" not in linux.replace("sync-test.omacus.com", ""), "linux production sync host")
need("dev.omacus.OmacusTest" in linux, "linux runtime id check")
need('strcmp(base, "omacus")' in linux, "refuses the installed name")
need('".local", "share", DATA_NAME' in linux and 'DATA_NAME "omacus-test"' in linux, "linux data folder")
need('".local", "share", "omacus"' not in linux, "does not build the installed data folder")
need('".config"' not in linux, "does not touch settings")
need("omacus://" not in linux and "x-scheme-handler" not in linux, "linux url scheme")
need("xdg-mime" not in linux and "update-desktop-database" not in linux, "linux does not register a handler")
need("Application Support" not in linux, "linux real data folder")
need("Name=omacus Test" in desktop, "linux desktop name")
need("StartupWMClass=dev.omacus.OmacusTest" in desktop, "linux window class")
need("MimeType" not in desktop and "omacus://" not in desktop, "linux desktop has no link handler")
need("Exec=omacus-test" in desktop and "Exec=omacus " not in desktop and "Exec=omacus\n" not in desktop, "linux desktop command")
need("omacus-test-linux-x86_64.tar.gz" in linux_script, "linux tarball")
need("libwebkit2gtk-4.1.so.0" in linux_script and "libgtk-3.so.0" in linux_script, "linux shared libraries")
for line in linux_script.splitlines():
    if "omacus.desktop" in line:
        need(line.strip().startswith("test ! -e"), "build does not install the real desktop file")
need("webkit2gtk-4.1" in notes and "webkit2gtk-4.1" in linux_notes and "webkit2gtk-4.1" in ops, "pacman package")
need("omacus-test-linux-x86_64.tar.gz" in notes and "omacus-test-linux-x86_64.tar.gz" in ops, "linux download name")
remove = 'rm -rf "$HOME/omacus-test-linux" "$HOME/.local/share/omacus-test" "$HOME/.cache/omacus-test"'
need(remove in notes and remove in linux_notes and remove in ops, "linux remove")
need('rm -f "$HOME/.local/share/applications/dev.omacus.OmacusTest.desktop"' in ops, "linux launcher remove")
for line in ops.splitlines() + notes.splitlines() + linux_notes.splitlines():
    if line.strip().startswith("rm ") or line.strip().startswith("rm\t"):
        need("omacus.desktop" not in line, "remove does not delete the installed desktop file")
        need(".local/share/omacus\"" not in line and ".local/share/omacus " not in line, "remove does not delete installed pages")
        need(".config/omacus" not in line, "remove does not delete settings")
need("prerelease" in workflow, "prerelease")
need("omacus-test-desktop" in workflow, "prerelease tag")
need("omacus-test-mac.zip" in workflow and "omacus-test-linux-x86_64.tar.gz" in workflow, "both prerelease files")
need(workflow.count("gh release create") == 1 and workflow.count("gh release delete") == 1, "one publish")
need("branches: [main]" not in workflow and "v${{" not in workflow, "not a normal release")
need('identifier = "dev.omacus.Omacus"' in cargo, "real app id unchanged")
need("OmacusTest" not in cargo, "cargo bundle id unchanged")

if errors:
    raise SystemExit("\n".join(errors))
print("desktop test ok")
