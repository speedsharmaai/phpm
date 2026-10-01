//! Shard records merged into one results file, its headline numbers, the
//! history the page charts, and the README badge.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::record::{Outcome, PhpmPath, Record};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub total: usize,
    pub installable: usize,
    pub identical: usize,
    pub different: usize,
    pub phpm_failed: usize,
    pub composer_failed: usize,
    pub fetch_failed: usize,
    /// Installable projects phpm installed with no step sent to Composer.
    pub native: usize,
    pub partial: usize,
    pub full: usize,
    /// `identical / installable`, 0 when nothing was installable.
    pub ratio: f64,
    pub native_rate: f64,
}

fn rate(n: usize, of: usize) -> f64 {
    if of == 0 {
        return 0.0;
    }
    let (n, of) = (
        u32::try_from(n).unwrap_or(u32::MAX),
        u32::try_from(of).unwrap_or(u32::MAX),
    );
    f64::from(n) / f64::from(of)
}

impl Summary {
    pub fn of<'a>(records: impl IntoIterator<Item = &'a Record>) -> Self {
        let mut s = Self::default();
        for r in records {
            s.total += 1;
            match r.outcome {
                Outcome::Identical => s.identical += 1,
                Outcome::Different => s.different += 1,
                Outcome::PhpmFailed => s.phpm_failed += 1,
                Outcome::ComposerFailed => s.composer_failed += 1,
                Outcome::FetchFailed => s.fetch_failed += 1,
            }
            if r.outcome.installable() {
                s.installable += 1;
                match r.phpm_path {
                    PhpmPath::Native => s.native += 1,
                    PhpmPath::Partial => s.partial += 1,
                    PhpmPath::Full => s.full += 1,
                    PhpmPath::Unknown => {}
                }
            }
        }
        s.ratio = rate(s.identical, s.installable);
        s.native_rate = rate(s.native, s.installable);
        s
    }

    /// "identical vendor/ on N of M projects".
    pub fn headline(&self) -> String {
        format!(
            "identical vendor/ on {} of {} projects ({:.1}%)",
            self.identical,
            self.installable,
            self.ratio * 100.0
        )
    }
}

/// The key a record's numbers are grouped under: the mode on Linux,
/// `<os>-<mode>` elsewhere.
pub fn group(r: &Record) -> String {
    if r.os == "linux" {
        r.mode.to_string()
    } else {
        format!("{}-{}", r.os, r.mode)
    }
}

/// Records read from JSON Lines, one per line; blank lines skipped.
pub fn parse_lines(text: &str) -> Result<Vec<Record>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("line {}: {e}", i + 1)))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Results {
    pub date: String,
    #[serde(default)]
    pub meta: BTreeMap<String, String>,
    pub summary: BTreeMap<String, Summary>,
    pub records: Vec<Record>,
}

impl Results {
    pub fn new(date: &str, meta: BTreeMap<String, String>, mut records: Vec<Record>) -> Self {
        records.sort_by(|a, b| {
            b.stars
                .cmp(&a.stars)
                .then_with(|| a.name().cmp(&b.name()))
                .then_with(|| a.mode.cmp(&b.mode))
                .then_with(|| a.os.cmp(&b.os))
        });
        let mut groups: BTreeMap<String, Vec<&Record>> = BTreeMap::new();
        for r in &records {
            groups.entry(group(r)).or_default().push(r);
        }
        let summary = groups
            .into_iter()
            .map(|(k, rs)| (k, Summary::of(rs)))
            .collect();
        Self {
            date: date.to_owned(),
            meta,
            summary,
            records,
        }
    }
}

/// The history file with today's headline numbers added (replacing an entry
/// for the same date).
pub fn add_history(history: Option<&str>, results: &Results) -> Value {
    let mut entries: Vec<Value> = history
        .and_then(|h| serde_json::from_str::<Vec<Value>>(h).ok())
        .unwrap_or_default();
    entries.retain(|e| e.get("date").and_then(Value::as_str) != Some(results.date.as_str()));
    let summary: BTreeMap<&String, Value> = results
        .summary
        .iter()
        .map(|(k, s)| {
            (
                k,
                json!({"identical": s.identical, "installable": s.installable, "ratio": s.ratio, "native_rate": s.native_rate}),
            )
        })
        .collect();
    entries.push(json!({"date": results.date, "summary": summary}));
    entries.sort_by(|a, b| {
        let d = |v: &Value| {
            v.get("date")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        d(a).cmp(&d(b))
    });
    Value::Array(entries)
}

/// A shields.io endpoint badge with the `pure` ratio.
pub fn badge(results: &Results) -> Value {
    match results.summary.get("pure") {
        Some(s) if s.installable > 0 => {
            let pct = s.ratio * 100.0;
            let color = if pct >= 95.0 {
                "brightgreen"
            } else if pct >= 80.0 {
                "yellow"
            } else {
                "red"
            };
            json!({
                "schemaVersion": 1,
                "label": "identical vendor/",
                "message": format!("{} of {} ({pct:.1}%)", s.identical, s.installable),
                "color": color,
            })
        }
        _ => {
            json!({"schemaVersion": 1, "label": "identical vendor/", "message": "no data", "color": "lightgrey"})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Results, Summary, add_history, badge, group, parse_lines};
    use crate::record::{Mode, Outcome, PhpmPath, Record};
    use std::collections::BTreeMap;

    fn rec(repo: &str, stars: u64, outcome: Outcome, path: PhpmPath) -> Record {
        Record {
            repo: Some(repo.to_owned()),
            commit: Some("c".to_owned()),
            fixture: None,
            stars: Some(stars),
            mode: Mode::Pure,
            os: "linux".to_owned(),
            outcome,
            packages: Some(1),
            compared: vec!["vendor".to_owned()],
            differences: 0,
            first_differences: vec![],
            phpm_path: path,
            phpm_reasons: vec![],
            composer: None,
            phpm: None,
            error: None,
        }
    }

    #[test]
    fn composer_failures_leave_the_ratio() {
        let rs = [
            rec("a/a", 1, Outcome::Identical, PhpmPath::Native),
            rec("a/b", 1, Outcome::Identical, PhpmPath::Partial),
            rec("a/c", 1, Outcome::Different, PhpmPath::Full),
            rec("a/d", 1, Outcome::PhpmFailed, PhpmPath::Unknown),
            rec("a/e", 1, Outcome::ComposerFailed, PhpmPath::Unknown),
            rec("a/f", 1, Outcome::FetchFailed, PhpmPath::Unknown),
        ];
        let s = Summary::of(&rs);
        assert_eq!((s.total, s.installable, s.identical), (6, 4, 2));
        assert_eq!((s.different, s.phpm_failed), (1, 1));
        assert_eq!((s.composer_failed, s.fetch_failed), (1, 1));
        assert_eq!((s.native, s.partial, s.full), (1, 1, 1));
        assert!((s.ratio - 0.5).abs() < 1e-9);
        assert!((s.native_rate - 0.25).abs() < 1e-9);
        assert_eq!(s.headline(), "identical vendor/ on 2 of 4 projects (50.0%)");
        assert!(Summary::of(&[]).ratio.abs() < f64::EPSILON);
    }

    #[test]
    fn groups_by_mode_and_names_other_systems() {
        let mut r = rec("a/a", 1, Outcome::Identical, PhpmPath::Native);
        assert_eq!(group(&r), "pure");
        r.mode = Mode::Default;
        assert_eq!(group(&r), "default");
        r.os = "windows".to_owned();
        assert_eq!(group(&r), "windows-default");
    }

    #[test]
    fn results_sort_by_stars_and_sum_each_group() {
        let mut win = rec("a/low", 1, Outcome::Different, PhpmPath::Native);
        win.os = "windows".to_owned();
        let res = Results::new(
            "2026-10-01",
            BTreeMap::new(),
            vec![
                rec("a/low", 1, Outcome::Identical, PhpmPath::Native),
                rec("a/high", 9, Outcome::Identical, PhpmPath::Native),
                win,
            ],
        );
        assert_eq!(res.records[0].repo.as_deref(), Some("a/high"));
        assert_eq!(res.summary["pure"].identical, 2);
        assert_eq!(res.summary["windows-pure"].different, 1);
    }

    #[test]
    fn reads_json_lines() {
        let r = rec("a/a", 1, Outcome::Identical, PhpmPath::Native);
        let line = serde_json::to_string(&r).unwrap();
        let text = format!("{line}\n\n{line}\n");
        assert_eq!(parse_lines(&text).unwrap().len(), 2);
        assert!(parse_lines("{").unwrap_err().starts_with("line 1:"));
    }

    #[test]
    fn history_keeps_one_entry_per_date_in_order() {
        let res = |d: &str| {
            Results::new(
                d,
                BTreeMap::new(),
                vec![rec("a/a", 1, Outcome::Identical, PhpmPath::Native)],
            )
        };
        let h = add_history(None, &res("2026-10-02"));
        let h = add_history(Some(&h.to_string()), &res("2026-10-01"));
        let h = add_history(Some(&h.to_string()), &res("2026-10-02"));
        let dates: Vec<&str> = h
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["date"].as_str().unwrap())
            .collect();
        assert_eq!(dates, ["2026-10-01", "2026-10-02"]);
        assert_eq!(h[0]["summary"]["pure"]["identical"], 1);
        assert_eq!(
            add_history(Some("not json"), &res("x"))
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn badge_colours_follow_the_gate() {
        let with = |outcomes: &[Outcome]| {
            let rs = outcomes
                .iter()
                .enumerate()
                .map(|(i, o)| rec(&format!("a/{i}"), 1, *o, PhpmPath::Native))
                .collect();
            badge(&Results::new("d", BTreeMap::new(), rs))
        };
        let green = with(&[Outcome::Identical; 20]);
        assert_eq!(green["color"], "brightgreen");
        assert_eq!(green["message"], "20 of 20 (100.0%)");
        let mut some = vec![Outcome::Identical; 9];
        some.push(Outcome::Different);
        assert_eq!(with(&some)["color"], "yellow");
        assert_eq!(with(&[Outcome::Different])["color"], "red");
        assert_eq!(with(&[])["message"], "no data");
    }
}
