# Vendored Composer files

Copied byte for byte from what Composer 2.10.3 writes into
`vendor/composer/` during `composer install`, not from Composer's git
repository. `phpm_lock::COMPOSER_VERSION` names the release.

| File | Source |
|---|---|
| `InstalledVersions.php` | `vendor/composer/InstalledVersions.php` after `composer install` |
| `ClassLoader.php` | `vendor/composer/ClassLoader.php` after `composer install` |
| `LICENSE` | `vendor/composer/LICENSE`, Composer's MIT licence with the blank first and last lines the phar adds |

`just golden` checks the copy against a fresh Composer install. When the
reference Composer version changes, update the files and the constant
together. `phpm-autoload` writes `ClassLoader.php` and `LICENSE` from here.
