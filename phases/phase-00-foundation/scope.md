# Phase 00: Foundation

## Why

Quality tooling is cheapest on an empty repository. Lints, hooks, commit
rules and CI added after the spike would mean fixing a hundred warnings in
code that was written without them, and arguing with every one.

phpm also has a quality problem most projects do not: its output must match
Composer's byte for byte. That shapes the tooling, not just the tests.
Whitespace fixers must never touch fixtures or snapshots. Line endings must
not change on checkout. HashMap iteration order must never reach output.
Those rules go in now, before there is a single fixture to damage.

And this is the template. Every tool in `devtools/` starts with this phase,
so it is written to be copied.

## What ships

An empty Cargo workspace (one `phpm` binary crate printing its version) that
passes every check below, locally and in CI.

### Code quality

- `rust-toolchain.toml` pinned; edition 2024; `rust-version` N-2.
- rustfmt (stable options), clippy via `[workspace.lints]` with `pedantic`,
  `unsafe_code = "forbid"`, `iter_over_hash_type`, `print_stdout`, `unwrap_used`.
  Warnings are errors in CI.
- rustdoc lints, `RUSTDOCFLAGS=-D warnings`.
- typos, tombi (TOML), markdownlint.

### Git hooks and commits (the Husky + commitlint equivalent)

- prek: pre-commit (fmt, typos, tombi, whitespace, private-key check),
  commit-msg (committed, conventional commits), pre-push (clippy, tests).
- One-line conventional commit subjects. No body required, no co-author lines.
  See [conventions](conventions.md).
- PR title check; squash merge only, so the PR title is the commit on `main`.

### Tests and coverage

- cargo-nextest, insta for readable snapshots, byte-level golden comparisons
  for anything that must match Composer, proptest for parsers.
- cargo-llvm-cov with a line-coverage floor, starting at 80%.
- criterion for micro-benchmarks; hyperfine stays the end-to-end tool.

### Supply chain and security

- cargo-deny: licences, bans, sources, RustSec advisories.
- cargo-shear: unused dependencies.
- cargo-hack: MSRV check.
- Dependabot for Cargo and GitHub Actions, grouped, with a cooldown.
- Every action pinned by SHA; zizmor and actionlint on workflows;
  `permissions: {}` by default.
- SECURITY.md.

### Sonar

- SonarQube Cloud (free for public repos), fed with clippy JSON and LCOV
  coverage. Its quality gate is a required check. See
  [decision 0008](../../docs/decisions/0008-quality-gates-before-code.md).

### Public-repo security (free because the repo is public, decision 0009)

- CodeQL for Rust and for GitHub Actions.
- OpenSSF Scorecard, with its badge in the README.
- A ruleset on `main`: PR required, `ci-ok`, PR title and Sonar required,
  linear history, no force push, no deletion, signed commits; owner bypass.
- Private vulnerability reporting, secret scanning and push protection,
  Dependabot alerts.
- Build provenance attestations wired into the release workflow.

### Release plumbing (configured, not used until Phase 05)

- release-plz for version PRs and a git-cliff changelog from conventional
  commits.
- cargo-dist is set up in Phase 05, when there is something to release.

### Repo hygiene

- `LICENSE-MIT`, `LICENSE-APACHE`, CONTRIBUTING.md, CODEOWNERS,
  `.editorconfig`, `.gitattributes`, issue forms, PR template.
- justfile: `just fmt`, `just lint`, `just test`, `just cov`, `just deny`,
  `just ci`. CI runs the same recipes.
- README badges: CI, Sonar quality gate, coverage, Scorecard, licence.

## Exit criteria

- `just ci` passes locally from a fresh clone on the Mac.
- CI green on Linux, macOS and Windows, with one aggregate `ci-ok` check.
- A commit with a bad message is rejected by the hook and by CI.
- A PR with a bad title fails the title check.
- A deliberate clippy warning, an unformatted file, a GPL dependency and an
  unused dependency each fail CI (tried on a throwaway branch, then removed).
- Sonar shows the project with coverage and clippy issues imported, and its
  gate is green.
- CodeQL and Scorecard have run once; Scorecard findings triaged.
- A direct push to `main` is rejected by the ruleset.
- Dependabot has opened its first PR or confirmed nothing is outdated.
- `docs/research/quality-tooling.md` records every version pinned.

## Out of scope

Any phpm feature. Fixtures, benchmarks against Composer, the store, the
autoloader: all Phase 01. Publishing a crate or a release; that is Phase 05.
