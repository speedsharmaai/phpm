//! `sweep/corpus.json` and the slices of it a run takes.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Project {
    pub repo: Option<String>,
    pub commit: Option<String>,
    #[serde(default)]
    pub stars: Option<u64>,
    #[serde(default)]
    pub fixture: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Corpus {
    pub projects: Vec<Project>,
}

impl Corpus {
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("corpus: {e}"))
    }
}

/// Which projects of the corpus one run takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    All,
    /// `I/N`: every Nth project starting at the Ith (1-based), so each
    /// shard gets a mix of large and small projects.
    Shard {
        index: usize,
        count: usize,
    },
    /// `A..B`: projects A (inclusive) to B (exclusive), in corpus order.
    Slice {
        start: usize,
        end: usize,
    },
}

impl Selection {
    pub fn parse_shard(s: &str) -> Option<Self> {
        let (i, n) = s.split_once('/')?;
        let index: usize = i.parse().ok()?;
        let count: usize = n.parse().ok()?;
        (index >= 1 && index <= count).then_some(Self::Shard { index, count })
    }

    pub fn parse_slice(s: &str) -> Option<Self> {
        let (a, b) = s.split_once("..")?;
        let start: usize = a.parse().ok()?;
        let end: usize = b.parse().ok()?;
        (start <= end).then_some(Self::Slice { start, end })
    }

    pub fn apply<'a>(&self, projects: &'a [Project]) -> Vec<&'a Project> {
        match *self {
            Self::All => projects.iter().collect(),
            Self::Shard { index, count } => projects
                .iter()
                .enumerate()
                .filter(|(i, _)| i % count == index - 1)
                .map(|(_, p)| p)
                .collect(),
            Self::Slice { start, end } => projects
                .iter()
                .skip(start)
                .take(end.saturating_sub(start))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Corpus, Project, Selection};
    use proptest::prelude::*;

    fn projects(n: usize) -> Vec<Project> {
        (0..n)
            .map(|i| Project {
                repo: Some(format!("o/r{i}")),
                commit: Some("c".to_owned()),
                stars: Some(1),
                fixture: None,
            })
            .collect()
    }

    fn names(ps: &[&Project]) -> Vec<String> {
        ps.iter().map(|p| p.repo.clone().unwrap()).collect()
    }

    #[test]
    fn parses_the_corpus_file() {
        let c = Corpus::parse(
            r#"{"taken": "2026-10-01", "projects": [
                {"repo": "a/b", "commit": "abc", "stars": 3, "taken": "x", "source": "s", "notes": ""},
                {"repo": null, "commit": null, "stars": null, "fixture": "ours"}
            ], "dropped": []}"#,
        )
        .unwrap();
        assert_eq!(c.projects.len(), 2);
        assert_eq!(c.projects[0].repo.as_deref(), Some("a/b"));
        assert_eq!(c.projects[1].fixture.as_deref(), Some("ours"));
        assert!(Corpus::parse("[]").is_err());
    }

    #[test]
    fn shards_take_every_nth_project() {
        let ps = projects(7);
        let s = Selection::parse_shard("2/3").unwrap();
        assert_eq!(names(&s.apply(&ps)), ["o/r1", "o/r4"]);
        assert_eq!(Selection::parse_shard("0/3"), None);
        assert_eq!(Selection::parse_shard("4/3"), None);
        assert_eq!(Selection::parse_shard("x"), None);
        assert_eq!(Selection::parse_shard("1/x"), None);
    }

    #[test]
    fn slices_take_a_range_and_stop_at_the_end() {
        let ps = projects(5);
        let s = Selection::parse_slice("1..3").unwrap();
        assert_eq!(names(&s.apply(&ps)), ["o/r1", "o/r2"]);
        let past = Selection::parse_slice("3..10").unwrap();
        assert_eq!(names(&past.apply(&ps)), ["o/r3", "o/r4"]);
        assert_eq!(Selection::parse_slice("3..1"), None);
        assert_eq!(Selection::parse_slice("3"), None);
        assert_eq!(Selection::All.apply(&ps).len(), 5);
    }

    proptest! {
        #[test]
        fn shards_cover_every_project_once(n in 0_usize..60, count in 1_usize..12) {
            let ps = projects(n);
            let mut seen = Vec::new();
            for index in 1..=count {
                seen.extend(names(&Selection::Shard { index, count }.apply(&ps)));
            }
            seen.sort();
            let mut all = names(&ps.iter().collect::<Vec<_>>());
            all.sort();
            prop_assert_eq!(seen, all);
        }

        #[test]
        fn selection_parsers_never_panic(s in ".{0,20}") {
            let _ = Selection::parse_shard(&s);
            let _ = Selection::parse_slice(&s);
        }
    }
}
