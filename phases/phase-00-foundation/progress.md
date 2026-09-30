# Phase 00: Progress

**Status:** done except two owner actions (Sonar token, signing key) and the
`main` ruleset, which is applied right after PR #1 merges.

## Definition of done

- [x] `just ci` passes from a fresh clone (exit 0)
- [x] CI green on Linux, macOS and Windows; one `ci-ok` aggregate (PR #1)
- [x] Bad commit message rejected by the commit-msg hook
- [x] Bad PR title fails the title check
- [x] A clippy warning, an unformatted file, a GPL dependency and an unused
      dependency each fail their gate
- [x] CodeQL (rust, actions) runs and passes
- [x] Private vulnerability reporting, secret scanning, push protection,
      Dependabot alerts and security updates on
- [x] Squash merge only, PR title as the commit, blank body, branches
      deleted on merge
- [ ] Sonar dashboard populated (needs `SONAR_TOKEN`)
- [ ] Ruleset on `main` (after PR #1 merges, so the check names exist)
- [ ] Scorecard first run (runs on the first push to `main`)

## Verification

```text
local   just ci                        exit 0
        line coverage                  97.87% (floor 80%)
        fresh clone just ci            exit 0, target/ 25 MB

hooks   "Added some stuff."            committed: Missing type in the commit summary
        clippy: Some(1).map(|x| x)     unnecessary map of the identity function
        fn version_line()->String{     rustfmt: Diff in crates/phpm/src/main.rs
        unused itoa dependency         cargo-shear: unused dependency `itoa`
        GPL-3.0-only path crate        cargo-deny: rejected, license is not explicitly allowed

PR #1   17 checks                      all pass, commit-message skipped (push-only)
        title "Set up tooling."        fail: No release type found in pull request title
```

## What the checks caught

- **taiki-e/install-action does not carry every tool.** tombi 1.6.1 was not
  in its manifest, and actionlint is not a crate so the binstall fallback
  failed. tombi now runs from PyPI (`pipx run tombi==1.6.1`) and actionlint
  through `go run ...@v1.7.12`, both pinned.
- **The markdown in the research docs broke 30 lint rules** the first time:
  missing blank lines, fences without a language, bold used as headings.
  Fixed once; the hook keeps it that way.
- **typos flagged "IST" and a hyphenated "preempted".** IST is allowed; the other was
  reworded.
- **tombi sorts lint tables alphabetically.** Harmless: `pedantic` keeps
  `priority = -1`, so specific lints still override it.
- **A first test of the title check looked like it passed a bad title.** It
  was the test loop reading an older run, not the gate. Rerun by run id: the
  gate fails as it should.
- **`unsafe_code` is `deny`, not `forbid`.** `forbid` cannot be overridden,
  and the `clonefile` call in Phase 01 needs one audited `allow`.

## Deliberately not done here

- insta, proptest and criterion arrive with the first Phase 01 code that
  uses them; adding them now fails cargo-shear.
- cargo-dist and the release workflow move to Phase 05.
- `NOTICE` for Composer arrives with the first vendored Composer file.
