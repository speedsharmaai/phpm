# shellcheck shell=bash
# Helpers shared by `run` and its tests: hyperfine timing, the fetch-at-pin
# used by the sweep, and the JSON record each project emits.

bench_real_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

opt() {
  local name="$1" default="${2:-}"
  shift 2
  local args=("$@")
  for ((i = 0; i < ${#args[@]}; i++)); do
    if [ "${args[$i]}" = "--$name" ]; then
      echo "${args[$((i + 1))]}"
      return
    fi
  done
  echo "$default"
}

# Prepare the project's tree at its pin in $1; a fixture's composer.json
# and composer.lock go on top, same as the sweep (a fixture stands in when
# the app itself commits no lock, e.g. laravel/laravel).
prepare_source() {
  local src="$1" repo="$2" commit="$3" fixture="$4"
  mkdir -p "$src"
  if [ -n "$repo" ]; then
    git init -q "$src"
    local url="${BENCH_GIT_BASE:-https://github.com/}$repo.git"
    local tries="${BENCH_FETCH_TRIES:-3}" i=1
    until git -C "$src" fetch -q --depth 1 "$url" "$commit"; do
      [ "$i" -ge "$tries" ] && { echo "git fetch $repo@$commit: failed after $tries tries" >&2; return 1; }
      i=$((i + 1))
      sleep 10
    done
    git -C "$src" -c advice.detachedHead=false checkout -q FETCH_HEAD
  fi
  if [ -n "$fixture" ]; then
    cp "$bench_real_root/fixtures/$fixture/fixture.json" "$src/composer.json"
    cp "$bench_real_root/fixtures/$fixture/fixture.lock" "$src/composer.lock"
  fi
  [ -n "$repo$fixture" ] || { echo "$src: neither a repository nor a fixture" >&2; return 1; }
  [ -f "$src/composer.lock" ] || { echo "$src: no composer.lock" >&2; return 1; }
}

# phpm's `decision plugins:` lines that sent a step to Composer: the
# plugin names involved, one per line.
fallback_plugin_names() {
  grep -oE 'decision plugins: .+ (not native|allowed)' "$1" 2>/dev/null \
    | sed -E 's/decision plugins: (.+) (not native|allowed)/\1/' \
    | tr ',' '\n' | sed -E 's/^ +| +$//g' | grep -v '^$' | sort -u || true
}

scenario_timing() {
  local dir="$1" cmd="$2" work="$3" name="$4"
  local clear_caches="rm -rf $work/cache"
  (
    cd "$dir" || exit 1
    HOME="$work/home" hyperfine -N --shell bash --runs 3 --warmup 0 \
      --prepare "bash -c 'rm -rf $dir/vendor; $clear_caches'" \
      -n "cold" "$cmd" --export-json "$work/$name-cold.json" >/dev/null
    HOME="$work/home" hyperfine -N --shell bash --runs 5 --warmup 1 \
      --prepare "rm -rf $dir/vendor" \
      -n "warm" "$cmd" --export-json "$work/$name-warm.json" >/dev/null
    HOME="$work/home" hyperfine -N --shell bash --runs 10 --warmup 1 \
      -n "noop" "$cmd" --export-json "$work/$name-noop.json" >/dev/null
  )
}

mean_of() {
  [ -f "$1" ] && jq '.results[0].mean' "$1" || echo null
}

run_project() {
  local repo="$1" commit="$2" fixture="$3" os="$4" group="$5" stars="$6"
  local work
  work=$(mktemp -d)
  trap 'rm -rf "$work"' RETURN

  local name="${repo:-fixtures/$fixture}"
  if ! prepare_source "$work/src" "$repo" "$commit" "$fixture" 2>"$work/fetch.err"; then
    jq -nc --arg group "$group" --arg repo "${repo:-null}" --arg fixture "${fixture:-null}" \
      --arg os "$os" --arg error "$(cat "$work/fetch.err")" \
      '{group: $group, repo: (if $repo == "null" then null else $repo end),
        fixture: (if $fixture == "null" then null else $fixture end),
        os: $os, identity: "install-failed", error: $error}'
    return
  fi
  local packages
  packages=$(jq '((.packages // []) | length) + ((."packages-dev" // []) | length)' "$work/src/composer.lock")

  cp -R "$work/src" "$work/composer"
  cp -R "$work/src" "$work/phpm"
  mkdir -p "$work/home"
  export COMPOSER_CACHE_DIR="$work/cache/composer" PHPM_CACHE_DIR="$work/cache/phpm" XDG_CACHE_HOME="$work/cache/xdg"

  echo "== $name ($os)" >&2
  if ! (cd "$work/composer" && HOME="$work/home" "${COMPOSER_BIN:-composer}" install --no-scripts --no-plugins --ignore-platform-reqs --no-interaction --no-progress -q) \
    >"$work/composer-install.log" 2>&1; then
    jq -nc --arg group "$group" --arg repo "${repo:-null}" --arg os "$os" --arg error "composer install failed, see log" \
      '{group: $group, repo: (if $repo == "null" then null else $repo end), os: $os, identity: "install-failed", error: $error}'
    return
  fi
  scenario_timing "$work/composer" "${COMPOSER_BIN:-composer} install --no-scripts --no-plugins --ignore-platform-reqs --no-interaction --no-progress -q" "$work" composer

  if ! (cd "$work/phpm" && HOME="$work/home" "${PHPM_BIN:-phpm}" install --no-scripts --no-plugins --ignore-platform-reqs --explain) \
    >"$work/phpm-install.log" 2>"$work/phpm-explain.log"; then
    cp "$work/phpm-explain.log" "$work/phpm.err"
  fi
  scenario_timing "$work/phpm" "${PHPM_BIN:-phpm} install --no-scripts --no-plugins --ignore-platform-reqs" "$work" phpm

  local identity=identical differences=0 diff_out
  if diff_out=$("${DIFFVENDOR_BIN:-diffvendor}" "$work/composer/vendor" "$work/phpm/vendor" 2>&1); then
    identity=identical
  else
    identity=different
    differences=$(echo "$diff_out" | tail -1 | grep -oE '^[0-9]+' || echo 1)
  fi

  local fallback=false fallback_plugins="[]"
  if grep -q 'fallback to' "$work/phpm-explain.log" 2>/dev/null; then
    fallback=true
    fallback_plugins=$(fallback_plugin_names "$work/phpm-explain.log" | jq -R -s -c 'split("\n") | map(select(length > 0))')
  fi

  jq -nc \
    --arg group "$group" --arg repo "${repo:-null}" --arg fixture "${fixture:-null}" \
    --argjson stars "${stars:-null}" --arg os "$os" --argjson packages "${packages:-null}" \
    --argjson cold "{\"composer_seconds\": $(mean_of "$work/composer-cold.json"), \"phpm_seconds\": $(mean_of "$work/phpm-cold.json")}" \
    --argjson warm "{\"composer_seconds\": $(mean_of "$work/composer-warm.json"), \"phpm_seconds\": $(mean_of "$work/phpm-warm.json")}" \
    --argjson noop "{\"composer_seconds\": $(mean_of "$work/composer-noop.json"), \"phpm_seconds\": $(mean_of "$work/phpm-noop.json")}" \
    --arg identity "$identity" --argjson differences "$differences" \
    --argjson fallback "$fallback" --argjson fallback_plugins "$fallback_plugins" \
    '{group: $group, repo: (if $repo == "null" then null else $repo end),
      fixture: (if $fixture == "null" then null else $fixture end),
      stars: $stars, os: $os, packages: $packages,
      cold: $cold, warm: $warm, noop: $noop,
      identity: $identity, differences: $differences,
      fallback: $fallback, fallback_plugins: $fallback_plugins}'
}
