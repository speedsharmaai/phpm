use crate::error::Error;
use serde_json::{Map, Value};

fn parse_object(json: &str, what: &'static str) -> Result<Map<String, Value>, Error> {
    into_object(serde_json::from_str::<Value>(json)?, what)
}

fn into_object(value: Value, what: &'static str) -> Result<Map<String, Value>, Error> {
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(Error::NotAnObject(what)),
    }
}

/// A project's `composer.json`, key order preserved.
#[derive(Debug, Clone)]
pub struct ComposerJson {
    data: Map<String, Value>,
}

impl ComposerJson {
    pub fn parse(json: &str) -> Result<Self, Error> {
        parse_object(json, "composer.json").map(|data| Self { data })
    }

    pub fn from_value(value: Value) -> Result<Self, Error> {
        into_object(value, "composer.json").map(|data| Self { data })
    }

    pub fn data(&self) -> &Map<String, Value> {
        &self.data
    }

    fn config(&self, key: &str) -> Option<&Value> {
        self.data.get("config").and_then(|c| c.get(key))
    }

    /// `config.vendor-dir` without trailing slashes, `vendor` by default.
    pub fn vendor_dir(&self) -> String {
        let dir = self
            .config("vendor-dir")
            .and_then(Value::as_str)
            .unwrap_or("vendor");
        dir.trim_end_matches(['/', '\\']).to_owned()
    }

    pub fn install_preferences(&self) -> InstallPreferences {
        InstallPreferences::from_config(self.config("preferred-install"))
    }
}

/// A `composer.lock`, key order preserved.
#[derive(Debug, Clone)]
pub struct Lock {
    data: Map<String, Value>,
}

/// One entry of the lock's `aliases` list (inline `as` aliases from the root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockAlias {
    pub package: String,
    pub alias: String,
    pub alias_normalized: String,
}

impl Lock {
    pub fn parse(json: &str) -> Result<Self, Error> {
        parse_object(json, "composer.lock").map(|data| Self { data })
    }

    pub fn from_value(value: Value) -> Result<Self, Error> {
        into_object(value, "composer.lock").map(|data| Self { data })
    }

    fn list(&self, key: &str) -> Result<Vec<&Map<String, Value>>, Error> {
        let Some(Value::Array(items)) = self.data.get(key) else {
            return Ok(Vec::new());
        };
        items
            .iter()
            .map(|item| {
                item.as_object()
                    .ok_or(Error::NotAnObject("a locked package"))
            })
            .collect()
    }

    pub fn packages(&self) -> Result<Vec<&Map<String, Value>>, Error> {
        self.list("packages")
    }

    pub fn packages_dev(&self) -> Result<Vec<&Map<String, Value>>, Error> {
        self.list("packages-dev")
    }

    pub fn content_hash(&self) -> Option<&str> {
        self.data.get("content-hash").and_then(Value::as_str)
    }

    pub fn aliases(&self) -> Vec<LockAlias> {
        let Some(Value::Array(items)) = self.data.get("aliases") else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|item| {
                let field = |k: &str| item.get(k).and_then(Value::as_str).map(str::to_owned);
                Some(LockAlias {
                    package: field("package")?,
                    alias: field("alias")?,
                    alias_normalized: field("alias_normalized")?,
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preference {
    Dist,
    Source,
    Auto,
}

impl Preference {
    fn parse(s: &str) -> Self {
        match s {
            "dist" => Self::Dist,
            "source" => Self::Source,
            _ => Self::Auto,
        }
    }
}

/// Which of source and dist Composer installs a package from, per
/// `config.preferred-install`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPreferences {
    global: Preference,
    patterns: Vec<(String, Preference)>,
}

impl InstallPreferences {
    // Composer: Config.php merge of preferred-install, Factory.php createDownloadManager
    fn from_config(value: Option<&Value>) -> Self {
        match value {
            Some(Value::Object(map)) => {
                let mut patterns: Vec<(String, Preference)> =
                    vec![("*".to_owned(), Preference::Dist)];
                for (pattern, pref) in map {
                    let pref = Preference::parse(pref.as_str().unwrap_or(""));
                    match patterns.iter_mut().find(|(p, _)| p == pattern) {
                        Some(slot) => slot.1 = pref,
                        None => patterns.push((pattern.clone(), pref)),
                    }
                }
                if let Some(i) = patterns.iter().position(|(p, _)| p == "*") {
                    let wildcard = patterns.remove(i);
                    patterns.push(wildcard);
                }
                Self {
                    global: Preference::Auto,
                    patterns,
                }
            }
            Some(Value::String(s)) => Self {
                global: Preference::parse(s),
                patterns: Vec::new(),
            },
            _ => Self {
                global: Preference::Dist,
                patterns: Vec::new(),
            },
        }
    }

    // Composer: Downloader/DownloadManager.php getAvailableSources
    pub fn prefers_dist(&self, name: &str, is_dev: bool) -> bool {
        match self.global {
            Preference::Source => false,
            Preference::Dist => true,
            Preference::Auto => {
                for (pattern, pref) in &self.patterns {
                    if wildcard_match(pattern, name) {
                        return *pref == Preference::Dist || (!is_dev && *pref == Preference::Auto);
                    }
                }
                !is_dev
            }
        }
    }
}

fn wildcard_match(pattern: &str, name: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::{ComposerJson, InstallPreferences, Lock, LockAlias, wildcard_match};
    use crate::error::Error;
    use serde_json::json;

    #[test]
    fn keeps_key_order() {
        let c = ComposerJson::parse(r#"{"z":1,"a":2,"m":3}"#).unwrap();
        let keys: Vec<&str> = c.data().keys().map(String::as_str).collect();
        assert_eq!(keys, ["z", "a", "m"]);
    }

    #[test]
    fn rejects_non_objects() {
        assert!(matches!(
            ComposerJson::parse("[]"),
            Err(Error::NotAnObject("composer.json"))
        ));
        assert!(matches!(
            Lock::parse("1"),
            Err(Error::NotAnObject("composer.lock"))
        ));
        assert!(matches!(Lock::parse("{"), Err(Error::Json(_))));
        let lock = Lock::parse(r#"{"packages":[1]}"#).unwrap();
        assert!(lock.packages().is_err());
    }

    #[test]
    fn vendor_dir_defaults_and_trims() {
        assert_eq!(ComposerJson::parse("{}").unwrap().vendor_dir(), "vendor");
        let c = ComposerJson::parse(r#"{"config":{"vendor-dir":"lib/deps/"}}"#).unwrap();
        assert_eq!(c.vendor_dir(), "lib/deps");
    }

    #[test]
    fn reads_lock_sections() {
        let lock = Lock::parse(
            r#"{"packages":[{"name":"a/a"}],"aliases":[{"package":"a/a","version":"dev-main","alias":"1.0","alias_normalized":"1.0.0.0"},{"package":"b"}]}"#,
        )
        .unwrap();
        assert_eq!(lock.packages().unwrap().len(), 1);
        assert!(lock.packages_dev().unwrap().is_empty());
        assert_eq!(
            lock.aliases(),
            [LockAlias {
                package: "a/a".into(),
                alias: "1.0".into(),
                alias_normalized: "1.0.0.0".into()
            }]
        );
        assert!(Lock::parse("{}").unwrap().aliases().is_empty());
        assert_eq!(lock.content_hash(), None);
        let hashed = Lock::parse(r#"{"content-hash":"ab12"}"#).unwrap();
        assert_eq!(hashed.content_hash(), Some("ab12"));
    }

    #[test]
    fn preferred_install_resolution() {
        let default = InstallPreferences::from_config(None);
        assert!(default.prefers_dist("a/b", true));
        let source = InstallPreferences::from_config(Some(&json!("source")));
        assert!(!source.prefers_dist("a/b", false));
        let auto = InstallPreferences::from_config(Some(&json!("auto")));
        assert!(auto.prefers_dist("a/b", false));
        assert!(!auto.prefers_dist("a/b", true));
        let map = InstallPreferences::from_config(Some(
            &json!({"*": "source", "acme/*": "dist", "x/y": "auto"}),
        ));
        assert!(map.prefers_dist("Acme/Foo", true));
        assert!(!map.prefers_dist("other/foo", false));
        assert!(map.prefers_dist("x/y", false));
        assert!(!map.prefers_dist("x/y", true));
        let partial = InstallPreferences::from_config(Some(&json!({"acme/*": "source"})));
        assert!(!partial.prefers_dist("acme/foo", false));
        assert!(partial.prefers_dist("other/foo", true));
    }

    #[test]
    fn wildcards_match_like_composer() {
        assert!(wildcard_match("*", "a/b"));
        assert!(wildcard_match("a/*", "a/b"));
        assert!(!wildcard_match("a/*", "b/a"));
        assert!(wildcard_match("*/b*c", "x/bzzc"));
        assert!(!wildcard_match("*/b*c", "x/bzz"));
        assert!(wildcard_match("a/b", "A/B"));
        assert!(!wildcard_match("a/b", "a/bc"));
        assert!(!wildcard_match("xy*yx", "xyx"));
    }
}
