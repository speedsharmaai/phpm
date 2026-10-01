# Phase 01 progress

Where the spike stands, for whoever writes `gate.md`. Numbers are from the
owner's Mac unless a row says otherwise.

## `phpm install`

`crates/phpm`, the binary. From `composer.json` + `composer.lock`:

1. Fast path: a stat-only check of a state file (below). Hit: exit.
2. Read and plan: parse both files, refuse what the spike cannot do (below),
   compare with `vendor/composer/installed.json` to find packages that are
   already in place.
3. Fetch what the store lacks (`phpm-store`), place the rest with
   `clonefile` per package, remove packages the old `installed.json` had and
   the lock does not.
4. `installed.json`, `installed.php`, `InstalledVersions.php` (`phpm-lock`).
5. Bin proxies: Composer's `BinaryInstaller` ported, both proxy templates,
   the PHPUnit special case, `.bat` proxies when `bin-compat` is `full`, bins
   that resolve outside their package skipped, targets made `0777 & ~umask`
   (copied first if hard-linked from the store).
6. Autoloader (`phpm-autoload`), with cached class scans for `-o` (below).
7. Write the state file.

Flags: `--no-dev`, `-d/--working-dir`, `--link-mode clone|hardlink|copy`,
`-o`, `-a`, `--no-autoloader`, `--no-scripts`, `--no-plugins`,
`--ignore-platform-reqs`, `--ignore-platform-req`, `-q`, `-v`. Accepted and
ignored: `-n/--no-interaction`, `--no-progress`, `--prefer-dist`,
`--no-audit`, `--dev`, `--ansi`, `--no-ansi`. Exit codes: 0 ok, 1 install
error, 2 usage error. Respects `config.vendor-dir`, `config.bin-dir`,
`config.bin-compat`, `COMPOSER`, `COMPOSER_VENDOR_DIR`, `COMPOSER_BIN_DIR`,
`COMPOSER_BIN_COMPAT` and `COMPOSER_ROOT_VERSION`.

Refused, with a message that names the packages or events: a lock with
`composer-plugin` or `composer-installer` packages unless `--no-plugins`;
root scripts for any install-time event unless `--no-scripts`; packages that
would install from source; dists other than zip.

### State file

`<cache>/state/v1/<sha256(project dir)>`, outside `vendor/`, so `vendor/`
stays byte-identical and `diffvendor` needs no ignore list. It holds a
SHA-256 over composer.json and composer.lock bytes, the flags that change
output, the Composer env vars above, `HOME`, git HEAD (read from files, never
by running git) and the phpm version; plus size, mtime and inode of every file
phpm wrote: `installed.*`, the autoload files, the bin proxies and each placed
package directory. Any mismatch takes the full path, which skips packages
already in place, so a stale state file costs time, never correctness.

Known gap: a tag added to the current commit in a packed-refs-only repo
changes Composer's root version without changing what the state file sees.

### Class scan cache

Store trees never change, so `-o` scans are cached per tree in
`<cache>/classes/v1/<build>/<vendor~pkg>/<reference>`, keyed by the phpm
binary's size and mtime so a scanner fix never reads stale results. Only
packages placed in this run use it; packages left in place are scanned from
`vendor/`, in case someone edited them. This needed one addition to
`phpm-autoload`'s API: `Options::known_classes` and `scan::file_classes`.
Warm `-o` autoload on the Laravel skeleton went from about 315 ms to 90 ms.

## Byte-identity

`just e2e` installs each fixture with `composer install --no-scripts
--no-plugins` and with `phpm install --no-scripts --no-plugins` (twice: a
first install, then again from the store with cached class scans) in
sibling temp directories outside the repo, and compares the whole `vendor/`
with `phpm-diffvendor`, bytes and modes, nothing ignored.

| Fixture | Extra flags (both tools) | Differences |
|---|---|---|
| laravel-skeleton | none (config has `optimize-autoloader`) | 0 |
| laravel-skeleton | `--no-dev` | 0 |
| laravel-skeleton | `--classmap-authoritative` | 0 |
| ytmate | none | 0 |
| ytmate | `--no-dev` | 0 |
| wicketyaari | none | 0 |
| symfony-demo | none (plugins disabled on both) | 0 |
| monica | `--ignore-platform-reqs` (this Mac lacks an extension monica needs) | 0 |

## Benchmarks

`PHPM_BIN=... VIVACITY_BIN=... tools/bench/bench.sh <fixture> composer phpm vivacity`,
2026-10-01, Apple M1 Pro, macOS 26.5.1, APFS, Composer 2.10.3 on PHP 8.4.13,
vivacity 0.19.1 (release binary, SHA-256 checked), phpm release build of this
branch, hyperfine 1.20.0. Cold n=3 (all caches deleted), warm n=5 (`vendor/`
deleted, caches kept), no-op n=10. Mean ± σ.

### laravel-skeleton (109 packages, 8,860 files)

| Tool | Cold | Warm | No-op |
|---|---|---|---|
| Composer | 11.98 s ± 1.43 | 4.70 s ± 0.94 (min 4.21) | 2.03 s ± 0.98 (min 1.50) |
| vivacity | 7.45 s ± 1.60 | 610 ms ± 56 | 400 ms ± 13 |
| phpm | 5.13 s ± 0.52 | **288 ms ± 60** (min 240) | **4.7 ms ± 0.2** |

### ytmate (42 packages)

| Tool | Cold | Warm | No-op |
|---|---|---|---|
| Composer | 7.37 s ± 0.73 | 3.46 s ± 0.12 | 1.04 s ± 0.02 |
| vivacity | 6.30 s ± 0.42 | 653 ms ± 218 | 403 ms ± 19 |
| phpm | 6.05 s ± 0.61 | **185 ms ± 7** | **4.4 ms ± 0.3** |

Warm: phpm is 16x Composer and 2.1x vivacity on Laravel, 19x and 3.5x on
ytmate. No-op: about 430x Composer and 85x vivacity. Cold installs are
network-bound for every tool.

### Where the warm time goes

`phpm install -v` on the Laravel skeleton, warm, about 220-280 ms in all:

- placing 109 packages: 110-140 ms. One `clonefile` per package directory;
  the kernel serialises them, so 1 thread and 32 threads take the same time.
- autoloader with `-o`: about 90 ms. With scans cached, what is left is
  walking the autoload directories and writing about 2 MB of PHP.
- guessing the root version: 35-40 ms of `git` processes, run on a thread
  while packages are placed, so it costs nothing.
- `installed.json`/`installed.php`: 5-20 ms; bin proxies: 3 ms; process
  start: about 3 ms.

## Not done in this milestone

- Riff and viv were not re-run here; `competitors.md` has their numbers.
- No Linux container run yet (gate criterion 5).
- Bins with the same name in two packages: the first in lock order wins;
  Composer's winner is the first in its install order. No fixture has a
  clash.
