#!/usr/bin/env python3
"""A stuck disk image is stopped and tried again. The step cannot run for hours."""

import os
import plistlib
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

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

release = (ROOT / ".github" / "workflows" / "release-build.yml").read_text()
macos = release[release.index("  macos:"):]
bundle = macos[macos.index("Bundle Omacus.app"):macos.index("Check the disk image")]
check = macos[macos.index("Check the disk image"):]
require("timeout-minutes: 30" in bundle, "the disk image step needs a timeout")
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

# Model the two Apple icon commands on Linux. macOS CI checks the actual icon
# and mounts the .dmg. Here we exercise the bundle layout and metadata.
with TemporaryDirectory() as directory:
    work = Path(directory)
    tools = work / "bin"
    tools.mkdir()
    sips = tools / "sips"
    sips.write_text('#!/bin/sh\ncp "$4" "$6"\n')
    sips.chmod(0o755)
    iconutil = tools / "iconutil"
    iconutil.write_text(
        '#!/bin/sh\n'
        'set -e\n'
        'test "$(find "$3" -name "icon_*.png" | wc -l)" -eq 10\n'
        'cp "$3/icon_512x512@2x.png" "$5"\n'
    )
    iconutil.chmod(0o755)
    binary = work / "test-binary"
    binary.write_bytes(b"fixture executable")
    binary.chmod(0o755)
    subprocess.run(
        [sys.executable, str(ROOT / ".github/scripts/bundle-macos.py"), str(binary), str(work)],
        env={**os.environ, "PATH": f"{tools}{os.pathsep}{os.environ['PATH']}"},
        check=True,
    )
    contents = work / "Omacus.app" / "Contents"
    plist = plistlib.loads((contents / "Info.plist").read_bytes())
    bundled = contents / "MacOS" / plist["CFBundleExecutable"]
    require(bundled.read_bytes() == binary.read_bytes() and os.access(bundled, os.X_OK),
            "the bundle must carry the prebuilt executable and preserve its executable bit")
    require((contents / "Resources" / plist["CFBundleIconFile"]).stat().st_size > 0,
            "the bundle must carry its declared icon")
    require(plist["CFBundleIdentifier"] == "dev.omacus.Omacus", "the bundle identifier changed")
    require(plist["CFBundleURLTypes"][0]["CFBundleURLSchemes"] == ["omacus"],
            "the bundle must register Omacus share links")

print("disk image retry and macOS bundle layout ok")
