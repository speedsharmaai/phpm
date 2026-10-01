# Phase 01 gate

Decided 2026-10-01. **Pass: all five criteria met. Phase 02 starts.**

## The numbers

macOS: M1 Pro, macOS 26.5.1, APFS, Composer 2.10.3 on PHP 8.4.13, phpm at
`bc2a160` (release build), riff 0.0.7, viv 0.20.0, vivacity 0.19.1, release
binaries checked against their published SHA-256 sums. Linux: GitHub
`ubuntu-latest`, ext4, Composer 2.10.3 on PHP 8.4, run
[36816577836](https://github.com/speedsharmaai/phpm/actions/runs/36816577836).
hyperfine via `tools/bench/bench.sh`; cold n=3 (every cache deleted), warm
n=5 (caches kept, `vendor/` deleted), no-op n=10. Every tool ran with
`--no-scripts --no-plugins` where it has those flags. Raw JSON in
[`bench/results/`](../../bench/results/).

### laravel-skeleton (109 packages), macOS

| Tool | Cold | Warm | No-op |
|---|---|---|---|
| Composer | 13.11 s ± 0.56 | 4.135 s ± 0.085 | 1.509 s ± 0.019 |
| riff | 5.80 s ± 0.63 | 2.192 s ± 0.039 | 1.293 s ± 0.020 |
| viv | 7.70 s ± 0.34 | 2.071 s ± 0.048 | 9.2 ms ± 0.4 |
| vivacity | 7.70 s ± 0.51 | 582 ms ± 24 | 407 ms ± 24 |
| **phpm** | 7.96 s ± 1.51 | **229 ms ± 16** | **4.9 ms ± 0.3** |

### ytmate (42 packages), macOS

| Tool | Cold | Warm | No-op |
|---|---|---|---|
| Composer | 8.13 s ± 1.04 | 3.308 s ± 0.118 | 1.077 s ± 0.027 |
| riff | 4.69 s ± 0.42 | 1.592 s ± 0.026 | 917 ms ± 21 |
| viv | 6.10 s ± 0.83 | 1.657 s ± 0.028 | 7.0 ms ± 0.4 |
| vivacity | 5.56 s ± 0.53 | 513 ms ± 22 | 426 ms ± 18 |
| **phpm** | 5.60 s ± 0.07 | **197 ms ± 11** | **4.8 ms ± 0.6** |

### Linux (GitHub ubuntu-latest, ext4, hardlinks)

| Fixture | Scenario | Composer | phpm | Speed-up |
|---|---|---|---|---|
| laravel-skeleton | cold | 3.918 s | 1.613 s | 2.4x |
| laravel-skeleton | warm | 1.762 s | 126 ms | 14.0x |
| laravel-skeleton | no-op | 931 ms | 1.5 ms | 633x |
| ytmate | cold | 2.500 s | 2.187 s | 1.1x |
| ytmate | warm | 1.131 s | 142 ms | 8.0x |
| ytmate | no-op | 428 ms | 1.2 ms | 355x |

### Identity

`diffvendor` on the whole `vendor/` tree, bytes and modes, nothing ignored,
phpm against `composer install --no-scripts --no-plugins`: **0 differences**
on laravel-skeleton (default, `--no-dev`, `--classmap-authoritative`), ytmate
(default, `--no-dev`), wicketyaari, symfony-demo and monica (both tools with
`--ignore-platform-reqs`, which this Mac's PHP needs for monica). Checked on a
first install and again on a reinstall from the store. Details in
[progress](progress.md).

## Against the criteria

| # | Criterion | Result | |
|---|---|---|---|
| 1 | Warm install, laravel-skeleton, macOS: ≥ 10x Composer | 4.135 s / 229 ms = **18.0x** | pass |
| 2 | `vendor/` byte-identical on every plugin-free fixture | 0 differences on all 5 | pass |
| 3 | No-op under 50 ms | **4.9 ms** | pass |
| 4 | Faster than riff and vivace on warm and no-op | warm 2.5x vivacity (best rival), no-op 1.9x viv (best rival) | pass |
| 5 | Linux: ≥ 3x faster warm | **14.0x** and **8.0x** | pass |

## What the numbers also say

- **Cold installs are not a win.** On macOS phpm is level with vivacity and
  slower than riff (ytmate 5.60 s vs 4.69 s), and phpm's laravel cold run has
  a 1.5 s spread. Cold is mostly network, as expected, but riff shows there is
  room. Phase 02 gets a task: profile cold fetches (connection reuse,
  per-host concurrency, extract-while-downloading).
- **Linux is fine without clonefile.** Hardlinks on ext4 give 8-14x warm. The
  macOS number is not a clonefile trick that disappears in CI.
- **The no-op number is the agent and CI story.** 1-5 ms against Composer's
  0.4-1.5 s means a worktree or a CI step can run `phpm install` every time
  and pay nothing when nothing changed.
- **Warm time is now filesystem-bound.** Of laravel's 229 ms, roughly
  110-140 ms is placing 109 package directories, and the kernel serialises
  `clonefile`; more threads do not help.

## Not yet done (carried into Phase 02, not gate criteria)

- The market check in [phases/README.md](../README.md#the-gate): ask r/PHP,
  r/laravel and ten developers whether install time is a top-three pain.
  This needs the owner; it does not block engineering but it decides whether
  Phase 06 has an audience.
- Plugins and scripts: phpm refuses them today (Phase 02 adds the Composer
  fallback).
- Composer 2.10's malware filter and audit (Phase 02, release blocker).

## Call

Pass. phpm is ahead of every installer measured on warm installs and no-op,
and byte-identical where the others are not (riff and viv both differ from
Composer). Phase 02 starts. Decision 0007's "contribute to the leader
instead" does not apply: phpm is the leader on the criteria that were set
before the code was written.
