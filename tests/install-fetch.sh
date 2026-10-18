#!/usr/bin/env bash
# install.sh's download check, against local "releases": a good file installs, a
# tampered one is refused with a message (it used to fail silently on macOS), and a
# release without the per-chip .dmg falls back to the universal one.
set -uo pipefail
cd "$(dirname "$0")/.."
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sum() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
release() { # release <dir> <file> <content> [tampered content]
  mkdir -p "$work/$1"
  echo "$3" >"$work/$1/$2"
  (cd "$work/$1" && sum "$2" >SHA256SUMS.txt)
  if [[ -n "${4:-}" ]]; then echo "$4" >"$work/$1/$2"; fi
}
release good omacus-macos-arm64.dmg app
release bad omacus-macos-arm64.dmg app swapped
release old omacus-macos-universal.dmg universal
try() { # install_macos's own download lines, run against a local release
  bash -c '
    set -euo pipefail
    eval "$(sed -n "/^fetch() {/,/^}/p" install.sh)"
    calls="$(sed -n "/fetch \"\$dl\" \"omacus-macos-\$arch.dmg\"/,/universal.dmg/p" install.sh)"
    [[ -n "$calls" ]] || { echo "download lines not found in install.sh"; exit 1; }
    CURL=(curl -fsSL)
    dl="file://$1" arch=arm64 TMP_DIR="$2"
    eval "$calls"
    cat "$2/omacus.dmg"' _ "$work/$1" "$(mktemp -d "$work/tmp.XXXX")" 2>&1
}
fail=0
check() { if [[ "$2" == *"$3"* ]]; then echo "ok: $1"; else echo "FAIL: $1: $2"; fail=1; fi; }
check "good download installs" "$(try good)" app
check "tampered download is refused, and says so" "$(try bad)" "didn't match the release's checksum"
check "older release falls back to the universal .dmg" "$(try old)" universal
exit $fail
