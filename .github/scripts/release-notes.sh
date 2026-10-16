#!/usr/bin/env bash
# Write release notes for a version to stdout: what changed since the previous
# release (from commit subjects), then how to install it on each platform.
#
#   .github/scripts/release-notes.sh 0.2.0 owner/repo
set -euo pipefail

version="$1"
repo="$2"
tag="v$version"
dl="https://github.com/$repo/releases/download/$tag"

previous="$(git describe --tags --abbrev=0 --match 'v*' HEAD 2>/dev/null || true)"
range="${previous:+$previous..}HEAD"

echo "## What's new"
echo
# One bullet per commit subject, with any "- " bullets from the commit body nested
# under it. Version bumps, chores, trailers and website-only commits are left out.
notes="$(
  git log --no-merges --pretty='format:%s%n%b%x1e' "$range" -- . ':!site' ':!.github/workflows/site.yml' ':!.github/workflows/freshen.yml' |
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
echo
echo

cat <<EOF
## Install

| Platform | Download |
|---|---|
| **macOS** (Apple Silicon & Intel) | [omacus-macos-universal.dmg]($dl/omacus-macos-universal.dmg) |
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
  echo "**Full changelog:** https://github.com/$repo/compare/$previous...$tag"
fi
