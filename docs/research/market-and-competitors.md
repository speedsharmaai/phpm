# Market and competitors

Researched 2026-10-01. [U] = uncertain or single secondary source.

## Verdict up front

The speed gap is real: 5-6x warm and 4-5x cold on a datacenter link (riff's
hyperfine data), 10x+ warm on APFS (our own floor measurement). The absolute
pain is small: single-digit seconds warm, 10-30 s cold, 1-2 s no-op. Most
people and the Composer maintainers consider Composer fast enough. The field
already has eight or more clones and none has users.

Research agent's honest read: **a weak venture and a plausible niche open
source project.** Worth a two-week spike to find out, and not worth more
unless the gate is passed clearly.

## Where the pain actually is

- **Warm installs** are 3-10x above the filesystem floor. That is the gap.
- **Cold installs** are bandwidth-bound on ordinary links. Raising Composer's
  parallelism from 12 to 48 changed nothing on a home connection.
- **Autoload dump** is about 1-1.6 s normally, pathological on Docker-on-Mac
  bind mounts (a developer reported five-minute `dump-autoload` runs; a
  Windows user hit 494 s in composer/composer#12313; large classmaps in #12754).
- **Scripts** are PHP-bound. Nothing helps.
- **Resolution** only matters for `update` (about 4 s on the Laravel skeleton).

Where installs multiply, the seconds add up: CI matrices, container builds,
AI agents. Packagist installs went from ~3B/month (Jan 2026) to >5B/month
(Sep 2026), credited by Packagist to AI coding tools.

Blog posts claiming "90 s to 4 min per deploy" are content marketing with
wrong facts (one says Composer downloads sequentially, which has not been
true since 2.0). Not usable as evidence.

## "It's fine" (evidence against)

- Mago maintainer azjezz, 2025-08-29: "the bulk of the time in composer
  install is spent downloading files (I/O)… a Rust version would still be
  limited by network speed and wouldn't provide a game-changing performance
  benefit… Composer is a great tool that works very well."
  github.com/carthage-software/mago/discussions/333
- HN, 2026-09: "composer isn't by any way slow today, nor do you deal with
  thousands of packages in a project like NodeJS." news.ycombinator.com/item?id=49710485
- HN Mago thread: "Composer is one of the best package managers in any
  language ecosystem." news.ycombinator.com/item?id=45232275
- libretto's own README: "Composer runs maybe 1-5 times per day locally."

## Competitors (GitHub API, 2026-10-01)

| Project | Lang | Stars | Created | State |
|---|---|---|---|---|
| paramientos/presto | Go | 237 | 2025 | beta 0.1.12; "10-20x, 100% compatible"; ~55-144 binary downloads per release |
| libretto-pm/libretto | Rust | 24 | 2026-01 | alpha, no plugins, quiet since Feb |
| HichemTab-tech/pomposer | Rust | 24 | 2025-07 | global store, quiet since Nov 2025 |
| Adelagric/vivacity | Rust | 21 | 2026-09 | active; crates vivacity-{core,resolver,autoload}; "cold network installs are not faster" |
| zanderlewis/composer-rs | Rust | 9 | 2025-09 | formerly lectern, no licence |
| cresset-tools/bougie | Rust | 8 | 2026-05 | EUPL-1.2 crates: composer-semver, composer-php-json (byte-exact json_encode), composer-wire, composer-autoload, composer-installers |
| **shyim/riff** | Rust | 3 | 2026-08 | **most serious**: Shopware core dev (519 followers), native flex + composer-patches adapters, `composer` shim, Homebrew, hyperfine: cold 1.71 s vs 7.80 s, warm 0.35 s vs 1.99 s on symfony/demo |
| svandragt/vivace (`viv`) | Rust | 3 | 2026 | v0.20, byte-identical vendor sweep, GitHub Action, container image, no Windows, refuses flex; began as a test of whether "a person directing coding agents can build a faster drop-in Composer" |
| TheLifus/concerto | Rust | 2 | 2026-06 | pnpm-style symlinked store |
| lschvn/tusk, zengbo/composer-rs, rabbiveesh/composr | Rust | 0-2 | 2026 | experiments |

Not install replacements: Mago (3,476 stars; linter/formatter/analyzer;
explicitly no package-manager plans), PIE (PHP Foundation extension
installer, 2,033 stars, 1.0), FrankenPHP (11.4k stars, in the PHP org),
Laravel Herd (bundles Composer [U]).

**The code is not the moat.** An AI-directed Rust Composer is now a genre.
The moat, if any, is trust (compatibility evidence), never breaking, and
distribution.

## Composer and Packagist posture

- Composer 2.9 (2025-11): HTTP/3, faster scripts. Composer 2.10 (2026):
  Aikido-backed malware blocking, dependency policies, source fallback
  deprecated. Roadmap is security, not speed. No public Composer 3 or Rust
  plans found [U].
- Packagist costs >$1M in 2026, Private Packagist covers more than half.
  Sponsorship programme (2026-07-30) names caching services and AI consumers
  as parties that should pay. 2025-09 post: automated traffic is the
  majority; rate limits and enterprise charges floated.
- Not hostile to well-behaved clients. A paid registry or cache would compete
  with Private Packagist, which funds Composer.

## Ecosystem size

- Packagist: 201.6B installs all time; 469k packages; 5.8M versions;
  ~3B/month (Jan 2026) → >5B/month (Sep 2026).
- SO Survey 2025: PHP 18.9% of respondents; Composer 11%; Rust 14.8%.
- JetBrains State of PHP 2025: Laravel 64%, WordPress 25%, Symfony 23%;
  PHPStan 36%; 42% use no quality tools.

## Plugin reality in real lockfiles

| Project | Plugins |
|---|---|
| laravel/laravel skeleton | 0 (the easy 64%) |
| monicahq/monica, Firefly III | php-http/discovery, phpstan/extension-installer |
| Magento 2 | magento-composer-installer + 3 |
| Drupal core | 10 (installers, scaffold, recipe-unpack, vendor-hardening, symfony/runtime, ...) |
| Symfony apps | symfony/flex (207M installs all time) |
| Bedrock | composer/installers (148M) |

## Will PHP developers adopt a Rust tool?

Yes when the gain is large: Mago (3.5k stars, HN 159 points, Laravel News
coverage, a Nuno Maduro video, JetBrains sponsorship, in setup-php),
FrankenPHP (PHP Foundation). But Mago also drew "the PHP community lacks the
resources to see a non-PHP tool thrive", and PHPStan still dwarfs it in use.

uv reached ~13% of PyPI downloads in ~9 months, but pip's pain was far worse
(no lockfile, slow resolution, venvs), and uv replaced five tools. phpm
replaces one tool people like.

## Distribution channels

- Laravel News: 55k+ newsletter subscribers (~45% open rate), 400k+ X followers.
- PHP Annotated (JetBrains, monthly), r/PHP, r/laravel, Laracasts,
  freek.dev, Laravel Podcast, PHP Roundtable [U on activity].
- People: Nuno Maduro (amplified Mago), Freek Van der Herten, Taylor Otwell,
  Kévin Dunglas, Ondřej Mirtes.
- setup-php (shivammathur, 3.3k stars) installs any `vendor/package` from
  Packagist, so a Packagist wrapper puts phpm in GitHub Actions for free; a
  first-class entry needs a PR.
- Official `composer` Docker image: high leverage, unlikely [U].
- Herd, Sail, Laravel Cloud: need Laravel's buy-in.

## Monetisation

- Hosted private registry: occupied by Private Packagist (from €649/year),
  which funds Composer. Ecosystem-hostile. Ruled out (decision 0006).
- Astral launched pyx (paid registry) Aug 2025; after the OpenAI acquisition
  (Mar 2026) it was wound down and its GPU index open-sourced.
- CI caching: GitHub Actions cache is free; Depot, Namespace etc. exist [U].
- Plausible small niches: Docker-on-Mac dev speed, worktree/monorepo disk
  dedup via the store, a supply-chain policy CLI.

## Kill criteria (from research, adopted in the gate)

1. riff or vivace reaches drop-in parity on Laravel, Symfony and Drupal
   before phpm has a clear differentiator → contribute instead (decision 0007).
2. Less than 3x on p50 **total CI job time** (not the install step) across 5
   real Laravel apps with standard `actions/cache` → stop.
3. Launch (HN, r/PHP, Laravel News) yields < 300 stars and < 1k real binary
   downloads in 30 days → stop. presto shows stars without usage.
4. Composer ships cached extraction or hardlinks, or objects publicly to
   third-party clients → stop or pivot.
5. The 2.10 malware and audit policies, plus fallback for the top 5 plugins,
   cannot be matched inside the MVP budget → stop.
6. Fewer than 10 teams say install time is a top-3 CI or dev pain (start with
   Docker-on-Mac teams) → stop.

## Sources

github.com/shyim/riff (docs/assets/symfony-demo-install.json) ·
github.com/svandragt/vivace · github.com/Adelagric/vivacity ·
github.com/cresset-tools/bougie · github.com/libretto-pm/libretto ·
github.com/TheLifus/concerto · github.com/HichemTab-tech/pomposer ·
github.com/zanderlewis/composer-rs · github.com/paramientos/presto ·
github.com/carthage-software/mago/discussions/333 ·
news.ycombinator.com/item?id=49710485 · news.ycombinator.com/item?id=45232275 ·
github.com/composer/composer/issues/12313 · github.com/composer/composer/issues/12754 ·
php.watch/articles/composer-2 · blog.packagist.com/composer-2-9/ ·
blog.packagist.com/composer-2-10-release/ ·
laravel-news.com/malware-blocking-and-dependency-policies-in-composer-210 ·
blog.packagist.com/announcing-the-composer-packagist-sponsorship-program/ ·
blog.packagist.com/a-call-for-sustainable-open-source-infrastructure/ ·
blog.packagist.com/15-years-of-packagist-over-200-billion-package-installs/ ·
packagist.org/apidoc · survey.stackoverflow.co/2025/technology ·
blog.jetbrains.com/phpstorm/2025/10/state-of-php-2025/ ·
laravel-news.com/mago · biggo.com/news/202509141922_Mago_PHP_Toolchain_Beta_Struggles ·
laravel-news.com/newsletter ·
pydevtools.com/blog/astral-winds-down-pyx-open-sources-gpu-packaging/ ·
siliconangle.com/2026/03/19/openai-acquires-open-source-python-tooling-startup-astral/
