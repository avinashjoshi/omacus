#!/usr/bin/env python3
"""The marketing pages keep Sync in the menu, and the sync page stays current."""

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / ".github" / "scripts"))
sys.path.insert(0, str(ROOT / "site" / "tools"))
import release_notes  # noqa: E402
import updates  # noqa: E402

SITE = ROOT / "site"
ALT = "Omacus Sync: pages encrypted on your device. The notepad needs no sign-up. Sync is invite-only."
SENTENCE = "The <strong>Turn on sync</strong> button in the sidebar explains sync, then continues in the web app."
FAQ = "The notepad needs no sign-up, no email, no password, no ads and no tracking. Sync is invite-only."


class SitePages(unittest.TestCase):
    def test_sync_is_in_the_top_menu(self):
        pages = [p for p in SITE.rglob("*.html") if 'id="site-nav"' in p.read_text()]
        self.assertGreaterEqual(len(pages), 15)
        for path in pages:
            text = path.read_text()
            nav = text.split('id="site-nav">', 1)[1].split("</div>", 1)[0]
            self.assertIn('<a href="/sync/">Sync</a>' if path != SITE / "sync" / "index.html"
                          else '<a href="/sync/" aria-current="page">Sync</a>', nav, path)
            self.assertIn('href="/story/"', nav, path)

    def test_the_sync_page_waits_for_a_real_video(self):
        text = (SITE / "sync" / "index.html").read_text()
        self.assertNotIn("sync-video", text)
        self.assertNotIn('href="#video"', text)
        self.assertNotIn("Nothing plays there yet", text)
        self.assertIn('<a href="https://app.omacus.com">Turn on sync in the web app</a>', text)
        self.assertIn(SENTENCE, text)
        self.assertIn(f'content="https://omacus.com/img/og-sync.png?v=2"', text)
        self.assertIn(f'content="{ALT}"', text)
        image = (SITE / "img" / "og-sync.png").read_bytes()
        self.assertTrue(image.startswith(b"\x89PNG"))
        self.assertEqual(int.from_bytes(image[16:20], "big"), 1200)
        self.assertEqual(int.from_bytes(image[20:24], "big"), 630)

    def test_privacy_uses_the_same_sentence(self):
        for path in (SITE / "privacy" / "index.html", SITE / "tools" / "pages" / "privacy.html"):
            self.assertIn(SENTENCE, path.read_text(), path)
            self.assertNotIn("Turn on sync in the sidebar explains sync and continues", path.read_text())

    def test_the_home_faq_names_what_you_do_not_make(self):
        text = (SITE / "index.html").read_text()
        self.assertEqual(text.count(FAQ), 2)
        self.assertNotIn("The app has no account", text)

    def test_the_upcoming_release_is_on_the_updates_page(self):
        notes = updates.upcoming_notes()
        self.assertIsNotNone(notes)
        body = release_notes.section((ROOT / "CHANGELOG.md").read_text(), "1.0.0")
        self.assertEqual(release_notes.problems(body), [])
        self.assertIn("/sync/", body)
        self.assertIn("/privacy/", body)
        self.assertIn("upcoming", body)
        page = (SITE / "updates" / "index.html").read_text()
        self.assertIn(updates.upcoming_html(notes, updates.Images(None, SITE)), page)
        self.assertIn('<a href="#v1.0.0">v1.0.0</a>', page)
        feed = (SITE / "updates" / "feed.xml").read_text()
        self.assertIn("v1.0.0 — Sync is in the web app", feed)
        self.assertLess(feed.index("#v1.0.0"), feed.index("#v0.2.15"))


if __name__ == "__main__":
    unittest.main()
