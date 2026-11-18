#!/usr/bin/env bash
# Ubuntu x64 runners use the pinned upstream CLI instead of compiling it.
set -euo pipefail

version=0.2.129
archive="wasm-bindgen-$version-x86_64-unknown-linux-musl"
sha256=82d12bb940e2d4e72e0d5605387fc1b8ca179044e012b620f0ce4e7440e8320e
directory="$(mktemp -d "$RUNNER_TEMP/wasm-bindgen.XXXXXX")"

curl -fsSL --retry 3 --max-time 30 \
  "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$version/$archive.tar.gz" \
  -o "$directory/$archive.tar.gz"
printf '%s  %s\n' "$sha256" "$directory/$archive.tar.gz" | sha256sum -c -
tar -xzf "$directory/$archive.tar.gz" -C "$directory" "$archive/wasm-bindgen"
"$directory/$archive/wasm-bindgen" --version
printf '%s\n' "$directory/$archive" >> "$GITHUB_PATH"
