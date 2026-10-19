# Working on Omacus

Notes for anyone (people or agents) changing this repo.

## Examples from bug reports

Bug reports often come with someone's own sheet. In tests, commits and PRs, use
made-up names and numbers that show the same bug.

## Every bug gets a test

A wrong answer or a bug is fixed together with a test that fails without the
fix: a line in `engine/tests/cases.txt` (see the notes at its top), a unit test,
or a check in `tests/`. Answers mustn't depend on today's date or the machine's
speed.

## Before a PR

    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy -p omacus-web --target wasm32-unknown-unknown -- -D warnings
    cargo test --workspace
    cargo build && python3 engine/tests/accuracy.py target/debug/omacus

To try the web app (`site/app`), build the engine for it first:

    cargo build -p omacus-web --release --target wasm32-unknown-unknown
    wasm-bindgen --target web --no-typescript --out-dir site/app/pkg target/wasm32-unknown-unknown/release/omacus_web.wasm

Bumping `version` in `Cargo.toml` on `main` releases it (see
`.github/workflows/release.yml`).
