//! A port of the parts of `composer/semver`'s `VersionParser` (3.4.x, the
//! copy bundled in Composer 2.10) that installs need.
//!
//! Patterns run on bytes with Unicode off, like PCRE without `/u`.
//! Possessive quantifiers are written greedy: in every pattern here the next
//! token can never match what backtracking would give back.

use regex::bytes::{Captures, Regex};
use std::fmt;
use std::sync::LazyLock;

/// `VersionParser::DEFAULT_BRANCH_ALIAS`.
pub const DEFAULT_BRANCH_ALIAS: &str = "9999999-dev";

const MODIFIER: &str =
    r"[._-]?(?:(stable|beta|b|RC|alpha|a|patch|pl|p)((?:[.-]?\d+)*)?)?([.-]?dev)?";

fn re(pattern: &str) -> Regex {
    Regex::new(&format!("(?-u){pattern}")).expect("version patterns are valid")
}

static ALIAS: LazyLock<Regex> = LazyLock::new(|| re(r"^([^,\s]+) +as +([^,\s]+)$"));
static STABILITY_FLAG: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)@(?:stable|RC|beta|alpha|dev)$"));
static BUILD_METADATA: LazyLock<Regex> = LazyLock::new(|| re(r"^([^,\s+]+)\+[^\s]+$"));
static CLASSICAL: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?i)^v?(\d{{1,5}})(\.\d+)?(\.\d+)?(\.\d+)?{MODIFIER}$"
    ))
});
static DATETIME: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?i)^v?(\d{{4}}(?:[.:-]?\d{{2}}){{1,6}}(?:[.:-]?\d{{1,3}}){{0,2}}){MODIFIER}$"
    ))
});
static DEV_SUFFIX: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(.*?)[.-]?dev$"));
static BRANCH: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)^v?(\d+)(\.(?:\d+|[xX*]))?(\.(?:\d+|[xX*]))?(\.(?:\d+|[xX*]))?\n?\z")
});
static NUMERIC_ALIAS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^((?:\d+\.)*\d+)(?:\.x)?-dev\n?\z"));

/// A version string `normalize` rejects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidVersion(pub String);

impl fmt::Display for InvalidVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid version string \"{}\"", self.0)
    }
}

impl std::error::Error for InvalidVersion {}

fn group<'h>(caps: &Captures<'h>, i: usize) -> &'h str {
    caps.get(i)
        .and_then(|m| std::str::from_utf8(m.as_bytes()).ok())
        .unwrap_or("")
}

/// PHP `trim()` with its default character list.
pub(crate) fn php_trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\n', '\r', '\0', '\x0b'])
}

fn expand_stability(stability: &str) -> String {
    let lower = stability.to_ascii_lowercase();
    match lower.as_str() {
        "a" => "alpha".to_owned(),
        "b" => "beta".to_owned(),
        "p" | "pl" => "patch".to_owned(),
        "rc" => "RC".to_owned(),
        _ => lower,
    }
}

/// `VersionParser::normalize`.
pub fn normalize(version: &str) -> Result<String, InvalidVersion> {
    let orig = php_trim(version);
    let mut version = orig.to_owned();

    if let Some(caps) = ALIAS.captures(version.as_bytes()) {
        version = group(&caps, 1).to_owned();
    }
    if let Some(m) = STABILITY_FLAG.find(version.as_bytes()) {
        version.truncate(m.start());
    }
    if matches!(version.as_str(), "master" | "trunk" | "default") {
        version = format!("dev-{version}");
    }
    if version.len() >= 4 && version.as_bytes()[..4].eq_ignore_ascii_case(b"dev-") {
        return Ok(format!("dev-{}", &version[4..]));
    }
    if let Some(caps) = BUILD_METADATA.captures(version.as_bytes()) {
        version = group(&caps, 1).to_owned();
    }

    let matched = if let Some(caps) = CLASSICAL.captures(version.as_bytes()) {
        let part = |i| match group(&caps, i) {
            "" => ".0",
            p => p,
        };
        let base = format!("{}{}{}{}", group(&caps, 1), part(2), part(3), part(4));
        Some((base, caps, 5))
    } else if let Some(caps) = DATETIME.captures(version.as_bytes()) {
        let base = group(&caps, 1)
            .chars()
            .map(|c| if c.is_ascii_digit() { c } else { '.' })
            .collect();
        Some((base, caps, 2))
    } else {
        None
    };

    if let Some((mut base, caps, index)) = matched {
        let stability = group(&caps, index);
        if !stability.is_empty() {
            if stability == "stable" {
                return Ok(base);
            }
            base.push('-');
            base.push_str(&expand_stability(stability));
            base.push_str(group(&caps, index + 1).trim_start_matches(['.', '-']));
        }
        if !group(&caps, index + 2).is_empty() {
            base.push_str("-dev");
        }
        return Ok(base);
    }

    if let Some(caps) = DEV_SUFFIX.captures(version.as_bytes()) {
        let normalized = normalize_branch(group(&caps, 1));
        if !normalized.contains("dev-") {
            return Ok(normalized);
        }
    }

    Err(InvalidVersion(orig.to_owned()))
}

/// `VersionParser::normalizeBranch`.
pub fn normalize_branch(name: &str) -> String {
    let name = php_trim(name);
    if let Some(caps) = BRANCH.captures(name.as_bytes()) {
        let mut version = String::new();
        for i in 1..5 {
            match caps.get(i) {
                Some(_) => version.push_str(&group(&caps, i).replace(['*', 'X'], "x")),
                None => version.push_str(".x"),
            }
        }
        return format!("{}-dev", version.replace('x', "9999999"));
    }
    format!("dev-{name}")
}

/// `VersionParser::normalizeDefaultBranch`.
pub fn normalize_default_branch(name: &str) -> String {
    if matches!(name, "dev-master" | "dev-default" | "dev-trunk") {
        DEFAULT_BRANCH_ALIAS.to_owned()
    } else {
        name.to_owned()
    }
}

/// `VersionParser::parseNumericAliasPrefix`: `2.1.x-dev` gives `2.1.`.
pub fn parse_numeric_alias_prefix(branch: &str) -> Option<String> {
    NUMERIC_ALIAS
        .captures(branch.as_bytes())
        .map(|caps| format!("{}.", group(&caps, 1)))
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_BRANCH_ALIAS, InvalidVersion, normalize, normalize_branch,
        normalize_default_branch, parse_numeric_alias_prefix,
    };
    use serde_json::Value;

    const CASES: &str = include_str!("../tests/data/version-parser.jsonl");

    #[test]
    fn matches_composer_on_every_recorded_case() {
        let mut checked = 0;
        for line in CASES.lines() {
            let row: Vec<Value> = serde_json::from_str(line).unwrap();
            let input = row[0].as_str().unwrap();
            let expected_normalized = row[1].as_str();
            assert_eq!(
                normalize(input).ok().as_deref(),
                expected_normalized,
                "normalize({input:?})"
            );
            assert_eq!(
                normalize_branch(input),
                row[2].as_str().unwrap(),
                "normalizeBranch({input:?})"
            );
            assert_eq!(
                parse_numeric_alias_prefix(input).as_deref(),
                row[3].as_str(),
                "parseNumericAliasPrefix({input:?})"
            );
            checked += 1;
        }
        assert!(checked > 500);
    }

    #[test]
    fn upstream_examples() {
        assert_eq!(normalize("1.0.0RC1dev").unwrap(), "1.0.0.0-RC1-dev");
        assert_eq!(normalize("dev-master as 1.0.0").unwrap(), "dev-master");
        assert_eq!(
            normalize("20100102-203040-p1").unwrap(),
            "20100102.203040-patch1"
        );
        assert_eq!(normalize("1.0.0-STABLE").unwrap(), "1.0.0.0-stable");
        assert_eq!(normalize("master").unwrap(), "dev-master");
        assert_eq!(normalize_branch("v1.0.3.*"), "1.0.3.9999999-dev");
        assert_eq!(normalize_branch("feature+issue-1"), "dev-feature+issue-1");
        assert_eq!(
            parse_numeric_alias_prefix("1.2.x-dev").as_deref(),
            Some("1.2.")
        );
        assert_eq!(
            parse_numeric_alias_prefix("1.2.x-dev\n").as_deref(),
            Some("1.2.")
        );
        assert_eq!(parse_numeric_alias_prefix("dev-master"), None);
    }

    #[test]
    fn rejects_invalid_versions() {
        let err = normalize(" 1.0.0-meh ").unwrap_err();
        assert_eq!(err, InvalidVersion("1.0.0-meh".into()));
        assert_eq!(err.to_string(), "Invalid version string \"1.0.0-meh\"");
        assert!(normalize("").is_err());
        assert!(normalize("feature-foo").is_err());
    }

    #[test]
    fn default_branch_names_map_to_the_alias() {
        assert_eq!(normalize_default_branch("dev-master"), DEFAULT_BRANCH_ALIAS);
        assert_eq!(
            normalize_default_branch("dev-default"),
            DEFAULT_BRANCH_ALIAS
        );
        assert_eq!(normalize_default_branch("dev-trunk"), DEFAULT_BRANCH_ALIAS);
        assert_eq!(normalize_default_branch("dev-main"), "dev-main");
    }
}
