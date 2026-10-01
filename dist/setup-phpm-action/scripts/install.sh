#!/usr/bin/env bash
# Downloads the release archive for PHPM_TRIPLE/PHPM_EXT at PHPM_VERSION,
# verifies it against the release's sha256.sum, extracts it, and adds the
# binary's directory to PATH.
set -euo pipefail

install_dir="$RUNNER_TEMP/phpm-bin"
mkdir -p "$install_dir"
archive="phpm-${PHPM_TRIPLE}.${PHPM_EXT}"
base_url="https://github.com/speedsharmaai/phpm/releases/download/v${PHPM_VERSION}"

curl --proto '=https' --tlsv1.2 -fsSL "${base_url}/${archive}" -o "$install_dir/$archive"
curl --proto '=https' --tlsv1.2 -fsSL "${base_url}/sha256.sum" -o "$install_dir/sha256.sum"

(
  cd "$install_dir"
  line=$(grep -F "$archive" sha256.sum)
  if command -v sha256sum >/dev/null 2>&1; then
    echo "$line" | sha256sum -c -
  else
    echo "$line" | shasum -a 256 -c -
  fi
)

if [ "$PHPM_EXT" = "zip" ]; then
  # Windows' bundled tar (bsdtar) reads .zip directly; its one release
  # asset is flat, no leading directory to strip.
  tar xf "$install_dir/$archive" -C "$install_dir"
  bin_path="$install_dir/phpm.exe"
else
  tar xf "$install_dir/$archive" --strip-components 1 -C "$install_dir"
  bin_path="$install_dir/phpm"
fi
chmod +x "$bin_path"

echo "$install_dir" >>"$GITHUB_PATH"
echo "path=$bin_path" >>"$GITHUB_OUTPUT"
