#!/usr/bin/env bash
# Checks for the sweep scripts: shellcheck, then the jq programs on small inputs.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/sweep/lib.sh
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

entry() {
  jq -c --arg repo "${2:-}" --arg fixture "${3:-}" --arg pin "${4:-}" \
    --arg source s --arg taken 2026-10-01 "$ENTRY_JQ" <<<"$1"
}

node='{"nameWithOwner":"a/app","stargazerCount":12,"isArchived":false,"isFork":false,"isPrivate":false,
  "defaultBranchRef":{"target":{"oid":"abc"}},"lock":{"byteSize":10},"json":{"byteSize":5}}'

check "keeps a repository with a lock" \
  '{"repo":"a/app","commit":"abc","stars":12,"taken":"2026-10-01","source":"s","notes":""}' \
  "$(entry "$node")"
check "drops a repository without a lock" \
  '{"repo":"a/app","dropped":"no root composer.lock at the default branch"}' \
  "$(entry "$(jq -c '.lock = null' <<<"$node")")"
check "drops a fork" '{"repo":"a/app","dropped":"fork"}' \
  "$(entry "$(jq -c '.isFork = true' <<<"$node")")"
check "drops an archived repository" '{"repo":"a/app","dropped":"archived"}' \
  "$(entry "$(jq -c '.isArchived = true' <<<"$node")")"
check "drops a repository without composer.json" '{"repo":"a/app","dropped":"no root composer.json"}' \
  "$(entry "$(jq -c '.json = null' <<<"$node")")"
check "names a repository that does not exist" '{"repo":"gone/app","dropped":"not found"}' \
  "$(entry null gone/app)"
check "a fixture stands in for a missing lock, at the app's pinned commit" \
  '{"repo":"a/app","commit":"def","stars":12,"taken":"2026-10-01","source":"s","notes":"no committed lock; installs fixtures/app-fx in this repository","fixture":"app-fx"}' \
  "$(entry "$(jq -c '.lock = null' <<<"$node")" a/app app-fx def)"

merge() {
  jq -c -s --argjson target "$1" --arg taken 2026-10-01 "$MERGE_JQ" <(echo "$2") <(echo "$3")
}
picked='[{"repo":"x/one","stars":5},{"repo":null,"fixture":"ours","stars":null},{"repo":"x/gone","dropped":"fork"}]'
found='[{"repo":"y/big","stars":900},{"repo":"X/One","stars":5},{"repo":"y/small","stars":10},{"repo":"y/mid","stars":50},{"repo":"y/x","dropped":"fork"}]'

check "hand-picked first, search results by stars, cut to the target" \
  '["y/big","y/mid","x/one","fixture:ours"]' \
  "$(merge 4 "$picked" "$found" | jq -c '[.projects[] | .repo // ("fixture:" + .fixture)]')"
check "lists dropped hand-picked repositories" '[{"repo":"x/gone","dropped":"fork"}]' \
  "$(merge 4 "$picked" "$found" | jq -c '.dropped')"
check "never cuts hand-picked projects" '2' \
  "$(merge 1 "$picked" "$found" | jq -c '.projects | length')"
check "keeps one entry per repository, ignoring case" '1' \
  "$(merge 10 "$picked" "$found" | jq -c '[.projects[] | select(.repo != null and (.repo | ascii_downcase) == "x/one")] | length')"

[ "$failures" -eq 0 ] || { echo "$failures failed"; exit 1; }
