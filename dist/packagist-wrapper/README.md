# Packagist wrapper (scaffold, not submitted)

A minimal Composer package, `speedsharma/phpm`, whose only job is to fetch
the phpm release binary for the current platform and expose it as
`vendor/bin/phpm`. This is what makes `setup-php`'s `tools: phpm` work on
day one, before any upstream PR to shivammathur/setup-php lands — setup-php
resolves tool names it doesn't know natively via Packagist.

Not published. `speedsharma/phpm` is unclaimed on Packagist as of this
check (see the phase 06 report).

## Layout

```text
composer.json   speedsharma/phpm, bin: ["bin/phpm"], post-install/post-update hooks
src/Installer.php   platform mapping, download, extract — Speedsharma\Phpm\Installer
bin/phpm        the shim composer symlinks to vendor/bin/phpm
tests/run.php   standalone smoke test (no PHPUnit), run via test.sh
```

## How it works

Same shape as the npm wrapper (`dist/npm-wrapper/`) and the shell/npm
installers cargo-dist generates, adapted to Composer idioms instead of
copying either:

- `post-install-cmd`/`post-update-cmd` eagerly download the binary for the
  current `PHP_OS_FAMILY`/`php_uname('m')` into
  `bin/.bin_real/<version>/phpm` right after `composer install`.
- `bin/phpm` (what `vendor/bin/phpm` resolves to) downloads lazily on first
  run if that eager step didn't happen — `composer install --no-scripts`,
  an offline install that's since gone online, etc. — so the eager step is
  allowed to fail without failing the whole `composer install`.
- Both paths share `Installer::ensureInstalled()`, `::platformTarget()`,
  `::artifactName()`.

`Installer::platformTarget()` only knows the 5 targets this workspace
builds (`dist-workspace.toml`); anything else throws
`UnsupportedPlatformException` naming what's supported, same as the npm
wrapper's `binary.js` error path.

## What's not done here, and why

- Not registered on Packagist (that's a "publish something public" action,
  owner go-ahead required).
- No upstream PR to `shivammathur/setup-php` for a first-class `tools: phpm`
  entry — this wrapper exists so `tools: phpm` already works via Packagist
  before that PR lands, per decision 0006 and the phase 06 "Public steps".
- The version is read from this package's own `composer.json` at runtime
  (`Installer::packageVersion()`), which is `0.0.0` here; a real release
  bumps it via release-plz, same as the main crate's `[workspace.package]
  version`.

## Smoke test

```sh
dist/packagist-wrapper/test.sh
```

`php -l` on every PHP file, `composer validate`, and `tests/run.php` against
`Installer`'s pure functions (platform mapping, artifact naming, path
building). The network-touching `download()`/`extract()` methods aren't
exercised here, same split the npm wrapper's test makes.
