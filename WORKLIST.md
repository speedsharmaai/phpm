# Worklist

Nothing is built. Research is written, a baseline is measured, the repo is
public, and Phases 00 and 01 are tasked.

Current state: **Phase 00 done (PR #1), Phase 01 spike next.**

## Next three things

1. **Owner actions from Phase 00.** `SONAR_TOKEN` (free on sonarcloud.io),
   OpenSSF Best Practices registration, a signing key. See
   [phase 00 progress](phases/phase-00-foundation/progress.md).
2. **Trademark search for `phpm`.** The repo is public now, so a rename gets
   more expensive every week.
3. **Read riff and vivace properly** (Phase 01's first task). Binaries are
   downloaded and verified; the head-to-head run is next.

## Now

- [x] Research: Composer internals, market and competitors, uv architecture, naming
- [x] Baseline benchmark on this Mac (Composer 2.10.3, PHP 8.4, APFS)
- [x] Proof the floor is real: 109 package dirs cloned in 0.14-0.24 s
- [x] Decisions 0001-0007
- [x] Phases 00 and 01 scoped and broken into tasks; phases 02-06 outlined
- [ ] riff and vivace read and run
- [ ] Trademark search and name reservation
- [ ] Fixture set collected

## Phase 00 · Foundation

- [ ] Owner: SonarQube Cloud org + `SONAR_TOKEN`; Best Practices badge; signing key
- [x] Workspace, toolchain, lints, rustfmt, clippy config
- [x] prek hooks, committed, typos, tombi, markdownlint
- [x] nextest, testkit, coverage floor (insta, proptest, criterion arrive with Phase 01 code)
- [x] cargo-deny, cargo-shear, cargo-hack, Dependabot, SECURITY.md
- [x] justfile
- [x] CI, PR title, Sonar, CodeQL, Scorecard, release-plz workflows (dist moved to Phase 05)
- [x] Ruleset on `main`, security settings
- [x] Licences, CONTRIBUTING, CODEOWNERS, templates, badges
- [x] Proof: every gate fails where it should

## Phase 01 · Spike

- [ ] Cargo workspace, one binary, `phpm install` only
- [ ] Lockfile parse and diff against `vendor/composer/installed.json`
- [ ] Parallel dist download into the store, GitHub top-dir strip, exec bits kept
- [ ] Per-package clone into `vendor/`
- [ ] `installed.json`, `installed.php`, `InstalledVersions.php` byte-identical
- [ ] Autoload family (non-optimised) byte-identical
- [ ] Bin proxies byte-identical
- [ ] No-op fast path
- [ ] Diff harness: phpm vs Composer `vendor/` on every fixture
- [ ] hyperfine harness: Composer vs riff vs vivace vs phpm, cold, warm, no-op
- [ ] **Gate: write `gate.md` with the table and the call**

## Phase 02 and beyond

Outlined, not tasked. What Phase 02 contains depends on which files the diff
harness shows are hard to match, and which plugins the fixtures actually hit.
See [phases/README.md](phases/README.md).
