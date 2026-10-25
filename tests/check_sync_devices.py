#!/usr/bin/env python3
"""The sync home screen renames every phone the same way."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sync = (ROOT / "site/app/sync.js").read_text()
css = (ROOT / "site/app/app.css").read_text()

start = sync.index("function drawLinked()")
end = sync.index("function addActions(", start)
home = sync[start:end]

errors = []
if "Name this" in home:
    errors.append("the device list still has its own name field")
if 'row.setAttribute("data-self", "1")' not in home:
    errors.append("this phone is not marked")
if "beginRename(row, device)" not in home:
    errors.append("Rename does not cover every row")
if "if (!device.self)" not in home or "Remove" not in home:
    errors.append("Remove is not limited to the other phones")
marker = '.sync-device[data-self="1"]'
if marker not in css:
    errors.append("this phone's row is not styled")
else:
    rule = css[css.index(marker):css.index(marker) + 400]
    if "var(--app-accent-soft)" not in rule or "var(--app-accent)" not in rule:
        errors.append("this phone's row does not use the accent")

off = sync.index("function drawOff()")
join = sync.index("function drawJoinType()")
if "Name this" not in sync[off:join]:
    errors.append("starting sync lost the name field")

if errors:
    raise SystemExit("\n".join(errors))
print("sync devices ok")
