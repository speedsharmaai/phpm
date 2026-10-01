#!/usr/bin/env bash
# Head-to-head install benchmark: cold, warm and no-op, with hyperfine.
#
#   tools/bench/bench.sh <fixture> [tools...]
#
# tools: composer phpm riff viv vivacity (default: all that are on PATH or set)
# Binaries: PHPM_BIN, RIFF_BIN, VIV_BIN, VIVACITY_BIN (default: name on PATH).
# Output: target/bench/<fixture>/<tool>-{cold,warm,noop}.json
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
fixture="${1:?fixture name, e.g. laravel-skeleton}"
shift
tools=("$@")
[ ${#tools[@]} -eq 0 ] && tools=(composer phpm riff viv vivacity)

src="$root/fixtures/$fixture"
[ -f "$src/fixture.lock" ] || { echo "no fixture: $src" >&2; exit 2; }
work="$root/target/bench/$fixture"
mkdir -p "$work"

plugin_free=true
if grep -q '"type": "composer-plugin"' "$src/fixture.lock"; then plugin_free=false; fi

# The runs happen inside the work dir, so relative binary paths are resolved first.
absolute() {
  case "$1" in
    */*) echo "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")" ;;
    *) echo "$1" ;;
  esac
}
for var in PHPM_BIN RIFF_BIN VIV_BIN VIVACITY_BIN; do
  [ -n "${!var:-}" ] && export "$var=$(absolute "${!var}")"
done

cmd_for() {
  case "$1" in
    composer) echo "composer install --no-scripts --no-plugins --no-interaction --no-progress -q" ;;
    phpm) echo "${PHPM_BIN:-phpm} install --no-scripts --no-plugins" ;;
    riff) echo "${RIFF_BIN:-riff} install --no-scripts --no-plugins" ;;
    viv) echo "${VIV_BIN:-viv} install --no-scripts --no-plugins --cache-dir $work/cache/viv" ;;
    vivacity) echo "${VIVACITY_BIN:-vivacity} install --no-progress" ;;
    *) echo "unknown tool $1" >&2; exit 2 ;;
  esac
}

export COMPOSER_CACHE_DIR="$work/cache/composer"
export PHPM_CACHE_DIR="$work/cache/phpm"
export VIVACITY_CACHE_DIR="$work/cache/vivacity"
export XDG_CACHE_HOME="$work/cache/xdg"
fake_home="$work/home"
mkdir -p "$fake_home"

for t in "${tools[@]}"; do
  bin=$(cmd_for "$t" | cut -d' ' -f1)
  command -v "$bin" >/dev/null || { echo "skip $t: $bin not found"; continue; }
  dir="$work/${t:?}"
  rm -rf "${dir:?}"
  mkdir -p "$dir"
  cp "$src/fixture.json" "$dir/composer.json"
  cp "$src/fixture.lock" "$dir/composer.lock"
  cmd=$(cmd_for "$t")
  clear_caches="rm -rf ${work:?}/cache ${fake_home:?}/Library/Caches"

  echo "== $t ($fixture)"
  (cd "$dir" && HOME="$fake_home" $cmd) >"$work/$t.log" 2>&1 || { echo "  failed, see $work/$t.log"; continue; }

  cd "$dir"
  HOME="$fake_home" hyperfine -N --runs 3 --warmup 0 \
    --prepare "bash -c 'rm -rf ${dir:?}/vendor; $clear_caches'" \
    -n "$t cold" "$cmd" --export-json "$work/$t-cold.json" | grep -E 'Time|Range'
  HOME="$fake_home" hyperfine -N --runs 5 --warmup 1 \
    --prepare "rm -rf ${dir:?}/vendor" \
    -n "$t warm" "$cmd" --export-json "$work/$t-warm.json" | grep -E 'Time|Range'
  HOME="$fake_home" hyperfine -N --runs 10 --warmup 1 \
    -n "$t no-op" "$cmd" --export-json "$work/$t-noop.json" | grep -E 'Time|Range'
done

echo "plugin-free fixture: $plugin_free"
