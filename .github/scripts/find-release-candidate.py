#!/usr/bin/env python3
"""Find a successful candidate run for the exact tree being published."""

import json
import os
import subprocess

ARTIFACTS = {
    "release-linux", "release-windows", "release-macos-arm64", "release-macos-x86_64", "web-app",
}
WORKFLOW = ".github/workflows/release-candidate.yml"


def api(path):
    return json.loads(subprocess.check_output(["gh", "api", path], timeout=15))


def find(repo, tree, runs):
    for run in runs:
        if (
            run.get("event") != "push" or run.get("conclusion") != "success"
            or run.get("path") != WORKFLOW or run.get("head_branch") == "main"
            or (run.get("head_repository") or {}).get("full_name") != repo
        ):
            continue
        sha = run["head_sha"]
        commit = api(f"repos/{repo}/git/commits/{sha}")
        if commit["tree"]["sha"] != tree:
            continue
        run_id = run["id"]
        artifacts = api(f"repos/{repo}/actions/runs/{run_id}/artifacts?per_page=100")["artifacts"]
        available = {asset["name"] for asset in artifacts if not asset["expired"]}
        release_assets = {name for name in available if name.startswith("release-") or name == "web-app"}
        if release_assets == ARTIFACTS:
            return run_id
    return None


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    tree = subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], text=True).strip()
    try:
        # Most pushes don't produce a candidate. Sort by updated time so recently
        # completed artifact builds are among the first runs checked.
        runs = api(f"repos/{repo}/actions/workflows/release-candidate.yml/runs"
                   "?event=push&status=success&per_page=20")["workflow_runs"]
        run_id = find(repo, tree, runs)
    except (subprocess.SubprocessError, ValueError, KeyError) as error:
        print(f"Candidate lookup unavailable ({error}). Building from the merged commit.")
        return
    if run_id is not None:
        print(f"Reusing release packages from successful run {run_id} for tree {tree}.")
        with open(os.environ["GITHUB_OUTPUT"], "a") as output:
            print(f"artifact_run={run_id}", file=output)


if __name__ == "__main__":
    main()
