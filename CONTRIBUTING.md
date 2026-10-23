# Contributing

Issues and pull requests are welcome. Using an AI agent to write the change is fine. The same bar applies either way.

## Build and run

Install [Rust](https://rustup.rs), then:

```sh
git clone https://github.com/avinashjoshi/omacus
cd omacus
cargo run
```

`cargo build --release` matches how releases are built. On Linux the window needs the usual libraries (`libxkbcommon`, Wayland or X11, and GL).

## Tests

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build && python3 engine/tests/accuracy.py target/debug/omacus
```

The browser build is checked with:

```sh
cargo clippy -p omacus-web --target wasm32-unknown-unknown -- -D warnings
```

A wrong answer or a bug is fixed together with a test: a line in `engine/tests/cases.txt`, a unit test, or a check in `tests/`. Answers must not depend on today's date or the machine's speed.

## Pull requests

`main` is protected. Open a pull request. It merges only after the checks pass on Linux, macOS, and Windows, and for the browser build.

## Design

Read [`docs/DESIGN.md`](docs/DESIGN.md) before changing the desktop app, the web app, or the website. Colors, type, spacing, and radii come from `src/theme.rs` and `site/css/tokens.css`.

## Notes for people using Omacus

If people will notice the change, add it under `## Unreleased` in [`CHANGELOG.md`](CHANGELOG.md): a short title in bold, then one or two sentences for someone using Omacus. The voice is in [`docs/DESIGN.md`](docs/DESIGN.md).

Those notes stay free of pull request numbers and of how the build is run. A small fix can add `<!-- minor -->`. A release with nothing new in the app can add `<!-- internal -->`.

Examples use made-up pages: Trip Budget, Groceries, `@trip.total`.
