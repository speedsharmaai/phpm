# Phase 02 progress

What was checked, and how. Numbers are from the owner's Mac (M1 Pro, macOS
26.5.1, APFS, Composer 2.10.3, PHP 8.4.13 without intl) unless a row says
otherwise.

## Track A: Composer fallback and scripts

### What phpm decides, before it touches `vendor/`

`phpm install --explain` prints one line per package and one per decision.

1. **Plugins.** Locked `composer-plugin` and `composer-installer` packages
   (dev ones only with dev), then global ones from
   `COMPOSER_HOME/vendor/composer/installed.json`. Allowed as Composer's
   `PluginManager` allows them: `config.allow-plugins` merged over
   `COMPOSER_HOME/config.json` the way `Config::merge` does, `true`/`false`,
   first matching glob wins, `composer/package-versions-deprecated` off,
   `plugin-optional` skipped quietly. A plugin nobody allowed fails the run
   with Composer's non-interactive message (phpm never prompts), and a lock
   older than plugin API 2.2 with no `allow-plugins` fails as it does in
   Composer. Global plugins use the global `composer.json` rules.
2. **Whole install to Composer** when an active plugin places, patches or
   downloads packages itself: `composer/installers` and anything with
   `plugin-modifies-install-path` or `plugin-modifies-downloads`,
   `composer-installer` types, the WordPress core installers,
   `oomphinc/composer-installers-extender`, `cweagans/composer-patches`,
   `drupal/core-composer-scaffold`, and `symfony/flex` when the install has
   `symfony-pack` packages (flex installs those with no files). Also when
   composer.json has scripts on events only Composer's own install loop fires
   (`pre/post-package-*` when packages change, `pre-operations-exec`,
   file-download events, `init`, `command`, `pre-command-run`,
   `pre-pool-create`). phpm then runs `composer install` with the same flags
   and writes no state file, so the next run asks Composer again.
3. **Otherwise phpm installs** (fetch, store, place, `installed.*`, bins) and
   each step goes where it can run:
   - other active plugins: `composer dump-autoload` (with `--dev`/`--no-dev`,
     `-o`, `-a`, platform flags, `--no-scripts`) so plugins activate and the
     autoload events reach them, then `composer run-script post-install-cmd`
     (a "not defined" answer means no plugin listens);
   - no plugins: phpm writes the autoloader itself; each of
     `pre-install-cmd`, `pre-autoload-dump`, `post-autoload-dump`,
     `post-install-cmd` runs natively when every listener is a string, and
     through `composer run-script <event>` when one is a PHP callable
     (`Class::method`), a Symfony command class, an object-shaped script, a
     root bin, or a reference loop. Order is Composer's: `pre-install-cmd`
     before anything is placed, then the autoload events around the dump,
     then `post-install-cmd`.
4. `composer` missing when a step needs it: a clear error, exit 1, and
   nothing in `vendor/` changed (the check runs before placement).
   `PHPM_COMPOSER` points at a specific Composer.

### Native scripts

Ported from `EventDispatcher::doDispatch`: shell commands through `/bin/sh -c`;
`@php` as Composer builds it (`php -d allow_url_fopen=... -d
disable_functions=... -d memory_limit=...`, from one probe of the PHP binary's
ini, memory limit raised to 1536M or `COMPOSER_MEMORY_LIMIT` as
`bin/composer` does; a first word that is not a project path is looked up on
PATH); `@composer` and plain `composer ...` through PHP and the Composer
binary; `@putenv` (set and clear) persisting across events;
`@other-script args` with argument passing, `@additional_args` and
`@no_additional_args`; a warning for references to missing scripts;
`COMPOSER_SKIP_SCRIPTS`; `process-timeout` and `COMPOSER_PROCESS_TIMEOUT`
(default 300 s). Environment: `COMPOSER_DEV_MODE`, `PHP_BINARY` for non-`@php`
commands, and the real bin dir prepended to `PATH` when it exists.

`COMPOSER_BINARY`: Composer sets it to its own binary. phpm sets it to the
real `composer` it found (PATH or `PHPM_COMPOSER`, symlinks resolved like
`realpath($argv[0])`) and leaves it unset when there is none; `@composer`
then fails with a message saying Composer is needed. A failing script stops
the install with Composer's message and the script's exit code.

Known gaps: event flags (`optimize`) are not passed to callables run through
`run-script`; the no-op fast path skips scripts, since nothing changed since
the last full run that ran them; plugin listeners on `pre-install-cmd` only
fire when composer.json also has scripts for it.

### Read-ahead before Composer

A cloned file starts with a cold page cache, and Composer reads files one at
a time. Before handing a step to Composer, phpm reads the `classmap` dirs of
the packages it just placed (PSR dirs too when Composer will dump an
optimised autoloader) on all cores, overlapped with its own autoload dump.
On the Laravel skeleton that is 1,455 files in about 90 ms, and
`composer run-script post-autoload-dump` drops from 870 ms to 500 ms.

### Checked

`just e2e-apps`: each fixture installed in its app (`fixtures/apps.txt`) by
`composer install` and then by `phpm install` at the same path, scripts and
plugins on. `vendor/` and the trees scripts write to compared with
`phpm-diffvendor`, then the app run from phpm's tree.

| Fixture | Flags | phpm's path | Compared | Differences | App check |
|---|---|---|---|---|---|
| laravel-skeleton | none | native, `post-autoload-dump` through `composer run-script` | `vendor/`, `bootstrap/` | 0 | `php artisan --version`: Laravel 13.34.0 |
| symfony-demo | none | whole install to Composer (flex and `symfony/apache-pack`) | `vendor/`, `config/`, `public/` | 0 | `php bin/console about`: Symfony 8.1.0 |
| monica | `--ignore-platform-reqs` | native, `dump-autoload` and `post-install-cmd` through Composer (php-http/discovery, phpstan/extension-installer) | `vendor/`, `bootstrap/` | 0 | `php artisan --version`: Laravel 12.22.1 |
| drupal-recommended | none | whole install to Composer (composer/installers, scaffold) | `vendor/`, `web/`, `recipes/` | 0 | `Drupal::VERSION` loads |
| bedrock | none | whole install to Composer (composer/installers) | `vendor/`, `web/` | 0 | `$wp_version` loads |

Run at different paths instead, monica and drupal differ only in
`phpstan/extension-installer/src/GeneratedConfig.php`, which records absolute
install paths. `just e2e` (Phase 01 identity, `--no-scripts --no-plugins`)
still passes on laravel-skeleton (default, `--no-dev`, `-a`), ytmate (default,
`--no-dev`) and wicketyaari.

The symfony demo and the three plugin-heavy apps get no speed-up yet: their
plugins place packages, which is native only once Phase 04 adapters exist.

### Benchmark: Laravel skeleton with its scripts

`BENCH_SCRIPTS=1 PHPM_BIN=... tools/bench/bench.sh laravel-skeleton composer phpm`
(the app at laravel/laravel@06d016a, scripts and plugins on, so
`package:discover` runs on every install), plus a 10-run warm rerun. Other
agents were building on the machine (load average 8-12), so Composer's
spread is wide. Raw JSON: `bench/results/2026-10-01-macos-m1pro/laravel-skeleton/*-scripts-*.json`.

| Tool | Cold (n=3) | Warm (n=10), mean | Warm median | Warm min | No-op |
|---|---|---|---|---|---|
| Composer | 17.84 s ± 1.76 | 5.734 s ± 1.947 | 4.929 s | 4.312 s | 1.917 s |
| phpm | 13.38 s ± 2.26 | 926 ms ± 78 | 918 ms | 851 ms | 5.0 ms |

Warm, including the fallback: **6.2x** on the mean, 5.4x on the median, 5.1x
min to min. Of phpm's ~920 ms, about 250 ms is phpm (placing 109 packages,
`installed.*`, bins, the `-o` autoloader) and the rest is one
`composer run-script post-autoload-dump`: Composer boot about 210 ms, the
`ComposerScripts::postAutoloadDump` callable (Composer builds a class loader
by scanning every `classmap` dir) and `artisan package:discover`. Sending the
whole autoload dump to Composer instead cost 2.6 s.

Those runs were at `d8f744c`'s Track A code, before the malware filter
(`909d6f4`) landed. Since then every install that places packages first
checks the filter lists over the network, 0.35-0.75 s here, so `main` at
`d8f744c` measures (10 runs each, `phpm-scripts-warm-filter.json` and
`phpm-scripts-warm-no-blocking.json`):

| phpm at `d8f744c` | Warm median | Warm min | vs Composer's median |
|---|---|---|---|
| default (filter checked) | 1.233 s | 1.138 s | 4.0x |
| `--no-blocking` | 822 ms | 787 ms | 6.0x |

A rerun at `20d6251` (download notifications added) gave the same picture:
1.208 s and 845 ms mean.

With the filter check made cheap (see "What the filter costs" under Track B),
`BENCH_SCRIPTS=1 tools/bench/bench.sh laravel-skeleton composer phpm` plus a
10-run warm rerun, filter on, no other agents running (load average about 2):

| Tool | Cold (n=3) | Warm (n=5), mean | Warm (n=10), median | Warm (n=10), min | No-op |
|---|---|---|---|---|---|
| Composer | 21.4-130.8 s (GitHub throttled one run) | 4.482 s ± 0.193 | 4.773 s | 4.135 s | 2.011 s |
| phpm | 15.47 s ± 0.36 | 818 ms ± 46 | 811 ms | 776 ms | 5.7 ms |

Warm with scripts and the filter: **5.5x** on the 5-run mean, **5.9x** on the
10-run median, 5.3x min to min. One of the ten phpm runs took 2.0 s, most likely where the
lists had gone stale (600 s after the first check) and phpm asked Packagist
again, which is the price once per ten minutes. Raw JSON:
`bench/results/2026-10-01-filter-reuse/laravel-skeleton/`.

## Track B: platform, auth, repositories, policy

### Platform

- One `php` subprocess runs `crates/phpm/src/platform/probe.php`: version,
  debug/ZTS/`PHP_INT_SIZE`/IPv6, every loaded extension and version, and the
  `lib-*` versions with `PlatformRepository`'s detection ported line for line.
  The answer is cached in `<cache>/platform/v1/`, keyed by the binary's path,
  size, mtime and inode plus `PHPRC`/`PHP_INI_SCAN_DIR`, and dropped when a
  loaded ini file or its directory changes.
- `composer_live_platform_matches_composer_show` (ignored test) compares
  every platform package name and version with `composer show --platform`:
  identical on this Mac.
- The lock check fixes every locked package and the root's platform
  requirements (plus the lock's `platform`/`platform-dev` and
  `platform-overrides`), with `--ignore-platform-req` wildcards and `name+`
  upper-bound semantics. Constraint parsing and matching moved to
  `phpm_lock::constraint` and is checked against 2,700 recorded
  composer/semver 3.4.4 results.
- Error text and exit code compared with `composer install` on a lock that
  needs `ext-intl`, `php >=9` and an unknown extension: the same problems,
  the same wording, exit 2. phpm says "run phpm with" in the extension hint.
- Not probed: HHVM, and Xdebug's "skipped version" (Composer restarting
  itself without Xdebug).

### Auth

- Sources merged in Composer's order: `COMPOSER_HOME/config.json`,
  `COMPOSER_HOME/auth.json`, `COMPOSER_AUTH`, composer.json `config`, the
  project's `auth.json`, then `COMPOSER_AUTH` again; one credential per
  domain in `loadConfiguration`'s type order.
- Applied as `AuthHelper::addAuthenticationOptions` does: GitHub tokens only
  on `api.github.com`, GitHub Enterprise and GitLab fallbacks as basic auth,
  GitLab `Bearer`/`PRIVATE-TOKEN` only on `gitlab-domains`, Bitbucket
  consumers exchanged for a token (tested against a local server), public
  Bitbucket downloads left alone, inline `user:pass@host` winning.
- Redirects are followed by hand so headers are recomputed per host;
  `custom-headers` never follow a redirect to another host.
- URLs in errors go through a port of `Url::sanitize`; `Debug` output of the
  auth store lists origins only.
- Not supported: `client-certificate`, interactive prompts, writing tokens
  back to auth.json.

### Repositories

- A temp project with a symlinked path repo, a `symlink: false` path repo
  (with `.gitattributes` export-ignore, a `.git` directory, links inside and
  outside the package, an empty directory and a 0750 script), a tar.gz
  artifact (0600 file, 0700 directory, a symlink entry) and a flat zip
  artifact: `composer install` and `phpm install` give an identical
  `vendor/` (diffvendor, modes and link targets included), and again with
  `COMPOSER_MIRROR_PATH_REPOS=1`.
- Tar dists extract as `PharData` does: exact file modes, default directory
  modes, symlink entries as empty files. bzip2 tars and `gzip` dists are
  refused with a clear message.
- Windows junctions are not made; phpm tries a directory symlink and mirrors
  when that fails. Only the unix paths are tested.

### Policy

- Malware filter as `FilterListPoolFilter` runs it for an install: each
  composer repository's `packages.json` (reused for 600 s), its
  `summary-url` (conditional GET) and per-package metadata for the
  candidates, or the filter `api-url`; cached in `<cache>/repo/<url>/`.
- Proof: a lock with `aikido/endpoint-test 0.0.1`, which is on Packagist's
  malware list as a harmless test package. `composer install` and
  `phpm install` print the same problem text and both exit 2; nothing is
  downloaded. `live_packagist_blocks_the_aikido_test_package` (ignored)
  repeats it; `the_malware_filter_blocks_a_flagged_locked_package` does it
  against a local repository.
- `policy`/`audit` config, `COMPOSER_POLICY*`, `--no-blocking`,
  `COMPOSER_NO_BLOCKING`, `ignore`, `ignore-source`, `block-scope` and
  `ignore-unreachable` (install default: warn and continue) are honoured.
  Only the project's composer.json is read for policy, not the global
  config.json.
- `--audit` / `--audit-format` follow `Auditor::audit`, exit 5 on findings.
  Advisories come from the `security-advisories` api-url in one POST in
  every format; Composer's summary format reads the same data from
  per-package metadata. The table format prints as plain.
- What the filter costs. The check starts on its own thread at the start of
  an install and runs while phpm reads the lock, checks the platform, runs
  `pre-install-cmd` (Composer also runs it before the filter) and downloads
  into the store. Nothing is removed from or placed into `vendor/` until the
  verdict is in, so a blocked package never reaches `vendor/`; one that was
  downloaded while the lists were being read is taken out of the store again.
  A download error waits for the verdict first, so a blocked package still
  reports as blocked.
- Freshness. After a check, phpm records per repository which locked
  `name version` pairs had no entry at all on the checked lists, until the
  earlier of the summary's `Cache-Control: max-age` (900 s on Packagist) and
  the root file's 600 s. An install whose packages are all in a fresh record
  asks nothing; anything new, any package with an entry (even an ignored
  one), a changed list set, an `api-url` repository or a summary without
  `max-age` goes to the network as before. Composer revalidates the summary
  with `If-Modified-Since` on every install, so the one difference is this:
  a package added to the list in the last 600 s at most, after phpm checked
  the same version clean, is still placed by phpm and refused by Composer.
  That is the freshness Packagist itself declares for the file (decision
  0006), and the same window the no-op path already used. Per-package
  lookups are only made for summary candidates (none on the Laravel
  skeleton), so skipping them when the summary is unchanged would save
  nothing measurable and is not done.
- Measured on the Laravel skeleton, `--no-scripts`: the check took 350-620 ms
  per warm install before (one TLS connection to the CDN in Singapore, about
  120 ms RTT from India, three round trips), and 2 ms within freshness.
- The no-op state expires with the repository's cache headers (600 s root
  file, `max-age` of the summary), so a no-op revalidates the filter list at
  most that often.

### Notifications

- One POST per `notification-url` with Composer's payload for the packages
  this run installed or updated; `downloaded` (bytes or `false`) only for
  packagist.org. Failures are ignored. Off with `config.notify-on-install:
  false` (the project's, else `COMPOSER_HOME/config.json`'s) or
  `COMPOSER_DISABLE_NETWORK`; the e2e tests set the latter.
- Sent by a detached copy of phpm (`phpm __notify`, payload on stdin) that
  finishes after the install has exited; a thread is the fallback when it
  cannot start. Composer waits for the POST. Sent in-process it added
  450-500 ms to every warm install here (a new TLS connection to
  packagist.org from India): the Laravel skeleton went from 224 ms to
  700 ms warm with `--no-scripts`, and is back to 238 ms ± 17 detached.

### Fixtures and timing

- `just e2e` for laravel-skeleton (plain, `--no-dev`,
  `--classmap-authoritative`), ytmate (plain, `--no-dev`) and wicketyaari:
  all byte-identical after these changes.
- laravel-skeleton no-op, release build, `hyperfine -N --warmup 5 --runs 50`:
  4.7 ms ± 0.3 ms (Phase 01: 4.7 ms). After the state expires, the next run
  revalidates the filter summary (about 350 ms from India) and is a no-op
  again for the next ten minutes.

## Track C: cold fetch and fuzzing

### Cold fetch: what the profile showed

Laravel skeleton (109 dists, 17.4 MB, all `api.github.com/.../zipball/<sha>`)
from India on home broadband, about 120 ms RTT to the Packagist CDN and
250 ms to GitHub:

- **Bandwidth is the floor.** `curl` fetching all 109 archives straight from
  codeload, 24 to 109 at a time, takes 8.0-8.9 s; one 6.9 MB archive alone
  (aws-sdk-php, on ytmate) streams at about 1.6 MB/s.
- **The API redirect costs a round trip per package.** Every dist URL answers
  302 to `codeload.github.com/<owner>/<repo>/legacy.zip/<sha>`. The same 109
  through the redirect took 10.8 s with `curl` against 8.3 s direct, and the
  bytes are identical (SHA-1 compared on several archives, owner casing kept
  as in the lock). Codeload builds archives on demand, so a first request can
  wait 5 s for its first byte; more requests in flight hide that.
- **One HTTP/2 connection is one TCP window.** reqwest multiplexes every
  download to a host on a single connection, which caps a bandwidth-bound
  batch on a long, lossy path.
- **The tail after the downloads was 1.3 s**, mostly the class scan of 109
  freshly extracted trees (525 ms) and the autoload dump reading them cold.
- **The filter check was serial before the downloads** (0.4-0.6 s).
- Codeload sends no `Content-Length` and ignores `Range`, so one large archive
  cannot be split; ytmate's aws-sdk-php bounds that fixture.

### What changed

- Public zipballs pinned to a full commit go straight to codeload; anything
  else (a branch, a query string, GitHub Enterprise, a project with a GitHub
  token, which may be for a private repository) uses the lock's URL, and a
  failed direct fetch falls back to it.
- Dists use eight HTTP/2 connections per host, round robin, and up to 48
  GitHub downloads in flight. Packagist hosts keep their limit of 10
  (decision 0006); metadata requests are unchanged.
- Each tree is class-scanned on the blocking pool as soon as it is extracted,
  while the other downloads run; the dump then reads the cache (3-4 ms).
- The malware check overlaps the downloads (Track B notes above).

Measured with the variants interleaved (8 rounds, `--no-blocking`, network
notifications off), Laravel cold medians: lock URLs on one connection 11.6 s,
direct codeload 10.5 s, plus four connections 9.7 s, eight connections and 48
in flight 9.2 s. The post-download tail went from about 1.3 s to 0.4 s.

### Cold benchmark against the field

`tools/bench/cold.sh 8 laravel-skeleton ytmate`: for each round every tool
installs once from an empty cache, the order rotating per round, one
`hyperfine --runs 1` per install, 8 rounds per fixture, all on the same
machine in the same hour. Every tool
with `--no-scripts --no-plugins` where it has them. The network was noisy
(Composer ranged 11.7-42.3 s), so the median and the interquartile range are
the honest numbers. Raw JSON: `bench/results/2026-10-01-cold-interleaved/`.

| Fixture | Tool | Median | IQR | Min | Max |
|---|---|---|---|---|---|
| laravel-skeleton | Composer | 21.08 s | 17.76-31.41 | 16.88 | 42.34 |
| laravel-skeleton | **phpm** | **8.67 s** | 7.91-11.36 | 7.01 | 13.73 |
| laravel-skeleton | riff 0.0.7 | 13.17 s | 9.43-15.26 | 9.05 | 26.47 |
| laravel-skeleton | vivacity 0.19.1 | 14.21 s | 12.29-26.57 | 9.57 | 40.49 |
| ytmate | Composer | 17.85 s | 13.33-25.85 | 11.71 | 26.45 |
| ytmate | **phpm** | **11.65 s** | 8.85-16.00 | 8.67 | 24.13 |
| ytmate | riff 0.0.7 | 13.19 s | 8.73-20.57 | 7.65 | 27.75 |
| ytmate | vivacity 0.19.1 | 11.70 s | 9.01-13.41 | 7.98 | 17.43 |

Round by round, phpm beat riff in 7 of 8 Laravel rounds and in 4 of 8 ytmate
rounds. So: cold ≤ riff on both fixtures by median; on Laravel clearly, on
ytmate a tie within the noise, because ytmate's time is one 6.9 MB archive
that codeload streams on one connection with no length and no ranges, and
every tool waits for it the same way. viv was not in this run (it was not
ahead of riff or vivacity cold at the gate).

### Fuzzing

`fuzz/` is a cargo-fuzz crate outside the workspace with four targets:

| Target | What it feeds |
|---|---|
| `lockfile` | `ComposerJson::parse`, `Lock::parse`, the lock's package lists, aliases, and every link constraint through `constraint::parse` |
| `version` | `version::normalize`, `normalize_branch`, numeric alias prefixes, `constraint::parse`, bounds, matching and `version_compare` |
| `classes` | `strip_whitespace` (short tags on and off) and `find_classes` (with and without enums) |
| `zip` | `Store::insert_zip` and `insert_tar` into a store inside a temp dir; fails if anything is written next to the store |

`.github/workflows/fuzz.yml` runs each target for five minutes every Monday
and on demand (`seconds` input), installing nightly in CI only, and uploads
`fuzz/artifacts/<target>` when a target fails. CI's `fuzz-targets` job (and
`just fuzz-check`) builds the crate on stable so an API change cannot break
it silently.

Found by running the targets built on stable (no coverage feedback) for
30-180 s each, then by two dispatched runs of the workflow (the first only
after pointing cargo-fuzz at the gnu target; the binary from install-action
defaults to musl, where the sanitizer cannot link). The second run, five
minutes per target, was clean on all four.

- `version`: `~` or `^` on a number past `PHP_INT_MAX` overflowed an `i64`
  (debug panic, release wrap-around). PHP turns the sum into a float and
  prints it with `precision` 14, so `~9223372036854775807` has the upper
  bound `9.2233720368548E+18.0.0.0-dev`. Ported, with a regression test
  holding five bounds recorded from composer/semver 3.4.4.
- `version` (first CI run, coverage-guided): `version_compare` on a string
  with a NUL recursed forever. PHP's C code stops at the first NUL; ported,
  with PHP 8.4's answers on the input as a regression test.
- Two wrong assertions of mine, not bugs. `strip_whitespace` can grow the
  source: PHP adds a space after each `<?` open tag, and phpm's output was
  byte-identical to `php_strip_whitespace` on the fuzzer's input (81 bytes in,
  84 out). And `normalize` is not idempotent in
  composer/semver either (`2222-222222222222222` normalises to
  `2222.222222222222222`, which normalises to `2222.222222222222222.0.0`).

## Close

Phase 02 exit criteria, all met on 2026-10-01:

| Criterion | Result |
|---|---|
| Phase 01 fixtures byte-identical; plugin fixtures give working apps | `just e2e`: 6 of 6 at 0 differences; `just e2e-apps`: 5 of 5 at 0 differences, apps run (re-run after the Track C changes) |
| Laravel with scripts at least 5x faster warm than Composer, fallback included | 5.5x mean / 5.9x median (quiet run), 5.3x mean and median in the closing run under load |
| Malware filter tested against a blocked package | `aikido/endpoint-test`, same text and exit code 2 as Composer |

Closing numbers, one session, phpm at `ff6828f`, `tools/bench/bench.sh`
(cold n=3, warm n=5, no-op n=10). Other jobs were running on the machine
during the ytmate and with-scripts rows (load average 8-13), so their spread
is wide. Raw JSON: `bench/results/2026-10-01-phase-02-close/`.

| Fixture | Scenario | Composer | phpm | riff | viv | vivacity |
|---|---|---|---|---|---|---|
| laravel-skeleton | warm | 4.179 s | **222 ms** (18.8x) | 2.486 s | 2.215 s | 684 ms |
| laravel-skeleton | no-op | 1.606 s | **4.5 ms** | 1.514 s | 8.8 ms | 470 ms |
| laravel-skeleton | warm, scripts on, median | 4.570 s | **865 ms** (5.3x) | | | |
| laravel-skeleton | no-op, scripts on | 2.295 s | **4.7 ms** | | | |
| ytmate | warm, median | 3.676 s | **311 ms** (11.8x) | 2.072 s | 2.213 s | 644 ms |
| ytmate | no-op, median | 1.151 s | **8 ms** | 1.052 s | 14 ms | 487 ms |

Cold, from the interleaved 8-round run above (median): laravel-skeleton
Composer 21.08 s, phpm 8.67 s, riff 13.17 s, vivacity 14.21 s; ytmate
Composer 17.85 s, phpm 11.65 s, riff 13.19 s, vivacity 11.70 s. The bench.sh
cold rows in the closing JSON (n=3) agree on Laravel (phpm 7.09 s, riff
10.10 s) and are noise on ytmate (one phpm run at 17 s).

Still open, carried forward: the known gaps under Track A (event flags for
callables, plugin listeners on `pre-install-cmd`), and the ytmate cold tie,
which needs either a smaller archive or GitHub sending a length.
