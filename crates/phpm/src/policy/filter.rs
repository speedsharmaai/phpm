//! The malware filter Composer 2.10 applies to a lock install: locked
//! packages flagged by a repository's filter lists are dropped from the
//! pool, and the install fails as an unsatisfiable lock.
//!
//! Composer: DependencyResolver/FilterListPoolFilter.php,
//! FilterList/FilterListProvider/FilterListProviderSet.php,
//! FilterList/FilterListAuditor.php, Repository/ComposerRepository.php getFilter.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use phpm_lock::constraint::{Constraint, Op, parse};
use serde_json::{Value, json};

use super::repo::{Cached, Client, ROOT_MAX_AGE, Repo, Unreachable, now};
use super::{IgnoreRule, Policy};
use crate::error::Error;
use crate::platform::is_platform_package;

/// A locked package as the filter sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Locked {
    pub(crate) name: String,
    pub(crate) pretty_name: String,
    pub(crate) pretty: String,
    pub(crate) version: String,
}

/// One filter list entry (`FilterListEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) package: String,
    pub(crate) list: String,
    pub(crate) constraint: Constraint,
    pub(crate) pretty_constraint: String,
    pub(crate) url: Option<String>,
    pub(crate) reason: Option<String>,
    pub(crate) id: Option<String>,
    pub(crate) source: Option<String>,
}

/// What the filter found, beyond any block: when its inputs go stale, and
/// warnings about sources it could not reach.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) expires: Option<u64>,
    pub(crate) warnings: Vec<String>,
}

// Composer: Policy/PolicyConfig.php RESERVED_NAMES, FUTURE_RESERVED_*
const RESERVED: [&str; 12] = [
    "advisories",
    "abandoned",
    "package",
    "packages",
    "license",
    "licence",
    "licenses",
    "licences",
    "support",
    "maintenance",
    "security",
    "minimum-release-age",
];

/// A repository's filter lists and where to read them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Advertised {
    lists: Vec<String>,
    summary_url: Option<String>,
    api_url: Option<String>,
    metadata_url: Option<String>,
}

/// `ComposerRepositoryFilterInformation::fromData` plus the repo's own skips.
fn advertised(repo: &Repo, root: &Value) -> Option<Advertised> {
    let filter = root.get("filter")?;
    if !filter
        .get("metadata")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let lists = filter
        .get("lists")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, c)| c.get("enabled").and_then(Value::as_bool).unwrap_or(false))
        .map(|(n, _)| n.clone())
        .filter(|n| {
            !RESERVED.contains(&n.as_str())
                && !n.starts_with("ignore")
                && !repo.skip_lists.contains(n)
        })
        .collect();
    let url = |k: &str| {
        filter
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| repo.canonical(s))
    };
    let metadata_url = root
        .get("metadata-url")
        .and_then(Value::as_str)
        .map(|s| repo.canonical(s));
    Some(Advertised {
        lists,
        summary_url: url("summary-url"),
        api_url: url("api-url"),
        metadata_url,
    })
}

fn exact(version: &str) -> Constraint {
    Constraint::Single {
        op: Op::Eq,
        version: version.to_owned(),
    }
}

/// `MultiConstraint::create` over the locked versions of each name.
fn constraint_map(packages: &[Locked]) -> BTreeMap<String, Constraint> {
    let mut versions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in packages.iter().filter(|p| !is_platform_package(&p.name)) {
        let list = versions.entry(p.name.clone()).or_default();
        if !list.contains(&p.version) {
            list.push(p.version.clone());
        }
    }
    versions
        .into_iter()
        .map(|(name, mut list)| {
            let c = if list.len() == 1 {
                exact(&list.remove(0))
            } else {
                Constraint::Multi {
                    conjunctive: false,
                    items: list.iter().map(|v| exact(v)).collect(),
                }
            };
            (name, c)
        })
        .collect()
}

// Composer: FilterList/FilterListEntryBuilder.php build
fn build(
    raw: &Value,
    map: &BTreeMap<String, Constraint>,
    default: Option<&str>,
    lists: &[String],
) -> Vec<Entry> {
    let mut out = Vec::new();
    for (list, entries) in raw.as_object().into_iter().flatten() {
        if !lists.contains(list) {
            continue;
        }
        for data in entries.as_array().into_iter().flatten() {
            let Some(pretty) = data.get("constraint").and_then(Value::as_str) else {
                continue;
            };
            let Some(package) = data.get("package").and_then(Value::as_str).or(default) else {
                continue;
            };
            let Ok(constraint) = parse(pretty) else {
                continue;
            };
            let Some(locked) = map.get(package) else {
                continue;
            };
            if !constraint.matches(locked) {
                continue;
            }
            let text = |k: &str| data.get(k).and_then(Value::as_str).map(str::to_owned);
            out.push(Entry {
                package: package.to_owned(),
                list: list.clone(),
                constraint,
                pretty_constraint: pretty.to_owned(),
                url: text("url"),
                reason: text("reason"),
                id: text("id"),
                source: text("source"),
            });
        }
    }
    out
}

fn expiry(cached: &Cached, fallback: u64) -> u64 {
    now() + cached.max_age.unwrap_or(fallback)
}

/// `name version` for each locked package the lists are asked about.
fn keys(packages: &[Locked]) -> Vec<String> {
    let set: BTreeSet<String> = packages
        .iter()
        .filter(|p| !is_platform_package(&p.name))
        .map(|p| format!("{} {}", p.name, p.version))
        .collect();
    set.into_iter().collect()
}

/// Every entry the repositories have for the locked packages on `lists`.
pub(crate) async fn collect(
    client: &Client<'_>,
    repos: &[Repo],
    packages: &[Locked],
    lists: &[String],
    ignore_unreachable: bool,
) -> Result<(Vec<Entry>, Outcome), Error> {
    let map = constraint_map(packages);
    let keys = keys(packages);
    let mut outcome = Outcome::default();
    let mut entries = Vec::new();
    let unreachable = |e: Unreachable, outcome: &mut Outcome| {
        if ignore_unreachable {
            outcome.warnings.push(e);
            Ok(())
        } else {
            Err(Error::install(e))
        }
    };
    let mut expires: Vec<u64> = Vec::new();
    for repo in repos.iter().filter(|r| !r.filter_off) {
        let root = match client.root(repo).await {
            Ok(Some(root)) => root,
            Ok(None) => continue,
            Err(e) => {
                unreachable(e, &mut outcome)?;
                continue;
            }
        };
        let root_until = root.checked + ROOT_MAX_AGE;
        expires.push(root_until);
        let Some(Advertised {
            lists: advertised,
            summary_url,
            api_url,
            metadata_url,
        }) = advertised(repo, &root.data)
        else {
            continue;
        };
        let relevant: Vec<String> = lists
            .iter()
            .filter(|l| advertised.contains(l))
            .cloned()
            .collect();
        if relevant.is_empty() || map.is_empty() {
            continue;
        }
        if let Some(api) = api_url {
            let purls: Vec<String> = map.keys().map(|n| format!("pkg://composer/{n}")).collect();
            let body = json!({"packages": purls, "lists": relevant})
                .to_string()
                .into_bytes();
            match client
                .fetcher
                .request(&api, None, Some(("application/json", body)))
                .await
            {
                Ok(r) if r.status == 200 => {
                    let decoded: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
                    match decoded.get("filter") {
                        Some(raw) => entries.extend(build(raw, &map, None, &relevant)),
                        None => unreachable(
                            format!("Filter api-url {api} returned an unexpected response"),
                            &mut outcome,
                        )?,
                    }
                }
                Ok(r) => unreachable(
                    format!("Filter api-url {api} returned HTTP {}", r.status),
                    &mut outcome,
                )?,
                Err(e) => unreachable(e.to_string(), &mut outcome)?,
            }
            continue;
        }
        let mut candidates: Vec<&String> = map.keys().collect();
        let mut fresh_until = None;
        if let Some(url) = summary_url {
            if let Some(until) = client.verified(repo, &relevant, &keys) {
                expires.push(until);
                continue;
            }
            let summary = match client.json(repo, "filter-summary.json", &url, None).await {
                Ok(Some(s)) => s,
                Ok(None) => {
                    unreachable(
                        format!("Filter summary URL {url} returned 404"),
                        &mut outcome,
                    )?;
                    continue;
                }
                Err(e) => {
                    unreachable(e, &mut outcome)?;
                    continue;
                }
            };
            let until = expiry(&summary, 0);
            expires.push(until);
            fresh_until = Some(until.min(root_until));
            let by_list = summary.data.get("filter").and_then(Value::as_object);
            candidates.retain(|name| {
                relevant.iter().any(|list| {
                    by_list
                        .and_then(|l| l.get(list))
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                        .any(|(pkg, c)| {
                            pkg.eq_ignore_ascii_case(name)
                                && c.as_str()
                                    .and_then(|c| parse(c).ok())
                                    .is_some_and(|c| c.matches(&map[*name]))
                        })
                })
            });
        }
        let first = entries.len();
        let mut reached = true;
        let lookups: Vec<(&String, String)> = metadata_url
            .map(|t| {
                candidates
                    .iter()
                    .map(|n| (*n, t.replace("%package%", n)))
                    .collect()
            })
            .unwrap_or_default();
        for (name, url) in lookups {
            let key = format!("provider-{}.json", name.replace('/', "~"));
            match client.json(repo, &key, &url, None).await {
                Ok(Some(meta)) => {
                    if let Some(raw) = meta.data.get("filter") {
                        entries.extend(build(raw, &map, Some(name), &relevant));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    reached = false;
                    unreachable(e, &mut outcome)?;
                }
            }
        }
        if let Some(until) = fresh_until.filter(|_| reached) {
            let flagged: BTreeSet<&str> = entries[first..]
                .iter()
                .map(|e| e.package.as_str())
                .collect();
            let clean: Vec<String> = keys
                .iter()
                .filter(|k| !k.split(' ').next().is_some_and(|n| flagged.contains(n)))
                .cloned()
                .collect();
            client.record(repo, &relevant, until, &clean);
        }
    }
    outcome.expires = expires.into_iter().min();
    Ok((entries, outcome))
}

// Composer: FilterList/FilterListAuditor.php matchingEntries
pub(crate) fn matching<'e>(
    package: &Locked,
    entries: &'e [Entry],
    ignore: &[IgnoreRule],
    ignore_source: &[String],
    block: bool,
) -> Vec<&'e Entry> {
    let ignored = ignore
        .iter()
        .any(|r| r.applies(&package.name, &package.version, block));
    if ignored {
        return Vec::new();
    }
    let version = exact(&package.version);
    entries
        .iter()
        .filter(|e| e.package == package.name)
        .filter(|e| {
            e.list != "malware" || e.source.as_ref().is_none_or(|s| !ignore_source.contains(s))
        })
        .filter(|e| e.constraint.matches(&version))
        .collect()
}

// Composer: DependencyResolver/Pool.php getFilterListEntryForPackageVersion,
// Problem.php getMissingLockedPackageReason
fn reason(package: &Locked, hits: &[&Entry]) -> String {
    let mut lists: Vec<(String, Vec<String>)> = Vec::new();
    for e in hits {
        let source = e
            .source
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!(" reported by {s}"))
            .unwrap_or_default();
        let url = e
            .url
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!(" (see {s})"))
            .unwrap_or_default();
        let why = e
            .reason
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!(" reason: {s}"))
            .unwrap_or_default();
        let text = format!("{source}{url}{why}");
        match lists.iter_mut().find(|(l, _)| *l == e.list) {
            Some((_, texts)) => texts.push(text),
            None => lists.push((e.list.clone(), vec![text])),
        }
    }
    let filters: Vec<String> = lists
        .iter()
        .map(|(list, texts)| {
            let action = if list == "malware" {
                "flagged as "
            } else {
                "filtered by "
            };
            format!("{action}{list}{}", texts.join(", "))
        })
        .collect();
    let paths = |what: &str| {
        lists
            .iter()
            .map(|(l, _)| format!("\"policy.{l}.{what}\""))
            .collect::<Vec<_>>()
            .join(" and ")
    };
    format!(
        "- Package {} {} (in the lock file) was not loaded, because it was {}. To ignore filters for this package, add the package to the {} config. To turn the feature off entirely, you can set {} to false.",
        package.name,
        package.pretty,
        filters.join(", "),
        paths("ignore"),
        paths("block")
    )
}

/// An install the filter stopped, and the packages it blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) error: Error,
    pub(crate) blocked: Vec<String>,
}

impl From<Error> for Refusal {
    fn from(error: Error) -> Self {
        Self {
            error,
            blocked: Vec::new(),
        }
    }
}

/// Run the malware filter for an install; a refusal carries Composer's
/// message and its exit code 2.
pub(crate) async fn check_install(
    client: &Client<'_>,
    repos: &[Repo],
    policy: &Policy,
    packages: &[Locked],
) -> Result<Outcome, Refusal> {
    if !policy.blocks_install() {
        return Ok(Outcome::default());
    }
    let lists = vec!["malware".to_owned()];
    let (entries, mut outcome) =
        collect(client, repos, packages, &lists, policy.unreachable_install).await?;
    if !outcome.warnings.is_empty() {
        let mut lines = vec!["Filter list data could not be fetched from some sources (ignored per policy.ignore-unreachable); matches may be incomplete:".to_owned()];
        lines.extend(outcome.warnings.iter().map(|w| format!("  - {w}")));
        outcome.warnings = lines;
    }
    let mut problems = Vec::new();
    let mut blocked = Vec::new();
    for package in packages.iter().filter(|p| !is_platform_package(&p.name)) {
        let hits = matching(
            package,
            &entries,
            &policy.malware.ignore,
            &policy.malware.ignore_source,
            true,
        );
        if !hits.is_empty() {
            problems.push(reason(package, &hits));
            blocked.push(package.name.clone());
        }
    }
    if problems.is_empty() {
        return Ok(outcome);
    }
    let mut text = "Your lock file does not contain a compatible set of packages. Please run composer update.\n".to_owned();
    for (i, p) in problems.iter().enumerate() {
        let _ = write!(text, "\n  Problem {}\n    {p}", i + 1);
    }
    Err(Refusal {
        error: Error::new(2, text),
        blocked,
    })
}

#[cfg(test)]
mod tests {
    use super::{Entry, Locked, advertised, build, constraint_map, matching, reason};
    use crate::policy::IgnoreRule;
    use crate::policy::repo::Repo;
    use phpm_lock::constraint::Constraint;
    use serde_json::json;

    fn locked(name: &str, version: &str) -> Locked {
        Locked {
            name: name.into(),
            pretty_name: name.into(),
            pretty: version.into(),
            version: format!("{version}.0"),
        }
    }

    #[test]
    fn reads_what_a_repository_advertises() {
        let repo = Repo {
            url: "https://r.test".into(),
            filter_off: false,
            skip_lists: vec!["skipped".into()],
        };
        let root = json!({
            "metadata-url": "/p2/%package%.json",
            "filter": {"metadata": true, "summary-url": "/s.json", "api-url": "",
                "lists": {"malware": {"enabled": true}, "advisories": {"enabled": true}, "ignore-me": {"enabled": true},
                          "off": {"enabled": false}, "skipped": {"enabled": true}, "security": {"enabled": true}, "custom": {"enabled": true}}},
        });
        let found = advertised(&repo, &root).unwrap();
        assert_eq!(found.lists, ["malware", "custom"]);
        assert_eq!(found.summary_url.as_deref(), Some("https://r.test/s.json"));
        assert_eq!(found.api_url, None);
        assert_eq!(
            found.metadata_url.as_deref(),
            Some("https://r.test/p2/%package%.json")
        );
        assert!(advertised(&repo, &json!({"filter": {"metadata": false}})).is_none());
        assert!(advertised(&repo, &json!({})).is_none());
    }

    #[test]
    fn builds_and_matches_entries_like_composer() {
        let packages = [
            locked("a/a", "1.0.0"),
            locked("a/a", "2.0.0"),
            locked("php", "8.4.0"),
            locked("b/b", "1.0.0"),
        ];
        let map = constraint_map(&packages);
        assert!(!map.contains_key("php"));
        assert!(matches!(
            map["a/a"],
            Constraint::Multi {
                conjunctive: false,
                ..
            }
        ));
        let raw = json!({
            "malware": [
                {"constraint": "1.0.0", "source": "s1", "url": "u1", "reason": "r1"},
                {"constraint": "~>2"},
                {"constraint": "3.0.0"},
                {"package": "b/b", "constraint": "*", "source": "s2"},
                {"package": "c/c", "constraint": "*"},
                {"no": "constraint"},
                "junk",
            ],
            "other": [{"constraint": "*"}],
        });
        let lists = vec!["malware".to_owned()];
        let entries = build(&raw, &map, Some("a/a"), &lists);
        assert_eq!(entries.len(), 2);
        assert!(
            build(&raw, &map, None, &lists)
                .iter()
                .all(|e| e.package == "b/b")
        );

        let a1 = &packages[0];
        let hits = matching(a1, &entries, &[], &[], true);
        assert_eq!(hits.len(), 1);
        assert!(matching(&packages[1], &entries, &[], &[], true).is_empty());
        assert!(matching(a1, &entries, &[], &["s1".into()], true).is_empty());
        let ignore = [IgnoreRule {
            pattern: "a/*".into(),
            constraint: Constraint::Any,
            reason: None,
            on_block: true,
            on_audit: false,
        }];
        assert!(matching(a1, &entries, &ignore, &[], true).is_empty());
        assert_eq!(matching(a1, &entries, &ignore, &[], false).len(), 1);

        let second = Entry {
            source: Some("s3".into()),
            url: None,
            reason: None,
            ..entries[0].clone()
        };
        let custom = Entry {
            list: "custom".into(),
            ..entries[0].clone()
        };
        let text = reason(a1, &[&entries[0], &second, &custom]);
        assert_eq!(
            text,
            "- Package a/a 1.0.0 (in the lock file) was not loaded, because it was flagged as malware reported by s1 (see u1) reason: r1,  reported by s3, filtered by custom reported by s1 (see u1) reason: r1. To ignore filters for this package, add the package to the \"policy.malware.ignore\" and \"policy.custom.ignore\" config. To turn the feature off entirely, you can set \"policy.malware.block\" and \"policy.custom.block\" to false."
        );
    }
}
