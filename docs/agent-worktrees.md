# Agent worktrees: one store, near-zero disk

The README's pitch: "Agents run installs in CI, in containers, and in
parallel git worktrees, each wanting its own `vendor/`. With a shared
store, a new worktree's `vendor/` costs a fraction of a second and almost
no disk." This is that, made concrete with real commands.

## The setup

One global store, several worktrees of the same repository, each one a
separate agent's working copy:

```sh
git clone https://github.com/you/your-app.git
cd your-app

export PHPM_CACHE_DIR="$HOME/.cache/phpm" # the default on Linux; see below

git worktree add ../your-app-agent-1 -b agent-1
git worktree add ../your-app-agent-2 -b agent-2
git worktree add ../your-app-agent-3 -b agent-3
```

Each worktree gets its own `composer.lock` state (whatever that branch
has) but shares one `PHPM_CACHE_DIR`. Install in each:

```sh
(cd ../your-app-agent-1 && phpm install)
(cd ../your-app-agent-2 && phpm install)
(cd ../your-app-agent-3 && phpm install)
```

The first install that touches a given `(package, reference)` pair fetches
and extracts it into the store once. Every later install of that same pair
— in any worktree, any branch, any clone on the same machine — clones the
already-extracted directory into that worktree's `vendor/`
(`libc::clonefile` on macOS/APFS, `FICLONE`/hardlink on Linux, hardlink on
Windows, plain copy only as a last resort — see
[topology](arch/topology.md)). No PHP, no zip, no re-extraction.

## Where the store lives

`PHPM_CACHE_DIR` always wins if set. Otherwise (from
`crates/phpm-store/src/store.rs`):

| Platform | Default |
|---|---|
| macOS | `$HOME/Library/Caches/phpm` |
| Linux | `$XDG_CACHE_HOME/phpm`, else `$HOME/.cache/phpm` |
| Windows | `%LOCALAPPDATA%\phpm` |

Agent worktrees on the same machine share this by default — nothing to
configure beyond making sure every agent's environment resolves to the
same `$HOME` (or pinning `PHPM_CACHE_DIR` explicitly, which is what
`dist/setup-phpm-action/` does in CI, since a CI runner's `$HOME` and its
cacheable path aren't always the same thing).

## Measuring it yourself

`tools/bench-real/worktrees` (Phase 05's benchmark tooling) runs exactly
this scenario — N git worktrees of one fixture, `composer install` x N
against N full `vendor/` copies vs `phpm install` x N from a warm store —
and reports total wall time and total disk (by free-space delta, so a
clone that shares blocks with the store isn't double-counted):

```sh
cargo build --release -p phpm
PHPM_BIN=target/release/phpm tools/bench-real/worktrees laravel-skeleton --os <name> --count 10
```

This repo's own published disk/time numbers for this scenario come from
Phase 05's benchmark run (`phases/phase-05-real-world-benchmarks/`), on
infrastructure sized for it — a quick local run's absolute numbers depend
heavily on free disk space and filesystem (APFS clonefile sharing behaves
differently near a full disk than ext4 hardlinks do), so this doc points
at the tool and the methodology rather than restating a number that would
only be true on one machine on one day.
