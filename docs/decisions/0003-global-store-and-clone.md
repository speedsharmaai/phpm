# 0003: A global store of extracted packages, cloned into `vendor/`

Status: accepted
Date: 2026-10-01

## Context

Composer caches zips, not extracted trees. Every install unzips every package
again. On the Laravel skeleton that is 109 archives and 8,860 files, about
2.6 s, on every warm install, in every project and every worktree.

Measured on APFS, same 109 packages already extracted:

| Method | Time |
|---|---|
| Composer warm extraction | ~2.6 s |
| `cp -R` (plain copy) | 4.8 s |
| `cp -Rc` (clonefile per file) | 2.3-3.3 s |
| `rsync --link-dest` (hardlinks) | 6.1 s |
| `clonefile()` per package directory, one syscall each | **0.14-0.24 s** |

The per-directory clone is the whole product. Per-file approaches lose most
of the gain to syscall count.

Symlinking `vendor/` entries into a store (pnpm's default, and what concerto
does) is faster still, but PHP tools that `realpath()` their own files, write
into `vendor/`, or expect real files break. Clearing the cache would also
break every project.

## Decision

- Store extracted packages once, under `~/.cache/phpm/pkgs/v1/<hash>/`, keyed by
  `(name, dist.reference)`. The reference is a commit SHA and is immutable;
  `dist.shasum` is empty for most GitHub-hosted packages and cannot be the key.
- Writes are append-only: extract to a temp directory, then atomic rename.
  Concurrent phpm processes are safe.
- Link into `vendor/` by, in order: `clonefile()` per package directory on
  macOS; FICLONE, then hardlink, on Linux; hardlink on Windows; copy last.
  `--link-mode=clone|hardlink|copy` overrides.
- Never symlink by default.
- Cache class-scan results in the store next to each package, keyed by file
  path. Files in the store never change, so a warm install does not rescan.

## Consequences

- Warm installs become a directory clone plus metadata writes.
- Worktrees and multiple projects share disk: cloned files take no extra
  space on APFS or reflink filesystems until they are modified.
- Hardlink mode has a real risk: someone patching a file in `vendor/` in place
  corrupts the store for every project. Clone is safe. Hardlink mode must say
  this in the docs and in `phpm doctor`.
- Linux ext4 has no reflink, so Linux gains come from hardlinks and are
  smaller than macOS. The benchmark page must state the filesystem.
