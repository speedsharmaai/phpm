#!/usr/bin/env bash
# Smoke test for the setup-phpm action scaffold: shellcheck every script,
# then exercise detect-target.sh's pure OS/arch mapping (the one piece that
# needs no network) against each of this workspace's build targets.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
shellcheck "$here/test.sh" "$here"/scripts/*.sh
# action.yml itself is covered by the repo-wide zizmor-action CI step, which
# scans "." by default and discovers action.yml files at any path.

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

run_detect() {
  local fake_uname="$1"
  env -i PATH="$PATH" GITHUB_OUTPUT=/dev/stdout \
    bash -c "uname() { if [ \"\$1\" = -s ]; then echo '${fake_uname%%:*}'; else echo '${fake_uname##*:}'; fi; }; export -f uname; source '$here/scripts/detect-target.sh'"
}

check "maps apple silicon to the macOS aarch64 target" \
  "triple=aarch64-apple-darwin
ext=tar.xz" "$(run_detect 'Darwin:arm64')"
check "maps intel macs to the macOS x86_64 target" \
  "triple=x86_64-apple-darwin
ext=tar.xz" "$(run_detect 'Darwin:x86_64')"
check "maps linux arm64 to the musl aarch64 target" \
  "triple=aarch64-unknown-linux-musl
ext=tar.xz" "$(run_detect 'Linux:aarch64')"
check "maps linux x86_64 to the musl x86_64 target" \
  "triple=x86_64-unknown-linux-musl
ext=tar.xz" "$(run_detect 'Linux:x86_64')"
check "maps a windows git-bash runner to the msvc target" \
  "triple=x86_64-pc-windows-msvc
ext=zip" "$(run_detect 'MINGW64_NT-10.0:x86_64')"

if ! out=$(run_detect 'Linux:riscv64' 2>&1); then
  check "an unsupported arch fails with a clear message" "true" "$(echo "$out" | grep -qF 'no release binary for Linux/riscv64' && echo true)"
else
  check "an unsupported arch fails with a clear message" "true" "false"
fi

[ "$failures" -eq 0 ] || {
  echo "$failures failed"
  exit 1
}
