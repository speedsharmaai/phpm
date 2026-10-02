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
    cargo nextest run -p phpm-lock -p phpm-autoload --locked --run-ignored only -E 'test(/^composer_live/)' --test-threads 1 --no-fail-fast

golden-bless:
    PHPM_BLESS=1 cargo nextest run -p phpm-lock -p phpm-autoload --locked --run-ignored only -E 'test(/^composer_live/)' --test-threads 1 --no-fail-fast

# composer vs phpm on every fixture, whole vendor/ compared
e2e:
    cargo nextest run -p phpm --locked --run-ignored only -E 'test(/^e2e_/) - test(/^e2e_app_/)' --test-threads 2

# each fixture in its real app with scripts and plugins: Composer fallback, working app, same vendor/
e2e-apps:
    cargo nextest run -p phpm --locked --run-ignored only -E 'test(/^e2e_app_/)' --test-threads 1 --no-fail-fast

# shellcheck and jq checks for tools/sweep
sweep-test:
    tools/sweep/test.sh

# shellcheck and jq checks for tools/bench-real
bench-real-test:
    tools/bench-real/test.sh

# shellcheck and helper checks for tools/install-smoke
install-smoke-test:
    tools/install-smoke/test.sh

# node --check and package.json shape checks for the npm wrapper scaffold
npm-wrapper-test:
    dist/npm-wrapper/test.sh

# php -l, composer validate and the Installer unit tests for the Packagist wrapper scaffold
packagist-wrapper-test:
    dist/packagist-wrapper/test.sh

# shellcheck and the detect-target.sh mapping tests for the setup-phpm action scaffold
setup-phpm-action-test:
    dist/setup-phpm-action/test.sh

ci: fmt-check lint docs test deny shear msrv typos md actions sweep-test bench-real-test install-smoke-test npm-wrapper-test packagist-wrapper-test setup-phpm-action-test cov

# warm placement time for a fixture; the first run fills the store
place fixture="laravel-skeleton" *args="":
    cargo run --release --locked -p phpm-store --example place -- fixtures/{{fixture}} {{args}}
