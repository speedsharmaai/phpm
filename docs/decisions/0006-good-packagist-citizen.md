# 0006: A good Packagist citizen

Status: accepted
Date: 2026-10-01

## Context

Packagist costs more than $1M a year to run in 2026. Private Packagist pays
more than half of it. In 2025 Packagist said automated traffic is now the
majority and floated rate limits and charges for heavy use. In July 2026 it
launched a sponsorship programme that names caching services and AI
consumers as parties that should pay.

Its API guidance is specific: a User-Agent with contact details, at most 10
concurrent requests (20 for static files), HTTP/2, If-Modified-Since.

Composer 2.10 added a malware filter that fetches a blocking list on every
install. A tool that skips it installs packages Composer would refuse, which
is a security regression enterprises will reject on sight.

Composer also POSTs to `notify-batch` after installs. That is how package
authors get their download counts.

## Decision

- User-Agent `phpm/<version> (+https://<repo>; mailto=<brand email>)`.
- Packagist metadata: at most 10 concurrent requests, conditional GETs
  always. Dist downloads from codeload go to GitHub, not Packagist, and are
  bounded separately.
- Download notifications are sent, batched, the same way Composer sends them.
- The malware filter and the security-advisory checks are honoured with
  Composer's defaults. Any caching of the filter list uses Packagist's own
  cache headers, not a longer TTL of our choosing.
- phpm never mirrors Packagist and never offers a hosted registry.

## Consequences

- The no-op path cannot be fully offline when the filter list is stale. That
  costs milliseconds with a 304, and it is the right trade.
- Before launch, a short note goes to the Composer maintainers saying what
  phpm is and how it behaves toward Packagist. Better they hear it from us.
