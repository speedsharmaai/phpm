# Phase 05 progress

## First full run, 2026-10-02

Run [36962624240](https://github.com/speedsharmaai/phpm/actions/runs/36962624240):
all 40 projects in [`bench-real/corpus.json`](../../bench-real/corpus.json) on
GitHub-hosted `ubuntu-latest`, `macos-latest` and `windows-latest`, Composer
2.10.3, PHP 8.4, phpm at `246aa97`. Every job green, page deployed to
[/bench/](https://speedsharmaai.github.io/phpm/bench/). The run's
`results.json` is committed as
[`bench-real/results/2026-10-02.json`](../../bench-real/results/2026-10-02.json).

Both tools install with `--no-scripts --no-plugins --ignore-platform-reqs`
(the sweep's `pure` mode), so these numbers measure phpm's own install path
and `fallback` is never set. Cold is 3 runs with every cache deleted, warm 5
runs with `vendor/` deleted, no-op 10 runs, each with a discarded first run
for warm and no-op.

### What the headline numbers leave out, and why

Six of the 40 projects per OS are not in the numbers below:

- **nextcloud/server, owncloud/core, joomla/joomla-cms, opencart/opencart,
  humhub/humhub: harness bug, not a phpm result.** All five set
  `config.vendor-dir` (`lib/composer`, `libraries/vendor`,
  `upload/system/storage/vendor/`, `protected/vendor`). The harness only
  deleted `vendor/` between warm runs, so every "warm" run was really a
  no-op (1-2 ms, up to "1926x"), and `diffvendor` was pointed at two
  `vendor/` directories that do not exist; it exited with an error, which
  the script recorded as one difference. Fixed in #111 (reads
  `config.vendor-dir`, treats a `diffvendor` error as a failed row, never a
  count) and checked end to end on nextcloud/server locally: identical,
  phpm warm 149 ms against Composer's 1.92 s. These five are re-measured in
  the next weekly run.
- **PrestaShop/PrestaShop: harness bug.** Its tree ships a dangling
  symlink, which `cp -RL` cannot copy, so the row is `install-failed` on all
  three OSes. Also fixed in #111.

The published `results.json` still contains all 120 rows as they ran; the
filter used for every number here is

```sh
jq '.records |= map(select(.identity == "identical"
  and ((.repo // "") as $r | ["nextcloud/server","owncloud/core",
  "joomla/joomla-cms","opencart/opencart","humhub/humhub"] | index($r) | not)))'
```

(identity rates below use the same exclusion but keep `different` rows).

### Identity

`diffvendor` on the whole vendor tree, bytes and modes, 34 comparable
projects per OS:

| OS | Identical | Different |
|---|---|---|
| ubuntu-latest | 33 of 34 (97.1%) | getgrav/grav |
| macos-latest | 33 of 34 (97.1%) | getgrav/grav |
| windows-latest | 30 of 34 (88.2%) | getgrav/grav, wallabag/wallabag, mautic/mautic, drupal/drupal (5 files) |

The single-difference counts were read by the pre-#111 parser, which could
not tell one real difference from a `diffvendor` error; they are re-checked
in the next run before anyone files them as phpm bugs.

### Speed-up, Composer time over phpm time

Median across the comparable projects (range in brackets):

| OS | Cold | Warm | No-op |
|---|---|---|---|
| ubuntu-latest | 2.6x (1.2-6.4) | **9.6x** (5.9-22.3) | 636x (138-2632) |
| macos-latest | 3.8x (2.6-14.1) | **18.3x** (10.2-62.8) | 420x (165-2296) |
| windows-latest | 0.9x (0.47-1.7) | **1.6x** (0.72-4.2) | 329x (90-3368) |

Windows is the honest loss. Cold is a wash or slower, and warm is 1.6x at
the median with at least one project slower than Composer (0.72x). phpm
places packages on Windows by hardlink (no `clonefile`), Windows Defender
scans every new file either tool writes, and the harness's Windows timing
goes through git-bash. Worth a profiling pass before Phase 06 makes any
Windows claim.

### Ten worktrees of laravel-skeleton

`composer install` x10 (ten full `vendor/` copies) against `phpm install`
x10 from a warm store, total wall time and real disk consumed (free-space
delta, so shared blocks are not double-counted):

| OS | Time | Disk |
|---|---|---|
| ubuntu-latest | 6.0x faster | 4.9x smaller |
| macos-latest | 11.4x faster | 5.8x smaller |
| windows-latest | 1.4x faster | 3.8x smaller |

### CI job time, with and without a warm cache

Four projects (symfony/demo, filamentphp/demo, kimai/kimai,
laravelio/laravel.io), whole job (install plus a lint pass over 200
`vendor/` files), p50 speed-up:

| OS | Cache warm | Cache cold |
|---|---|---|
| ubuntu-latest | 1.26x (1.25-1.33) | 1.31x (1.15-1.43) |
| macos-latest | 1.35x (1.27-2.29) | 1.58x (1.29-2.00) |
| windows-latest | 1.30x (1.19-1.35) | 0.86x (0.72-1.00) |

**Kill criterion 2 from the market research, "at least 3x on p50 total CI
job time", is not met.** Once the install is a fraction of the job, a faster
install moves the total by 25-35%. The install-step numbers above stand; the
claim the launch post can make is about installs, agents and worktrees, not
about CI jobs getting 3x shorter.

### Time saved per year

The page multiplies Packagist's ~5 billion installs a month (the figure in
[why phpm](../../docs/why-phpm.md)) by the per-install warm saving and an assumed 1%
share of installs that are warm reinstalls like these. The 1% is a guess,
stated as one on the page, not a measurement.

## Getting Windows to run at all

The Linux and macOS paths worked on the first full attempt. Windows took
eight rounds, each found by a validation slice
(`oses=windows-latest, projects_per_shard=5, max_shards=1`):

1. `hyperfine --prepare "rm -rf ..."` runs through `cmd.exe` by default,
   which has no `rm` (#90 tried `--shell bash`).
2. `-N` is `--shell=none`, so `-N --shell bash` is rejected outright (#93
   went back to `-N` with `bash -c` in the prepare string).
3. Under `-N` the prepare string is tokenised by hyperfine itself and the
   quoting does not survive on Windows (#97: `--shell bash` for cold and
   warm only).
4. hyperfine 1.20.0's shell-spawning calibration fails for any non-default
   shell on `windows-latest`; an absolute bash path did not help (#99).
   Read from hyperfine's `executor.rs`: the `/C` in its error message is
   cosmetic, the spawn itself fails.
5. Stopped using hyperfine's `--prepare` and `--shell` entirely: cleanup is
   plain bash between iterations, each timed by `hyperfine -N --runs 1`, and
   the per-iteration times are combined into the same JSON shape (#102).
   This costs a few milliseconds of process spawn per iteration and nothing
   else.
6. `cp -R` cannot create symlinks without elevation on `windows-latest`, and
   setup-php's Windows PHP lacks `zip`, which symfony/demo's lock needs
   (#106).
7. One project's timing failing (invoiceninja, 310 packages, stalled for
   about 50 minutes on a cold run) took down every project after it in the
   shard (#107).
8. The record was built by interpolating each mean into a hand-written JSON
   string, so one missing scenario file made the whole record invalid
   (#110: each value is now its own `--argjson`).

Separately, the first deploys came back `skipped` while a `sweep.yml`
deploy was in flight; both workflows now share a `pages` concurrency group
on their deploy jobs (#88), and this run deployed.

## Not done

- **The owner's own Mac column.** Deferred by the coordinator so the Mac
  stayed free for this phase; the Phase 01/02 Mac numbers in the README
  stand in the meantime.
- **Headline chart PNG at 3200x1800.** The page draws the charts in plain
  SVG; exporting one as an image is a follow-up.
- **No per-run timeout.** A stalled install can hold one Windows shard for
  close to an hour (invoiceninja); #107 stops it taking other projects down
  with it, but not the wait.
