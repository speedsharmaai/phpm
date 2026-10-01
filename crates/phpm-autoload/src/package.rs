use phpm_php::{PhpArray, PhpKey, PhpValue};
use serde_json::{Map, Value};

/// A JSON value as `json_decode($json, true)` gives it to PHP.
pub(crate) fn to_php(value: &Value) -> PhpValue {
    match value {
        Value::Null => PhpValue::Null,
        Value::Bool(b) => PhpValue::Bool(*b),
        Value::Number(n) => n
            .as_i64()
            .map_or_else(|| PhpValue::String(n.to_string()), PhpValue::Int),
        Value::String(s) => PhpValue::String(s.clone()),
        Value::Array(items) => PhpValue::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, v)| (PhpKey::Int(i64::try_from(i).unwrap_or(i64::MAX)), to_php(v)))
                .collect(),
        ),
        Value::Object(map) => PhpValue::Array(
            map.iter()
                .map(|(k, v)| (PhpKey::from(k.as_str()), to_php(v)))
                .collect(),
        ),
    }
}

/// A link target and its constraint string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) target: String,
    pub(crate) constraint: String,
}

/// The fields of a package that autoloading reads, loaded the way
/// `ArrayLoader` loads them.
#[derive(Debug, Clone, Default)]
pub(crate) struct Package {
    pub(crate) name: String,
    pub(crate) pretty_name: String,
    pub(crate) kind: String,
    pub(crate) target_dir: Option<String>,
    pub(crate) autoload: PhpArray,
    pub(crate) dev_autoload: PhpArray,
    pub(crate) include_paths: Vec<String>,
    pub(crate) requires: Vec<Link>,
    pub(crate) replaces: Vec<Link>,
    pub(crate) provides: Vec<Link>,
}

fn isset<'a>(config: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    config.get(key).filter(|v| !v.is_null())
}

fn array(config: &Map<String, Value>, key: &str) -> PhpArray {
    match isset(config, key).map(to_php) {
        Some(PhpValue::Array(a)) => a,
        _ => PhpArray::new(),
    }
}

// Composer: Package/Loader/ArrayLoader.php parseLinks
fn links(config: &Map<String, Value>, key: &str, self_version: &str) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    let Some(Value::Object(map)) = isset(config, key) else {
        return out;
    };
    for (target, constraint) in map {
        let Value::String(constraint) = constraint else {
            continue;
        };
        let constraint = if constraint == "self.version" {
            self_version.to_owned()
        } else {
            constraint.clone()
        };
        let target = target.to_ascii_lowercase();
        match out.iter_mut().find(|l| l.target == target) {
            Some(link) => link.constraint = constraint,
            None => out.push(Link { target, constraint }),
        }
    }
    out
}

impl Package {
    pub(crate) fn from_config(config: &Map<String, Value>, default_name: &str) -> Self {
        let pretty_name = isset(config, "name")
            .and_then(Value::as_str)
            .unwrap_or(default_name)
            .to_owned();
        let version = isset(config, "version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let target_dir = match isset(config, "target-dir") {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        };
        let include_paths = match isset(config, "include-path") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => Vec::new(),
        };
        Self {
            name: pretty_name.to_ascii_lowercase(),
            kind: isset(config, "type")
                .and_then(Value::as_str)
                .unwrap_or("library")
                .to_ascii_lowercase(),
            target_dir,
            autoload: array(config, "autoload"),
            dev_autoload: array(config, "autoload-dev"),
            include_paths,
            requires: links(config, "require", &version),
            replaces: links(config, "replace", &version),
            provides: links(config, "provide", &version),
            pretty_name,
        }
    }

    /// `getNames()`: the name plus everything it replaces or provides.
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.name.as_str()).chain(
            self.replaces
                .iter()
                .chain(&self.provides)
                .map(|l| l.target.as_str()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Link, Package, to_php};
    use phpm_php::{PhpKey, PhpValue};
    use serde_json::{Map, Value, json};

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn loads_autoload_fields() {
        let p = Package::from_config(
            &obj(json!({
                "name": "Acme/Foo",
                "version": "1.2.0",
                "type": "Library",
                "target-dir": "Acme/Foo",
                "autoload": {"psr-0": {"Acme": "src/"}},
                "include-path": ["lib/"],
                "require": {"PHP": ">=8.1", "a/b": "self.version", "c/d": 1},
                "replace": {"x/y": "*"},
                "provide": {"ext-foo": "*"}
            })),
            "__root__",
        );
        assert_eq!(p.name, "acme/foo");
        assert_eq!(p.pretty_name, "Acme/Foo");
        assert_eq!(p.kind, "library");
        assert_eq!(p.target_dir.as_deref(), Some("Acme/Foo"));
        assert_eq!(p.include_paths, ["lib/"]);
        assert_eq!(
            p.requires,
            [
                Link {
                    target: "php".into(),
                    constraint: ">=8.1".into()
                },
                Link {
                    target: "a/b".into(),
                    constraint: "1.2.0".into()
                }
            ]
        );
        assert_eq!(
            p.names().collect::<Vec<_>>(),
            ["acme/foo", "x/y", "ext-foo"]
        );
        assert!(p.autoload.contains_key(&PhpKey::from("psr-0")));
        assert!(p.dev_autoload.is_empty());

        let root = Package::from_config(
            &obj(json!({"include-path": "x", "autoload": "bad"})),
            "__root__",
        );
        assert_eq!(root.name, "__root__");
        assert_eq!(root.include_paths, ["x"]);
        assert!(root.autoload.is_empty());
    }

    #[test]
    fn json_becomes_php_arrays() {
        let v = to_php(&json!({"0": [1.5, null, true], "a": "b"}));
        let PhpValue::Array(a) = v else {
            unreachable!()
        };
        let PhpValue::Array(list) = &a[&PhpKey::Int(0)] else {
            unreachable!()
        };
        assert_eq!(list[&PhpKey::Int(0)], PhpValue::String("1.5".into()));
        assert_eq!(list[&PhpKey::Int(1)], PhpValue::Null);
        assert_eq!(list[&PhpKey::Int(2)], PhpValue::Bool(true));
    }
}
