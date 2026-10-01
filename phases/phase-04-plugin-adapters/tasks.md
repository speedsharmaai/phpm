# Phase 04 tasks

Each adapter lands as its own PR with unit tests built from the plugin's own
documented cases and golden output from real Composer, `just ci` green and
Sonar green. An adapter is on only for its exact package name and a version
range whose source was read and compared; anything else falls back to
Composer exactly as Phase 02 does (decision 0004).

Verification for every adapter: `just e2e-apps` (Composer and phpm at the
same path, `vendor/` plus `web/`, `public/`, `config/`, `recipes/` compared
with `phpm-diffvendor`, 0 differences) and the app smoke checks
(`Drupal::VERSION`, `$wp_version`, `bin/console about`, `artisan --version`).

## Groundwork

- [x] Install paths: one map from package to install directory used by
  placement, `installed.json`/`installed.php` `install-path`, the
  autoloader, bin proxies and removal, so a plugin can move a package.
- [x] Local repository order: plugins that walk the local repository
  (pest-plugin, `include_paths.php`) see Composer's first install in the
  order archives finished unzipping, which varies; a second install uses
  `installed.json`'s order. phpm uses that order, and `just e2e-apps`
  compares against a second Composer install before counting a difference,
  as the sweep does.
- [x] Adapter registry: decide per active plugin, before anything is placed,
  whether a native adapter covers it; `--explain` says which and why not.
  Installer plugins go native only when every active plugin is covered: with
  any other plugin left, Composer runs the whole install as before, because
  its hooks would see a different local repository.

## composer/installers

- [x] Every installer type at the locked version (2.3.0): framework prefix
  lookup as `Installer::findFrameworkType`, `supports()` pattern, each
  installer's locations and `inflectPackageVars`.
- [x] Root `extra.installer-paths` with `{$name}`, `{$vendor}`, `{$type}`
  and `type:`, `vendor:` and package-name keys; `extra.installer-name`;
  `extra.installer-disable`.
- [x] Installers with side effects or environment lookups (CakePHP's local
  repository check, Oxid's vendor metadata file, Bitrix's duplicate prompt)
  stay on fallback. Checked against the real plugin on 855 type and name
  cases (`crates/phpm/tests/golden/installers/`).
- [x] roots/wordpress-core-installer (1.100.0 to 4.0.0, identical sources):
  `wordpress-install-dir` for `wordpress-core` packages. johnpbloch's
  stays on fallback.
- [x] Bedrock's `web/` identical; `install-path` in `installed.*` matches.
  Confirmed via `e2e_app_bedrock` with `native: true`: no step hands off to
  Composer, `vendor/` and `web/` byte-identical, `$wp_version` reads back.

## drupal/core-composer-scaffold

- [x] File mappings from allowed packages (implicit `drupal/core` and
  `drupal/legacy-scaffold-assets`, root `allowed-packages`, recursive).
  Only the plain `replace` shape (a string, or `{path, overwrite}`) is
  reproduced; `append`/`prepend`/`mode` and `overwrite: false` decline
  rather than guess, as does a root package with its own
  `drupal-scaffold.file-mapping`. Later packages override earlier ones for
  the same destination, matching `ScaffoldFileCollection`. `symlink` mode
  is a Windows-only Composer concern and out of scope.
- [x] `locations` (`web-root`, `project-root`) and the interpolator;
  `[web-root]/autoload.php` and `autoload_runtime.php` regenerated from the
  real plugin templates (not "unless committed" — Composer's own plugin
  always overwrites them). `.gitignore` management: phpm shells to
  `git rev-parse --show-toplevel` and `git check-ignore vendor` and only
  goes native when management would stay disabled (no committed
  `.gitignore` ignoring `vendor`), the real drupal-recommended fixture's
  case; declines rather than reproduce the gitignore-writing path.
- [x] `preAutoloadDump`: the root classmap additions (conditional
  symfony/http-foundation, symfony/http-kernel, symfony/dependency-injection
  and psr/container files) and `vendor/drupal/DrupalInstalled.php` with its
  xxh3 versions hash (`twox-hash`, verified byte-for-byte against PHP's
  `hash('xxh3', ...)`). Threaded into `phpm-autoload` as
  `Options.extra_root_classmap`, appended to the root package's `classmap`
  exactly as Composer's `AutoloadGenerator` does for an absolute path on
  the root (empty `install_path`, so no path rewriting).
  Pinned to the exact commit drupal-recommended's lock locks
  (`Versions::Reference`), since it is a dev-branch package with no tagged
  release and the version string alone cannot distinguish one commit from
  another.
- [x] drupal/core-project-message (prints only, verified it writes nothing)
  and drupal/core-recipe-unpack (`postUpdate`/`postCreateProject` hooks,
  neither of which `composer install` fires) are `Role::NoOp`.
  drupal/core-vendor-hardening is not in the fixture; it stays on fallback.
- [x] Scaffold destinations can be in directories placement never creates
  (`recipes/README.txt`, `web/sites/default/...`); the shared adapter
  `write()` helper now `create_dir_all`s the parent first, matching
  Composer's `Filesystem::ensureDirectoryExists` before every write.

## symfony/runtime

- [x] `vendor/autoload_runtime.php` from the package's own template with
  `extra.runtime` options, `project_dir` nesting, custom
  `autoload_template`, `runtime: false`. Verified every stable tag
  v7.0.0-v8.1.0 (4 content changes, each a comment or a `\sprintf`
  namespace prefix, never the generated bytes); the common case matches
  drupal-recommended's and symfony-demo's real `vendor/autoload_runtime.php`
  byte for byte. Covered only when it is the only active plugin; both
  fixtures still have others, so neither goes fully native from this alone.

## phpstan/extension-installer

- [x] `GeneratedConfig.php`: extensions, `NOT_INSTALLED`, the compacted
  phpstan constraint, `ignore`; absolute install paths compared at the same
  path. Verified 1.4.0-1.4.3 (byte-identical sources). The compacted
  constraint is the intersection of each qualifying package's own
  `phpstan/phpstan` bound (`Bound::is_higher_than`/`is_lower_than`, the same
  fold `MultiConstraint::extractBounds` does for a conjunction); declines
  when the intersection is empty rather than guess at `Intervals`' general
  interval algebra. `relative_install_path` and extensions' shape checked
  against drupal-recommended's real captured `GeneratedConfig.php`.
  Declines a dev/branch-version qualifying package (would need
  `getFullPrettyVersion()`'s truncated-reference suffix) rather than guess.

## php-http/discovery

- [x] Verified against the real `src/Composer/Plugin.php` (1.20.0, the
  version drupal-recommended locks): `postUpdate` binds to
  `post-update-cmd`, which `composer install` never runs, so only
  `preAutoloadDump` matters. With `extra.discovery` empty or unset (the
  common case, true for drupal-recommended) its only effect is removing a
  stale `vendor/composer/GeneratedDiscoveryStrategy.php` from an earlier
  run; phpm reproduces exactly that. A pinned `extra.discovery` would
  generate and classmap a candidate-strategy class; phpm declines rather
  than reproduce that generation.

## dealerdirect/phpcodesniffer-composer-installer

- [x] `CodeSniffer.conf` `installed_paths` as the plugin computes them
  (Finder depth rules, relative paths, sort). Verified 1.2.1 only (every
  release changed `src/Plugin.php`). Checked against a real install of
  drupal-recommended's exact packages (drupal/coder, sirbrillig's and
  slevomat's standards): identical `installed_paths` string, byte for byte.
  No existing-config merge: phpm only does fresh installs, and the file is
  entirely plugin-managed (never user-edited in practice), so a full
  recompute each run matches Composer's real first-install behaviour; the
  file is written only when phpm finds at least one `ruleset.xml` (matching
  Composer's own `saveInstalledPaths` only running on a change).

## Others the fixtures load

- [ ] pestphp/pest-plugin: `vendor/pest-plugins.json` in local repository
  order (Bedrock dev).
- [ ] symfony/flex, cweagans/composer-patches and
  wikimedia/composer-merge-plugin stay on full fallback (scope).

## Exit

- [x] Bedrock and drupal-recommended install with no Composer at all,
  identical trees, working apps. `e2e_app_bedrock` and
  `e2e_app_drupal_recommended` both run with `native: true`: every active
  plugin (composer/installers, drupal/core-composer-scaffold,
  drupal/core-project-message, drupal/core-recipe-unpack, symfony/runtime,
  dealerdirect/phpcodesniffer-composer-installer, php-http/discovery,
  phpstan/extension-installer for drupal-recommended;
  composer/installers, roots/wordpress-core-installer for bedrock) is
  native, no step hands off to Composer, `vendor/`/`web/`/`recipes/`
  byte-identical, and `Drupal::VERSION`/`$wp_version` read back.
- [ ] Warm benchmarks for both against Composer under
  `bench/results/<date>-phase-04/`.
- [ ] `progress.md`, README numbers and roadmap, WORKLIST.
