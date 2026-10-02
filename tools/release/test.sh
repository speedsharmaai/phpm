#!/usr/bin/env bash
# Lints tools/release, then runs check-dist-calls on good and bad input
# and on the real release workflow.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
shellcheck "$here/check-dist-calls" "$here/test.sh"

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
exit_of() { if "$@" >/dev/null; then echo 0; else echo 1; fi; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat >"$tmp/good.yml" <<'YML'
        run: |
          dist $DIST_PLAN_CMD --output-format=json > plan.json
          dist build $DIST_TAG_FLAG --output-format=json "--artifacts=global" > m.json
          dist host $DIST_TAG_FLAG --steps=upload > m.json
          dist print-upload-files-from-manifest --manifest m.json
          echo "dist ran successfully"
YML
cat >"$tmp/bad.yml" <<'YML'
        run: |
          dist $DIST_TAG_FLAG --output-format=json "--artifacts=global" > m.json
YML

check "accepts every subcommand the workflow uses" 0 "$(exit_of "$here/check-dist-calls" "$tmp/good.yml")"
check "rejects a call with no subcommand" 1 "$(exit_of "$here/check-dist-calls" "$tmp/bad.yml")"
check "names the line" "$tmp/bad.yml:2: dist called without a subcommand: dist \$DIST_TAG_FLAG --output-format=json \"--artifacts=global\" > m.json" \
  "$("$here/check-dist-calls" "$tmp/bad.yml" || true)"
check "the release workflow passes" 0 "$(exit_of "$here/check-dist-calls" "$here/../../.github/workflows/v-release.yml")"

[ "$failures" -eq 0 ] || { echo "$failures failed"; exit 1; }
