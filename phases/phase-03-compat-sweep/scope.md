# Phase 03: Compatibility sweep

Outline. Tasked only after Phase 02 ships.

## Why

"Drop-in" is a claim every competitor makes and none proves. The sweep
turns it into a number, updated nightly, that anyone can check. It is the
launch headline (decision 0004) and the only honest answer to "why trust a
new binary on my install path".

## What ships

- A corpus of lockfiles: the most-installed Packagist projects that ship a
  lock, popular open-source apps (Laravel, Symfony, Drupal, WordPress/Bedrock,
  Magento, Sylius, Monica, Firefly III, Nextcloud apps), and our own.
  Target 300+. Pinned, refreshed monthly.
- A nightly GitHub Actions job: Composer and phpm into sibling directories,
  `diffvendor` on every file, JSON result per project.
- A static results page: "identical `vendor/` on N of M projects", every
  failure listed with the first differing file. Failures that come from
  fallback count as identical only if the app's own `vendor/` is identical.
- Linux and Windows builds in the sweep. Windows: hardlinks, `.bat` proxies,
  junctions for path repos.
- Linux performance work if the Phase 01 gate showed less than 3x there.

## Gate

At least **95% identical** across the corpus, with every remaining difference
understood and ticketed. Below that, the launch post has no headline and
Phase 05 waits.

## Out of scope

Launch, distribution, plugin adapters.
