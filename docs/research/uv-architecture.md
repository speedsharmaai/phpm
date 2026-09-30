# What to copy from uv, Bun and pnpm

Researched 2026-10-01.

## 1. Lockfile-first fast path

A lock install needs no resolver and no metadata: each entry already has
`dist.url`, `dist.type`, `dist.reference`. Parse with serde_json (simd-json
is not worth it at 200 packages), diff against `installed.json`, fetch only
what is missing.

No-op in milliseconds: a state file holding the lock content-hash, every
package reference, the phpm version, and vendor's mtime. All match → exit in
under 20 ms. This is why uv can re-sync on every `uv run`.

Composer 2.10 revalidates the malware filter list on every install. phpm does
the same with a conditional GET (decision 0006); a 304 costs milliseconds.

## 2. Global store and linking

- Key by `(name, dist.reference)`. The reference is an immutable commit SHA;
  `shasum` is usually empty.
- Store extracted trees, not zips. Version the bucket
  (`pkgs/v1/`) so different phpm versions can share one cache dir, as uv does.
- Append-only: extract to temp, atomic rename. uv's cache is "thread-safe and
  append-only".
- Link modes, uv's defaults: clone on macOS and Linux, hardlink on Windows,
  copy fallback. Bun's measurements: macOS `clonefile()` of a whole tree in one
  syscall is 2.32x faster than copyfile; on Linux hardlink is 2.97x faster
  than copy, then `ioctl_ficlone`, `copy_file_range`, `sendfile`, copy.
- Crates: `reflink-copy` (`reflink_or_copy`) for files; `libc::clonefile`
  directly for whole directories on macOS.
- Not symlinks by default: uv warns clearing the cache breaks installs, and
  pomposer/concerto show tools that expect real files in `vendor/` break.
- Hardlinks: a user patching `vendor/` in place corrupts the store. Clone is
  safe. Document it.

## 3. Parallel I/O

- tokio + reqwest (rustls, HTTP/2, pooling). Packagist: ≤ 10 concurrent
  (20 static), User-Agent with contact. codeload.github.com carries the bulk;
  16-32 connections there, `github-oauth` from auth.json.
- Bun: 64 connections and a work-stealing pool. Rust version: download on
  tokio, extract on `spawn_blocking` or rayon.
- uv speeds cold installs by extracting each streaming zip in one blocking
  task and reusing buffers (Astral's `rs-async-zip` fork).
- Security, uv CVE-2025-54368: a streaming extractor must reconcile local
  headers with the central directory, verify CRC32 and sizes, reject trailing
  data and nested EOCD. **v1 plan:** buffer each zip in memory (PHP packages
  are small) and extract with the `zip` crate on rayon. Stream only if
  profiling says so.
- Strip GitHub's `owner-repo-sha/` top directory.

## 4. Resolver (Phase 07)

`pubgrub` (MPL-2.0, uv's resolver): CDCL-style learning, far better conflict
messages than a SAT port. Encode `replace`, `provide`, `conflict`, stability,
`minimum-stability`, `prefer-stable`, platform packages. uv's tricks: prefer
locked versions, prefetch metadata for selected packages' requirements in the
background, reprioritise packages that keep conflicting. uv: "the slowest
part of resolution is loading package and version metadata, even if it's
cached." Metadata: p2 files, minified expansion, If-Modified-Since, a parsed
binary cache (rkyv or bincode). Reusable: `composer-semver`, `composer-wire`
(licence check: EUPL-1.2).

## 5. Autoload

Byte-identical output (see composer-internals.md). Class scanning with a
lexer-level scanner, not a full parser: find `namespace` and
`class|interface|trait|enum`, skip strings, heredocs, comments. Options:
`mago-syntax` lexer (maintained, fast), a hand-rolled scanner (libretto claims
~7x faster than tree-sitter-php), `tree-sitter-php` (slowest),
`php-parser-rs` (stale). Run on rayon; cache per file keyed by
`(reference, path)` in the store. Warm installs skip scanning entirely. This
is the biggest warm-path win after linking.

## 6. Compatibility honesty

Plugins need PHP. Detect `allow-plugins` / `composer-plugin` packages; use a
proven adapter or fall back to Composer, with a clear line. Scripts
(`@php`, `@composer`, shell) run with system PHP. Publish a parity number the
way Ruff did (">99.9% Black-compatible"): diff `vendor/` across the top N
projects and report "identical vendor/ on X of Y".

## 7. Benchmarks

Copy uv's BENCHMARKS.md plus riff's rigour: hyperfine; scenarios
`install-cold`, `install-warm`, `no-op` (later `update-cold`, `update-warm`);
cold = archives and metadata cache deleted; warm = kept; `vendor/` deleted in
`--prepare`. Fixtures: laravel/laravel, symfony/demo, Sylius, Magento. Flags
`--no-plugins --no-scripts --prefer-dist` on every tool. Alternate run order,
keep raw samples, record versions, OS and filesystem as JSON. Say it like uv:
"performance may vary dramatically across operating systems and filesystems."

## 8. The launch post, Astral's template

- Tagline shape: "An extremely fast Python package installer and resolver,
  written in Rust." Ruff formatter: "An extremely fast, Black-compatible
  Python formatter."
- Structure: one horizontal bar chart on a real project, the tool
  highlighted, 3-4 competitors → cold/warm multipliers → "drop-in" framing →
  design principles → roadmap → acknowledgements.
- The formatter post opened with "30x faster than Black and >99.9%
  compatible". Lead with compatibility and speed together.

## 9. Distribution

cargo-dist: GitHub Actions release workflow, `curl | sh`, PowerShell,
Homebrew tap, npm wrapper, MSI, checksums. Also a PyPI wheel via maturin (as
uv and Ruff do), a Docker image, a Packagist wrapper package for setup-php,
and a `composer` shim binary (riff has one). Targets x86_64/aarch64 Linux
(musl), macOS, Windows. `indicatif` for progress.

## Sources

astral.sh/blog/uv · astral.sh/blog/uv-unified-python-packaging ·
docs.astral.sh/uv/reference/settings/#link-mode · docs.astral.sh/uv/concepts/cache/ ·
docs.astral.sh/uv/reference/internals/resolver/ ·
github.com/astral-sh/uv/blob/main/BENCHMARKS.md · github.com/astral-sh/rs-async-zip ·
astral.sh/blog/uv-security-advisory-cve-2025-54368 · astral.sh/blog/the-ruff-formatter ·
bun.com/blog/behind-the-scenes-of-bun-install · pnpm.io/motivation ·
github.com/pubgrub-rs/pubgrub · docs.rs/reflink-copy · axodotdev.github.io/cargo-dist/book/ ·
mago.carthage.software/tools/lexer-parser/overview
