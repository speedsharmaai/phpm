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

# Times one iteration of $cmd via hyperfine, no shell involved (`-N`):
# hyperfine's own shell-spawning calibration (used whenever a `--shell`
# other than none is given) has proven unreliable on windows-latest across
# four different shell/path combinations, so cleanup between iterations
# happens as plain bash here, never inside hyperfine's own measurement.
single_run() {
  local home="$1" cmd="$2" out="$3"
  HOME="$home" hyperfine -N --runs 1 --warmup 0 "$cmd" --export-json "$out" >/dev/null
}

# `runs` recorded iterations of $cmd (plus `warmup` unrecorded ones first),
# with $cleanup run as plain bash before every iteration; combined into one
# hyperfine-shaped JSON (mean_of reads `.results[0].mean`) at $out.
manual_runs() {
  local home="$1" cmd="$2" out="$3" runs="$4" warmup="$5" cleanup="$6"
  local tmp times="[]" i t
  for ((i = 1; i <= warmup; i++)); do
    eval "$cleanup"
    tmp=$(mktemp)
    single_run "$home" "$cmd" "$tmp"
    rm -f "$tmp"
  done
  for ((i = 1; i <= runs; i++)); do
    eval "$cleanup"
    tmp=$(mktemp)
    single_run "$home" "$cmd" "$tmp"
    t=$(jq '.results[0].times[0]' "$tmp")
    times=$(jq -c --argjson t "$t" '. + [$t]' <<<"$times")
    rm -f "$tmp"
  done
  jq -n --argjson times "$times" \
    '{results: [{times: $times, mean: (if ($times | length) > 0 then ($times | add / length) else 0 end)}]}' \
    >"$out"
}

# The project's vendor directory, honouring composer.json's
# config.vendor-dir (nextcloud, joomla, opencart and others move it).
vendor_dir() {
  local v
  v=$(jq -r '.config["vendor-dir"] // "vendor"' "$1/composer.json" 2>/dev/null) || v=vendor
  [ -n "$v" ] || v=vendor
  echo "$1/${v%/}"
}

scenario_timing() {
  local dir="$1" cmd="$2" work="$3" name="$4"
  local home="$work/home" vendor
  vendor=$(vendor_dir "$dir")
  (
    cd "$dir" || exit 1
    manual_runs "$home" "$cmd" "$work/$name-cold.json" 3 0 \
      "rm -rf '$vendor' '$work/cache'"
    manual_runs "$home" "$cmd" "$work/$name-warm.json" 5 1 \
      "rm -rf '$vendor'"
    manual_runs "$home" "$cmd" "$work/$name-noop.json" 10 1 ":"
  )
}

mean_of() {
  local v=""
  if [ -f "$1" ]; then
    v=$(jq '.results[0].mean' "$1" 2>/dev/null) || v=""
  fi
  # jq always prints a value for well-formed JSON (including the text
  # "null"); empty output means the read itself failed outright (missing
  # or malformed file), which is folded into the same "null" the record
  # already uses for "this scenario has no number".
  case "$v" in
    '' | *$'\n'*) echo null ;;
    *) echo "$v" ;;
  esac
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

  # -L dereferences symlinks in the source tree: creating a real symlink
  # needs elevated privileges on windows-latest, and the install only ever
  # reads composer.json/composer.lock through these copies anyway.
  # A dangling symlink can't be dereferenced (PrestaShop ships one); keep
  # it as a link then, which works everywhere but windows-latest.
  copy_tree() { cp -RL "$1" "$2" 2>>"$work/copy.err" || { rm -rf "$2" && cp -R "$1" "$2" 2>>"$work/copy.err"; }; }
  if ! copy_tree "$work/src" "$work/composer" || ! copy_tree "$work/src" "$work/phpm"; then
    jq -nc --arg group "$group" --arg repo "${repo:-null}" --arg fixture "${fixture:-null}" \
      --arg os "$os" --arg error "copying the source tree failed: $(tail -3 "$work/copy.err")" \
      '{group: $group, repo: (if $repo == "null" then null else $repo end),
        fixture: (if $fixture == "null" then null else $fixture end),
        os: $os, identity: "install-failed", error: $error}'
    return
  fi
  mkdir -p "$work/home"
  export COMPOSER_CACHE_DIR="$work/cache/composer" PHPM_CACHE_DIR="$work/cache/phpm" XDG_CACHE_HOME="$work/cache/xdg"

  echo "== $name ($os)" >&2
  if ! (cd "$work/composer" && HOME="$work/home" "${COMPOSER_BIN:-composer}" install --no-scripts --no-plugins --ignore-platform-reqs --no-interaction --no-progress -q) \
    >"$work/composer-install.log" 2>&1; then
    jq -nc --arg group "$group" --arg repo "${repo:-null}" --arg os "$os" --arg error "composer install failed, see log" \
      '{group: $group, repo: (if $repo == "null" then null else $repo end), os: $os, identity: "install-failed", error: $error}'
    return
  fi
  # A timed run failing mid-scenario (flaky network, a transient registry
  # error) degrades that one project to whatever scenarios did complete,
  # rather than losing every other project left in the batch.
  scenario_timing "$work/composer" "${COMPOSER_BIN:-composer} install --no-scripts --no-plugins --ignore-platform-reqs --no-interaction --no-progress -q" "$work" composer \
    || echo "$name: composer timing failed partway, see $work logs" >&2

  if ! (cd "$work/phpm" && HOME="$work/home" "${PHPM_BIN:-phpm}" install --no-scripts --no-plugins --ignore-platform-reqs --explain) \
    >"$work/phpm-install.log" 2>"$work/phpm-explain.log"; then
    cp "$work/phpm-explain.log" "$work/phpm.err"
  fi
  scenario_timing "$work/phpm" "${PHPM_BIN:-phpm} install --no-scripts --no-plugins --ignore-platform-reqs" "$work" phpm \
    || echo "$name: phpm timing failed partway, see $work logs" >&2

  # diffvendor: 0 identical, 1 differences (last line "N differences"),
  # anything else means the comparison itself failed and is not a result.
  local identity=identical differences=0 diff_out diff_status=0
  diff_out=$("${DIFFVENDOR_BIN:-diffvendor}" "$(vendor_dir "$work/composer")" "$(vendor_dir "$work/phpm")" 2>&1) || diff_status=$?
  case "$diff_status" in
    0) identity=identical ;;
    1)
      identity=different
      differences=$(echo "$diff_out" | tail -1 | grep -oE '^[0-9]+' || echo 0)
      ;;
    *)
      jq -nc --arg group "$group" --arg repo "${repo:-null}" --arg fixture "${fixture:-null}" \
        --arg os "$os" --arg error "diffvendor failed: $(echo "$diff_out" | tail -3)" \
        '{group: $group, repo: (if $repo == "null" then null else $repo end),
          fixture: (if $fixture == "null" then null else $fixture end),
          os: $os, identity: "install-failed", error: $error}'
      return
      ;;
  esac

  local fallback=false fallback_plugins="[]"
  if grep -q 'fallback to' "$work/phpm-explain.log" 2>/dev/null; then
    fallback=true
    fallback_plugins=$(fallback_plugin_names "$work/phpm-explain.log" | jq -R -s -c 'split("\n") | map(select(length > 0))')
  fi

  # Each mean is its own --argjson rather than hand-built into a JSON
  # string: jq validates each one independently, so one scenario that
  # never finished can't corrupt the whole record (seen in practice: a
  # hand-built "{...}" string let one bad value turn into invalid JSON
  # that failed the entire --argjson parse).
  jq -nc \
    --arg group "$group" --arg repo "${repo:-null}" --arg fixture "${fixture:-null}" \
    --argjson stars "${stars:-null}" --arg os "$os" --argjson packages "${packages:-null}" \
    --argjson composer_cold "$(mean_of "$work/composer-cold.json")" \
    --argjson phpm_cold "$(mean_of "$work/phpm-cold.json")" \
    --argjson composer_warm "$(mean_of "$work/composer-warm.json")" \
    --argjson phpm_warm "$(mean_of "$work/phpm-warm.json")" \
    --argjson composer_noop "$(mean_of "$work/composer-noop.json")" \
    --argjson phpm_noop "$(mean_of "$work/phpm-noop.json")" \
    --arg identity "$identity" --argjson differences "$differences" \
    --argjson fallback "$fallback" --argjson fallback_plugins "$fallback_plugins" \
    '{group: $group, repo: (if $repo == "null" then null else $repo end),
      fixture: (if $fixture == "null" then null else $fixture end),
      stars: $stars, os: $os, packages: $packages,
      cold: {composer_seconds: $composer_cold, phpm_seconds: $phpm_cold},
      warm: {composer_seconds: $composer_warm, phpm_seconds: $phpm_warm},
      noop: {composer_seconds: $composer_noop, phpm_seconds: $phpm_noop},
      identity: $identity, differences: $differences,
      fallback: $fallback, fallback_plugins: $fallback_plugins}'
}
