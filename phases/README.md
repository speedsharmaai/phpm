# Phases

Phase 00 is written in full. Phases 01-05 are outlined with a scope each,
not tasked. Two reasons.

The first is the gate. Phase 00 can end the project, and tasking five phases
of work that may never happen is writing fiction and then feeling committed
to it.

The second is that the order of Phase 01 depends on evidence. Which
Composer-written files turn out hard to match byte for byte, and which
plugins the fixtures actually hit, decides what Phase 01 has to contain.

| # | Phase | Ships | Weeks | Gate |
|---|---|---|---|---|
| 00 | [Spike](phase-00-spike/scope.md) | `phpm install` on plugin-free lockfiles, byte-identical, benchmarked against Composer, riff and vivace | 2 | **yes, can kill** |
| 01 | [Parity core](phase-01-parity-core/scope.md) | `-o` class scanning, platform checks, auth, path repos, malware filter, scripts, Composer fallback | 3 | no |
| 02 | [Compatibility sweep](phase-02-compat-sweep/scope.md) | nightly `vendor/` diff across hundreds of lockfiles, a public number, Linux and Windows | 2 | yes: ≥ 95% identical |
| 03 | [Plugin adapters](phase-03-plugin-adapters/scope.md) | native composer/installers, symfony/runtime, phpstan/extension-installer, php-http/discovery; flex stays on fallback | 3 | no |
| 04 | [Launch](phase-04-launch/scope.md) | cargo-dist release, Homebrew, setup-php wrapper, Docker image, GitHub Action, benchmark page, launch post | 2 | yes: 30-day adoption |
| 05 | [Resolver](phase-05-resolver/scope.md) | `update` and `require` with pubgrub, lockfile byte-identical to Composer's | 4+ | only if 04 passes |

About 12 weeks to a launch if every gate passes. The field is re-checked at
each phase boundary (decision 0007).

## The gate

**Phase 00 ends in a gate, not a release.**

Five numbers, written into `phase-00-spike/gate.md`, on the fixed fixture set:

1. **Warm install, Laravel skeleton, macOS:** phpm at least **10x** faster
   than Composer (Composer measured 4.1 s on 2026-10-01, so phpm ≤ 0.4 s).
2. **Byte-identity:** `vendor/` identical to Composer's on **every**
   plugin-free fixture. Not most. Every file, bytes and mode.
3. **No-op:** under 50 ms.
4. **Head to head:** faster than riff and vivace on warm and no-op, on the
   same machine, same fixtures, same flags.
5. **Linux:** at least 3x faster warm in a Docker Linux container, because
   that is where CI runs and there is no clonefile.

Then the call:

- All five pass → Phase 01 starts.
- 1-3 pass but 4 fails → phpm is not ahead. Stop, and contribute the
  diff harness and benchmark suite to the better project (decision 0007).
- 1 or 2 fails → the premise is wrong. Stop. Write down why.
- 5 fails alone → continue, but the pitch becomes macOS and worktrees first,
  and Phase 02 carries a Linux performance task.

Separately, before Phase 01 starts: ask in r/PHP and r/laravel, and to at
least ten developers directly, whether install time is a top-three CI or
local pain. Docker-on-Mac teams first. Fewer than ten yes answers means
Phase 04's launch has no audience, and that is worth knowing at week 2
instead of week 12.

Writing the decision down is part of the gate.

## Why the spike is not the product

The spike installs plugin-free lockfiles, without `-o`, on the owner's Mac and
in one Linux container. No fallback, no auth, no platform verification
beyond what the fixtures need, no Windows, no release. Everything that makes
it a product is real work, and all of it is wasted if phpm is not clearly
faster and clearly identical on the easy case.
