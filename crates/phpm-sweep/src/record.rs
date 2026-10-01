//! One project's result, as the sweep writes it.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Which flags both tools get.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// `--no-scripts --no-plugins --ignore-platform-reqs`: phpm's own install.
    Pure,
    /// `--ignore-platform-reqs`: what a user gets, Composer fallback included.
    Default,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pure" => Some(Self::Pure),
            "default" => Some(Self::Default),
            _ => None,
        }
    }

    pub fn flags(self) -> &'static [&'static str] {
        match self {
            Self::Pure => &["--no-scripts", "--no-plugins", "--ignore-platform-reqs"],
            Self::Default => &["--ignore-platform-reqs"],
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pure => "pure",
            Self::Default => "default",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Identical,
    Different,
    PhpmFailed,
    /// Composer could not install it either; left out of the ratio.
    ComposerFailed,
    /// The project's files could not be fetched at the pin; left out too.
    FetchFailed,
}

impl Outcome {
    pub fn installable(self) -> bool {
        !matches!(self, Self::ComposerFailed | Self::FetchFailed)
    }
}

/// What phpm did with the install, from `--explain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhpmPath {
    Native,
    /// Some steps went to Composer.
    Partial,
    /// The whole install went to Composer.
    Full,
    Unknown,
}

/// One tool's run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub exit: Option<i32>,
    pub seconds: f64,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
}

impl Run {
    pub fn ok(&self) -> bool {
        self.exit == Some(0) && !self.timed_out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub repo: Option<String>,
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture: Option<String>,
    #[serde(default)]
    pub stars: Option<u64>,
    pub mode: Mode,
    pub os: String,
    pub outcome: Outcome,
    #[serde(default)]
    pub packages: Option<usize>,
    #[serde(default)]
    pub compared: Vec<String>,
    #[serde(default)]
    pub differences: usize,
    #[serde(default)]
    pub first_differences: Vec<String>,
    /// Differences that come from Composer itself and were not counted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normalized: Vec<String>,
    pub phpm_path: PhpmPath,
    #[serde(default)]
    pub phpm_reasons: Vec<String>,
    #[serde(default)]
    pub composer: Option<Run>,
    #[serde(default)]
    pub phpm: Option<Run>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Record {
    /// How the project is named on the page and in logs.
    pub fn name(&self) -> String {
        match (&self.repo, &self.fixture) {
            (Some(repo), _) => repo.clone(),
            (None, Some(f)) => format!("fixtures/{f}"),
            (None, None) => "?".to_owned(),
        }
    }
}

/// The path phpm took and why, from the `decision` lines `--explain` prints.
pub fn phpm_path(stderr: &str) -> (PhpmPath, Vec<String>) {
    let mut path = PhpmPath::Unknown;
    let mut reasons = Vec::new();
    for line in stderr.lines() {
        let Some(rest) = line.trim().strip_prefix("decision ") else {
            continue;
        };
        let Some((step, what)) = rest.split_once(": ") else {
            continue;
        };
        if step == "install" {
            if let Some(why) = what.strip_prefix("fallback to composer install, ") {
                return (PhpmPath::Full, vec![why.to_owned()]);
            }
            path = PhpmPath::Native;
        } else if let Some(why) = what.strip_prefix("fallback to Composer, ") {
            reasons.push(format!("{step}: {why}"));
        }
    }
    if path == PhpmPath::Native && !reasons.is_empty() {
        path = PhpmPath::Partial;
    }
    (path, reasons)
}

/// The last `lines` lines of `text`, at most `max` bytes of them.
pub fn tail(text: &str, lines: usize, max: usize) -> String {
    let all: Vec<&str> = text.trim_end().lines().collect();
    let joined = all[all.len().saturating_sub(lines)..].join("\n");
    if joined.len() <= max {
        return joined;
    }
    let mut start = joined.len() - max;
    while !joined.is_char_boundary(start) {
        start += 1;
    }
    joined[start..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::{Mode, Outcome, PhpmPath, Record, Run, phpm_path, tail};
    use proptest::prelude::*;

    #[test]
    fn reads_a_native_install() {
        let err = "package a/b 1.0.0: native, placed from the store\n\
                   decision plugins: none loaded, --no-plugins\n\
                   decision install: native, phpm fetches and places every package\n\
                   decision autoload: native, phpm writes the autoloader\n\
                   decision post-install-cmd: skipped, --no-scripts\n";
        assert_eq!(phpm_path(err), (PhpmPath::Native, vec![]));
    }

    #[test]
    fn reads_a_partial_fallback_with_its_reasons() {
        let err = "decision install: native, phpm fetches and places every package\n\
                   decision autoload: fallback to Composer, plugins hook it\n\
                   decision post-autoload-dump: fallback to Composer, a PHP callable\n";
        assert_eq!(
            phpm_path(err),
            (
                PhpmPath::Partial,
                vec![
                    "autoload: plugins hook it".to_owned(),
                    "post-autoload-dump: a PHP callable".to_owned()
                ]
            )
        );
    }

    #[test]
    fn reads_a_full_fallback() {
        let err = "decision plugins: composer/installers allowed\n\
                   decision install: fallback to composer install, composer/installers places packages\n";
        assert_eq!(
            phpm_path(err),
            (
                PhpmPath::Full,
                vec!["composer/installers places packages".to_owned()]
            )
        );
    }

    #[test]
    fn a_no_op_is_native_and_no_decisions_is_unknown() {
        let noop = "decision install: nothing to do, composer.json, composer.lock and vendor/ are as phpm left them";
        assert_eq!(phpm_path(noop).0, PhpmPath::Native);
        assert_eq!(phpm_path("error: bad lock").0, PhpmPath::Unknown);
        assert_eq!(phpm_path("decision nothing").0, PhpmPath::Unknown);
    }

    #[test]
    fn tails_by_lines_then_bytes() {
        assert_eq!(tail("a\nb\nc\n", 2, 100), "b\nc");
        assert_eq!(tail("abcdef", 5, 3), "def");
        assert_eq!(tail("ééé", 5, 3), "é");
        assert_eq!(tail("", 5, 3), "");
    }

    #[test]
    fn modes_parse_and_print() {
        assert_eq!(Mode::parse("pure"), Some(Mode::Pure));
        assert_eq!(Mode::parse("default"), Some(Mode::Default));
        assert_eq!(Mode::parse("other"), None);
        assert_eq!(Mode::Pure.to_string(), "pure");
        assert_eq!(Mode::Default.to_string(), "default");
        assert!(Mode::Pure.flags().contains(&"--no-plugins"));
        assert_eq!(Mode::Default.flags(), &["--ignore-platform-reqs"]);
    }

    #[test]
    fn composer_failures_are_not_installable() {
        assert!(Outcome::Identical.installable());
        assert!(Outcome::PhpmFailed.installable());
        assert!(!Outcome::ComposerFailed.installable());
        assert!(!Outcome::FetchFailed.installable());
    }

    #[test]
    fn a_run_is_ok_only_with_exit_zero_in_time() {
        let run = |exit, timed_out| Run {
            exit,
            seconds: 1.0,
            timed_out,
            stderr_tail: None,
        };
        assert!(run(Some(0), false).ok());
        assert!(!run(Some(1), false).ok());
        assert!(!run(None, true).ok());
    }

    #[test]
    fn records_round_trip_as_kebab_case_json() {
        let r = Record {
            repo: None,
            commit: None,
            fixture: Some("ours".to_owned()),
            stars: None,
            mode: Mode::Pure,
            os: "linux".to_owned(),
            outcome: Outcome::ComposerFailed,
            packages: Some(3),
            compared: vec![],
            differences: 0,
            first_differences: vec![],
            normalized: vec![],
            phpm_path: PhpmPath::Unknown,
            phpm_reasons: vec![],
            composer: None,
            phpm: None,
            error: None,
        };
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains(r#""outcome":"composer-failed""#), "{text}");
        assert!(!text.contains("error"), "{text}");
        assert_eq!(serde_json::from_str::<Record>(&text).unwrap(), r);
        assert_eq!(r.name(), "fixtures/ours");
        let named = Record {
            repo: Some("a/b".to_owned()),
            ..r.clone()
        };
        assert_eq!(named.name(), "a/b");
        assert_eq!(Record { fixture: None, ..r }.name(), "?");
    }

    proptest! {
        #[test]
        fn explain_parsing_never_panics(s in ".{0,200}") {
            let _ = phpm_path(&s);
        }

        #[test]
        fn tails_stay_within_bounds(s in ".{0,200}", lines in 0_usize..10, max in 0_usize..50) {
            let t = tail(&s, lines, max);
            prop_assert!(t.len() <= max);
        }
    }
}
