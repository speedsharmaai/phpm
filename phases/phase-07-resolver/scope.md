# Phase 07: Resolver

Outline. Starts only if Phase 06's gate passes.

## Why

Once people use phpm for installs, the next thing they ask for is `update`
and `require`. uv followed the same path: `uv pip install` first, the
project manager later. A resolver is also where Composer is slowest on big
graphs (Magento, Drupal).

It is last because it is the riskiest. A resolver that picks a different
version from Composer silently changes what runs in production (decision 0002).

## What ships

- p2 metadata client: minified expansion, If-Modified-Since, a parsed binary
  cache, Packagist limits.
- A `pubgrub` resolver encoding Composer semantics: constraints, stability,
  `replace`/`provide`/`conflict`, aliases, platform packages, and
  `DefaultPolicy` tie-breaks (highest, prefer-stable, repository priority,
  replacer vs original).
- Lockfile writer, byte-identical to Composer's (key order, sorting,
  `plugin-api-version`).
- `phpm update`, `phpm require`, `phpm remove`.
- Conflict messages at least as clear as Composer's, which is where pubgrub
  should win.

## Gate

The sweep resolves every corpus project and produces a lockfile identical to
Composer's on at least 99% of them. Anything less stays behind a flag.

## Out of scope

A new lockfile format. Workspaces. Anything Composer does not do.
