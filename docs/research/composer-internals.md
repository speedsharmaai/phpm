# Composer internals, for a byte-compatible install

Read from composer/composer master (commit 0fc32b9, 2026-09-30; latest
release 2.10.3, 2026-08-27) and composer/class-map-generator, plus live
Packagist endpoints and a `composer install -vvv` run. Content-hash, the dist
cache key and the autoloader suffix were verified by hand.

**[EXACT]** must match byte for byte or behave identically. **[FREE]** may differ.

## 1. `composer install` from a lock

`Installer::run()` → `Installer::doInstall()` (src/Composer/Installer.php).

1. **Root package.** composer.json loaded. Root version: `version` in
   composer.json, else `COMPOSER_ROOT_VERSION`, else `VersionGuesser`:
   `git branch -a -v`, `git describe --exact-match --tags`,
   `git rev-list -n1 HEAD`, then hg/fossil/svn, all run in the working
   directory, so a project inside another git checkout gets that checkout's
   branch. Feature branches take the nearest non-feature branch by
   `git rev-list candidate..branch` length. Numeric branches print as
   `2.1.x-dev`. No VCS → `1.0.0+no-version-set` / `1.0.0.0`, reference null.
   The commit becomes the root's reference. **[EXACT]**, it lands in
   `installed.php['root']`.
2. `pre-install-cmd` (scripts only). `COMPOSER_DEV_MODE=1/0` exported.
3. **Lock freshness.** `Locker::isFresh()` compares content-hash; mismatch is a
   warning. Missing root requires in the lock → `ERROR_LOCK_FILE_INVALID`
   unless `allow-missing-requirements`.
4. **Platform verification is a real SAT solve.** `PlatformRepository`
   (php, ext-*, lib-*, `config.platform` overrides), every locked package fixed,
   `Solver::solve()`; any resulting operation → "lock file cannot be installed
   on this system". **[EXACT semantics]**: needs a `php` probe for version,
   `PHP_INT_SIZE`, extensions and versions, lib versions;
   `--ignore-platform-req(s)`.
5. **Malware filter (2.10, default on).** Even a lock install fetches
   `packages.json` and `https://repo.packagist.org/lists/all/summary.json`
   (conditional GET, cached as `filter-summary.json`). **[EXACT behaviour]**.
6. **Plan.** `LocalRepoTransaction(locked, installed.json)`. Batches:
   plugins with `extra.plugin-modifies-downloads` alone first; then plugins
   and `composer-installer` types; then the rest.
7. **Download.** `FileDownloader::processUrl` (auth, GitHub), 3 retries,
   `dist.mirrors`. Temp `vendor/composer/tmp-<md5>.<ext>`. Cache key
   `files/<vendor>/<name>/<sha1(processed dist URL)>.<type>`. sha1 verified
   only if `shasum` non-empty, which on Packagist it almost never is. **[FREE]**
   if phpm uses its own store; **[EXACT]** only to share Composer's cache.
8. **Extract.** Into `vendor/composer/<8 hex>/`; single top-level dir is
   unwrapped (GitHub's `owner-repo-sha/`); `.DS_Store` ignored. Prefers system
   `unzip -qq`, then 7z, then ZipArchive. **[EXACT]** Unix mode bits kept.
9. **Install path.** `vendor-dir/<prettyName>[/<target-dir>]`; custom
   installers override.
10. **Bin proxies** (`BinaryInstaller::generateUnixyProxyCode`). PHP targets
    (optional shebang then `<?php`) get a PHP proxy with `namespace Composer;`,
    `$GLOBALS['_composer_bin_dir']`, `$GLOBALS['_composer_autoload_path']`, a
    `phpvfscomposer://` wrapper when there is a shebang, and a PHPUnit
    isolation hack for `vendor/phpunit/phpunit/phpunit`. Others get a
    `#!/usr/bin/env sh` proxy. `.bat` on Windows/WSL. Mode `0777 & ~umask`.
    2.10.2/2.10.3 skip bin paths that escape the package. **[EXACT]**, copy the
    templates verbatim.
11. **installed.json** (`FilesystemRepository::write`):
    `{packages:[ArrayDumper + installation-source + install-path], dev, dev-package-names:[sorted]}`,
    packages sorted by strcmp(name), `JSON_PRETTY_PRINT|UNESCAPED_SLASHES|UNESCAPED_UNICODE`,
    4-space indent, trailing newline. `install-path` relative to
    `vendor/composer`. Each entry is the lock entry passed through
    `ArrayLoader` then `ArrayDumper`, which normalises: `version_normalized`
    added, link targets lowercased and ksorted, `bin`/`license` strings become
    lists, `type` lowercased, `time` reformatted as RFC 3339, empty arrays
    dropped, keywords sorted. Metapackages are never downloaded, so they have
    no `installation-source` and `install-path` is `null`.
    `installation-source` follows `preferred-install` (default `dist`).
    Alias packages are not written here. **[EXACT]**
12. **installed.php** via `dumpToPhpCode`: `<?php return array(...);`, 4-space
    indent, `var_export`'d scalars, `install_path` as `__DIR__ . '/...'`; root,
    `versions[*]` (pretty_version, version, reference, type, install_path,
    aliases, dev_requirement), replaced/provided entries. ksort on versions,
    SORT_NATURAL on alias/replaced/provided lists. `InstalledVersions.php`
    copied verbatim. Aliases come from `extra.branch-alias`, from
    `default-branch: true` (`9999999-dev`) and from the lock's `aliases`; an
    alias's `self.version` replace/provide adds both the alias and the
    branch version. Platform targets (`php`, `ext-*`, ...) are skipped. The
    order packages are visited only shows when one target is both replaced
    and provided by different packages; Composer visits them in operation
    order. **[EXACT]**
13. Autoload dump, `ensureBinariesPresence`, funding message,
    `post-install-cmd`, audit (network, default on; exit code under
    `--audit`/policy), `notify-batch` POST to `https://packagist.org/downloads/`,
    `.htaccess` "Deny from all" in cache dirs.

Flags: `--no-dev` filters by the lock's `packages-dev` names, also in the
autoloader. `preferred-install` defaults to dist; source uses `GitDownloader`
with a cache mirror. 2.10: no automatic dist→source fallback unless
`source-fallback`.

## 2. Autoload generation (`AutoloadGenerator::dump`)

Files in `vendor/composer/`: `autoload_namespaces.php`, `autoload_psr4.php`,
`autoload_classmap.php`, `autoload_files.php` and `include_paths.php` (only if
non-empty, else deleted), `autoload_static.php`, `platform_check.php` (only if
there is something to check, else deleted), `autoload_real.php`,
`ClassLoader.php` + `LICENSE` (verbatim; the phar's `LICENSE` has a blank
first and last line), and `vendor/autoload.php`. **[EXACT]** all of them, and
`phpm-autoload` matches Composer 2.10.3 byte for byte on all five fixtures,
dev and `--no-dev` (`just golden`).

- **Suffix** (`ComposerAutoloaderInit<suffix>`): `config.autoloader-suffix` →
  suffix already in `vendor/autoload.php` → lock content-hash if hex → random.
  An empty `autoloader-suffix` counts as unset.
- **Order.** `PackageSorter::sortPackages`: weighted reverse-dependency sort,
  weight = -Σ(1 - weight(user)), ties by `strnatcasecmp`, relies on PHP 8's
  stable usort. The root is not in the sorted set, so only installed
  packages' `require` counts. A dependency cycle (league/flysystem ↔
  flysystem-local in Laravel and Monica) makes the weights depend on the
  installed repository's order; install and dump-autoload agree on all
  fixtures. psr-0/psr-4/classmap in reverse order (root first), files in
  forward order. psr-0 and psr-4 then `krsort`. Root `autoload-dev` merged with
  `array_merge_recursive` in dev mode. With `--no-dev` and an empty dev list
  Composer falls back to keeping what the root's `require` reaches.
- **Files identifier:** `md5(packageName . ':' . path)`.
- **Path code:** `$vendorDir . '/x'` / `$baseDir . '/x'`, `'phar://' .` prefix
  when the path contains `.phar`.
- **autoload_static.php:** `var_export` of ClassLoader's private props, absolute
  paths replaced by `__DIR__ . '/..' . '/...'`, re-indented (leading spaces
  doubled, trailing stripped). `prefixLengthsPsr4` bucketed by first character
  in insertion order. `var_export` quirks: `array (`, `=>` then newline for
  nested arrays, `0 =>`. `$files` comes first, read back from
  `autoload_files.php`; every other property is written only if non-empty.
- Classmap always contains `Composer\InstalledVersions`, ksorted.
- **platform_check.php:** highest lower bound of `php`/`php-64bit` over non-dev
  packages and the root → `PHP_VERSION_ID >= NNNNN` (`>` for an exclusive
  bound). Needs composer/semver's `parseConstraints` lower bounds and PHP's
  `version_compare`. `--ignore-platform-reqs` removes the file. `ext-*` only if `platform-check: true`
  (default `php-only`). Extensions provided/replaced are skipped; pcntl and
  readline are CLI-only.
- **`-o`:** scan every PSR-0/4 dir into the classmap, krsort-namespace order,
  `avoidDuplicateScans`; a vendor dir inside a scanned PSR dir is excluded.
  **`--classmap-authoritative`** implies `-o` and adds
  `setClassMapAuthoritative(true)`. **`--apcu-autoloader`** random prefix, **[FREE]**.

### Class discovery (`PhpFileParser::findClasses`)

Not a token walk:

1. `php_strip_whitespace()`, PHP's own lexer, drops comments and whitespace.
   Run by the CLI it also skips a leading `#!` line; `short_open_tag` comes
   from php.ini (off in the shipped php.ini files, assumed off). phpm ports
   the lexer states that change the output (heredoc/nowdoc, interpolation,
   `?>`, `yield from`, casts, the token after a heredoc's closing label);
   `__halt_compiler` data is lexed as code, as PHP does. Checked against
   `php` on every file of the Laravel vendor tree and on random token soup.
2. Quick regex precheck.
3. `PhpFileCleaner` removes strings, heredocs, nowdocs.
4. A large regex captures `class|interface|trait|enum` and `namespace`. `enum`
   only when Composer runs on PHP ≥ 8.1. Anonymous classes skipped, XHP names
   rewritten, enum backing types stripped. Class names are bytes:
   symfony/cache declares a class named `\xA9`.

PSR mode: `filterByNamespace` keeps classes matching the rule; violations
inside vendor are silent. For psr-4 it only compares the path after cutting
`strlen(namespace)` bytes off the class name, so a class outside the
namespace with a matching tail is kept. `exclude-from-classmap`: `**` → `.+?`, `*` →
`[^/]+?`, anchored to the install dir realpath plus `($|/)`, matched against
realpath and non-realpath; patterns whose constant prefix is unrelated to the
scanned dir are dropped first. Extensions: php, inc, hh. Symfony Finder with
followLinks, dot files/dirs and VCS dirs skipped, **unsorted**, so duplicate
classes are "first found wins" by readdir order. phpm walks with `read_dir`,
which returns the same order PHP's `readdir` sees on the same filesystem
(differential test). On APFS that order is a hash of the name, independent of
creation order, so a vendor/ phpm builds scans in the order Composer's would;
on ext4 small directories follow creation order. None of the five fixtures
has a duplicate class.

## 3. composer.lock and content-hash

Keys in order: `_readme` (3 fixed lines), `content-hash`, `packages`,
`packages-dev`, `aliases`, `minimum-stability`, `stability-flags` (ksorted,
`{}` when empty), `prefer-stable`, `prefer-lowest`, `platform`,
`platform-dev` (`{}` when empty), `platform-overrides` (only if non-empty),
`plugin-api-version` ("2.9.0" in 2.10.x; "2.11.0" on master). Package entries:
ArrayDumper output minus `version_normalized` and `installation-source`,
`time` last, link maps and `suggest` ksorted, keywords sorted; sorted by name
then version. **[EXACT for update, not needed for install]**

**content-hash** (`Locker::getContentHash`, verified):
keys `name, version, require, require-dev, conflict, replace, provide,
minimum-stability, prefer-stable, repositories, extra` if present, plus
`config.platform`; top-level ksort only (nested order preserved);
`md5(json_encode($x, 0))`. Traps: flags 0 escapes `/` as `\/` and non-ASCII as
`\uXXXX`; `{}` decodes to an array and re-encodes as `[]`; floats use
serialize_precision -1. **[EXACT]**

## 4. Packagist

- Root `https://repo.packagist.org/packages.json`: `metadata-url`
  (`/p2/%package%.json`), `metadata-changes-url`, `providers-url` (legacy),
  `notify-batch`, `security-advisories {metadata:true, api-url}`,
  `filter {metadata:true, lists:{malware:{enabled:true}}, summary-url}`.
  Composer 1 was shut down 2025-09-01.
- Per package: `p2/{vendor}/{pkg}.json` (tags), `~dev.json` (branches);
  `{minified:"composer/2.0", packages:{name:[...]}, security-advisories:[...]}`.
  `MetadataMinifier::expand`: first entry complete, each later one a diff
  against the previous expanded one, `"__unset"` deletes a key.
- Caching: Composer stores `Last-Modified` inside the cached JSON and sends
  `If-Modified-Since`. CDN `cache-control: max-age=900`.
- Dist URLs: `https://api.github.com/repos/{o}/{r}/zipball/{sha}` → 302 to
  `codeload.github.com/.../legacy.zip/{sha}`. `shasum` empty for GitHub
  packages: no integrity check exists. GitHub archives honour
  `.gitattributes export-ignore`. Unauthenticated api.github.com: 60 req/h/IP;
  `github-oauth` raises it.
- Etiquette: User-Agent with contact, ≤ 10 concurrent (20 static), HTTP/2,
  If-Modified-Since, `metadata/changes.json?since=` for mirroring. Composer
  itself: 12 parallel jobs, `CURLMOPT_MAX_HOST_CONNECTIONS=8`.

## 5. Repositories and auth

Types: `composer` (Satis, Private Packagist), `vcs` (GitHub/GitLab/Bitbucket/
Forgejo/git/hg/svn/fossil/perforce drivers), `path` (symlink by default,
relative option, junction on Windows, copy fallback,
`COMPOSER_MIRROR_PATH_REPOS`), `artifact`, `package`. Locked packages carry
their own dist info, so a lock install rarely needs repo metadata; path repos
and the 2.10 filter are the exceptions.

auth sources: `COMPOSER_HOME/auth.json`, `./auth.json`, `COMPOSER_AUTH`,
composer.json `config`. Keys: `http-basic`, `bearer`, `github-oauth`,
`gitlab-oauth`, `gitlab-token`, `bitbucket-oauth`, `forgejo-token`,
`custom-headers`, client certs, inline `user:pass@host`. **[EXACT]** read all.
Recent fixes: GitLab host matching (#12988), token sanitisation.

## 6. Plugins and scripts

Events during install: `init`, `pre-command-run`, `pre-install-cmd`,
`pre-pool-create`, `pre-operations-exec`, `pre/post-package-install|update|uninstall`,
`pre/post-file-download` (plugins can rewrite URLs and cache keys),
`pre/post-autoload-dump`, `post-install-cmd`. Scripts: shell and `@php`
strings as subprocesses; `Class::method` callables run inside Composer's
process with a `Composer\Script\Event`. Plugins implement `PluginInterface`,
gated by `config.allow-plugins`, loaded right after install.

Popular plugins (Packagist monthly installs, Oct 2026; phpunit is 20.6M):

| Package | Monthly | What it does |
|---|---|---|
| php-http/discovery | 12.5M | may write generated classes into vendor |
| pestphp/pest-plugin | 7.4M | plugin manifests |
| dealerdirect/phpcodesniffer-composer-installer | 6.9M | PHPCS installed_paths |
| symfony/runtime | 5.6M | writes `vendor/autoload_runtime.php`; app breaks without it |
| phpstan/extension-installer | 5.4M | `GeneratedConfig.php` |
| symfony/flex | 5.4M | recipes, symfony.lock, hooks downloads |
| cweagans/composer-patches | 3.5M | applies patches; Drupal depends on it |
| composer/installers | 3.1M | custom paths for wordpress-plugin, drupal-module, ... |
| tbachert/spi | 2.9M | OpenTelemetry SPI class generation |
| drupal/core-composer-scaffold | 1.3M | copies index.php, .htaccess into web root |
| bamarni/composer-bin-plugin | 1.2M | isolated vendor-bin dirs |
| wikimedia/composer-merge-plugin | 1.1M | merges other composer.json files; affects resolution |

Laravel skeleton: zero plugins, but `post-autoload-dump` is
`Illuminate\Foundation\ComposerScripts::postAutoloadDump` (PHP callable) then
`@php artisan package:discover --ansi`, which reads installed.json.

## 7. Resolver (for Phase 07)

src/Composer/DependencyResolver/: `PoolBuilder` (lazy loading per
name+constraint, unlock via update allow-list, advisory and filter-list pool
filters, root aliases), `PoolOptimizer`, `RuleSetGenerator` (ROOT_REQUIRE,
FIXED, PACKAGE_REQUIRES, PACKAGE_CONFLICT, PACKAGE_SAME_NAME, ALIAS,
LEARNED), `Solver` (CDCL port of libsolv, 2-literal watches), `DefaultPolicy`
(highest; prefer-stable; prefer-lowest; repo priority; replacer vs original),
`LockTransaction`.

Constraints (composer/semver; Composer 2.10.3 bundles semver 3.4.4): `^ ~ * .x`, ranges, `||`, AND by comma/space,
stability suffixes, `minimum-stability`, `prefer-stable`, `dev-*`,
`9999999-dev` default-branch alias, `extra.branch-alias`, inline `as`
aliases, `replace/provide/conflict`, `self.version`, platform packages
(`php`, `php-64bit`, `php-ipv6`, `php-zts`, `php-debug`, `ext-*`, `lib-*`,
`composer-plugin-api`, `composer-runtime-api`, `composer`).

Slow cases: Magento 2, Drupal, large symfony/* replacements, partial updates
with dependencies. **[FREE]** algorithm; **[EXACT]** chosen versions and lock bytes.

## 8. What Composer 2 already does fast

curl_multi with HTTP/2 multiplexing (12 jobs, 8 per host), async promises,
async `unzip` subprocesses (`COMPOSER_MAX_PARALLEL_PROCESSES`, default 10),
lazy p2 metadata, PoolOptimizer, shared files cache.

Still slow: PHP start and bootstrap, the platform solve even on install,
per-package install and rename, single-threaded `php_strip_whitespace` class
scanning for `-o`, re-extraction on every warm install, installed.* rewrite,
filter and audit round trips. Scripts like `package:discover` cannot be sped up.

## Hardest traps, ranked

1. Plugins and PHP-callable scripts need PHP → fallback (decision 0004).
2. Class discovery depends on PHP's lexer: comments, heredoc/nowdoc,
   `?>` inline HTML, `__halt_compiler`, the enum gate.
3. Byte-exact `var_export` / `dumpToPhpCode`, PackageSorter weights with a
   stable sort and `strnatcasecmp`.
4. content-hash via PHP `json_encode` flags 0.
5. Platform detection via the real `php`, overrides and ignore flags.
6. Root version guessing via git/hg/svn.
7. 2.10 malware filter and audit.
8. readdir order for duplicate classes (reproduced by reading the directory;
   creation order can still differ on ext4).
9. Zip semantics: top-dir strip, exec bits, `.DS_Store`, shared install paths.
10. GitHub: empty shasum, 60 req/h unauthenticated, codeload 400 retries,
    auth precedence.
11. Path repos: relative symlinks, Windows junctions, relative install-path.
12. `plugin-api-version` differs by Composer version.

## Sources

github.com/composer/composer @0fc32b9: Installer.php, Package/Locker.php,
Json/JsonFile.php, Package/Dumper/ArrayDumper.php,
Autoload/AutoloadGenerator.php, Autoload/ClassLoader.php,
Util/PackageSorter.php, Installer/{Library,Binary}Installer.php,
Installer/InstallationManager.php, Downloader/*, Repository/{Filesystem,Composer}Repository.php,
Util/HttpDownloader.php, Util/Http/CurlDownloader.php, DependencyResolver/*,
Policy/MalwarePolicyConfig.php, Config.php, CHANGELOG.md,
doc/05-repositories.md, doc/articles/authentication-for-private-packages.md ·
github.com/composer/class-map-generator ·
github.com/composer/metadata-minifier ·
repo.packagist.org/packages.json · repo.packagist.org/p2/monolog/monolog.json ·
packagist.org/apidoc · getcomposer.org/doc/articles/autoloader-optimization.md
