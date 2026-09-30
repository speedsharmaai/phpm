# Worklist

Nothing is built. Research is written, a baseline is measured on this Mac,
the name is provisionally clear, and Phase 00 is scoped.

Current state: **docs written, baseline measured, spike not yet coded.**

## Next three things

1. **Read riff and vivace properly.** Clone both, run them on the Laravel
   skeleton and symfony/demo, and write down what each gets right and wrong.
   If either is already where phpm wants to be, that changes Phase 00 into
   "contribute there" and the project is over before it costs anything.
2. **Confirm the name.** crates.io, npm, Homebrew and GitHub were free for
   `phpm` on 2026-10-01. Run a USPTO/EUIPO search, then reserve the crate and
   the npm name with placeholder publishes (owner action, brand account).
3. **Collect the fixture set.** The Laravel skeleton, symfony/demo, and five
   real lockfiles, two of them from our own PHP projects (ytmate,
   wicketyaari). Pinned in `fixtures/`, never updated during the spike.

Then write the spike, run the gate, and write the decision down.

## Now

- [x] Research: Composer internals, market and competitors, uv architecture, naming
- [x] Baseline benchmark on this Mac (Composer 2.10.3, PHP 8.4, APFS)
- [x] Proof the floor is real: 109 package dirs cloned in 0.14-0.24 s
- [x] Decisions 0001-0007
- [x] Phase 00 scoped and broken into tasks; phases 01-05 outlined
- [ ] riff and vivace read and run
- [ ] Trademark search and name reservation
- [ ] Fixture set collected

## Phase 00 · Spike

- [ ] Cargo workspace, one binary, `phpm install` only
- [ ] Lockfile parse and diff against `vendor/composer/installed.json`
- [ ] Parallel dist download into the store, GitHub top-dir strip, exec bits kept
- [ ] Per-package clone into `vendor/`
- [ ] `installed.json`, `installed.php`, `InstalledVersions.php` byte-identical
- [ ] Autoload family (non-optimised) byte-identical
- [ ] Bin proxies byte-identical
- [ ] No-op fast path
- [ ] Diff harness: phpm vs Composer `vendor/` on every fixture
- [ ] hyperfine harness: Composer vs riff vs vivace vs phpm, cold, warm, no-op
- [ ] **Gate: write `gate.md` with the table and the call**

## Phase 01 and beyond

Outlined, not tasked. What Phase 01 contains depends on which files the diff
harness shows are hard to match, and which plugins the fixtures actually hit.
See [phases/README.md](phases/README.md).
