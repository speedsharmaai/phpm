# Phase 05: Real-world benchmarks

Outline. Tasked after Phase 04 ships.

## Why

A benchmark on the Laravel skeleton convinces nobody. A table that says "we
installed Magento, Nextcloud, Drupal, BookStack and forty other real apps, a
thousand times each, on the same runners, and here is every number" does. It
is the launch headline, the poster, the video, and the answer to "sure, but
does it work on my project".

It also keeps us honest. Where phpm falls back to Composer, is slower cold,
or produces a different `vendor/`, the table says so.

## What ships

### The corpus

The largest open-source PHP applications that commit a `composer.lock`,
ranked by GitHub stars, pinned to a commit. Libraries without a lockfile
(laravel/framework, symfony/symfony) are left out rather than given a lock we
generated, because nobody installs them that way.

Starting list, to be checked for a committed lock and a permissive setup:

| Group | Projects |
|---|---|
| Frameworks' own apps | laravel/laravel, symfony/demo, Sylius-Standard |
| CMS | drupal/drupal, Bedrock (WordPress), Grav, Statamic, Craft CMS starter, Flarum |
| Big apps | magento/magento2, nextcloud/server, matomo-org/matomo, phpmyadmin, monicahq/monica, firefly-iii, BookStack, Snipe-IT, Akaunting, Invoice Ninja, Kimai, Pixelfed, koel, Bagisto |
| Laravel ecosystem | Filament demo, laravel.io, Coolify, Cachet |
| Our own | ytmate, wicketyaari |

Target: 40 projects, at least 10 of them over 10k stars.

### The runs

- GitHub-hosted runners (ubuntu, macos, windows), so anyone can see the
  hardware and rerun it. The owner's Mac numbers are a separate, labelled
  column.
- Per project and OS: Composer vs phpm, cold, warm and no-op, hyperfine,
  10 warm runs; plus `diffvendor` identity, fallback yes/no, and which
  plugins caused it.
- Worktree scenario: 10 worktrees of the same project. Total time and total
  disk for Composer (10 full `vendor/` copies) vs phpm (one store, clones).
  This is the number for AI-agent workflows.
- CI scenario: the project's own install step with `actions/cache`, before
  and after, total job time not just the install step. This is kill
  criterion 2 from the market research, measured in public.
- Weekly workflow; results committed as JSON with tool versions, runner
  image and commit SHAs.

### The page

- A static results page (GitHub Pages) generated from the JSON: one row per
  project, stars, package count, Composer vs phpm for each scenario, speedup,
  identity, fallback.
- Charts: one horizontal bar chart per scenario in the Astral style, and a
  "time saved per year" estimate from Packagist install counts, with the
  assumption written next to it.
- A README table of the top 10 with a link to the rest.
- Exportable images at X and poster sizes, so the promo material uses the
  same numbers.

## Rules

- Every number reproducible from the committed workflow. No hand-typed
  results.
- Losses are published. A project where phpm is slower or falls back stays
  in the table with the reason.
- No pull requests or issues opened on the benchmarked projects to advertise
  phpm. Being measured is not an invitation.
- Project names and logos are used factually, not as endorsements.

## Exit criteria

- 40 projects run on three OSes, results page live.
- Every row reproducible from one workflow run.
- Worktree and CI scenarios published.
- The launch post and poster numbers are taken from this page.

## Out of scope

Resolver benchmarks (`update`), which wait for Phase 07. Paid runners.
