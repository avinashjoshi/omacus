#!/usr/bin/env python3
"""Fill the website's examples with answers from the real Omacus engine.

    site/tools/gen.py            (builds target/release/omacus if needed)

- docs/index.html: every `<!-- ex:NAME -->…<!-- /ex -->` block becomes a table of the
  EXAMPLES[NAME] lines and their results.
- index.html: the feature cards' example results.
- js/demo-data.js: the demo sheets in tools/demo/, with results, for the animated hero.
- guides/ and use-cases/: built from tools/pages/*.html (see build_pages), with every
  example and sheet calculated live.
- The header and footer of every page, from one copy below.
- llms-full.txt: the docs as Markdown, for AI assistants and answer engines.
"""

import base64
import datetime
import html
import json
import re
import subprocess
import zlib
from pathlib import Path

SITE = Path(__file__).resolve().parent.parent
ROOT = SITE.parent
BIN = ROOT / "target" / "release" / "omacus"

# Each group is evaluated as one sheet, so variables carry over between its lines.
EXAMPLES = {
    "arithmetic": ["2 + 2 * 3", "(1 + 2) ^ 3", "sqrt(144) + 10 mod 3", "5 x 3",
                   "10 times 3 plus 2", "1.5k * 4", "1e6 / 3"],
    "percentages": ["20% of 340", "5% on $200", "15% off $1,299", "$120 as % of $480",
                    "5% of what is 6", "18% tip on $62", "$62 + 18% tip", "10% off 20% off $100", "$84 split 3 ways",
                    "split $240 between 4 people"],
    "units": ["3.5 km in miles", "2 hours + 45 min in minutes", "72 fahrenheit in celsius",
              "60 mph in km/h", "2 cups in ml", "1 imperial pint in ml", "10 GB / 4", "1 acre in m^2",
              "4 GB at 100 Mbps", "80 GB at 50 Mbps"],
    "currency": ["$30 in EUR", "€50 + £20 in USD", "£250 in USD", "¥10,000 in USD",
                 "CHF 300 in USD", "C$100 in USD", "₹5,000 in USD"],
    "variables": ["rent = $1,450", "rent * 12", "rate = $85/hour", "rate * 30 hours"],
    "totals": ["Coffee $4.50", "Lunch with Sam $18", "Taxi $12", "total", "average",
               "prev * 2"],
    "named-totals": ["Hotel €420", "Dinner €96", "subtotal = sum", "vat = 20% of subtotal",
                     "subtotal + vat"],
    "timezones": ["6pm PST in Tokyo", "10:30 London to New York", "noon UTC in Sydney",
                  "Standup 9:30am PST in Berlin", "3pm PST on Dec 1 in London",
                  "9am PST tomorrow in Tokyo", "3pm PST in IST"],
    "bases": ["255 in hex", "0xff in decimal", "10 in binary"],
    "design": ["16px in rem", "1.5rem in px", "12pt in px", "24px in rem at 18px", "#ff8800 in rgb",
               "#f80 in hsl", "rgb(51, 102, 153) in hex", "hsl(150, 60%, 45%)"],
    "notes": ["# A header", "// a comment line", "Rent: $1,450", "Lunch with Sam $18",
              '"quoted text" 40 + 2', "- hotel = 3 nights * $180", "hotel * 2",
              "rent = $100 # monthly"],
}

DEMO_ORDER = ["kyoto", "invoice", "standup", "budget", "kitchen"]


def evaluate(lines):
    if not BIN.exists():
        subprocess.run(["cargo", "build", "--release", "-q"], cwd=ROOT, check=True)
    out = subprocess.run([str(BIN), "-"], input="\n".join(lines), capture_output=True,
                         text=True, check=True).stdout
    results = out.split("\n")
    return [(results[i] if i < len(results) else "") for i in range(len(lines))]


def table(name):
    lines = EXAMPLES[name]
    rows = "\n".join(
        f'      <tr><td><code>{html.escape(line)}</code></td>'
        f'<td class="ans">{html.escape(result) if result else "<span class=muted>—</span>"}</td></tr>'
        for line, result in zip(lines, evaluate(lines))
    )
    return ('<table class="ex">\n    <thead><tr><th>You type</th><th>Omacus shows</th></tr></thead>\n'
            f"    <tbody>\n{rows}\n    </tbody>\n  </table>")


def fill_docs():
    path = SITE / "docs" / "index.html"
    text = path.read_text()
    text = re.sub(r"<!-- ex:([\w-]+) -->.*?<!-- /ex -->",
                  lambda m: f"<!-- ex:{m[1]} -->\n  {table(m[1])}\n  <!-- /ex -->",
                  text, flags=re.S)
    path.write_text(text)


def fill_home():
    """The feature cards' `<div><span>line</span><b>result</b></div>` examples. Each
    card's lines are evaluated together, so a variable defined above is in scope."""
    path = SITE / "index.html"
    line_re = re.compile(r"<div><span>(.*?)</span><b>(.*?)</b></div>")

    def card(m):
        block = m[0]
        lines = [html.unescape(x[1]) for x in line_re.finditer(block)]
        results = iter(evaluate(lines))
        return line_re.sub(lambda x: f"<div><span>{x[1]}</span><b>{html.escape(next(results))}</b></div>", block)

    text = re.sub(r'<div class="eg">.*?</div></div>', card, path.read_text(), flags=re.S)
    path.write_text(text)


def demo_data():
    sheets = []
    for name in DEMO_ORDER:
        lines = (SITE / "tools" / "demo" / f"{name}.txt").read_text().rstrip("\n").split("\n")
        sheets.append({
            "title": lines[0].lstrip("# ").strip(),
            "link": share_payload("\n".join(lines)),
            "lines": [{"text": t, "result": r} for t, r in zip(lines, evaluate(lines))],
        })
    js = ("// Generated by site/tools/gen.py from site/tools/demo/*.txt. Do not edit.\n"
          f"window.OMACUS_DEMO = {json.dumps(sheets, ensure_ascii=False, indent=1)};\n")
    (SITE / "js" / "demo-data.js").write_text(js)


def to_markdown(fragment):
    """The docs article as Markdown. Handles just the markup the docs use."""
    t = fragment
    t = re.sub(r"<!--.*?-->", "", t, flags=re.S)

    def table(m):
        rows = []
        for tr in re.findall(r"<tr>(.*?)</tr>", m[0], re.S):
            cells = [inline(c).strip() or " " for c in re.findall(r"<t[hd][^>]*>(.*?)</t[hd]>", tr, re.S)]
            rows.append("| " + " | ".join(cells) + " |")
        if len(rows) > 1:
            rows.insert(1, "|" + "---|" * rows[0].count(" | ") + "---|")
        return "\n\n" + "\n".join(rows) + "\n\n"

    def annot(m):
        lines = []
        for code, note in re.findall(r'<code[^>]*>(.*?)</code>(?:<span class="c">(.*?)</span>)?', m[1], re.S):
            code = html.unescape(code)
            lines.append(f"{code}  # {html.unescape(note).lstrip('# ')}" if note else code)
        return "\n\n```\n" + "\n".join(lines) + "\n```\n\n"

    t = re.sub(r"<table.*?</table>", table, t, flags=re.S)
    t = re.sub(r'<div class="annot">(.*?)</div>', annot, t, flags=re.S)
    t = re.sub(r"<pre>(.*?)</pre>",
               lambda m: "\n\n```\n" + html.unescape(re.sub(r"<[^>]+>", "", m[1])) + "\n```\n\n", t, flags=re.S)
    t = re.sub(r"<h1[^>]*>(.*?)</h1>", lambda m: f"\n# {inline(m[1])}\n\n", t, flags=re.S)
    t = re.sub(r"<h2[^>]*>(.*?)</h2>", lambda m: f"\n\n## {inline(m[1])}\n\n", t, flags=re.S)
    t = re.sub(r"<h3[^>]*>(.*?)</h3>", lambda m: f"\n### {inline(m[1])}\n\n", t, flags=re.S)
    t = re.sub(r"<li>(.*?)</li>", lambda m: f"- {inline(m[1]).strip()}\n", t, flags=re.S)
    t = re.sub(r"<(?:p|div)[^>]*>(.*?)</(?:p|div)>", lambda m: f"\n{inline(m[1]).strip()}\n\n", t, flags=re.S)
    t = re.sub(r"</?(?:ul|ol|article|div)[^>]*>", "\n", t)
    out, fenced = [], False
    for line in t.split("\n"):
        if line.strip().startswith("```"):
            fenced = not fenced
        out.append(line.rstrip() if fenced or line.strip().startswith("```") else line.strip())
    t = re.sub(r"\n{3,}", "\n\n", "\n".join(out))
    t = re.sub(r"(\n- [^\n]*)\n\n(?=- )", r"\1\n", t)  # keep list items together
    return t.strip() + "\n"


def inline(s):
    s = re.sub(r"<(?:code|kbd)>(.*?)</(?:code|kbd)>", lambda m: f"`{html.unescape(m[1])}`", s, flags=re.S)
    s = re.sub(r'<a href="([^"]+)"[^>]*>(.*?)</a>',
               lambda m: f"[{m[2]}]({m[1] if m[1].startswith('http') else 'https://omacus.com/docs/' + m[1] if m[1].startswith('#') else 'https://omacus.com' + m[1]})", s, flags=re.S)
    s = re.sub(r"</?(?:strong|b)>", "**", s)
    s = re.sub(r"</?(?:em|i)>", "*", s)
    s = re.sub(r"<[^>]+>", "", s)
    return " ".join(html.unescape(s).split()) if "\n" in s else html.unescape(s)


def llms_full():
    text = (SITE / "docs" / "index.html").read_text()
    article = text[text.index('<article class="prose">'):text.index("</article>")]
    body = to_markdown(article)
    header = ("<!-- Generated by site/tools/gen.py from docs/index.html. Do not edit. -->\n"
              "Source: https://omacus.com/docs/\n\n")
    (SITE / "llms-full.txt").write_text(header + body)


# ---------------------------------------------------------------------------------------
# Shared header and footer

NAV = [("/#features", "Features"), ("/use-cases/", "Use cases"), ("/guides/", "Guides"),
       ("/docs/", "Docs"), ("/updates/", "Updates"), ("/story/", "Story"),
       ("/sync/", "Sync"),
       ("https://app.omacus.com", "Web app"),
       ("https://github.com/avinashjoshi/omacus", "GitHub")]
FOOTER = [("/docs/", "Docs"), ("/guides/", "Guides"), ("/use-cases/", "Use cases"),
          ("/updates/", "Updates"), ("/story/", "Story"), ("/sync/", "Sync"), ("/privacy/", "Privacy"),
          ("https://github.com/avinashjoshi/omacus/issues/new/choose", "Report an issue"),
          ("https://github.com/avinashjoshi/omacus", "GitHub")]


def header(path):
    links = []
    for href, label in NAV:
        attrs = ""
        if href != "/#features" and path.startswith(href):
            attrs += ' aria-current="page"'
        links.append(f'      <a href="{href}"{attrs}>{label}</a>')
    return ('<header class="site-header">\n  <div class="wrap">\n'
            '    <a class="brand" href="/"><img src="/img/icon.svg" alt="" width="28" height="28">omacus</a>\n'
            '    <nav class="nav" aria-label="Main">\n      <div class="nav-links" id="site-nav">\n'
            + "\n".join("  " + l for l in links) + "\n      </div>\n"
            '      <a class="btn primary" href="/download/"><svg aria-hidden="true"><use href="/img/sprite.svg#download"/></svg>Download</a>\n'
            '      <button class="menu-toggle" type="button" aria-controls="site-nav" aria-expanded="false" aria-label="Menu">'
            "<span></span><span></span><span></span></button>\n"
            "    </nav>\n  </div>\n</header>")


def footer():
    links = "\n".join(f'      <a href="{h}">{l}</a>' for h, l in FOOTER)
    return ('<footer class="site-footer">\n  <div class="wrap">\n'
            '    <a class="brand" href="/"><img src="/img/icon.svg" alt="" width="24" height="24">omacus</a>\n'
            "    <span>MIT licensed · made for Omarchy, at home everywhere</span>\n"
            f'    <nav aria-label="Footer">\n{links}\n    </nav>\n  </div>\n</footer>')


def sync_chrome(file, path):
    text = file.read_text()
    text = re.sub(r'<header class="site-header">.*?</header>', lambda _: header(path), text, flags=re.S)
    text = re.sub(r'<footer class="site-footer">.*?</footer>', lambda _: footer(), text, flags=re.S)
    file.write_text(text)


# ---------------------------------------------------------------------------------------
# Guides and use cases
#
# A page source in tools/pages/ starts with a front-matter comment:
#
#   <!--
#   path: /guides/time-zones/
#   title: <title> for search results
#   description: one or two sentences for search results
#   kicker: guide
#   h1: The heading on the page
#   lede: The paragraph under it
#   related: /guides/currency/, /use-cases/
#   -->
#
# and its body can use:
#
#   <!-- ex
#   3pm PST in IST
#   noon UTC in Tokyo
#   -->                      a table of lines and their answers (one sheet)
#   <!-- sheet meeting -->   a sheet from tools/pages/sheets/ or tools/demo/, drawn like the app
#
# An <h2 id="questions"> section of <h3> + <p> pairs becomes the page's FAQ structured data.

PAGES = SITE / "tools" / "pages"
AUTHOR = {"@type": "Person", "@id": "https://omacus.com/#author", "name": "Avinash Joshi",
          "url": "https://github.com/avinashjoshi"}
# path -> (title, blurb) for "related" cards; pages add themselves as they're read.
CARD_TEXT = {"/docs/": ("Docs", "Every rule, shortcut and setting on one page.")}


def front_matter(text):
    m = re.match(r"\s*<!--\n(.*?)\n-->\n", text, re.S)
    meta = dict(line.split(": ", 1) for line in m[1].splitlines() if ": " in line)
    return meta, text[m.end():]


def ex_table(lines):
    rows = "\n".join(
        f'      <tr><td><code>{html.escape(line)}</code></td>'
        f'<td class="ans">{html.escape(result) if result else "<span class=muted>—</span>"}</td></tr>'
        for line, result in zip(lines, evaluate(lines)))
    return ('<table class="ex">\n    <thead><tr><th>You type</th><th>Omacus shows</th></tr></thead>\n'
            f"    <tbody>\n{rows}\n    </tbody>\n  </table>")


KEYWORDS = set("in to as of on off what is prev sum total avg average mean today now tomorrow "
               "yesterday plus minus times mod".split())


def highlight(line, variables):
    """The app's syntax colors (src/highlight.rs), simplified, as HTML."""
    if line.lstrip().startswith("#"):
        return f'<span class="t-head">{html.escape(line)}</span>'
    code, comment = (line.split("//", 1) + [None])[:2]
    out = ""
    label = re.match(r"^(\s*[A-Za-z][A-Za-z '\-_&/]*:)(?= )", code)
    if label:
        out += f'<span class="t-label">{html.escape(label[1])}</span>'
        code = code[label.end():]
    for m in re.finditer(r'("[^"]*"?)|(\d[\w.,]*)|([A-Za-z_]\w*)|([$£€¥₹₩₽₺%])|(\s+)|(.)', code):
        tok = html.escape(m[0])
        if m[1]:
            out += f'<span class="t-com">{tok}</span>'
        elif m[2] or m[4]:
            out += f'<span class="t-num">{tok}</span>'
        elif m[3]:
            cls = "t-var" if m[3] in variables else "t-kw" if m[3].lower() in KEYWORDS else ""
            out += f'<span class="{cls}">{tok}</span>' if cls else tok
        elif m[5]:
            out += tok
        else:
            out += f'<span class="t-op">{tok}</span>'
    if comment is not None:
        out += f'<span class="t-com">//{html.escape(comment)}</span>'
    return out


def share_payload(text):
    """The `v1.` payload the app and the engine make (raw deflate, base64url, no padding)."""
    packer = zlib.compressobj(9, zlib.DEFLATED, -15)
    packed = packer.compress(text.encode()) + packer.flush()
    return "v1." + base64.urlsafe_b64encode(packed).decode().rstrip("=")


def share_link(text):
    """A link that opens `text` at app.omacus.com."""
    return "https://app.omacus.com#" + share_payload(text)


def sheet_html(name):
    path = PAGES / "sheets" / f"{name}.txt"
    if not path.exists():
        path = SITE / "tools" / "demo" / f"{name}.txt"
    lines = path.read_text().rstrip("\n").split("\n")
    variables = {m[1] for l in lines if (m := re.match(r"^\s*([A-Za-z_]\w*)\s*=[^=]", l))}
    rows = "\n".join(
        f'    <li><span class="src">{highlight(line, variables) or " "}</span>'
        f'<span class="res">{html.escape(result)}</span></li>'
        for line, result in zip(lines, evaluate(lines)))
    title = lines[0].lstrip("# ").strip()
    open_link = (f'<a class="open-sheet" href="{share_link(chr(10).join(lines))}" target="_blank" rel="noopener">'
                 'Open in the web app <svg aria-hidden="true"><use href="/img/sprite.svg#arrow"/></svg></a>')
    return (f'<figure class="sheet-card">\n  <figcaption><i></i><i></i><i></i><span>{html.escape(title)}</span>'
            f'{open_link}</figcaption>\n'
            f'  <ol class="sheet-lines">\n{rows}\n  </ol>\n</figure>')


def ld(obj):
    return ('  <script type="application/ld+json">\n' + json.dumps(obj, ensure_ascii=False, indent=1)
            + "\n  </script>\n")


def head(meta, faq):
    url = "https://omacus.com" + meta["path"]
    crumbs = [("Omacus", "https://omacus.com/")]
    if meta["path"].startswith("/guides/") and meta["path"] != "/guides/":
        crumbs.append(("Guides", "https://omacus.com/guides/"))
    crumbs.append((meta["h1"] if meta["path"] != "/guides/" else "Guides", url))
    graph = [
        {"@type": ("CollectionPage" if meta["path"] == "/guides/" else
                   "TechArticle" if meta["path"].startswith("/guides/") else "WebPage"),
         "@id": url + "#page", "headline": meta["h1"], "description": meta["description"], "url": url,
         "inLanguage": "en", "dateModified": "{{lastmod}}", "author": AUTHOR, "publisher": AUTHOR,
         "about": {"@type": "SoftwareApplication", "@id": "https://omacus.com/#app", "name": "Omacus"},
         "image": "https://omacus.com/img/og.png?v=2"},
        {"@type": "BreadcrumbList", "itemListElement": [
            {"@type": "ListItem", "position": i + 1, "name": n, "item": u} for i, (n, u) in enumerate(crumbs)]},
    ]
    if faq:
        graph.append({"@type": "FAQPage", "@id": url + "#questions", "mainEntity": [
            {"@type": "Question", "name": q, "acceptedAnswer": {"@type": "Answer", "text": a}} for q, a in faq]})
    e = html.escape
    return f'''<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'self' 'unsafe-inline' https://static.cloudflareinsights.com; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self' https://cloudflareinsights.com; media-src 'self'; frame-src 'self' https://app.omacus.com; worker-src 'self'; manifest-src 'self'; base-uri 'self'; object-src 'none'; form-action 'self'">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{e(meta["title"])}</title>
  <meta name="description" content="{e(meta["description"])}">
  <meta name="author" content="Avinash Joshi">
  <link rel="canonical" href="{url}">
  <meta name="theme-color" content="#090909">
  <meta property="og:type" content="article">
  <meta property="og:site_name" content="Omacus">
  <meta property="og:url" content="{url}">
  <meta property="og:title" content="{e(meta["h1"])}">
  <meta property="og:description" content="{e(meta["description"])}">
  <meta property="og:image" content="https://omacus.com/img/og.png?v=2">
  <meta property="og:image:width" content="1200">
  <meta property="og:image:height" content="630">
  <meta property="og:image:alt" content="Omacus: a notepad calculator with answers beside each line">
  <meta name="twitter:card" content="summary_large_image">
  <meta name="twitter:title" content="{e(meta["h1"])}">
  <meta name="twitter:description" content="{e(meta["description"])}">
  <meta name="twitter:image" content="https://omacus.com/img/og.png?v=2">
  <link rel="manifest" href="/site.webmanifest">
  <link rel="icon" href="/img/icon.svg" type="image/svg+xml">
  <link rel="icon" href="/img/favicon-32.png" sizes="32x32">
  <link rel="apple-touch-icon" href="/img/apple-touch-icon.png">
  <link rel="preload" href="/fonts/inter-latin.woff2" as="font" type="font/woff2" crossorigin>
  <link rel="preload" href="/fonts/inter-tight-latin.woff2" as="font" type="font/woff2" crossorigin>
  <link rel="stylesheet" href="/css/site.css">
{ld({"@context": "https://schema.org", "@graph": graph})}</head>
<body>
'''


def related_cards(paths):
    cards = []
    for p in paths:
        title, blurb = CARD_TEXT[p]
        cards.append(f'      <a class="guide-card" href="{p}"><b>{html.escape(title)}</b>'
                     f'<span>{html.escape(blurb)}</span></a>')
    return "\n".join(cards)


def build_pages():
    sources = sorted(PAGES.glob("*.html"))
    parsed = []
    for src in sources:
        meta, body = front_matter(src.read_text())
        CARD_TEXT[meta["path"]] = (meta.get("card", meta["h1"]), meta.get("blurb", meta["description"]))
        parsed.append((meta, body))
    today = datetime.date.today().strftime("%-d %B %Y")
    built = []
    for meta, body in parsed:
        body = re.sub(r"<!-- ex\n(.*?)\n-->", lambda m: ex_table(m[1].split("\n")), body, flags=re.S)
        body = re.sub(r"<!-- sheet ([\w-]+) -->", lambda m: sheet_html(m[1]), body)
        guides = sorted((m for m, _ in parsed if m["path"].startswith("/guides/") and m["path"] != "/guides/"),
                        key=lambda m: int(m.get("order", 99)))
        body = body.replace("{{guide_cards}}", related_cards([m["path"] for m in guides]))
        body = body.replace("{{calculated_on}}", today)
        faq = []
        if 'id="questions"' in body:
            qa = body[body.index('id="questions"'):]
            faq = [(H(q), " ".join(H(a).split())) for q, a in re.findall(r"<h3>(.*?)</h3>\s*<p>(.*?)</p>", qa, re.S)]
        related = [p.strip() for p in meta.get("related", "").split(",") if p.strip()]
        crumb = '<a href="/">Omacus</a>'
        if meta["path"].startswith("/guides/") and meta["path"] != "/guides/":
            crumb += ' <span>/</span> <a href="/guides/">Guides</a>'
        sections = re.findall(r'<h2 id="([\w-]+)">(.*?)</h2>', body)
        toc = ""
        if len(sections) >= 4:
            toc = ('  <nav class="toc" aria-label="On this page">\n    <p>On this page</p>\n'
                   + "\n".join(f'    <a href="#{i}">{t}</a>' for i, t in sections) + "\n  </nav>\n")
        page = head(meta, faq) + header(meta["path"]) + f'''

<main class="wrap page{" has-toc" if toc else ""}">
  <nav class="crumbs" aria-label="Breadcrumb">{crumb}</nav>
{toc}  <article class="prose page-body">
    <span class="kicker">// {html.escape(meta.get("kicker", "guide"))}</span>
    <h1>{meta["h1"]}</h1>
    <p class="lead">{meta["lede"]}</p>
{body.strip()}
  </article>
''' + (f'''
  <section class="related" aria-labelledby="related">
    <h2 id="related">Keep reading</h2>
    <div class="guide-cards">
{related_cards(related)}
    </div>
  </section>
''' if related else "") + '''
  <div class="cta">
    <h2>Try it on your own <em class="serif">numbers</em>.</h2>
    <p>Free for Linux, macOS and Windows. One command, and Omacus is a keystroke away.</p>
    <div class="cmd"><span class="prompt">$</span><code>curl -fsSL https://omacus.com/install | bash</code><button class="copy" data-copy="curl -fsSL https://omacus.com/install | bash"><svg aria-hidden="true"><use href="/img/sprite.svg#copy"/></svg>Copy</button></div>
    <div class="dl-row"><a class="btn" href="/#download"><svg aria-hidden="true"><use href="/img/sprite.svg#download"/></svg>All downloads</a><a class="btn" href="/docs/"><svg aria-hidden="true"><use href="/img/sprite.svg#book"/></svg>Read the docs</a></div>
  </div>
</main>

''' + footer() + '''

<script src="/js/site.js"></script>
</body>
</html>
'''
        out = SITE / meta["path"].strip("/") / "index.html"
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(page)
        built.append(meta["path"])
    return built


def H(s):
    return html.unescape(re.sub(r"<[^>]+>", "", s)).strip()


# ---------------------------------------------------------------------------------------
# Downloads
#
# The platform cards live on /download/; the home page links into them. Short links like
# /mac lead to the right card; only a button you click downloads a file.

DL = "https://github.com/avinashjoshi/omacus/releases/latest/download"
INSTALL_CMD = ('<div class="cmd"><span class="prompt">$</span><code>curl -fsSL https://omacus.com/install | bash</code>'
               '<button class="copy" data-copy="curl -fsSL https://omacus.com/install | bash"><svg aria-hidden="true">'
               '<use href="/img/sprite.svg#copy"/></svg>Copy</button></div>')
SHORT_LINKS = {"mac": "mac", "windows": "windows", "linux": "linux", "deb": "linux", "rpm": "linux"}


def btn(href, icon, label, cls="btn", attrs=""):
    return (f'<a class="{cls}" href="{href}"{attrs}><svg aria-hidden="true"><use href="/img/sprite.svg#{icon}"/></svg>'
            f'{label}</a>')


def download_cards(indent):
    cards = f'''<div class="downloads">
  <article class="dl" id="omarchy">
    <header><svg aria-hidden="true"><use href="/img/sprite.svg#archlinux"/></svg><h2>Omarchy <small>&amp; Arch</small></h2></header>
    <p>Installs the app, then offers a keybind (<kbd>Super Ctrl Alt =</kbd> by default, checked against yours) and a floating window.</p>
    {INSTALL_CMD}
    <p class="note">Colors follow your Omarchy theme automatically.</p>
  </article>

  <article class="dl" id="mac">
    <header><svg aria-hidden="true"><use href="/img/sprite.svg#apple"/></svg><h2>macOS <small>Apple Silicon &amp; Intel</small></h2></header>
    <p>The one-liner picks the build for your Mac's chip, installs <code>Omacus.app</code> into Applications and opens without a security prompt.</p>
    {INSTALL_CMD}
    <div class="links">
      {btn(f"{DL}/omacus-macos-arm64.dmg", "download", "Apple Silicon .dmg", attrs=' data-arch="arm64"')}
      {btn(f"{DL}/omacus-macos-x86_64.dmg", "download", "Intel .dmg", attrs=' data-arch="x86_64"')}
    </div>
    <p class="note">Not sure which? Apple menu → About This Mac shows the chip. The app isn't notarized yet. On macOS 15 or later, open it once, then <b>System Settings → Privacy &amp; Security → Open Anyway</b>. On an older Mac, right-click the app and choose Open.</p>
  </article>

  <article class="dl" id="windows">
    <header><svg aria-hidden="true"><use href="/img/sprite.svg#windows"/></svg><h2>Windows <small>64-bit</small></h2></header>
    <p>The installer puts Omacus in Start and opens it, with no admin prompt. It updates itself. A portable <code>omacus.exe</code> is there too.</p>
    <div class="links">{btn(f"{DL}/omacus-windows-x86_64-setup.exe", "download", "Download installer", "btn primary")}{btn(f"{DL}/omacus-windows-x86_64.zip", "download", "Portable .zip")}</div>
    <p class="note">Omacus isn't signed yet, so a downloaded installer may get a SmartScreen warning: choose <b>More info → Run anyway</b>.</p>
  </article>

  <article class="dl" id="linux">
    <header><svg aria-hidden="true"><use href="/img/sprite.svg#linux"/></svg><h2>Other Linux <small>x86_64</small></h2></header>
    <p>The same one-liner works on any distribution, or use a package:</p>
    <div class="links">
      {btn(f"{DL}/omacus-linux-x86_64.deb", "ubuntu", ".deb")}
      {btn(f"{DL}/omacus-linux-x86_64.rpm", "fedora", ".rpm")}
      {btn(f"{DL}/omacus-linux-x86_64.tar.gz", "terminal", ".tar.gz")}
    </div>
    <p class="note">Ubuntu: <code>sudo apt install ./omacus-linux-x86_64.deb</code> · Fedora: <code>sudo dnf install ./omacus-linux-x86_64.rpm</code></p>
  </article>
</div>'''
    return "\n".join((indent + l) if l else l for l in cards.split("\n"))


def fill_downloads():
    """/download/, and the short links that point into it."""

    meta = {"path": "/download/", "h1": "Download Omacus",
            "title": "Download Omacus: free notepad calculator for Linux, macOS and Windows",
            "description": "Download Omacus, the free and open-source notepad calculator: a one-line installer for "
                           "Omarchy, Linux and macOS, .dmg for Apple Silicon and Intel Macs, a portable Windows .exe, "
                           "and .deb, .rpm and .tar.gz packages."}
    page = head(meta, []) + header("/download/") + f'''

<main class="wrap download-page">
  <div class="section-head center">
    <span class="kicker">// download</span>
    <h1>Get <em class="serif">Omacus</em>.</h1>
    <p>Free and MIT licensed. <span data-version-text>The latest release</span>, for every platform. Once installed, Omacus tells you when there's a new version and updates in a click.</p>
  </div>

  <div class="try-web web-first">
    <p class="phone-note">On a phone or tablet? The downloads below are for computers. Omacus runs in your browser instead, and works offline once you add it to your Home Screen.</p>
    <a class="web-app-link" href="https://app.omacus.com" target="_blank" rel="noopener">
      <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z"/></svg>
      <span><b>Open the web app</b><small>Runs in your browser. Nothing to install.</small></span>
      <svg class="arrow" aria-hidden="true"><use href="/img/sprite.svg#arrow"/></svg>
    </a>
    <span class="or">or download the app</span>
  </div>

{download_cards("  ")}

  <section class="release-facts" aria-labelledby="every-release">
    <h2 id="every-release">Every release</h2>
    <ul>
      <li><a href="/updates/">What's new</a> lists every version in plain words. <a href="https://github.com/avinashjoshi/omacus/releases/latest">GitHub</a> has the files.</li>
      <li><a href="{DL}/SHA256SUMS.txt">SHA256SUMS.txt</a> has each file's checksum: <code>sha256sum -c SHA256SUMS.txt --ignore-missing</code>.</li>
      <li>Runs on 64-bit Linux with glibc 2.35 or newer (Ubuntu 22.04, Fedora 36 and later), macOS 11 or newer, and Windows 10 and 11.</li>
      <li>Built from source on GitHub for every release. <a href="/docs/#install">Install notes</a> cover each platform.</li>
    </ul>
  </section>
</main>

''' + footer() + '''

<script src="/js/site.js"></script>
</body>
</html>
'''
    out = SITE / "download" / "index.html"
    out.parent.mkdir(exist_ok=True)
    out.write_text(page)

    for name, card in SHORT_LINKS.items():
        url = f"/download/#{card}"
        (SITE / name / "index.html").write_text(f'''<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'self' 'unsafe-inline' https://static.cloudflareinsights.com; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self' https://cloudflareinsights.com; media-src 'self'; frame-src 'self' https://app.omacus.com; worker-src 'self'; manifest-src 'self'; base-uri 'self'; object-src 'none'; form-action 'self'">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Download Omacus</title>
  <meta name="robots" content="noindex">
  <meta http-equiv="refresh" content="0; url={url}">
  <link rel="canonical" href="https://omacus.com/download/">
  <script>location.replace("{url}");</script>
</head>
<body><p><a href="{url}">Download Omacus</a></p></body>
</html>
''')
    return ["/download/"]


def sitemap(paths):
    entries = []
    for p in ["/", "/docs/", "/updates/", "/story/", "/sync/"] + paths:
        extra = ""
        if p == "/":
            extra = "".join(f"\n    <image:image><image:loc>https://omacus.com/img/screens/{n}.webp</image:loc></image:image>"
                            for n in ("examples-dark", "web-examples", "unfold-dark", "matte-black", "tokyo-night", "help-syntax"))
        entries.append(f"  <url>\n    <loc>https://omacus.com{p}</loc>\n    <lastmod>{{{{lastmod}}}}</lastmod>{extra}\n  </url>")
    (SITE / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"\n'
        '        xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">\n' + "\n".join(entries) + "\n</urlset>\n")


if __name__ == "__main__":
    fill_docs()
    fill_home()
    demo_data()
    llms_full()
    pages = build_pages() + fill_downloads()
    for f, path in [(SITE / "index.html", "/"), (SITE / "docs" / "index.html", "/docs/")]:
        sync_chrome(f, path)
    sitemap(pages)
    print("examples and demo data updated")
