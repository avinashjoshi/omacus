#!/usr/bin/env bash
# Build and install Omacus for the current user on Linux (no root needed).
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release
install -Dm755 target/release/omacus "$HOME/.local/bin/omacus"
install -Dm644 assets/omacus.desktop "$HOME/.local/share/applications/omacus.desktop"
install -Dm644 assets/icon-256.png "$HOME/.local/share/icons/hicolor/256x256/apps/omacus.png"
install -Dm644 assets/icon.svg "$HOME/.local/share/icons/hicolor/scalable/apps/omacus.svg"
echo "Installed omacus to ~/.local/bin"
