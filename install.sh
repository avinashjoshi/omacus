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

# Everything runs from main, at the very end: if the download is cut short, bash
# never reaches the call, so a partial script does nothing.

REPO="avinashjoshi/omacus"
BIN_DIR="$HOME/.local/bin"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
CURL=(curl --proto '=https' --tlsv1.2 -fsSL)

TMP_DIR=""
MOUNT=""
cleanup() {
  if [[ -n "$MOUNT" ]]; then hdiutil detach -quiet "$MOUNT" 2>/dev/null || true; fi
  if [[ -n "$TMP_DIR" ]]; then rm -rf "$TMP_DIR"; fi
}
trap cleanup EXIT

# The folder this script was run from, only when it's a file on disk. Piped from curl
# there is no such folder, and the current directory is never trusted: a shared folder
# like /tmp could hold someone else's "release".
script_dir() {
  local src="${BASH_SOURCE[0]:-}"
  [[ -n "$src" && -f "$src" ]] || return 0
  cd "$(dirname "$src")" && pwd
}

# macOS: install Omacus.app from the release .dmg. Files downloaded with curl aren't
# quarantined, so Gatekeeper doesn't block the first launch.
install_macos() {
  local app new
  TMP_DIR="$(mktemp -d)"
  MOUNT="$TMP_DIR/mnt"
  # The chip, not the shell: a terminal under Rosetta reports x86_64 on Apple Silicon.
  local arch=x86_64
  [[ "$(sysctl -n hw.optional.arm64 2>/dev/null)" == 1 ]] && arch=arm64
  local dl="https://github.com/$REPO/releases/latest/download"
  echo "Downloading the latest Omacus for macOS ($arch)…"
  # Up to 0.1.7 there was one universal .dmg for both chips.
  "${CURL[@]}" "$dl/omacus-macos-$arch.dmg" -o "$TMP_DIR/omacus.dmg" 2>/dev/null ||
    "${CURL[@]}" "$dl/omacus-macos-universal.dmg" -o "$TMP_DIR/omacus.dmg"
  hdiutil attach -quiet -nobrowse -readonly -mountpoint "$MOUNT" "$TMP_DIR/omacus.dmg"

  if [[ -n "${OMACUS_APP:-}" ]]; then
    # An update from inside the app: replace that exact bundle, wherever it is.
    app="$OMACUS_APP"
  elif [[ -n "${OMACUS_APP_DIR:-}" ]]; then
    app="$OMACUS_APP_DIR/Omacus.app" # how 0.1.4 and 0.1.5 asked for it
  elif [[ -w /Applications ]]; then
    app=/Applications/Omacus.app
  else
    mkdir -p "$HOME/Applications"
    app="$HOME/Applications/Omacus.app"
  fi
  if [[ ! -w "$(dirname "$app")" ]]; then
    echo "No permission to install Omacus in $(dirname "$app"). Download it from https://omacus.com/download/#mac" >&2
    exit 1
  fi

  # Copy next to the old app first, so a failed copy never leaves no app at all.
  new="$(dirname "$app")/.Omacus-$$.app"
  rm -rf "$new"
  ditto "$MOUNT/Omacus.app" "$new"
  xattr -dr com.apple.quarantine "$new" 2>/dev/null || true
  rm -rf "$app"
  mv "$new" "$app"

  mkdir -p "$BIN_DIR"
  ln -sf "$app/Contents/MacOS/omacus" "$BIN_DIR/omacus"
  echo "Installed Omacus to $app. Open it from Spotlight or Launchpad."
  echo "The command line tool is linked at $BIN_DIR/omacus."
}

install_linux() {
  local here bin desktop png svg
  here="$(script_dir)"
  if [[ -n "$here" && -f "$here/omacus" && -f "$here/omacus.desktop" ]]; then
    # Unpacked release tarball.
    bin="$here/omacus" desktop="$here/omacus.desktop" png="$here/icon-256.png" svg="$here/icon.svg"
  elif [[ -n "$here" && -f "$here/Cargo.toml" ]] && grep -q '^name = "omacus"' "$here/Cargo.toml"; then
    # Source checkout.
    echo "Building Omacus from source…"
    (cd "$here" && cargo build --release)
    bin="$here/target/release/omacus" desktop="$here/assets/omacus.desktop"
    png="$here/assets/icon-256.png" svg="$here/assets/icon.svg"
  else
    # Piped from curl: fetch the latest release.
    if [[ "$(uname -m)" != "x86_64" ]]; then
      echo "Prebuilt binaries are x86_64 only. Build from source instead:" >&2
      echo "  git clone https://github.com/$REPO && cd omacus && ./install.sh" >&2
      exit 1
    fi
    TMP_DIR="$(mktemp -d)"
    echo "Downloading the latest Omacus release…"
    "${CURL[@]}" "https://github.com/$REPO/releases/latest/download/omacus-linux-x86_64.tar.gz" | tar -xz -C "$TMP_DIR"
    bin="$TMP_DIR/omacus/omacus" desktop="$TMP_DIR/omacus/omacus.desktop"
    png="$TMP_DIR/omacus/icon-256.png" svg="$TMP_DIR/omacus/icon.svg"
  fi

  install -Dm755 "$bin" "$BIN_DIR/omacus"
  install -Dm644 "$desktop" "$DATA_DIR/applications/omacus.desktop"
  install -Dm644 "$png" "$DATA_DIR/icons/hicolor/256x256/apps/omacus.png"
  install -Dm644 "$svg" "$DATA_DIR/icons/hicolor/scalable/apps/omacus.svg"
  if command -v update-desktop-database >/dev/null; then
    update-desktop-database "$DATA_DIR/applications" 2>/dev/null || true
  fi
  echo "Installed omacus to $BIN_DIR"

  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Note: $BIN_DIR is not on your PATH. Add it to run 'omacus' from a terminal." ;;
  esac
}

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

# "super+alt c" -> "SUPER ALT + C" (how `omarchy menu keybindings` prints it). Anything
# that isn't a plain modifier or key name is rejected, since it ends up in a Lua string.
normalize_key() {
  local tok mods="" key="" m out="" words
  read -r -a words <<<"${1//+/ }"
  for tok in "${words[@]}"; do
    case "${tok^^}" in
      SUPER | SHIFT | CTRL | ALT) mods+=" ${tok^^}" ;;
      CONTROL) mods+=" CTRL" ;;
      *) [[ -z "$key" ]] || return 1; key="$tok" ;;
    esac
  done
  [[ -n "$key" ]] || return 1
  case "$key" in
    "=") key=EQUAL ;; "-") key=MINUS ;; ";") key=SEMICOLON ;; "'") key=APOSTROPHE ;;
    ".") key=PERIOD ;; ",") key=COMMA ;; "/") key=SLASH ;;
  esac
  [[ ${#key} -eq 1 || "$key" =~ ^[a-z]+$ ]] && key="${key^^}"
  [[ "$key" =~ ^[A-Za-z0-9_]+$ ]] || return 1
  for m in SUPER SHIFT CTRL ALT; do [[ "$mods " == *" $m "* ]] && out+="$m "; done
  echo "${out:+${out% } + }$key"
}

# "SUPER ALT + C" -> "SUPER + ALT + C" (the form hl.bind takes)
lua_key() {
  local k="${1// + / }"
  echo "${k// / + }"
}

# What a key does today, or nothing if it's free.
bound_to() {
  command -v omarchy >/dev/null || return 0
  omarchy menu keybindings --print 2>/dev/null |
    awk -v k="$1" 'toupper(substr($0, 1, length(k))) == toupper(k) && substr($0, length(k) + 1) ~ /^ +→/ {
      sub(/^[^→]*→ */, ""); print; exit }' || true # awk exits early; ignore the broken pipe
}

setup_hyprland() {
  local hyprland="$1" key="$2"
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
      if [[ "$taken" == "not a valid key" ]]; then
        echo "Hyprland: \"${key:-$DEFAULT_KEY}\" isn't a valid key. Pick another with --key \"...\"; no keybind added." >&2
      else
        echo "Hyprland: $chosen is already bound to \"$taken\". Pick another with --key \"...\"; no keybind added." >&2
      fi
      chosen=""
      break
    fi
    chosen="$(ask "That key is taken or invalid (\"$taken\"). Another key, or blank to skip: " "")"
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
  [[ -z "$chosen" ]] && ! $calc_key && return

  # Errors that were there before we touched anything aren't ours to roll back for.
  local before stamp errors
  before="$(hyprctl configerrors 2>/dev/null || true)"
  stamp="$(date +%s)"
  cp "$HYPR/bindings.lua" "$HYPR/bindings.lua.bak.$stamp"
  cp "$HYPR/looknfeel.lua" "$HYPR/looknfeel.lua.bak.$stamp"
  {
    echo
    echo "-- Omacus notepad calculator: launch or focus. Esc closes it; sheets are saved as you type."
    [[ -n "$chosen" ]] && echo "o.bind(\"$(lua_key "$chosen")\", \"Omacus calculator\", { launch = \"omacus\", focus = \"^omacus\$\" })"
    if $calc_key; then
      [[ -n "$calc_now" ]] && echo "hl.unbind(\"XF86Calculator\") -- was: ${calc_now//[^[:alnum:] ._-]/}"
      echo "o.bind(\"XF86Calculator\", \"Omacus calculator\", { launch = \"omacus\", focus = \"^omacus\$\" })"
    fi
  } >>"$HYPR/bindings.lua"
  if ! grep -q '"^omacus\$"' "$HYPR/looknfeel.lua"; then
    printf '\n-- Omacus floats over whatever you'"'"'re doing.\no.window("^omacus$", { float = true, size = { 980, 600 }, center = true })\n' >>"$HYPR/looknfeel.lua"
  fi

  hyprctl reload >/dev/null 2>&1 || true
  errors="$(hyprctl configerrors 2>/dev/null || true)"
  if [[ "$errors" != "$before" && -n "${errors//[[:space:]]/}" ]]; then
    echo "Hyprland reported config errors; restoring your backups:" >&2
    echo "$errors" >&2
    # Copy (not move) back, so a symlinked config stays a symlink.
    cp "$HYPR/bindings.lua.bak.$stamp" "$HYPR/bindings.lua"
    cp "$HYPR/looknfeel.lua.bak.$stamp" "$HYPR/looknfeel.lua"
    hyprctl reload >/dev/null 2>&1 || true
    return 1
  fi
  [[ -n "$chosen" ]] && echo "Hyprland: $chosen opens Omacus."
  $calc_key && echo "Hyprland: the Calculator key opens Omacus."
  echo "Hyprland: Omacus floats, centered. Backups saved as *.bak.$stamp"
}

main() {
  local hyprland=auto key="" # auto: ask when there's a terminal; yes/no from flags
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
        echo "Install Omacus: curl -fsSL https://omacus.com/install | bash"
        echo "  --hyprland, --key \"SUPER ALT + N\", --no-hyprland   Hyprland keybind and float rule"
        exit 0
        ;;
      *)
        echo "Unknown option: $1" >&2
        exit 1
        ;;
    esac
    shift
  done

  if [[ "$(id -u)" == 0 ]]; then
    echo "Omacus installs for your user; run this without sudo." >&2
    exit 1
  fi

  local here
  here="$(script_dir)"
  if [[ "$(uname -s)" == Darwin ]]; then
    if [[ -n "$here" && -f "$here/Cargo.toml" ]] && grep -q '^name = "omacus"' "$here/Cargo.toml"; then
      echo "Building Omacus from source…"
      (cd "$here" && cargo build --release)
      mkdir -p "$BIN_DIR"
      cp "$here/target/release/omacus" "$BIN_DIR/omacus"
      echo "Installed omacus to $BIN_DIR (for the .app, use the release .dmg)."
    else
      install_macos
    fi
    return
  fi

  install_linux
  setup_hyprland "$hyprland" "$key"
}

main "$@"
