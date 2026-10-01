#!/usr/bin/env bash
# Resolves inputs.version ("latest" or a pinned "x.y.z") to a concrete
# version string. Needs PHPM_VERSION_INPUT and GH_TOKEN.
set -euo pipefail

if [ "$PHPM_VERSION_INPUT" = "latest" ]; then
  version=$(gh api repos/speedsharmaai/phpm/releases/latest --jq .tag_name | sed 's/^v//')
else
  version="$PHPM_VERSION_INPUT"
fi

echo "version=$version" >>"$GITHUB_OUTPUT"
