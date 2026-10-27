#!/bin/bash
# dmgbuild and hdiutil can wait forever on a stuck disk image.
# Usage: dmg-retry.sh SECONDS command...
# The command is tried twice. Between tries, a mounted Omacus volume is
# detached so the next hdiutil does not wait on the same image.
set -u

deadline() {
  local seconds="$1"
  shift
  "$@" &
  local pid=$!
  local i=0
  while kill -0 "$pid" 2>/dev/null; do
    if [[ "$i" -ge "$seconds" ]]; then
      kill "$pid" 2>/dev/null || true
      sleep 1
      kill -9 "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
      return 124
    fi
    sleep 1
    i=$((i + 1))
  done
  wait "$pid"
}

detach_stuck() {
  if [[ -d /Volumes/Omacus ]]; then
    hdiutil detach -force /Volumes/Omacus >/dev/null 2>&1 || true
  fi
}

seconds="${1:-}"
if [[ -z "$seconds" ]]; then
  echo "usage: dmg-retry.sh SECONDS command..." >&2
  exit 2
fi
shift

if [[ "$#" -eq 0 ]]; then
  echo "usage: dmg-retry.sh SECONDS command..." >&2
  exit 2
fi

try=1
while [[ "$try" -le 2 ]]; do
  if deadline "$seconds" "$@"; then
    exit 0
  fi
  echo "disk image command did not finish (try $try)" >&2
  detach_stuck
  try=$((try + 1))
done
exit 1
