#!/usr/bin/env bash
# Regenerate the website screenshots in site/img/screens (WebP; needs ImageMagick).
#
# Each shot runs Omacus against throwaway data, config and cache folders (your own
# sheets are never touched) with OMACUS_SCREENSHOT set, so the app saves its own
# rendered pixels and quits; windows behind it never end up in the image.
#
#   site/tools/screenshots.sh            needs Omarchy's themes in /usr/share/omarchy/themes
set -euo pipefail
cd "$(dirname "$0")/../.."

cargo build --release -q
bin="$PWD/target/release/omacus"
out="$PWD/site/img/screens"
demo="$PWD/site/tools/demo"
themes=/usr/share/omarchy/themes
mkdir -p "$out"

# shoot <name> <omarchy-theme> <active-sheet> [help-tab]
shoot() {
  local name=$1 theme=$2 active=$3 tab=${4:-} root
  root="$(mktemp -d)"
  mkdir -p "$root/data/omacus/pages" "$root/cfg/omacus" "$root/cache/omacus"
  cp "$demo"/*.txt "$root/data/omacus/pages/"
  printf '{"order":["kyoto","invoice","standup","budget","kitchen"],"active":"%s","cursors":{},"sidebar":true}' \
    "$active" >"$root/data/omacus/state.json"
  cp "$themes/$theme/colors.toml" "$root/cfg/omacus/theme.toml"
  cp ~/.cache/omacus/rates.json "$root/cache/omacus/" 2>/dev/null || true
  env XDG_DATA_HOME="$root/data" XDG_CONFIG_HOME="$root/cfg" XDG_CACHE_HOME="$root/cache" \
    OMACUS_SCREENSHOT="$out/$name.png" ${tab:+OMACUS_SCREENSHOT_HELP=$tab} \
    timeout 20 "$bin"
  rm -rf "$root"
  magick "$out/$name.png" -quality 88 "$out/$name.webp" && rm "$out/$name.png"
  echo "$name.webp"
}

shoot matte-black matte-black kyoto
shoot tokyo-night tokyo-night invoice
shoot catppuccin catppuccin budget
shoot gruvbox gruvbox standup
shoot everforest everforest kitchen
shoot nord nord kyoto
shoot rose-pine rose-pine invoice
shoot catppuccin-latte catppuccin-latte budget
shoot help-syntax matte-black kyoto syntax
shoot help-keyboard matte-black kyoto keyboard
