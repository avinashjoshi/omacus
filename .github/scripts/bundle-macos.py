#!/usr/bin/env python3
"""Wrap an already-built Omacus executable in a macOS app bundle."""

import argparse
import json
import plistlib
import shutil
import subprocess
from pathlib import Path
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("output_dir", type=Path)
    args = parser.parse_args()

    workspace = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
    ))
    package = next(p for p in workspace["packages"] if p["name"] == "omacus")
    bundle = package["metadata"]["bundle"]
    name, version = bundle["name"], package["version"]
    contents = args.output_dir / f"{name}.app" / "Contents"
    resources = contents / "Resources"
    resources.mkdir(parents=True, exist_ok=True)
    executable_dir = contents / "MacOS"
    executable_dir.mkdir(exist_ok=True)
    shutil.copy2(args.executable, executable_dir / package["name"])

    icon = f"{name}.icns"
    # The largest source supplies every size in the macOS icon set.
    source = ROOT / bundle["icon"][-1]
    with TemporaryDirectory() as directory:
        iconset = Path(directory) / f"{name}.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            for scale, suffix in ((1, ""), (2, "@2x")):
                pixels = str(size * scale)
                subprocess.run([
                    "sips", "-z", pixels, pixels, str(source),
                    "--out", str(iconset / f"icon_{size}x{size}{suffix}.png"),
                ], check=True, stdout=subprocess.DEVNULL)
        subprocess.run([
            "iconutil", "-c", "icns", str(iconset), "-o", str(resources / icon),
        ], check=True)

    with (contents / "Info.plist").open("wb") as plist:
        plistlib.dump({
            "CFBundleDevelopmentRegion": "English",
            "CFBundleDisplayName": name,
            "CFBundleExecutable": package["name"],
            "CFBundleIconFile": icon,
            "CFBundleIdentifier": bundle["identifier"],
            "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundleName": name,
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": version,
            "CFBundleVersion": version,
            "CFBundleURLTypes": [{
                "CFBundleURLName": name,
                "CFBundleTypeRole": "Viewer",
                "CFBundleURLSchemes": bundle["osx"]["url_schemes"],
            }],
            "LSApplicationCategoryType": bundle["category"],
            "NSHighResolutionCapable": True,
        }, plist)


if __name__ == "__main__":
    main()
