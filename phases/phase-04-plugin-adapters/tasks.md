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

- [ ] Install paths: one map from package to install directory used by
  placement, `installed.json`/`installed.php` `install-path`, the
  autoloader, bin proxies and removal, so a plugin can move a package.
- [ ] Local repository order: Composer's install order (`Transaction`
  ordering, plugins first) for plugins that read the local repository.
- [ ] Adapter registry: decide per active plugin, before anything is placed,
  whether a native adapter covers it; `--explain` says which and why not.
  When every active plugin is covered, the install runs with no Composer;
  when only the install-path plugins are covered, phpm places packages and
  Composer still runs the hooks of the rest.

## composer/installers

- [ ] Every installer type at the locked version (2.x): framework prefix
  lookup as `Installer::findFrameworkType`, `supports()` pattern, each
  installer's locations and `inflectPackageVars`.
- [ ] Root `extra.installer-paths` with `{$name}`, `{$vendor}`, `{$type}`
  and `type:`, `vendor:` and package-name keys; `extra.installer-name`;
  `extra.installer-disable`.
- [ ] Installers with side effects or environment lookups (CakePHP's local
  repository check, Oxid's vendor metadata file) stay on fallback.
- [ ] roots/wordpress-core-installer and johnpbloch's: `wordpress-install-dir`
  for `wordpress-core` packages.
- [ ] Bedrock's `web/` identical; `install-path` in `installed.*` matches.

## drupal/core-composer-scaffold

- [ ] File mappings from allowed packages (implicit `drupal/core` and
  `drupal/legacy-scaffold-assets`, root `allowed-packages`, recursive),
  `replace`/`append`/`prepend`/`default`/`skip`, `overwrite`,
  `force-append`, overrides between packages, unchanged files left alone.
- [ ] `locations` (`web-root`, `project-root`), `[web-root]/autoload.php`
  and `autoload_runtime.php` unless committed, `.gitignore` management
  through the same git commands the plugin runs, `symlink` mode.
- [ ] `preAutoloadDump`: the root classmap additions and
  `vendor/drupal/DrupalInstalled.php` with its xxh3 versions hash.
- [ ] drupal/core-project-message (prints only) and
  drupal/core-recipe-unpack (no install-time hook) handled natively.
  drupal/core-vendor-hardening is not in the fixture; it stays on fallback.

## symfony/runtime

- [ ] `vendor/autoload_runtime.php` from the package's own template with
  `extra.runtime` options, `project_dir` nesting, custom
  `autoload_template`, `runtime: false`.

## phpstan/extension-installer

- [ ] `GeneratedConfig.php`: extensions, `NOT_INSTALLED`, the compacted
  phpstan constraint, `ignore`; absolute install paths compared at the same
  path.

## php-http/discovery

- [ ] Install time is `preAutoloadDump` only: with no `extra.discovery` it
  removes a stale `GeneratedDiscoveryStrategy.php` and does nothing else.
  Pinned implementations change the root classmap; decide native or
  fallback by fidelity.

## dealerdirect/phpcodesniffer-composer-installer

- [ ] `CodeSniffer.conf` `installed_paths` as the plugin computes them
  (Finder depth rules, relative paths, sort), existing config merged.

## Others the fixtures load

- [ ] pestphp/pest-plugin: `vendor/pest-plugins.json` in local repository
  order (Bedrock dev).
- [ ] symfony/flex, cweagans/composer-patches and
  wikimedia/composer-merge-plugin stay on full fallback (scope).

## Exit

- [ ] Bedrock and drupal-recommended install with no Composer at all,
  identical trees, working apps.
- [ ] Warm benchmarks for both against Composer under
  `bench/results/<date>-phase-04/`.
- [ ] `progress.md`, README numbers and roadmap, WORKLIST.
