#!/usr/bin/env bash
# Checks for the install-smoke scripts: shellcheck, then the helpers in lib.sh.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/install-smoke/lib.sh
. "$here/lib.sh"

shellcheck -x "$here/run" "$here/test.sh" "$here/lib.sh"

failures=0
check() {
  local name="$1" want="$2" got="$3"
  if [ "$want" = "$got" ]; then
    echo "ok   $name"
  else
    echo "FAIL $name"
    echo "  want: $want"
    echo "  got:  $got"
    failures=$((failures + 1))
  fi
}
yes_no() { if "$@"; then echo yes; else echo no; fi; }

check "release tag is valid" yes "$(yes_no valid_tag v0.1.0)"
check "prerelease tag is valid" yes "$(yes_no valid_tag v0.1.0-rc.1)"
check "tag without v is rejected" no "$(yes_no valid_tag 0.1.0)"
check "tag with shell metacharacters is rejected" no "$(yes_no valid_tag 'v0.1.0;rm')"
check "tag with a path is rejected" no "$(yes_no valid_tag 'v0.1.0/../x')"
check "asset url" "https://github.com/speedsharmaai/phpm/releases/download/v0.1.0-rc.1/phpm-installer.sh" \
  "$(asset_url v0.1.0-rc.1 phpm-installer.sh)"
check "tag version drops the v" "0.1.0-rc.1" "$(tag_version v0.1.0-rc.1)"
check "version output matches its tag" yes "$(yes_no version_matches v0.1.0-rc.1 'phpm 0.1.0-rc.1')"
check "version output for another tag does not" no "$(yes_no version_matches v0.1.0 'phpm 0.1.0-rc.1')"

check "composer phar url" "https://getcomposer.org/download/2.10.3/composer.phar" \
  "$(composer_phar_url 2.10.3)"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
printf abc >"$tmp"
check "sha256 of a known file matches" yes \
  "$(yes_no sha256_matches "$tmp" ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad)"
check "a different sha256 does not" no \
  "$(yes_no sha256_matches "$tmp" 0000000000000000000000000000000000000000000000000000000000000000)"

[ "$failures" -eq 0 ] || { echo "$failures failed"; exit 1; }
