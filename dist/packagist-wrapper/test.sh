#!/usr/bin/env bash
# Smoke test for the Packagist wrapper scaffold: every .php file parses,
# composer.json validates, and the pure platform/path logic in
# src/Installer.php behaves (tests/run.php, no PHPUnit dependency).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
shellcheck "$here/test.sh"

for f in "$here"/bin/phpm "$here"/src/*.php "$here"/tests/*.php; do
  php -l "$f" >/dev/null
done

(cd "$here" && composer validate --no-check-publish)

php "$here/tests/run.php"
