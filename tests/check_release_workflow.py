#!/usr/bin/env python3
"""The Release workflow can publish after main has moved, and can still start."""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
text = (ROOT / ".github" / "workflows" / "release.yml").read_text()


def job(name, nxt):
    start = text.index(f"  {name}:")
    end = text.index(f"\n  {nxt}:", start)
    return text[start:end]


publish = job("publish", "install-test")
deploy_app = text[text.index("  deploy-app:"):]

errors = []
tag_at = publish.find('git push origin "refs/tags/v${version}"')
release_at = publish.find("softprops/action-gh-release@")
if tag_at < 0 or release_at < 0 or tag_at > release_at:
    errors.append("publish must push the tag before creating the GitHub release")
if "persist-credentials: true" not in publish:
    errors.append("publish must keep credentials so it can push the tag")
header, _, _ = deploy_app.partition("uses:")
if "contents: write" not in header:
    errors.append("deploy-app must grant contents: write to the called app workflow")

if errors:
    print("\n".join(errors))
    sys.exit(1)
print("release workflow ok")
