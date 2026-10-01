#!/usr/bin/env bash
# Smoke test for the npm wrapper scaffold: every .js file parses, and
# package.json's supportedPlatforms covers this workspace's real build
# targets (dist-workspace.toml) with no artifact name that was never built.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
shellcheck "$here/test.sh"

for f in "$here"/*.js; do
  node --check "$f"
done

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

pkg="$here/package.json"
real_artifacts='["phpm-aarch64-apple-darwin.tar.xz","phpm-x86_64-apple-darwin.tar.xz","phpm-aarch64-unknown-linux-musl.tar.xz","phpm-x86_64-unknown-linux-musl.tar.xz","phpm-x86_64-pc-windows-msvc.zip"]'

check "bin.phpm points at the postinstall-downloaded binary" \
  '"run-phpm.js"' "$(jq -c '.bin.phpm' "$pkg")"
check "postinstall runs install.js" \
  '"node ./install.js"' "$(jq -c '.scripts.postinstall' "$pkg")"
check "every supportedPlatforms entry names a real build artifact" \
  'true' \
  "$(jq -c --argjson real "$real_artifacts" \
    '[.supportedPlatforms[].artifactName] | unique | all(. as $a | $real | index($a) != null)' "$pkg")"
check "every real build artifact is reachable from some platform" \
  'true' \
  "$(jq -c --argjson real "$real_artifacts" \
    '[.supportedPlatforms[].artifactName] | unique as $named | $real | all(. as $r | $named | index($r) != null)' "$pkg")"

[ "$failures" -eq 0 ] || {
  echo "$failures failed"
  exit 1
}
