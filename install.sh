#!/usr/bin/env bash
# Install Omacus on Linux or macOS. No root needed.
#
#   curl -fsSL https://omacus.com/install | bash
#   ./install.sh                 from an unpacked release, or from a source checkout (builds it)
#
# On Omarchy/Hyprland it asks whether to add a keybind (default SUPER CTRL ALT + =, checked
# against your existing bindings) and a floating-window rule. Without a terminal, use:
#   --hyprland            add them with the default key
#   --key "SUPER ALT + N" add them with this key
#   --no-hyprland         don't touch the Hyprland config
set -euo pipefail

REPO="avinashjoshi/omacus"
BIN_DIR="$HOME/.local/bin"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"

hyprland=auto # auto: ask when there's a terminal; yes/no from flags
key=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --hyprland) hyprland=yes ;;
    --no-hyprland) hyprland=no ;;
    --key)
      key="${2:?--key needs a value, e.g. --key \"SUPER ALT + C\"}"
      hyprland=yes
      shift
      ;;
    -h | --help)
      sed -n '2,12p' "${BASH_SOURCE[0]:-$0}" 2>/dev/null | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
  shift
done

here="$(cd "$(dirname "${BASH_SOURCE[0]:-.}")" && pwd)"

# macOS: install Omacus.app from the release .dmg. Files downloaded with curl aren't
# quarantined, so Gatekeeper doesn't block the first launch.
install_macos() {
  local tmp mnt dest
  tmp="$(mktemp -d)"
  mnt="$tmp/mnt"
  trap 'hdiutil detach -quiet "$mnt" 2>/dev/null || true; rm -rf "$tmp"' EXIT
  echo "Downloading the latest Omacus for macOS…"
  curl -fsSL "https://github.com/$REPO/releases/latest/download/omacus-macos-universal.dmg" -o "$tmp/omacus.dmg"
  hdiutil attach -quiet -nobrowse -readonly -mountpoint "$mnt" "$tmp/omacus.dmg"
  dest=/Applications
  if [[ ! -w "$dest" ]]; then
    dest="$HOME/Applications"
    mkdir -p "$dest"
  fi
  rm -rf "$dest/Omacus.app"
  ditto "$mnt/Omacus.app" "$dest/Omacus.app"
  xattr -dr com.apple.quarantine "$dest/Omacus.app" 2>/dev/null || true
  mkdir -p "$BIN_DIR"
  ln -sf "$dest/Omacus.app/Contents/MacOS/omacus" "$BIN_DIR/omacus"
  echo "Installed Omacus to $dest/Omacus.app. Open it from Spotlight or Launchpad."
  echo "The command line tool is linked at $BIN_DIR/omacus."
}

if [[ "$(uname -s)" == Darwin && ! -f "$here/Cargo.toml" ]]; then
  install_macos
  exit 0
fi

if [[ -f "$here/omacus" && -f "$here/omacus.desktop" ]]; then
  # Unpacked release tarball.
  src="$here"
  bin="$src/omacus" desktop="$src/omacus.desktop" png="$src/icon-256.png" svg="$src/icon.svg"
elif [[ -f "$here/Cargo.toml" ]] && grep -q '^name = "omacus"' "$here/Cargo.toml"; then
  # Source checkout.
  echo "Building Omacus from source…"
  (cd "$here" && cargo build --release)
  if [[ "$(uname -s)" == Darwin ]]; then
    mkdir -p "$BIN_DIR"
    cp "$here/target/release/omacus" "$BIN_DIR/omacus"
    echo "Installed omacus to $BIN_DIR (for the .app, use the release .dmg)."
    exit 0
  fi
  bin="$here/target/release/omacus" desktop="$here/assets/omacus.desktop"
  png="$here/assets/icon-256.png" svg="$here/assets/icon.svg"
else
  # Piped from curl: fetch the latest release.
  if [[ "$(uname -m)" != "x86_64" ]]; then
    echo "Prebuilt binaries are x86_64 only. Build from source instead:" >&2
    echo "  git clone https://github.com/$REPO && cd omacus && ./install.sh" >&2
    exit 1
  fi
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  echo "Downloading the latest Omacus release…"
  curl -fsSL "https://github.com/$REPO/releases/latest/download/omacus-linux-x86_64.tar.gz" | tar -xz -C "$tmp"
  src="$tmp/omacus"
  bin="$src/omacus" desktop="$src/omacus.desktop" png="$src/icon-256.png" svg="$src/icon.svg"
fi

install -Dm755 "$bin" "$BIN_DIR/omacus"
install -Dm644 "$desktop" "$DATA_DIR/applications/omacus.desktop"
install -Dm644 "$png" "$DATA_DIR/icons/hicolor/256x256/apps/omacus.png"
install -Dm644 "$svg" "$DATA_DIR/icons/hicolor/scalable/apps/omacus.svg"
command -v update-desktop-database >/dev/null && update-desktop-database "$DATA_DIR/applications" 2>/dev/null || true
echo "Installed omacus to $BIN_DIR"

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "Note: $BIN_DIR is not on your PATH. Add it to run 'omacus' from a terminal." ;;
esac

# --- Omarchy / Hyprland ---------------------------------------------------------------

DEFAULT_KEY="SUPER CTRL ALT + EQUAL"
HYPR="$HOME/.config/hypr"

has_tty() { { : </dev/tty; } 2>/dev/null; }

# ask "Question" "default" -> reply (reads the terminal, so it works under curl | bash)
ask() {
  local reply
  read -r -p "$1" reply </dev/tty || reply=""
  echo "${reply:-$2}"
}

# "super+alt c" -> "SUPER ALT + C" (how `omarchy menu keybindings` prints it)
normalize_key() {
  local tok mods="" key="" m out=""
  for tok in ${1//+/ }; do
    case "${tok^^}" in
      SUPER | SHIFT | CTRL | ALT) mods+=" ${tok^^}" ;;
      CONTROL) mods+=" CTRL" ;;
      *) key="$tok" ;;
    esac
  done
  [[ -n "$key" ]] || return 1
  case "$key" in
    "=") key=EQUAL ;; "-") key=MINUS ;; ";") key=SEMICOLON ;; "'") key=APOSTROPHE ;;
    ".") key=PERIOD ;; ",") key=COMMA ;; "/") key=SLASH ;;
  esac
  [[ ${#key} -eq 1 || "$key" =~ ^[a-z]+$ ]] && key="${key^^}"
  for m in SUPER SHIFT CTRL ALT; do [[ "$mods " == *" $m "* ]] && out+="$m "; done
  echo "${out:+${out% } + }$key"
}

# "SUPER ALT + C" -> "SUPER + ALT + C" (the form hl.bind takes)
lua_key() { echo "${1// + / }" | sed 's/ / + /g'; }

# What a key does today, or nothing if it's free.
bound_to() {
  command -v omarchy >/dev/null || return 0
  omarchy menu keybindings --print 2>/dev/null |
    awk -v k="$1" 'toupper(substr($0, 1, length(k))) == toupper(k) && substr($0, length(k) + 1) ~ /^ +→/ {
      sub(/^[^→]*→ */, ""); print; exit }' || true # awk exits early; ignore the broken pipe
}

setup_hyprland() {
  [[ "$hyprland" == no ]] && return
  if [[ ! -f "$HYPR/bindings.lua" || ! -f "$HYPR/looknfeel.lua" ]]; then
    [[ "$hyprland" == yes ]] && echo "Hyprland: no Lua config in $HYPR (older Omarchy?). Bind a key to launch 'omacus' and float the window class 'omacus'."
    return
  fi
  if grep -q 'launch = "omacus"' "$HYPR/bindings.lua"; then
    echo "Hyprland: Omacus already has a keybind in bindings.lua, leaving your config alone."
    return
  fi

  local interactive=false
  if [[ "$hyprland" == auto ]]; then
    if ! has_tty; then
      echo "Hyprland: re-run with --hyprland (or --key \"SUPER ALT + N\") to add a keybind and float rule."
      return
    fi
    interactive=true
    echo
    case "$(ask "Add a Hyprland keybind and floating window for Omacus? [Y/n] " y)" in
      [nN]*) return ;;
    esac
  fi

  local chosen="${key:-$DEFAULT_KEY}" taken
  if $interactive && [[ -z "$key" ]]; then
    chosen="$(ask "Keybind [$DEFAULT_KEY]: " "$DEFAULT_KEY")"
  fi
  while true; do
    if ! chosen="$(normalize_key "$chosen")"; then
      taken="not a valid key"
    else
      taken="$(bound_to "$chosen")"
    fi
    [[ -z "$taken" || "$taken" == "Omacus calculator" ]] && break
    if ! $interactive; then
      echo "Hyprland: $chosen is already bound to \"$taken\". Pick another with --key \"...\"; no keybind added." >&2
      chosen=""
      break
    fi
    chosen="$(ask "$chosen is taken (\"$taken\"). Another key, or blank to skip: " "")"
    [[ -z "$chosen" ]] && break
  done

  # The dedicated calculator key, if the keyboard has one.
  local calc_key=false calc_now
  calc_now="$(bound_to "XF86Calculator")"
  if $interactive; then
    local prompt="Also open Omacus with the Calculator key"
    [[ -n "$calc_now" ]] && prompt+=" (it now opens \"$calc_now\")"
    case "$(ask "$prompt? [y/N] " n)" in [yY]*) calc_key=true ;; esac
  fi

  local stamp
  stamp="$(date +%s)"
  cp "$HYPR/bindings.lua" "$HYPR/bindings.lua.bak.$stamp"
  cp "$HYPR/looknfeel.lua" "$HYPR/looknfeel.lua.bak.$stamp"
  {
    echo
    echo "-- Omacus notepad calculator: launch or focus. Esc closes it; sheets are saved as you type."
    [[ -n "$chosen" ]] && echo "o.bind(\"$(lua_key "$chosen")\", \"Omacus calculator\", { launch = \"omacus\", focus = \"^omacus\$\" })"
    if $calc_key; then
      [[ -n "$calc_now" ]] && echo "hl.unbind(\"XF86Calculator\") -- was: $calc_now"
      echo "o.bind(\"XF86Calculator\", \"Omacus calculator\", { launch = \"omacus\", focus = \"^omacus\$\" })"
    fi
  } >>"$HYPR/bindings.lua"
  if ! grep -q '"^omacus\$"' "$HYPR/looknfeel.lua"; then
    printf '\n-- Omacus floats over whatever you'"'"'re doing.\no.window("^omacus$", { float = true, size = { 980, 600 }, center = true })\n' >>"$HYPR/looknfeel.lua"
  fi

  hyprctl reload >/dev/null 2>&1 || true
  local errors
  errors="$(hyprctl configerrors 2>/dev/null || true)"
  if [[ -n "${errors//[[:space:]]/}" ]]; then
    echo "Hyprland reported config errors; restoring your backups:" >&2
    echo "$errors" >&2
    mv "$HYPR/bindings.lua.bak.$stamp" "$HYPR/bindings.lua"
    mv "$HYPR/looknfeel.lua.bak.$stamp" "$HYPR/looknfeel.lua"
    hyprctl reload >/dev/null 2>&1 || true
    return 1
  fi
  [[ -n "$chosen" ]] && echo "Hyprland: $chosen opens Omacus."
  $calc_key && echo "Hyprland: the Calculator key opens Omacus."
  echo "Hyprland: Omacus floats, centered. Backups saved as *.bak.$stamp"
}

setup_hyprland
