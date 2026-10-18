#!/usr/bin/env bash
# Renders the installer art from installer/art/*.svg, in the site's own fonts:
#   - wizard-*.png: the Windows setup's side panel, at each display scale Inno uses
#   - small-*.png:  its header icon
#   - dmg-background.png (+ @2x): the macOS .dmg window
# Needs rsvg-convert and Python with fonttools and brotli. The PNGs are committed;
# run this after changing an SVG or the icon.
set -euo pipefail
cd "$(dirname "$0")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
for f in inter-tight-latin inter-latin instrument-serif-italic-latin; do
  python3 -c "from fontTools.ttLib import TTFont; f = TTFont('../site/fonts/$f.woff2'); f.flavor = None; f.save('$tmp/$f.ttf')"
done
cat > "$tmp/fonts.conf" <<CONF
<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig><include ignore_missing="yes">/etc/fonts/fonts.conf</include><dir>$tmp</dir><cachedir>$tmp/cache</cachedir></fontconfig>
CONF
export FONTCONFIG_FILE="$tmp/fonts.conf"
icon="$(python3 -c "import re; s = open('../assets/icon.svg').read(); print(re.sub(r'^.*?<svg[^>]*>', '', s, flags=re.S).rsplit('</svg>', 1)[0])")"
inline() { python3 -c 'import sys; s = open(sys.argv[1]).read(); print(s.replace("<!--ICON-->", sys.argv[2]))' "$1" "$icon"; }
inline art/wizard.svg > "$tmp/wizard.svg"
printf '<svg xmlns="http://www.w3.org/2000/svg" width="55" height="55" viewBox="0 0 512 512">%s</svg>' "$icon" > "$tmp/small.svg"
for z in 100 125 150 175 200 225 250; do
  rsvg-convert -z "$(awk "BEGIN { print $z / 100 }")" "$tmp/wizard.svg" -o "wizard-$z.png"
  rsvg-convert -z "$(awk "BEGIN { print $z / 100 }")" "$tmp/small.svg" -o "small-$z.png"
done
rsvg-convert art/dmg.svg -o dmg-background.png
rsvg-convert -z 2 art/dmg.svg -o dmg-background@2x.png
