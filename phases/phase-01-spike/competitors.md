# Competitors, measured

2026-10-01, the owner's Mac (M1 Pro, APFS), Composer 2.10.3, PHP 8.4.13.
Fixture `laravel-skeleton` (109 packages). hyperfine: cold n=3 (all caches
deleted), warm n=5 (caches kept, `vendor/` deleted), no-op n=10 (`vendor/`
present). Release binaries from each project's GitHub release, SHA-256
verified against the published checksums.

| Tool | Version | Cold | Warm | No-op | `vendor/` vs Composer |
|---|---|---|---|---|---|
| Composer | 2.10.3 | 13.5 s ± 4.0 | 4.28 s ± 0.23 | 1.53 s ± 0.03 | reference |
| riff | 0.0.7 | 5.5 s ± 0.3 | 2.46 s ± 0.19 | 1.37 s ± 0.22 | 4 files differ: `installed.json`, `installed.php`, `InstalledVersions.php`, `LICENSE` |
| viv (vivace) | 0.20.0 | 9.5 s ± 2.5 | 2.70 s ± 0.03 | **9.3 ms** | 8,845 differences: every file mode 444 (read-only hardlinks) where Composer writes 644; `LICENSE` differs; extra `.vivace-state` |
| vivacity | 0.19.1 | 8.1 s ± 1.9 | **0.76 s ± 0.46** (min 0.54) | 368 ms ± 13 | **identical**, bytes and modes |

Composer was run with `--no-scripts --no-plugins`, riff and viv likewise.
vivacity has no such flags; the fixture has no plugins, and the run did not
execute scripts.

## What each gets right and wrong

**vivacity** is the one to beat. Byte-identical on Laravel, including file
modes, and the fastest warm install at roughly 5.6x Composer. Its no-op is
368 ms, which suggests it re-checks something over the network or rescans on
every run. Apache-2.0, splits into reusable crates, active (created
2026-09-10, last push 2026-09-30).

**viv** has the fastest no-op in the field (9 ms, a state file) and the most
features (workspaces, audit, `x`, a PHP downloader). Its warm install is slow
for a native tool: 2.7 s with 14.8 s of system time, one hardlink syscall per
file. Hardlinking makes every file read-only, which changes 8,845 file modes
against Composer and breaks anyone who patches `vendor/`.

**riff** is the broadest Composer clone (almost every command, flex and
patches adapters), but on this fixture it is only 1.7x faster warm, barely
faster on no-op, and its metadata files differ from Composer's.

## Against the gate

| Gate criterion | Best competitor | Met? |
|---|---|---|
| 1. warm ≥ 10x Composer (≤ 0.43 s) | vivacity 0.54-0.76 s | no |
| 2. `vendor/` byte-identical | vivacity | yes |
| 3. no-op < 50 ms | viv 9.3 ms | yes |
| 1-3 together, one tool | none | no |

No competitor meets criteria 1-3 together, so the spike goes ahead as
planned (decision 0007).

What phpm has to show to be clearly ahead, on the same machine and fixture:

- warm ≤ 0.40 s, which is 1.4-1.9x vivacity's
- no-op within a few milliseconds of viv's 9.3 ms
- zero differences from `diffvendor`, modes included

If phpm lands behind vivacity on warm installs or does not match it on
identity, the call in decision 0007 applies: stop, and contribute the
benchmark harness and `diffvendor` to vivacity.

## Reproduce

```sh
tools/bench/bench.sh laravel-skeleton composer riff viv vivacity
cargo run -p phpm-diffvendor --release -- \
  target/bench/laravel-skeleton/composer/vendor \
  target/bench/laravel-skeleton/vivacity/vendor
```
