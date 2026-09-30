# Contributing

## Setup

```sh
brew install prek just typos-cli committed tombi cargo-nextest cargo-llvm-cov \
  cargo-deny cargo-shear cargo-hack zizmor actionlint markdownlint-cli2
prek install
```

The Rust toolchain is pinned in `rust-toolchain.toml`; rustup picks it up.

## Before you push

```sh
just ci
```

That runs everything CI runs: format, clippy, rustdoc, tests, cargo-deny,
cargo-shear, the MSRV check, typos, markdownlint, workflow linting and the
coverage floor. The pre-push hook runs clippy and the tests.

## Commits and pull requests

- One-line conventional commits: `feat: clone package dirs from the store`.
  The commit-msg hook checks this.
- One change per pull request. The PR title follows the same format, because
  it becomes the commit on `main` when squashed.
- Output that must match Composer is tested as bytes, not snapshots. See
  `crates/phpm-testkit`.

Full rules: [conventions](phases/phase-00-foundation/conventions.md).

## Licence

By contributing you agree that your work is dual licensed under MIT OR
Apache-2.0, like the rest of the project.
