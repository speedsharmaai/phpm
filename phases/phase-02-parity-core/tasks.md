# Phase 02 tasks

Three tracks that can run in parallel. Each lands as small PRs with tests in
the same PR, `just ci` green, Sonar green.

## Fixtures

- [x] Add plugin-heavy fixtures: `drupal-recommended` (drupal/recommended-project), `bedrock` (roots/bedrock), and keep `symfony-demo` and `monica` without `--no-plugins` from now on. Stored as `fixture.json` / `fixture.lock`.

## Track A: Composer fallback and scripts (decision 0004)

- [x] Detect what needs PHP: `composer-plugin` packages allowed by `config.allow-plugins`, global plugins in `COMPOSER_HOME`, PHP-callable scripts (`Class::method`) on install events.
- [x] Fallback: phpm does fetch, store, place, installed.*, bins; then runs real `composer dump-autoload` (plus `--optimize`/`--classmap-authoritative` flags as configured) and `composer run-script` for the install events, so plugins activate and events fire. Stop with a clear error, writing nothing partial, when a fallback is needed and `composer` is not on PATH.
- [x] String scripts and `@php`, `@composer`, `@putenv`, script references (`@other-script`) run natively with Composer's environment (`COMPOSER_DEV_MODE`, `COMPOSER_BINARY`, PATH with `vendor/bin` first); event order `pre-install-cmd`, `pre-autoload-dump`, `post-autoload-dump`, `post-install-cmd`.
- [x] `--explain`: one line per package and per decision (native, fallback, why).
- [x] Laravel skeleton with scripts (`package:discover`): working app, still ≥ 5x faster warm than Composer including the fallback cost.
- [x] Symfony demo with flex and runtime, monica with its plugins: working app after fallback; `vendor/` matches Composer except files plugins write differently (list them).

## Track B: platform, auth, repositories, policy

- [x] Platform: probe `php` once (version, `PHP_INT_SIZE`, extensions and versions, lib versions), cache by PHP binary path and mtime; verify locked requirements like Composer's platform solve; `config.platform`, `--ignore-platform-req(s)`; Composer's error wording.
- [x] Auth: every source (`COMPOSER_HOME/auth.json`, project `auth.json`, `COMPOSER_AUTH`, composer.json `config`) and type (`http-basic`, `bearer`, `github-oauth`, `gitlab-oauth`, `gitlab-token`, `bitbucket-oauth`, `forgejo-token`, `custom-headers`, inline `user:pass@host`, `github-domains`/`gitlab-domains`), Composer's precedence.
- [x] Repositories: `path` (symlink default, `relative`, `symlink: false` copy, `COMPOSER_MIRROR_PATH_REPOS`), `artifact`, private `composer` repos (Satis, Private Packagist) via the lock's dist URLs.
- [x] Policy: Composer 2.10 malware filter (`packages.json` filter metadata, `lists/all/summary.json`, conditional GET, Composer's defaults and exit codes); security-advisory audit after install with Composer's defaults; `--no-audit`, `--no-blocking`. Test against a package on the block list. Release blocker.
- [x] `notify-batch` download notifications to Packagist, batched, as Composer sends them.

## Track C: cold fetch and fuzzing

- [x] Profile a cold laravel-skeleton install: DNS, TLS, connection reuse, per-host concurrency, time to first byte, extraction overlap. Compare with riff (4.7 s on ytmate vs phpm 5.6 s).
- [x] Fix what the profile shows: HTTP/2 connection reuse to codeload, concurrency per host, extract while downloading; target cold ≤ riff on both fixtures.
- [x] cargo-fuzz targets: lockfile parser, version normaliser, class scanner (strip + find_classes), zip extraction. A scheduled CI workflow runs each for a few minutes on nightly; crashes become regression tests.

## Exit

- [x] All Phase 01 fixtures still byte-identical; new plugin fixtures produce working apps.
- [x] Laravel with scripts ≥ 5x faster warm than Composer.
- [x] Malware filter proven against a blocked package.
- [x] `progress.md` with what was checked.
