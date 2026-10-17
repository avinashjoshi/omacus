#!/usr/bin/env bash
# Bundle CodeMirror (the web app's editor) into site/app/editor.js. The output is
# committed, so deploying the site needs no Node; rerun this after changing versions.
set -euo pipefail
cd "$(dirname "$0")"
[[ -d node_modules ]] || npm ci --silent --no-audit --no-fund
npx esbuild entry.js --bundle --format=esm --minify --legal-comments=eof \
  --outfile=../../app/editor.js
ls -l ../../app/editor.js
