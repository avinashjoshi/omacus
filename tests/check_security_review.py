#!/usr/bin/env python3
"""The security-review fixes stay wired: environments, hashes, headers, npm."""

from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def fail(message: str) -> None:
    raise SystemExit(message)


def header_values(path: Path, name: str) -> list[str]:
    values = []
    prefix = name + ":"
    for line in path.read_text().splitlines():
        stripped = line.strip()
        if stripped.startswith(prefix):
            values.append(stripped.split(":", 1)[1].strip())
    return values


def main() -> int:
    app = (ROOT / ".github" / "workflows" / "app.yml").read_text()
    deploy = (ROOT / ".github" / "actions" / "deploy-app" / "action.yml").read_text()
    release = (ROOT / ".github" / "workflows" / "release.yml").read_text()
    if "environment: app" not in app or "environment: app" not in release:
        fail("the web deploy has no app environment")
    if "web-SHA256SUMS.txt" not in deploy or "app.omacus.com/SHA256SUMS.txt" not in deploy:
        fail("the web deploy does not publish the file list")
    uploads = [line.strip() for line in deploy.splitlines() if "gh release upload" in line]
    if uploads != ['gh release upload "v$version" "dist/web-SHA256SUMS.txt" --clobber']:
        fail("the web deploy must upload web-SHA256SUMS.txt and no other release file")
    if "SHA256SUMS.txt#" in deploy or "dist/SHA256SUMS.txt\"" in deploy.split("gh release upload", 1)[-1]:
        fail("the web deploy can replace the desktop checksum file")
    guard = r"^[[:xdigit:]]{64} [ *]?(install\.sh|omacus-)"
    if guard not in deploy:
        fail("the web deploy does not refuse a desktop checksum list")
    # The same guard, in Python, so a desktop line cannot pass as a web path.
    desktop = re.compile(r"^[0-9a-f]{64} [ *]?(install\.sh|omacus-)")
    if not desktop.search("589e09ada2a738fb4cdeb73128859a2aac923a1ecb241fca7342b7b48a3819fc  omacus-linux-x86_64.tar.gz"):
        fail("the desktop checksum guard missed a package")
    if desktop.search("22852b99782593c24c2be310b45099264d768d373854f0bd7094cf8905a1cb9e  ./app.js"):
        fail("the desktop checksum guard matched a web file")
    if not desktop.search("727f0720f04bc9d9f8390b5f433f23afd2ae24f976660599ed84ded740af4c2c  install.sh"):
        fail("the desktop checksum guard missed install.sh")

    build = (ROOT / ".github" / "scripts" / "build-web.sh").read_text()
    if "SHA256SUMS.txt" not in build:
        fail("the web build does not write a file list")

    dependabot = (ROOT / ".github" / "dependabot.yml").read_text()
    if "package-ecosystem: npm" not in dependabot:
        fail("dependabot does not watch npm")

    runbook = (ROOT / "docs" / "ops" / "runbook.md").read_text()
    for phrase in (
        "GitHub Pages does not send",
        "Copies from 0.2.x",
        "omacus-update.json.minisig",
        "rotate `ADMIN_KEY`",
    ):
        if phrase not in runbook:
            fail(f"runbook is missing {phrase}")

    pub = (ROOT / "keys" / "omacus-update.pub").read_text().splitlines()
    key = next(line.strip() for line in pub if line.startswith("RW"))
    if key not in (ROOT / "install.sh").read_text():
        fail("install.sh does not carry the update public key")
    if "minisign -Vm" not in (ROOT / "install.sh").read_text():
        fail("install.sh does not check a minisign signature")

    rules = json.loads((ROOT / "docs" / "ops" / "cloudflare-headers.json").read_text())
    policies = [rule["action_parameters"]["headers"]["Content-Security-Policy"]["value"]
                for rule in rules["rules"]
                if "Content-Security-Policy" in rule["action_parameters"]["headers"]]
    site = header_values(ROOT / "site" / "_headers", "Content-Security-Policy")
    app_headers = header_values(ROOT / "site" / "app" / "_headers", "Content-Security-Policy")
    for policy in policies:
        if policy not in site and policy not in app_headers:
            fail("a Cloudflare policy is not the policy in _headers")
    if "Strict-Transport-Security:" not in (ROOT / "site" / "app" / "_headers").read_text():
        fail("the web app headers have no HSTS")
    if "require-trusted-types-for" not in app_headers[0]:
        fail("the web app policy dropped trusted types")
    if not (ROOT / "docs" / "ops" / "release-protection.md").is_file():
        fail("the settings notes are missing")
    print("security review checks ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
