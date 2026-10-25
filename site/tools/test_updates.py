#!/usr/bin/env python3
"""The updates page: grouping and the feed."""

import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

import updates

ATOM = "{http://www.w3.org/2005/Atom}"
PNG = (
    b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01"
    b"\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\x0cIDATx\x9cc\xf8\xcf\xc0"
    b"\x00\x00\x00\x03\x00\x01\x00\x05\xfe\xd4\xef\x00\x00\x00\x00IEND\xaeB`\x82"
)


def release(tag, notes, day="2026-10-20T12:00:00Z", assets=None):
    body = f"## What's new\n\n{notes}\n\n## Install\n\nINSTALL_BOILERPLATE_MARKER\n"
    return {
        "tag_name": tag,
        "name": f"Omacus {tag}",
        "body": body,
        "published_at": day,
        "draft": False,
        "prerelease": False,
        "html_url": f"https://github.com/avinashjoshi/omacus/releases/tag/{tag}",
        "assets": assets or [],
    }


def build(releases, fetch=None):
    out = Path(tempfile.mkdtemp())
    updates.build(releases, out, fetch=fetch)
    page = (out / "updates" / "index.html").read_text()
    feed = (out / "updates" / "feed.xml").read_text()
    return page, feed, out


class UpdatesPage(unittest.TestCase):
    def test_smaller_fixes_fold_under_the_newer_entry(self):
        page, feed, _ = build([
            release("v0.3.2", "- Fix the cursor when a line wraps", "2026-10-23T03:00:00Z"),
            release("v0.3.1", "- Keep the sidebar from jumping", "2026-10-23T02:00:00Z"),
            release("v0.3.0", "- Add folders to the sidebar\n  - Drag a page into a folder\n- Show a value on hover", "2026-10-22T12:00:00Z"),
            release("v0.2.0", "- Omacus for Linux, macOS and Windows\n- Save pages as you type\n- Follow the system theme", "2026-10-16T12:00:00Z"),
        ])
        # The newest fix is the timeline entry; the older fix is inside its group.
        head = page.split('<ol class="timeline">', 1)[1]
        self.assertLess(head.index('id="v0.3.2"'), head.index('id="v0.3.1"'))
        self.assertLess(head.index('id="v0.3.1"'), head.index('id="v0.3.0"'))
        self.assertIn("1 smaller update", page)
        self.assertIn('class="fix-range">v0.3.1<', page)
        self.assertIn("Fix the cursor when a line wraps", page)
        self.assertNotIn("INSTALL_BOILERPLATE_MARKER", page)
        self.assertNotIn("INSTALL_BOILERPLATE_MARKER", feed)
        # A .0 release is its own entry, not a folded fix. Its id is on an entry.
        self.assertIn('<li class="entry" id="v0.3.0">', page)
        self.assertIn('<li class="entry" id="v0.2.0">', page)
        root = ET.fromstring(feed)
        self.assertEqual(root.tag, f"{ATOM}feed")
        entries = root.findall(f"{ATOM}entry")
        self.assertEqual(len(entries), 4)
        self.assertTrue(entries[0].find(f"{ATOM}link").attrib["href"].endswith("#v0.3.2"))

    def test_no_inline_script_and_images_stay_on_the_site(self):
        def fetch(url):
            self.assertTrue(url.startswith("https://"))
            return PNG, "image/png"

        page, feed, out = build([
            release(
                "v1.2.0",
                "- Add a picture of the new sidebar\n\n![The sidebar](https://example.com/side.png)",
                assets=[{"name": "extra.png", "browser_download_url": "https://example.com/extra.png"}],
            ),
        ], fetch=fetch)
        self.assertNotIn("<script", page.replace('<script src="/js/site.js"></script>', ""))
        self.assertNotIn("onclick=", page)
        self.assertNotIn("javascript:", page)
        self.assertNotIn("api.github.com", page)
        self.assertNotIn("example.com", page)
        self.assertNotIn("example.com", feed)
        self.assertIn("/updates/media/", page)
        media = list((out / "updates" / "media").iterdir())
        self.assertEqual(len(media), 2)
        self.assertTrue(all(p.suffix == ".png" for p in media))

    def test_a_fix_with_an_explanation_still_folds(self):
        page, _, _ = build([
            release("v0.4.2", "- Add a sidebar\n  - Drag a page into a folder", "2026-10-23T04:00:00Z"),
            release("v0.4.1", "- Fix the cursor when a line wraps\n  - The caret jumped to the next line", "2026-10-23T03:00:00Z"),
        ])
        self.assertIn('<li class="entry" id="v0.4.2">', page)
        self.assertNotIn('<li class="entry" id="v0.4.1">', page)
        self.assertIn("1 smaller update", page)
        self.assertIn("The caret jumped to the next line", page)

    def test_ordinary_money_and_mrr_stay_on_the_page(self):
        page, feed, _ = build([
            release("v1.0.1", "- $4 a day is $28 a week\n- MRR and dilution, in the founders guide"),
        ])
        self.assertIn("$4 a day", page)
        self.assertIn("MRR and dilution", page)
        self.assertIn("$4 a day", feed)

    def test_updates_rules_use_the_site_tokens(self):
        """The page takes its colors from tokens.css. site.css has no hex of its own."""
        css_path = Path(__file__).parents[1].joinpath("css", "site.css")
        tokens_path = Path(__file__).parents[1].joinpath("css", "tokens.css")
        section = css_path.read_text().split("/* ---------- Updates", 1)[1]
        self.assertNotRegex(section, r"#[0-9a-fA-F]{3,8}\b")
        tokens = tokens_path.read_text()
        light = tokens.split('html[data-page="updates"]', 1)[1]
        self.assertIn("--bg:", light)
        self.assertIn("--accent:", light)
        self.assertIn("--prose-gap: 12px;", tokens)
        prose = css_path.read_text().split(".prose p,", 1)[1].split("}", 1)[0]
        self.assertIn("var(--prose-gap)", prose)

    def test_a_version_counts_once_it_is_listed(self):
        listed = [{"tag_name": "v0.2.11"}]
        self.assertTrue(updates.release_is_listed(listed, "0.2.11"))
        self.assertTrue(updates.release_is_listed(listed, "v0.2.11"))
        self.assertFalse(updates.release_is_listed(listed, "0.2.12"))
        updates.os.environ.pop("OMACUS_EXPECT_WAIT", None)
        self.assertEqual(updates.expect_wait_seconds(""), 0)
        self.assertEqual(updates.expect_wait_seconds("0.2.11"), 60)

    def test_the_page_links_itself_in_the_nav(self):
        page, _, _ = build([release("v0.1.0", "- Omacus\n- Pages\n- Themes")])
        self.assertIn('href="/updates/" aria-current="page"', page)
        self.assertIn('href="/updates/feed.xml"', page)

    def test_a_title_is_the_headline_and_the_sentence_follows(self):
        notes = (
            "**Lock pages and folders with a password**\n\n"
            "A page stays locked until you type the password.\n\n"
            "- Lock one page, or a whole folder\n\n"
            "**Details:** https://github.com/avinashjoshi/omacus/compare/v1.3.0...v1.4.0"
        )
        page, feed, _ = build([release("v1.4.0", notes)])
        self.assertIn("<h2>Lock pages and folders with a password</h2>", page)
        self.assertIn("A page stays locked until you type the password.", page)
        self.assertIn("Lock one page, or a whole folder", page)
        self.assertNotIn("<h2>Lock one page", page)
        self.assertEqual(page.count(">Details</a>"), 1)
        self.assertNotIn("Full changes", page)
        self.assertNotIn("**Details:**", page)
        self.assertIn("v1.4.0 — Lock pages and folders with a password", feed)

    def test_a_titled_note_stays_on_the_timeline(self):
        page, _, _ = build([
            release("v1.8.2", "**A cleaner, calmer look**\n\nThe app and the website share one quiet look.", "2026-10-23T04:00:00Z"),
            release("v1.8.1", "**See every update in one place**\n\nThe updates page lists each version in plain words.", "2026-10-23T03:00:00Z"),
        ])
        self.assertIn('<li class="entry" id="v1.8.2">', page)
        self.assertIn('<li class="entry" id="v1.8.1">', page)
        self.assertIn("<h2>A cleaner, calmer look</h2>", page)
        self.assertIn("The app and the website share one quiet look.", page)

    def test_a_minor_marker_folds_under_the_newer_entry(self):
        page, _, _ = build([
            release(
                "v1.6.2",
                "**Rename a name everywhere it appears**\n\nEvery line that uses it updates.",
                "2026-10-23T04:00:00Z",
            ),
            release(
                "v1.6.1",
                "**Answers update as you type on a phone**\n\nA new line shows its answer immediately.\n\n<!-- minor -->",
                "2026-10-23T03:00:00Z",
            ),
        ])
        self.assertIn('<li class="entry" id="v1.6.2">', page)
        self.assertNotIn('<li class="entry" id="v1.6.1">', page)
        self.assertIn("Answers update as you type on a phone", page)
        self.assertNotIn("<!--", page)
        self.assertIn("1 smaller update", page)

    def test_an_internal_release_folds_even_as_a_point_zero(self):
        page, feed, _ = build([
            release(
                "v1.7.1",
                "**Folders in the sidebar**\n\nDrag a page into a folder.",
                "2026-10-23T04:00:00Z",
            ),
            release(
                "v1.7.0",
                "**Behind-the-scenes improvements to how updates ship**\n\nNothing you see in the app changed.\n\n<!-- internal -->",
                "2026-10-23T03:00:00Z",
            ),
        ])
        self.assertIn('<li class="entry" id="v1.7.1">', page)
        self.assertNotIn('<li class="entry" id="v1.7.0">', page)
        self.assertIn("Behind-the-scenes improvements to how updates ship", page)
        self.assertNotIn("<!--", page)
        self.assertIn("v1.7.0 — Behind-the-scenes improvements to how updates ship", feed)


if __name__ == "__main__":
    unittest.main()
