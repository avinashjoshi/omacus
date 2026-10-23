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

## Design

Read `docs/DESIGN.md` before changing the desktop app, the web app, or the website.
Colors, type, spacing, and radii come from `src/theme.rs` and `site/css/tokens.css`.
When you add a component, add it to the guide. A UI pull request includes before
and after screenshots.

## Before a PR

    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy -p omacus-web --target wasm32-unknown-unknown -- -D warnings
    cargo test --workspace
    cargo build && python3 engine/tests/accuracy.py target/debug/omacus

To try the web app (`site/app`), build the engine for it first:

    cargo build -p omacus-web --release --target wasm32-unknown-unknown
    wasm-bindgen --target web --no-typescript --out-dir site/app/pkg target/wasm32-unknown-unknown/release/omacus_web.wasm

Releasing does not push straight to `main`. Write the notes first, under
`## Unreleased` in `CHANGELOG.md`: a short title in bold, then one or two
sentences for someone using Omacus (see Voice in `docs/DESIGN.md`). Run the
Release workflow with action `prepare` (see `.github/workflows/release.yml`).
It opens a pull request that bumps `version` in `Cargo.toml` and renames that
section to the version. With auto-merge allowed and the CI checks required on
`main`, that pull request merges when the checks pass, and the workflow then
tags `v<version>` and publishes the GitHub release from those notes. The checks
to require are `test (ubuntu-22.04)`, `test (windows-latest)`,
`test (macos-latest)`, and `wasm`.

If `## Unreleased` is missing, the pull request drafts notes from commit
subjects, and CI rejects them (`tests/check_release_notes.py`). Notes that are
only commit titles, or that mention pull requests, CI, or other plumbing, fail
that check. A small fix can add `<!-- minor -->`, and a release with nothing
new in the app can add `<!-- internal -->`, so the updates page folds it under
the next entry. The monthly example refresh opens a pull request with
auto-merge (`.github/workflows/freshen.yml`) instead of pushing to `main`.
