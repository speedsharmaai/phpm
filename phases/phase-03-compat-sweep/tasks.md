# Phase 03 tasks

## Corpus

- [ ] `sweep/corpus.json`: one entry per project with `repo`, pinned `commit`, `stars`, `taken` date, `source` (search query or hand-picked) and `notes`. Lockfiles are not committed; the sweep fetches `composer.json` and `composer.lock` at the pinned commit.
- [ ] `tools/sweep/build-corpus`: GitHub search for repositories with a root `composer.lock`, language PHP, sorted by stars, not archived, not forks; skip libraries without a lock. Hand-picked additions: Laravel, Symfony demo, Drupal recommended, Bedrock, Magento 2, Sylius-Standard, Nextcloud server, Matomo, phpMyAdmin, Monica, Firefly III, BookStack, Snipe-IT, Kimai, Akaunting, Invoice Ninja, Pixelfed, Koel, Bagisto, Flarum, Grav, Statamic, Filament demo, laravel.io, Coolify, Cachet, and our own ytmate and wicketyaari.
- [ ] Target 300+ projects, at least 30 over 10k stars. Refreshed monthly by a scheduled workflow that opens a PR with the new pins.

## Runner

- [ ] `tools/sweep/run`: for one project, fetch its files at the pin, then in sibling temp dirs run `composer install` and `phpm install` with the same flags, `diffvendor` the two `vendor/` trees (and `web/`, `public/`, `config/` when installers or recipes write there), and emit one JSON record: identity, differing paths (first 20), which path phpm took (native, partial fallback, full fallback, and why), exit codes and stderr tails on failure, timings.
- [ ] Two modes per project: `pure` (`--no-scripts --no-plugins --ignore-platform-reqs` on both) measures phpm's own install; `default` (`--ignore-platform-reqs`) measures what a user gets, including fallback.
- [ ] A project that Composer itself cannot install (dead dist URL, private repo, bad lock) is recorded as `composer-failed` and excluded from the identity ratio, but listed.

## Nightly workflow

- [ ] `sweep.yml`: scheduled nightly and manual; build phpm once (release), shard the corpus across a matrix (about 30 projects per job), setup-php 8.4 with Composer 2.10.3, shared Composer and phpm caches per shard, `permissions: {}` and SHA-pinned actions.
- [ ] Aggregate job: merge shard JSON into `results/<date>.json`, compute the headline numbers, publish the page.
- [ ] Windows: a 20-project subset on `windows-latest` in `pure` mode (hardlinks, `.bat` proxies, path-repo junctions or symlinks).

## Results page

- [ ] Static page on GitHub Pages generated from the JSON: headline "identical `vendor/` on N of M projects" for each mode, native-install rate, a row per project (stars, packages, mode results, path taken, first difference), and a history chart of the headline over time.
- [ ] README badge with the current `pure` identity ratio.

## Triage

- [ ] Every non-identical project gets a cause: phpm bug (issue opened and fixed), Composer-side nondeterminism (documented), or out of scope (documented). No unexplained rows.
- [ ] Fix phpm bugs the sweep finds, each with a regression test built from the minimal reproducer.

## Gate

- [ ] `pure` mode: at least **95% identical** across installable projects, every remaining difference explained.
- [ ] `progress.md` with the first full sweep and the triage table.
