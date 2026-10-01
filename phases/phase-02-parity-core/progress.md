# Phase 02 progress

What was checked, and how. Numbers are from the owner's Mac (M1 Pro, macOS
26.5.1, APFS, Composer 2.10.3, PHP 8.4.13 without intl) unless a row says
otherwise.

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
- The no-op state expires with the repository's cache headers (600 s root
  file, `max-age` of the summary), so a no-op revalidates the filter list at
  most that often.

### Notifications

- One POST per `notification-url` with Composer's payload for the packages
  this run installed or updated; `downloaded` (bytes or `false`) only for
  packagist.org. Runs on its own thread during the autoload dump; failures
  are ignored. Off with `config.notify-on-install: false` or
  `COMPOSER_DISABLE_NETWORK`; the e2e tests set the latter.

### Fixtures and timing

- `just e2e` for laravel-skeleton (plain, `--no-dev`,
  `--classmap-authoritative`), ytmate (plain, `--no-dev`) and wicketyaari:
  all byte-identical after these changes.
- laravel-skeleton no-op, release build, `hyperfine -N --warmup 5 --runs 50`:
  4.7 ms ± 0.3 ms (Phase 01: 4.7 ms). After the state expires, the next run
  revalidates the filter summary (about 350 ms from India) and is a no-op
  again for the next ten minutes.
