#!/usr/bin/env bash
# Build the launch video end to end. See README.md.
set -euo pipefail
cd "$(dirname "$0")"
[[ -d node_modules ]] || npm ci --silent --no-audit --no-fund
[[ -d .venv ]] || { python3 -m venv .venv && .venv/bin/pip install -q numpy==2.1.3; }
(cd .. && cargo build --release -q)
mkdir -p build
node data.mjs >/dev/null                  # every answer, from the real engine
if [[ -s ~/.config/elevenlabs/api_key ]]; then
  node voice.mjs                          # narration (cached; only changed lines cost credits)
else
  echo "No ElevenLabs key at ~/.config/elevenlabs/api_key; building without narration."
fi
node render.mjs                           # 1080p60 frames -> build/video.mp4
.venv/bin/python audio.py                 # music bed and sound effects
node mux.mjs                              # mix -> out/
