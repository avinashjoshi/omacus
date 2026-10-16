#!/usr/bin/env bash
# Install Omacus for the current user on Linux. No root needed.
#
#   curl -fsSL https://github.com/avinashjoshi/omacus/releases/latest/download/install.sh | bash
#   curl -fsSL …/install.sh | bash -s -- --hyprland     # Omarchy: also add SUPER+ALT+= and a float rule
#
#   ./install.sh [--hyprland]    from an unpacked release, or from a source checkout (builds it)
set -euo pipefail

REPO="avinashjoshi/omacus"
BIN_DIR="$HOME/.local/bin"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"

hyprland=false
for arg in "$@"; do
  case "$arg" in
    --hyprland) hyprland=true ;;
    -h | --help)
      sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown option: $arg" >&2
      exit 1
      ;;
  esac
done

here="$(cd "$(dirname "${BASH_SOURCE[0]:-.}")" && pwd)"

if [[ -f "$here/omacus" && -f "$here/omacus.desktop" ]]; then
  # Unpacked release tarball.
  src="$here"
  bin="$src/omacus" desktop="$src/omacus.desktop" png="$src/icon-256.png" svg="$src/icon.svg"
elif [[ -f "$here/Cargo.toml" ]] && grep -q '^name = "omacus"' "$here/Cargo.toml"; then
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

# Omarchy / Hyprland: a launch-or-focus keybind and a floating window rule.
setup_hyprland() {
  local hypr="$HOME/.config/hypr" stamp
  stamp="$(date +%s)"
  if [[ -f "$hypr/bindings.lua" && -f "$hypr/looknfeel.lua" ]]; then
    if grep -q 'omacus' "$hypr/bindings.lua" "$hypr/looknfeel.lua"; then
      echo "Hyprland: Omacus is already configured, leaving your config alone."
      return
    fi
    cp "$hypr/bindings.lua" "$hypr/bindings.lua.bak.$stamp"
    cp "$hypr/looknfeel.lua" "$hypr/looknfeel.lua.bak.$stamp"
    cat >>"$hypr/bindings.lua" <<'LUA'

-- Omacus notepad calculator: launch or focus. Esc closes it; sheets are saved as you type.
o.bind("SUPER + ALT + EQUAL", "Omacus calculator", { launch = "omacus", focus = "^omacus$" })
LUA
    cat >>"$hypr/looknfeel.lua" <<'LUA'

-- Omacus floats over whatever you're doing.
o.window("^omacus$", { float = true, size = { 980, 600 }, center = true })
LUA
    hyprctl reload >/dev/null 2>&1 || true
    local errors
    errors="$(hyprctl configerrors 2>/dev/null || true)"
    if [[ -n "${errors//[[:space:]]/}" ]]; then
      echo "Hyprland reported config errors; restoring your backups:" >&2
      echo "$errors" >&2
      mv "$hypr/bindings.lua.bak.$stamp" "$hypr/bindings.lua"
      mv "$hypr/looknfeel.lua.bak.$stamp" "$hypr/looknfeel.lua"
      hyprctl reload >/dev/null 2>&1 || true
      return 1
    fi
    echo "Hyprland: SUPER + ALT + = opens Omacus (backups saved as *.bak.$stamp)."
  else
    echo "Hyprland: no Lua config found (older Omarchy?). Add a keybind to launch 'omacus'"
    echo "and a float rule for the window class 'omacus' to your Hyprland config."
  fi
}

if $hyprland; then
  setup_hyprland
fi
