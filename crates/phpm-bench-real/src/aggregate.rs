//! Shard records merged into one results file, the headline numbers per
//! OS, the history the page charts, and a shields.io badge.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::record::{CiScenarioRecord, Identity, ProjectRecord, Timing, WorktreeRecord};

fn median(mut xs: Vec<f64>) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = xs.len() / 2;
    Some(if xs.len().is_multiple_of(2) {
        f64::midpoint(xs[mid - 1], xs[mid])
    } else {
        xs[mid]
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScenarioSummary {
    pub n: usize,
    pub median_speedup: f64,
    pub min_speedup: f64,
    pub max_speedup: f64,
}

impl ScenarioSummary {
    fn of(speedups: Vec<f64>) -> Option<Self> {
        if speedups.is_empty() {
            return None;
        }
        let min = speedups.iter().copied().fold(f64::INFINITY, f64::min);
        let max = speedups.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Some(Self {
            n: speedups.len(),
            median_speedup: median(speedups).unwrap_or(0.0),
            min_speedup: min,
            max_speedup: max,
        })
    }
}

fn scenario_summary(
    records: &[&ProjectRecord],
    pick: impl Fn(&ProjectRecord) -> Option<Timing>,
) -> Option<ScenarioSummary> {
    let speedups = records
        .iter()
        .filter_map(|r| pick(r).and_then(|t| t.speedup()))
        .collect();
    ScenarioSummary::of(speedups)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub total: usize,
    pub identical: usize,
    pub different: usize,
    pub install_failed: usize,
    pub fallback: usize,
    /// `identical / (identical + different)`, 0 when neither happened.
    pub identity_rate: f64,
    pub fallback_rate: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cold: Option<ScenarioSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warm: Option<ScenarioSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noop: Option<ScenarioSummary>,
}

impl Summary {
    pub fn of<'a>(records: impl IntoIterator<Item = &'a ProjectRecord>) -> Self {
        let records: Vec<&ProjectRecord> = records.into_iter().collect();
        let mut s = Self::default();
        for r in &records {
            s.total += 1;
            match r.identity {
                Identity::Identical => s.identical += 1,
                Identity::Different => s.different += 1,
                Identity::InstallFailed => s.install_failed += 1,
            }
            if r.fallback {
                s.fallback += 1;
            }
        }
        let compared = s.identical + s.different;
        s.identity_rate = rate(s.identical, compared);
        s.fallback_rate = rate(s.fallback, s.total);
        s.cold = scenario_summary(&records, |r| r.cold);
        s.warm = scenario_summary(&records, |r| r.warm);
        s.noop = scenario_summary(&records, |r| r.noop);
        s
    }

    /// "N of M identical".
    pub fn headline(&self) -> String {
        format!(
            "{} of {} identical",
            self.identical,
            self.identical + self.different
        )
    }
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorktreeSummary {
    pub os: String,
    pub worktrees: usize,
    pub time_speedup: f64,
    pub disk_ratio: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CiSummary {
    pub os: String,
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_cache_speedup: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub without_cache_speedup: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Results {
    pub date: String,
    #[serde(default)]
    pub meta: BTreeMap<String, String>,
    pub summary: BTreeMap<String, Summary>,
    pub records: Vec<ProjectRecord>,
    #[serde(default)]
    pub worktree: Vec<WorktreeSummary>,
    #[serde(default)]
    pub ci_scenario: Vec<CiSummary>,
}

impl Results {
    pub fn new(
        date: &str,
        meta: BTreeMap<String, String>,
        mut records: Vec<ProjectRecord>,
        worktree: &[WorktreeRecord],
        ci: &[CiScenarioRecord],
    ) -> Self {
        records.sort_by(|a, b| {
            b.stars
                .cmp(&a.stars)
                .then_with(|| a.name().cmp(&b.name()))
                .then_with(|| a.os.cmp(&b.os))
        });
        let mut by_os: BTreeMap<String, Vec<&ProjectRecord>> = BTreeMap::new();
        for r in &records {
            by_os.entry(r.os.clone()).or_default().push(r);
        }
        let summary = by_os
            .into_iter()
            .map(|(os, rs)| (os, Summary::of(rs)))
            .collect();

        let mut by_os_wt: BTreeMap<String, Vec<&WorktreeRecord>> = BTreeMap::new();
        for w in worktree {
            by_os_wt.entry(w.os.clone()).or_default().push(w);
        }
        let worktree_summary = by_os_wt
            .into_iter()
            .map(|(os, ws)| WorktreeSummary {
                os,
                worktrees: ws.first().map_or(0, |w| w.worktrees),
                time_speedup: median(ws.iter().filter_map(|w| w.time_speedup()).collect())
                    .unwrap_or(0.0),
                disk_ratio: median(ws.iter().filter_map(|w| w.disk_ratio()).collect())
                    .unwrap_or(0.0),
            })
            .collect();

        let mut by_os_project: BTreeMap<(String, String), Vec<&CiScenarioRecord>> = BTreeMap::new();
        for c in ci {
            by_os_project
                .entry((c.os.clone(), c.project.clone()))
                .or_default()
                .push(c);
        }
        let ci_summary = by_os_project
            .into_iter()
            .map(|((os, project), cs)| CiSummary {
                os,
                project,
                with_cache_speedup: cs.iter().find(|c| c.cache).and_then(|c| c.speedup()),
                without_cache_speedup: cs.iter().find(|c| !c.cache).and_then(|c| c.speedup()),
            })
            .collect();

        Self {
            date: date.to_owned(),
            meta,
            summary,
            records,
            worktree: worktree_summary,
            ci_scenario: ci_summary,
        }
    }

    /// The `n` projects with the highest warm speedup on `os`, highest first.
    pub fn top_by_warm_speedup(&self, os: &str, n: usize) -> Vec<&ProjectRecord> {
        let mut rs: Vec<&ProjectRecord> = self
            .records
            .iter()
            .filter(|r| r.os == os && r.warm.and_then(|t| t.speedup()).is_some())
            .collect();
        rs.sort_by(|a, b| {
            let sa = a.warm.and_then(|t| t.speedup()).unwrap_or(0.0);
            let sb = b.warm.and_then(|t| t.speedup()).unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        rs.truncate(n);
        rs
    }
}

/// One calendar day's headline numbers, keyed by OS.
fn day_summary(results: &Results) -> Value {
    let by_os: BTreeMap<&String, Value> = results
        .summary
        .iter()
        .map(|(os, s)| {
            let entry = json!({
                "identity_rate": s.identity_rate,
                "fallback_rate": s.fallback_rate,
                "warm_median_speedup": s.warm.as_ref().map(|w| w.median_speedup),
            });
            (os, entry)
        })
        .collect();
    json!({"date": results.date, "summary": by_os})
}

/// The history, with today's entry added or replaced, oldest first. Kept
/// as a date-keyed map while building so a rerun on the same day overwrites
/// rather than duplicates.
pub fn add_history(history: Option<&str>, results: &Results) -> Value {
    let mut by_date: BTreeMap<String, Value> = BTreeMap::new();
    let previous = history.and_then(|h| serde_json::from_str::<Vec<Value>>(h).ok());
    for entry in previous.into_iter().flatten() {
        if let Some(date) = entry.get("date").and_then(Value::as_str) {
            by_date.insert(date.to_owned(), entry);
        }
    }
    by_date.insert(results.date.clone(), day_summary(results));
    Value::Array(by_date.into_values().collect())
}

/// A shields.io endpoint badge with the best (highest-speedup) OS's warm
/// median speedup.
pub fn badge(results: &Results) -> Value {
    let best = results
        .summary
        .values()
        .filter_map(|s| s.warm.as_ref().map(|w| w.median_speedup))
        .fold(0.0_f64, f64::max);
    if best <= 0.0 {
        return json!({"schemaVersion": 1, "label": "phpm vs composer", "message": "no data", "color": "lightgrey"});
    }
    let color = if best >= 10.0 {
        "brightgreen"
    } else if best >= 3.0 {
        "yellow"
    } else {
        "red"
    };
    json!({
        "schemaVersion": 1,
        "label": "phpm vs composer, warm install",
        "message": format!("{best:.1}x"),
        "color": color,
    })
}

#[cfg(test)]
mod tests {
    use super::{Results, Summary, add_history, badge};
    use crate::record::{CiScenarioRecord, Identity, ProjectRecord, Timing, WorktreeRecord};
    use std::collections::BTreeMap;

    fn timing(c: f64, p: f64) -> Timing {
        Timing {
            composer_seconds: c,
            phpm_seconds: p,
        }
    }

    fn rec(
        repo: &str,
        stars: u64,
        os: &str,
        identity: Identity,
        warm: Option<Timing>,
    ) -> ProjectRecord {
        ProjectRecord {
            group: "big-apps".to_owned(),
            repo: Some(repo.to_owned()),
            fixture: None,
            stars: Some(stars),
            os: os.to_owned(),
            packages: Some(1),
            cold: None,
            warm,
            noop: None,
            identity,
            differences: 0,
            fallback: false,
            fallback_plugins: vec![],
            error: None,
        }
    }

    #[test]
    fn summary_counts_identity_and_fallback() {
        let mut a = rec(
            "a/a",
            1,
            "ubuntu-latest",
            Identity::Identical,
            Some(timing(10.0, 2.0)),
        );
        a.fallback = true;
        let b = rec(
            "a/b",
            1,
            "ubuntu-latest",
            Identity::Different,
            Some(timing(8.0, 2.0)),
        );
        let c = rec("a/c", 1, "ubuntu-latest", Identity::InstallFailed, None);
        let s = Summary::of(&[a, b, c]);
        assert_eq!(
            (s.total, s.identical, s.different, s.install_failed),
            (3, 1, 1, 1)
        );
        assert_eq!(s.fallback, 1);
        assert!((s.identity_rate - 0.5).abs() < 1e-9);
        assert!((s.fallback_rate - (1.0 / 3.0)).abs() < 1e-9);
        let warm = s.warm.clone().unwrap();
        assert_eq!(warm.n, 2);
        assert!((warm.median_speedup - 4.5).abs() < 1e-9);
        assert!((warm.min_speedup - 4.0).abs() < 1e-9);
        assert!((warm.max_speedup - 5.0).abs() < 1e-9);
        assert!(Summary::of(&[]).warm.is_none());
        assert_eq!(s.headline(), "1 of 2 identical");
    }

    #[test]
    fn results_group_by_os_and_sort_by_stars() {
        let rs = Results::new(
            "2026-10-01",
            BTreeMap::new(),
            vec![
                rec(
                    "a/low",
                    1,
                    "ubuntu-latest",
                    Identity::Identical,
                    Some(timing(4.0, 1.0)),
                ),
                rec(
                    "a/high",
                    9,
                    "ubuntu-latest",
                    Identity::Identical,
                    Some(timing(8.0, 1.0)),
                ),
                rec("a/win", 5, "windows-latest", Identity::Different, None),
            ],
            &[],
            &[],
        );
        assert_eq!(rs.records[0].repo.as_deref(), Some("a/high"));
        assert_eq!(rs.summary["ubuntu-latest"].total, 2);
        assert_eq!(rs.summary["windows-latest"].different, 1);
        let top = rs.top_by_warm_speedup("ubuntu-latest", 1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].repo.as_deref(), Some("a/high"));
        assert!(rs.top_by_warm_speedup("windows-latest", 5).is_empty());
    }

    #[test]
    fn worktree_and_ci_summaries_split_by_os_and_project() {
        let wt = vec![WorktreeRecord {
            os: "ubuntu-latest".to_owned(),
            project: "laravel-skeleton".to_owned(),
            worktrees: 10,
            composer_total_seconds: 40.0,
            phpm_total_seconds: 4.0,
            composer_disk_bytes: 1_000_000,
            phpm_disk_bytes: 100_000,
        }];
        let ci = vec![
            CiScenarioRecord {
                os: "ubuntu-latest".to_owned(),
                project: "laravel-skeleton".to_owned(),
                cache: true,
                composer_job_seconds: 30.0,
                phpm_job_seconds: 10.0,
            },
            CiScenarioRecord {
                os: "ubuntu-latest".to_owned(),
                project: "laravel-skeleton".to_owned(),
                cache: false,
                composer_job_seconds: 60.0,
                phpm_job_seconds: 12.0,
            },
        ];
        let rs = Results::new("d", BTreeMap::new(), vec![], &wt, &ci);
        assert_eq!(rs.worktree.len(), 1);
        assert_eq!(rs.worktree[0].worktrees, 10);
        assert!((rs.worktree[0].time_speedup - 10.0).abs() < 1e-9);
        assert!((rs.worktree[0].disk_ratio - 10.0).abs() < 1e-9);
        assert_eq!(rs.ci_scenario.len(), 1);
        assert_eq!(rs.ci_scenario[0].with_cache_speedup, Some(3.0));
        assert_eq!(rs.ci_scenario[0].without_cache_speedup, Some(5.0));
    }

    #[test]
    fn history_keeps_one_entry_per_date_in_order() {
        let res = |d: &str, warm: Timing| {
            Results::new(
                d,
                BTreeMap::new(),
                vec![rec(
                    "a/a",
                    1,
                    "ubuntu-latest",
                    Identity::Identical,
                    Some(warm),
                )],
                &[],
                &[],
            )
        };
        let h = add_history(None, &res("2026-10-02", timing(4.0, 1.0)));
        let h = add_history(Some(&h.to_string()), &res("2026-10-01", timing(4.0, 1.0)));
        let h = add_history(Some(&h.to_string()), &res("2026-10-02", timing(8.0, 1.0)));
        let dates: Vec<&str> = h
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["date"].as_str().unwrap())
            .collect();
        assert_eq!(dates, ["2026-10-01", "2026-10-02"]);
        assert_eq!(h[1]["summary"]["ubuntu-latest"]["warm_median_speedup"], 8.0);
        assert_eq!(
            add_history(Some("not json"), &res("x", timing(1.0, 1.0)))
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn badge_colours_follow_the_speedup() {
        let with = |warm: Timing| {
            badge(&Results::new(
                "d",
                BTreeMap::new(),
                vec![rec(
                    "a/a",
                    1,
                    "ubuntu-latest",
                    Identity::Identical,
                    Some(warm),
                )],
                &[],
                &[],
            ))
        };
        assert_eq!(with(timing(20.0, 1.0))["color"], "brightgreen");
        assert_eq!(with(timing(5.0, 1.0))["color"], "yellow");
        assert_eq!(with(timing(2.0, 1.0))["color"], "red");
        let empty = Results::new("d", BTreeMap::new(), vec![], &[], &[]);
        assert_eq!(badge(&empty)["message"], "no data");
    }
}
