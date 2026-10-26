#!/usr/bin/env python3
"""The sync health workflow can open one issue, and the runbook names the real worker."""

from __future__ import annotations

import os
import stat
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "sync-health.yml"
RUNBOOK = ROOT / "docs" / "ops" / "runbook.md"
HEADERS = ROOT / "site" / "_headers"
HOME = ROOT / "site" / "index.html"


def fail(message: str) -> None:
    raise SystemExit(message)


def main() -> int:
    workflow = WORKFLOW.read_text()
    if "workflow_dispatch:" not in workflow:
        fail("sync health cannot be started by hand")
    if "issues: write" not in workflow:
        fail("sync health cannot open an issue")
    if "gh issue list --label sync-health --state open" not in workflow:
        fail("sync health does not look for an open issue")
    if "an open sync-health issue already exists" not in workflow:
        fail("sync health would open a second issue")
    script = workflow.split("Open an issue when sync is down", 1)[-1]
    script = script.split("run: |", 1)[-1]
    # Drop the YAML indent and leave the GitHub expressions as plain text.
    lines = []
    for line in script.splitlines():
        if line.startswith("          "):
            lines.append(line[10:])
        elif line.strip():
            break
    body = "\n".join(lines) + "\n"
    body = body.replace("${{", "").replace("}}", "")
    with tempfile.TemporaryDirectory() as tmp:
        bin_dir = Path(tmp) / "bin"
        bin_dir.mkdir()
        state = Path(tmp) / "state"
        stub = bin_dir / "gh"
        stub.write_text(
            "#!/bin/sh\n"
            "case \"$1 $2\" in\n"
            "  'label list') exit 0 ;;\n"
            "  'label create') exit 0 ;;\n"
            "  'issue list')\n"
            f"    if [ -f {state} ]; then echo 1; else echo 0; fi\n"
            "    exit 0 ;;\n"
            "  'issue create')\n"
            f"    if [ -f {state} ]; then echo second > {state}.again; exit 1; fi\n"
            f"    echo created > {state}\n"
            "    exit 0 ;;\n"
            "  *) echo \"unexpected gh $*\" >&2; exit 2 ;;\n"
            "esac\n"
        )
        stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
        env = os.environ.copy()
        env["PATH"] = str(bin_dir) + os.pathsep + env.get("PATH", "")
        first = subprocess.run(["bash", "-c", body], env=env, capture_output=True, text=True)
        if first.returncode == 0:
            fail(f"the first failure did not stay red\n{first.stdout}\n{first.stderr}")
        if not state.exists():
            fail(f"the first failure did not open an issue\n{first.stdout}\n{first.stderr}")
        second = subprocess.run(["bash", "-c", body], env=env, capture_output=True, text=True)
        if second.returncode == 0:
            fail("a later failure was treated as success")
        if (Path(str(state) + ".again")).exists():
            fail("a later failure opened a second issue")
        if "an open sync-health issue already exists" not in second.stdout:
            fail(f"a later failure did not say the issue is already open\n{second.stdout}\n{second.stderr}")

    runbook = RUNBOOK.read_text()
    for phrase in (
        "wrangler.jsonc",
        "MAILBOXES",
        "BACKUPS",
        "omacus-sync-backups",
        "./build.sh",
        "npx wrangler deploy",
        ".github/workflows/app.yml",
    ):
        if phrase not in runbook:
            fail(f"runbook is missing {phrase}")
    for phrase in ("omacus-sync-test", "sync-test.omacus.com", "--env production"):
        if phrase in runbook:
            fail(f"runbook still names a retired sync host: {phrase}")
    if "not `wrangler.toml`" not in runbook:
        fail("runbook does not say the config is wrangler.jsonc rather than wrangler.toml")
    if "Mailboxes namespace" in runbook:
        fail("runbook still calls Mailboxes a namespace")
    order = runbook.split("## Go live", 1)[-1]
    steps = ("Merge to `main`.", "Deploy the production worker", "Smoke test that worker", "Publish the sync build", "Smoke test the app")
    cursor = 0
    for step in steps:
        found = order.find(step, cursor)
        if found < 0:
            fail(f"go-live order is missing {step}")
        cursor = found + len(step)

    headers = HEADERS.read_text()
    if "Strict-Transport-Security:" not in headers or "Content-Security-Policy:" not in headers:
        fail("omacus.com headers are missing HSTS or CSP")
    home = HOME.read_text()
    if 'http-equiv="Content-Security-Policy"' not in home:
        fail("the homepage does not carry the content security policy")
    if 'kind="captions"' not in home:
        fail("the homepage film has no captions track")
    if "Open the web app" not in home:
        fail("the homepage does not offer the web app")
    print("sync health and runbook ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
