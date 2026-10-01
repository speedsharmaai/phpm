# Worklist

The one-screen view. Details live in [phases](phases/README.md).

Current state: **Phase 01 gate passed (2026-10-01). Phase 02 next.**

## Next three things

1. **Phase 02 tasks.** Write `phases/phase-02-parity-core/tasks.md` from the
   scope plus what the gate found: Composer fallback for plugins and scripts,
   platform checks, auth, malware filter and audit, faster cold fetch,
   fuzzing.
2. **Rename fixture lockfiles** so GitHub's dependency graph stops treating
   them as dependencies; then turn automated security-fix PRs back on.
3. **Market check (owner).** Ask r/PHP, r/laravel and ten developers whether
   install time is a top-three pain. Decides whether Phase 06 has an audience.

## Owner actions

- [x] `SONAR_TOKEN` added; Sonar quality gate runs on every PR
- [x] SSH signing key registered; `main` requires verified signatures
- [ ] OpenSSF Best Practices registration at bestpractices.dev
- [ ] Trademark search for `phpm`

## Phase 01 · Spike

- [x] Competitors measured: riff, viv, vivacity ([competitors](phases/phase-01-spike/competitors.md))
- [x] Fixtures, hyperfine harness, `diffvendor`
- [x] PHP writers: `json_encode`, `var_export`, `dumpToPhpCode` (#12, #16)
- [x] Version normalisation port (#13)
- [x] `installed.json`, `installed.php`, `InstalledVersions.php` byte-identical on 5/5 fixtures (#14)
- [x] Fetch, global store, extraction, clonefile placement, pruning (#17)
- [x] Autoload files without class scanning (#18)
- [x] Class scanning for `-o` and classmap autoloads; autoload files byte-identical on 5/5 fixtures, dev and `--no-dev` (#20)
- [x] `phpm install`, bin proxies, no-op fast path, class scan cache (#22, #24)
- [x] **Gate passed: [gate.md](phases/phase-01-spike/gate.md)**

## Phase 02 and beyond

Outlined, not tasked. Phase 02's order depends on what the gate and the diff
harness show. See [phases/README.md](phases/README.md).
