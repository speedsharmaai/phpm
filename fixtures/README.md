# Fixtures

Pinned `composer.json` + `composer.lock` pairs, stored as `fixture.json` and
`fixture.lock` so GitHub's dependency graph does not treat them as this
repository's dependencies (they pin old, sometimes vulnerable versions on
purpose). Tests and `tools/bench` copy them back to the Composer names.

Never updated during a phase; a new snapshot is a new directory. Excluded from every formatter and hook, and
marked `-text` in `.gitattributes`, because phpm's output is compared to
Composer's byte for byte.

| Fixture | Source | Taken | Prod | Dev | Plugins | Flags in Phase 01 |
|---|---|---|---|---|---|---|
| `laravel-skeleton` | `composer create-project laravel/laravel`, then `composer update` | 2026-10-01 | 76 | 33 | none | none |
| `symfony-demo` | symfony/demo@8d2e2ef | 2026-10-01 | 99 | 54 | symfony/flex, symfony/runtime | `--no-plugins` |
| `monica` | monicahq/monica@6e6ec21 | 2026-10-01 | 165 | 76 | php-http/discovery, phpstan/extension-installer | `--no-plugins` |
| `ytmate` | our app, saas/ytmate.in | 2026-10-01 | 15 | 27 | none | none |
| `wicketyaari` | our app, saas/wicketyaari.in | 2026-10-01 | 0 | 27 | none | none |
| `drupal-recommended` | drupal/recommended-project@dbcffd8 (11.x, 2026-05-11), its committed lock | 2026-10-01 | 69 | 85 | composer/installers, drupal/core-composer-scaffold, drupal/core-project-message, drupal/core-recipe-unpack, symfony/runtime, php-http/discovery, tbachert/spi; dev: phpstan/extension-installer, dealerdirect/phpcodesniffer-composer-installer | new in Phase 02 |
| `bedrock` | roots/bedrock@b905a0a (2026-09-22) has no lock; generated with `composer update --no-install` on Composer 2.10.3 | 2026-10-01 | 15 | 58 | composer/installers, roots/wordpress-core-installer; dev: pestphp/pest-plugin | new in Phase 02 |

Reference tool: Composer 2.10.3 on PHP 8.4.
