# Phase 05: Launch

Outline. Tasked only after Phase 03's gate.

## Why

The owner's earlier tools were built well and nobody found them (loadbearing:
14 downloads). The research on how dev tools spread is clear: one install
command, one reproducible benchmark, a technical launch post, then channels in
sequence, then becoming a dependency of something bigger. This phase is that,
done deliberately.

## What ships

- **Release.** `dist init`, then SHA-pin and zizmor-clean the generated workflow, with `actions/attest` for provenance. cargo-dist: GitHub Releases for x86_64/aarch64 Linux (musl),
  macOS, Windows; `curl | sh` and PowerShell installers; Homebrew tap; npm
  wrapper; checksums. Public repo on the brand account.
- **Where installs happen.**
  - A Packagist wrapper package so setup-php (`tools: phpm`) works on day
    one; then a PR to shivammathur/setup-php for a first-class entry.
  - A GitHub Action (`speedsharmaai/setup-phpm`) with the store cached.
  - A Docker image and a documented multi-stage pattern for PHP-free build
    stages.
  - A worktree recipe: many agent worktrees, one store, near-zero disk.
- **Benchmark page.** hyperfine charts in Astral's style, cold / warm / no-op,
  Composer, riff, vivace, phpm, macOS and Linux, filesystem stated, raw JSON
  linked. The sweep number next to it.
- **Launch post.** "phpm: Composer-compatible installs, N x faster warm,
  identical vendor/ on X of Y projects." Technical deep-dive on the store,
  clonefile and byte-exact autoloading. Not a feature list.
- **A note to the Composer maintainers** before launch, saying what phpm is
  and how it treats Packagist (decision 0006).
- **Channels, in order:** Show HN at 12:00-17:00 UTC (17:30-22:30 IST);
  r/PHP and r/laravel; X; Laravel News tip; PHP Annotated; freek.dev; direct
  notes to Nuno Maduro and others who amplified Mago.

## Gate (30 days after launch)

At least **300 GitHub stars and 1,000 real binary downloads** (release assets
plus Homebrew plus setup-php installs), and at least five issues from people
we do not know. Below that, the tool is left maintained but no new phases
start. presto shows stars without usage; downloads are the number that counts.

## Out of scope

Paid anything. A hosted registry (decision 0006).
