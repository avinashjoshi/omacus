#!/usr/bin/env bash
# Bump the package version and prepend a changelog section.
# Usage: prepare-release.sh patch|minor|major
#
# The Release workflow commits the result on a release branch and opens a pull
# request. This script only edits files.
set -euo pipefail

bump="${1:-patch}"
cd "$(dirname "$0")/../.."

current="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if [[ ! "$current" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Cargo.toml version is not major.minor.patch: $current" >&2
  exit 1
fi

IFS=. read -r major minor patch <<<"$current"
case "$bump" in
  major) major=$((major + 1)); minor=0; patch=0 ;;
  minor) minor=$((minor + 1)); patch=0 ;;
  patch) patch=$((patch + 1)) ;;
  *)
    echo "bump must be patch, minor, or major" >&2
    exit 1
    ;;
esac
new="$major.$minor.$patch"

if git rev-parse -q --verify "refs/tags/v$new" >/dev/null; then
  echo "v$new already exists" >&2
  exit 1
fi
if git ls-remote --exit-code --tags origin "refs/tags/v$new" >/dev/null 2>&1; then
  echo "v$new already exists on origin" >&2
  exit 1
fi

python3 - "$current" "$new" <<'PY'
import pathlib, sys
current, new = sys.argv[1], sys.argv[2]
toml_path = pathlib.Path("Cargo.toml")
toml = toml_path.read_text()
old = f'version = "{current}"'
idx = toml.find(old)
if idx < 0:
    raise SystemExit("package version not found in Cargo.toml")
toml_path.write_text(toml[:idx] + f'version = "{new}"' + toml[idx + len(old):])

lock_path = pathlib.Path("Cargo.lock")
locked = lock_path.read_text()
needle = f'name = "omacus"\nversion = "{current}"\n'
repl = f'name = "omacus"\nversion = "{new}"\n'
count = locked.count(needle)
if count != 1:
    raise SystemExit(f"expected one omacus entry in Cargo.lock, found {count}")
lock_path.write_text(locked.replace(needle, repl, 1))
PY

if ! git describe --tags --abbrev=0 --match 'v*' HEAD >/dev/null 2>&1; then
  echo "No previous v* tag at HEAD; fetch tags before preparing a release." >&2
  exit 1
fi

repo="${GITHUB_REPOSITORY:-avinashjoshi/omacus}"
notes="$(.github/scripts/release-notes.sh "$new" "$repo")"
section="$(
  printf '%s\n' "$notes" | awk -v version="$new" '
    /^## Install/ { exit }
    NR == 1 { print "## " version; next }
    { print }
  '
)"
# Drop the trailing blank lines awk kept before the install heading.
section="$(printf '%s\n' "$section" | sed -e :a -e '/^[[:space:]]*$/{$d;N;ba' -e '}')"

if [[ -f CHANGELOG.md ]]; then
  {
    printf '# Changelog\n\n%s\n\n' "$section"
    awk 'NR>1 { print }' CHANGELOG.md | sed '1{/^$/d;}'
  } > CHANGELOG.md.tmp
  mv CHANGELOG.md.tmp CHANGELOG.md
else
  printf '# Changelog\n\n%s\n' "$section" > CHANGELOG.md
fi

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "version=$new" >> "$GITHUB_OUTPUT"
fi
echo "$new"
