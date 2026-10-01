# Phase 01 tasks

## Before any code

- [x] Clone shyim/riff and svandragt/vivace. Run both on the Laravel skeleton and symfony/demo. Write `competitors.md` in this folder: what each does, speed, where `vendor/` differs from Composer's. Done, with vivacity 0.19.1 added: it is the strongest.
- [x] If either already meets gate criteria 1-3, stop here and write `gate.md` saying so (decision 0007). None does; see competitors.md.
- [ ] Trademark search for `phpm` (USPTO, EUIPO, IP India class 9/42).
- [x] Public repo `speedsharmaai/phpm` on the brand account, SSH key `~/.ssh/speedsharma` (decision 0009).

## Setup

- [ ] Cargo workspace per [topology](../../docs/arch/topology.md): `phpm`, `phpm-lock`, `phpm-php`, `phpm-store`, `phpm-fetch`, `phpm-autoload`.
- [ ] `rust-toolchain.toml` pinned; clippy and rustfmt in CI from the first commit.
- [ ] Pin Composer 2.10.3 as the reference version in `fixtures/COMPOSER_VERSION`; vendor its verbatim files (`ClassLoader.php`, `InstalledVersions.php`, `LICENSE`, bin proxy templates) into `crates/phpm-autoload/composer/` with their MIT licence.

## Fixtures

- [x] `fixtures/<name>/composer.json` + `composer.lock` for each fixture in [scope](scope.md). Copies, never symlinks to the live apps.
- [x] `fixtures/README.md`: source, date, package count, plugin list, flags used.

## PHP-format writers (`phpm-php`)

- [x] `json_encode` writer: pretty print, unescaped slashes and unicode, 4-space indent, `{}` vs `[]` rules. Golden tests against PHP output for 50 generated inputs. `phpm-php`, plus proptests against `php` in CI.
- [ ] `var_export`-compatible writer for `installed.php` and `autoload_static.php`. Golden tests the same way. The `installed.php` half is done (`dump_to_php_code`); `autoload_static.php` is not.
- [ ] content-hash, flags 0, verified against every fixture lock.

## Install path

- [ ] Lock parser and plan (install / update / remove) against `installed.json`.
- [x] Root version guessing via git, matching `VersionGuesser` for tag, branch and no-VCS cases. hg, svn and fossil checkouts are refused.
- [ ] Fetch: reqwest + tokio, HTTP/2, User-Agent with contact, ≤ 10 Packagist, ≤ 24 codeload, 3 retries, codeload-400 retry.
- [ ] Store: `~/.cache/phpm/pkgs/v1/`, key `(name, reference)`, zip buffered in memory, `zip` crate on rayon, top-dir strip, `.DS_Store` skip, modes kept, temp + atomic rename.
- [ ] Link: `libc::clonefile` per package directory (macOS); hardlink per file (Linux); copy fallback; stale packages removed.
- [x] `installed.json`, `installed.php`, `InstalledVersions.php` written. Byte-identical to Composer 2.10.3 on all five fixtures, dev and `--no-dev` (`just golden`).
- [ ] Bin proxies from Composer's templates.
- [ ] Autoload: PackageSorter port (weights, `strnatcasecmp`, stable sort), psr-0, psr-4, classmap and files sections, suffix rules, `autoload_static.php`, `platform_check.php` (php-only default).
- [ ] State file and no-op fast path.
- [ ] `--no-dev`.

## Measurement

- [x] `tools/diffvendor`: Composer `--no-scripts --no-plugins` vs phpm into sibling dirs; compare bytes and mode of every file; exit non-zero on any difference.
- [x] `tools/bench`: hyperfine, cold / warm / no-op, `--prepare` deletes `vendor/` (and caches for cold), run order alternated, raw JSON kept with tool versions, OS, filesystem.
- [ ] Run on macOS APFS and in a Linux Docker container (no bind mount; vendor inside the container).
- [ ] Profile phpm's warm path and write where the remaining time goes.

## Gate

- [ ] Fill the five numbers from [phases/README.md](../README.md#the-gate).
- [ ] Write `gate.md`: table, what was surprising, the call.
- [ ] If it passes: ask r/PHP, r/laravel and ten developers directly whether install time is a top-three pain. Record answers in `gate.md` before scoping Phase 02 tasks.
- [ ] If it fails: say so in `gate.md`, stop, and either contribute to the leader or go back to the shortlist in `devtools/_research/infra-landscape-2026-10.md`.
