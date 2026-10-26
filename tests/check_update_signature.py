#!/usr/bin/env python3
"""The Node signer and the locked minisign fixture stay the same signature."""

import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "fixtures"
SECRET = "omacus-update-key-v1\n" + ("22" * 8) + "\n" + ("11" * 32) + "\n"


def main() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        message = Path(tmp) / "omacus-update-test.txt"
        message.write_bytes((FIXTURE / "omacus-update-test.txt").read_bytes())
        env = os.environ.copy()
        env["MINISIGN_SECRET_KEY"] = SECRET
        env["TRUSTED_COMMENT"] = "timestamp:1"
        subprocess.run(
            ["node", str(ROOT / "scripts" / "sign-update.mjs"), str(message)],
            check=True,
            env=env,
        )
        public = subprocess.run(
            ["node", str(ROOT / "scripts" / "sign-update.mjs"), "--public"],
            check=True,
            env=env,
            capture_output=True,
            text=True,
        )
        got_sig = (message.with_name(message.name + ".minisig")).read_text()
        want_sig = (FIXTURE / "omacus-update-test.minisig").read_text()
        want_pub = (FIXTURE / "omacus-update-test.pub").read_text()
        if got_sig != want_sig or public.stdout != want_pub:
            print("sign-update.mjs drifted from the locked Ed25519 fixture")
            return 1
    print("update signature ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
