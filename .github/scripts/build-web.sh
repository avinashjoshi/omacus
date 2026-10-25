#!/usr/bin/env bash
# Reproducible build of the web app. The hash is sha256 of the wasm the
# browser runs. Two builds of the same source, with the same toolchain, match.
#
#   .github/scripts/build-web.sh --into site
#   .github/scripts/build-web.sh --dist dist
set -euo pipefail

mode="dist"
out="dist"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --into)
      mode="site"
      out="site/app"
      shift
      # `--into site` names the tree. Anything else after the flag is a mistake.
      if [[ $# -gt 0 && "$1" == "site" ]]; then
        shift
      fi
      ;;
    --dist)
      mode="dist"
      out="${2:-dist}"
      shift
      if [[ $# -gt 0 && "$1" != --* ]]; then
        out="$1"
        shift
      fi
      ;;
    *)
      echo "usage: build-web.sh --into site | --dist [dir]" >&2
      exit 1
      ;;
  esac
done

cd "$(dirname "$0")/../.."

export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"
export CARGO_INCREMENTAL=0
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
sysroot="$(rustc --print sysroot)"
export RUSTFLAGS="--remap-path-prefix=${PWD}=/omacus --remap-path-prefix=${cargo_home}=/cargo --remap-path-prefix=${sysroot}=/rustc ${RUSTFLAGS:-}"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  cargo install wasm-bindgen-cli --version 0.2.129 --locked
fi

cargo build -p omacus-web --release --locked --target wasm32-unknown-unknown

if [[ "$mode" == "site" ]]; then
  mkdir -p site/app/pkg
  wasm-bindgen --target web --no-typescript --out-dir site/app/pkg \
    target/wasm32-unknown-unknown/release/omacus_web.wasm
  hash="$(sha256sum site/app/pkg/omacus_web_bg.wasm | awk '{print $1}')"
  printf '%s\n' "$hash" > site/app/build-hash.txt
  echo "$hash"
  exit 0
fi

rm -rf "$out"
mkdir -p "$out/pkg" "$out/css" "$out/fonts" "$out/img"
wasm-bindgen --target web --no-typescript --out-dir "$out/pkg" \
  target/wasm32-unknown-unknown/release/omacus_web.wasm
cp site/app/index.html site/app/app.js site/app/app.css site/app/editor.js \
  site/app/worker.js site/app/tree.js site/app/layer.js site/app/store.js site/app/sw.js \
  site/app/trust.js site/app/recover.js site/app/frame.js site/app/backup.js \
  site/app/seed.js site/app/examples.js site/app/sync.js site/app/device-name.js site/app/sync-save.mjs site/app/sync-leader.mjs site/app/sync-config.js \
  site/app/manifest.webmanifest site/app/migrate.js site/app/migrate.html \
  site/app/migrate-client.js site/app/migrate-protocol.js site/app/_headers "$out/"
cp site/css/site.css site/css/tokens.css "$out/css/"
cp site/fonts/*.woff2 "$out/fonts/"
cp site/img/icon.svg site/img/icon-192.png site/img/icon-512.png site/img/apple-touch-icon.png "$out/img/"

hash="$(sha256sum "$out/pkg/omacus_web_bg.wasm" | awk '{print $1}')"
printf '%s\n' "$hash" > "$out/build-hash.txt"
python3 - "$out/index.html" "$hash" <<'PY'
import pathlib, sys
path, digest = sys.argv[1], sys.argv[2]
file = pathlib.Path(path)
text = file.read_text()
needle = 'name="omacus-build" content="'
start = text.find(needle)
if start < 0:
    raise SystemExit("index.html has no omacus-build meta")
start += len(needle)
end = text.find('"', start)
file.write_text(text[:start] + digest + text[end:])
PY

for f in omacus_web_bg.wasm omacus_web.js; do
  short="$(sha256sum "$out/pkg/$f" | cut -c1-10)"
  sed -i "s#pkg/$f\"#pkg/$f?v=$short\"#g" "$out/app.js" "$out/worker.js"
done
short="$(sha256sum "$out/worker.js" | cut -c1-10)"
sed -i "s#\"./worker.js\"#\"./worker.js?v=$short\"#" "$out/app.js"
short="$(sha256sum "$out/editor.js" | cut -c1-10)"
sed -i "s#\"./editor.js\"#\"./editor.js?v=$short\"#" "$out/app.js"
short="$(sha256sum "$out/tree.js" | cut -c1-10)"
sed -i "s#\"./tree.js\"#\"./tree.js?v=$short\"#" "$out/app.js"
short="$(sha256sum "$out/layer.js" | cut -c1-10)"
sed -i "s#\"./layer.js\"#\"./layer.js?v=$short\"#" "$out/app.js"
short="$(sha256sum "$out/store.js" | cut -c1-10)"
sed -i "s#\"./store.js\"#\"./store.js?v=$short\"#" "$out/app.js" "$out/migrate.js"
short="$(sha256sum "$out/backup.js" | cut -c1-10)"
sed -i "s#\"./backup.js\"#\"./backup.js?v=$short\"#" "$out/app.js"
# examples.js is imported by seed.js, and seed.js by app.js. Hash the leaf first
# so the published import is the file that actually got uploaded.
short="$(sha256sum "$out/examples.js" | cut -c1-10)"
sed -i "s#\"./examples.js\"#\"./examples.js?v=$short\"#" "$out/seed.js"
short="$(sha256sum "$out/seed.js" | cut -c1-10)"
sed -i "s#\"./seed.js\"#\"./seed.js?v=$short\"#" "$out/app.js"
short="$(sha256sum "$out/migrate-protocol.js" | cut -c1-10)"
sed -i "s#\"./migrate-protocol.js\"#\"./migrate-protocol.js?v=$short\"#" "$out/migrate.js" "$out/migrate-client.js"
short="$(sha256sum "$out/app.css" | cut -c1-10)"
sed -i "s#\"app.css\"#\"app.css?v=$short\"#" "$out/index.html"
short="$(sha256sum "$out/trust.js" | cut -c1-10)"
sed -i "s#\"trust.js\"#\"trust.js?v=$short\"#" "$out/index.html"
short="$(sha256sum "$out/frame.js" | cut -c1-10)"
sed -i "s#\"frame.js\"#\"frame.js?v=$short\"#" "$out/index.html"
short="$(sha256sum "$out/recover.js" | cut -c1-10)"
sed -i "s#\"recover.js\"#\"recover.js?v=$short\"#" "$out/index.html"
python3 - "$out/sync-config.js" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old = 'export const SYNC_URL = "";'
new = 'export const SYNC_URL = "https://sync.omacus.com";'
if old not in text:
    raise SystemExit("sync-config.js has no empty SYNC_URL")
path.write_text(text.replace(old, new, 1))
PY
short="$(sha256sum "$out/device-name.js" | cut -c1-10)"
sed -i "s#\"./device-name.js\"#\"./device-name.js?v=$short\"#" "$out/sync.js"
short="$(sha256sum "$out/sync-config.js" | cut -c1-10)"
sed -i "s#\"./sync-config.js\"#\"./sync-config.js?v=$short\"#" "$out/sync.js"
short="$(sha256sum "$out/sync.js" | cut -c1-10)"
sed -i "s#\"./sync.js\"#\"./sync.js?v=$short\"#" "$out/app.js"
short="$(sha256sum "$out/app.js" | cut -c1-10)"
sed -i "s#\"app.js\"#\"app.js?v=$short\"#" "$out/index.html"

test -f "$out/_headers"
grep -q "wasm-unsafe-eval" "$out/_headers"
grep -q "require-trusted-types-for" "$out/_headers"
grep -q "open.er-api.com" "$out/_headers"
grep -q "https://sync.omacus.com" "$out/_headers"
grep -q 'https://sync.omacus.com' "$out/sync-config.js"
! grep -q "unsafe-inline" "$out/_headers"
! grep -q "1.0.0-beta" "$out/index.html"
grep -q "name=\"omacus-build\" content=\"$hash\"" "$out/index.html"
test -s "$out/seed.js"
test -s "$out/examples.js"
grep -q 'seed.js?v=' "$out/app.js"
grep -q 'examples.js?v=' "$out/seed.js"
echo "$hash"
