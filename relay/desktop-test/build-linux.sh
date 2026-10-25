#!/bin/bash
# Build the Linux omacus Test tarball. Run this on x86_64 Linux.
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
rm -rf "$root/dist/linux"
mkdir -p "$root/dist/linux"

if [[ "$(uname -m)" != "x86_64" ]]; then
  echo "omacus Test for Linux is built for x86_64, not $(uname -m)." >&2
  exit 1
fi

gcc -O2 -Wall -Wextra -Werror -o "$root/dist/linux/omacus-test" "$root/main-linux.c" \
  $(pkg-config --cflags --libs gtk+-3.0 webkit2gtk-4.1)
strip "$root/dist/linux/omacus-test"
file "$root/dist/linux/omacus-test" | grep -q 'x86-64'
ldd "$root/dist/linux/omacus-test" | grep -q 'libwebkit2gtk-4.1.so.0'
ldd "$root/dist/linux/omacus-test" | grep -q 'libgtk-3.so.0'

cp "$root/dev.omacus.OmacusTest.desktop" "$root/dist/linux/dev.omacus.OmacusTest.desktop"
cp "$root/LINUX.txt" "$root/dist/linux/README.txt"

home="$(mktemp -d)"
trap 'rm -rf "$home"' EXIT
out="$(HOME="$home" OMACUS_TEST_CHECK=1 "$root/dist/linux/omacus-test")"
printf '%s\n' "$out" | grep -qx "$home/.local/share/omacus-test"
printf '%s\n' "$out" | grep -qx "$home/.cache/omacus-test"
test -d "$home/.local/share/omacus-test"
test -d "$home/.cache/omacus-test"
test ! -e "$home/.local/share/omacus"
test ! -e "$home/.config/omacus"
test ! -e "$home/.local/share/applications/omacus.desktop"
test ! -e "$home/.local/share/applications/dev.omacus.OmacusTest.desktop"
cp "$root/dist/linux/omacus-test" "$home/omacus"
if HOME="$home" "$home/omacus"; then
  echo "omacus Test opened under the installed app's name." >&2
  exit 1
fi
rm -rf "$home"
trap - EXIT

tar -C "$root/dist/linux" -czf "$root/dist/omacus-test-linux-x86_64.tar.gz" \
  omacus-test \
  dev.omacus.OmacusTest.desktop \
  README.txt
echo "$root/dist/omacus-test-linux-x86_64.tar.gz"
