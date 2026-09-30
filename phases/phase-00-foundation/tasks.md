# Phase 00 tasks

Versions are the ones checked on 2026-10-01 in
[quality tooling](../../docs/research/quality-tooling.md). Re-check before
pinning; bump the doc if any moved.

## Owner actions

- [ ] SonarQube Cloud: sign in with the speedsharmaai GitHub account, create the organisation (free, public project), import `phpm`, add `SONAR_TOKEN` as a repo secret.
- [ ] Decide the Git signing key (SSH signing with `~/.ssh/speedsharma` is the easy path) and add it to GitHub as a signing key.
- [ ] Repo settings: squash merge only, auto-delete head branches, Dependabot alerts on.

## Workspace

- [ ] `rust-toolchain.toml`: channel `1.98.1`, components rustfmt, clippy, llvm-tools-preview, profile minimal. Confirm the version exists with `rustup`.
- [ ] Root `Cargo.toml`: resolver 3, `members = ["crates/*"]`, `[workspace.package]` (edition 2024, `rust-version = "1.96"`, `license = "MIT OR Apache-2.0"`, repository).
- [ ] `[workspace.lints.rust]`, `[workspace.lints.rustdoc]`, `[workspace.lints.clippy]` as in the research doc.
- [ ] `clippy.toml`: unwrap/expect/print allowed in tests; `disallowed-methods` for `std::env::set_var` and `std::process::exit`.
- [ ] `rustfmt.toml`: stable options only.
- [ ] `crates/phpm`: binary printing `phpm <version>`, `[lints] workspace = true`, `publish = false` for now, one test.
- [ ] `Cargo.lock` committed.

## Hooks and commits

- [ ] `prek.toml`: builtin whitespace, end-of-file, merge-conflict, private-key, yaml and large-file hooks; typos; committed on commit-msg; zizmor; local cargo-fmt and tombi; clippy and nextest on pre-push. Fixtures and snapshots excluded.
- [ ] Verify the prek field names and builtin hook ids against the prek docs (flagged uncertain in research).
- [ ] `committed.toml`: conventional style, 72 chars, lowercase subjects, the allowed types from [conventions](conventions.md), no merge commits.
- [ ] `_typos.toml` with any false positives (Composer terms like `psr`, `autoload`).
- [ ] `.markdownlint.jsonc` tuned so the existing docs pass without rewriting them.
- [ ] `CONTRIBUTING.md` first step: `brew install prek just && prek install`.

## Tests and coverage

- [ ] Dev dependencies: insta, proptest, criterion. A `phpm-testkit` crate (unpublished) with a byte-level golden-file helper.
- [ ] `.config/nextest.toml`: CI profile with JUnit output and one retry.
- [ ] Coverage: `cargo llvm-cov nextest --lcov --output-path lcov.info --fail-under-lines 80`.

## Supply chain

- [ ] `deny.toml`: advisories (yanked deny), licence allow-list, bans (`openssl-sys` denied, wildcards denied), sources (crates.io only). Verify the `unmaintained` scope syntax.
- [ ] `.github/dependabot.yml`: cargo and github-actions, weekly, grouped, cooldown 7 days / 30 for majors.
- [ ] `SECURITY.md`: how to report, supported versions, response time.

## Task runner

- [ ] `justfile`: `fmt`, `fmt-check`, `lint`, `docs`, `test`, `cov`, `deny`, `shear`, `msrv`, `typos`, `ci` (all of them in CI order).

## CI (`.github/workflows/`)

- [ ] `ci.yml`: `permissions: {}`, concurrency cancel-in-progress, checkout with `persist-credentials: false`, Swatinem/rust-cache, taiki-e/install-action for tools. Jobs:
  - [ ] fmt (rustfmt, tombi)
  - [ ] clippy on ubuntu and windows, `-D warnings`
  - [ ] docs, `RUSTDOCFLAGS=-D warnings`
  - [ ] test on ubuntu, macos, windows (nextest + doctests, `INSTA_UPDATE=no`)
  - [ ] coverage on ubuntu, floor 80%, upload `lcov.info` as an artifact
  - [ ] msrv via cargo-hack
  - [ ] deny, shear, typos, zizmor, actionlint, prek `--all-files`
  - [ ] ci-ok aggregate (re-actions/alls-green)
- [ ] `pr-title.yml`: `on: pull_request` (not `pull_request_target`), amannn/action-semantic-pull-request pinned by SHA.
- [ ] `sonar.yml`: after coverage, `cargo clippy --message-format=json > clippy.json`, SonarSource/sonarqube-scan-action with `sonar.rust.lcov.reportPaths` and `sonar.rust.clippyReport.reportPaths`; the quality gate is a required check.
- [ ] `release-plz.yml` and `release-plz.toml`: release PRs from conventional commits; `cliff.toml` for the changelog. Dry-run only until Phase 05.
- [ ] ~~`dist-workspace.toml` via `dist init`~~ moved to Phase 05: the generated release workflow is not SHA-pinned and needs hand edits, and nothing is released before then.
- [ ] Every `uses:` pinned by full SHA with a version comment; zizmor clean.

## Repo hygiene

- [ ] `LICENSE-MIT`, `LICENSE-APACHE`; `NOTICE` crediting Composer (MIT) for vendored files later.
- [ ] `.editorconfig`, `.gitattributes` (`* text=auto eol=lf`, fixtures and snapshots `-text`).
- [ ] `.github/CODEOWNERS`, issue forms (bug, feature), PR template (what, why, how tested).
- [ ] README badges: CI, Sonar gate, coverage, Scorecard, licence.

## Public-repo security

- [x] Repo made public (2026-10-01, decision 0009).
- [ ] `codeql.yml`: languages rust and actions, on push, PR and weekly.
- [ ] `scorecard.yml`: ossf/scorecard-action, publish results, SARIF upload.
- [ ] Settings: private vulnerability reporting, secret scanning, push protection, Dependabot alerts and security updates.
- [ ] Ruleset on `main` via `gh api`: PR required, required checks `ci-ok`, PR title, Sonar; linear history; block force push and deletion; signed commits; owner bypass. Applied after the first green CI run so the check names exist.
- [ ] `release.yml` (dist) includes `actions/attest` for build provenance.

## Proof

- [ ] Throwaway branch with a clippy warning, an unformatted file, a GPL dependency, an unused dependency and a bad commit message: each fails where it should. Branch deleted.
- [ ] `just ci` green from a fresh clone.
- [ ] CI green on all three OSes; Sonar dashboard populated.
- [ ] `progress.md` written with what was checked and what surprised.
