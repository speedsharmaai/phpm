# speedsharmaai/homebrew-phpm (scaffold, not created yet)

This is what the tap repository `speedsharmaai/homebrew-phpm` should contain
once it exists: a standard Homebrew tap, one formula.

```text
homebrew-phpm/
  Formula/
    phpm.rb
  README.md
```

`Formula/phpm.rb` in this directory is a snapshot of what
`dist build --artifacts=global` generates from `dist-workspace.toml` and
`crates/phpm/Cargo.toml` today (`--installer homebrew`). It picks the right
of the 4 macOS/Linux release assets by `OS.mac?`/`Hardware::CPU.arm?` and
installs the `phpm` binary; the two Windows targets don't apply to Homebrew.

## What's not done here, and why

cargo-dist can manage a tap directly — on tag push, with
`[dist].tap = "speedsharmaai/homebrew-phpm"` and a `HOMEBREW_TAP_TOKEN`
secret, the release workflow's `host` job would open a PR against that repo
updating `Formula/phpm.rb` to the new version and URLs on every release.
That's wired up but **turned off**: no `tap` is set in `dist-workspace.toml`
(see the `cargo-dist` PR), so the generated workflow builds `phpm.rb` as a
release asset only and never pushes anywhere.

Turning it on needs, in order:

1. The `speedsharmaai/homebrew-phpm` repository to exist (see below).
2. `tap = "speedsharmaai/homebrew-phpm"` added to `[dist]` in
   `dist-workspace.toml`, then `dist generate` re-run and the release
   workflow re-hardened (SHA-pins, permissions — see
   `.github/workflows/v-release.yml`'s header comment).
3. A `HOMEBREW_TAP_TOKEN` secret (a PAT with write access to the tap repo)
   added to the `phpm` repo.

## The repo itself

Creating `speedsharmaai/homebrew-phpm` on GitHub is a new public-facing
artifact — not done as part of this engineering pass. See the phase 06
report for the owner's go/no-go on an empty placeholder vs. waiting for the
first real release.

## Local verification, once a real release exists

```sh
brew install --build-from-source ./Formula/phpm.rb
phpm --version
brew uninstall phpm
```

This can't be exercised for real yet: `brew install --build-from-source`
needs a release asset URL that resolves (a real GitHub Release), and no tag
has been cut. The formula's structure was verified instead by generating it
locally with `dist build --tag=v0.0.0 --artifacts=global --allow-dirty` and
reading the result (done for this scaffold; not committed as a build
artifact).
