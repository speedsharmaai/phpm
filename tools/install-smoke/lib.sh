# shellcheck shell=bash
# Pure helpers for the install-smoke run script.

repo_url="https://github.com/speedsharmaai/phpm"

valid_tag() {
  [[ "$1" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]
}

asset_url() {
  echo "$repo_url/releases/download/$1/$2"
}

tag_version() {
  echo "${1#v}"
}

version_matches() {
  local tag="$1" output="$2"
  [ "$output" = "phpm $(tag_version "$tag")" ]
}
