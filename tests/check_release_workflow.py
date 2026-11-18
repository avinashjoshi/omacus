#!/usr/bin/env python3
"""The Release workflow can publish after main has moved, and can still start."""

import importlib.util
import re
import sys
from pathlib import Path
from unittest.mock import patch

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
build = (ROOT / ".github" / "workflows" / "release-build.yml").read_text()
def build_job(name):
    return re.split(r"\n  [a-z][a-z0-9-]*:\n", build.split(f"\n  {name}:\n", 1)[1], maxsplit=1)[0]

for name in ("ci", "linux", "windows", "macos"):
    if "    needs:" in build_job(name):
        errors.append(f"{name} must start alongside the other build jobs")
if "needs.build.result == 'success'" not in publish:
    errors.append("publish must wait for the full build and CI unless a prepared run was verified")
if "!needs.check.outputs.artifact_run" not in job("build", "build-app"):
    errors.append("a verified candidate should skip the merged commit rebuild")
macos = build_job("macos")
if not all(part in macos for part in ("matrix:", "aarch64-apple-darwin", "x86_64-apple-darwin", "bundle-macos.py", "codesign --verify")):
    errors.append("macOS must bundle one prebuilt executable per chip in parallel")
if "cargo install cargo-bundle" in macos:
    errors.append("macOS release must package with the runner's existing tools")
if "pattern: release-*" not in publish:
    errors.append("publish must download only desktop artifacts into the signed manifest")
ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
if "prepare_web_release: ${{ inputs.prepare_web_release }}" not in build_job("ci"):
    errors.append("release CI must prepare the web app during the wasm job")
if "name: web-app" not in ci[ci.index("  wasm:"):]:
    errors.append("release CI must upload the web app built by the wasm job")
if "needs.check.outputs.release" in job("build-app", "publish"):
    errors.append("only an already-tagged release should build the app outside of CI")
if any(part not in deploy_app for part in ("needs.build-app.result == 'success'", 'name: web-app', 'prebuilt: "true"')):
    errors.append("deploy-app must reuse the web app artifact")
site = (ROOT / ".github" / "workflows" / "site.yml").read_text()
if "reuse_web_build: true" not in job("deploy-site", "deploy-app") or "name: web-app" not in site:
    errors.append("the release site must reuse the browser engine from the wasm job")
# A re-run uploads a second Pages artifact. One name per attempt keeps
# deploy-pages from finding two, and one retry covers an artifact the API
# does not list yet.
attempt_name = "github-pages-${{ github.run_attempt }}"
if site.count(attempt_name) != 3:
    errors.append("the Pages artifact must be named per attempt, for the upload and both deploys")
if "continue-on-error: true" not in site or "id: deployment-retry" not in site:
    errors.append("the Pages deploy must retry once after a failure")
if "steps.deployment-retry.outputs.page_url" not in site:
    errors.append("the Pages environment URL must come from the retry when it ran")
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
check = job("check", "build")
if "web_catchup" not in check or "https://app.omacus.com/version.json" not in check:
    errors.append("a released version must notice when the live web app is older")
if '["$live" != "$version" ]' not in check and '[ "$live" != "$version" ]' not in check:
    errors.append("the live web app must be compared with this version")
if "needs.check.outputs.web_catchup == 'true'" not in job("build-app", "publish"):
    errors.append("deploy-app must publish when the live web app is behind this version")
if "needs.check.outputs.release == 'true'" not in deploy_app:
    errors.append("a new version must still publish the web app with the desktop release")
sync = (ROOT / ".github" / "workflows" / "sync.yml").read_text()
deploy_sync = text[text.index("  deploy-sync:"):]
if "uses: ./.github/workflows/sync.yml" not in deploy_sync:
    errors.append("a release must publish the sync worker")
if "ref: ${{ needs.check.outputs.sha }}" not in deploy_sync or "ref: ${{ inputs.ref || github.sha }}" not in sync:
    errors.append("a release must deploy the sync worker from the built commit")
if "reuse_web_build: ${{ needs.publish.result == 'success' }}" not in deploy_sync or "name: web-app" not in sync:
    errors.append("the release sync worker must reuse the browser engines from the wasm job")
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

spec = importlib.util.spec_from_file_location(
    "release_candidate", ROOT / ".github/scripts/find-release-candidate.py",
)
candidate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(candidate)
repo = "example/notepad"
run = {
    "id": 42,
    "event": "push",
    "conclusion": "success",
    "path": ".github/workflows/release-candidate.yml",
    "head_branch": "release/example",
    "head_repository": {"full_name": repo},
    "head_sha": "candidate-sha",
}
commit = {"tree": {"sha": "tested-tree"}}
artifacts = {"artifacts": [{"name": name, "expired": False} for name in candidate.ARTIFACTS]}
with patch.object(candidate, "api", side_effect=[commit, artifacts]):
    if candidate.find(repo, "tested-tree", [run]) != 42:
        errors.append("a successful candidate with the same tree should be reused")
with patch.object(candidate, "api", return_value=commit):
    if candidate.find(repo, "changed-tree", [run]) is not None:
        errors.append("a changed merge must rebuild")
with patch.object(candidate, "api", side_effect=[commit, {"artifacts": artifacts["artifacts"][1:]}]):
    if candidate.find(repo, "tested-tree", [run]) is not None:
        errors.append("an incomplete candidate must rebuild")
with patch.object(candidate, "api") as api:
    wrong_runs = [
        {**run, "event": "pull_request"},
        {**run, "conclusion": "failure"},
        {**run, "path": ".github/workflows/other.yml"},
        {**run, "head_repository": {"full_name": "example/fork"}},
    ]
    if candidate.find(repo, "tested-tree", wrong_runs) is not None or api.called:
        errors.append("only a successful push run of this repository's candidate workflow can be reused")

if errors:
    print("\n".join(errors))
    sys.exit(1)
print("release workflow ok")
