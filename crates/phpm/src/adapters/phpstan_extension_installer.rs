//! `vendor/phpstan/extension-installer/src/GeneratedConfig.php` as the
//! plugin writes it.
// Composer: phpstan/extension-installer src/Plugin.php process(); behaviour
// identical 1.4.0-1.4.3 (checked).

use phpm_lock::constraint::{self, Bound};
use phpm_lock::find_shortest_path;
use phpm_php::{PhpArray, PhpKey, PhpValue, var_export};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::Paths;

const TEMPLATE: &str =
    include_str!("../../vendor/phpstan-extension-installer/generated-config.template");

// Never counted as a missing extension even though their name contains
// "phpstan".
const ALWAYS_SUPPORTED: [&str; 4] = [
    "phpstan/phpstan",
    "phpstan/phpstan-shim",
    "phpstan/phpdoc-parser",
    "phpstan/extension-installer",
];

fn text<'a>(entry: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

fn extra(entry: &Map<String, Value>) -> Option<&Map<String, Value>> {
    entry.get("extra").and_then(Value::as_object)
}

/// `$installationManager->getInstallPath($package)`, absolute.
fn install_path(entry: &Map<String, Value>, paths: &Paths, vendor: &str) -> String {
    let name = text(entry, "name").unwrap_or_default();
    if let Some(custom) = paths.normalized.get(&name.to_ascii_lowercase()) {
        return custom.clone();
    }
    let mut path = format!("{vendor}/{name}");
    if let Some(target) = text(entry, "target-dir").filter(|t| !t.is_empty() && *t != "0") {
        path = format!("{path}/{target}");
    }
    phpm_lock::normalize_path(&path)
}

/// `$package->getFullPrettyVersion()`, limited to tagged releases: `Err`
/// declines a dev/branch version rather than guess its truncated reference.
fn full_pretty_version(entry: &Map<String, Value>) -> Result<String, String> {
    let pretty = text(entry, "version").unwrap_or_default();
    if pretty.starts_with("dev-") || pretty.ends_with("-dev") {
        return Err(format!("{pretty} is a branch version"));
    }
    Ok(pretty.to_owned())
}

/// The package's own `require.phpstan/phpstan` constraint's bounds, if it
/// has one. `Err` when phpm cannot parse it (declines rather than guess).
fn phpstan_bounds(entry: &Map<String, Value>) -> Result<Option<(Bound, Option<Bound>)>, String> {
    let Some(req) = entry.get("require").and_then(Value::as_object) else {
        return Ok(None);
    };
    let Some(value) = req.get("phpstan/phpstan").and_then(Value::as_str) else {
        return Ok(None);
    };
    let parsed =
        constraint::parse(value).map_err(|e| format!("require.phpstan/phpstan {value}: {e}"))?;
    Ok(Some(parsed.bounds()))
}

/// Whether `[lower, upper)` (per their own inclusivity) matches no version.
fn range_is_empty(lower: &Bound, upper: &Bound) -> bool {
    match constraint::version_compare(&lower.version, &upper.version) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => !(lower.inclusive && upper.inclusive),
        std::cmp::Ordering::Less => false,
    }
}

// Composer: phpstan/extension-installer Plugin::constraintIntoString
fn bounds_to_string(lower: &Bound, upper: &Bound) -> String {
    format!(
        "{}{}, {}{}",
        if lower.inclusive { ">=" } else { ">" },
        lower.version,
        if upper.inclusive { "<=" } else { "<" },
        upper.version
    )
}

struct Qualified<'a> {
    name: &'a str,
    install_path: String,
    relative_install_path: Option<String>,
    extra: Option<&'a Value>,
    version: String,
    phpstan_constraint: Option<(Bound, Bound)>,
}

/// Whether `entry` is one phpstan/extension-installer would treat as an
/// extension: `type: phpstan-extension`, or any type with `extra.phpstan`.
fn is_extension(entry: &Map<String, Value>) -> bool {
    text(entry, "type") == Some("phpstan-extension")
        || extra(entry).is_some_and(|e| e.contains_key("phpstan"))
}

#[allow(clippy::too_many_lines, reason = "one port of Plugin::process")]
/// What the first pass through the local repository found.
struct Collected<'a> {
    qualified: Vec<Qualified<'a>>,
    not_installed: BTreeMap<&'a str, String>,
}

fn collect<'a>(
    packages: &'a [&'a Map<String, Value>],
    root_extra: Option<&Map<String, Value>>,
    generated_config_dir: &str,
    paths: &Paths,
    vendor: &str,
) -> Result<Collected<'a>, String> {
    let ignore: Vec<&str> = root_extra
        .and_then(|e| e.get("phpstan/extension-installer"))
        .and_then(|e| e.get("ignore"))
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut data: BTreeMap<&str, Qualified<'_>> = BTreeMap::new();
    let mut not_installed: BTreeMap<&str, String> = BTreeMap::new();
    for entry in packages {
        let Some(name) = text(entry, "name") else {
            continue;
        };
        if !is_extension(entry) {
            if name.contains("phpstan") && !ALWAYS_SUPPORTED.contains(&name) {
                not_installed.insert(name, full_pretty_version(entry)?);
            }
            continue;
        }
        if ignore.contains(&name) {
            continue;
        }
        let absolute = install_path(entry, paths, vendor);
        let phpstan_constraint = match phpstan_bounds(entry)? {
            None => None,
            Some((lower, upper)) => {
                let Some(upper) = upper else {
                    continue; // positive infinity: excluded entirely, like Composer
                };
                if lower.is_zero() {
                    continue; // zero lower bound: excluded entirely, like Composer
                }
                Some((lower, upper))
            }
        };
        data.insert(
            name,
            Qualified {
                name,
                relative_install_path: find_shortest_path(generated_config_dir, &absolute, true),
                install_path: absolute,
                extra: extra(entry).and_then(|e| e.get("phpstan")),
                version: full_pretty_version(entry)?,
                phpstan_constraint,
            },
        );
    }
    Ok(Collected {
        qualified: data.into_values().collect(),
        not_installed,
    })
}

/// `vendor/phpstan/extension-installer/src/GeneratedConfig.php`'s bytes.
pub(crate) fn generate(
    packages: &[&Map<String, Value>],
    root_extra: Option<&Map<String, Value>>,
    paths: &Paths,
    vendor: &str,
) -> Result<Vec<u8>, String> {
    let own_path = paths
        .normalized
        .get("phpstan/extension-installer")
        .cloned()
        .unwrap_or_else(|| format!("{vendor}/phpstan/extension-installer"));
    let generated_config_dir = format!("{own_path}/src");

    // Composer also tracks ignored names for its own console output,
    // which phpm does not replicate.
    let collected = collect(packages, root_extra, &generated_config_dir, paths, vendor)?;

    let mut constraints: Vec<(Bound, Bound)> = Vec::new();
    let mut extensions = PhpArray::new();
    for q in &collected.qualified {
        let per_package = q
            .phpstan_constraint
            .as_ref()
            .map(|(l, u)| bounds_to_string(l, u));
        if let Some(c) = &q.phpstan_constraint {
            constraints.push(c.clone());
        }
        let mut entry = PhpArray::new();
        entry.insert(PhpKey::from("install_path"), q.install_path.clone().into());
        entry.insert(
            PhpKey::from("relative_install_path"),
            q.relative_install_path.clone().into(),
        );
        entry.insert(
            PhpKey::from("extra"),
            match q.extra {
                Some(v) => phpm_php::value_from_json(v)
                    .map_err(|e| format!("{} extra.phpstan: {e}", q.name))?,
                None => PhpValue::Null,
            },
        );
        entry.insert(PhpKey::from("version"), q.version.clone().into());
        entry.insert(PhpKey::from("phpstanVersionConstraint"), per_package.into());
        extensions.insert(PhpKey::from(q.name), PhpValue::Array(entry));
    }

    // Composer: Intervals::compactConstraint of a conjunctive MultiConstraint
    // of simple bounded ranges always has these bounds: the intersection's,
    // computed the same way `MultiConstraint::extractBounds` folds a
    // conjunction (max of the lowers, min of the uppers).
    let overall = match constraints.split_first() {
        None => None,
        Some(((first_lower, first_upper), rest)) => {
            let mut lower = first_lower.clone();
            let mut upper = first_upper.clone();
            for (l, u) in rest {
                if l.is_higher_than(&lower) {
                    lower = l.clone();
                }
                if u.is_lower_than(&upper) {
                    upper = u.clone();
                }
            }
            if range_is_empty(&lower, &upper) {
                return Err(
                    "the phpstan/phpstan constraints across extensions do not overlap".into(),
                );
            }
            Some(bounds_to_string(&lower, &upper))
        }
    };

    let not_installed_array: PhpArray = collected
        .not_installed
        .into_iter()
        .map(|(name, version)| (PhpKey::from(name), PhpValue::from(version)))
        .collect();

    let code = TEMPLATE.replacen("%s", &var_export(&PhpValue::Array(extensions)), 1);
    let code = code.replacen("%s", &var_export(&PhpValue::Array(not_installed_array)), 1);
    let code = code.replacen("%s", &var_export(&overall.into()), 1);
    Ok(code.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::generate;
    use crate::adapters::Paths;
    use serde_json::{Map, Value, json};

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    fn pkg(name: &str, phpstan_extra: &Value, require: &Value) -> Map<String, Value> {
        obj(
            json!({"name": name, "version": "1.0.0", "extra": {"phpstan": phpstan_extra}, "require": require}),
        )
    }

    #[test]
    fn matches_the_real_drupal_recommended_shape() {
        let packages = [
            pkg(
                "composer/composer",
                &json!({"includes": ["phpstan/rules.neon"]}),
                &json!({}),
            ),
            pkg(
                "mglaman/phpstan-drupal",
                &json!({"includes": ["extension.neon", "rules.neon"]}),
                &json!({"phpstan/phpstan": "^2.1"}),
            ),
            pkg(
                "phpstan/phpstan-deprecation-rules",
                &json!({"includes": ["rules.neon"]}),
                &json!({"phpstan/phpstan": "^2.1.39"}),
            ),
            pkg(
                "phpstan/phpstan-phpunit",
                &json!({"includes": ["extension.neon", "rules.neon"]}),
                &json!({"phpstan/phpstan": "^2.1.32"}),
            ),
            obj(json!({"name": "phpstan/phpstan", "version": "2.1.40"})),
            obj(json!({"name": "a/lib", "version": "1.0.0"})),
        ];
        let refs: Vec<&Map<String, Value>> = packages.iter().collect();
        let paths = Paths::default();
        let code = generate(&refs, None, &paths, "/p/vendor").unwrap();
        let code = String::from_utf8(code).unwrap();
        assert!(
            code.contains(
                "public const PHPSTAN_VERSION_CONSTRAINT = '>=2.1.39.0-dev, <3.0.0.0-dev';"
            ),
            "{code}"
        );
        assert!(
            code.contains("'install_path' => '/p/vendor/mglaman/phpstan-drupal',"),
            "{code}"
        );
        assert!(
            code.contains("'relative_install_path' => '../../../mglaman/phpstan-drupal',"),
            "{code}"
        );
        assert!(
            code.contains("'phpstanVersionConstraint' => '>=2.1.0.0-dev, <3.0.0.0-dev',"),
            "{code}"
        );
        assert!(
            code.contains("public const NOT_INSTALLED = array (\n);"),
            "{code}"
        );
        assert!(!code.contains("a/lib"), "{code}");
    }

    #[test]
    fn excludes_degenerate_phpstan_phpstan_bounds_entirely() {
        let packages = [
            pkg(
                "x/zero",
                &json!({"a": true}),
                &json!({"phpstan/phpstan": "*"}),
            ),
            pkg(
                "x/inf",
                &json!({"a": true}),
                &json!({"phpstan/phpstan": ">=1.0"}),
            ),
        ];
        let refs: Vec<&Map<String, Value>> = packages.iter().collect();
        let code = generate(&refs, None, &Paths::default(), "/p/vendor").unwrap();
        let code = String::from_utf8(code).unwrap();
        assert!(!code.contains("x/zero"), "{code}");
        assert!(!code.contains("x/inf"), "{code}");
        assert!(
            code.contains("PHPSTAN_VERSION_CONSTRAINT = NULL;"),
            "{code}"
        );
    }

    #[test]
    fn tracks_missing_extensions_and_respects_ignore() {
        let packages = [
            obj(json!({"name": "phpstan/phpstan-nope", "version": "1.0.0"})),
            obj(json!({"name": "phpstan/phpstan", "version": "1.0.0"})),
            pkg("x/ignored", &json!({"a": true}), &json!({})),
        ];
        let refs: Vec<&Map<String, Value>> = packages.iter().collect();
        let root = obj(json!({"phpstan/extension-installer": {"ignore": ["x/ignored"]}}));
        let code = generate(&refs, Some(&root), &Paths::default(), "/p/vendor").unwrap();
        let code = String::from_utf8(code).unwrap();
        assert!(
            code.contains("'phpstan/phpstan-nope' => '1.0.0',"),
            "{code}"
        );
        assert!(!code.contains("phpstan/phpstan'"), "{code}");
        assert!(!code.contains("x/ignored"), "{code}");
    }

    #[test]
    fn declines_a_dev_version_or_a_non_overlapping_set() {
        let dev = pkg("x/dev", &json!({"a": true}), &json!({}));
        let mut dev = dev;
        dev.insert("version".into(), json!("dev-main"));
        let refs: Vec<&Map<String, Value>> = vec![&dev];
        assert!(generate(&refs, None, &Paths::default(), "/p/vendor").is_err());

        let a = pkg(
            "x/a",
            &json!({"a": true}),
            &json!({"phpstan/phpstan": "^1.0"}),
        );
        let b = pkg(
            "x/b",
            &json!({"a": true}),
            &json!({"phpstan/phpstan": "^2.0"}),
        );
        let refs: Vec<&Map<String, Value>> = vec![&a, &b];
        assert!(generate(&refs, None, &Paths::default(), "/p/vendor").is_err());
    }
}
