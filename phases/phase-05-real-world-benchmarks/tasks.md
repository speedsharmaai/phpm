# Phase 05 tasks

Reuse Phase 03's sweep infrastructure instead of rebuilding it: `sweep/corpus.json`
already pins 330+ projects with commits, and `tools/sweep/run` already knows how
to fetch a project's composer.json/composer.lock at a pin. Phase 03 may still be
editing those files for bug fixes while this runs — read them, don't rewrite them;
put new tooling in its own path (`tools/bench-real/`) so there is no file overlap.

## Corpus

- [x] Select 40 projects from `sweep/corpus.json` by stars, covering every group in scope.md (frameworks' apps, CMS, big apps, Laravel ecosystem, ytmate/wicketyaari), at least 10 over 10k stars. Add any named project missing from the sweep corpus (check: Grav, Statamic, Craft CMS starter, Flarum, nextcloud/server, Matomo, phpMyAdmin, Snipe-IT, Akaunting, Invoice Ninja, Kimai, Pixelfed, Koel, Bagisto, Filament demo, laravel.io, Coolify, Cachet) via the same pinning method Phase 03 uses, into `bench-real/corpus.json` (references the pin, doesn't duplicate the sweep file's other 290+ entries).

## Runs

- [x] `tools/bench-real/run`: per project, per OS, Composer vs phpm cold/warm/no-op via hyperfine (10 warm runs), plus a `diffvendor` identity check and fallback yes/no with plugin names (phpm's `--explain`).
- [x] Worktree scenario: `tools/bench-real/worktrees` — 10 git worktrees of one representative project (laravel-skeleton), total wall time and total disk for `composer install` x10 vs `phpm install` x10 from a warm store.
- [x] CI scenario: `tools/bench-real/ci-scenario` — simulate a project's own install step with and without `actions/cache`, before/after phpm, total job time not just the install line.
- [x] `bench-real.yml`: weekly + manual, matrix across ubuntu/macos/windows runners, SHA-pinned actions, `permissions: {}`. Results committed as JSON with tool versions, runner image, commit SHAs — reuse ci.yml's pinning conventions.
- [ ] A labelled column/row for the owner's own Mac numbers (already have Phase 01/02 data; re-run fresh once this exists) alongside the CI-runner numbers.

## Results page

- [x] Static page (GitHub Pages, `/bench/` path so it coexists with the existing `/` sweep-results page from Phase 03 — coordinate the Pages structure, don't clobber the existing `index.html`/`results.json` at the root) built from the JSON: one row per project with stars, package count, per-scenario Composer vs phpm, speedup, identity, fallback.
- [x] Charts: one horizontal bar chart per scenario (reuse the `dataviz` skill's palette guidance — the design agent's tokens.md has the project's amber/dark palette, use it), plus a "time saved per year" estimate from Packagist's published install-count stats with the assumption stated next to it.
- [x] README table: top 10 by speedup or by stars (pick one, say which), linking to the full page.
- [ ] Export the headline chart at X-post and poster pixel sizes (match the existing `docs/design/poster/x-card.png` dimensions, 3200x1800) so promo material can reuse real numbers without re-deriving them.

## Exit

- [x] 40 projects run on ubuntu/macos/windows, results page live.
- [x] Every row reproducible from one `bench-real.yml` run.
- [x] Worktree and CI scenarios published with real numbers.
- [x] `progress.md` summarizing the headline numbers; README/WORKLIST updated.
