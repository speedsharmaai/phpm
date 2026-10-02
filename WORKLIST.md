# Worklist

The one-screen view. Details live in [phases](phases/README.md).

Current state: **Phase 02, 04 and 05 done; Phase 03 in progress.**
Phase 05 (2026-10-02): 40 real apps on GitHub's three runner OSes, median
warm 9.6x (Linux), 18.3x (macOS), 1.6x (Windows), no-op 329-636x; whole CI
jobs only 1.26x faster on Linux, so market-research kill criterion 2 (3x)
is not met.
Phase 04 (2026-10-02): Bedrock and a real Drupal site install fully
natively, no Composer fallback for any step, 0 `vendor/`/`web/`/`recipes/`
differences, warm 14.1x and 7.7x faster than Composer, no-op 256x and 233x
faster. Phase 02 exit criteria met 2026-10-01: identical `vendor/` on every
fixture, plugin apps working through the Composer fallback, Laravel with
scripts 5.3-5.9x faster warm than Composer, malware filter proven against a
blocked package. Fuzzing runs weekly.

## Next three things

1. **Phase 03: compatibility sweep.** The corpus runner and the nightly sweep
   page have started (#43, #45, #46, #49); next is the published number
   against the 95% gate.
2. **Phase 05 follow-ups.** Re-run with #111 so the five `vendor-dir`
   projects and PrestaShop are measured; profile Windows (warm 1.6x, one
   project slower than Composer) before Phase 06 claims anything there.
3. **Rename fixture lockfiles** so GitHub's dependency graph stops treating
   them as dependencies; then turn automated security-fix PRs back on.

## Owner actions

- [x] `SONAR_TOKEN` added; Sonar quality gate runs on every PR
- [x] SSH signing key registered; `main` requires verified signatures
- [ ] OpenSSF Best Practices registration at bestpractices.dev
- [ ] Trademark search for `phpm`
- [ ] Market check: ask r/PHP, r/laravel and ten developers whether install
  time is a top-three pain. Decides whether Phase 06 has an audience.

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

## Phase 04 · Plugin adapters

- [x] Task doc and adapter registry: per-plugin native coverage decided
  before anything is placed, every active plugin must be covered or
  Composer runs the whole install (#60)
- [x] composer/installers: every installer type, root `installer-paths`,
  `installer-name`, `installer-disable`; 855 golden cases (#64)
- [x] pestphp/pest-plugin: `vendor/pest-plugins.json` in local repository
  order (#69)
- [x] symfony/runtime: `vendor/autoload_runtime.php`, every stable tag
  v7.0.0-v8.1.0 verified (#70)
- [x] phpstan/extension-installer: `GeneratedConfig.php`, compacted
  constraint, 1.4.0-1.4.3 verified (#72)
- [x] dealerdirect/phpcodesniffer-composer-installer: `CodeSniffer.conf`
  `installed_paths`, checked against a real drupal-recommended install (#74)
- [x] drupal/core-composer-scaffold (file mappings, `DrupalInstalled.php`,
  xxh3 hash), drupal/core-project-message and drupal/core-recipe-unpack
  (no-ops), php-http/discovery (stale-file cleanup, declines when pinned) (#81)
- [x] **Exit criteria met: Bedrock and drupal-recommended install fully
  natively, 0 differences, warm 14.1x/7.7x faster than Composer, no-op
  256x/233x faster:
  [progress](phases/phase-04-plugin-adapters/progress.md#close)**

symfony/flex, cweagans/composer-patches and wikimedia/composer-merge-plugin
stay on fallback by scope (decision 0004), not fidelity.

## Phase 05 · Real-world benchmarks

- [x] 40-project corpus from the sweep's pins, every scope group, 19 over
  10k stars (#76)
- [x] `phpm-bench-real` aggregator and README table generator (#78)
- [x] `tools/bench-real/run`, `worktrees`, `ci-scenario` (#83, #85)
- [x] `bench-real.yml`: weekly, three OSes, sharded, `/bench/` on Pages
  alongside the sweep page (#84, #88)
- [x] Windows: eight rounds of harness fixes, hyperfine's shell handling
  replaced by plain-bash cleanup between iterations (#90-#110)
- [x] `config.vendor-dir` and `diffvendor` errors handled (#111)
- [x] **First full run published:
  [progress](phases/phase-05-real-world-benchmarks/progress.md)**
- [ ] Owner's Mac column, 3200x1800 chart export, per-run timeout

## Phase 03 and beyond

See [phases/README.md](phases/README.md).
