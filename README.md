# phpm

[![ci](https://github.com/speedsharmaai/phpm/actions/workflows/ci.yml/badge.svg)](https://github.com/speedsharmaai/phpm/actions/workflows/ci.yml)
[![Quality Gate](https://sonarcloud.io/api/project_badges/measure?project=speedsharmaai_phpm&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=speedsharmaai_phpm)
[![Coverage](https://sonarcloud.io/api/project_badges/measure?project=speedsharmaai_phpm&metric=coverage)](https://sonarcloud.io/summary/new_code?id=speedsharmaai_phpm)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/speedsharmaai/phpm/badge)](https://scorecard.dev/viewer/?uri=github.com/speedsharmaai/phpm)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#licence)

An extremely fast, Composer-compatible PHP installer, written in Rust. It reads
your `composer.json` and `composer.lock` unchanged and writes the same
`vendor/`, byte for byte, in a fraction of the time on every install that is
not bound by the network.

> **Status: pre-release.** phpm passed its Phase 01 gate and finished
> Phase 02 on 2026-10-01. `phpm install` works on any lockfile with dist
> archives: string scripts run natively, and plugins or PHP-callable scripts fall back to real
> Composer for the steps that need them (`--explain` says which). There is no
> release or installer yet, so it builds from source only.

## Now

*Updated 2026-10-01.* Phases 03 and 04 run in parallel.

- **Phase 03, compatibility sweep (in progress).** 330 open-source PHP
  projects pinned from GitHub search. Every night CI installs each one with
  Composer and with phpm and compares the trees byte for byte. First full run
  ([results page](https://speedsharmaai.github.io/phpm/)): **269 of 312
  installable projects identical (86.2%)** with scripts and plugins off, 199
  of 280 (71.1%) with them on. The gate is 95%. The sweep has already found
  and fixed four bugs (dist URL placeholders, a repository named `packagist`,
  plugins Composer cannot load, and Composer's own run-to-run differences
  being counted against phpm); the next run includes those fixes.
- **Phase 04, plugin adapters (started).** Native support for
  composer/installers (WordPress, Drupal paths), Drupal scaffold,
  symfony/runtime, phpstan's extension installer and the PHPCS installer, so
  Bedrock and Drupal install without falling back to Composer.

## Numbers

Laravel skeleton, 109 packages, M1 Pro, APFS, Composer 2.10.3, hyperfine,
phpm at the end of Phase 02, every tool measured in the same session
([raw results](bench/results/2026-10-01-phase-02-close/),
[progress](phases/phase-02-parity-core/progress.md)):

| Tool | Cold (median of 8) | Warm install | No-op | `vendor/` vs Composer |
|---|---|---|---|---|
| Composer 2.10.3 | 21.08 s | 4.179 s | 1.606 s | reference |
| riff 0.0.7 | 13.17 s | 2.486 s | 1.514 s | 4 files differ |
| viv 0.20.0 | not run | 2.215 s | 8.8 ms | every file mode differs |
| vivacity 0.19.1 | 14.21 s | 684 ms | 470 ms | identical |
| **phpm** | **8.67 s** | **222 ms** | **4.5 ms** | **identical** |

Cold is network-bound and noisy from here (India, home broadband), so it
comes from a separate run with the tools interleaved, 8 rounds
([cold results](bench/results/2026-10-01-cold-interleaved/)). On the ytmate
fixture phpm's cold median is 11.65 s against riff's 13.19 s, a tie within
the noise: its time is one 6.9 MB archive that every tool waits for.

With the skeleton's scripts on (`package:discover`, a PHP callable, so that
event goes through real Composer) and the malware filter checked, phpm
installs warm in 0.87 s against Composer's 4.57 s (median, 5.3x; 5.9x in a
quieter 10-run rerun).

On Linux (GitHub `ubuntu-latest`, ext4) phpm was 14x faster than Composer
warm and 633x on a no-op at the Phase 01 gate.

`vendor/` is byte-identical to Composer's, bytes and file modes, on all five
fixtures (Laravel, Symfony demo, Monica, and two production apps), including
the optimised class map; the plugin-heavy apps (Symfony, Drupal, Bedrock,
Monica) install through the Composer fallback with 0 differences.

## Try it from source

```sh
cargo build --release -p phpm
cd /path/to/your/php/project
/path/to/phpm/target/release/phpm install --explain
```

## Roadmap

| Phase | What | State |
|---|---|---|
| 00 | Quality foundation: lints, hooks, CI on 3 OSes, coverage, CodeQL, Scorecard, Sonar | done |
| 01 | Spike: `phpm install`, byte-identical, benchmarked, gate | **passed** |
| 02 | Parity core: platform checks, auth, scripts, malware filter, Composer fallback, faster cold fetch | **done** |
| 03 | Compatibility sweep: nightly diff against Composer across 330 projects | in progress: 86.2% identical, gate 95% |
| 04 | Plugin adapters: composer/installers, Drupal scaffold, symfony/runtime, phpstan installer | in progress |
| 05 | Real-world benchmarks: Composer vs phpm on the largest open-source PHP apps | |
| 06 | Launch: release, Homebrew, setup-php, Docker, GitHub Action | |
| 07 | Resolver: `update` and `require` | |

Full plan: [phases](phases/README.md) · [decisions](docs/decisions/README.md) ·
[topology](docs/arch/topology.md) · [research](docs/research/)

## Repository

```text
crates/
  phpm            the binary
  phpm-lock       composer.json / composer.lock, installed.json, installed.php
  phpm-php        byte-exact PHP json_encode and var_export writers
  phpm-store      fetch, global store, extraction, clonefile placement
  phpm-autoload   Composer's autoload files and class scanning
  phpm-diffvendor compare two vendor/ trees byte for byte, modes included
  phpm-testkit    test helpers
fixtures/         pinned lockfiles the gate is measured on
bench/results/    hyperfine JSON behind every published number
tools/bench/      hyperfine harness: Composer vs phpm vs other installers
```

Build and check everything the way CI does:

```sh
just ci
```

See [CONTRIBUTING](CONTRIBUTING.md).

---

## Product thesis

Composer is good. That is the first thing to accept. It has a lockfile,
parallel downloads, one blessed registry, and a maintainer team that is
respected. The Mago maintainer, the one person best placed to build this,
declined in 2025 and called Composer "great". Most PHP developers would agree.

So the question is not "is Composer slow". It is "where is it slow enough that
someone would install a second tool".

Measured on an M1 Pro, Composer 2.10.3, fresh Laravel skeleton, 109 packages:

| Step | Composer | Floor measured | Where the time goes |
|---|---|---|---|
| cold install | 12-16 s | network | downloading 17 MB of zips |
| warm install | 4.1-4.4 s | **0.14-0.24 s** | re-unzipping 8,860 files from the cache on every install |
| no-op install | 1.6 s | milliseconds | PHP boot, platform solve, filter-list revalidation |
| autoload dump | ~0.9 s | ~0.1 s | single-threaded class scanning |

The floor is `clonefile()` of pre-extracted package directories from a global
store, one syscall per package. That is uv's design, and it is the whole
product. Cold installs stay network-bound; nobody fixes bandwidth. See
[benchmarks](docs/research/benchmarks-2026-10-01.md).

Three answers to "why would anyone use ours", in order of how much they matter:

**1. It never breaks a project.** Every competitor either refuses plugins,
emulates a few, or breaks silently. phpm installs natively when it can prove
the result is identical to Composer's, and hands the rest to real Composer
when it cannot. A user should never be worse off for having tried it. See
[decision 0004](docs/decisions/0004-byte-identical-or-fall-back.md).

**2. Compatibility is published, not claimed.** A nightly sweep diffs phpm's
`vendor/` against Composer's across hundreds of real lockfiles and publishes
the number, the way Ruff published ">99.9% Black-compatible". Nobody in the
field does this at scale. It is the only credible answer to "why trust a new
binary on my install path".

**3. It is built for where installs actually multiply now.** Packagist went
from about 3 billion installs a month in January 2026 to over 5 billion in
September, and Packagist credits AI coding tools. Agents run installs in CI,
in containers, and in parallel git worktrees, each of which wants its own
`vendor/`. With a shared store, a new worktree's `vendor/` costs a fraction of
a second and almost no disk. Humans run Composer a few times a day; agents run
it constantly.

**How it earns.** It probably does not, and that is stated here so nobody is
surprised later. Private registries fund Composer itself (Private Packagist),
and Astral wound down its own paid registry after the OpenAI acquisition. phpm
is a reputation and distribution project for the speedsharma brand. If a
business appears, it will be in CI and container caching, and not before the
tool has users.

**Where the users come from.** Laravel first: 64% of PHP developers use it,
and the default skeleton has zero Composer plugins. Then GitHub Actions via
setup-php, which already installs any Packagist tool. Then Docker image
builds. See [market](docs/research/market-and-competitors.md).

---

## Rules

**1. Phase 01 is a spike and it can kill the project.**
Warm install at least 10x faster than Composer on the Laravel skeleton and five
real apps, with a byte-identical `vendor/`, measured head to head against riff
and vivace. If it is not clearly ahead, the project stops, or becomes a
contribution to whichever of them is. See [the gate](phases/README.md#the-gate).

**2. Byte-identical or fall back.**
The output files that other tools parse (`installed.json`, `installed.php`,
the autoload family, bin proxies) match Composer's bytes. When phpm cannot
guarantee that, it runs Composer for the part it cannot do, and says so in one
line. It never guesses.

**3. Install before update.**
`install` from a lockfile needs no resolver and no registry metadata. That is
where the speed is and where Phase 01 lives. `update` and `require` need a
resolver whose choices must match Composer's, and that is Phase 07 at the
earliest. See [decision 0002](docs/decisions/0002-lockfile-install-first.md).

**4. A good Packagist citizen.**
User-Agent with a contact, concurrency within the published limits, download
notifications sent so package authors still get their numbers, and Composer
2.10's malware filter honoured. Packagist is funded by the same people who
build Composer; phpm does not make their bill bigger. See
[decision 0006](docs/decisions/0006-good-packagist-citizen.md).

**5. Every number is reproducible.**
Benchmarks run through hyperfine with a published script, pinned lockfiles,
cold and warm defined in writing, filesystem and OS stated. No blog-post
numbers. Half the "Composer takes minutes" posts found in research were
content marketing with wrong facts in them.

---

## Non-negotiables

- **No resolver in Phase 01-06.** A resolver that picks different versions
  than Composer silently changes what runs in production.
- **No symlinks into the store by default.** Tools that expect real files in
  `vendor/` break, and clearing the cache would break installs. Clone on macOS
  and Linux, hardlink on Windows, copy as the fallback.
- **Plugins are never emulated partially.** An adapter is either proven
  identical by the sweep, or the package goes to Composer.
- **No paid registry.** It would compete with the thing that funds Composer.
- **No launch without the compatibility number.** The launch post leads with
  "identical `vendor/` on N of M projects", not with speed.

## Stack

| Concern | Choice | Why |
|---|---|---|
| Language | Rust | single static binary, no PHP needed for the fast path; same as uv, Ruff, Mago |
| Async I/O | tokio + reqwest (rustls, HTTP/2) | bounded parallel downloads |
| Extraction | `zip` crate on rayon, in-memory buffers | PHP packages are small; streaming unzip only if profiling asks for it |
| Store | content-addressed by `(name, dist.reference)`, append-only, atomic rename | concurrent-safe, shared across projects and worktrees |
| Linking | `libc::clonefile` per package dir (macOS), FICLONE/hardlink (Linux), hardlink (Windows), copy fallback | measured 0.14 s for 109 packages vs 2.6 s of unzip |
| JSON | serde_json + a byte-exact PHP `json_encode` writer | installed.json and content-hash must match PHP's bytes |
| PHP output | a `var_export`-compatible writer | autoload_static.php and installed.php |
| Class scanning | `mago-syntax` lexer, results cached per file in the store | files in the store never change, so warm installs skip scanning |
| Platform | run `php` once, cache the answer | versions, extensions, lib versions |
| Resolver (Phase 07) | `pubgrub` | uv's resolver; better conflict messages than a SAT port |
| Distribution | cargo-dist: GitHub Releases, curl installer, Homebrew, npm wrapper; plus a Packagist wrapper for setup-php | every channel PHP developers already use |
| Benchmarks | hyperfine, JSON output committed | reproducible or it did not happen |

## Licence

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT),
at your option.
