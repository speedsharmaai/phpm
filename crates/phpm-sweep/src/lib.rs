//! The compatibility sweep: install a corpus of lockfiles with Composer and
//! with phpm, compare the trees, and publish the numbers.

pub mod cli;
pub mod corpus;
pub mod record;
pub mod report;
pub mod runner;
