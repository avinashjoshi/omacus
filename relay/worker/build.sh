#!/bin/sh
# Copies the test pages and the site's tokens, fonts and icon into ./public.
set -e
cd "$(dirname "$0")"
rm -rf public && mkdir -p public
cp -r ../www/. public/
cp ../../site/css/tokens.css public/tokens.css
cp -r ../../site/fonts public/fonts
cp -r ../../site/img public/img
printf '/*\n  Cache-Control: no-store\n' > public/_headers
