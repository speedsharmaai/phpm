#!/usr/bin/env bash
# Cold installs with the tools interleaved, so network drift hits all alike.
#
#   tools/bench/cold.sh <rounds> <fixture>...
#
# Each round installs once per tool from empty caches, rotating the order.
# Binaries: PHPM_BIN, RIFF_BIN, VIVACITY_BIN (default: name on PATH).
# Output: target/bench/cold/<fixture>-<tool>-r<round>.json, one hyperfine run each.
set -uo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
rounds="${1:?rounds, e.g. 8}"
shift
work="$root/target/bench/cold"
out="$work/results"
mkdir -p "$out" "$work/home"
export COMPOSER_CACHE_DIR="$work/cache/composer" PHPM_CACHE_DIR="$work/cache/phpm"
export VIVACITY_CACHE_DIR="$work/cache/vivacity" XDG_CACHE_HOME="$work/cache/xdg"

cmd_for() {
  case "$1" in
    composer) echo "composer install --no-scripts --no-plugins --no-interaction --no-progress -q" ;;
    phpm) echo "${PHPM_BIN:-phpm} install --no-scripts --no-plugins" ;;
    riff) echo "${RIFF_BIN:-riff} install --no-scripts --no-plugins" ;;
    vivacity) echo "${VIVACITY_BIN:-vivacity} install --no-progress" ;;
  esac
}

tools=(composer phpm riff vivacity)
for fixture in "$@"; do
  for r in $(seq 1 "$rounds"); do
    for i in 0 1 2 3; do
      t=${tools[$(((i + r) % 4))]}
      dir="$work/$fixture-$t"
      rm -rf "${dir:?}"
      mkdir -p "$dir"
      cp "$root/fixtures/$fixture/fixture.json" "$dir/composer.json"
      cp "$root/fixtures/$fixture/fixture.lock" "$dir/composer.lock"
      json="$out/$fixture-$t-r$r.json"
      (cd "$dir" && HOME="$work/home" hyperfine -N --runs 1 \
        --prepare "bash -c 'rm -rf ${dir:?}/vendor ${work:?}/cache ${work:?}/home/Library/Caches'" \
        -n "$t cold" "$(cmd_for "$t")" --export-json "$json" >/dev/null 2>&1) || echo "failed: $fixture $t round $r"
      echo "$fixture r$r $t $(jq '.results[0].mean' "$json" 2>/dev/null)"
    done
  done
done
