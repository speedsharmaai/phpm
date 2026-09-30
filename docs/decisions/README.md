# Decisions

Numbered, immutable once accepted. Supersede with a new record, never edit.

- [0001](0001-name.md) The working name is phpm, cleared on crates.io, npm, Homebrew and GitHub, trademark search pending
- [0002](0002-lockfile-install-first.md) `install` from a lockfile first; no resolver until Phase 07
- [0003](0003-global-store-and-clone.md) A global content-addressed store of extracted packages, cloned into `vendor/`, never symlinked
- [0004](0004-byte-identical-or-fall-back.md) Output is byte-identical to Composer's, or that part is handed to Composer
- [0005](0005-rust-single-binary.md) Rust, one static binary, PHP only called for platform detection and scripts
- [0006](0006-good-packagist-citizen.md) Stay inside Packagist's limits, send download notifications, honour the malware filter
- [0007](0007-crowded-field-contribute-if-behind.md) Eight clones exist; if phpm is not clearly ahead at the gate, contribute to the leader instead
- [0008](0008-quality-gates-before-code.md) Quality gates (clippy, prek, committed, Sonar, deny, CI) set up on an empty workspace before any feature code
- [0009](0009-public-from-day-one.md) The repo is public from the first commit; every public-repo security feature is on from Phase 00
