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

composer_phar_url() {
  echo "https://getcomposer.org/download/$1/composer.phar"
}

sha256_matches() {
  local file="$1" want="$2" got
  got=$( (sha256sum "$file" 2>/dev/null || shasum -a 256 "$file") | cut -d' ' -f1)
  [ "$got" = "$want" ]
}
