#!/usr/bin/env python3
"""The story page: counts, milestones, and the private-text backstop."""

import os
import subprocess
import tempfile
import unittest
from datetime import date
from pathlib import Path

import story

# A stand-in phrase. The real list lives outside the repo.
TERM = "internal-codename"


def release(tag, notes, day="2026-10-16T12:00:00Z"):
    return {
        "tag_name": tag,
        "name": tag,
        "body": f"## What's new\n\n{notes}\n",
        "published_at": day,
        "draft": False,
        "prerelease": False,
    }


def sample_releases():
    return [
        release("v0.1.0", "**A notepad for math**\n\nOmacus keeps a whole page.", "2026-10-16T12:00:00Z"),
        release(
            "v0.1.10",
            "**Share a page with a link**\n\nThe web app keeps pages in this browser, and can open when you are offline.",
            "2026-10-17T12:00:00Z",
        ),
        release("v0.2.11", "**See every update in one place**\n\nThe updates page lists each version.", "2026-10-23T12:00:00Z"),
        release(
            "v0.2.12",
            "**The web app works offline**\n\nThe web app keeps working with no connection.",
            "2026-10-23T18:00:00Z",
        ),
        release("v9.9.9", "- Secret internal-codename stays in the notes\n- The web app is mentioned again", "2026-10-24T12:00:00Z"),
    ]


class terms:
    def __init__(self, value):
        self.value = value

    def __enter__(self):
        self.saved = os.environ.get("OMACUS_PRIVATE_TERMS")
        if self.value is None:
            os.environ.pop("OMACUS_PRIVATE_TERMS", None)
        else:
            os.environ["OMACUS_PRIVATE_TERMS"] = self.value

    def __exit__(self, *_exc):
        if self.saved is None:
            os.environ.pop("OMACUS_PRIVATE_TERMS", None)
        else:
            os.environ["OMACUS_PRIVATE_TERMS"] = self.saved


class StoryPage(unittest.TestCase):
    def setUp(self):
        self._saved_terms = os.environ.pop("OMACUS_PRIVATE_TERMS", None)

    def tearDown(self):
        if self._saved_terms is None:
            os.environ.pop("OMACUS_PRIVATE_TERMS", None)
        else:
            os.environ["OMACUS_PRIVATE_TERMS"] = self._saved_terms

    def test_milestones_follow_release_history_and_skip_private_text(self):
        tags = {
            "v0.1.0": "2026-10-16",
            "v0.1.10": "2026-10-17",
            "v0.2.11": "2026-10-23",
            # v0.2.12 and v9.9.9 take their dates from the release, not a guess.
        }
        found = story.select_milestones(sample_releases(), tags)
        keyed = {item["id"]: item for item in found}
        self.assertEqual(keyed["first"]["tag"], "v0.1.0")
        self.assertEqual(keyed["first"]["day"], "2026-10-16")
        self.assertEqual(keyed["web"]["tag"], "v0.1.10")
        self.assertEqual(keyed["web"]["day"], "2026-10-17")
        self.assertEqual(keyed["updates"]["tag"], "v0.2.11")
        self.assertEqual(keyed["offline"]["tag"], "v0.2.12")
        self.assertEqual(keyed["offline"]["day"], "2026-10-23")
        self.assertTrue(keyed["sync"]["soon"])
        self.assertEqual(keyed["sync"]["day"], "")
        self.assertEqual([item["id"] for item in found], ["first", "web", "updates", "offline", "sync"])

        page, _out = self.write(sample_releases(), tags, {"days": 7, "releases": 4, "pulls": 2, "rust": 1200})
        self.assertIn('href="/updates/#v0.1.10"', page)
        self.assertIn('datetime="2026-10-23"', page)
        self.assertIn(">Soon<", page)
        self.assertNotIn("internal-codename", page.lower())
        self.assertNotIn("v9.9.9", page)
        sync = page.split('id="sync"', 1)[1].split("</li>", 1)[0]
        self.assertNotIn("<time", sync)
        self.assertIn("1,200", page)
        self.assertIn("Days since the first commit", page)
        self.assertIn("Pull requests merged", page)
        self.assertIn("from the first version", page)
        self.assertIn("on his Mac", page)

    def test_a_missing_figure_is_left_out(self):
        page, _out = self.write(sample_releases(), {"v0.1.0": "2026-10-16"}, {})
        self.assertNotIn('id="numbers"', page)
        self.assertNotIn("Days since", page)
        self.assertNotIn("Lines of Rust", page)
        self.assertIn('id="why"', page)
        self.assertIn('href="#timeline"', page)
        self.assertNotIn('href="#numbers"', page)

    def test_one_of_a_count_uses_the_singular(self):
        page, _out = self.write(
            [release("v0.1.0", "**A notepad for math**\n\nHello.")],
            {"v0.1.0": "2026-10-16"},
            {"days": 1, "releases": 1, "pulls": 1, "rust": 1},
        )
        self.assertIn("Day since the first commit", page)
        self.assertIn(">Release<", page)
        self.assertIn("Pull request merged", page)
        self.assertIn("Line of Rust", page)

    def test_git_date_wins_over_the_published_date(self):
        found = story.select_milestones(
            [release("v0.1.0", "**A notepad for math**\n\nHello.", "2026-11-02T00:00:00Z")],
            {"v0.1.0": "2026-10-16"},
        )
        self.assertEqual(found[0]["day"], "2026-10-16")

    def test_a_release_with_no_date_is_not_invented(self):
        raw = release("v0.3.0", "**The web app**\n\nThe web app is here.")
        raw["published_at"] = ""
        found = story.select_milestones([raw], {})
        self.assertEqual([item["id"] for item in found], ["sync"])

    def test_days_between_and_merged_pulls(self):
        self.assertEqual(story.days_between("2026-10-16", "2026-10-23"), 7)
        self.assertEqual(story.days_between("2026-10-23", "2026-10-23"), 0)
        self.assertIsNone(story.days_between("2026-10-24", "2026-10-23"))
        self.assertIsNone(story.days_between("nope", "2026-10-23"))
        self.assertEqual(story.count_merged([
            {"merged_at": "2026-10-16T00:00:00Z"},
            {"merged_at": None},
            {"merged_at": "2026-10-17T00:00:00Z"},
        ]), 2)
        self.assertIsNone(story.count_merged([{"merged_at": "x"}, "nope"]))
        self.assertIsNone(story.count_merged({"total_count": 5}))

    def test_rust_lines_follow_newline_counts(self):
        root = Path(tempfile.mkdtemp())
        (root / "src").mkdir()
        (root / "src" / "lib.rs").write_text("fn main() {}\nlet _ = 1;\n")
        (root / "src" / "skip.txt").write_text("not rust\n")
        self.assertEqual(story.rust_lines_in([root / "src" / "lib.rs"]), 2)

    def test_required_story_and_no_private_probes(self):
        page, _out = self.write(sample_releases(), {"v0.1.0": "2026-10-16", "v0.1.10": "2026-10-17",
                                                    "v0.2.11": "2026-10-23", "v0.2.12": "2026-10-23"},
                                {"days": 7})
        for phrase in ("Ajith", "Numi", "on a Mac", "Omarchy", "every day", "macOS", "Windows",
                       "MIT", "Claude", "Grok Bot", "docs/DESIGN.md",
                       "oh-MAH-kus", "CONTRIBUTING.md", "taste, decisions, and review"):
            self.assertIn(phrase, page)
        self.assertIn('aria-current="page"', page)
        self.assertIn('>Story</a>', page)

    def test_a_configured_term_stops_the_page_without_writing(self):
        rel = release("v9.9.9", f"- Secret {TERM} stays in the notes\n- The web app is mentioned again")
        with terms(TERM):
            self.assertEqual(story.notes_text(rel), "")
            plain = release("v0.1.10", "The web app keeps pages in this browser.")
            self.assertIn("web app", story.notes_text(plain))
            out = Path(tempfile.mkdtemp())
            with self.assertRaises(SystemExit) as caught:
                story.build([rel, plain], out, {}, {"v0.1.10": "2026-10-17"})
            message = str(caught.exception)
            self.assertIn("v9.9.9", message)
            self.assertNotIn("v0.1.10", message)
            self.assertNotIn(TERM, message.casefold())
            self.assertFalse((out / "story" / "index.html").exists())

    def test_first_commit_day_is_the_root_not_the_latest(self):
        root = Path(tempfile.mkdtemp())
        env = os.environ.copy()
        env["GIT_AUTHOR_NAME"] = "Ada Example"
        env["GIT_AUTHOR_EMAIL"] = "ada@example.com"
        env["GIT_COMMITTER_NAME"] = "Ada Example"
        env["GIT_COMMITTER_EMAIL"] = "ada@example.com"

        def run(*args, when):
            env["GIT_AUTHOR_DATE"] = when
            env["GIT_COMMITTER_DATE"] = when
            subprocess.run(["git", "-C", str(root), *args], check=True, env=env,
                           capture_output=True, text=True)

        subprocess.run(["git", "init", "-q", "-b", "main", str(root)], check=True, capture_output=True)
        subprocess.run(["git", "-C", str(root), "config", "commit.gpgsign", "false"], check=True, capture_output=True)
        (root / "note.txt").write_text("first\n")
        run("add", "note.txt", when="2026-01-01T00:00:00Z")
        run("commit", "-q", "-m", "first", when="2026-01-01T00:00:00Z")
        (root / "note.txt").write_text("second\n")
        run("add", "note.txt", when="2026-01-05T00:00:00Z")
        run("commit", "-q", "-m", "second", when="2026-01-05T00:00:00Z")
        self.assertEqual(story.first_commit_day(root), "2026-01-01")
        self.assertFalse(story.shallow(root))

    def test_today_is_not_baked_into_the_day_count(self):
        self.assertEqual(story.days_between("2026-01-01", date(2026, 1, 3).isoformat()), 2)

    def write(self, releases, tags, facts):
        out = Path(tempfile.mkdtemp())
        story.build(releases, out, facts, tags)
        return (out / "story" / "index.html").read_text(), out


if __name__ == "__main__":
    unittest.main()
