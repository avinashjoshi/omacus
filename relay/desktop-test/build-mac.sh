#!/bin/bash
# Build the unsigned omacus Test app. Run this on a Mac.
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$root/../.." && pwd)"
dest="$root/dist/omacus Test.app"
rm -rf "$root/dist"
mkdir -p "$dest/Contents/MacOS" "$dest/Contents/Resources"
cp "$root/Info.plist" "$dest/Contents/Info.plist"
swiftc -O -o "$dest/Contents/MacOS/omacus-test" "$root/main.swift" -framework Cocoa -framework WebKit
chmod 755 "$dest/Contents/MacOS/omacus-test"
sips -s format icns "$repo/assets/icon-512.png" --out "$dest/Contents/Resources/icon.icns"
codesign --force --sign - "$dest"
ditto -c -k --keepParent "$dest" "$root/dist/omacus-test-mac.zip"
echo "$root/dist/omacus-test-mac.zip"
