#!/usr/bin/env python3
"""Browser tests use the pinned Playwright image, and CI jobs are bounded."""

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
version = json.loads((ROOT / "package.json").read_text())["devDependencies"]["@playwright/test"]
if not re.fullmatch(r"\d+\.\d+\.\d+", version):
    raise SystemExit(f"unexpected Playwright version {version}")

ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
errors = []

image = rf"mcr\.microsoft\.com/playwright:v{re.escape(version)}-noble@sha256:[0-9a-f]{{64}}"
if not re.search(image, ci):
    errors.append(f"the wasm job must run browsers from the Playwright {version} image, pinned by digest")
if "playwright install --with-deps" in ci:
    errors.append("the wasm job must not apt-install Playwright system libraries")
if "PLAYWRIGHT_BROWSERS_PATH=/ms-playwright" not in ci:
    errors.append("the wasm job must use the browsers baked into the image")
if "PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1" not in ci:
    errors.append("npm ci must not download a second copy of the browsers")


def job(name):
    start = ci.index(f"  {name}:\n")
    steps = ci.index("\n    steps:\n", start)
    return ci[start:steps]


for name, limit in (("test", 30), ("wasm", 20)):
    header = job(name)
    match = re.search(r"timeout-minutes:\s*(\d+)", header)
    if not match:
        errors.append(f"the {name} job must set timeout-minutes")
        continue
    minutes = int(match.group(1))
    if minutes > limit:
        errors.append(f"the {name} job timeout is {minutes} minutes, above {limit}")

config = (ROOT / "playwright.config.mjs").read_text()
if 'name: "chromium"' not in config or 'name: "webkit"' not in config:
    errors.append("browser tests must still cover Chromium and WebKit")

if errors:
    raise SystemExit("\n".join(errors))
print(f"browser CI uses Playwright {version}")
