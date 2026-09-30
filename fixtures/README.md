# Fixtures

Pinned `composer.json` + `composer.lock` pairs. Never updated during a phase;
a new snapshot is a new directory. Excluded from every formatter and hook, and
marked `-text` in `.gitattributes`, because phpm's output is compared to
Composer's byte for byte.

| Fixture | Source | Taken | Prod | Dev | Plugins | Flags in Phase 01 |
|---|---|---|---|---|---|---|
| `laravel-skeleton` | `composer create-project laravel/laravel`, then `composer update` | 2026-10-01 | 76 | 33 | none | none |
| `symfony-demo` | symfony/demo@8d2e2ef | 2026-10-01 | 99 | 54 | symfony/flex, symfony/runtime | `--no-plugins` |
| `monica` | monicahq/monica@6e6ec21 | 2026-10-01 | 165 | 76 | php-http/discovery, phpstan/extension-installer | `--no-plugins` |
| `ytmate` | our app, saas/ytmate.in | 2026-10-01 | 15 | 27 | none | none |
| `wicketyaari` | our app, saas/wicketyaari.in | 2026-10-01 | 0 | 27 | none | none |

Reference tool: Composer 2.10.3 on PHP 8.4.
