#!/usr/bin/env bash
# Build the engine for the browser into site/app/pkg (what app.omacus.com loads).
# Needs: rustup target add wasm32-unknown-unknown; cargo install wasm-bindgen-cli --version 0.2.129
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo build -p omacus-web --release --locked --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir site/app/pkg \
  target/wasm32-unknown-unknown/release/omacus_web.wasm
ls -l site/app/pkg
