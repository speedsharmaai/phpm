//! The README's top-N table: plain markdown, generated from `Results` so
//! it is never hand-typed.

use std::fmt::Write as _;

use crate::aggregate::Results;

/// A markdown table of the `n` projects with the highest warm speedup on
/// `os`, linking each repo and the full page.
pub fn top_table(results: &Results, os: &str, n: usize, page_url: &str) -> String {
    let top = results.top_by_warm_speedup(os, n);
    if top.is_empty() {
        return format!("No {os} results yet. Full page: [{page_url}]({page_url}).\n");
    }
    let mut out = String::new();
    out.push_str("| Project | Stars | Packages | Composer warm | phpm warm | Speed-up |\n");
    out.push_str("|---|---|---|---|---|---|\n");
    for r in top {
        let Some(t) = r.warm else { continue };
        let Some(speedup) = t.speedup() else { continue };
        let name = r.name();
        let link = r
            .repo
            .as_ref()
            .map(|repo| format!("[{name}](https://github.com/{repo})"))
            .unwrap_or(name);
        let _ = writeln!(
            out,
            "| {link} | {} | {} | {:.3} s | {:.3} s | {speedup:.1}x |",
            r.stars.map_or("-".to_owned(), |s| s.to_string()),
            r.packages.map_or("-".to_owned(), |p| p.to_string()),
            t.composer_seconds,
            t.phpm_seconds,
        );
    }
    let _ = write!(
        out,
        "\nFull table, all {} projects, three OSes: [{page_url}]({page_url}).\n",
        results.records.len()
    );
    out
}

#[cfg(test)]
mod tests {
    use super::top_table;
    use crate::aggregate::Results;
    use crate::record::{Identity, ProjectRecord, Timing};
    use std::collections::BTreeMap;

    fn rec(repo: &str, stars: u64, warm: Option<Timing>) -> ProjectRecord {
        ProjectRecord {
            group: "big-apps".to_owned(),
            repo: Some(repo.to_owned()),
            fixture: None,
            stars: Some(stars),
            os: "ubuntu-latest".to_owned(),
            packages: Some(42),
            cold: None,
            warm,
            noop: None,
            identity: Identity::Identical,
            differences: 0,
            fallback: false,
            fallback_plugins: vec![],
            error: None,
        }
    }

    #[test]
    fn ranks_by_warm_speedup_and_links_each_repo() {
        let results = Results::new(
            "2026-10-01",
            BTreeMap::new(),
            vec![
                rec(
                    "slow/one",
                    10,
                    Some(Timing {
                        composer_seconds: 4.0,
                        phpm_seconds: 2.0,
                    }),
                ),
                rec(
                    "fast/one",
                    5,
                    Some(Timing {
                        composer_seconds: 20.0,
                        phpm_seconds: 1.0,
                    }),
                ),
            ],
            &[],
            &[],
        );
        let table = top_table(&results, "ubuntu-latest", 1, "https://example.test/bench/");
        assert!(table.contains("[fast/one](https://github.com/fast/one)"));
        assert!(!table.contains("slow/one"));
        assert!(table.contains("20.0x"));
        assert!(table.contains("https://example.test/bench/"));
    }

    #[test]
    fn says_so_when_there_is_nothing_for_that_os() {
        let results = Results::new("d", BTreeMap::new(), vec![], &[], &[]);
        let table = top_table(
            &results,
            "windows-latest",
            10,
            "https://example.test/bench/",
        );
        assert!(table.contains("No windows-latest results yet"));
    }
}
