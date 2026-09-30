# 0004: Byte-identical output, or that part goes to Composer

Status: accepted
Date: 2026-10-01

## Context

Other tools read what Composer writes. Laravel's `package:discover` and
Symfony Flex parse `vendor/composer/installed.json`. PHPStan and others read
`installed.php` through `InstalledVersions`. PHPUnit relies on globals set in
the bin proxies. A `vendor/` that works but differs in bytes can break any of
these in ways that surface weeks later.

Plugins and PHP-callable scripts cannot run without PHP and Composer's own
classes. They are common:

- Laravel skeleton: zero plugins, but `post-autoload-dump` calls
  `Illuminate\Foundation\ComposerScripts::postAutoloadDump`, a PHP callable.
- Symfony: symfony/flex, symfony/runtime (breaks the app if missing).
- Drupal core: ten plugins, including scaffold and patches.
- Bedrock/WordPress: composer/installers.
- Many apps: php-http/discovery, phpstan/extension-installer.

The competitors chose differently: libretto has no plugin support, vivace
refuses symfony/flex, riff emulates flex and composer-patches natively. Each
choice breaks some projects, or trusts an emulation nobody has measured.

## Decision

1. The files Composer writes are reproduced byte for byte: `installed.json`,
   `installed.php`, `InstalledVersions.php`, `ClassLoader.php`, the
   `autoload_*.php` family, `platform_check.php`, and bin proxies. Templates
   Composer copies verbatim are vendored into the binary, pinned to a stated
   Composer version.
2. When the project uses a plugin phpm has no proven adapter for, or a
   PHP-callable script, phpm does the fast part (download, store, clone,
   metadata) and then runs `composer dump-autoload` and the relevant scripts
   through real Composer, which activates plugins and fires the events.
   Measured cost of that step on the Laravel skeleton: about 1 s.
3. A native plugin adapter ships only after the compatibility sweep shows its
   output identical to the real plugin's across every fixture that uses it.
4. `phpm install --explain` prints which path each package took and why.

## Consequences

- phpm is never worse than Composer, only sometimes not faster. That is the
  promise, and it is the difference from every competitor.
- Fallback needs Composer on PATH. When it is missing and a fallback is
  needed, phpm stops with a clear error rather than writing a partial vendor.
- The speed headline comes from plugin-free projects (Laravel default, most
  libraries). The honest benchmark page shows both paths.
