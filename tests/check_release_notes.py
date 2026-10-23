#!/usr/bin/env python3
"""Release notes have to be written for someone using Omacus.

On a release pull request (a branch named release/*) the changelog section
that will ship is checked too. Commit subjects and pull-request jargon fail.
"""

import os
import re
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / ".github" / "scripts"))
import release_notes  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
GOOD = """\
**Clearer release notes**

What's new is written for you: a short title, then a sentence about what changed.

- The updates page uses the same titles
"""
OLD = """\
- Publish a release when the push skips the prepare job. (#50)
- Add a design language for the app and the website (#47)
- Cut releases through a pull request (#48)
"""
BEHIND = """\
**Behind-the-scenes improvements to how updates ship**

Nothing you see in the app changed.

<!-- internal -->
"""


class ReleaseNotes(unittest.TestCase):
    def test_a_user_facing_section_passes(self):
        self.assertEqual(release_notes.problems(GOOD), [])

    def test_commit_titles_fail(self):
        found = release_notes.problems(OLD)
        self.assertTrue(found)
        self.assertTrue(any("pull request" in item or "title" in item for item in found))

    def test_plumbing_jargon_fails_even_with_a_title(self):
        bad = (
            "**Publish a release when the push skips the prepare job**\n\n"
            "The workflow publishes when CI skips that job.\n"
        )
        found = release_notes.problems(bad)
        self.assertTrue(any("plumbing" in item for item in found))

    def test_a_note_about_the_notes_fails(self):
        bad = (
            "**The web app works offline**\n\n"
            "The web app keeps working with no connection. "
            "What's new is a short title and a sentence or two, on the updates page and in the app.\n"
        )
        found = release_notes.problems(bad)
        self.assertTrue(any("note about the notes" in item for item in found))
        self.assertEqual(release_notes.problems(GOOD), [])

    def test_the_release_title_is_the_headline(self):
        self.assertEqual(release_notes.headline(GOOD), "Clearer release notes")
        self.assertIsNone(release_notes.GENERIC_TITLE.match(release_notes.headline(GOOD)))
        self.assertIsNotNone(release_notes.GENERIC_TITLE.match("Omacus v0.2.13"))
        titled = "**Omacus v0.2.13**\n\nA sentence about the app that someone can read.\n"
        self.assertIn("just the version", " ".join(release_notes.problems(titled)))
        self.assertEqual(release_notes.headline("Omacus v0.2.13\n\nA sentence about the app.\n"), "")

    def test_a_behind_the_scenes_note_passes(self):
        self.assertEqual(release_notes.problems(BEHIND), [])

    def test_promote_renames_unreleased_and_extract_reads_it_back(self):
        text = "# Changelog\n\n## Unreleased\n\n" + GOOD + "\n## 0.2.11\n\n**See every update in one place**\n\nThe updates page lists each version.\n"
        promoted = release_notes.promote(text, "0.2.12")
        self.assertNotIn("## Unreleased", promoted)
        self.assertIn("## 0.2.12\n", promoted)
        self.assertIn("## 0.2.11\n", promoted)
        self.assertEqual(release_notes.section(promoted, "0.2.12"), GOOD.strip())
        self.assertIsNone(release_notes.promote(promoted, "0.2.13"))

    def test_an_empty_unreleased_section_is_an_error(self):
        with self.assertRaises(ValueError):
            release_notes.promote("# Changelog\n\n## Unreleased\n\n## 0.2.11\n\n**Title**\n\nA sentence about the change.\n", "0.2.12")

    def test_checked_in_changelog_sections_pass(self):
        text = (ROOT / "CHANGELOG.md").read_text()
        for heading in ("Unreleased",):
            body = release_notes.section(text, heading)
            if body:
                self.assertEqual(release_notes.problems(body), [], heading)
        # Every version already in the file is held to the same voice.
        for match in re.finditer(r"^## (\d+\.\d+\.\d+)[ \t]*$", text, re.M):
            body = release_notes.section(text, match.group(1))
            self.assertEqual(release_notes.problems(body), [], match.group(1))

    def test_a_release_branch_rejects_bad_notes(self):
        branch = os.environ.get("GITHUB_HEAD_REF", "")
        if not branch.startswith("release/") and os.environ.get("OMACUS_CHECK_CHANGELOG") != "1":
            self.skipTest("not a release pull request")
        text = (ROOT / "CHANGELOG.md").read_text()
        version = branch.removeprefix("release/v") if branch.startswith("release/") else ""
        body = release_notes.section(text, version) if version else None
        if body is None:
            # The newest version section is what the pull request will publish.
            match = re.search(r"^## (\d+\.\d+\.\d+)[ \t]*$", text, re.M)
            self.assertIsNotNone(match, "a release pull request needs a version section")
            body = release_notes.section(text, match.group(1))
        found = release_notes.problems(body)
        self.assertEqual(found, [], "release notes: " + "; ".join(found))


if __name__ == "__main__":
    unittest.main()
