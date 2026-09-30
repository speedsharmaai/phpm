# Topology

One binary, one command in Phase 00: `phpm install`. The diagram is the
install path from a lockfile. The resolver (Phase 05) is drawn at the bottom
only so the module boundary is in the right place from the start.

## The install path

```
  composer.json + composer.lock
             │
  ┌──────────▼──────────────┐
  │ 0. FAST PATH CHECK      │  state file: lock content-hash + per-package reference
  │    nothing changed?     │  + phpm version + vendor mtime  ──▶ exit in < 20 ms
  └──────────┬──────────────┘
             │ changed
  ┌──────────▼──────────────┐
  │ 1. READ & PLAN          │  parse lock; diff against vendor/composer/installed.json
  │                         │  -> install / update / remove per package
  │                         │  root version via git (feeds installed.php)
  └──────────┬──────────────┘
             │
  ┌──────────▼──────────────┐
  │ 2. PLATFORM             │  run `php` once, cache: version, PHP_INT_SIZE,
  │                         │  extensions + versions, lib-* versions
  │                         │  apply config.platform + --ignore-platform-req
  │                         │  verify locked requirements, fail like Composer
  └──────────┬──────────────┘
             │
  ┌──────────▼──────────────┐
  │ 3. POLICY               │  Composer 2.10 malware filter list (conditional GET)
  │                         │  block exactly what Composer blocks
  └──────────┬──────────────┘
             │
  ┌──────────▼──────────────┐       ┌──────────────────────────────────┐
  │ 4. FETCH (missing only) │──────▶│ STORE  ~/.cache/phpm/pkgs/v1/    │
  │ tokio + reqwest, HTTP/2 │       │ key = (name, dist.reference)     │
  │ auth.json / COMPOSER_AUTH│      │ extracted tree, GitHub top dir   │
  │ bounded concurrency     │       │ stripped, exec bits kept         │
  │ extract on rayon        │       │ temp dir + atomic rename         │
  └──────────┬──────────────┘       │ + classes.json per package       │
             │                      └───────────────┬──────────────────┘
  ┌──────────▼──────────────┐                       │
  │ 5. PLACE                │◀──────────────────────┘
  │ clonefile per pkg dir   │  macOS: clonefile()   Linux: FICLONE -> hardlink
  │ remove stale packages   │  Windows: hardlink    all: copy fallback
  │ path repos: symlink/copy│
  └──────────┬──────────────┘
             │
  ┌──────────▼──────────────┐
  │ 6. METADATA (byte-exact)│  installed.json   (PHP json_encode, pretty, sorted)
  │                         │  installed.php    (var_export-compatible writer)
  │                         │  InstalledVersions.php (vendored verbatim)
  └──────────┬──────────────┘
             │
  ┌──────────▼──────────────┐
  │ 7. BINS                 │  vendor/bin proxies, Composer's templates verbatim
  └──────────┬──────────────┘
             │
  ┌──────────▼──────────────┐
  │ 8. AUTOLOAD (byte-exact)│  PackageSorter order; psr-0/psr-4/classmap/files
  │                         │  autoload_static.php, autoload_real.php, suffix rules
  │                         │  -o: class scan via mago-syntax, cached per file
  │                         │  platform_check.php
  └──────────┬──────────────┘
             │
  ┌──────────▼──────────────┐        needs PHP?
  │ 9. PLUGINS & SCRIPTS    │── no ──▶ run string scripts with system php
  │    decide the path      │── yes ─▶ FALLBACK: `composer dump-autoload`
  │                         │          + scripts via real Composer
  │                         │          (plugins activate, events fire)
  └──────────┬──────────────┘
             │
  ┌──────────▼──────────────┐
  │ 10. AFTER               │  notify-batch downloads to Packagist
  │                         │  audit (Composer defaults), write state file
  │                         │  --explain: per-package path + reason
  └─────────────────────────┘
```

Phase 05 adds a resolver that sits before step 1 and writes a lockfile:

```
  composer.json ──▶ METADATA (p2, minified expand, If-Modified-Since)
                ──▶ RESOLVE (pubgrub, Composer DefaultPolicy tie-breaks)
                ──▶ composer.lock (byte-exact) ──▶ install path above
```

## Crate layout

```
crates/
  phpm            the binary: CLI, output, --explain
  phpm-lock       composer.json / composer.lock types, content-hash
  phpm-php        byte-exact PHP json_encode and var_export writers
  phpm-store      store layout, atomic writes, linking modes
  phpm-fetch      HTTP, auth.json, retries, Packagist etiquette
  phpm-autoload   PackageSorter, autoload family, class scanning
  phpm-platform   php probing and platform verification
  phpm-compat     fallback decisions, plugin adapters (Phase 03)
  phpm-resolve    pubgrub resolver (Phase 05)
tools/
  diffvendor      phpm vs Composer vendor/ diff, used by the sweep
  bench           hyperfine wrapper, writes JSON
fixtures/         pinned lockfiles, never updated during a phase
```

## The compatibility sweep (Phase 02)

```
  top-N Packagist projects + real app lockfiles
          │
          ├──▶ composer install --no-scripts ──▶ vendor-a/
          └──▶ phpm install     --no-scripts ──▶ vendor-b/
                                                     │
                       diffvendor: every file, bytes + mode ◀──┘
                                                     │
                   nightly JSON ──▶ public page: "identical vendor/ on N of M"
```
