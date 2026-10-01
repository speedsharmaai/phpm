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
