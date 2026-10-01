# Phase 04: Plugin adapters

Tasked in [tasks.md](tasks.md). Starts in parallel with Phase 03; the sweep
confirms each adapter at the Phase 03 and Phase 04 gates.

## Why

Fallback keeps plugin projects correct, but it gives back most of the speed.
A handful of plugins cover most real lockfiles. Each one that runs natively
moves a class of projects from "correct" to "correct and fast".

## What ships

Native adapters, each shipped only when the sweep shows its output
identical to the real plugin's on every fixture that uses it:

| Plugin | Why first |
|---|---|
| composer/installers | Bedrock/WordPress, Drupal, CMS paths; pure path mapping |
| symfony/runtime | writes `autoload_runtime.php`; Symfony apps break without it |
| phpstan/extension-installer | writes `GeneratedConfig.php`; very common in dev |
| php-http/discovery | 12.5M monthly; usually a no-op at install |
| dealerdirect/phpcodesniffer-composer-installer | sets PHPCS installed_paths |
| drupal/core-composer-scaffold | copies scaffold files; Drupal needs it |

symfony/flex, cweagans/composer-patches and wikimedia/composer-merge-plugin
stay on fallback. Flex runs recipes and hooks downloads; patches mutates
package contents; merge-plugin changes resolution. Emulating them partially
is exactly what decision 0004 forbids. Revisit only if the sweep shows
they dominate the failures.

## Exit criteria

- Each adapter identical to the real plugin across the sweep.
- Bedrock and a Drupal site install without falling back.

## Out of scope

Running arbitrary third-party plugins natively. That is what Composer is for.
