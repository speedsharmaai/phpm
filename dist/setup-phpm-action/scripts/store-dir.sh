#!/usr/bin/env bash
# Pins PHPM_CACHE_DIR to a known path under the runner's temp directory, so
# the store's location doesn't depend on each OS's own default (see
# crates/phpm-store/src/store.rs's cache_dir_from) and can be cached by one
# fixed path regardless of runner OS.
set -euo pipefail

dir="$RUNNER_TEMP/phpm-store"
echo "path=$dir" >>"$GITHUB_OUTPUT"
echo "PHPM_CACHE_DIR=$dir" >>"$GITHUB_ENV"
