#!/usr/bin/env python3
"""Build the static updates page and its Atom feed from GitHub releases.

    python3 site/tools/updates.py [--out site] [--input releases.json]

The site workflow runs this while assembling omacus.com, so the published page
does not call api.github.com from the browser. Release notes are markdown.
The repeated install instructions are left off (the download page covers that).

The headline is a bold title on the first line of the notes, then a sentence
or two. Older notes that are still a list of commit subjects keep the first
line as the headline.

Smaller fixes fold under the newer entry, so a run of one-line patches does not
read as a stack of equal announcements. A `<!-- minor -->` marker folds a
release even when its title does not look like a fix. A `<!-- internal -->`
marker folds a release that has nothing new in the app, including a .0.
Without a marker, a release stays on the timeline when it has a title, is a
.0 version, carries a screenshot, or is not just a fix.

Some release notes have quoted a private page. Those strings are matched below
and never written to the page or the feed. The version tags are printed so the
notes can be edited on GitHub; the text is not.
"""

import argparse
import hashlib
import html
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
import xml.sax.saxutils as xml_escape
from pathlib import Path

import gen

SITE = Path(__file__).resolve().parent.parent
REPO = "avinashjoshi/omacus"
API = f"https://api.github.com/repos/{REPO}/releases"
PAGE = "https://omacus.com/updates/"
FEED = "https://omacus.com/updates/feed.xml"

# Matched case-insensitively against the whole release (notes, title, image
# text, urls). A line that matches is dropped. A match that survives that
# (split across lines) drops the notes for that version.
PRIVATE = [
    re.compile(r"private-term", re.I),
    re.compile(r"duo\s+offer\s+discussion", re.I),
    re.compile(r"duo\s+digital", re.I),
    re.compile(r"series\s+a\s+team", re.I),
    re.compile(r"\bprivate-term\b", re.I),
    re.compile(r"\bprivate-term\b", re.I),
    re.compile(r"private-term", re.I),
    re.compile(r"\$\s*4\s*m\b", re.I),
    re.compile(r"\$\s*110\s*k\s*mrr\b", re.I),
]

# Examples the tests feed through the filter. Kept next to the patterns so a
# new pattern has to say what it catches and what it must leave alone.
PROBES = [
    ("private-term", "soft landing"),
    ("private-term", "offer discussion"),
    ("private-term", "digital"),
    ("private-term", "series of talks"),
    ("private-term", "up front"),
    ("private-term", "staff"),
    ("private-term", "companies"),
    ("private-term", "$4 a day"),
    ("private-term", "MRR and dilution"),
]

REVISED = "These notes are being edited before they go on the site."
MONTHS = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split()

# A maintenance note, even with a nested explanation. A .0 release, a
# screenshot, or a note that announces something else stays on the timeline.
MINOR_START = (
    "fix ", "keep ", "expect ", "draw ", "give ", "position ", "accuracy",
    "web-mobile", "harden ", "smaller ", "readme", "use https",
    "make deleting", "make the ", "read ", "recalculate ",
    "windows: leave", "windows: open without",
)

TOKEN = re.compile(
    r"!\[(?P<alt>[^\]]*)\]\((?P<img>[^)\s]+)\)"
    r"|\[(?P<label>[^\]]+)\]\((?P<href>[^)\s]+)\)"
    r"|`(?P<code>[^`]+)`"
    r"|\*\*(?P<bold>[^*]+)\*\*"
    r"|\(#(?P<issue>\d+)\)"
)
IMG_TAG = re.compile(r"<img\b[^>]*>", re.I)
BULLET = re.compile(r"^( *)- (.*)$")
COMPARE = re.compile(r"https://github\.com/\S+/compare/\S+")
TITLE_LINE = re.compile(r"^\*\*(.+)\*\*$")
DETAILS_LINE = re.compile(r"^\*\*Details:\*\*\s+\S+\s*$", re.I)
FLAG = re.compile(r"<!--\s*(internal|minor)\s*-->", re.I)
COMMENT = re.compile(r"<!--.*?-->", re.S)
MAX_IMAGE = 8_000_000


class Item:
    def __init__(self, text):
        self.parts = [text]
        self.children = []

    @property
    def text(self):
        return " ".join(self.parts)


def private(text):
    return any(p.search(text or "") for p in PRIVATE)


def plain(text):
    text = TOKEN.sub(lambda m: m.group("alt") or m.group("label") or m.group("code")
                     or m.group("bold") or (f"#{m.group('issue')}" if m.group("issue") else ""), text)
    return re.sub(r"\s+", " ", text).strip()


def looks_minor(text):
    return plain(text).lower().startswith(MINOR_START)


def version_tuple(tag):
    m = re.search(r"(\d+)\.(\d+)\.(\d+)", tag or "")
    return tuple(int(x) for x in m.groups()) if m else None


def version_label(tag):
    m = re.search(r"\d+\.\d+\.\d+", tag or "")
    return f"v{m.group(0)}" if m else (tag or "release")


def anchor_for(tag):
    return version_label(tag)


def fmt_date(iso):
    y, m, d = (iso or "1970-01-01")[:10].split("-")
    return f"{MONTHS[int(m) - 1]} {int(d)}, {y}"


def fmt_short(iso):
    y, m, d = (iso or "1970-01-01")[:10].split("-")
    return f"{MONTHS[int(m) - 1]} {int(d)}"


def normalize_images(text):
    def img(m):
        tag = m.group(0)
        src = re.search(r'\bsrc\s*=\s*"([^"]+)"', tag, re.I) or re.search(r"\bsrc\s*=\s*'([^']+)'", tag, re.I)
        alt = re.search(r'\balt\s*=\s*"([^"]*)"', tag, re.I) or re.search(r"\balt\s*=\s*'([^']*)'", tag, re.I)
        if not src:
            return ""
        return f"![{alt.group(1) if alt else ''}]({src.group(1)})"

    return IMG_TAG.sub(img, text or "")


def whats_new(body):
    text = normalize_images(body or "")
    m = re.search(r"^## What's new\s*$", text, re.M)
    if m:
        text = text[m.end():]
    n = re.search(r"^## ", text, re.M)
    if n:
        text = text[:n.start()]
    return text.strip()


def redact(text):
    """Drop lines that contain a private string, and the bullets nested under them."""
    dropped = None
    kept = []
    hit = False
    for line in text.splitlines():
        indent = len(line) - len(line.lstrip(" "))
        if dropped is not None:
            if line.strip() and indent > dropped:
                hit = True
                continue
            dropped = None
        if private(line):
            hit = True
            if line.lstrip().startswith("-"):
                dropped = indent
            continue
        kept.append(line)
    cleaned = "\n".join(kept).strip()
    if private(cleaned):
        return "", True
    return cleaned, hit


def parse_notes(text):
    """A list of ('item', Item) and ('p', str), in order."""
    blocks = []
    stack = []
    for line in text.splitlines():
        if not line.strip():
            continue
        m = BULLET.match(line)
        if m:
            indent = len(m.group(1))
            item = Item(m.group(2).strip())
            while stack and stack[-1][0] >= indent:
                stack.pop()
            if stack:
                stack[-1][1].children.append(item)
            else:
                blocks.append(("item", item))
            stack.append((indent, item))
            continue
        indent = len(line) - len(line.lstrip(" "))
        if stack and indent > stack[-1][0]:
            stack[-1][1].parts.append(line.strip())
            continue
        stack.clear()
        blocks.append(("p", line.strip()))
    return blocks


def items_of(blocks):
    return [b[1] for b in blocks if b[0] == "item"]


def strip_flags(text):
    """Return the notes without HTML comments, and the minor/internal markers."""
    flags = {m.group(1).lower() for m in FLAG.finditer(text or "")}
    return COMMENT.sub("", text or "").strip(), flags


def split_title(text):
    """A bold first line is the title. The rest is the summary and the bullets."""
    lines = text.splitlines()
    i = 0
    while i < len(lines) and not lines[i].strip():
        i += 1
    if i >= len(lines):
        return "", text
    match = TITLE_LINE.match(lines[i].strip())
    if not match:
        return "", text
    return match.group(1).strip(), "\n".join(lines[i + 1:]).strip()


def is_minor(tag, blocks, images, flags=(), title=""):
    """A fix, even one with a nested explanation. A .0 release, a screenshot,
    a written title, or a note that announces something other than a fix stays
    on the timeline. `internal` folds anyway. `minor` folds a patch."""
    if "internal" in flags:
        return True
    ver = version_tuple(tag)
    if images or (ver is not None and ver[2] == 0):
        return False
    if "minor" in flags:
        return True
    if title:
        return False
    items = items_of(blocks)
    if any(not looks_minor(it.text) for it in items):
        return False
    return True


def safe_url(url):
    url = (url or "").strip()
    lower = url.lower()
    if lower.startswith(("javascript:", "data:", "vbscript:")):
        return None
    if lower.startswith(("https://", "http://", "/", "#", "mailto:")):
        return url
    return None


class Images:
    def __init__(self, fetch, media):
        self.fetch = fetch
        self.media = media
        self.written = set()
        self.seen = {}

    def localize(self, url):
        url = (url or "").strip()
        if not url or private(url):
            return None
        if url in self.seen:
            return self.seen[url]
        local = None
        if url.startswith("/updates/media/"):
            local = url
        elif url.startswith("https://") and self.fetch is not None:
            got = self.fetch(url)
            if got:
                data, ctype = got
                ext = _ext(data, ctype)
                if ext and data and len(data) <= MAX_IMAGE:
                    name = hashlib.sha256(url.encode()).hexdigest()[:16] + "." + ext
                    self.media.mkdir(parents=True, exist_ok=True)
                    (self.media / name).write_bytes(data)
                    self.written.add(name)
                    local = f"/updates/media/{name}"
        self.seen[url] = local
        return local


def _ext(data, ctype):
    ctype = (ctype or "").split(";")[0].strip().lower()
    by_type = {"image/png": "png", "image/jpeg": "jpg", "image/webp": "webp", "image/gif": "gif"}
    if ctype in by_type:
        return by_type[ctype]
    if data.startswith(b"\x89PNG"):
        return "png"
    if data.startswith(b"\xff\xd8"):
        return "jpg"
    if data.startswith(b"GIF8"):
        return "gif"
    if data.startswith(b"RIFF") and data[8:12] == b"WEBP":
        return "webp"
    return None


def inline(text, images):
    out = []
    pos = 0
    for m in TOKEN.finditer(text):
        out.append(html.escape(text[pos:m.start()]))
        if m.group("img") is not None:
            alt = m.group("alt") or ""
            if private(alt):
                pos = m.end()
                continue
            src = images.localize(m.group("img"))
            if src:
                out.append(f'<img src="{html.escape(src, quote=True)}" alt="{html.escape(alt, quote=True)}">')
        elif m.group("label") is not None:
            href = safe_url(m.group("href"))
            label = m.group("label")
            if href and not private(href):
                out.append(f'<a href="{html.escape(href, quote=True)}">{html.escape(label)}</a>')
            else:
                out.append(html.escape(label))
        elif m.group("code") is not None:
            out.append(f"<code>{html.escape(m.group('code'))}</code>")
        elif m.group("bold") is not None:
            out.append(f"<strong>{html.escape(m.group('bold'))}</strong>")
        elif m.group("issue") is not None:
            n = m.group("issue")
            out.append(f'<a href="https://github.com/{REPO}/issues/{n}">#{n}</a>')
        pos = m.end()
    out.append(html.escape(text[pos:]))
    return "".join(out)


def render_items(items, images):
    if not items:
        return ""
    lis = []
    for it in items:
        kids = render_items(it.children, images)
        lis.append(f"<li>{inline(it.text, images)}{kids}</li>")
    return "<ul>\n" + "\n".join(lis) + "\n</ul>"


def paragraph(value, images):
    only = re.fullmatch(r"!\[([^\]]*)\]\(([^)\s]+)\)", value.strip())
    if only:
        alt, url = only.group(1), only.group(2)
        if private(alt):
            return ""
        src = images.localize(url)
        if src:
            cap = f"<figcaption>{html.escape(alt)}</figcaption>" if alt else ""
            return f'<figure><img src="{html.escape(src, quote=True)}" alt="{html.escape(alt, quote=True)}">{cap}</figure>'
    return f"<p>{inline(value, images)}</p>"


def render_blocks(blocks, images):
    parts = []
    run = []

    def flush():
        if run:
            parts.append(render_items(run, images))
            run.clear()

    for kind, value in blocks:
        if kind == "item":
            run.append(value)
        else:
            flush()
            rendered = paragraph(value, images)
            if rendered:
                parts.append(rendered)
    flush()
    return "\n".join(parts)


def asset_images(release):
    found = []
    for asset in release.get("assets") or []:
        name = asset.get("name") or ""
        url = asset.get("browser_download_url") or ""
        if re.search(r"\.(png|jpe?g|webp|gif)$", name, re.I) and url:
            found.append((url, name))
    return found


def prepare(raw):
    """One release, ready to place. `flagged` means the GitHub notes need editing."""
    body = raw.get("body") or ""
    name = raw.get("name") or ""
    flagged = private(body) or private(name)
    cleaned, line_hit = redact(whats_new(body))
    flagged = flagged or line_hit
    cleaned, flags = strip_flags(cleaned)
    title, body_text = split_title(cleaned)
    # The page adds its own Details link from the compare URL.
    body_text = "\n".join(
        line for line in body_text.splitlines() if not DETAILS_LINE.match(line.strip())
    ).strip()
    if flagged and private(title):
        title = ""
    if flagged and private(body_text):
        body_text = ""
    if flagged and not body_text and not title:
        blocks = [("p", REVISED)]
        title = ""
    else:
        blocks = parse_notes(body_text)
    images = asset_images(raw)
    # Image text in the body is already inside `cleaned`. Asset names too.
    if any(private(u) or private(n) for u, n in images):
        flagged = True
        images = [(u, n) for u, n in images if not private(u) and not private(n)]
    compare = COMPARE.search(body)
    tag = raw.get("tag_name") or ""
    published = raw.get("published_at") or ""
    return {
        "tag": tag,
        "label": version_label(tag),
        "anchor": anchor_for(tag),
        "name": name,
        "published": published,
        "date": fmt_date(published),
        "short": fmt_short(published),
        "day": published[:10],
        "html_url": raw.get("html_url") or f"https://github.com/{REPO}/releases/tag/{tag}",
        "compare": compare.group(0).rstrip(").,") if compare else "",
        "prerelease": bool(raw.get("prerelease")),
        "blocks": blocks,
        "images": images,
        "title": title,
        "minor": is_minor(tag, blocks, images, flags, title),
        "flagged": flagged,
        "revised": bool(flagged and blocks == [("p", REVISED)]),
    }


def group(releases):
    """Newest first. A minor release folds into the entry just opened."""
    entries = []
    for rel in releases:
        if not entries or not rel["minor"]:
            entries.append({"head": rel, "fixes": []})
        else:
            entries[-1]["fixes"].append(rel)
    return entries


def title_and_body(rel, images):
    """The headline is the written title, or the first change when the notes are still a list."""
    if rel["revised"]:
        return REVISED, ""
    if rel.get("title"):
        return inline(rel["title"], images), render_blocks(rel["blocks"], images)
    blocks = rel["blocks"]
    top = items_of(blocks)
    if len(top) == 1 and all(kind == "item" for kind, _ in blocks):
        return inline(top[0].text, images), render_items(top[0].children, images)
    if top:
        parts = []
        if top[0].children:
            parts.append(render_items(top[0].children, images))
        if len(top) > 1:
            parts.append(render_items(top[1:], images))
        for kind, value in blocks:
            if kind == "p":
                parts.append(paragraph(value, images))
        return inline(top[0].text, images), "\n".join(parts)
    if blocks and blocks[0][0] == "p":
        return inline(blocks[0][1], images), render_blocks(blocks[1:], images)
    return "Maintenance release", ""


def feed_html(rel, images):
    if rel["revised"]:
        return f"<p>{REVISED}</p>"
    return render_blocks(rel["blocks"], images) or f"<p>{html.escape(plain_title(rel) or 'Maintenance release')}</p>"


def plain_title(rel):
    if rel["revised"]:
        return "Notes being edited"
    if rel.get("title"):
        return plain(rel["title"])
    top = items_of(rel["blocks"])
    if top:
        return plain(top[0].text)
    for kind, value in rel["blocks"]:
        if kind == "p":
            return plain(value)
    return "Maintenance release"


def gallery(rel, images):
    parts = []
    for url, name in rel["images"]:
        src = images.localize(url)
        if not src:
            continue
        alt = "Screenshot" if re.fullmatch(r"[\w.-]+", name) else name
        parts.append(
            f'<figure><img src="{html.escape(src, quote=True)}" alt="{html.escape(alt, quote=True)}"></figure>'
        )
    return "\n".join(parts)


def fix_html(rel, images):
    bits = [
        f'<li id="{html.escape(rel["anchor"])}">',
        f'<p class="fix-line"><a class="version-pill" href="#{html.escape(rel["anchor"])}">{html.escape(rel["label"])}</a>',
        f'<time datetime="{html.escape(rel["day"])}">{html.escape(rel["date"])}</time>',
    ]
    if rel["prerelease"]:
        bits.append('<span class="pre">Pre-release</span>')
    bits.append("</p>")
    title, body = title_and_body(rel, images)
    bits.append(f'<div class="fix-copy"><p>{title}</p>{body}</div>')
    bits.append(f'<p class="entry-links"><a href="{html.escape(rel["html_url"], quote=True)}">GitHub</a></p>')
    bits.append("</li>")
    return "\n".join(bits)


def fixes_html(fixes, images):
    if not fixes:
        return ""
    n = len(fixes)
    word = "update" if n == 1 else "updates"
    newest, oldest = fixes[0], fixes[-1]
    when = newest["short"] if newest["day"] == oldest["day"] else f'{oldest["short"]} – {newest["short"]}'
    span = newest["label"] if n == 1 else f'{oldest["label"]} – {newest["label"]}'
    items = "\n".join(fix_html(rel, images) for rel in fixes)
    return (
        f'<details class="fix-group">\n'
        f'<summary><span>{n} smaller {word}</span>'
        f'<span class="fix-range">{html.escape(span)}</span>'
        f'<span class="fix-when">{html.escape(when)}</span></summary>\n'
        f'<ol class="fixes">\n{items}\n</ol>\n</details>'
    )


def entry_html(entry, images):
    rel = entry["head"]
    title, body = title_and_body(rel, images)
    shots = gallery(rel, images)
    pre = '<span class="pre">Pre-release</span>' if rel["prerelease"] else ""
    links = [f'<a href="{html.escape(rel["html_url"], quote=True)}">GitHub</a>']
    if rel["compare"] and not private(rel["compare"]):
        links.append(f'<a href="{html.escape(rel["compare"], quote=True)}">Details</a>')
    return f'''<li class="entry" id="{html.escape(rel["anchor"])}">
<div class="entry-meta">
<a class="version-pill" href="#{html.escape(rel["anchor"])}">{html.escape(rel["label"])}</a>
{pre}
<time datetime="{html.escape(rel["day"])}">{html.escape(rel["date"])}</time>
</div>
<div class="entry-body">
<h2>{title}</h2>
<div class="notes">
{body}
{shots}
</div>
{fixes_html(entry["fixes"], images)}
<p class="entry-links">{" · ".join(links)}</p>
</div>
</li>'''


def toc_html(entries):
    links = "\n".join(
        f'<a href="#{html.escape(e["head"]["anchor"])}">{html.escape(e["head"]["label"])}</a>'
        for e in entries
    )
    return f'<nav class="toc" aria-label="Versions">\n<p>Versions</p>\n{links}\n</nav>'


def page_html(entries, images):
    # theme-color matches --bg in tokens.css (near-black, and the light paper
    # on this page). Browser chrome only; the page itself uses the variables.
    timeline = "\n".join(entry_html(e, images) for e in entries)
    count = sum(1 + len(e["fixes"]) for e in entries)
    return f'''<!doctype html>
<html lang="en" data-page="updates">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>What's new in Omacus</title>
  <meta name="description" content="Every Omacus release, newest first. What changed, in plain words, with the small fixes grouped together.">
  <meta name="author" content="Avinash Joshi">
  <link rel="canonical" href="{PAGE}">
  <link rel="alternate" type="application/atom+xml" title="Omacus updates" href="/updates/feed.xml">
  <meta name="theme-color" content="#090909" media="(prefers-color-scheme: dark)">
  <meta name="theme-color" content="#f6f5f2" media="(prefers-color-scheme: light)">
  <meta property="og:type" content="website">
  <meta property="og:site_name" content="Omacus">
  <meta property="og:url" content="{PAGE}">
  <meta property="og:title" content="What's new in Omacus">
  <meta property="og:description" content="Every Omacus release, newest first. What changed, in plain words.">
  <meta property="og:image" content="https://omacus.com/img/og.png?v=2">
  <meta name="twitter:card" content="summary_large_image">
  <meta name="twitter:title" content="What's new in Omacus">
  <meta name="twitter:description" content="Every Omacus release, newest first. What changed, in plain words.">
  <meta name="twitter:image" content="https://omacus.com/img/og.png?v=2">
  <link rel="manifest" href="/site.webmanifest">
  <link rel="icon" href="/img/icon.svg" type="image/svg+xml">
  <link rel="icon" href="/img/favicon-32.png" sizes="32x32">
  <link rel="apple-touch-icon" href="/img/apple-touch-icon.png">
  <link rel="preload" href="/fonts/inter-latin.woff2" as="font" type="font/woff2" crossorigin>
  <link rel="preload" href="/fonts/inter-tight-latin.woff2" as="font" type="font/woff2" crossorigin>
  <link rel="preload" href="/fonts/instrument-serif-italic-latin.woff2" as="font" type="font/woff2" crossorigin>
  <link rel="stylesheet" href="/css/site.css">
</head>
<body>
{gen.header("/updates/")}

<main class="wrap page has-toc updates-page">
  <nav class="crumbs" aria-label="Breadcrumb"><a href="/">Omacus</a> <span>/</span> Updates</nav>
  {toc_html(entries)}
  <article class="page-body">
    <header class="updates-hero">
      <span class="kicker">// updates</span>
      <h1>What's <em class="serif">new</em></h1>
      <p class="updates-lead">Every release, newest first. A short title, then what changed for you. Small fixes sit together under the next update, so the list stays readable.</p>
      <p class="updates-tools"><a href="/updates/feed.xml">RSS</a> · <a href="/download/">Download</a> · <a href="https://github.com/{REPO}/releases">GitHub</a></p>
    </header>
    <ol class="timeline">
{timeline}
    </ol>
    <p class="updates-count">{count} {"release" if count == 1 else "releases"}</p>
  </article>
</main>

{gen.footer()}

<script src="/js/site.js"></script>
</body>
</html>
'''


def atom(releases, images):
    updated = releases[0]["published"] if releases else "1970-01-01T00:00:00Z"
    entries = []
    for rel in releases:
        title = f'{rel["label"]} — {plain_title(rel)}'
        content = feed_html(rel, images)
        link = f'{PAGE}#{rel["anchor"]}'
        published = rel["published"] or updated
        entries.append(f'''  <entry>
    <title>{xml_escape.escape(title)}</title>
    <link href="{xml_escape.escape(link)}"/>
    <id>{xml_escape.escape(link)}</id>
    <published>{xml_escape.escape(published)}</published>
    <updated>{xml_escape.escape(published)}</updated>
    <content type="html">{xml_escape.escape(content)}</content>
  </entry>''')
    body = "\n".join(entries)
    return f'''<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Omacus updates</title>
  <subtitle>Every release of Omacus, in plain words.</subtitle>
  <link href="{PAGE}"/>
  <link rel="self" type="application/atom+xml" href="{FEED}"/>
  <id>{PAGE}</id>
  <updated>{xml_escape.escape(updated)}</updated>
  <author><name>Avinash Joshi</name></author>
{body}
</feed>
'''


def build(raw_releases, out: Path, fetch=None):
    raw_releases = [r for r in raw_releases if not r.get("draft")]
    raw_releases.sort(key=lambda r: (r.get("published_at") or "", version_tuple(r.get("tag_name")) or ()), reverse=True)
    prepared = [prepare(r) for r in raw_releases]
    flagged = [rel["label"] for rel in prepared if rel["flagged"]]
    media = out / "updates" / "media"
    images = Images(fetch, media)
    entries = group(prepared)
    dest = out / "updates"
    dest.mkdir(parents=True, exist_ok=True)
    page = page_html(entries, images)
    feed = atom(prepared, images)
    if private(page) or private(feed):
        raise SystemExit("updates: private text reached the generated page; not writing it")
    (dest / "index.html").write_text(page)
    (dest / "feed.xml").write_text(feed)
    if media.exists():
        for child in media.iterdir():
            if child.is_file() and child.name not in images.written:
                child.unlink()
        if not any(media.iterdir()):
            media.rmdir()
    return flagged


def release_is_listed(releases, expect):
    expect = (expect or "").removeprefix("v")
    if not expect:
        return True
    return any(
        version_label(r.get("tag_name")) == f"v{expect}"
        or (r.get("tag_name") or "").removeprefix("v") == expect
        for r in releases
    )


def expect_wait_seconds(expect):
    """How long to poll when a version must be on the API before the page is written.

    The default covers the short gap after a release is published. A version bump
    that deploys the site before that publish sets OMACUS_EXPECT_WAIT longer.
    """
    if not expect:
        return 0
    raw = os.environ.get("OMACUS_EXPECT_WAIT", "")
    if raw.isdigit():
        return int(raw)
    return 60


def fetch_releases(expect=""):
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    expect = (expect or "").removeprefix("v")
    deadline = time.time() + expect_wait_seconds(expect)
    pause = 15 if expect_wait_seconds(expect) > 60 else 3
    last_error = None
    while True:
        try:
            releases = _fetch_pages(token)
        except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, json.JSONDecodeError) as exc:
            last_error = exc
            releases = None
        if releases is not None and release_is_listed(releases, expect):
            return releases
        if releases is not None:
            last_error = f"v{expect} is not listed yet"
        if time.time() >= deadline:
            raise SystemExit(f"updates: couldn't read GitHub releases ({last_error})")
        time.sleep(pause)


def _fetch_pages(token):
    releases = []
    url = API + "?per_page=100"
    pages = 0
    while url and pages < 10:
        req = urllib.request.Request(url, headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "omacus-site",
            "X-GitHub-Api-Version": "2022-11-28",
        })
        if token:
            req.add_header("Authorization", f"Bearer {token}")
        with urllib.request.urlopen(req, timeout=30) as res:
            batch = json.load(res)
            link = res.headers.get("Link") or ""
        if not isinstance(batch, list):
            raise SystemExit("updates: unexpected releases response")
        releases.extend(batch)
        url = None
        for part in link.split(","):
            if 'rel="next"' in part:
                url = part[part.find("<") + 1:part.find(">")]
        pages += 1
    return releases


def fetch_image(url):
    req = urllib.request.Request(url, headers={"User-Agent": "omacus-site"})
    try:
        with urllib.request.urlopen(req, timeout=30) as res:
            ctype = res.headers.get("Content-Type") or ""
            data = res.read(MAX_IMAGE + 1)
    except (urllib.error.URLError, TimeoutError):
        return None
    if len(data) > MAX_IMAGE:
        return None
    return data, ctype


def main():
    parser = argparse.ArgumentParser(description="Build omacus.com/updates from GitHub releases")
    parser.add_argument("--out", type=Path, default=SITE, help="directory that contains updates/")
    parser.add_argument("--input", type=Path, help="a JSON file of releases, instead of the API")
    args = parser.parse_args()
    if args.input:
        raw = json.loads(args.input.read_text())
        flagged = build(raw, args.out, fetch=fetch_image)
    else:
        expect = os.environ.get("OMACUS_EXPECT_VERSION", "")
        try:
            raw = fetch_releases(expect)
        except SystemExit:
            existing = args.out / "updates" / "index.html"
            if expect or not existing.exists():
                raise
            print("updates: kept the checked-in page; GitHub releases were unavailable", file=sys.stderr)
            return
        flagged = build(raw, args.out, fetch=fetch_image)
    print(f"updates: {len(raw)} releases, {len(flagged)} with notes to edit")
    if flagged:
        print("updates: edit the GitHub release notes for " + ", ".join(flagged), file=sys.stderr)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as fh:
            fh.write(f"Updates page: {len(raw)} releases.\n")
            if flagged:
                fh.write("Release notes to edit (private text was left off the page): " + ", ".join(flagged) + "\n")


if __name__ == "__main__":
    main()
