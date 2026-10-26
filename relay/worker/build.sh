#!/bin/sh
# Build the worker assets into ./public.
# Production serves the admin at /admin and the sealed mailbox at /v1.
# It does not serve the app at /. The built copy talks to the production sync host.
# The source file stays empty.
set -e
cd "$(dirname "$0")"
root="../.."
rm -rf public
mkdir -p public/pkg public/css

cd "$root"
cargo build -p omacus-web --release --target wasm32-unknown-unknown
cargo build -p omacus-sync-web --release --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir relay/worker/public/pkg-web \
  target/wasm32-unknown-unknown/release/omacus_web.wasm
wasm-bindgen --target web --no-typescript --out-dir relay/worker/public/pkg-sync \
  target/wasm32-unknown-unknown/release/omacus_sync_web.wasm

cd relay/worker
cp public/pkg-web/* public/pkg-sync/* public/pkg/
rm -rf public/pkg-web public/pkg-sync
cp ../../site/app/*.js ../../site/app/*.mjs ../../site/app/*.css ../../site/app/*.html ../../site/app/*.webmanifest public/
mkdir -p public/admin
cp admin/index.html admin/admin.css admin/admin.js admin/when.js public/admin/
cp ../../site/css/tokens.css ../../site/css/site.css public/css/
cp -r ../../site/fonts public/fonts
cp -r ../../site/img public/img
cp ../www/words.txt public/words.txt
python3 - <<'PY'
from pathlib import Path
config = Path("public/sync-config.js")
text = config.read_text()
old = 'export const SYNC_URL = "";'
new = 'export const SYNC_URL = "https://sync.omacus.com";'
if old not in text:
    raise SystemExit("sync-config.js has no empty SYNC_URL")
config.write_text(text.replace(old, new, 1))
headers = Path("../../site/app/_headers").read_text()
if "Content-Security-Policy" not in headers or "https://sync.omacus.com" not in headers:
    raise SystemExit("build lost the sync content security policy")
if "https://sync-test.omacus.com" in headers:
    raise SystemExit("source headers name a retired sync host")
if "Cache-Control:" not in headers:
    headers = headers.rstrip() + "\n  Cache-Control: no-store\n"
Path("public/_headers").write_text(headers if headers.endswith("\n") else headers + "\n")
PY
echo "Built relay/worker/public"
