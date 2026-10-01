# Worklist

The one-screen view. Details live in [phases](phases/README.md).

Current state: **Phase 00 done. Phase 01 spike: metadata and store merged,
autoload and the install command in progress.**

## Next three things

1. **Finish the install path.** Class scanning for `-o` (milestone 4), then
   `phpm install` with bin proxies and the no-op fast path (milestone 5).
2. **Run the gate.** `tools/bench/bench.sh` for Composer, phpm and vivacity on
   every fixture, `diffvendor` on every `vendor/`, then write
   `phases/phase-01-spike/gate.md` with the five numbers and the call.
3. **Rename fixture lockfiles** so GitHub's dependency graph stops treating
   them as dependencies (Dependabot opened six PRs against them; automated
   security-fix PRs are paused until this lands).

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
- [ ] Class scanning for `-o` and classmap autoloads
- [ ] `phpm install`, bin proxies, no-op fast path
- [ ] **Gate: write `gate.md` with the table and the call**

## Phase 02 and beyond

Outlined, not tasked. Phase 02's order depends on what the gate and the diff
harness show. See [phases/README.md](phases/README.md).
