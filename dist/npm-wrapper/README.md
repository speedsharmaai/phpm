# npm wrapper (scaffold, not published)

This is a snapshot of the npm package `dist build --artifacts=global`
produces from `dist-workspace.toml` and `crates/phpm/Cargo.toml`. It is
checked in here for review before launch; it is not published to npm and
nothing in this directory runs in CI except the smoke test below.

At a real release, cargo-dist's `v-release.yml` builds the current version
of this package itself (`build-global-artifacts` job, `phpm-npm-package.tar.gz`)
from the same source, so this snapshot will drift from what actually ships —
that is expected. Don't hand-edit `package.json`'s `supportedPlatforms` or
`artifactDownloadUrls`; they come from the workspace config.

## What it is

- `package.json` — name `phpm`, `bin.phpm` runs `run-phpm.js`, `postinstall`
  runs `install.js`. `supportedPlatforms` maps every Rust target triple
  cargo-dist knows about to a release asset name; this workspace only builds
  5 of them (see `dist-workspace.toml`), the rest resolve to a "platform not
  supported" error at install time until the target list changes.
- `binary.js` — picks the right entry from `supportedPlatforms` for the
  current OS/arch/libc (including musl vs glibc detection and a glibc
  version floor).
- `binary-install.js` — downloads and extracts the matching GitHub Release
  asset into `node_modules/.bin_real`, with HTTP(S) proxy support and
  redirect following.
- `install.js` / `run-phpm.js` — the `postinstall` and `bin` entry points.
- `npm-shrinkwrap.json` — pins the one runtime dependency (`detect-libc`).

This is the same pattern uv and ruff's npm wrappers use (a postinstall
download shim keyed by `os.type()`/`os.arch()`), generated for phpm from
phpm's own release metadata rather than copied from either.

## Name

`phpm` is unclaimed on npm as of this check (see the phase 06 report).
`@speedsharmaai/phpm` is also free, kept as the fallback if `phpm` is taken
before launch.

## Smoke test

```sh
dist/npm-wrapper/test.sh
```

Checks every `.js` file parses (`node --check`) and that `package.json` has
the shape `binary.js` expects: a `bin.phpm` entry, a `supportedPlatforms`
entry for each of this workspace's 5 build targets, and matching `bins`/
`zipExt` fields for each of those.
