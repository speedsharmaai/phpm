#!/usr/bin/env bash
# Checks for the bench-real scripts: shellcheck, then the pure helpers in
# lib.sh on small inputs.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/bench-real/lib.sh
. "$here/lib.sh"

scripts=()
for f in "$here"/*; do
  case "$(head -1 "$f")" in '#!/usr/bin/env bash' | '# shellcheck shell=bash'*) scripts+=("$f") ;; esac
done
shellcheck -x "${scripts[@]}"

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

check "opt reads the value after its flag" "b" "$(opt x "" a --x b c)"
check "opt falls back to its default" "d" "$(opt missing d a b)"
check "opt returns the first match" "1" "$(opt x "" --x 1 --x 2)"

explain=$(mktemp)
trap 'rm -f "$explain"' EXIT
cat >"$explain" <<'EOF'
decision plugins: a/one, a/two not native, no adapter for version 9
decision plugins: b/three allowed, so Composer runs the steps they hook
decision install: native, phpm fetches and places every package
EOF
check "fallback plugin names, deduped and sorted" \
  "$(printf 'a/one\na/two\nb/three')" \
  "$(fallback_plugin_names "$explain")"

empty=$(mktemp)
trap 'rm -f "$empty"' EXIT
check "no decision lines means no plugin names" "" "$(fallback_plugin_names "$empty")"

proj=$(mktemp -d)
trap 'rm -rf "$proj"' EXIT
echo '{}' >"$proj/composer.json"
check "vendor_dir defaults to vendor" "$proj/vendor" "$(vendor_dir "$proj")"
echo '{"config":{"vendor-dir":"lib/composer"}}' >"$proj/composer.json"
check "vendor_dir honours config.vendor-dir" "$proj/lib/composer" "$(vendor_dir "$proj")"
echo '{"config":{"vendor-dir":"upload/system/storage/vendor/"}}' >"$proj/composer.json"
check "vendor_dir drops a trailing slash" "$proj/upload/system/storage/vendor" "$(vendor_dir "$proj")"

[ "$failures" -eq 0 ] || { echo "$failures failed"; exit 1; }
