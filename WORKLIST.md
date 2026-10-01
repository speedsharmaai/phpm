# Worklist

The one-screen view. Details live in [phases](phases/README.md).

Current state: **Phase 02 done (2026-10-01).** Exit criteria met: identical
`vendor/` on every fixture, plugin apps working through the Composer
fallback, Laravel with scripts 5.3-5.9x faster warm than Composer, malware
filter proven against a blocked package. Cold installs ahead of riff by median
on both fixtures (clearly on Laravel, a tie on ytmate). Fuzzing runs weekly.

## Next three things

1. **Phase 03: compatibility sweep.** The corpus runner and the nightly sweep
   page have started (#43, #45, #46, #49); next is the published number.
2. **Rename fixture lockfiles** so GitHub's dependency graph stops treating
   them as dependencies; then turn automated security-fix PRs back on.
3. **Market check (owner).** Ask r/PHP, r/laravel and ten developers whether
   install time is a top-three pain. Decides whether Phase 06 has an audience.

## Owner actions

- [x] `SONAR_TOKEN` added; Sonar quality gate runs on every PR
- [x] SSH signing key registered; `main` requires verified signatures
- [ ] OpenSSF Best Practices registration at bestpractices.dev
- [ ] Trademark search for `phpm`

## Phase 01 · Spike

- [x] Competitors measured: riff, viv, vivacity ([competitors](phases/phase-01-spike/competitors.md))
- [x] Fixtures, hyperfine harness, `diffvendor`
- [x] PHP writers: `json_encode`, `var_export`, `dumpToPhpCode` (#12, #16)
- [x] Version normalisation port (#13)
- [x] `installed.json`, `installed.php`, `InstalledVersions.php` byte-identical on 5/5 fixtures (#14)
- [x] Fetch, global store, extraction, clonefile placement, pruning (#17)
- [x] Autoload files without class scanning (#18)
- [x] Class scanning for `-o` and classmap autoloads; autoload files byte-identical on 5/5 fixtures, dev and `--no-dev` (#20)
- [x] `phpm install`, bin proxies, no-op fast path, class scan cache (#22, #24)
- [x] **Gate passed: [gate.md](phases/phase-01-spike/gate.md)**

## Phase 02 · Parity core

- [x] Composer fallback for plugins and PHP-callable scripts, native string scripts (#32, #36)
- [x] Platform checks, auth, path/artifact/private repositories (#31, #33, #34)
- [x] Malware filter and `--audit`, proven against a blocked package (#35); cheap on warm installs (#41)
- [x] Download notifications (#37), sent from a detached process (#42)
- [x] Cold fetch: direct codeload, more connections, class scans during download (#47)
- [x] cargo-fuzz targets and a weekly workflow; two bugs found and fixed (#48, #50)
- [x] **Exit criteria met: [progress](phases/phase-02-parity-core/progress.md#close)**

## Phase 03 and beyond

See [phases/README.md](phases/README.md).
