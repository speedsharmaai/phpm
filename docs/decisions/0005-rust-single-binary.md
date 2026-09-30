# 0005: Rust, one static binary

Status: accepted
Date: 2026-10-01

## Context

Composer's fixed costs come from being a PHP program: interpreter start,
bootstrap, and single-threaded class scanning through `php_strip_whitespace`.
The no-op install is 1.6 s of mostly that. A native binary removes the fixed
cost and can scan files on every core.

The owner has shipped Rust before (whyset). The PHP tooling that did get
adopted from outside PHP is Rust or Go: Mago (Rust, 3.5k stars, in
setup-php), FrankenPHP (Go, under the PHP organisation). uv and Ruff set the
expectation for what a Rust package tool feels like.

Go would also work. Rust wins on the crates this needs: `pubgrub` (uv's
resolver), `mago-syntax` (a maintained PHP lexer), `reflink-copy`, and
cargo-dist for distribution.

## Decision

Rust, a single static binary per target: x86_64 and aarch64 for Linux (musl),
macOS and Windows. PHP is called only to detect the platform (once, cached)
and to run scripts and fallbacks.

## Consequences

- Installable without PHP for the fast path, which helps slim Docker build
  stages.
- Licences of every crate are checked on entry. The `composer-*` crates from
  cresset-tools are EUPL-1.2 and need a compatibility check before reuse.
- One more language in the speedsharma portfolio to maintain.
