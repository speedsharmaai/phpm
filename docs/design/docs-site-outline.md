# Docs site: shape and information architecture

This is a design note, not a build plan. It records what the docs site
should be made of and what pages it needs at this stage of the project.
Writing the actual prose and building/deploying the site are separate,
future work that needs its own explicit go-ahead — nothing here is
implementation.

## Technical shape: mdBook

**Recommendation: mdBook**, not plain static HTML and not a JS static-site
generator.

Why, against the project's own rules:

- **Rust-native, zero npm.** phpm's one dependency rule so far (decisions
  0005, 0008) is "no unnecessary dependencies" and "every tool in CI is
  pinned to an exact version." mdBook is a single Rust binary, installable
  the same way the project already installs `cargo-dist`, `cargo-deny` and
  the rest of its toolchain. A Next.js or Astro docs site would be the
  first npm dependency anywhere in the repository, for a project whose
  entire pitch is "no PHP needed for the fast path" and "single static
  binary." Pulling in Node to build the website undercuts that story.
- **SHA-pinnable in CI**, matching decision 0008's "every action pinned by
  SHA" — `mdbook` installs via `cargo install --locked` or a pinned GitHub
  release binary, same pattern as every other CI tool already in
  `.github/workflows/`.
- **It is what the ecosystem already expects.** The Rust toolchain docs,
  Cargo's own book, and most of the Rust crates phpm already depends on
  (`pubgrub`, `mago-syntax`) ship mdBook sites. A PHP developer landing on
  phpm's docs after `cargo build` will recognise the shape immediately.
- **Honest about where the project is.** mdBook's default theme (light /
  rust / coal / navy / ayu, a sidebar, a search index) is already most of
  what a pre-release CLI tool's docs need: no custom component library to
  design and maintain on top of the marketing site's design system.

What mdBook does **not** cover: the landing/marketing page (hero, headline
numbers, comparison page) and the benchmarks page's data-driven table are
not "book" content — mdBook is a book renderer, not a layout engine for a
hero section or a filterable table. The recommended split:

- **A small static HTML landing page** (hand-written, using this design
  system's tokens directly as CSS custom properties — no framework, same
  "no unnecessary dependencies" logic) for `/`: hero, headline numbers,
  quickstart teaser, comparison teaser, links into the book.
- **mdBook** mounted at `/docs/` for the actual reference content (install,
  CLI reference, architecture, contributing, FAQ, changelog).
- **The existing sweep page** (already a hand-written static HTML page,
  `tools/sweep/page/`) stays hand-written for the same reason: it is a
  live data table driven by `results.json`/`history.json`, not prose, and
  mdBook has no good story for that.

This keeps every piece of the site honest about what it actually is (prose
→ mdBook, marketing → one static page, live data → one static page with a
small script), rather than forcing all three into one tool that is good at
only one of them.

## Where the sweep page lives: `/sweep/`, not the repository root

The sweep page is already live at `https://speedsharmaai.github.io/phpm/`
— it currently owns the whole GitHub Pages root, because
`.github/workflows/sweep.yml` publishes `tools/sweep/page/*` straight to
the Pages artifact.

**Recommendation: move it to `/sweep/`** once a real docs/landing site is
built, and let the landing page own `/`.

Why:

- **The root is the front door and the front door should be the pitch, not
  a results table.** A nightly compatibility dashboard is excellent
  evidence inside the pitch (decision 0004's "compatibility is published,
  not claimed"), but it is not itself a landing page — it has no headline,
  no install instructions, no comparison to Composer. Someone arriving at
  the bare root today gets none of that.
- **It is low-risk to move.** The sweep workflow already treats its output
  as a directory of files (`site/`) copied from `tools/sweep/page/*` plus
  generated JSON; publishing that directory under a `sweep/` prefix instead
  of the Pages root is a one-line change to the `publish` job in
  `sweep.yml`, not a rewrite of the page itself.
- **It keeps the sweep page's own links intact.** `results.json`,
  `history.json` and the corpus link are all relative; moving the whole
  directory under `/sweep/` changes nothing about how the page talks to its
  own data, only where the directory sits.
- **The benchmarks page in this design links out to it**, the way the
  README already does (`[results page]`) — `/sweep/` reads as "the detail
  behind the headline number," which is exactly its job.

This is a recommendation for whoever eventually builds the site (a
separate, explicitly-scoped task); nothing in `sweep.yml` is changed here.

## Information architecture

Grounded in what phpm actually has today (2026-10-01: Phase 02 done, Phase
03/04 in progress, no release). No page below promises a feature that does
not exist yet — in particular, nothing for `update`/`require` (Phase 07,
not built), nothing for a package-manager install (`brew install phpm`,
`npm install -g phpm` — Phase 06, not built).

### `/` — Landing

The pitch in one screen: headline number (18x warm, byte-identical),
sub-pitch, two CTAs (build from source, read the gate numbers). See
`Screen - Landing` in `phpm.pen`.

### `/docs/quickstart` — Build from source

The only installation path today. Clone, `cargo build --release -p phpm`,
run `phpm install --explain` against a real project. No package-manager
install section — one does not exist, and a docs page that lists one
anyway is exactly the kind of thing decision 0004's whole project avoids.

### `/docs/cli` — CLI reference

`phpm install` and its flags (`--explain`, `--no-scripts`, `--no-plugins`,
`--link-mode`, `--audit`, `--audit-format`, `--no-blocking`), exit codes,
environment variables (`PHPM_COMPOSER`, `COMPOSER_BINARY`,
`COMPOSER_MEMORY_LIMIT`, `COMPOSER_DEV_MODE`, `PHPM_COMPOSER` and friends
already named in `phases/phase-02-parity-core/progress.md`). This page
grows as the CLI grows; today it describes `install` only, matching
decision 0002.

### `/docs/how-it-works` — Architecture

The global store and `clonefile`/hardlink/copy placement (decision 0003),
the byte-identical-or-fallback rule and what triggers a Composer fallback
(decision 0004), native script execution, the malware filter, the
`--explain` decision log. This is the page that earns trust with a PHP
maintainer skeptical of a new binary touching their `vendor/`.

### `/docs/compatibility` — Benchmarks & the sweep

The Phase 01 gate numbers, the Phase 02 closing numbers, a link to
`/sweep/` for the live nightly number, and the methodology paragraph
(hyperfine, pinned fixtures, cold/warm/no-op defined in writing) that
already exists in the README almost verbatim. See `Screen - Benchmarks` in
`phpm.pen`.

### `/compare` — phpm vs Composer, riff, viv, vivacity

The honest comparison table: measured speed-up, byte-identity, whether a
compatibility sweep is published, resolver support, runtime. Numbers not
independently verifiable (a competitor's own plugin support, for instance)
are marked "not published" rather than guessed. See `Screen - Compare` in
`phpm.pen`.

### `/docs/contributing` — Contributing

Points to `CONTRIBUTING.md` and `phases/phase-00-foundation/conventions.md`
rather than duplicating them: commit conventions, the quality gates, how to
add a fixture to the compatibility sweep corpus, `just ci`.

### `/docs/faq` — FAQ

Grounded, specific questions only, pulled from the product thesis the
project has already written down: why not just use Composer, why Rust,
what does `install` not do yet (`update`/`require`, Phase 07), what happens
when a plugin isn't supported (falls back to Composer, never breaks the
project), what the compatibility number means and how it's measured.

### `/docs/changelog` — Changelog

Empty placeholder until Phase 06 ships a first release. `release-plz` and
`git-cliff` are already configured (`cliff.toml`, `release-plz.toml`); this
page becomes their output, not hand-written prose, once there is a tag to
describe.

## Explicitly out of scope here

- Writing the actual prose for any of the above pages.
- Building or deploying mdBook, the landing page, or moving the sweep
  workflow's publish path.
- Anything involving `update`, `require`, a package-manager install, or any
  other Phase 06/07 feature that does not exist in the binary today.

All of the above need their own explicit go-ahead, per the project's
docs-vs-code rule.
