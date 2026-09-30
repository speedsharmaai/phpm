# 0008: Quality gates before the first line of feature code

Status: accepted
Date: 2026-10-01

## Context

Tooling added after code exists arrives with a backlog of warnings, and the
backlog gets waived instead of fixed. phpm's output must match Composer's
byte for byte, so some of its quality rules protect correctness directly:
fixtures that a whitespace hook "fixes" are silently wrong, and HashMap
iteration that reaches output makes it non-deterministic.

The owner asked for the JavaScript-world set by name: SonarQube, ESLint,
conventional commits, Husky. The Rust equivalents were researched
([quality tooling](../research/quality-tooling.md)).

## Decision

Phase 00 sets up, on an empty workspace:

| Asked for | Rust equivalent | Gate |
|---|---|---|
| ESLint | clippy via `[workspace.lints]` (pedantic) + rustfmt | hard, `-D warnings` |
| Husky | prek (pre-commit, commit-msg, pre-push hooks) | local; CI reruns everything |
| Conventional commits / commitlint | committed + PR title check + squash merge | hard |
| SonarQube | SonarQube Cloud, free for public repos, clippy JSON + LCOV | hard, quality gate |
| (implied) | cargo-deny, cargo-shear, cargo-hack MSRV, typos, zizmor | hard |
| (implied) | nextest, cargo-llvm-cov floor 80% | hard |
| (implied) | release-plz + git-cliff, cargo-dist | configured, used from Phase 06 |

Sonar repeats many of clippy's findings. It stays because its complexity,
duplication and new-code coverage gates catch what clippy does not, and on a
public repo it costs nothing. If its gate ever blocks a change that clippy
and the tests are happy with, the gate's rule is reviewed, not skipped.

Every tool is pinned. Every action is pinned by SHA.

## Consequences

- Phase 01's spike is written under the same rules as everything after it.
  No "clean up the spike" phase.
- The first week produces no phpm feature. That is the cost.
- This phase is the template for every tool in `devtools/`.
- The repo is public from the first commit (decision 0009), so CodeQL,
  rulesets, Scorecard, attestations and private vulnerability reporting are
  all switched on in this phase rather than deferred.
