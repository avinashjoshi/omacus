#!/usr/bin/env python3
"""A stuck disk image is stopped and tried again. The step cannot run for hours."""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / ".github" / "scripts" / "dmg-retry.sh"


def run(args, timeout):
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )


def require(condition, message):
    if not condition:
        print(message)
        sys.exit(1)


ok = run(["2", "true"], timeout=10)
require(ok.returncode == 0, f"a finished command should succeed\n{ok.stderr}")

stuck = run(["1", "sleep", "30"], timeout=15)
require(stuck.returncode == 1, f"a stuck command should fail, got {stuck.returncode}")
require(
    "did not finish (try 1)" in stuck.stderr and "did not finish (try 2)" in stuck.stderr,
    f"a stuck command should be tried twice\n{stuck.stderr}",
)

release = (ROOT / ".github" / "workflows" / "release.yml").read_text()
macos = release[release.index("  macos:"):release.index("\n  publish:")]
bundle = macos[macos.index("Bundle Omacus.app"):macos.index("Check each .dmg")]
check = macos[macos.index("Check each .dmg"):]
require("timeout-minutes: 40" in bundle, "the disk image step needs a timeout")
require("dmg-retry.sh 180 dmgbuild" in bundle, "dmgbuild needs a deadline and a retry")
require("timeout-minutes: 10" in check, "opening the disk image needs a timeout")
require("dmg-retry.sh 60 hdiutil attach" in check, "hdiutil attach needs a retry")
require("dmg-retry.sh 60 hdiutil detach" in check, "hdiutil detach needs a retry")

ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
step = ci[ci.index("name: macOS .dmg"):ci.index("name: Keep the .dmg")]
require("timeout-minutes: 40" in step, "the CI disk image step needs a timeout")
require("dmg-retry.sh 180 dmgbuild" in step, "the CI disk image needs a retry")
require("dmg-retry.sh 60 hdiutil attach" in step, "the CI attach needs a retry")
require("dmg-retry.sh 60 hdiutil detach" in step, "the CI detach needs a retry")

print("disk image retry ok")
