# Why phpm

The full rationale behind the project, and the hard rules it follows. The
short version lives in the [README](../README.md#why-phpm).

## Product thesis

Composer is good. That is the first thing to accept. It has a lockfile,
parallel downloads, one blessed registry, and a maintainer team that is
respected. The Mago maintainer, the one person best placed to build this,
declined in 2025 and called Composer "great". Most PHP developers would agree.

So the question is not "is Composer slow". It is "where is it slow enough that
someone would install a second tool".

Measured on an M1 Pro, Composer 2.10.3, fresh Laravel skeleton, 109 packages:

| Step | Composer | Floor measured | Where the time goes |
|---|---|---|---|
| cold install | 12-16 s | network | downloading 17 MB of zips |
| warm install | 4.1-4.4 s | **0.14-0.24 s** | re-unzipping 8,860 files from the cache on every install |
| no-op install | 1.6 s | milliseconds | PHP boot, platform solve, filter-list revalidation |
| autoload dump | ~0.9 s | ~0.1 s | single-threaded class scanning |

The floor is `clonefile()` of pre-extracted package directories from a global
store, one syscall per package. That is uv's design, and it is the whole
product. Cold installs stay network-bound; nobody fixes bandwidth. See
[benchmarks](research/benchmarks-2026-10-01.md).

Three answers to "why would anyone use ours", in order of how much they matter:

**1. It never breaks a project.** Every competitor either refuses plugins,
emulates a few, or breaks silently. phpm installs natively when it can prove
the result is identical to Composer's, and hands the rest to real Composer
when it cannot. A user should never be worse off for having tried it. See
[decision 0004](decisions/0004-byte-identical-or-fall-back.md).

**2. Compatibility is published, not claimed.** A nightly sweep diffs phpm's
`vendor/` against Composer's across hundreds of real lockfiles and publishes
the number, the way Ruff published ">99.9% Black-compatible". Nobody in the
field does this at scale. It is the only credible answer to "why trust a new
binary on my install path".

**3. It is built for where installs actually multiply now.** Packagist went
from about 3 billion installs a month in January 2026 to over 5 billion in
September, and Packagist credits AI coding tools. Agents run installs in CI,
in containers, and in parallel git worktrees, each of which wants its own
`vendor/`. With a shared store, a new worktree's `vendor/` costs a fraction of
a second and almost no disk. Humans run Composer a few times a day; agents run
it constantly.

**How it earns.** It probably does not, and that is stated here so nobody is
surprised later. Private registries fund Composer itself (Private Packagist),
and Astral wound down its own paid registry after the OpenAI acquisition. phpm
is a reputation and distribution project for the speedsharma brand. If a
business appears, it will be in CI and container caching, and not before the
tool has users.

**Where the users come from.** Laravel first: 64% of PHP developers use it,
and the default skeleton has zero Composer plugins. Then GitHub Actions via
setup-php, which already installs any Packagist tool. Then Docker image
builds. See [market](research/market-and-competitors.md).

## Rules

**1. Phase 01 is a spike and it can kill the project.**
Warm install at least 10x faster than Composer on the Laravel skeleton and five
real apps, with a byte-identical `vendor/`, measured head to head against riff
and vivace. If it is not clearly ahead, the project stops, or becomes a
contribution to whichever of them is. See [the gate](../phases/README.md#the-gate).

**2. Byte-identical or fall back.**
The output files that other tools parse (`installed.json`, `installed.php`,
the autoload family, bin proxies) match Composer's bytes. When phpm cannot
guarantee that, it runs Composer for the part it cannot do, and says so in one
line. It never guesses.

**3. Install before update.**
`install` from a lockfile needs no resolver and no registry metadata. That is
where the speed is and where Phase 01 lives. `update` and `require` need a
resolver whose choices must match Composer's, and that is Phase 07 at the
earliest. See [decision 0002](decisions/0002-lockfile-install-first.md).

**4. A good Packagist citizen.**
User-Agent with a contact, concurrency within the published limits, download
notifications sent so package authors still get their numbers, and Composer
2.10's malware filter honoured. Packagist is funded by the same people who
build Composer; phpm does not make their bill bigger. See
[decision 0006](decisions/0006-good-packagist-citizen.md).

**5. Every number is reproducible.**
Benchmarks run through hyperfine with a published script, pinned lockfiles,
cold and warm defined in writing, filesystem and OS stated. No blog-post
numbers. Half the "Composer takes minutes" posts found in research were
content marketing with wrong facts in them.

## Non-negotiables

- **No resolver in Phase 01-06.** A resolver that picks different versions
  than Composer silently changes what runs in production.
- **No symlinks into the store by default.** Tools that expect real files in
  `vendor/` break, and clearing the cache would break installs. Clone on macOS
  and Linux, hardlink on Windows, copy as the fallback.
- **Plugins are never emulated partially.** An adapter is either proven
  identical by the sweep, or the package goes to Composer.
- **No paid registry.** It would compete with the thing that funds Composer.
- **No launch without the compatibility number.** The launch post leads with
  "identical `vendor/` on N of M projects", not with speed.
