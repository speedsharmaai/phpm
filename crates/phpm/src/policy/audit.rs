//! `composer install --audit`: security advisories, abandoned packages and
//! filter-list matches for what was installed.
//!
//! Composer: Advisory/Auditor.php audit. Advisories come from each
//! repository's `security-advisories` api-url in one POST; Composer's
//! summary format reads the same data from per-package metadata instead.

use std::fmt::Write as _;

use phpm_lock::constraint::{Constraint, Op, parse};
use serde_json::{Map, Value, json};

use super::filter::{Entry, Locked, collect, matching};
use super::repo::{Client, Repo};
use super::{AuditMode, Policy};
use crate::platform::is_platform_package;

/// `--audit-format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditFormat {
    Table,
    Plain,
    Json,
    Summary,
}

impl AuditFormat {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Plain => "plain",
            Self::Json => "json",
            Self::Summary => "summary",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "table" => Some(Self::Table),
            "plain" => Some(Self::Plain),
            "json" => Some(Self::Json),
            "summary" => Some(Self::Summary),
            _ => None,
        }
    }
}

/// An installed package as the audit needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Audited {
    pub(crate) locked: Locked,
    pub(crate) abandoned: Option<Abandonment>,
}

/// An abandoned package's suggested replacement, if it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Abandonment(pub(crate) Option<String>);

#[derive(Debug, Clone)]
struct Advisory {
    id: String,
    package: String,
    affected: String,
    data: Map<String, Value>,
    ignored: bool,
    ignore_reason: Option<String>,
}

impl Advisory {
    fn field(&self, key: &str) -> Option<&str> {
        self.data.get(key).and_then(Value::as_str)
    }
}

/// What the audit prints and whether it failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub(crate) stdout: String,
    pub(crate) stderr: Vec<String>,
    pub(crate) failed: bool,
}

// Composer: http_build_query(['packages' => $names])
fn form(names: &[&String]) -> Vec<u8> {
    let enc = |s: &str| {
        s.bytes().fold(String::new(), |mut out, b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
                out.push(char::from(b));
            } else {
                let _ = write!(out, "%{b:02X}");
            }
            out
        })
    };
    names
        .iter()
        .enumerate()
        .map(|(i, n)| format!("packages%5B{i}%5D={}", enc(n)))
        .collect::<Vec<_>>()
        .join("&")
        .into_bytes()
}

fn versions_constraint(packages: &[Audited], name: &str) -> Constraint {
    Constraint::Multi {
        conjunctive: false,
        items: packages
            .iter()
            .filter(|p| p.locked.name == name)
            .map(|p| Constraint::Single {
                op: Op::Eq,
                version: p.locked.version.clone(),
            })
            .collect(),
    }
}

// Composer: Advisory/PartialSecurityAdvisory.php create
fn affected(text: &str) -> Constraint {
    parse(text).unwrap_or_else(|_| {
        let head: String = text
            .chars()
            .take_while(|c| matches!(c, '>' | '=' | '<' | '^' | '~'))
            .collect();
        let digits: String = text[head.len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        parse(&format!("{head}{digits}")).unwrap_or(Constraint::Single {
            op: Op::Eq,
            version: "0.0.0-invalid-version".to_owned(),
        })
    })
}

async fn advisories(
    client: &Client<'_>,
    repos: &[Repo],
    packages: &[Audited],
) -> Result<Vec<Advisory>, String> {
    let mut names: Vec<&String> = packages
        .iter()
        .map(|p| &p.locked.name)
        .filter(|n| !is_platform_package(n))
        .collect();
    names.sort();
    names.dedup();
    let mut out = Vec::new();
    for repo in repos {
        let Some(root) = client.root(repo).await? else {
            continue;
        };
        let Some(api) = root
            .data
            .get("security-advisories")
            .and_then(|s| s.get("api-url"))
            .and_then(Value::as_str)
            .map(|u| repo.canonical(u))
        else {
            continue;
        };
        if names.is_empty() {
            continue;
        }
        let response = client
            .fetcher
            .request(
                &api,
                None,
                Some(("application/x-www-form-urlencoded", form(&names))),
            )
            .await
            .map_err(|e| e.to_string())?;
        if response.status != 200 {
            return Err(format!("{api}: HTTP {}", response.status));
        }
        let body: Value = serde_json::from_slice(&response.body).map_err(|e| e.to_string())?;
        for (name, list) in body
            .get("advisories")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if !names.contains(&name) {
                continue;
            }
            let locked = versions_constraint(packages, name);
            for data in list
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
            {
                let text = |k: &str| {
                    data.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                if !affected(&text("affectedVersions")).matches(&locked) {
                    continue;
                }
                out.push(Advisory {
                    id: text("advisoryId"),
                    package: name.clone(),
                    affected: text("affectedVersions"),
                    data: data.clone(),
                    ignored: false,
                    ignore_reason: None,
                });
            }
        }
    }
    Ok(out)
}

// Composer: Advisory/Auditor.php processAdvisories
fn process(policy: &Policy, list: Vec<Advisory>) -> (Vec<Advisory>, Vec<Advisory>) {
    let mut ignore: Vec<(String, Option<String>)> = policy
        .advisories
        .ignore_id
        .iter()
        .filter(|r| r.on_audit)
        .map(|r| (r.id.clone(), r.reason.clone()))
        .collect();
    ignore.extend(
        policy
            .advisories
            .ignore
            .iter()
            .filter(|r| r.on_audit)
            .map(|r| (r.pattern.clone(), r.reason.clone())),
    );
    let severities: Vec<(String, Option<String>)> = policy
        .advisories
        .ignore_severity
        .iter()
        .filter(|r| r.on_audit)
        .map(|r| (r.id.clone(), r.reason.clone()))
        .collect();
    let mut active = Vec::new();
    let mut skipped = Vec::new();
    for mut a in list {
        let lookup = |key: &str| {
            ignore
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, r)| r.clone())
        };
        let mut ignored = false;
        let mut reason: Option<String> = None;
        for key in [a.package.as_str(), a.id.as_str()] {
            if let Some(r) = lookup(key) {
                (ignored, reason) = (true, r);
            }
        }
        if let Some(sev) = a.field("severity")
            && let Some((_, r)) = severities.iter().find(|(s, _)| s == sev)
        {
            ignored = true;
            reason = Some(
                r.clone()
                    .unwrap_or_else(|| format!("{sev} severity is ignored")),
            );
        }
        if let Some(cve) = a.field("cve")
            && let Some(r) = lookup(cve)
        {
            (ignored, reason) = (true, r);
        }
        let remote = a
            .data
            .get("sources")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|s| s.get("remoteId").and_then(Value::as_str))
            .find_map(lookup);
        if let Some(r) = remote {
            (ignored, reason) = (true, r);
        }
        if ignored {
            a.ignored = true;
            a.ignore_reason = reason;
            skipped.push(a);
        } else {
            active.push(a);
        }
    }
    (active, skipped)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        one.to_owned()
    } else {
        many.to_owned()
    }
}

fn count(list: &[Advisory]) -> (usize, usize) {
    let names: std::collections::BTreeSet<&str> = list.iter().map(|a| a.package.as_str()).collect();
    (names.len(), list.len())
}

fn plain(out: &mut String, list: &[Advisory]) {
    for (i, a) in list.iter().enumerate() {
        if i > 0 {
            out.push_str("--------\n");
        }
        let _ = writeln!(out, "Package: {}", a.package);
        let _ = writeln!(out, "Severity: {}", a.field("severity").unwrap_or_default());
        let _ = writeln!(out, "Advisory ID: {}", a.id);
        let _ = writeln!(out, "CVE: {}", a.field("cve").unwrap_or("NO CVE"));
        let _ = writeln!(out, "Title: {}", a.field("title").unwrap_or_default());
        let _ = writeln!(out, "URL: {}", a.field("link").unwrap_or_default());
        let _ = writeln!(out, "Affected versions: {}", a.affected);
        let reported = a
            .field("reportedAt")
            .unwrap_or_default()
            .replacen(' ', "T", 1);
        let _ = writeln!(out, "Reported at: {reported}+00:00");
        if a.ignored {
            let _ = writeln!(
                out,
                "Ignore reason: {}",
                a.ignore_reason.as_deref().unwrap_or("None specified")
            );
        }
    }
}

fn advisory_json(a: &Advisory) -> Value {
    let mut out = Map::new();
    out.insert("advisoryId".into(), json!(a.id));
    out.insert("packageName".into(), json!(a.package));
    out.insert("affectedVersions".into(), json!(a.affected));
    for key in ["title", "cve", "link"] {
        out.insert(key.into(), a.data.get(key).cloned().unwrap_or(Value::Null));
    }
    let reported = a
        .field("reportedAt")
        .map(|r| format!("{}+00:00", r.replacen(' ', "T", 1)));
    out.insert("reportedAt".into(), json!(reported));
    for key in ["sources", "severity"] {
        out.insert(key.into(), a.data.get(key).cloned().unwrap_or(Value::Null));
    }
    if a.ignored {
        out.insert("ignoreReason".into(), json!(a.ignore_reason));
    }
    Value::Object(out)
}

fn by_package(list: &[Advisory]) -> Value {
    let mut map = Map::new();
    for a in list {
        if let Value::Array(items) = map.entry(a.package.clone()).or_insert_with(|| json!([])) {
            items.push(advisory_json(a));
        }
    }
    Value::Object(map)
}

/// Audit `packages`; `Err` only for a failure Composer reports as
/// "Failed to audit installed packages." and then ignores.
pub(crate) async fn run_audit(
    client: &Client<'_>,
    repos: &[Repo],
    policy: &Policy,
    packages: &[Audited],
    format: AuditFormat,
) -> Result<Report, String> {
    let mut report = Report::default();
    let (active, ignored) = process(policy, advisories(client, repos, packages).await?);

    let abandoned: Vec<&Audited> = if policy.abandoned.audit == AuditMode::Ignore {
        Vec::new()
    } else {
        packages
            .iter()
            .filter(|p| p.abandoned.is_some())
            .filter(|p| {
                !policy
                    .abandoned
                    .ignore
                    .iter()
                    .any(|r| r.on_audit && crate::platform::glob(&r.pattern, &p.locked.name))
            })
            .collect()
    };
    let abandoned_failing = policy.abandoned.audit == AuditMode::Fail && !abandoned.is_empty();

    let mut filtered: Vec<(String, Vec<Entry>)> = Vec::new();
    let mut filtered_failing = 0;
    if policy.enabled && policy.malware.audit != AuditMode::Ignore {
        let locked: Vec<Locked> = packages.iter().map(|p| p.locked.clone()).collect();
        let (entries, outcome) = collect(
            client,
            repos,
            &locked,
            &["malware".to_owned()],
            policy.unreachable_audit,
        )
        .await
        .map_err(|e| e.message)?;
        report.stderr.extend(outcome.warnings);
        for p in &locked {
            let hits = matching(
                p,
                &entries,
                &policy.malware.ignore,
                &policy.malware.ignore_source,
                false,
            );
            if !hits.is_empty() {
                if policy.malware.audit == AuditMode::Fail {
                    filtered_failing += hits.len();
                }
                filtered.push((p.name.clone(), hits.into_iter().cloned().collect()));
            }
        }
    }
    report.failed = !active.is_empty() || abandoned_failing || filtered_failing > 0;

    let out = &mut report.stdout;
    if format == AuditFormat::Json {
        let mut doc = Map::new();
        doc.insert("advisories".into(), by_package(&active));
        if !ignored.is_empty() {
            doc.insert("ignored-advisories".into(), by_package(&ignored));
        }
        let mut gone = Map::new();
        for p in &abandoned {
            gone.insert(
                p.locked.pretty_name.clone(),
                json!(p.abandoned.as_ref().and_then(|a| a.0.clone())),
            );
        }
        doc.insert("abandoned".into(), Value::Object(gone));
        let mut filter = Map::new();
        for (name, entries) in &filtered {
            let list: Vec<Value> = entries
                .iter()
                .map(|e| json!({"packageName": e.package, "listName": e.list, "constraint": e.pretty_constraint, "url": e.url, "reason": e.reason, "id": e.id, "source": e.source}))
                .collect();
            filter.insert(name.clone(), json!(list));
        }
        doc.insert("filter".into(), Value::Object(filter));
        let _ = writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&Value::Object(doc)).unwrap_or_default()
        );
        return Ok(report);
    }
    let summary = format == AuditFormat::Summary;
    let punctuation = if summary { "." } else { ":" };
    if !active.is_empty() || !ignored.is_empty() {
        for (list, label) in [(&ignored, "ignored "), (&active, "")] {
            let (pkgs, total) = count(list);
            if pkgs > 0 {
                let _ = writeln!(
                    out,
                    "Found {total} {label}security vulnerability advisor{} affecting {pkgs} package{}{punctuation}",
                    plural(total, "y", "ies"),
                    plural(pkgs, "", "s")
                );
                if !summary {
                    plain(out, list);
                }
            }
        }
        if summary {
            out.push_str("Run \"composer audit\" for a full list of advisories.\n");
        }
    } else {
        out.push_str("No security vulnerability advisories found.\n");
    }
    if !abandoned.is_empty() && !summary {
        let _ = writeln!(
            out,
            "Found {} abandoned package{}:",
            abandoned.len(),
            plural(abandoned.len(), "", "s")
        );
        for p in &abandoned {
            let replacement = match p.abandoned.as_ref().and_then(|a| a.0.clone()) {
                Some(r) => format!("Use {r} instead"),
                None => "No replacement was suggested".to_owned(),
            };
            let _ = writeln!(out, "{} is abandoned. {replacement}.", p.locked.pretty_name);
        }
    }
    if !filtered.is_empty() {
        let _ = writeln!(
            out,
            "Found {} package{} matching filters{punctuation}",
            filtered.len(),
            plural(filtered.len(), "", "s")
        );
        if !summary {
            for (_, entries) in &filtered {
                for e in entries {
                    let mut parts = vec![format!(
                        "{} matched dependency policy \"{}\"",
                        e.package, e.list
                    )];
                    parts.extend(e.reason.as_ref().map(|r| format!("Reason: {r}")));
                    parts.extend(e.url.as_ref().map(|u| format!("URL: {u}")));
                    parts.extend(e.source.as_ref().map(|s| format!("Source: {s}")));
                    let _ = writeln!(out, "{}.", parts.join(". "));
                }
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::{Advisory, AuditFormat, affected, count, form, plain, process};
    use crate::policy::Policy;
    use serde_json::{Value, json};

    fn advisory(package: &str, id: &str, data: &Value) -> Advisory {
        Advisory {
            id: id.into(),
            package: package.into(),
            affected: ">=1".into(),
            data: data.as_object().unwrap().clone(),
            ignored: false,
            ignore_reason: None,
        }
    }

    #[test]
    fn parses_formats_and_encodes_forms() {
        assert_eq!(AuditFormat::parse("summary"), Some(AuditFormat::Summary));
        assert_eq!(AuditFormat::parse("table"), Some(AuditFormat::Table));
        assert_eq!(AuditFormat::parse("plain"), Some(AuditFormat::Plain));
        assert_eq!(AuditFormat::parse("json"), Some(AuditFormat::Json));
        assert_eq!(AuditFormat::parse("xml"), None);
        for f in ["table", "plain", "json", "summary"] {
            assert_eq!(AuditFormat::parse(f).unwrap().name(), f);
        }
        let a = "a/b".to_owned();
        let b = "c_d/e.f".to_owned();
        assert_eq!(
            form(&[&a, &b]),
            b"packages%5B0%5D=a%2Fb&packages%5B1%5D=c_d%2Fe.f"
        );
    }

    #[test]
    fn keeps_what_it_can_of_bad_affected_versions() {
        assert!(affected("<=3.20-test2").matches_version("3.19.0.0"));
        assert!(!affected("nonsense").matches_version("1.0.0.0"));
        assert!(affected(">=1.0,<2.0").matches_version("1.5.0.0"));
    }

    #[test]
    fn sorts_advisories_into_active_and_ignored() {
        let lookup = |_: &str| None;
        let config = json!({"audit": {
            "ignore": {"PKSA-1": "id", "x/pkg": null, "CVE-3": "cve", "GHSA-4": "remote"},
            "ignore-severity": {"low": null},
        }});
        let policy = Policy::from_config(config.as_object(), &lookup, false).unwrap();
        let list = vec![
            advisory("a/a", "PKSA-1", &json!({})),
            advisory("x/pkg", "PKSA-2", &json!({})),
            advisory("a/a", "PKSA-3", &json!({"cve": "CVE-3"})),
            advisory(
                "a/a",
                "PKSA-4",
                &json!({"sources": [{"remoteId": "GHSA-4"}]}),
            ),
            advisory("a/a", "PKSA-5", &json!({"severity": "low"})),
            advisory(
                "b/b",
                "PKSA-6",
                &json!({"severity": "high", "reportedAt": "2026-01-01 00:00:00"}),
            ),
        ];
        let (active, ignored) = process(&policy, list);
        assert_eq!(active.len(), 1);
        assert_eq!(ignored.len(), 5);
        assert_eq!(
            ignored[4].ignore_reason.as_deref(),
            Some("low severity is ignored")
        );
        assert_eq!(count(&ignored), (2, 5));
        let mut out = String::new();
        plain(&mut out, &ignored[..2]);
        assert!(
            out.contains(
                "--------\nPackage: x/pkg\nSeverity: \nAdvisory ID: PKSA-2\nCVE: NO CVE\n"
            ),
            "{out}"
        );
        assert!(out.contains("Ignore reason: id\n"), "{out}");
        assert!(out.contains("Ignore reason: None specified\n"), "{out}");
    }
}
