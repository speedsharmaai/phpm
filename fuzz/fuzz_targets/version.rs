#![no_main]

use libfuzzer_sys::fuzz_target;
use phpm_lock::constraint::{lower_bound, parse, version_compare};
use phpm_lock::version::{normalize, normalize_branch, parse_numeric_alias_prefix};

fuzz_target!(|text: &str| {
    let normalized = normalize(text);
    let constraint = parse(text);
    if let (Ok(v), Ok(c)) = (&normalized, &constraint) {
        let _ = c.matches_version(v);
        let _ = c.bounds();
        let _ = version_compare(v, text);
    }
    let _ = normalize_branch(text);
    let _ = parse_numeric_alias_prefix(text);
    let _ = lower_bound(text);
});
