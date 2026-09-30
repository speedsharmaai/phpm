# 0007: Eight clones exist; contribute if phpm is behind at the gate

Status: accepted
Date: 2026-10-01

## Context

The research that picked this project said nobody owned "uv for PHP". That
was wrong. As of 2026-10-01:

| Project | Lang | Stars | Created | Notes |
|---|---|---|---|---|
| paramientos/presto | Go | 237 | 2025 | claims 10-20x; ~55-144 binary downloads per release |
| libretto-pm/libretto | Rust | 24 | 2026-01 | no plugins; quiet since February |
| HichemTab-tech/pomposer | Rust | 24 | 2025-07 | global store; quiet since November |
| Adelagric/vivacity | Rust | 21 | 2026-09 | active; splits into reusable crates |
| zanderlewis/composer-rs (ex-lectern) | Rust | 9 | 2025-09 | |
| cresset-tools/bougie | Rust | 8 | 2026-05 | EUPL crates incl. byte-exact json_encode |
| shyim/riff | Rust | 3 | 2026-08 | Shopware core dev; flex and patches adapters; hyperfine data: 5.7x warm on symfony/demo |
| svandragt/vivace | Rust | 3 | 2026 | byte-identical sweep, GitHub Action; refuses flex |
| TheLifus/concerto, lschvn/tusk, zengbo/composer-rs, rabbiveesh/composr | Rust | 0-2 | 2026 | experiments |

Several say openly they were built by directing coding agents. The code is
no longer the moat; anyone can produce a Rust Composer in a month. What
nobody has is adoption: the most-starred one has almost no downloads.

## Decision

phpm competes on three things only: never breaking a project (0004), a
published compatibility number across hundreds of lockfiles, and
distribution into the places installs happen (setup-php, Docker, Laravel).

At the Phase 00 gate, phpm is benchmarked head to head against riff and
vivace on the same fixtures. If one of them is already at or ahead of phpm
on both speed and byte-identity, phpm stops and the effort goes into
contributing to that project instead: the compatibility sweep, adapters, or
distribution work. A second-best clone in a field of twelve is not worth
maintaining.

## Consequences

- The owner may end this project with a merged contribution and no tool of
  their own. That is an acceptable outcome and is written down now so it does
  not feel like a failure later.
- The field is re-checked at every phase boundary, not just once.
