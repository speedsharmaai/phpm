set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

fmt:
    cargo fmt --all
    tombi format

fmt-check:
    cargo fmt --all --check
    tombi format --check

lint:
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

test:
    cargo nextest run --workspace --locked
    cargo test --doc --workspace --locked

cov:
    cargo llvm-cov nextest --workspace --locked --lcov --output-path lcov.info --fail-under-lines 80

deny:
    cargo deny --locked check

shear:
    cargo shear

msrv:
    cargo hack check --rust-version --workspace --locked

typos:
    typos

md:
    markdownlint-cli2

actions:
    zizmor --offline .github/workflows
    actionlint

golden:
    cargo nextest run -p phpm-lock --locked --run-ignored only -E 'test(/^composer_live/)' --test-threads 1

golden-bless:
    PHPM_BLESS=1 cargo nextest run -p phpm-lock --locked --run-ignored only -E 'test(/^composer_live/)' --test-threads 1

ci: fmt-check lint docs test deny shear msrv typos md actions cov

# warm placement time for a fixture; the first run fills the store
place fixture="laravel-skeleton" *args="":
    cargo run --release --locked -p phpm-store --example place -- fixtures/{{fixture}} {{args}}
