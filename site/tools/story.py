#!/usr/bin/env python3
"""Build the static story page from the git checkout and GitHub.

    python3 site/tools/story.py [--out site]

The site workflow runs this while assembling omacus.com. Counts (days since
the first commit, releases, merged pull requests, lines of Rust) are computed
here. A figure that cannot be counted reliably is left off the page. Timeline
dates come from git tags when the clone has them, otherwise from the release's
published date. Nothing on the page is a guessed date or a guessed count.

The narrative is fixed. The numbers and the milestones are not.
"""

import argparse
import html
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from datetime import date, datetime, timezone
from pathlib import Path

import updates

SITE = Path(__file__).resolve().parent.parent
ROOT = SITE.parent
PAGE = "https://omacus.com/story/"
REPO = updates.REPO
TAG = re.compile(r"^v\d+\.\d+\.\d+$")
PULLS = f"https://api.github.com/repos/{REPO}/pulls?state=closed&per_page=100"

# Shown only when a real release matches. The date and version come from that
# release. Copy stays free of counts, so a missing release drops the whole row.
RULES = [
    (
        "first",
        "First version",
        "A notepad for math, on Linux, macOS, and Windows from the first version. I was already using it, and so was Ajith, on his Mac.",
        lambda notes: True,
    ),
    (
        "web",
        "The web app",
        "The same engine, in the browser. A shared page stays inside the link.",
        lambda notes: "web app" in notes,
    ),
    (
        "updates",
        "An updates page",
        "Every release, in plain words, newest first.",
        lambda notes: "updates page" in notes,
    ),
    (
        "offline",
        "The web app works offline",
        "After the first visit, the web app keeps working with no connection.",
        lambda notes: "works offline" in notes or "no connection" in notes,
    ),
]

# The first rule matches every release so it can mean "the earliest one".
# The others match on the notes, and the earliest matching release wins.
EARLIEST_ONLY = {"first"}


def git(root, *args):
    try:
        run = subprocess.run(
            ["git", "-C", str(root), *args],
            capture_output=True, text=True, timeout=60, check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if run.returncode != 0:
        return None
    return run.stdout


def shallow(root):
    out = git(root, "rev-parse", "--is-shallow-repository")
    return out is None or out.strip() != "false"


def tag_days(root):
    """Map vX.Y.Z tags to the tagged commit's date. None when tags or history are incomplete."""
    if shallow(root):
        return None
    names = git(root, "tag", "--list")
    if names is None:
        return None
    days = {}
    for tag in names.split():
        if not TAG.match(tag):
            continue
        day = git(root, "log", "-1", "--format=%cs", tag)
        if not day or len(day.strip()) < 10:
            return None
        days[tag] = day.strip()[:10]
    return days


def first_commit_day(root):
    """Calendar day of the oldest root commit. None on a shallow clone, which lacks it."""
    if shallow(root):
        return None
    out = git(root, "rev-list", "--max-parents=0", "HEAD")
    if not out:
        return None
    days = []
    for sha in out.split():
        day = git(root, "log", "-1", "--format=%cs", sha)
        if not day or len(day.strip()) < 10:
            return None
        days.append(day.strip()[:10])
    return min(days) if days else None


def days_between(start, end):
    try:
        span = (date.fromisoformat(end) - date.fromisoformat(start)).days
    except ValueError:
        return None
    if span < 0:
        return None
    return span


def rust_lines_in(paths):
    """Newline count, the same figure `wc -l` prints. None if a file can't be read."""
    total = 0
    found = False
    for path in paths:
        try:
            data = Path(path).read_bytes()
        except OSError:
            return None
        found = True
        total += data.count(b"\n")
    return total if found else None


def rust_lines(root):
    listed = git(root, "ls-files", "*.rs")
    if listed is None:
        return None
    paths = [Path(root) / rel for rel in listed.splitlines() if rel.endswith(".rs")]
    if not paths:
        return None
    return rust_lines_in(paths)


def count_merged(pulls):
    """How many closed pull requests actually merged. None if the list is unusable."""
    if not isinstance(pulls, list):
        return None
    count = 0
    for item in pulls:
        if not isinstance(item, dict):
            return None
        if item.get("merged_at"):
            count += 1
    return count


def _next_link(header):
    for part in (header or "").split(","):
        if 'rel="next"' in part and "<" in part and ">" in part:
            return part[part.find("<") + 1:part.find(">")]
    return None


def fetch_merged_count():
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    url = PULLS
    pulls = []
    pages = 0
    while url and pages < 20:
        req = urllib.request.Request(url, headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "omacus-site",
            "X-GitHub-Api-Version": "2022-11-28",
        })
        if token:
            req.add_header("Authorization", f"Bearer {token}")
        try:
            with urllib.request.urlopen(req, timeout=30) as res:
                batch = json.load(res)
                link = res.headers.get("Link") or ""
        except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, json.JSONDecodeError):
            return None
        if not isinstance(batch, list):
            return None
        pulls.extend(batch)
        url = _next_link(link)
        pages += 1
    if url:
        return None
    return count_merged(pulls)


def notes_text(rel):
    """Plain notes for matching a milestone."""
    raw = updates.whats_new(rel.get("body") or "")
    return updates.plain(raw).lower()


def day_of(rel, tag_days_map):
    tag = rel.get("tag_name") or ""
    if tag_days_map and tag in tag_days_map:
        return tag_days_map[tag]
    published = (rel.get("published_at") or "")[:10]
    if len(published) == 10 and published[4] == "-" and published[7] == "-":
        return published
    return None


def sort_key(rel, tag_days_map):
    day = day_of(rel, tag_days_map) or "9999-99-99"
    return (day, updates.version_tuple(rel.get("tag_name")) or ())


def published_releases(raw):
    kept = []
    for rel in raw or []:
        if not isinstance(rel, dict) or rel.get("draft") or rel.get("prerelease"):
            continue
        if not TAG.match(rel.get("tag_name") or ""):
            continue
        if not day_of(rel, None) and not rel.get("tag_name"):
            continue
        kept.append(rel)
    return kept


def select_milestones(raw, tag_days_map):
    """Earliest release that matches each rule, then Sync with no date."""
    releases = published_releases(raw)
    releases.sort(key=lambda rel: sort_key(rel, tag_days_map))
    chosen = []
    for key, title, blurb, pred in RULES:
        match = None
        for rel in releases:
            notes = "" if key in EARLIEST_ONLY else notes_text(rel)
            if (key in EARLIEST_ONLY or pred(notes)) and day_of(rel, tag_days_map):
                match = rel
                break
        if not match:
            continue
        tag = match.get("tag_name")
        chosen.append({
            "id": key,
            "title": title,
            "blurb": blurb,
            "tag": tag,
            "label": updates.version_label(tag),
            "day": day_of(match, tag_days_map),
            "soon": False,
        })
    chosen.sort(key=lambda item: (item["day"], updates.version_tuple(item["tag"]) or ()))
    chosen.append({
        "id": "sync",
        "title": "Sync",
        "blurb": "Coming soon. Pages that stay in step across computers. Not shipped yet.",
        "tag": "",
        "label": "Soon",
        "day": "",
        "soon": True,
    })
    return chosen


def comma(n):
    return f"{n:,}"


def facts_html(facts):
    rows = []
    specs = [
        ("days", "Day since the first commit", "Days since the first commit"),
        ("releases", "Release", "Releases"),
        ("pulls", "Pull request merged", "Pull requests merged"),
        ("rust", "Line of Rust", "Lines of Rust"),
    ]
    for key, singular, plural in specs:
        value = facts.get(key)
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            continue
        label = singular if value == 1 else plural
        rows.append(
            f'<li><b>{html.escape(comma(value))}</b><span>{html.escape(label)}</span></li>'
        )
    if not rows:
        return ""
    items = "\n".join(rows)
    return f'''<section id="numbers">
<h2>By the numbers</h2>
<p>Counted from the repository when this page is built.</p>
<ul class="story-stats">
{items}
</ul>
</section>'''


def milestone_html(item):
    title = html.escape(item["title"])
    blurb = html.escape(item["blurb"])
    if item["soon"]:
        return f'''<li class="entry story-soon" id="sync">
<div class="entry-meta"><span class="version-pill">Soon</span></div>
<div class="entry-body">
<h2>{title}</h2>
<div class="notes"><p>{blurb}</p></div>
</div>
</li>'''
    label = html.escape(item["label"])
    day = html.escape(item["day"])
    shown = html.escape(updates.fmt_date(item["day"]))
    anchor = html.escape(item["label"])
    return f'''<li class="entry" id="story-{anchor}">
<div class="entry-meta">
<a class="version-pill" href="/updates/#{anchor}">{label}</a>
<time datetime="{day}">{shown}</time>
</div>
<div class="entry-body">
<h2>{title}</h2>
<div class="notes"><p>{blurb}</p></div>
</div>
</li>'''


def toc_html(include_numbers):
    items = [
        ("why", "Why it exists"),
        ("built", "How it was built"),
    ]
    if include_numbers:
        items.append(("numbers", "By the numbers"))
    items.extend([
        ("timeline", "Timeline"),
        ("learned", "What I learned"),
        ("contribute", "Contribute"),
    ])
    links = "\n".join(f'<a href="#{i}">{html.escape(label)}</a>' for i, label in items)
    return f'<nav class="toc" aria-label="On this page">\n<p>On this page</p>\n{links}\n</nav>'


def page_html(milestones, facts):
    numbers = facts_html(facts)
    timeline = "\n".join(milestone_html(item) for item in milestones)
    e = html.escape
    description = "Why Omacus exists, and how it was built. A personal project, written in the first person."
    return f'''<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>The story of Omacus</title>
  <meta name="description" content="{e(description)}">
  <meta name="author" content="Avinash Joshi">
  <link rel="canonical" href="{PAGE}">
  <meta name="theme-color" content="#090909">
  <meta property="og:type" content="article">
  <meta property="og:site_name" content="Omacus">
  <meta property="og:url" content="{PAGE}">
  <meta property="og:title" content="The story of Omacus">
  <meta property="og:description" content="{e(description)}">
  <meta property="og:image" content="https://omacus.com/img/og.png?v=2">
  <meta name="twitter:card" content="summary_large_image">
  <meta name="twitter:title" content="The story of Omacus">
  <meta name="twitter:description" content="{e(description)}">
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
{gen_header()}

<main class="wrap page has-toc story-page">
  <nav class="crumbs" aria-label="Breadcrumb"><a href="/">Omacus</a> <span>/</span> Story</nav>
  {toc_html(bool(numbers))}
  <article class="page-body">
    <header class="story-hero">
      <span class="kicker">// story</span>
      <h1>A side <em class="serif">quest</em></h1>
      <p class="story-lead">I missed a notepad calculator, so I built one. This is why, and how.</p>
      <p class="story-say">Said <span>oh-MAH-kus</span>.</p>
    </header>

    <section id="why">
      <h2>Why it exists</h2>
      <p>Ajith and I were talking through numbers. I used to use Numi, a notepad calculator, on a Mac. After I moved to Linux, on Omarchy, I missed it.</p>
      <p>I wrote down what I missed, in a single prompt. About 30 minutes later I had a first version that worked. Then I kept refining it, because I was using it.</p>
      <p>Ajith is on a Mac, and he wanted to use Omacus too. So it runs on Linux on Omarchy, on macOS, on Windows, and on the web. We both use it every day.</p>
      <p>I built it for myself. I open-sourced it, under the MIT license, in case someone else wanted it too.</p>
    </section>

    <section id="built">
      <h2>How it was built</h2>
      <p>I didn't hand-write the code. I set the direction, made the decisions, and reviewed what came back. AI agents wrote every line.</p>
      <p>It started with Claude. It now runs on Grok Bot. Grok Bot plans the work and runs cloud coding agents. Those agents open pull requests. Grok Bot watches the checks and ships the releases.</p>
      <p>What makes that safe enough:</p>
      <ul class="story-net">
        <li><strong>Protected main.</strong> A change only lands through a pull request.</li>
        <li><strong>Checks on every system we ship.</strong> Linux, macOS, Windows, and the browser build all have to pass.</li>
        <li><strong>A release is a pull request.</strong> When the checks pass, it merges on its own, and the new version goes out.</li>
        <li><strong>One design guide.</strong> <a href="https://github.com/{REPO}/blob/main/docs/DESIGN.md">docs/DESIGN.md</a> is what keeps the app and the site looking like the same product.</li>
        <li><strong>Notes for people using it.</strong> Release notes say what changed for you, in plain words.</li>
      </ul>
      <p>My part is taste, decisions, and review.</p>
    </section>

    {numbers}

    <section id="timeline">
      <h2>Timeline</h2>
      <p>A few moments from the releases. Sync is the one that is not out yet.</p>
      <ol class="timeline">
{timeline}
      </ol>
    </section>

    <section id="learned">
      <h2>What I learned</h2>
      <p>Personal notes. Not a method, and not advice.</p>
      <ol class="story-lessons">
        <li>
          <h3>The short part was the start</h3>
          <p>Getting from "I wish this existed" to a version I could type into was fast. Almost everything I kept came after that.</p>
        </li>
        <li>
          <h3>The safety net matters more than the prompt</h3>
          <p>A prompt gets a draft. I trust the result because main is protected, the checks cover every system we ship, and the notes are written for someone using Omacus.</p>
        </li>
        <li>
          <h3>Taste is the job</h3>
          <p>Agents can fill a screen. Choosing what should be quiet, what a sentence should say, and what to leave out is still on me.</p>
        </li>
        <li>
          <h3>One person kept it smaller</h3>
          <p>If I wouldn't use a feature on a page of my own, it stayed out.</p>
        </li>
      </ol>
    </section>

    <section id="contribute">
      <h2>Contribute</h2>
      <p>The code is MIT licensed. If something is wrong, or missing, open an issue. If you want to change it, send a pull request.</p>
      <p>You can use AI agents too. I do. The same bar applies: the checks have to pass, the design guide is the look, and the notes should make sense to someone using Omacus.</p>
      <p class="story-links"><a href="https://github.com/{REPO}">Source code</a><a href="https://github.com/{REPO}/issues">Issues</a><a href="https://github.com/{REPO}/blob/main/CONTRIBUTING.md">Contributing</a></p>
    </section>
  </article>
</main>

{gen_footer()}

<script src="/js/site.js"></script>
</body>
</html>
'''


def gen_header():
    import gen
    return gen.header("/story/")


def gen_footer():
    import gen
    return gen.footer()


def build(raw_releases, out: Path, facts, tag_days_map=None):
    milestones = select_milestones(raw_releases, tag_days_map or {})
    page = page_html(milestones, facts or {})
    dest = Path(out) / "story"
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "index.html").write_text(page)
    return milestones


def gather(root, today=None):
    """Counts and tag dates from a full checkout. Missing pieces stay absent."""
    facts = {}
    tags = tag_days(root)
    if tags is not None:
        facts["releases"] = len(tags)
    first = first_commit_day(root)
    if first:
        today = today or datetime.now(timezone.utc).date().isoformat()
        span = days_between(first, today)
        if span is not None:
            facts["days"] = span
    lines = rust_lines(root)
    if lines is not None:
        facts["rust"] = lines
    pulls = fetch_merged_count()
    if isinstance(pulls, int):
        facts["pulls"] = pulls
    return facts, tags


def main():
    parser = argparse.ArgumentParser(description="Build omacus.com/story from git and GitHub")
    parser.add_argument("--out", type=Path, default=SITE, help="directory that contains story/")
    parser.add_argument("--input", type=Path, help="a JSON file of releases, instead of the API")
    args = parser.parse_args()
    facts, tags = gather(ROOT)
    if args.input:
        raw = json.loads(args.input.read_text())
    else:
        try:
            raw = updates.fetch_releases("")
        except SystemExit as exc:
            existing = args.out / "story" / "index.html"
            if existing.exists():
                print(f"story: kept the checked-in page ({exc})", file=sys.stderr)
                return
            raise
    milestones = build(raw, args.out, facts, tags)
    shown = ", ".join(f"{key}={facts[key]}" for key in ("days", "releases", "pulls", "rust") if key in facts)
    print(f"story: {len(milestones) - 1} milestones, {shown or 'no counts'}")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as fh:
            fh.write(f"Story page: {shown or 'no counts'}.\n")


if __name__ == "__main__":
    main()
