#!/usr/bin/env python3
"""User-facing release notes.

The changelog is the source for a release. `## Unreleased` holds the notes
someone will read. Preparing a release renames that heading to the version.
Publishing copies the section into the GitHub release, above the install links.

A section is a short title in bold, then one or two sentences. Optional
bullets follow. `<!-- minor -->` folds a small fix under the newer entry on
the updates page. `<!-- internal -->` does the same for a release with nothing
new in the app.

    python3 .github/scripts/release_notes.py promote 0.2.12
    python3 .github/scripts/release_notes.py extract 0.2.12
"""

import re
import sys
from pathlib import Path

TITLE = re.compile(r"^\*\*(.+)\*\*$")
UNRELEASED = re.compile(r"^## Unreleased[ \t]*$", re.M)
COMMENT = re.compile(r"<!--.*?-->", re.S)

# Best effort. A real sentence can still mention a tool by name; this catches
# the notes that are commit subjects, pull requests, or release plumbing.
JARGON = (
    (re.compile(r"\(#\d+\)"), "a pull request number"),
    (re.compile(r"(?<![/\w])#\d+\b"), "an issue number"),
    (re.compile(r"prepare job", re.I), "release plumbing"),
    (re.compile(r"\bworkflows?\b", re.I), "release plumbing"),
    (re.compile(r"\bclippy\b", re.I), "tooling jargon"),
    (re.compile(r"\bwasm(?:32)?\b", re.I), "tooling jargon"),
    (re.compile(r"\bCI\b"), "release plumbing"),
    (re.compile(r"\bpull requests?\b", re.I), "a pull request"),
    (re.compile(r"github actions", re.I), "release plumbing"),
    (re.compile(r"\bcargo\b", re.I), "tooling jargon"),
    (re.compile(r"\bfend\b", re.I), "engine jargon"),
    (re.compile(r"\bstdio\b", re.I), "tooling jargon"),
    (re.compile(r"Co-authored-by|Signed-off-by"), "a commit trailer"),
)


def section(text, heading):
    """Body under a `## heading`, or None when that heading is absent."""
    match = re.search(rf"^## {re.escape(heading)}[ \t]*$", text, re.M)
    if not match:
        return None
    rest = text[match.end():]
    nxt = re.search(r"^## ", rest, re.M)
    body = (rest[:nxt.start()] if nxt else rest).strip()
    return body


def promote(text, version):
    """Rename `## Unreleased` to `## version`. None when there is no such heading.

    An empty Unreleased section is an error: someone meant to write the notes.
    """
    match = UNRELEASED.search(text)
    if not match:
        return None
    if not section(text, "Unreleased"):
        raise ValueError("CHANGELOG.md has an empty Unreleased section. Write the notes before preparing a release.")
    return text[:match.start()] + f"## {version}" + text[match.end():]


def _lines(text):
    return [line.strip() for line in text.splitlines() if line.strip()]


def _sentence(line):
    if line.startswith("- ") or line.startswith("**") or line.lower().startswith("details:"):
        return False
    words = re.findall(r"[A-Za-z0-9']+", line)
    return len(words) >= 6 or line.endswith((".", "!", "?"))


def problems(body):
    """Why this section is not ready for a release, or an empty list."""
    text = COMMENT.sub("", body or "").strip()
    found = []
    for pattern, why in JARGON:
        if pattern.search(text):
            found.append(why)
    lines = _lines(text)
    title = ""
    if lines and (match := TITLE.match(lines[0])):
        title = match.group(1).strip()
    else:
        found.append("a short title in bold, on the first line")
    if title and not 8 <= len(title) <= 80:
        found.append("a title of one short line")
    sentences = sum(1 for line in lines[1:] if _sentence(line))
    bullets = sum(1 for line in lines if line.startswith("- "))
    if sentences < 1:
        found.append("a sentence about what changed for the reader")
    if bullets and sentences < 1 and not title:
        found.append("notes that read like commit titles")
    return list(dict.fromkeys(found))


def _path(argv):
    return Path(argv[3]) if len(argv) > 3 else Path("CHANGELOG.md")


def main(argv):
    if len(argv) < 3 or argv[1] not in ("promote", "extract"):
        print("usage: release_notes.py promote|extract VERSION [CHANGELOG]", file=sys.stderr)
        return 2
    path = _path(argv)
    text = path.read_text()
    version = argv[2]
    if argv[1] == "extract":
        body = section(text, version)
        if not body:
            return 2
        print(body)
        return 0
    try:
        updated = promote(text, version)
    except ValueError as exc:
        print(exc, file=sys.stderr)
        return 1
    if updated is None:
        return 2
    path.write_text(updated)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
