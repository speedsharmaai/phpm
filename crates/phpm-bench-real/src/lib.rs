//! Aggregate `tools/bench-real/*`'s JSON Lines output into the
//! `bench-real/` results page's data files, and the README's top-N table.

pub mod aggregate;
pub mod cli;
pub mod readme;
pub mod record;
