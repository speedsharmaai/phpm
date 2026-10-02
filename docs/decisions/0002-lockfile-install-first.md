# 0002: `install` from a lockfile first; no resolver until Phase 07

Status: accepted
Date: 2026-10-01

## Context

`composer install` with a `composer.lock` present needs no dependency
resolution and almost no registry metadata. Every locked package already
carries its `dist.url`, `dist.type` and `dist.reference`. The work is:
download, extract, place, write metadata, generate the autoloader.

That is also where the speed difference is. The measured warm install spends
about 2.6 s of 4.1 s re-extracting zips Composer already has in its cache.
Resolution does not happen at all in that path.

`update` and `require` are different. They need a resolver whose choices match
Composer's `DefaultPolicy` tie-breaks exactly (highest version, prefer-stable,
repository priority, replacers), plus byte-exact lockfile writing. A resolver
that picks a different version than Composer changes what runs in production,
silently.

## Decision

Phases 01-06 implement `install` only. phpm has no `update`, `require` or
`remove` command; run Composer yourself for those. Passing them straight
through to Composer, with one line saying so, is planned and not built.

A resolver is Phase 07 at the earliest, built on `pubgrub`, and does not ship
until it reproduces Composer's lockfile byte for byte across the compatibility
sweep.

## Consequences

- phpm is useful from day one on every CI run and every fresh clone, which is
  where installs actually happen.
- The launch story is narrower ("installs") than uv's was. That is fine; uv
  also launched as `uv pip install` before it was a project manager.
- Developers still need Composer installed for updates. phpm is a companion,
  not a replacement, until Phase 07 says otherwise.

## Notes

- 2026-10-02: corrected the decision text. It used to say `update`,
  `require` and `remove` were already passed through to Composer. They are
  not implemented at all; the pass-through is future work.
