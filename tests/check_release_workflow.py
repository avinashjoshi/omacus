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
install = (ROOT / ".github" / "workflows" / "install-test.yml").read_text()
update = install[install.index("  update:"):]

errors = []
tag_at = publish.find('git push origin "refs/tags/v${version}"')
release_at = publish.find("softprops/action-gh-release@")
sign_at = publish.find("scripts/sign-update.mjs")
if sign_at < 0 or tag_at < 0 or sign_at > tag_at:
    errors.append("publish must sign the update manifest before pushing the tag")
if "MINISIGN_SECRET_KEY is not set" not in publish:
    errors.append("publish must refuse an unsigned update")
if "environment: release" not in publish:
    errors.append("publish must use the release environment")
if "RELEASE_TOKEN is not set" in publish or "Refusing to create a version tag" in publish:
    errors.append("publish must not refuse a version tag when RELEASE_TOKEN is missing")
tag_step = publish[publish.find("Point the tag"):]
if "secrets.RELEASE_TOKEN" in tag_step or "gh auth setup-git" in tag_step:
    errors.append("the tag push must use the checkout token")
if tag_at < 0 or release_at < 0 or tag_at > release_at:
    errors.append("publish must push the tag before creating the GitHub release")
if "persist-credentials: true" not in publish:
    errors.append("publish must keep credentials so it can push the tag")
header, _, steps = deploy_app.partition("steps:")
if "contents: write" not in header:
    errors.append("deploy-app must grant contents: write so it can record the build hash")
if "environment: app" not in header:
    errors.append("deploy-app must use the app environment")
if "uses: ./.github/workflows/app.yml" in deploy_app:
    errors.append("the web app deploy must run in the release workflow")
if "EXPECT_VERSION" not in deploy_app and "expect_version" not in deploy_app:
    errors.append("the web app deploy must ship the same version as the desktop release")
if "./.github/actions/deploy-app" not in steps:
    errors.append("deploy-app must publish the web app")
check = job("check", "ci")
if "web_catchup" not in check or "https://app.omacus.com/version.json" not in check:
    errors.append("a released version must notice when the live web app is older")
if '["$live" != "$version" ]' not in check and '[ "$live" != "$version" ]' not in check:
    errors.append("the live web app must be compared with this version")
if "needs.check.outputs.web_catchup == 'true'" not in deploy_app:
    errors.append("deploy-app must publish when the live web app is behind this version")
if "needs.check.outputs.release == 'true'" not in deploy_app:
    errors.append("a new version must still publish the web app with the desktop release")
sync = (ROOT / ".github" / "workflows" / "sync.yml").read_text()
if "uses: ./.github/workflows/sync.yml" not in text:
    errors.append("a release must publish the sync worker")
if "npx wrangler deploy" not in sync or "--env production" in sync:
    errors.append("the sync worker deploy must publish omacus-sync and no second worker")
if "push" not in sync:
    errors.append("the sync worker deploy must check that the live worker can push")
if "--exclude-drafts" not in update or "--exclude-pre-releases" not in update:
    errors.append("the update job must skip drafts and pre-releases")
if "^v[0-9]" not in update:
    errors.append("the update job must keep only version tags")
if "--limit 2" in update:
    errors.append("the update job must not take the first two releases before filtering")
if "--limit 50" not in update:
    errors.append("the update job must fetch more than the two it keeps")

if errors:
    print("\n".join(errors))
    sys.exit(1)
print("release workflow ok")
