# Baseline benchmarks, 2026-10-01

Machine: the owner's Mac, Apple Silicon, APFS. Composer 2.10.3, PHP 8.4.13.
Fixture: `laravel/laravel` skeleton, fresh `composer update`, 76 prod + 33 dev
= 109 packages, `vendor/` 69 MB, 8,860 files. Home internet.

Rough timings with a Python timer, n = 1 to 3. The spike redoes all of this
with hyperfine; these numbers only establish that the gap exists.

## Composer

| Scenario | Time |
|---|---|
| cold cache, no scripts | 16.1 s, 11.9 s |
| warm cache, empty vendor, no scripts | 5.8 s, 4.1 s, 4.4 s |
| warm + `-o` | 4.1 s (Laravel sets `optimize-autoloader: true` already) |
| warm, with scripts (`package:discover`) | 5.8 s |
| no-op, vendor present | 1.6 s |
| `dump-autoload -o` alone | 0.97 s |

`--profile` of a warm install:

```text
0.10 s  start
0.64 s  platform verified, "Dependency resolution completed in 0.000 seconds"
0.75 s  extraction starts (async unzip)
3.29 s  "Generating optimized autoload files"
4.16 s  done
```

So: about 0.6 s of fixed PHP cost, about 2.6 s of extraction, about 0.9 s of
autoload generation.

## The floor

Same 109 packages, already extracted in a store directory:

| Method | Time |
|---|---|
| serial `unzip` of all 109 cached zips | 3.78 s |
| `cp -R` store → vendor | 4.82 s |
| `cp -Rc` (clonefile per file) | 2.31 s, 2.94 s, 3.29 s |
| `rsync --link-dest` (hardlink per file) | 6.09 s |
| `clonefile()` per package directory (109 syscalls) | **0.143 s, 0.238 s, 0.162 s** |

Per-directory clone is 11-18x faster than Composer's extraction step alone.
Add a native autoload writer (~0.1 s with cached scans) and metadata writes,
and a warm install lands around 0.3-0.4 s against Composer's 4.1 s. That is
the 10x the gate asks for, on macOS. Linux without reflink will be less.

## Independent numbers from research

| Source | Fixture | Composer | Other | Notes |
|---|---|---|---|---|
| research agent, Docker Linux VM | laravel skeleton | cold 8.9-19.6 s, warm 2.4-2.7 s, no-op 1.2 s | tar-extract floor 0.30 s, hardlink copy 0.13 s | bind-mount free |
| research agent, Docker Linux VM | monicahq/monica, 241 pkgs | cold 16-27 s, warm 4.1-4.5 s, no-op 1.5 s | floor 0.62 s | |
| research agent, Docker Linux VM | magento2, 214 pkgs | cold 27 s, warm 3.2 s, no-op 0.9 s | floor 0.89 s | |
| shyim/riff README, EPYC, hyperfine | symfony/demo, 153 pkgs, no plugins/scripts | cold 7.80 s, warm 1.99 s | riff cold 1.71 s, warm 0.35 s | the competitor to beat |

Raising `COMPOSER_MAX_PARALLEL_HTTP` from 12 to 48 made no difference cold on
a home connection: cold installs are bandwidth-bound. Do not promise cold
speedups beyond what better connection reuse gives.

## What this says about the pitch

- Warm and no-op are where the multiples are. Cold is not.
- The absolute saving per install is seconds. It matters where installs
  multiply: CI matrices, container builds, agents in parallel worktrees,
  Docker-on-Mac bind mounts (one developer reported five-minute
  `dump-autoload` runs there).
- The launch benchmark must show all three scenarios, and say which
  filesystem.
