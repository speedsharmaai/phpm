# setup-phpm (scaffold, not published)

What `speedsharmaai/setup-phpm` should contain: a composite GitHub Action
that installs phpm for the runner's platform and caches the content-addressed
store between jobs, the way `shivammathur/setup-php` installs PHP and
`Swatinem/rust-cache` caches Cargo's.

## Usage (once published)

```yaml
- uses: speedsharmaai/setup-phpm@v1
  with:
    version: "0.1.0" # or omit for latest
- run: phpm install
```

## Layout

```text
action.yml            composite action: 5 steps, each a script below
scripts/
  resolve-version.sh   "latest" -> a concrete version via the GitHub API
  detect-target.sh     uname -s/-m -> one of this workspace's 5 build targets
  store-dir.sh          pins PHPM_CACHE_DIR to $RUNNER_TEMP/phpm-store
  install.sh            downloads, sha256-verifies, extracts, adds to PATH
```

Shell logic lives in `scripts/*.sh` rather than inline `run:` blocks so it
can be shellchecked and unit-tested directly — `actionlint` doesn't lint
standalone `action.yml` files (only `.github/workflows/*.yml`), and a
`${{ }}`-laden inline block can't be shellchecked without first stripping
the expressions. `action.yml` itself only sets `env:` and calls the script
at `${{ github.action_path }}/scripts/<name>.sh`.

## Caching

The store cache key is `phpm-store-<os>-<version>[-<suffix>]`, restoring
from the most specific match down to just `phpm-store-<os>-` if the exact
version isn't cached yet — the store is content-addressed and additive, so
a stale-version restore still saves most of the work. `PHPM_CACHE_DIR` is
pinned to `$RUNNER_TEMP/phpm-store` rather than left at phpm's own
per-OS default (`crates/phpm-store/src/store.rs`'s `cache_dir_from`), so one
cache path works the same way on every runner OS.

## What's not done here, and why

Not published to the Marketplace, and `speedsharmaai/setup-phpm` does not
exist as a repository yet — creating it is a new public-facing artifact,
left for the owner's go-ahead (see the phase 06 report).

## Smoke test

```sh
dist/setup-phpm-action/test.sh
```

Shellchecks every script, then exercises `detect-target.sh`'s OS/arch
mapping directly (stubbing `uname`) against all 5 build targets plus one
unsupported case. `resolve-version.sh` and `install.sh` touch the network
and GitHub's API and aren't exercised here.
