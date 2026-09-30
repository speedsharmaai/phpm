# Phase 00 tasks

Versions are the ones checked on 2026-10-01 in
[quality tooling](../../docs/research/quality-tooling.md). Re-check before
pinning; bump the doc if any moved.

## Owner actions

- [ ] SonarQube Cloud: sign in with the speedsharmaai GitHub account, create the organisation (free, public project), import `phpm`, add `SONAR_TOKEN` as a repo secret.
- [ ] Register at bestpractices.dev for the OpenSSF Best Practices badge (free).
- [ ] Decide the Git signing key (SSH signing with `~/.ssh/speedsharma` is the easy path) and add it to GitHub as a signing key.
- [x] Repo settings: squash merge only, auto-delete head branches, Dependabot alerts on.

## Workspace

- [x] `rust-toolchain.toml`: channel `1.98.1`, components rustfmt, clippy, llvm-tools-preview, profile minimal. Confirm the version exists with `rustup`.
- [x] Root `Cargo.toml`: resolver 3, `members = ["crates/*"]`, `[workspace.package]` (edition 2024, `rust-version = "1.96"`, `license = "MIT OR Apache-2.0"`, repository).
- [x] `[workspace.lints.rust]`, `[workspace.lints.rustdoc]`, `[workspace.lints.clippy]` as in the research doc.
- [x] `clippy.toml`: unwrap/expect/print allowed in tests; `disallowed-methods` for `std::env::set_var` and `std::process::exit`.
- [x] `rustfmt.toml`: stable options only.
- [x] `crates/phpm`: binary printing `phpm <version>`, `[lints] workspace = true`, `publish = false` for now, one test.
- [x] `Cargo.lock` committed.

## Hooks and commits

- [x] `prek.toml`: builtin whitespace, end-of-file, merge-conflict, private-key, yaml and large-file hooks; typos; committed on commit-msg; zizmor; local cargo-fmt and tombi; clippy and nextest on pre-push. Fixtures and snapshots excluded.
- [x] Verify the prek field names and builtin hook ids against the prek docs (flagged uncertain in research).
- [x] `committed.toml`: conventional style, 72 chars, lowercase subjects, the allowed types from [conventions](conventions.md), no merge commits.
- [x] `_typos.toml` with any false positives (Composer terms like `psr`, `autoload`).
- [x] `.markdownlint.jsonc` tuned so the existing docs pass without rewriting them.
- [x] `CONTRIBUTING.md` first step: `brew install prek just && prek install`.

## Tests and coverage

- [x] `phpm-testkit` crate (unpublished) with a byte-level comparison helper and its own tests.
- [ ] insta, proptest, criterion: added by the first Phase 01 code that uses them. Adding them now fails cargo-shear as unused.
- [x] `.config/nextest.toml`: CI profile with JUnit output and one retry.
- [x] Coverage: `cargo llvm-cov nextest --lcov --output-path lcov.info --fail-under-lines 80`.

## Supply chain

- [x] `deny.toml`: advisories (yanked deny), licence allow-list, bans (`openssl-sys` denied, wildcards denied), sources (crates.io only). `unmaintained` left at its default until there are dependencies to judge it on.
- [x] `.github/dependabot.yml`: cargo and github-actions, weekly, grouped, cooldown 7 days / 30 for majors.
- [x] `SECURITY.md`: how to report, supported versions, response time.

## Task runner

- [x] `justfile`: `fmt`, `fmt-check`, `lint`, `docs`, `test`, `cov`, `deny`, `shear`, `msrv`, `typos`, `ci` (all of them in CI order).

## CI (`.github/workflows/`)

- [x] `ci.yml`: `permissions: {}`, concurrency cancel-in-progress, checkout with `persist-credentials: false`, Swatinem/rust-cache, taiki-e/install-action for tools. Jobs:
  - [x] fmt (rustfmt, tombi)
  - [x] clippy on ubuntu and windows, `-D warnings`
  - [x] docs, `RUSTDOCFLAGS=-D warnings`
  - [x] test on ubuntu, macos, windows (nextest + doctests, `INSTA_UPDATE=no`)
  - [x] coverage on ubuntu, floor 80%, upload `lcov.info` as an artifact
  - [x] msrv via cargo-hack
  - [x] deny, shear, typos, markdownlint, zizmor, actionlint (a `hygiene` job; prek itself stays local, CI runs the same tools directly)
  - [x] ci-ok aggregate (re-actions/alls-green)
- [x] `pr-title.yml`: `on: pull_request` (not `pull_request_target`), amannn/action-semantic-pull-request pinned by SHA.
- [x] `sonar.yml`: after coverage, `cargo clippy --message-format=json > clippy.json`, SonarSource/sonarqube-scan-action with `sonar.rust.lcov.reportPaths` and `sonar.rust.clippyReport.reportPaths`; the quality gate is a required check.
- [x] `release-plz.yml` and `release-plz.toml`: release PRs from conventional commits; `cliff.toml` for the changelog. Dry-run only until Phase 05.
- [ ] ~~`dist-workspace.toml` via `dist init`~~ moved to Phase 05: the generated release workflow is not SHA-pinned and needs hand edits, and nothing is released before then.
- [x] Every `uses:` pinned by full SHA with a version comment; zizmor clean.

## Repo hygiene

- [x] `LICENSE-MIT`, `LICENSE-APACHE`.
- [ ] `NOTICE` crediting Composer (MIT): with the first vendored Composer file in Phase 01.
- [x] `.editorconfig`, `.gitattributes` (`* text=auto eol=lf`, fixtures and snapshots `-text`).
- [x] `.github/CODEOWNERS`, issue forms (bug, feature), PR template (what, why, how tested).
- [x] README badges: CI, Sonar gate, coverage, Scorecard, licence.

## Public-repo security

- [x] Repo made public (2026-10-01, decision 0009).
- [x] `codeql.yml`: languages rust and actions, on push, PR and weekly.
- [x] `scorecard.yml`: ossf/scorecard-action, publish results, SARIF upload.
- [x] Settings: private vulnerability reporting, secret scanning, push protection, Dependabot alerts and security updates.
- [x] Ruleset on `main` via `gh api`: PR required, required checks `ci-ok`, PR title (Sonar added once the token exists); linear history; block force push and deletion; signed commits; owner bypass. Applied after the first green CI run so the check names exist.
- [ ] `release.yml` (dist) with `actions/attest`: Phase 05, with dist.

## Proof

- [x] Throwaway branch with a clippy warning, an unformatted file, a GPL dependency, an unused dependency and a bad commit message: each fails where it should. Branch deleted.
- [x] `just ci` green from a fresh clone.
- [x] CI green on all three OSes (PR #1).
- [ ] Sonar dashboard populated: waits on `SONAR_TOKEN`.
- [x] `progress.md` written with what was checked and what surprised.
