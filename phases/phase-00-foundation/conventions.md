# Conventions

The rules the Phase 00 tooling enforces, written down once.

## Commits

One line. Conventional commit. Lowercase, imperative, no full stop, 72
characters at most. No body unless a revert needs the reason. No co-author
lines, no generated-by footers.

```
feat: clone package dirs from the store
fix: keep exec bits when extracting zips
perf: cache class scans per file
docs: add phase 00 foundation
test: golden files for installed.php
build: pin toolchain to 1.98.1
ci: run clippy on windows
chore: bump dependencies
refactor: split fetch from extract
```

Allowed types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`,
`ci`, `chore`, `revert`, `style`. A scope is optional and is a crate name
without the `phpm-` prefix: `fix(autoload): ...`.

Breaking changes use `!`: `feat!: drop --link-mode=symlink`.

Enforced by `committed` in the commit-msg hook and in CI.

## Branches and PRs

- `main` is always releasable. No direct pushes once the repo is public.
- Branch names: `<type>/<short-thing>`, e.g. `feat/store-clone`.
- One PR, one change. The PR title follows the commit rules above, because
  squash merge makes it the commit on `main`.
- A PR is mergeable when `ci-ok` and the title check are green.

## Code

- No `unsafe`. The one exception is the `clonefile` FFI call, isolated in
  one module with `#[allow(unsafe_code, reason = "...")]` and a test.
- No `unwrap()` outside tests. `expect()` only with a reason that would help
  a user reading the panic.
- No `println!`. All output goes through one writer so `--quiet`, `--json`
  and colour handling live in one place.
- No iteration over `HashMap`/`HashSet` where order can reach output. Use
  `BTreeMap`, `IndexMap`, or sort first. Clippy's `iter_over_hash_type` is on.
- Comments only where the code cannot say it. Composer behaviour being
  matched gets a one-line pointer to the Composer source file, nothing more.

## Byte-exact files

- Fixtures live in `fixtures/`, snapshots in `**/snapshots/`. Hooks never
  touch them: excluded from whitespace and end-of-file fixers.
- `.gitattributes` marks them `-text` so checkouts never change line endings.
- Anything that must match Composer is compared as bytes (`assert_eq!` on
  `&[u8]` or a golden-file helper), not through insta, which may normalise
  whitespace. insta is for human-readable output.

## Versions

- Toolchain pinned in `rust-toolchain.toml`. MSRV (`rust-version`) is two
  releases behind stable and moves no faster than that.
- Every tool in CI is pinned to an exact version. Every action is pinned to a
  full SHA with a `# vX.Y.Z` comment.
- Dependabot proposes updates weekly after a seven-day cooldown, 30 days for
  major versions.
