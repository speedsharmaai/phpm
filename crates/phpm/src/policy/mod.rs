//! Composer 2.10's dependency policy: `config.policy` with the `config.audit`
//! fallbacks, the env overrides and `--no-blocking`.
//!
//! Only the parts an install from a lock uses are parsed: the malware list
//! (the one list that blocks at install time), and the advisories, abandoned
//! and filter-list settings `--audit` reports with.

mod audit;
mod filter;
mod repo;

use phpm_lock::constraint::{Constraint, parse};
use serde_json::{Map, Value};

use crate::error::Error;
use crate::platform::glob;
use crate::project::Env;

pub(crate) use audit::{Abandonment, AuditFormat, Audited, run_audit};
pub(crate) use filter::{Locked, check_install};
pub(crate) use repo::{Client, repos};

/// How `composer audit` treats a list's matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditMode {
    Ignore,
    Report,
    Fail,
}

impl AuditMode {
    fn parse(value: Option<&Value>, default: Self) -> Result<Self, Error> {
        match value.and_then(Value::as_str) {
            None => Ok(default),
            Some("ignore") => Ok(Self::Ignore),
            Some("report") => Ok(Self::Report),
            Some("fail") => Ok(Self::Fail),
            Some(other) => Err(Error::install(format!(
                "Invalid audit value \"{other}\". Expected one of ignore, report, fail."
            ))),
        }
    }
}

/// `IgnorePackageRule`: a package name pattern and the versions it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IgnoreRule {
    pub(crate) pattern: String,
    pub(crate) constraint: Constraint,
    pub(crate) reason: Option<String>,
    pub(crate) on_block: bool,
    pub(crate) on_audit: bool,
}

impl IgnoreRule {
    fn any(pattern: &str, reason: Option<String>, on_block: bool, on_audit: bool) -> Self {
        Self {
            pattern: pattern.to_owned(),
            constraint: Constraint::Any,
            reason,
            on_block,
            on_audit,
        }
    }

    pub(crate) fn applies(&self, name: &str, version: &str, block: bool) -> bool {
        (if block { self.on_block } else { self.on_audit })
            && glob(&self.pattern, name)
            && self.constraint.matches_version(version)
    }
}

/// An ignored advisory id or severity, with its reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IgnoreId {
    pub(crate) id: String,
    pub(crate) reason: Option<String>,
    pub(crate) on_audit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Malware {
    pub(crate) block: bool,
    pub(crate) audit: AuditMode,
    /// `all`, `install` or `update`.
    pub(crate) block_scope: String,
    pub(crate) ignore: Vec<IgnoreRule>,
    pub(crate) ignore_source: Vec<String>,
}

impl Malware {
    fn disabled() -> Self {
        Self {
            block: false,
            audit: AuditMode::Ignore,
            block_scope: "all".to_owned(),
            ignore: Vec::new(),
            ignore_source: Vec::new(),
        }
    }

    /// `MalwarePolicyConfig::shouldBlock('install')`.
    pub(crate) fn blocks_install(&self) -> bool {
        self.block && matches!(self.block_scope.as_str(), "all" | "install")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Advisories {
    pub(crate) ignore: Vec<IgnoreRule>,
    pub(crate) ignore_id: Vec<IgnoreId>,
    pub(crate) ignore_severity: Vec<IgnoreId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Abandoned {
    pub(crate) audit: AuditMode,
    pub(crate) ignore: Vec<IgnoreRule>,
}

/// The parts of `PolicyConfig` an install uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Policy {
    pub(crate) enabled: bool,
    pub(crate) malware: Malware,
    pub(crate) advisories: Advisories,
    pub(crate) abandoned: Abandoned,
    pub(crate) unreachable_install: bool,
    pub(crate) unreachable_audit: bool,
}

// Composer: Util/Platform.php getBoolEnv
fn bool_env(env: Env<'_>, name: &str) -> Result<Option<bool>, Error> {
    match env(name).as_deref() {
        None | Some("") => Ok(None),
        Some("1" | "true" | "on") => Ok(Some(true)),
        Some("0" | "false" | "off") => Ok(Some(false)),
        Some(other) => Err(Error::install(format!(
            "Invalid value for {name}: {other}. Expected 0, 1, false, true, off or on."
        ))),
    }
}

fn constraint_of(value: Option<&Value>) -> Result<Constraint, Error> {
    match value.and_then(Value::as_str) {
        Some(c) => parse(c).map_err(Error::install),
        None => Ok(Constraint::Any),
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

fn flag(map: &Map<String, Value>, key: &str) -> bool {
    map.get(key).and_then(Value::as_bool).unwrap_or(true)
}

// Composer: Policy/IgnorePackageRule.php parseIgnoreMap
fn ignore_map(value: Option<&Value>) -> Result<Vec<IgnoreRule>, Error> {
    let mut rules = Vec::new();
    match value {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for item in list {
                let name = item.as_str().ok_or_else(|| {
                    Error::install("Invalid ignore entry: expected a package name.")
                })?;
                rules.push(IgnoreRule::any(name, None, true, true));
            }
        }
        Some(Value::Object(map)) => {
            for (name, rule) in map {
                match rule {
                    Value::Null => rules.push(IgnoreRule::any(name, None, true, true)),
                    Value::String(reason) => {
                        rules.push(IgnoreRule::any(name, Some(reason.clone()), true, true));
                    }
                    Value::Array(list) => {
                        for item in list {
                            let obj = item.as_object().ok_or_else(|| {
                                Error::install(format!(
                                    "Invalid ignore rule for \"{name}\": expected an object."
                                ))
                            })?;
                            rules.push(rule_object(name, obj)?);
                        }
                    }
                    Value::Object(obj) => rules.push(rule_object(name, obj)?),
                    _ => {
                        return Err(Error::install(format!(
                            "Invalid ignore entry at key \"{name}\": expected null, a reason string, a rule object, or a list of rule objects."
                        )));
                    }
                }
            }
        }
        Some(_) => return Err(Error::install("Invalid ignore config: expected an object.")),
    }
    Ok(rules)
}

fn rule_object(name: &str, obj: &Map<String, Value>) -> Result<IgnoreRule, Error> {
    Ok(IgnoreRule {
        pattern: name.to_owned(),
        constraint: constraint_of(obj.get("constraint"))?,
        reason: text(obj.get("reason")),
        on_block: flag(obj, "on-block"),
        on_audit: flag(obj, "on-audit"),
    })
}

// Composer: Policy/IgnoreIdRule.php parseIgnoreIdMap, IgnoreSeverityRule.php
fn id_map(value: Option<&Value>) -> Vec<IgnoreId> {
    let mut out = Vec::new();
    match value {
        Some(Value::Array(list)) => {
            for id in list.iter().filter_map(Value::as_str) {
                out.push(IgnoreId {
                    id: id.to_owned(),
                    reason: None,
                    on_audit: true,
                });
            }
        }
        Some(Value::Object(map)) => {
            for (id, v) in map {
                let (reason, on_audit) = match v {
                    Value::String(r) => (Some(r.clone()), true),
                    Value::Object(o) => (text(o.get("reason")), flag(o, "on-audit")),
                    _ => (None, true),
                };
                out.push(IgnoreId {
                    id: id.clone(),
                    reason,
                    on_audit,
                });
            }
        }
        _ => {}
    }
    out
}

// Composer: Policy/ListPolicyConfig.php parseLegacySingleIgnore
fn legacy_apply(value: &Value) -> Result<(Option<String>, bool, bool), Error> {
    match value {
        Value::String(reason) => Ok((Some(reason.clone()), true, true)),
        Value::Object(o) => {
            let apply = o.get("apply").and_then(Value::as_str).unwrap_or("all");
            if !matches!(apply, "audit" | "block" | "all") {
                return Err(Error::install(format!(
                    "Invalid 'apply' value: {apply}. Expected 'audit', 'block', or 'all'."
                )));
            }
            Ok((text(o.get("reason")), apply != "audit", apply != "block"))
        }
        _ => Ok((None, true, true)),
    }
}

fn legacy_entries(value: Option<&Value>) -> Vec<(String, &Value)> {
    match value {
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(|v| v.as_str().map(|s| (s.to_owned(), v)))
            .collect(),
        Some(Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
        _ => Vec::new(),
    }
}

/// `config.audit` with Composer's defaults merged in.
fn audit_config(raw: Option<&Value>) -> Map<String, Value> {
    let mut audit = Map::new();
    audit.insert("ignore".into(), Value::Array(Vec::new()));
    audit.insert("abandoned".into(), Value::String("fail".into()));
    if let Some(Value::Object(user)) = raw {
        for (k, v) in user {
            audit.insert(k.clone(), v.clone());
        }
    }
    audit
}

/// A list's config: `None` when it is turned off with `false`.
fn list_config<'a>(
    policy: &'a Map<String, Value>,
    name: &str,
    empty: &'a Map<String, Value>,
) -> Option<&'a Map<String, Value>> {
    match policy.get(name) {
        Some(Value::Bool(false)) => None,
        Some(Value::Object(m)) => Some(m),
        _ => Some(empty),
    }
}

impl Policy {
    /// `PolicyConfig::fromConfig` for a project's composer.json `config`.
    // Composer: Policy/PolicyConfig.php fromConfig, Config.php get('policy')
    pub(crate) fn from_config(
        config: Option<&Map<String, Value>>,
        env: Env<'_>,
        no_blocking: bool,
    ) -> Result<Self, Error> {
        let raw_policy = config.and_then(|c| c.get("policy"));
        let mut enabled = !matches!(raw_policy, Some(Value::Bool(false)));
        match bool_env(env, "COMPOSER_POLICY")? {
            Some(false) => enabled = false,
            Some(true) => enabled = true,
            None => {}
        }
        let empty = Map::new();
        let policy = match raw_policy {
            Some(Value::Object(m)) => m,
            _ => &empty,
        };
        let audit = audit_config(config.and_then(|c| c.get("audit")));
        if !enabled {
            return Ok(Self::disabled());
        }

        let mut malware = match list_config(policy, "malware", &empty) {
            None => Malware::disabled(),
            Some(m) => Malware {
                block: m.get("block").and_then(Value::as_bool).unwrap_or(true),
                audit: AuditMode::parse(m.get("audit"), AuditMode::Fail)?,
                block_scope: text(m.get("block-scope")).unwrap_or_else(|| "all".into()),
                ignore: ignore_map(m.get("ignore"))?,
                ignore_source: m
                    .get("ignore-source")
                    .and_then(Value::as_array)
                    .map(|l| {
                        l.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
            },
        };
        if let Some(block) = bool_env(env, "COMPOSER_POLICY_MALWARE_BLOCK")? {
            malware.block = block;
        }

        let advisories = if policy.contains_key("advisories") {
            match list_config(policy, "advisories", &empty) {
                None => Advisories {
                    ignore: Vec::new(),
                    ignore_id: Vec::new(),
                    ignore_severity: Vec::new(),
                },
                Some(a) => Advisories {
                    ignore: ignore_map(a.get("ignore"))?,
                    ignore_id: id_map(a.get("ignore-id")),
                    ignore_severity: id_map(a.get("ignore-severity")),
                },
            }
        } else {
            let mut ignore = Vec::new();
            let mut ignore_id = Vec::new();
            for (id, value) in legacy_entries(audit.get("ignore")) {
                let (reason, on_block, on_audit) = legacy_apply(value)?;
                if id.contains('/') {
                    ignore.push(IgnoreRule::any(&id, reason, on_block, on_audit));
                } else {
                    ignore_id.push(IgnoreId {
                        id,
                        reason,
                        on_audit,
                    });
                }
            }
            let mut ignore_severity = Vec::new();
            for (id, value) in legacy_entries(audit.get("ignore-severity")) {
                let (reason, _, on_audit) = legacy_apply(value)?;
                ignore_severity.push(IgnoreId {
                    id,
                    reason,
                    on_audit,
                });
            }
            Advisories {
                ignore,
                ignore_id,
                ignore_severity,
            }
        };

        let mut abandoned = if policy.contains_key("abandoned") {
            match list_config(policy, "abandoned", &empty) {
                None => Abandoned {
                    audit: AuditMode::Ignore,
                    ignore: Vec::new(),
                },
                Some(a) => Abandoned {
                    audit: AuditMode::parse(a.get("audit"), AuditMode::Fail)?,
                    ignore: ignore_map(a.get("ignore"))?,
                },
            }
        } else {
            let mut ignore = Vec::new();
            for (name, value) in legacy_entries(audit.get("ignore-abandoned")) {
                let (reason, on_block, on_audit) = legacy_apply(value)?;
                ignore.push(IgnoreRule::any(&name, reason, on_block, on_audit));
            }
            Abandoned {
                audit: AuditMode::parse(audit.get("abandoned"), AuditMode::Fail)?,
                ignore,
            }
        };
        if let Some(mode) = env("COMPOSER_AUDIT_ABANDONED") {
            abandoned.audit = AuditMode::parse(Some(&Value::String(mode.clone())), AuditMode::Fail)
                .map_err(|_| {
                    Error::install(format!(
                        "Invalid value for COMPOSER_AUDIT_ABANDONED: {mode}. Expected one of ignore, report, fail."
                    ))
                })?;
        }

        let (mut unreachable_audit, mut unreachable_install) = (false, true);
        let raw_unreachable = policy
            .get("ignore-unreachable")
            .or_else(|| audit.get("ignore-unreachable"));
        match raw_unreachable {
            Some(Value::Array(scopes)) => {
                let has = |s: &str| scopes.iter().any(|v| v == s);
                unreachable_audit = has("audit");
                unreachable_install = has("install");
            }
            Some(Value::Bool(all)) => {
                unreachable_audit = *all;
                unreachable_install = *all;
            }
            _ => {}
        }

        let no_blocking = no_blocking
            || bool_env(env, "COMPOSER_NO_BLOCKING")?.unwrap_or(false)
            || bool_env(env, "COMPOSER_NO_SECURITY_BLOCKING")?.unwrap_or(false);
        if no_blocking {
            malware.block = false;
        }
        Ok(Self {
            enabled,
            malware,
            advisories,
            abandoned,
            unreachable_install,
            unreachable_audit,
        })
    }

    fn disabled() -> Self {
        Self {
            enabled: false,
            malware: Malware::disabled(),
            advisories: Advisories {
                ignore: Vec::new(),
                ignore_id: Vec::new(),
                ignore_severity: Vec::new(),
            },
            abandoned: Abandoned {
                audit: AuditMode::Ignore,
                ignore: Vec::new(),
            },
            unreachable_install: true,
            unreachable_audit: true,
        }
    }

    /// Whether an install consults the filter lists at all.
    pub(crate) fn blocks_install(&self) -> bool {
        self.enabled && self.malware.blocks_install()
    }
}

#[cfg(test)]
mod tests {
    use super::{AuditMode, Policy};
    use serde_json::{Value, json};

    fn policy(config: &Value, env: &[(&str, &str)], no_blocking: bool) -> Result<Policy, String> {
        let pairs: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let lookup = move |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        Policy::from_config(config.as_object(), &lookup, no_blocking).map_err(|e| e.message)
    }

    #[test]
    fn defaults_block_malware_at_install() {
        let p = policy(&json!({}), &[], false).unwrap();
        assert!(p.blocks_install());
        assert_eq!(p.malware.audit, AuditMode::Fail);
        assert_eq!(p.abandoned.audit, AuditMode::Fail);
        assert!(p.unreachable_install && !p.unreachable_audit);
    }

    #[test]
    fn switches_that_turn_blocking_off() {
        assert!(
            !policy(&json!({"policy": false}), &[], false)
                .unwrap()
                .blocks_install()
        );
        assert!(
            !policy(&json!({}), &[("COMPOSER_POLICY", "0")], false)
                .unwrap()
                .blocks_install()
        );
        assert!(
            policy(
                &json!({"policy": false}),
                &[("COMPOSER_POLICY", "1")],
                false
            )
            .unwrap()
            .blocks_install()
        );
        assert!(
            !policy(&json!({"policy": {"malware": false}}), &[], false)
                .unwrap()
                .blocks_install()
        );
        assert!(
            !policy(
                &json!({"policy": {"malware": {"block": false}}}),
                &[],
                false
            )
            .unwrap()
            .blocks_install()
        );
        assert!(
            !policy(
                &json!({"policy": {"malware": {"block-scope": "update"}}}),
                &[],
                false
            )
            .unwrap()
            .blocks_install()
        );
        assert!(
            policy(
                &json!({"policy": {"malware": {"block-scope": "install"}}}),
                &[],
                false
            )
            .unwrap()
            .blocks_install()
        );
        assert!(
            !policy(
                &json!({}),
                &[("COMPOSER_POLICY_MALWARE_BLOCK", "off")],
                false
            )
            .unwrap()
            .blocks_install()
        );
        assert!(
            !policy(&json!({}), &[("COMPOSER_NO_BLOCKING", "1")], false)
                .unwrap()
                .blocks_install()
        );
        assert!(
            !policy(
                &json!({}),
                &[("COMPOSER_NO_SECURITY_BLOCKING", "true")],
                false
            )
            .unwrap()
            .blocks_install()
        );
        assert!(!policy(&json!({}), &[], true).unwrap().blocks_install());
        assert!(policy(&json!({}), &[("COMPOSER_POLICY", "maybe")], false).is_err());
    }

    #[test]
    fn parses_ignore_rules_in_every_shape() {
        let p = policy(
            &json!({"policy": {"malware": {
                "ignore": {
                    "a/*": null,
                    "b/b": "trusted",
                    "c/c": {"constraint": "^1.0", "on-block": false},
                    "d/d": [{"constraint": "1.0.0"}, {"constraint": "2.0.0", "reason": "x"}],
                },
                "ignore-source": ["aikido"],
            }, "ignore-unreachable": ["audit"]}}),
            &[],
            false,
        )
        .unwrap();
        let ignore = &p.malware.ignore;
        assert_eq!(ignore.len(), 5);
        assert!(ignore[0].applies("a/x", "1.0.0.0", true));
        assert_eq!(ignore[1].reason.as_deref(), Some("trusted"));
        assert!(!ignore[2].applies("c/c", "1.2.0.0", true));
        assert!(ignore[2].applies("c/c", "1.2.0.0", false));
        assert!(!ignore[3].applies("d/d", "2.0.0.0", true));
        assert!(ignore[4].applies("d/d", "2.0.0.0", true));
        assert_eq!(p.malware.ignore_source, ["aikido"]);
        assert!(p.unreachable_audit && !p.unreachable_install);
        for bad in [
            json!({"policy": {"malware": {"ignore": {"a/a": 1}}}}),
            json!({"policy": {"malware": {"ignore": {"a/a": [1]}}}}),
            json!({"policy": {"malware": {"ignore": "x"}}}),
            json!({"policy": {"malware": {"ignore": [1]}}}),
            json!({"policy": {"malware": {"ignore": {"a/a": {"constraint": "~>1"}}}}}),
            json!({"policy": {"malware": {"audit": "loud"}}}),
        ] {
            assert!(policy(&bad, &[], false).is_err(), "{bad}");
        }
        let list = policy(
            &json!({"policy": {"malware": {"ignore": ["x/y"]}}}),
            &[],
            false,
        )
        .unwrap();
        assert!(list.malware.ignore[0].applies("x/y", "1.0.0.0", true));
    }

    #[test]
    fn reads_legacy_audit_config() {
        let p = policy(&json!({"audit": {
                "ignore": {"CVE-1": "known", "a/b": {"apply": "audit"}, "GHSA-x": {"apply": "block", "reason": "r"}},
                "ignore-severity": ["low"],
                "abandoned": "report",
                "ignore-abandoned": ["old/*"],
                "ignore-unreachable": true,
            }}),
            &[],
            false,
        )
        .unwrap();
        assert_eq!(p.advisories.ignore_id.len(), 2);
        assert!(!p.advisories.ignore_id[1].on_audit);
        assert!(p.advisories.ignore[0].applies("a/b", "1.0.0.0", false));
        assert!(!p.advisories.ignore[0].applies("a/b", "1.0.0.0", true));
        assert_eq!(p.advisories.ignore_severity[0].id, "low");
        assert_eq!(p.abandoned.audit, AuditMode::Report);
        assert_eq!(p.abandoned.ignore[0].pattern, "old/*");
        assert!(p.unreachable_audit);
        assert!(
            policy(
                &json!({"audit": {"ignore": {"x": {"apply": "nope"}}}}),
                &[],
                false
            )
            .is_err()
        );
        let env = policy(&json!({}), &[("COMPOSER_AUDIT_ABANDONED", "ignore")], false).unwrap();
        assert_eq!(env.abandoned.audit, AuditMode::Ignore);
        assert!(policy(&json!({}), &[("COMPOSER_AUDIT_ABANDONED", "x")], false).is_err());
    }

    #[test]
    fn reads_the_policy_shape() {
        let p = policy(&json!({"policy": {
                "advisories": {"ignore": {"a/a": null}, "ignore-id": {"PKSA-1": {"reason": "r", "on-audit": false}}, "ignore-severity": {"low": "meh"}},
                "abandoned": {"audit": "ignore", "ignore": ["x/*"]},
            }}),
            &[],
            false,
        )
        .unwrap();
        assert_eq!(p.advisories.ignore_id[0].reason.as_deref(), Some("r"));
        assert!(!p.advisories.ignore_id[0].on_audit);
        assert_eq!(
            p.advisories.ignore_severity[0].reason.as_deref(),
            Some("meh")
        );
        assert_eq!(p.abandoned.audit, AuditMode::Ignore);
        let off = policy(
            &json!({"policy": {"advisories": false, "abandoned": false}}),
            &[],
            false,
        )
        .unwrap();
        assert!(off.advisories.ignore.is_empty());
        assert_eq!(off.abandoned.audit, AuditMode::Ignore);
    }
}
