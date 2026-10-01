#!/usr/bin/env bash
# Maps the runner's OS/arch to one of this workspace's 5 build targets
# (dist-workspace.toml) and the archive extension dist packages it as.
set -euo pipefail

os=$(uname -s)
arch=$(uname -m)

case "$os" in
  Darwin)
    case "$arch" in
      arm64) triple=aarch64-apple-darwin ;;
      x86_64) triple=x86_64-apple-darwin ;;
      *)
        echo "phpm has no release binary for Darwin/$arch" >&2
        exit 1
        ;;
    esac
    ext=tar.xz
    ;;
  Linux)
    case "$arch" in
      aarch64) triple=aarch64-unknown-linux-musl ;;
      x86_64) triple=x86_64-unknown-linux-musl ;;
      *)
        echo "phpm has no release binary for Linux/$arch" >&2
        exit 1
        ;;
    esac
    ext=tar.xz
    ;;
  MINGW* | MSYS* | CYGWIN*)
    triple=x86_64-pc-windows-msvc
    ext=zip
    ;;
  *)
    echo "phpm has no release binary for $os/$arch" >&2
    exit 1
    ;;
esac

echo "triple=$triple" >>"$GITHUB_OUTPUT"
echo "ext=$ext" >>"$GITHUB_OUTPUT"
