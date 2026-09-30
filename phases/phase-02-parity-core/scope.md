# Phase 02: Parity core

Outline. Tasked only after the Phase 01 gate passes.

## Why

The spike proves the easy case: plugin-free lockfiles without `-o`. Real
projects need the rest of what `composer install` does, and every missing
piece is a project that breaks or silently differs. This phase closes the
gap for everything that does not need a PHP plugin to run.

## What ships

- **`-o` and `--classmap-authoritative`.** Class scanning with the
  `mago-syntax` lexer on rayon, reproducing `PhpFileParser` semantics:
  comments, heredoc/nowdoc, `?>` inline HTML, `__halt_compiler`, the enum
  gate, anonymous classes, `exclude-from-classmap` patterns. Results cached
  per file in the store. readdir-order ambiguity detected and reported.
  Laravel sets `optimize-autoloader: true` by default, so this is not optional.
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

## Exit criteria

- Every Phase 01 fixture plus the plugin fixtures (Symfony, Drupal, Bedrock,
  Monica) installs correctly: identical `vendor/` on plugin-free ones,
  working app after fallback on the rest.
- The Laravel skeleton with scripts and `-o` is still at least 5x faster warm
  than Composer, including the fallback cost of its PHP-callable script.
- Malware filter tested against a package on the block list.

## Out of scope

Native plugin adapters (Phase 04). Windows (Phase 03). Resolver (Phase 06).
