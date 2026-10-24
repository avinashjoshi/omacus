#!/bin/sh
# Build the private test app into ./public.
# The worker serves that app at / and the sealed mailbox at /v1.
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
cp ../../site/app/*.js ../../site/app/*.css ../../site/app/*.html ../../site/app/*.webmanifest public/
cp ../../site/css/tokens.css ../../site/css/site.css public/css/
cp -r ../../site/fonts public/fonts
cp -r ../../site/img public/img
cp ../www/words.txt public/words.txt
python3 - <<'PY'
from pathlib import Path
page = Path("public/index.html")
text = page.read_text()
text = text.replace('<html lang="en">', '<html lang="en" data-sync-test="1">', 1)
text = text.replace("https://app.omacus.com/", "https://sync-test.omacus.com/")
page.write_text(text)
manifest = Path("public/manifest.webmanifest")
body = manifest.read_text()
body = body.replace('"name": "Omacus"', '"name": "Omacus sync test"', 1)
body = body.replace('"short_name": "Omacus"', '"short_name": "Omacus sync"', 1)
manifest.write_text(body)
PY
printf '/*\n  Cache-Control: no-store\n' > public/_headers
echo "Built relay/worker/public"
