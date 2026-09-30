use crate::error::Error;
use crate::time::rfc3339;
use crate::version::{
    DEFAULT_BRANCH_ALIAS, normalize, normalize_branch, parse_numeric_alias_prefix,
};
use phpm_php::smart_strcmp;
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

const LINK_TYPES: [&str; 5] = ["require", "conflict", "provide", "replace", "require-dev"];

static NINES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:\.9{7})+").expect("valid pattern"));

/// `Preg::replace('{(\.9{7})+}', '.x', $version)`: `2.9999999.9999999-dev`
/// prints as `2.x-dev`.
pub(crate) fn pretty_alias(normalized: &str) -> String {
    NINES.replace_all(normalized, ".x").into_owned()
}

/// PHP `empty()`.
fn php_empty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null | Value::Bool(false)) => true,
        Some(Value::String(s)) => s.is_empty() || s == "0",
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        Some(Value::Bool(true)) => false,
    }
}

fn is_php_array(v: &Value) -> bool {
    matches!(v, Value::Array(_) | Value::Object(_))
}

/// PHP `isset()` on an array key.
fn isset<'a>(config: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    config.get(key).filter(|v| !v.is_null())
}

/// The `(string)` cast for the scalars that appear in locks.
fn php_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) if n.is_i64() => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "1".to_owned() } else { String::new() }),
        _ => None,
    }
}

/// Keys and values of a PHP array decoded from JSON, lists keyed 0..n.
fn php_entries(v: &Value) -> Vec<(String, &Value)> {
    match v {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
        _ => Vec::new(),
    }
}

/// Rebuilds a PHP array from entries, as a list when the keys are 0..n.
fn php_array(entries: Vec<(String, Value)>) -> Value {
    Value::Object(entries.into_iter().collect())
}

fn ksort(map: &mut [(String, Value)]) {
    map.sort_by(|(a, _), (b, _)| smart_strcmp(a, b));
}

/// A locked package loaded the way `ArrayLoader` loads it.
#[derive(Debug, Clone)]
pub(crate) struct Package {
    pub(crate) name: String,
    pub(crate) pretty_name: String,
    pub(crate) pretty_version: String,
    pub(crate) version: String,
    pub(crate) kind: String,
    pub(crate) target_dir: Option<String>,
    pub(crate) source_type: Option<String>,
    pub(crate) source_reference: Option<String>,
    pub(crate) dist_type: Option<String>,
    pub(crate) dist_reference: Option<String>,
    pub(crate) replaces: Vec<(String, String)>,
    pub(crate) provides: Vec<(String, String)>,
    pub(crate) branch_alias: Option<String>,
    head: Vec<(String, Value)>,
    tail: Vec<(String, Value)>,
}

fn invalid(name: &str, reason: &str) -> Error {
    Error::InvalidPackage {
        package: name.to_owned(),
        reason: reason.to_owned(),
    }
}

/// Links of one type as `ArrayLoader::parseLinks` keeps them: lowercase
/// targets, string constraints only.
fn links(config: &Map<String, Value>, link_type: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(value) = isset(config, link_type).filter(|v| is_php_array(v)) else {
        return out;
    };
    for (target, constraint) in php_entries(value) {
        let Value::String(constraint) = constraint else {
            continue;
        };
        let target = target.to_ascii_lowercase();
        match out.iter_mut().find(|(t, _)| *t == target) {
            Some(slot) => slot.1.clone_from(constraint),
            None => out.push((target, constraint.clone())),
        }
    }
    out
}

// Composer: Package/Loader/ArrayLoader.php getBranchAlias
pub(crate) fn branch_alias(config: &Map<String, Value>, version: &str) -> Option<String> {
    if !version.starts_with("dev-") && !version.ends_with("-dev") {
        return None;
    }
    if let Some(aliases) = config
        .get("extra")
        .and_then(|e| e.get("branch-alias"))
        .filter(|v| is_php_array(v))
    {
        for (source, target) in php_entries(aliases) {
            let Some(target) = target.as_str() else {
                continue;
            };
            let Some(stem) = target.strip_suffix("-dev") else {
                continue;
            };
            let validated = if target == DEFAULT_BRANCH_ALIAS {
                DEFAULT_BRANCH_ALIAS.to_owned()
            } else {
                normalize_branch(stem)
            };
            if !validated.ends_with("-dev") || !version.eq_ignore_ascii_case(&source) {
                continue;
            }
            if let (Some(sp), Some(tp)) = (
                parse_numeric_alias_prefix(&source),
                parse_numeric_alias_prefix(target),
            ) && !tp
                .to_ascii_lowercase()
                .starts_with(&sp.to_ascii_lowercase())
            {
                continue;
            }
            return Some(validated);
        }
    }
    let unprefixed = version.strip_prefix('v').unwrap_or(version);
    if config.get("default-branch") == Some(&Value::Bool(true))
        && parse_numeric_alias_prefix(unprefixed).is_none()
    {
        return Some(DEFAULT_BRANCH_ALIAS.to_owned());
    }
    None
}

impl Package {
    // Composer: Package/Loader/ArrayLoader.php load, then Package/Dumper/ArrayDumper.php dump
    pub(crate) fn from_lock(config: &Map<String, Value>) -> Result<Self, Error> {
        let pretty_name = isset(config, "name")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("(unknown)", "has no name"))?
            .to_owned();
        let pretty_version = isset(config, "version")
            .and_then(php_string)
            .ok_or_else(|| invalid(&pretty_name, "has no version"))?;
        let version = match config.get("version_normalized").and_then(Value::as_str) {
            Some(v) if v != DEFAULT_BRANCH_ALIAS => v.to_owned(),
            _ => normalize(&pretty_version)?,
        };

        let mut head: Vec<(String, Value)> = vec![
            ("name".into(), pretty_name.clone().into()),
            ("version".into(), pretty_version.clone().into()),
            ("version_normalized".into(), version.clone().into()),
        ];

        let target_dir = isset(config, "target-dir").cloned();
        if let Some(dir) = &target_dir {
            head.push(("target-dir".into(), dir.clone()));
        }

        let (mut source_type, mut source_reference) = (None, None);
        if let Some(source) = isset(config, "source") {
            let field = |k: &str| source.get(k).filter(|v| !v.is_null());
            let (Some(t), Some(url), Some(reference)) =
                (field("type"), field("url"), field("reference"))
            else {
                return Err(invalid(
                    &pretty_name,
                    "source needs type, url and reference",
                ));
            };
            let reference = php_string(reference)
                .ok_or_else(|| invalid(&pretty_name, "bad source reference"))?;
            let mut out = Map::new();
            out.insert("type".into(), t.clone());
            out.insert("url".into(), url.clone());
            out.insert("reference".into(), reference.clone().into());
            if let Some(m) = field("mirrors").filter(|m| !php_empty(Some(m))) {
                out.insert("mirrors".into(), m.clone());
            }
            source_type = php_string(t);
            source_reference = Some(reference);
            head.push(("source".into(), Value::Object(out)));
        }

        let (mut dist_type, mut dist_reference) = (None, None);
        if let Some(dist) = isset(config, "dist") {
            let field = |k: &str| dist.get(k).filter(|v| !v.is_null());
            let (Some(t), Some(url)) = (field("type"), field("url")) else {
                return Err(invalid(&pretty_name, "dist needs type and url"));
            };
            let mut out = Map::new();
            out.insert("type".into(), t.clone());
            out.insert("url".into(), url.clone());
            if let Some(reference) = field("reference") {
                let reference = php_string(reference)
                    .ok_or_else(|| invalid(&pretty_name, "bad dist reference"))?;
                out.insert("reference".into(), reference.clone().into());
                dist_reference = Some(reference);
            }
            if let Some(shasum) = field("shasum") {
                out.insert("shasum".into(), shasum.clone());
            }
            if let Some(m) = field("mirrors").filter(|m| !php_empty(Some(m))) {
                out.insert("mirrors".into(), m.clone());
            }
            dist_type = php_string(t);
            head.push(("dist".into(), Value::Object(out)));
        }

        for link_type in LINK_TYPES {
            let mut map: Vec<(String, Value)> = links(config, link_type)
                .into_iter()
                .map(|(t, c)| (t, Value::String(c)))
                .collect();
            if !map.is_empty() {
                ksort(&mut map);
                head.push((link_type.into(), php_array(map)));
            }
        }

        if let Some(suggest) = isset(config, "suggest").filter(|v| is_php_array(v)) {
            let mut map: Vec<(String, Value)> = php_entries(suggest)
                .into_iter()
                .map(|(k, v)| match v {
                    Value::String(s) if crate::version::php_trim(s) == "self.version" => {
                        (k, pretty_version.clone().into())
                    }
                    v => (k, v.clone()),
                })
                .collect();
            if !map.is_empty() {
                ksort(&mut map);
                head.push(("suggest".into(), php_array(map)));
            }
        }

        if !php_empty(config.get("time")) {
            let time = config.get("time").and_then(php_string).unwrap_or_default();
            let formatted = rfc3339(&time)
                .ok_or_else(|| invalid(&pretty_name, &format!("unsupported time {time:?}")))?;
            head.push(("time".into(), formatted.into()));
        }

        if config.get("default-branch") == Some(&Value::Bool(true)) {
            head.push(("default-branch".into(), true.into()));
        }

        if let Some(bin) = isset(config, "bin") {
            let trim = |v: &Value| match v {
                Value::String(s) => Value::String(s.trim_start_matches('/').to_owned()),
                other => other.clone(),
            };
            let bins = match bin {
                Value::Array(items) => Value::Array(items.iter().map(trim).collect()),
                Value::Object(map) => {
                    Value::Object(map.iter().map(|(k, v)| (k.clone(), trim(v))).collect())
                }
                scalar => Value::Array(vec![trim(scalar)]),
            };
            if !php_empty(Some(&bins)) {
                head.push(("bin".into(), bins));
            }
        }

        let kind = match isset(config, "type").and_then(php_string) {
            Some(t) => t.to_ascii_lowercase(),
            None => "library".to_owned(),
        };
        head.push(("type".into(), kind.clone().into()));

        if let Some(extra) =
            isset(config, "extra").filter(|v| is_php_array(v) && !php_empty(Some(v)))
        {
            head.push(("extra".into(), extra.clone()));
        }

        let mut tail: Vec<(String, Value)> = Vec::new();
        let mut value_key = |key: &str, value: Option<&Value>| {
            if let Some(v) =
                value.filter(|v| !(v.is_null() || (is_php_array(v) && php_empty(Some(v)))))
            {
                tail.push((key.to_owned(), v.clone()));
            }
        };
        value_key("autoload", isset(config, "autoload"));
        value_key("autoload-dev", isset(config, "autoload-dev"));
        value_key(
            "notification-url",
            config
                .get("notification-url")
                .filter(|v| !php_empty(Some(v))),
        );
        value_key("include-path", isset(config, "include-path"));
        value_key("php-ext", isset(config, "php-ext"));

        let mut archive = Map::new();
        let archive_field = |k: &str| {
            config
                .get("archive")
                .and_then(|a| a.get(k))
                .filter(|v| !php_empty(Some(v)))
        };
        if let Some(name) = archive_field("name") {
            archive.insert("name".into(), name.clone());
        }
        if let Some(exclude) = archive_field("exclude") {
            archive.insert("exclude".into(), exclude.clone());
        }
        if !archive.is_empty() {
            tail.push(("archive".into(), Value::Object(archive)));
        }

        if let Some(scripts) = isset(config, "scripts").filter(|v| is_php_array(v)) {
            let cast: Vec<(String, Value)> = php_entries(scripts)
                .into_iter()
                .map(|(event, listeners)| {
                    let listeners = match listeners {
                        Value::Array(_) | Value::Object(_) => listeners.clone(),
                        Value::Null => Value::Array(Vec::new()),
                        scalar => Value::Array(vec![scalar.clone()]),
                    };
                    (event, listeners)
                })
                .collect();
            if !cast.is_empty() {
                tail.push(("scripts".into(), php_array(cast)));
            }
        }

        if !php_empty(config.get("license")) {
            let license = config.get("license").cloned().unwrap_or(Value::Null);
            let license = if is_php_array(&license) {
                license
            } else {
                Value::Array(vec![license])
            };
            tail.push(("license".into(), license));
        }
        if let Some(authors) = config
            .get("authors")
            .filter(|v| is_php_array(v) && !php_empty(Some(v)))
        {
            tail.push(("authors".into(), authors.clone()));
        }
        for key in ["description", "homepage"] {
            if let Some(v) = config
                .get(key)
                .filter(|v| v.is_string() && !php_empty(Some(v)))
            {
                tail.push((key.into(), v.clone()));
            }
        }
        if let Some(keywords) = config
            .get("keywords")
            .filter(|v| is_php_array(v) && !php_empty(Some(v)))
        {
            let mut words: Vec<String> = php_entries(keywords)
                .into_iter()
                .map(|(_, v)| php_string(v).ok_or_else(|| invalid(&pretty_name, "bad keyword")))
                .collect::<Result<_, _>>()?;
            words.sort_by(|a, b| smart_strcmp(a, b));
            tail.push((
                "keywords".into(),
                Value::Array(words.into_iter().map(Value::String).collect()),
            ));
        }
        if let Some(support) =
            isset(config, "support").filter(|v| is_php_array(v) && !php_empty(Some(v)))
        {
            tail.push(("support".into(), support.clone()));
        }
        if let Some(funding) = config
            .get("funding")
            .filter(|v| is_php_array(v) && !php_empty(Some(v)))
        {
            tail.push(("funding".into(), funding.clone()));
        }
        match isset(config, "abandoned") {
            None | Some(Value::Bool(false)) => {}
            Some(Value::String(s)) if !s.is_empty() && s != "0" => {
                tail.push(("abandoned".into(), s.clone().into()));
            }
            Some(_) => tail.push(("abandoned".into(), true.into())),
        }
        if let Some(options) = isset(config, "transport-options").filter(|v| !php_empty(Some(v))) {
            tail.push(("transport-options".into(), options.clone()));
        }

        Ok(Self {
            name: pretty_name.to_ascii_lowercase(),
            branch_alias: branch_alias(config, &pretty_version),
            pretty_name,
            pretty_version,
            version,
            kind,
            target_dir: target_dir.as_ref().and_then(php_string),
            source_type,
            source_reference,
            dist_type,
            dist_reference,
            replaces: links(config, "replace"),
            provides: links(config, "provide"),
            head,
            tail,
        })
    }

    /// `Package::isDev()`.
    pub(crate) fn is_dev(&self) -> bool {
        self.version.starts_with("dev-") || self.version.ends_with("-dev")
    }

    /// `ArrayDumper::dump` with the installation source Composer recorded.
    pub(crate) fn dump(&self, installation_source: Option<&str>) -> Map<String, Value> {
        let mut out: Map<String, Value> = self.head.iter().cloned().collect();
        if let Some(source) = installation_source {
            out.insert("installation-source".into(), source.into());
        }
        out.extend(self.tail.iter().cloned());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{Package, branch_alias, pretty_alias};
    use crate::error::Error;
    use serde_json::{Map, Value, json};

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    fn keys(m: &Map<String, Value>) -> Vec<&str> {
        m.keys().map(String::as_str).collect()
    }

    #[test]
    fn dumps_in_array_dumper_order() {
        let lock = obj(json!({
            "name": "Acme/Foo",
            "version": "v1.2.3",
            "source": {"type": "git", "url": "u", "reference": "abc", "mirrors": []},
            "dist": {"type": "zip", "url": "d", "reference": "abc", "shasum": ""},
            "require": {"php": ">=8", "Psr/Log": "^1"},
            "require-dev": {"x/y": "*"},
            "suggest": {"b/b": "self.version", "a/a": "why"},
            "bin": "/bin/foo",
            "type": "Library",
            "extra": {},
            "autoload": {"psr-4": {"Acme\\": "src/"}},
            "notification-url": "https://packagist.org/downloads/",
            "license": "MIT",
            "keywords": ["b", "10", "9", "a"],
            "description": "",
            "homepage": "https://x",
            "support": {},
            "abandoned": "new/pkg",
            "time": "2024-01-02T03:04:05+00:00"
        }));
        let p = Package::from_lock(&lock).unwrap();
        assert_eq!(p.name, "acme/foo");
        assert_eq!(p.version, "1.2.3.0");
        let d = p.dump(Some("dist"));
        assert_eq!(
            keys(&d),
            [
                "name",
                "version",
                "version_normalized",
                "source",
                "dist",
                "require",
                "require-dev",
                "suggest",
                "time",
                "bin",
                "type",
                "installation-source",
                "autoload",
                "notification-url",
                "license",
                "homepage",
                "keywords",
                "abandoned",
            ]
        );
        assert_eq!(
            d["source"],
            json!({"type": "git", "url": "u", "reference": "abc"})
        );
        assert_eq!(d["require"], json!({"php": ">=8", "psr/log": "^1"}));
        assert_eq!(keys(d["suggest"].as_object().unwrap()), ["a/a", "b/b"]);
        assert_eq!(d["suggest"]["b/b"], "v1.2.3");
        assert_eq!(d["bin"], json!(["bin/foo"]));
        assert_eq!(d["type"], "library");
        assert_eq!(d["license"], json!(["MIT"]));
        assert_eq!(d["keywords"], json!(["9", "10", "a", "b"]));
        assert!(!p.dump(None).contains_key("installation-source"));
    }

    #[test]
    fn metapackage_without_optional_fields() {
        let p = Package::from_lock(&obj(json!({"name": "a/m", "version": "dev-master", "type": "metapackage",
            "abandoned": true, "archive": {"name": "x", "exclude": ["/t"]}, "scripts": {"post": "cmd", "n": null},
            "transport-options": {"ssl": {}}, "target-dir": "A/B", "default-branch": true}))).unwrap();
        let d = p.dump(None);
        assert_eq!(
            keys(&d),
            [
                "name",
                "version",
                "version_normalized",
                "target-dir",
                "default-branch",
                "type",
                "archive",
                "scripts",
                "abandoned",
                "transport-options"
            ]
        );
        assert_eq!(d["scripts"], json!({"post": ["cmd"], "n": []}));
        assert_eq!(d["abandoned"], true);
        assert_eq!(p.target_dir.as_deref(), Some("A/B"));
        assert!(p.is_dev());
        assert_eq!(p.branch_alias.as_deref(), Some("9999999-dev"));
    }

    #[test]
    fn rejects_broken_entries() {
        assert!(matches!(
            Package::from_lock(&obj(json!({"version": "1.0"}))),
            Err(Error::InvalidPackage { .. })
        ));
        assert!(Package::from_lock(&obj(json!({"name": "a/b"}))).is_err());
        assert!(Package::from_lock(&obj(json!({"name": "a/b", "version": "nope"}))).is_err());
        assert!(
            Package::from_lock(&obj(
                json!({"name": "a/b", "version": "1.0", "source": {"type": "git"}})
            ))
            .is_err()
        );
        assert!(
            Package::from_lock(&obj(
                json!({"name": "a/b", "version": "1.0", "dist": {"url": "x"}})
            ))
            .is_err()
        );
        assert!(
            Package::from_lock(&obj(
                json!({"name": "a/b", "version": "1.0", "time": "soon"})
            ))
            .is_err()
        );
        assert!(
            Package::from_lock(&obj(
                json!({"name": "a/b", "version": "1.0", "keywords": [[1]]})
            ))
            .is_err()
        );
        assert!(
            Package::from_lock(&obj(json!({"name": "a/b", "version": "1.0", "source": {"type": "git", "url": "u", "reference": [1]}})))
                .is_err()
        );
    }

    #[test]
    fn keeps_normalized_version_from_lock_unless_default_alias() {
        let p = Package::from_lock(&obj(
            json!({"name": "a/b", "version": "1.0", "version_normalized": "1.0.0.0"}),
        ))
        .unwrap();
        assert_eq!(p.version, "1.0.0.0");
        let p = Package::from_lock(&obj(
            json!({"name": "a/b", "version": "dev-main", "version_normalized": "9999999-dev"}),
        ))
        .unwrap();
        assert_eq!(p.version, "dev-main");
        let p = Package::from_lock(&obj(json!({"name": "a/b", "version": 2}))).unwrap();
        assert_eq!(p.pretty_version, "2");
    }

    #[test]
    fn branch_aliases_follow_array_loader() {
        let cfg = |v: Value| obj(v);
        assert_eq!(
            branch_alias(
                &cfg(json!({"extra": {"branch-alias": {"dev-main": "2.x-dev"}}})),
                "dev-main"
            )
            .as_deref(),
            Some("2.9999999.9999999.9999999-dev")
        );
        assert_eq!(
            branch_alias(
                &cfg(json!({"extra": {"branch-alias": {"dev-main": "2.x"}}})),
                "dev-main"
            ),
            None
        );
        assert_eq!(
            branch_alias(
                &cfg(json!({"extra": {"branch-alias": {"dev-other": "2.x-dev"}}})),
                "dev-main"
            ),
            None
        );
        assert_eq!(
            branch_alias(
                &cfg(json!({"extra": {"branch-alias": {"1.x-dev": "2.0.x-dev"}}})),
                "1.x-dev"
            ),
            None
        );
        assert_eq!(
            branch_alias(
                &cfg(json!({"extra": {"branch-alias": {"1.x-dev": "1.2.x-dev"}}})),
                "1.x-dev"
            )
            .as_deref(),
            Some("1.2.9999999.9999999-dev")
        );
        assert_eq!(
            branch_alias(
                &cfg(json!({"extra": {"branch-alias": {"dev-main": "9999999-dev"}}})),
                "dev-main"
            )
            .as_deref(),
            Some("9999999-dev")
        );
        assert_eq!(
            branch_alias(
                &cfg(json!({"extra": {"branch-alias": {"dev-main": "foo-dev"}}})),
                "dev-main"
            ),
            None
        );
        assert_eq!(
            branch_alias(&cfg(json!({"default-branch": true})), "1.0.0"),
            None
        );
        assert_eq!(
            branch_alias(&cfg(json!({"default-branch": true})), "v2.x-dev"),
            None
        );
        assert_eq!(
            branch_alias(&cfg(json!({"default-branch": true})), "dev-trunk").as_deref(),
            Some("9999999-dev")
        );
    }

    #[test]
    fn pretty_alias_collapses_nines() {
        assert_eq!(pretty_alias("2.0.9999999.9999999-dev"), "2.0.x-dev");
        assert_eq!(pretty_alias("9999999-dev"), "9999999-dev");
    }
}
