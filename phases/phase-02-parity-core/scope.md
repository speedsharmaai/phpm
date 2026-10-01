# Phase 02: Parity core

Gate passed 2026-10-01. Tasks: [tasks.md](tasks.md).

## Why

The spike proves the easy case: plugin-free lockfiles without `-o`. Real
projects need the rest of what `composer install` does, and every missing
piece is a project that breaks or silently differs. This phase closes the
gap for everything that does not need a PHP plugin to run.

## What ships

- **`-o` and `--classmap-authoritative`.** Done in Phase 01 (class scanner
  and per-store-tree class cache).
- **Platform.** `php` probed once and cached; locked requirements verified;
  `config.platform`, `--ignore-platform-req(s)`; failure messages that say
  the same thing Composer says.
- **Auth.** Every auth.json source and key type (http-basic, bearer,
  github-oauth, gitlab-oauth/token, bitbucket-oauth, forgejo-token,
  custom-headers), with Composer's precedence.
- **Repositories.** Path repos (symlink, relative, copy), artifact repos,
  private composer repos (Satis, Private Packagist) for dist URLs in the lock.
- **Policy.** Composer 2.10 malware filter and security-advisory checks,
  same defaults, same exit codes (decision 0006). Release blocker.
- **Scripts.** String scripts and `@php` run with system PHP, with the
  environment Composer sets (`COMPOSER_DEV_MODE`, `COMPOSER_BINARY` etc.).
- **Fallback (decision 0004).** Any plugin, PHP-callable script, or unsupported
  repo type → phpm does its part, then runs `composer dump-autoload` and the
  script events through real Composer. `--explain` prints each decision.
- **Notifications.** `notify-batch` to Packagist.
- **Cold fetch.** The gate showed phpm level with vivacity and behind riff on
  cold installs. Profile and fix: connection reuse, per-host concurrency,
  extract while downloading.
- **Fuzzing.** cargo-fuzz targets for the lockfile parser and the class
  scanner, run in CI on a schedule. Both read untrusted input.

## Exit criteria

- Every Phase 01 fixture plus the plugin fixtures (Symfony, Drupal, Bedrock,
  Monica) installs correctly: identical `vendor/` on plugin-free ones,
  working app after fallback on the rest.
- The Laravel skeleton with scripts and `-o` is still at least 5x faster warm
  than Composer, including the fallback cost of its PHP-callable script.
- Malware filter tested against a package on the block list.

## Out of scope

Native plugin adapters (Phase 04). Windows (Phase 03). Resolver (Phase 07).
