#!/usr/bin/env bash
# Write release notes for a version to stdout: the changelog section written
# for users, then how to install on each platform. With no changelog section,
# fall back to commit subjects (a release pull request's CI rejects that).
#
#   .github/scripts/release-notes.sh 0.2.0 owner/repo
set -euo pipefail

version="$1"
repo="$2"
tag="v$version"
dl="https://github.com/$repo/releases/download/$tag"

previous="$(git describe --tags --abbrev=0 --match 'v*' HEAD 2>/dev/null || true)"

echo "## What's new"
echo
set +e
written="$(python3 .github/scripts/release_notes.py extract "$version")"
status=$?
set -e
if [[ $status -eq 0 ]]; then
  printf '%s\n' "$written"
elif [[ $status -eq 2 ]]; then
  # One bullet per commit subject, with any "- " bullets from the commit body nested
  # under it. Version bumps, chores, trailers and website-only commits are left out.
  range="${previous:+$previous..}HEAD"
  notes="$(
    git log --no-merges --pretty='format:%s%n%b%x1e' "$range" -- . ':!site' ':!.github/workflows/site.yml' ':!.github/workflows/freshen.yml' ':!video' |
      awk -v RS='\x1e' '
        {
          n = split($0, lines, "\n"); subject = ""
          for (i = 1; i <= n; i++) {
            line = lines[i]
            if (subject == "") {
              if (line == "") continue
              subject = line
              if (tolower(subject) ~ /^(release|bump|chore)([:( ]|$)/ || subject ~ /^v?[0-9]+\.[0-9]+\.[0-9]+$/) next
              print "- " subject
            } else if (line ~ /^- /) {
              print "  " line
            } else if (line ~ /^  [^ ]/ && last ~ /^  - /) {
              print "  " line   # wrapped continuation of a body bullet
              continue
            }
            last = (line ~ /^- / ? "  " line : "")
          }
        }'
  )"
  echo "${notes:-- Maintenance release}"
else
  exit "$status"
fi
echo
echo

cat <<EOF
## Install

| Platform | Download |
|---|---|
| **macOS** (Apple Silicon) | [omacus-macos-arm64.dmg]($dl/omacus-macos-arm64.dmg) |
| **macOS** (Intel) | [omacus-macos-x86_64.dmg]($dl/omacus-macos-x86_64.dmg) |
| **Windows** (x64) | [omacus-windows-x86_64.zip]($dl/omacus-windows-x86_64.zip) |
| **Ubuntu / Debian** | [omacus-linux-x86_64.deb]($dl/omacus-linux-x86_64.deb) |
| **Fedora / openSUSE** | [omacus-linux-x86_64.rpm]($dl/omacus-linux-x86_64.rpm) |
| **Omarchy / Arch / any Linux** | [omacus-linux-x86_64.tar.gz]($dl/omacus-linux-x86_64.tar.gz) or the installer below |

**Omarchy** (installs the app, then offers a keybind, \`SUPER + CTRL + ALT + =\` by default, and a floating window rule):

\`\`\`sh
curl -fsSL https://omacus.com/install | bash
\`\`\`

**Any other Linux** (installs to \`~/.local/bin\` with a launcher entry):

\`\`\`sh
curl -fsSL https://omacus.com/install | bash
\`\`\`

**Ubuntu / Debian:** \`sudo apt install ./omacus-linux-x86_64.deb\`
**Fedora:** \`sudo dnf install ./omacus-linux-x86_64.rpm\`

**macOS:** open the \`.dmg\` and drag Omacus to Applications. It isn't notarized yet, so the first
time right-click the app and choose **Open**, or run \`xattr -dr com.apple.quarantine /Applications/Omacus.app\`.

**Windows:** unzip and run \`omacus.exe\`. If SmartScreen warns about an unknown publisher, choose
**More info → Run anyway**. It works from a terminal too: \`omacus.exe "\$100 in INR"\`.

Checksums are in \`SHA256SUMS.txt\`.
EOF

if [[ -n "$previous" ]]; then
  echo
  echo "**Details:** https://github.com/$repo/compare/$previous...$tag"
fi
