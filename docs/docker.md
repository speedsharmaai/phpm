# Docker: a PHP-free build stage

phpm is one static binary (decision 0005): no PHP interpreter needed for the
fast path, `php` is only called once for platform detection and real
Composer scripts/plugins. That means the stage that turns `composer.lock`
into `vendor/` doesn't need a PHP image at all — copy the phpm binary in,
run `phpm install`, then copy the result into the real PHP runtime stage.

```text
  FROM speedsharmaai/phpm AS phpm        no PHP, just the binary
             │ COPY --from=phpm
  ┌──────────▼──────────────┐
  │ deps stage               │  COPY composer.json composer.lock
  │ (no PHP interpreter)     │  RUN phpm install --no-dev --no-scripts
  └──────────┬──────────────┘
             │ COPY --from=deps /app/vendor
  ┌──────────▼──────────────┐
  │ php:8.4-fpm (or similar) │  COPY --from=deps vendor/
  │ the real runtime image   │  COPY . .
  └──────────────────────────┘
```

A release image at `speedsharmaai/phpm` (built from the same cargo-dist
release, not yet published — see the phase 06 report) would make the first
stage a plain `COPY --from=`. Until it exists, the pattern works the same
way by downloading the release binary directly:

```dockerfile
# syntax=docker/dockerfile:1

FROM debian:bookworm-slim AS phpm-install
ARG PHPM_VERSION=0.1.1
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && case "$TARGETARCH" in \
         amd64) triple=x86_64-unknown-linux-musl ;; \
         arm64) triple=aarch64-unknown-linux-musl ;; \
         *) echo "unsupported arch: $TARGETARCH" >&2; exit 1 ;; \
       esac \
    && curl --proto '=https' --tlsv1.2 -fsSL \
         "https://github.com/speedsharmaai/phpm/releases/download/v${PHPM_VERSION}/phpm-${triple}.tar.xz" \
         -o /tmp/phpm.tar.xz \
    && tar xf /tmp/phpm.tar.xz --strip-components 1 -C /usr/local/bin phpm \
    && rm /tmp/phpm.tar.xz

# This stage never installs PHP: phpm's fast path doesn't call it, and
# --no-scripts/--no-plugins means it never needs to for this build.
FROM phpm-install AS deps
WORKDIR /app
COPY composer.json composer.lock ./
RUN phpm install --no-dev --no-scripts --no-plugins --ignore-platform-reqs

FROM php:8.4-fpm AS runtime
WORKDIR /app
COPY --from=deps /app/vendor ./vendor
COPY . .
```

## Why `--no-scripts --no-plugins` here

Real Composer scripts (`post-autoload-dump` calling into application code,
a package's own install hooks) and plugins need PHP, and phpm hands those
to a real `composer install` when it can't reproduce them (decision 0004).
A build stage with no PHP interpreter can't fall back, so it has to opt out
of the parts that would need one. Most Laravel/Symfony apps' own
post-install steps (`artisan package:discover`, cache warming) belong in
the runtime stage anyway, after the full application code is present — this
stage only needs `vendor/` to exist, correctly and byte-identical to what
Composer would have produced.

If a project's scripts or plugins are load-bearing for the dependency step
itself (rare), drop the PHP-free stage and run `phpm install` (without
`--no-scripts`/`--no-plugins`) in a stage that does have PHP — still faster
than Composer for the native-install portion, with Composer handling just
the step that needs it.

## Worth combining with the store

Building many images (a monorepo, a matrix of PHP versions, CI running the
same `composer.lock` across jobs) multiplies installs the same way parallel
worktrees do — see [the worktree recipe](agent-worktrees.md) for the shared
`PHPM_CACHE_DIR` pattern. Mounting the store as a BuildKit cache mount
(`RUN --mount=type=cache,target=/root/.cache/phpm phpm install ...`) gets
the same warm-store benefit across image builds, the way
`actions/cache` does for `setup-phpm` in CI (`dist/setup-phpm-action/`).
