# Vendored Composer files

Copied byte for byte from what Composer 2.10.3 writes into
`vendor/composer/` during `composer install`, not from Composer's git
repository. `phpm_lock::COMPOSER_VERSION` names the release.

| File | Source |
|---|---|
| `InstalledVersions.php` | `vendor/composer/InstalledVersions.php` after `composer install` |
| `LICENSE` | Composer's MIT licence |

`just golden` checks the copy against a fresh Composer install. When the
reference Composer version changes, update both files and the constant
together.
