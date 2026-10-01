//! The three record shapes `tools/bench-real/*` emits as JSON Lines, and
//! the corpus they were run from.

use serde::{Deserialize, Serialize};

/// One project in `bench-real/corpus.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub group: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub fixture: Option<String>,
    #[serde(default)]
    pub stars: Option<u64>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl ProjectEntry {
    /// How the project is named on the page and in the README.
    pub fn name(&self) -> String {
        match (&self.repo, &self.fixture) {
            (Some(repo), _) => repo.clone(),
            (None, Some(f)) => format!("fixtures/{f}"),
            (None, None) => "?".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Excluded {
    pub repo: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Corpus {
    pub taken: String,
    #[serde(default)]
    pub selection: String,
    pub projects: Vec<ProjectEntry>,
    #[serde(default)]
    pub excluded: Vec<Excluded>,
}

impl Corpus {
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }
}

/// Composer vs phpm, in seconds, for one scenario.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Timing {
    pub composer_seconds: f64,
    pub phpm_seconds: f64,
}

impl Timing {
    /// Composer time over phpm time; `None` when phpm measured as zero
    /// (can't divide) or either side is not a finite positive number.
    pub fn speedup(&self) -> Option<f64> {
        if !self.composer_seconds.is_finite()
            || !self.phpm_seconds.is_finite()
            || self.composer_seconds <= 0.0
            || self.phpm_seconds <= 0.0
        {
            return None;
        }
        Some(self.composer_seconds / self.phpm_seconds)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Identity {
    Identical,
    Different,
    /// phpm or Composer failed to install; no `vendor/` to compare.
    InstallFailed,
}

/// One project, on one OS, from `tools/bench-real/run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub group: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub fixture: Option<String>,
    #[serde(default)]
    pub stars: Option<u64>,
    pub os: String,
    #[serde(default)]
    pub packages: Option<usize>,
    #[serde(default)]
    pub cold: Option<Timing>,
    #[serde(default)]
    pub warm: Option<Timing>,
    #[serde(default)]
    pub noop: Option<Timing>,
    pub identity: Identity,
    #[serde(default)]
    pub differences: usize,
    #[serde(default)]
    pub fallback: bool,
    #[serde(default)]
    pub fallback_plugins: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ProjectRecord {
    pub fn name(&self) -> String {
        match (&self.repo, &self.fixture) {
            (Some(repo), _) => repo.clone(),
            (None, Some(f)) => format!("fixtures/{f}"),
            (None, None) => "?".to_owned(),
        }
    }

    /// The three named scenarios in display order, skipping ones not run.
    pub fn scenarios(&self) -> Vec<(&'static str, Timing)> {
        [
            ("cold", self.cold),
            ("warm", self.warm),
            ("noop", self.noop),
        ]
        .into_iter()
        .filter_map(|(name, t)| t.map(|t| (name, t)))
        .collect()
    }
}

/// Ten worktrees of one project: Composer's ten full `vendor/` copies
/// against phpm cloning/hardlinking from one warm store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorktreeRecord {
    pub os: String,
    pub project: String,
    pub worktrees: usize,
    pub composer_total_seconds: f64,
    pub phpm_total_seconds: f64,
    pub composer_disk_bytes: u64,
    pub phpm_disk_bytes: u64,
}

impl WorktreeRecord {
    pub fn time_speedup(&self) -> Option<f64> {
        Timing {
            composer_seconds: self.composer_total_seconds,
            phpm_seconds: self.phpm_total_seconds,
        }
        .speedup()
    }

    /// How many times smaller phpm's total disk footprint is; `None` when
    /// phpm used zero bytes (can't divide) or Composer's side is zero.
    #[allow(
        clippy::cast_precision_loss,
        reason = "disk byte counts stay well under 2^52; the ratio is a human-facing number, not an exact one"
    )]
    pub fn disk_ratio(&self) -> Option<f64> {
        if self.composer_disk_bytes == 0 || self.phpm_disk_bytes == 0 {
            return None;
        }
        Some(self.composer_disk_bytes as f64 / self.phpm_disk_bytes as f64)
    }
}

/// A project's own CI install step, simulated with and without
/// `actions/cache`, before and after phpm: total job time, not just the
/// install line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CiScenarioRecord {
    pub os: String,
    pub project: String,
    pub cache: bool,
    pub composer_job_seconds: f64,
    pub phpm_job_seconds: f64,
}

impl CiScenarioRecord {
    pub fn speedup(&self) -> Option<f64> {
        Timing {
            composer_seconds: self.composer_job_seconds,
            phpm_seconds: self.phpm_job_seconds,
        }
        .speedup()
    }
}

/// Records read from JSON Lines, one per line; blank lines skipped.
pub fn parse_lines<T: for<'de> Deserialize<'de>>(text: &str) -> Result<Vec<T>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("line {}: {e}", i + 1)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        CiScenarioRecord, Corpus, Identity, ProjectEntry, ProjectRecord, Timing, WorktreeRecord,
        parse_lines,
    };

    fn record(name: &str, cold: Option<Timing>, warm: Option<Timing>) -> ProjectRecord {
        ProjectRecord {
            group: "big-apps".to_owned(),
            repo: Some(name.to_owned()),
            fixture: None,
            stars: Some(10),
            os: "ubuntu-latest".to_owned(),
            packages: Some(5),
            cold,
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
    fn speedup_is_composer_over_phpm() {
        let t = Timing {
            composer_seconds: 10.0,
            phpm_seconds: 2.0,
        };
        assert_eq!(t.speedup(), Some(5.0));
    }

    #[test]
    fn speedup_is_none_for_non_positive_or_non_finite_times() {
        let bad = [
            (0.0, 1.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (f64::NAN, 1.0),
            (f64::INFINITY, 1.0),
        ];
        for (c, p) in bad {
            let t = Timing {
                composer_seconds: c,
                phpm_seconds: p,
            };
            assert_eq!(t.speedup(), None, "{c} {p}");
        }
    }

    #[test]
    fn project_entry_names_a_repo_or_a_fixture() {
        let repo = ProjectEntry {
            group: "own".to_owned(),
            repo: Some("a/b".to_owned()),
            commit: None,
            fixture: Some("ignored".to_owned()),
            stars: None,
            notes: None,
        };
        assert_eq!(repo.name(), "a/b");
        let fixture = ProjectEntry { repo: None, ..repo };
        assert_eq!(fixture.name(), "fixtures/ignored");
        let neither = ProjectEntry {
            fixture: None,
            ..fixture
        };
        assert_eq!(neither.name(), "?");
    }

    #[test]
    fn project_record_lists_only_the_scenarios_it_ran() {
        let warm = Timing {
            composer_seconds: 4.0,
            phpm_seconds: 1.0,
        };
        let r = record("a/a", None, Some(warm));
        assert_eq!(r.scenarios(), vec![("warm", warm)]);
        assert_eq!(r.name(), "a/a");
        let fixture = ProjectRecord {
            repo: None,
            fixture: Some("ytmate".to_owned()),
            ..r
        };
        assert_eq!(fixture.name(), "fixtures/ytmate");
    }

    #[test]
    fn worktree_speedups_and_disk_ratio() {
        let w = WorktreeRecord {
            os: "ubuntu-latest".to_owned(),
            project: "laravel-skeleton".to_owned(),
            worktrees: 10,
            composer_total_seconds: 40.0,
            phpm_total_seconds: 2.0,
            composer_disk_bytes: 100_000_000,
            phpm_disk_bytes: 10_000_000,
        };
        assert_eq!(w.time_speedup(), Some(20.0));
        assert_eq!(w.disk_ratio(), Some(10.0));
        let zero = WorktreeRecord {
            phpm_disk_bytes: 0,
            ..w
        };
        assert_eq!(zero.disk_ratio(), None);
    }

    #[test]
    fn ci_scenario_speedup() {
        let c = CiScenarioRecord {
            os: "ubuntu-latest".to_owned(),
            project: "laravel-skeleton".to_owned(),
            cache: true,
            composer_job_seconds: 30.0,
            phpm_job_seconds: 10.0,
        };
        assert_eq!(c.speedup(), Some(3.0));
    }

    #[test]
    fn corpus_parses_projects_and_excluded() {
        let text = r#"{"taken":"2026-10-01","selection":"s","projects":[
            {"group":"own","fixture":"ytmate"}
        ],"excluded":[{"repo":"a/b","reason":"no lock"}]}"#;
        let c = Corpus::parse(text).unwrap();
        assert_eq!(c.projects.len(), 1);
        assert_eq!(c.excluded[0].repo, "a/b");
        assert!(Corpus::parse("{").is_err());
    }

    #[test]
    fn reads_json_lines_and_skips_blanks() {
        let a = record("a/a", None, None);
        let b = record("a/b", None, None);
        let text = format!(
            "{}\n\n{}\n",
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        let out: Vec<ProjectRecord> = parse_lines(&text).unwrap();
        assert_eq!(out.len(), 2);
        assert!(
            parse_lines::<ProjectRecord>("{")
                .unwrap_err()
                .starts_with("line 1:")
        );
    }
}
