# Phase 01: Spike

## Why

Two claims hold the whole project up, and both are testable in two weeks.

The first is that a warm `composer install` can be ten times faster. The
floor measurement says yes: 109 package directories cloned in 0.14-0.24 s
against 2.6 s of Composer extraction. But a floor is not a tool. Metadata
writes, the autoloader, and bin proxies all add time, and the only way to
know the real number is to build the path end to end.

The second is that the output can be byte-identical. Composer's autoload
files depend on a weighted package sort with natural-case tie-breaks, PHP's
`var_export` formatting, and `json_encode` quirks. Every one of those is a
place to be almost right. Almost right is worse than slow, because it breaks
production weeks later.

And a third question that did not exist when this project was picked: riff
and vivace are already doing this. If either is ahead, phpm should not exist.
The spike measures them on the same fixtures.

## What ships

- A Cargo workspace with one binary. `phpm install` is the only command.
- The install path from [topology](../../docs/arch/topology.md), steps 0-8,
  for plugin-free lockfiles:
  - fast-path state file and no-op exit
  - lock parse, diff against `installed.json`
  - bounded parallel dist download (Packagist etiquette, decision 0006)
  - extraction into the store, top-dir strip, exec bits kept, atomic rename
  - `clonefile()` per package directory on macOS; hardlink on Linux
  - `installed.json`, `installed.php`, `InstalledVersions.php`
  - autoload family without `-o`: `autoload.php`, `autoload_real.php`,
    `autoload_static.php`, `autoload_psr4.php`, `autoload_namespaces.php`,
    `autoload_classmap.php`, `autoload_files.php`, `platform_check.php`,
    `ClassLoader.php`
  - bin proxies
- `tools/diffvendor`: runs Composer and phpm into two directories and diffs
  every file, bytes and mode. Prints the first difference per file.
- `tools/bench`: hyperfine over Composer, riff, vivace and phpm; cold, warm,
  no-op; JSON output committed to `bench/results/`.
- `fixtures/`: pinned lockfiles, never updated during the phase.
- `gate.md` in this folder: the five numbers, what was learned, and the call.

## Fixtures

| Fixture | Why |
|---|---|
| laravel/laravel skeleton (2026-10-01 lock) | the 64% case, zero plugins |
| symfony/demo with `--no-plugins` | riff's published benchmark, direct comparison |
| ytmate.in `app/` | our own real app |
| wicketyaari.in `app/` | our own real app |
| one large plugin-free library (e.g. a Laravel package's dev lock) | many files, deep trees |
| monicahq/monica with `--no-plugins` | 241 packages, a big real app |

Plugins are disabled on fixtures that have them, on both tools. Phase 01
tests the plugin-free path only.

## Exit criteria

- `phpm install` works from a cold checkout on every fixture, no manual steps.
- `diffvendor` reports zero differences on every fixture.
- hyperfine results committed for Composer, riff, vivace and phpm, three
  scenarios each, macOS APFS and one Linux container.
- The no-op path is under 50 ms.
- Nothing is released: no crate, no npm package, no binary release.
- `gate.md` written with the numbers and the decision.

## Out of scope

`-o` and classmap-authoritative. Plugins, scripts, fallback to Composer.
auth.json and private repositories. Path, vcs and artifact repositories.
Platform verification beyond failing loudly. The malware filter (stubbed with
a TODO that blocks release). Windows. `update`, `require`, anything with a
resolver. Releases, installers, docs site, benchmark page, launch.

None of these are cut for time. They are cut because each is wasted work if
the gate says no, and none of them changes what the gate says.
