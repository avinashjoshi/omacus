#!/usr/bin/env bash
# Bump the package version and rename the changelog's Unreleased section.
# Usage: prepare-release.sh patch|minor|major
#
# For one change, run this on that pull request, then merge it. A push to main
# whose version has no tag yet publishes the release.
#
# The Release workflow's prepare action runs this when several changes already
# on main should ship together. This script only edits files.
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

html_path = pathlib.Path("site/app/index.html")
html = html_path.read_text()
meta = f'name="omacus-version" content="{current}"'
meta_new = f'name="omacus-version" content="{new}"'
label = f">omacus v{current}<"
label_new = f">omacus v{new}<"
if meta not in html or label not in html:
    raise SystemExit("web app version in site/app/index.html does not match Cargo.toml")
html_path.write_text(html.replace(meta, meta_new, 1).replace(label, label_new, 1))

def bump_text(path, old, new, label):
    file = pathlib.Path(path)
    text = file.read_text()
    if old not in text:
        raise SystemExit(f"{label} does not match Cargo.toml")
    file.write_text(text.replace(old, new, 1))

bump_text("site/app/version.js", f'export const APP_VERSION = "{current}"', f'export const APP_VERSION = "{new}"', "version.js")
bump_text("site/app/sw.js", f'const APP_VERSION = "{current}"', f'const APP_VERSION = "{new}"', "sw.js")
bump_text("site/app/version.json", f'"version": "{current}"', f'"version": "{new}"', "version.json")

packages = ["omacus", "omacus-doc", "omacus-engine", "omacus-relay", "omacus-sync-web", "omacus-web"]
for name in packages:
    needle = f'name = "{name}"\nversion = "{current}"\n'
    count = locked.count(needle)
    if count != 1:
        raise SystemExit(f"expected one {name} entry in Cargo.lock, found {count}")
    locked = locked.replace(needle, f'name = "{name}"\nversion = "{new}"\n', 1)
lock_path.write_text(locked)
PY

if ! git describe --tags --abbrev=0 --match 'v*' HEAD >/dev/null 2>&1; then
  echo "No previous v* tag at HEAD; fetch tags before preparing a release." >&2
  exit 1
fi

# ## Unreleased is the notes someone will read. Rename it to this version.
# With no Unreleased section, draft from commit subjects. CI rejects that
# draft on the release pull request until the notes are rewritten for users.
set +e
python3 .github/scripts/release_notes.py promote "$new"
status=$?
set -e
if [[ $status -eq 0 ]]; then
  :
elif [[ $status -eq 2 ]]; then
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
else
  exit "$status"
fi

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "version=$new" >> "$GITHUB_OUTPUT"
fi
echo "$new"
